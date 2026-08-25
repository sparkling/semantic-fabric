// SPDX-License-Identifier: MIT

import {
  chmodSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { afterEach, describe, expect, it } from 'vitest';
import { NativeAdapterStructuredClient } from '../src/native-client.js';
import {
  ClaudeCodeSubscriptionAdapter,
  CodexSubscriptionAdapter,
} from '../src/models/native-adapters.js';
import type {
  NativeProcessRequest,
  NativeProcessResult,
  NativeProcessRunner,
} from '../src/models/types.js';

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function root(prefix: string): string {
  const path = mkdtempSync(join(tmpdir(), prefix));
  roots.push(path);
  return path;
}

let sequence = 0;
const ok = (stdout: string): NativeProcessResult => ({
  executionId: `native-run:10000000-0000-4000-8000-${String(++sequence).padStart(12, '0')}`,
  exitCode: 0,
  stdout,
  stderr: '',
  timedOut: false,
  stdoutDigest: createHash('sha256').update(stdout).digest('hex'),
  stderrDigest: createHash('sha256').update('').digest('hex'),
});

class FakeRunner implements NativeProcessRunner {
  constructor(private readonly response: (request: NativeProcessRequest) => NativeProcessResult) {}

  async run(request: NativeProcessRequest): Promise<NativeProcessResult> {
    return this.response(request);
  }
}

describe('native adapter structured client', () => {
  it('bounds native structured responses to verifier-compatible shapes', async () => {
    const evidenceRoot = root('coding-harness-native-evidence-');
    const workspaceRoot = root('coding-harness-native-workspace-');
    const schemas: Record<string, unknown>[] = [];
    const runner = new FakeRunner((request) => {
      const schemaIndex = request.args.indexOf('--output-schema');
      const outputIndex = request.args.indexOf('--output-last-message');
      const schema = JSON.parse(
        readFileSync(request.args[schemaIndex + 1], 'utf8'),
      ) as Record<string, unknown>;
      schemas.push(schema);
      const properties = schema.properties as Record<string, unknown>;
      const output = 'patch' in properties
        ? { patch: validPatch() }
        : 'accepted' in properties
          ? { accepted: true, reasons: [] }
          : {
              proposal: {
                summary: 'Preserve checked binding semantics.',
                invariants: ['False bindings prune the branch.'],
                steps: ['Check every bind result.'],
              },
              confidence: 1,
            };
      writeFileSync(request.args[outputIndex + 1], JSON.stringify(output));
      return ok('');
    });
    const adapter = new CodexSubscriptionAdapter({
      executable: '/tools/codex',
      runner,
      sourceEnvironment: { HOME: '/home/tester', PATH: '/usr/bin' },
      evidenceRoot,
    });
    const client = new NativeAdapterStructuredClient({
      adapter,
      evidenceRoot,
      workspaceRoot,
      timeoutMs: 1_000,
    });

    await client.invoke({
      candidate: { host: 'codex', model: 'gpt-5.6' },
      operation: 'architecture',
      prompt: 'return a bounded architecture',
    });
    await client.invoke({
      candidate: { host: 'codex', model: 'gpt-5.6' },
      operation: 'implementation',
      prompt: 'return a bounded patch',
    });
    await client.invoke({
      candidate: { host: 'codex', model: 'gpt-5.6' },
      operation: 'review',
      prompt: 'return a bounded review',
    });

    expect(schemas[0]).toMatchObject({
      properties: {
        proposal: {
          type: 'object',
          additionalProperties: false,
          required: ['summary', 'invariants', 'steps'],
          properties: {
            summary: { maxLength: 2_000 },
            invariants: { maxItems: 8, items: { maxLength: 400 } },
            steps: { maxItems: 8, items: { maxLength: 400 } },
          },
        },
      },
    });
    const maximallyEscapedProposal = {
      summary: '\0'.repeat(2_000),
      invariants: Array.from({ length: 8 }, () => '\0'.repeat(400)),
      steps: Array.from({ length: 8 }, () => '\0'.repeat(400)),
    };
    expect(Buffer.byteLength(JSON.stringify(maximallyEscapedProposal), 'utf8'))
      .toBeLessThanOrEqual(64_000);
    expect(schemas[1]).toMatchObject({
      properties: {
        patch: {
          maxLength: 256_000,
          pattern: '^diff --git ',
          description: expect.stringContaining('Do not include Markdown fences'),
        },
      },
    });
    expect(schemas[2]).toMatchObject({
      properties: {
        reasons: { maxItems: 8, items: { maxLength: 1_000 } },
      },
    });
  });

  it('assigns Codex invocation identity outside model-controlled JSON', async () => {
    const evidenceRoot = root('coding-harness-native-evidence-');
    const workspaceRoot = root('coding-harness-native-workspace-');
    const runner = new FakeRunner((request) => {
      const outputIndex = request.args.indexOf('--output-last-message');
      expect(outputIndex).toBeGreaterThan(0);
      writeFileSync(request.args[outputIndex + 1], JSON.stringify({ patch: validPatch() }));
      return ok('');
    });
    const adapter = new CodexSubscriptionAdapter({
      executable: '/tools/codex',
      runner,
      sourceEnvironment: { HOME: '/home/tester', PATH: '/usr/bin' },
      evidenceRoot,
    });
    const client = new NativeAdapterStructuredClient({
      adapter,
      evidenceRoot,
      workspaceRoot,
      timeoutMs: 1_000,
    });

    const first = await client.invoke({
      candidate: { host: 'codex', model: 'gpt-5.6' },
      operation: 'implementation',
      prompt: 'return a patch',
    });
    const second = await client.invoke({
      candidate: { host: 'codex', model: 'gpt-5.6' },
      operation: 'implementation',
      prompt: 'return a patch',
    });

    expect(first.output).toEqual({ patch: validPatch() });
    expect(first.invocationId).toMatch(/^native-run:/);
    expect(first.invocationId).not.toBe(second.invocationId);
    expect(first.outputDigest).toMatch(/^[a-f0-9]{64}$/);
    for (const directory of readdirSync(evidenceRoot)) {
      expect(statSync(join(evidenceRoot, directory)).mode & 0o077).toBe(0);
    }
  });

  it('extracts Claude structured output while retaining the raw envelope digest', async () => {
    const evidenceRoot = root('coding-harness-native-evidence-');
    const workspaceRoot = root('coding-harness-native-workspace-');
    const runner = new FakeRunner(() => ok(JSON.stringify({
      type: 'result',
      subtype: 'success',
      is_error: false,
      structured_output: { accepted: false, reasons: ['invariant mismatch'] },
    })));
    const adapter = new ClaudeCodeSubscriptionAdapter({
      executable: '/tools/claude',
      runner,
      sourceEnvironment: { HOME: '/home/tester', PATH: '/usr/bin' },
    });
    const client = new NativeAdapterStructuredClient({
      adapter,
      evidenceRoot,
      workspaceRoot,
      timeoutMs: 1_000,
    });

    const invocation = await client.invoke({
      candidate: { host: 'claude-code', model: 'claude-sonnet-5' },
      operation: 'review',
      prompt: 'review the patch',
    });

    expect(invocation.output).toEqual({ accepted: false, reasons: ['invariant mismatch'] });
    expect(invocation.invocationId).toMatch(/^native-run:/);
    expect(invocation.outputDigest).toMatch(/^[a-f0-9]{64}$/);
  });

  it('rejects a Claude error envelope even if it carries structured output', async () => {
    const evidenceRoot = root('coding-harness-native-evidence-');
    const workspaceRoot = root('coding-harness-native-workspace-');
    const runner = new FakeRunner(() => ok(JSON.stringify({
      type: 'result',
      subtype: 'error_max_turns',
      is_error: true,
      structured_output: { accepted: true, reasons: [] },
    })));
    const adapter = new ClaudeCodeSubscriptionAdapter({
      executable: '/tools/claude',
      runner,
      sourceEnvironment: { HOME: '/home/tester', PATH: '/usr/bin' },
    });
    const client = new NativeAdapterStructuredClient({
      adapter,
      evidenceRoot,
      workspaceRoot,
      timeoutMs: 1_000,
    });

    await expect(client.invoke({
      candidate: { host: 'claude-code', model: 'claude-sonnet-5' },
      operation: 'review',
      prompt: 'review the patch',
    })).rejects.toThrow('HARNESS_NATIVE_STRUCTURED_ENVELOPE_INVALID');
  });

  it('rejects a Claude success envelope that withholds structured output', async () => {
    const evidenceRoot = root('coding-harness-native-evidence-');
    const workspaceRoot = root('coding-harness-native-workspace-');
    const runner = new FakeRunner(() => ok(JSON.stringify({
      type: 'result',
      subtype: 'success',
      is_error: false,
      result: '{"patch":"untrusted free-text fallback"}',
    })));
    const adapter = new ClaudeCodeSubscriptionAdapter({
      executable: '/tools/claude',
      runner,
      sourceEnvironment: { HOME: '/home/tester', PATH: '/usr/bin' },
    });
    const client = new NativeAdapterStructuredClient({
      adapter,
      evidenceRoot,
      workspaceRoot,
      timeoutMs: 1_000,
    });

    await expect(client.invoke({
      candidate: { host: 'claude-code', model: 'claude-sonnet-5' },
      operation: 'implementation',
      prompt: 'return a patch',
    })).rejects.toThrow('HARNESS_NATIVE_STRUCTURED_OUTPUT_MISSING');
  });

  it('classifies malformed Claude envelope JSON without exposing it', async () => {
    const evidenceRoot = root('coding-harness-native-evidence-');
    const workspaceRoot = root('coding-harness-native-workspace-');
    const runner = new FakeRunner(() => ok('untrusted malformed response'));
    const adapter = new ClaudeCodeSubscriptionAdapter({
      executable: '/tools/claude',
      runner,
      sourceEnvironment: { HOME: '/home/tester', PATH: '/usr/bin' },
    });
    const client = new NativeAdapterStructuredClient({
      adapter,
      evidenceRoot,
      workspaceRoot,
      timeoutMs: 1_000,
    });

    await expect(client.invoke({
      candidate: { host: 'claude-code', model: 'claude-sonnet-5' },
      operation: 'implementation',
      prompt: 'return a patch',
    })).rejects.toThrow('HARNESS_NATIVE_STRUCTURED_ENVELOPE_INVALID');
  });

  it('rejects evidence directories that are visible to other users', () => {
    const workspaceRoot = root('coding-harness-native-workspace-');
    const runner = new FakeRunner(() => ok('{}'));
    const adapter = new ClaudeCodeSubscriptionAdapter({
      executable: '/tools/claude',
      runner,
      sourceEnvironment: { HOME: '/home/tester', PATH: '/usr/bin' },
    });
    const publicRoot = root('coding-harness-native-public-');
    chmodSync(publicRoot, 0o755);
    expect(() => new NativeAdapterStructuredClient({
      adapter,
      evidenceRoot: publicRoot,
      workspaceRoot,
      timeoutMs: 1_000,
    })).toThrow('HARNESS_NATIVE_EVIDENCE_ROOT_INVALID');
  });
});

function validPatch(): string {
  return 'diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n';
}
