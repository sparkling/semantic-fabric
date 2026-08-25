// SPDX-License-Identifier: MIT

import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import { BoundedNativeProcessRunner } from '../src/native-process.js';
import type { NativeProcessRequest } from '../src/models/types.js';
import type { NativeModelOriginPinningBoundary } from '../src/network.js';
import type { NativeModelFilesystemBoundary } from '../src/native-filesystem.js';
import { digestValue } from '../src/receipts.js';
import type { NativeResourceBoundary } from '../src/resource-boundary.js';
import { fakeResourceBoundary, TEST_RESOURCE_LIMITS, TEST_RESOURCE_SCOPE } from './helpers.js';

const roots: string[] = [];
const fixture = join(dirname(fileURLToPath(import.meta.url)), 'fixtures/native-process-fixture.mjs');

afterEach(() => {
  for (const root of roots.splice(0)) {
    const pidPath = join(root, 'holder.pid');
    if (existsSync(pidPath)) {
      try { killHolder(pidPath); } catch { /* exited or invalid */ }
    }
    rmSync(root, { recursive: true, force: true });
  }
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
    privateEphemeralHome: true,
    hostCredentialPathMounted: false,
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
  maskedWorkspacePaths: readonly string[] = [],
  resourceBoundary: NativeResourceBoundary = fakeResourceBoundary,
  networkBoundary: NativeModelOriginPinningBoundary = egressBoundary,
): BoundedNativeProcessRunner {
  return new BoundedNativeProcessRunner({
    config: SECURE_HARNESS_CONFIG,
    executables: { codex: process.execPath, 'claude-code': process.execPath },
    allowedRoots: [root],
    allowedReadRoots,
    allowedWriteRoots,
    forbiddenRoots,
    egressBoundary: networkBoundary,
    filesystemBoundary,
    resourceBoundary,
    resourceLimits: TEST_RESOURCE_LIMITS,
    maskedWorkspacePaths,
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
    expect(bridge.executionEvidence(result.executionId).filesystem).toMatchObject({
      hostFileConfidentiality: true,
      emptyPrivateHome: true,
      privateEphemeralHome: true,
      hostRootMounted: false,
      hostCredentialPathMounted: false,
    });
    expect(bridge.executableEvidence().codex.digest).toMatch(/^[a-f0-9]{64}$/);
  });

  it('enforces the combined output ceiling', async () => {
    const root = workspace();
    const terminateAndVerify = vi.fn().mockResolvedValue(undefined);
    const result = await runner(root, 128, [root], [root], [workspace()], [], {
      ...fakeResourceBoundary, terminateAndVerify,
    }).run(request(root, 'output', { stdin: undefined }));
    expect(result.exitCode).not.toBe(0);
    expect(result.outputLimitExceeded).toBe(true);
    expect(Buffer.byteLength(result.stdout) + Buffer.byteLength(result.stderr)).toBeLessThanOrEqual(128);
    expect(terminateAndVerify).toHaveBeenCalledTimes(2);
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

  it('adds required evaluator paths to every workspace mask', async () => {
    const root = workspace();
    mkdirSync(join(root, 'sealed'), { recursive: true });
    writeFileSync(join(root, 'sealed/evaluator.rs'), 'oracle law\n');
    const bridge = runner(root, 10_000, [root], [root], [workspace()], [
      'sealed/evaluator.rs',
    ]);

    await bridge.run(request(root, 'stdin'));

    expect(bridge.filesystemEvidence()[0].maskedPaths).toEqual([
      join(root, '.git'),
      join(root, 'sealed/evaluator.rs'),
    ]);
  });

  it('fails closed when a required evaluator mask is absent', async () => {
    const root = workspace();
    await expect(runner(root, 10_000, [root], [root], [workspace()], [
      'sealed/missing.rs',
    ]).run(request(root, 'stdin'))).rejects.toThrow('HARNESS_NATIVE_REQUIRED_MASK_PATH_MISSING');
  });

  it('terminates the process group on timeout and cancellation', async () => {
    const root = workspace();
    const timedTermination = vi.fn().mockResolvedValue(undefined);
    const bridge = runner(root, 10_000, [root], [root], [workspace()], [], {
      ...fakeResourceBoundary, terminateAndVerify: timedTermination,
    });
    const timed = await bridge.run(request(root, 'wait', { timeoutMs: 50, stdin: undefined }));
    expect(timed.timedOut).toBe(true);
    expect(timedTermination).toHaveBeenCalledTimes(2);
    expect(bridge.resourceEvidence()[0]?.limits.runtimeSeconds).toBe(1);

    const controller = new AbortController();
    const cancelledTermination = vi.fn().mockResolvedValue(undefined);
    const pending = runner(root, 10_000, [root], [root], [workspace()], [], {
      ...fakeResourceBoundary, terminateAndVerify: cancelledTermination,
    }).run(request(root, 'wait', {
      signal: controller.signal,
      stdin: undefined,
    }));
    setTimeout(() => controller.abort(), 50);
    const cancelled = await pending;
    expect(cancelled.cancelled).toBe(true);
    expect(cancelledTermination).toHaveBeenCalledTimes(2);
  });

  it('rechecks a late scope on direct exit before inherited pipes close', async () => {
    const root = workspace();
    const pidPath = join(root, 'holder.pid');
    let holderPid: number | undefined;
    const terminateAndVerify = vi.fn().mockImplementation(async () => {
      if (terminateAndVerify.mock.calls.length !== 2) return;
      holderPid = killHolder(pidPath);
    });
    const bridge = runner(root, 10_000, [root], [root], [workspace()], [], {
      ...fakeResourceBoundary, terminateAndVerify,
    });
    const controller = new AbortController();
    const pending = bridge.run(request(root, 'wait-with-held-stdout', {
      args: [fixture, 'wait-with-held-stdout', pidPath],
      signal: controller.signal,
      stdin: undefined,
    }));
    await waitFor(() => existsSync(pidPath));
    const stopped = Date.now();
    controller.abort();
    const result = await pending;

    expect(result.cancelled).toBe(true);
    expect(terminateAndVerify).toHaveBeenCalledTimes(2);
    expect(holderPid).toBeTypeOf('number');
    expect(Date.now() - stopped).toBeLessThan(1_000);
  });

  it('rejects a held-pipe escape and revokes egress without waiting for close', async () => {
    const root = workspace();
    const pidPath = join(root, 'holder.pid');
    const terminateAndVerify = vi.fn().mockImplementation(() => {
      throw new Error('unverifiable scope');
    });
    const complete = vi.fn(() => ({
      allowedConnections: 1,
      deniedConnections: 0,
      connectDigest: digestValue('revoked-connect'),
    }));
    const bridge = runner(
      root, 10_000, [root], [root], [workspace()], [],
      { ...fakeResourceBoundary, terminateAndVerify },
      { ...egressBoundary, complete },
    );
    const controller = new AbortController();
    const pending = bridge.run(request(root, 'wait-with-held-stdout', {
      args: [fixture, 'wait-with-held-stdout', pidPath],
      signal: controller.signal,
      stdin: undefined,
    }));
    await waitFor(() => existsSync(pidPath));
    const stopped = Date.now();
    controller.abort();
    await expect(pending).rejects.toThrow('HARNESS_NATIVE_RESOURCE_TERMINATION_FAILED');

    expect(terminateAndVerify).toHaveBeenCalledTimes(2);
    expect(complete).toHaveBeenCalledTimes(1);
    expect(bridge.allExecutionEvidence()).toEqual([]);
    expect(Date.now() - stopped).toBeLessThan(1_000);
  });

  it('awaits exact scope release and rejects unverifiable termination', async () => {
    const root = workspace();
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const terminateAndVerify = vi.fn()
      .mockImplementationOnce(async () => await gate)
      .mockResolvedValue(undefined);
    const boundary = { ...fakeResourceBoundary, terminateAndVerify };
    const bridge = runner(root, 10_000, [root], [root], [workspace()], [], boundary);
    const pending = bridge.run(request(root, 'wait', { timeoutMs: 50, stdin: undefined }));
    await waitFor(() => terminateAndVerify.mock.calls.length === 1);
    let finished = false;
    void pending.then(() => { finished = true; });
    await new Promise((resolve) => setImmediate(resolve));
    expect(finished).toBe(false);
    release();
    expect((await pending).timedOut).toBe(true);
    expect(terminateAndVerify).toHaveBeenCalledTimes(2);
    expect(terminateAndVerify).toHaveBeenCalledWith(TEST_RESOURCE_SCOPE);

    const rejectedBoundary = {
      ...fakeResourceBoundary,
      terminateAndVerify: async () => {
        await new Promise((resolve) => setImmediate(resolve));
        throw new Error('unverifiable scope');
      },
    };
    const complete = vi.fn(() => ({
      allowedConnections: 1,
      deniedConnections: 0,
      connectDigest: digestValue('failed-connect'),
    }));
    const rejected = runner(
      root, 10_000, [root], [root], [workspace()], [], rejectedBoundary,
      { ...egressBoundary, complete },
    );
    await expect(rejected.run(request(root, 'wait', {
      timeoutMs: 50, stdin: undefined,
    }))).rejects.toThrow('HARNESS_NATIVE_RESOURCE_TERMINATION_FAILED');
    expect(complete).toHaveBeenCalledTimes(1);
    expect(rejected.allExecutionEvidence()).toEqual([]);
  });

  it('requires exact resource termination capability', () => {
    const root = workspace();
    const { terminateAndVerify: _omitted, ...incomplete } = fakeResourceBoundary;
    expect(() => runner(
      root, 10_000, [root], [root], [workspace()], [], incomplete as never,
    )).toThrow('HARNESS_NATIVE_RESOURCE_BOUNDARY_REQUIRED');
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

async function waitFor(predicate: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  throw new Error('condition was not observed');
}

function killHolder(pidPath: string): number {
  const value = readFileSync(pidPath, 'utf8');
  if (!/^[1-9][0-9]*$/.test(value)) throw new Error('invalid holder PID');
  const pid = Number(value);
  if (!Number.isSafeInteger(pid) || pid <= 1) throw new Error('invalid holder PID');
  const command = readFileSync(`/proc/${pid}/cmdline`, 'utf8').split('\0');
  if (!command.includes(fixture) || !command.includes('hold-stdout')) {
    throw new Error('unexpected holder process');
  }
  process.kill(pid, 'SIGKILL');
  return pid;
}
