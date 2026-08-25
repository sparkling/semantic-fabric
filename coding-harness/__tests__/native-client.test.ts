// SPDX-License-Identifier: MIT

import { chmodSync, mkdtempSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
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

const ok = (stdout: string): NativeProcessResult => ({
  exitCode: 0,
  stdout,
  stderr: '',
  timedOut: false,
});

class FakeRunner implements NativeProcessRunner {
  constructor(private readonly response: (request: NativeProcessRequest) => NativeProcessResult) {}

  async run(request: NativeProcessRequest): Promise<NativeProcessResult> {
    return this.response(request);
  }
}

describe('native adapter structured client', () => {
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
    expect(first.invocationId).toMatch(/^native:codex:implementation:/);
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
    expect(invocation.invocationId).toMatch(/^native:claude-code:review:/);
    expect(invocation.outputDigest).toMatch(/^[a-f0-9]{64}$/);
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
