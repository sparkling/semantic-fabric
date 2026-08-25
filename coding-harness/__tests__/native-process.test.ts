// SPDX-License-Identifier: MIT

import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it } from 'vitest';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import { BoundedNativeProcessRunner } from '../src/native-process.js';
import type { NativeProcessRequest } from '../src/models/types.js';
import type { NativeModelOriginPinningBoundary } from '../src/network.js';
import type { NativeModelFilesystemBoundary } from '../src/native-filesystem.js';
import { digestValue } from '../src/receipts.js';
import { fakeResourceBoundary, TEST_RESOURCE_LIMITS } from './helpers.js';

const roots: string[] = [];
const fixture = join(dirname(fileURLToPath(import.meta.url)), 'fixtures/native-process-fixture.mjs');

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function workspace(): string {
  const root = mkdtempSync(join(tmpdir(), 'coding-harness-native-'));
  mkdirSync(join(root, '.git'));
  roots.push(root);
  return root;
}

function request(
  root: string,
  mode: string,
  overrides: Partial<NativeProcessRequest> = {},
): NativeProcessRequest {
  return {
    host: 'codex',
    purpose: 'model-invocation',
    model: 'gpt-5.6',
    operation: 'implementation',
    executable: process.execPath,
    args: [fixture, mode],
    cwd: root,
    env: {
      PATH: process.env.PATH ?? '/usr/bin',
      HOME: root,
      CODEX_HOME: root,
    },
    timeoutMs: 2_000,
    stdin: 'prompt on stdin',
    ...overrides,
  };
}

const egressBoundary: NativeModelOriginPinningBoundary = {
  pin: (command, origins) => ({
    enforcement: 'origin-pinned-process-boundary',
    mechanism: 'test-egress-firewall',
    pinnedOrigins: origins,
    command: {
      ...command,
      executable: '/usr/bin/env',
      args: [command.executable, ...command.args],
    },
  }),
  complete: () => ({
    allowedConnections: 1,
    deniedConnections: 0,
    connectDigest: digestValue('test-connect'),
  }),
};

const filesystemBoundary: NativeModelFilesystemBoundary = {
  isolate: (command, policy) => ({
    enforcement: 'os-filesystem-namespace',
    mechanism: 'test-filesystem-namespace',
    mountManifestDigest: digestValue(policy),
    configurationMaskDigest: digestValue(policy.maskedPaths),
    ...policy,
    command: {
      ...command,
      executable: '/usr/bin/env',
      args: [command.executable, ...command.args],
    },
  }),
};

function runner(
  root: string,
  maxOutputBytes = 10_000,
  allowedReadRoots: readonly string[] = [root],
  allowedWriteRoots: readonly string[] = [root],
  forbiddenRoots: readonly string[] = [workspace()],
): BoundedNativeProcessRunner {
  return new BoundedNativeProcessRunner({
    config: SECURE_HARNESS_CONFIG,
    executables: { codex: process.execPath, 'claude-code': process.execPath },
    allowedRoots: [root],
    allowedReadRoots,
    allowedWriteRoots,
    forbiddenRoots,
    egressBoundary,
    filesystemBoundary,
    resourceBoundary: fakeResourceBoundary,
    resourceLimits: TEST_RESOURCE_LIMITS,
    maxOutputBytes,
    terminationGraceMs: 25,
  });
}

describe('bounded native subscription process bridge', () => {
  it('writes prompt stdin and passes only the already-sanitized native environment', async () => {
    const root = workspace();
    const bridge = runner(root);
    const result = await bridge.run(request(root, 'stdin'));

    expect(result.exitCode).toBe(0);
    expect(result.timedOut).toBe(false);
    expect(JSON.parse(result.stdout)).toEqual({ input: 'prompt on stdin' });
    expect(bridge.networkEvidence()).toHaveLength(1);
    expect(bridge.networkEvidence()[0]).toMatchObject({
      host: 'codex',
      enforcement: 'origin-pinned-process-boundary',
      fallback: 'none',
    });
    expect(bridge.filesystemEvidence()[0]).toMatchObject({
      enforcement: 'os-filesystem-namespace',
      hostFileConfidentiality: true,
    });
    expect(bridge.executableEvidence().codex.digest).toMatch(/^[a-f0-9]{64}$/);
  });

  it('enforces the combined output ceiling', async () => {
    const root = workspace();
    const result = await runner(root, 128).run(request(root, 'output', { stdin: undefined }));
    expect(result.exitCode).not.toBe(0);
    expect(result.outputLimitExceeded).toBe(true);
    expect(Buffer.byteLength(result.stdout) + Buffer.byteLength(result.stderr)).toBeLessThanOrEqual(128);
  });

  it('binds an exact workspace root and separate evidence-file capabilities', async () => {
    const root = workspace();
    const evidence = workspace();
    const schema = join(evidence, 'response.schema.json');
    const output = join(evidence, 'response.json');
    writeFileSync(schema, '{}\n');
    writeFileSync(output, '');
    const bridge = runner(root, 10_000, [root, evidence], [evidence]);

    const result = await bridge.run(request(root, 'stdin', {
      readOnlyPaths: [schema],
      writablePaths: [output],
    }));

    expect(result.exitCode).toBe(0);
    expect(bridge.filesystemEvidence()[0].maskedPaths).toEqual([join(root, '.git')]);
  });

  it('terminates the process group on timeout and cancellation', async () => {
    const root = workspace();
    const timed = await runner(root).run(request(root, 'wait', { timeoutMs: 50, stdin: undefined }));
    expect(timed.timedOut).toBe(true);

    const controller = new AbortController();
    const pending = runner(root).run(request(root, 'wait', {
      signal: controller.signal,
      stdin: undefined,
    }));
    setTimeout(() => controller.abort(), 50);
    const cancelled = await pending;
    expect(cancelled.cancelled).toBe(true);
  });

  it('fails closed for executable substitution and forbidden transport variables', async () => {
    const root = workspace();
    await expect(runner(root).run(request(root, 'stdin', { executable: '/bin/false' }))).rejects.toThrow(
      'HARNESS_NATIVE_EXECUTABLE_MISMATCH',
    );
    await expect(runner(root).run(request(root, 'stdin', {
      env: { PATH: '/usr/bin', OPENAI_BASE_URL: 'https://openrouter.ai' },
    }))).rejects.toThrow('HARNESS_NATIVE_ENVIRONMENT_FORBIDDEN');
    const outside = workspace();
    await expect(runner(root).run(request(outside, 'stdin'))).rejects.toThrow(
      'HARNESS_NATIVE_CWD_OUTSIDE_ALLOWED_ROOTS',
    );
  });

  it('requires a real origin-pinning wrapper', () => {
    const root = workspace();
    expect(() => new BoundedNativeProcessRunner({
      config: SECURE_HARNESS_CONFIG,
      executables: { codex: process.execPath, 'claude-code': process.execPath },
      allowedRoots: [root],
      allowedReadRoots: [root],
      allowedWriteRoots: [root],
      forbiddenRoots: [workspace()],
    } as never)).toThrow('HARNESS_NATIVE_ORIGIN_BOUNDARY_REQUIRED');
  });

  it('requires filesystem isolation and rejects symlinked clients', () => {
    const root = workspace();
    expect(() => new BoundedNativeProcessRunner({
      config: SECURE_HARNESS_CONFIG,
      executables: { codex: process.execPath, 'claude-code': process.execPath },
      allowedRoots: [root],
      allowedReadRoots: [root],
      allowedWriteRoots: [root],
      forbiddenRoots: [workspace()],
      egressBoundary,
    } as never)).toThrow('HARNESS_NATIVE_FILESYSTEM_BOUNDARY_REQUIRED');
    const linked = join(root, 'codex-link');
    symlinkSync(process.execPath, linked);
    expect(() => new BoundedNativeProcessRunner({
      config: SECURE_HARNESS_CONFIG,
      executables: { codex: linked, 'claude-code': process.execPath },
      allowedRoots: [root],
      allowedReadRoots: [root],
      allowedWriteRoots: [root],
      forbiddenRoots: [workspace()],
      egressBoundary,
      filesystemBoundary,
      resourceBoundary: fakeResourceBoundary,
      resourceLimits: TEST_RESOURCE_LIMITS,
    })).toThrow('HARNESS_NATIVE_EXECUTABLE_UNTRUSTED:codex');
  });
});
