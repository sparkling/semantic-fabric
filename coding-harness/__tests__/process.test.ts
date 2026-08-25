// SPDX-License-Identifier: MIT

import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it } from 'vitest';
import type { StructuredCommand } from '../src/contracts.js';
import { runStructuredProcess, sanitizeEnvironment } from '../src/process.js';
import { createTestConfig } from './helpers.js';

const temporaryRoots: string[] = [];
const fixture = join(dirname(fileURLToPath(import.meta.url)), 'fixtures/process-fixture.mjs');

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function workspace(): string {
  const root = mkdtempSync(join(tmpdir(), 'coding-harness-process-'));
  temporaryRoots.push(root);
  return root;
}

function command(mode: string, overrides: Partial<StructuredCommand> = {}): StructuredCommand {
  return {
    tool: 'node',
    executable: process.execPath,
    argv: [fixture, mode],
    cwd: '.',
    env: {},
    timeoutMs: 2_000,
    maxOutputBytes: 10_000,
    ...overrides,
  };
}

describe('structured process runner', () => {
  it('passes only explicitly allowed, non-provider environment variables', async () => {
    const root = workspace();
    const config = createTestConfig();
    const result = await runStructuredProcess(command('environment', { env: { SAFE_FLAG: 'visible' } }), {
      workspaceRoot: root,
      config,
      declaredTools: ['node'],
      sourceEnvironment: {
        PATH: process.env.PATH,
        OPENAI_API_KEY: 'secret',
        OPENROUTER_API_KEY: 'secret',
        HTTP_PROXY: 'http://127.0.0.1:8080',
        OPENAI_BASE_URL: 'https://gateway.invalid',
      },
    });
    expect(result.success).toBe(true);
    expect(JSON.parse(result.stdout)).toEqual({ safe: 'visible' });
  });

  it('rejects forbidden environment overrides and shell metacharacters', async () => {
    const config = createTestConfig();
    expect(() => sanitizeEnvironment({}, { OPENAI_API_KEY: 'secret' }, config)).toThrow(/not allowed/);
    await expect(runStructuredProcess(command('environment', { argv: [fixture, 'environment', '&&'] }), {
      workspaceRoot: workspace(),
      config,
      declaredTools: ['node'],
    })).rejects.toThrow(/shell metacharacter/);
  });

  it('enforces the combined output ceiling', async () => {
    const result = await runStructuredProcess(command('output', { maxOutputBytes: 128 }), {
      workspaceRoot: workspace(),
      config: createTestConfig(),
      declaredTools: ['node'],
    });
    expect(result.success).toBe(false);
    expect(result.outputLimitExceeded).toBe(true);
    expect(Buffer.byteLength(result.stdout) + Buffer.byteLength(result.stderr)).toBeLessThanOrEqual(128);
  });

  it('terminates the process group on timeout', async () => {
    const result = await runStructuredProcess(command('wait', { timeoutMs: 50 }), {
      workspaceRoot: workspace(),
      config: createTestConfig(),
      declaredTools: ['node'],
    });
    expect(result.success).toBe(false);
    expect(result.timedOut).toBe(true);
    expect(result.durationMs).toBeLessThan(1_000);
  });

  it('propagates cancellation to the process group', async () => {
    const controller = new AbortController();
    const pending = runStructuredProcess(command('wait'), {
      workspaceRoot: workspace(),
      config: createTestConfig(),
      declaredTools: ['node'],
      signal: controller.signal,
    });
    setTimeout(() => controller.abort(), 50);
    const result = await pending;
    expect(result.success).toBe(false);
    expect(result.cancelled).toBe(true);
    expect(result.durationMs).toBeLessThan(1_000);
  });
});
