// SPDX-License-Identifier: MIT
import { chmodSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { afterEach, describe, expect, it } from 'vitest';

const roots: string[] = [];
const thread = '01a03612-aacf-70e1-ad27-063b931641b2';
const script = resolve('../scripts/queue-programme-review.sh');
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });

function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'sf-review-queue-'));
  roots.push(root);
  const codex = join(root, 'codex with spaces');
  const prompt = join(root, 'review prompt.md');
  const calls = join(root, 'calls.jsonl');
  writeFileSync(codex, `#!${process.execPath}\n`
    + `const fs = require('node:fs');\n`
    + `fs.appendFileSync(process.env.REVIEW_TEST_CALLS, JSON.stringify({\n`
    + `  args: process.argv.slice(2),\n`
    + `  openai: process.env.OPENAI_API_KEY, anthropic: process.env.ANTHROPIC_API_KEY,\n`
    + `  gateway: process.env.OPENROUTER_API_KEY, base: process.env.OPENAI_BASE_URL\n`
    + `}) + '\\n');\n`
    + `process.exit(Number(process.env.REVIEW_TEST_EXIT || 0));\n`);
  chmodSync(codex, 0o700);
  writeFileSync(prompt, 'Review delivery; literal $(not-a-command), `text`, quotes " and Unicode λ.\n');
  const run = (args = [codex, thread, prompt], exitCode = 0) => spawnSync('/usr/bin/bash', [script, ...args], {
    encoding: 'utf8',
    env: {
      ...process.env, REVIEW_TEST_CALLS: calls, REVIEW_TEST_EXIT: String(exitCode),
      OPENAI_API_KEY: 'test-only', ANTHROPIC_API_KEY: 'test-only',
      OPENROUTER_API_KEY: 'test-only', OPENAI_BASE_URL: 'https://invalid.example',
    },
  });
  return { codex, prompt, calls, run };
}

describe('six-hour review delivery', () => {
  it('queues the exact prompt once into the pinned native conversation without changing model', () => {
    const f = fixture();
    expect(f.run().status).toBe(0);
    const calls = readFileSync(f.calls, 'utf8').trim().split('\n').map(line => JSON.parse(line));
    expect(calls).toEqual([{ args: [
      'queue', '--thread', thread, '--message', readFileSync(f.prompt, 'utf8').trimEnd(),
      '--cd', resolve('..'),
    ] }]);
    expect(calls[0].args).not.toContain('exec');
    expect(calls[0].args).not.toContain('resume');
    expect(calls[0].args).not.toContain('--model');
  });

  it('propagates native queue failure without retrying or starting a second writer', () => {
    const f = fixture();
    expect(f.run(undefined, 23).status).toBe(23);
    expect(readFileSync(f.calls, 'utf8').trim().split('\n')).toHaveLength(1);
  });

  it('rejects missing/ambiguous session and invalid executable before native invocation', () => {
    const f = fixture();
    for (const args of [[], [f.codex, '--last', f.prompt], ['codex', thread, f.prompt],
      ['/no/such/codex', thread, f.prompt], [f.codex, thread, 'relative.md']]) {
      expect(f.run(args).status).toBe(64);
    }
    expect(existsSync(f.calls)).toBe(false);
  });

  it('rejects missing, empty and whitespace-only prompts without a fallback', () => {
    const f = fixture();
    expect(f.run([f.codex, thread, `${f.prompt}.missing`]).status).toBe(64);
    for (const value of ['', ' \n\t']) {
      writeFileSync(f.prompt, value);
      expect(f.run().status).toBe(64);
    }
    expect(existsSync(f.calls)).toBe(false);
  });
});
