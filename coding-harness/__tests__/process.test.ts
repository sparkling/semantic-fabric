// SPDX-License-Identifier: MIT

import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { StructuredCommand } from '../src/contracts.js';
import type { OfflineProcessIsolator } from '../src/network.js';
import { runStructuredProcess, sanitizeEnvironment } from '../src/process.js';
import { createTestConfig, TEST_RESOURCE_SCOPE } from './helpers.js';

const temporaryRoots: string[] = [];
const fixture = join(dirname(fileURLToPath(import.meta.url)), 'fixtures/process-fixture.mjs');

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) {
    const pidPath = join(root, 'holder.pid');
    if (existsSync(pidPath)) {
      try { killHolder(pidPath); } catch { /* exited or invalid */ }
    }
    rmSync(root, { recursive: true, force: true });
  }
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
      boundary: { kind: 'trusted-control-plane' },
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
      boundary: { kind: 'trusted-control-plane' },
    })).rejects.toThrow(/shell metacharacter/);
  });

  it('enforces the combined output ceiling', async () => {
    const result = await runStructuredProcess(command('output', { maxOutputBytes: 128 }), {
      workspaceRoot: workspace(),
      config: createTestConfig(),
      declaredTools: ['node'],
      boundary: { kind: 'trusted-control-plane' },
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
      boundary: { kind: 'trusted-control-plane' },
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
      boundary: { kind: 'trusted-control-plane' },
    });
    setTimeout(() => controller.abort(), 50);
    const result = await pending;
    expect(result.success).toBe(false);
    expect(result.cancelled).toBe(true);
    expect(result.durationMs).toBeLessThan(1_000);
  });

  it('does not spawn after cancellation during cooperative isolation', async () => {
    const root = workspace();
    const artifact = join(root, 'must-not-exist.txt');
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const assertStable = vi.fn(async () => await gate);
    const isolator: OfflineProcessIsolator = {
      assertStable,
      async terminateAndVerify() {},
      isolate: (source) => ({
        enforcement: 'os-network-namespace', mechanism: 'test-offline-scope',
        resourceScope: TEST_RESOURCE_SCOPE,
        command: { ...source, executable: '/usr/bin/env', args: [source.executable, ...source.args] },
      }),
    };
    const controller = new AbortController();
    const pending = runStructuredProcess(command('artifact', {
      argv: [fixture, 'artifact', artifact],
    }), {
      workspaceRoot: root, config: createTestConfig(), declaredTools: ['node'],
      signal: controller.signal,
      boundary: { kind: 'offline-candidate', isolator, writablePaths: [] },
    });
    await waitFor(() => assertStable.mock.calls.length === 1);
    controller.abort();
    release();

    const result = await pending;
    expect(result.cancelled).toBe(true);
    expect(existsSync(artifact)).toBe(false);
  });

  it('does not resolve an offline timeout before its exact resource scope is released', async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    const terminateAndVerify = vi.fn()
      .mockImplementationOnce(async () => await gate)
      .mockResolvedValue(undefined);
    const isolator: OfflineProcessIsolator = {
      assertStable() {},
      terminateAndVerify,
      isolate: (source) => ({
        enforcement: 'os-network-namespace',
        mechanism: 'test-offline-scope',
        resourceScope: TEST_RESOURCE_SCOPE,
        command: {
          ...source,
          executable: '/usr/bin/env',
          args: [source.executable, ...source.args],
        },
      }),
    };
    const pending = runStructuredProcess(command('wait', { timeoutMs: 50 }), {
      workspaceRoot: workspace(),
      config: createTestConfig(),
      declaredTools: ['node'],
      boundary: { kind: 'offline-candidate', isolator, writablePaths: [] },
    });
    await waitFor(() => terminateAndVerify.mock.calls.length === 1);
    let finished = false;
    void pending.then(() => { finished = true; });
    await new Promise((resolve) => setImmediate(resolve));
    expect(finished).toBe(false);
    release();
    expect((await pending).timedOut).toBe(true);
    expect(terminateAndVerify).toHaveBeenCalledTimes(2);
    expect(terminateAndVerify).toHaveBeenCalledWith(TEST_RESOURCE_SCOPE);
  });

  it('rechecks a late offline scope on exit before inherited pipes close', async () => {
    const root = workspace();
    const pidPath = join(root, 'holder.pid');
    const terminateAndVerify = vi.fn().mockImplementation(async () => {
      if (terminateAndVerify.mock.calls.length === 2) {
        killHolder(pidPath);
      }
    });
    const isolator: OfflineProcessIsolator = {
      assertStable() {},
      terminateAndVerify,
      isolate: (source) => ({
        enforcement: 'os-network-namespace',
        mechanism: 'test-offline-scope',
        resourceScope: TEST_RESOURCE_SCOPE,
        command: { ...source, executable: '/usr/bin/env', args: [source.executable, ...source.args] },
      }),
    };
    const controller = new AbortController();
    const pending = runStructuredProcess(command('wait-with-held-stdout', {
      argv: [fixture, 'wait-with-held-stdout', pidPath],
    }), {
      workspaceRoot: root,
      config: createTestConfig(),
      declaredTools: ['node'],
      signal: controller.signal,
      boundary: { kind: 'offline-candidate', isolator, writablePaths: [] },
    });
    await waitFor(() => existsSync(pidPath));
    const stopped = Date.now();
    controller.abort();
    const result = await pending;

    expect(result.cancelled).toBe(true);
    expect(terminateAndVerify).toHaveBeenCalledTimes(2);
    expect(Date.now() - stopped).toBeLessThan(1_000);
  });

  it('rejects an offline result when exact scope release cannot be verified', async () => {
    const isolator: OfflineProcessIsolator = {
      assertStable() {},
      async terminateAndVerify() { throw new Error('unverifiable scope'); },
      isolate: (source) => ({
        enforcement: 'os-network-namespace',
        mechanism: 'test-offline-scope',
        resourceScope: TEST_RESOURCE_SCOPE,
        command: { ...source, executable: '/usr/bin/env', args: [source.executable, ...source.args] },
      }),
    };
    await expect(runStructuredProcess(command('wait', { timeoutMs: 50 }), {
      workspaceRoot: workspace(),
      config: createTestConfig(),
      declaredTools: ['node'],
      boundary: { kind: 'offline-candidate', isolator, writablePaths: [] },
    })).rejects.toThrow('HARNESS_OFFLINE_RESOURCE_TERMINATION_FAILED');
  });

  it('rejects a held-pipe offline escape without waiting for close', async () => {
    const root = workspace();
    const pidPath = join(root, 'holder.pid');
    const terminateAndVerify = vi.fn().mockImplementation(() => {
      throw new Error('unverifiable scope');
    });
    const isolator: OfflineProcessIsolator = {
      assertStable() {},
      terminateAndVerify,
      isolate: (source) => ({
        enforcement: 'os-network-namespace',
        mechanism: 'test-offline-scope',
        resourceScope: TEST_RESOURCE_SCOPE,
        command: { ...source, executable: '/usr/bin/env', args: [source.executable, ...source.args] },
      }),
    };
    const controller = new AbortController();
    const pending = runStructuredProcess(command('wait-with-held-stdout', {
      argv: [fixture, 'wait-with-held-stdout', pidPath],
    }), {
      workspaceRoot: root,
      config: createTestConfig(),
      declaredTools: ['node'],
      signal: controller.signal,
      boundary: { kind: 'offline-candidate', isolator, writablePaths: [] },
    });
    await waitFor(() => existsSync(pidPath));
    const stopped = Date.now();
    controller.abort();
    await expect(pending).rejects.toThrow('HARNESS_OFFLINE_RESOURCE_TERMINATION_FAILED');

    expect(terminateAndVerify).toHaveBeenCalledTimes(2);
    expect(Date.now() - stopped).toBeLessThan(1_000);
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
