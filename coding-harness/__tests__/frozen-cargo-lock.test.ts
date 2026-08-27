// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import {
  chmodSync,
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import {
  createStructuredFrozenCargoLockExecutor,
  frozenCargoLockCommandDigest,
  prepareFrozenCargoLock,
  type FrozenCargoLockExecution,
  type FrozenCargoLockExecutor,
} from '../src/frozen-cargo-lock.js';
import { parseFrozenCargoMetadata } from '../src/frozen-cargo-metadata.js';
import type { OfflineProcessIsolator } from '../src/network.js';
import type { ProcessResult } from '../src/process.js';
import type { GitIdentity } from '../src/receipts.js';
import {
  cleanupRequiresAncestorPreservation,
  ParentedResourceCleanupError,
} from '../src/resource-cleanup.js';
import { TEST_RESOURCE_SCOPE } from './helpers.js';

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

const offlineIsolator: OfflineProcessIsolator = Object.freeze({
  assertStable() {},
  async terminateAndVerify() {},
  isolate: (command) => ({
    enforcement: 'os-network-namespace',
    mechanism: 'test-offline-boundary',
    resourceScope: TEST_RESOURCE_SCOPE,
    command: {
      ...command,
      executable: '/usr/bin/env',
      args: [command.executable, ...command.args],
    },
  }),
});

describe('historical frozen Cargo lock preparation', () => {
  it('rejects duplicate resolve-node identities in Cargo metadata', () => {
    const metadata = JSON.stringify({
      workspace_root: '/workspace',
      target_directory: '/target',
      packages: [],
      workspace_members: [],
      resolve: { nodes: [{ id: 'duplicate' }, { id: 'duplicate' }] },
    });

    expect(() => parseFrozenCargoMetadata(metadata, '/workspace', '/target'))
      .toThrow('HARNESS_FROZEN_LOCK_METADATA_NODE_INVALID');
  });

  for (const kind of ['structured-offline', 'native-offline'] as const) {
    it(`prepares and validates a baseline lock through ${kind}`, async () => {
      const fixture = repository();
      const parent = privateRoot('coding-harness-frozen-lock-');
      const currentLock = join(fixture.root, 'Cargo.lock');
      writeFileSync(currentLock, 'current incompatible lock\n');
      const cargo = cargoExecutable();
      const config = SECURE_HARNESS_CONFIG;
      const structured = createStructuredFrozenCargoLockExecutor({
        config,
        offlineIsolator,
        sourceEnvironment: { PATH: process.env.PATH },
      });
      const executor = kind === 'structured-offline' ? structured : nativeAdapter(structured);
      const scratchRoot = join(parent, 'lease');
      const lease = await prepareFrozenCargoLock({
        repositoryRoot: fixture.root,
        scratchRoot,
        baseline: fixture.baseline,
        source: fixture.source,
        cargoExecutable: cargo,
        cargoEnvironment: cargoEnvironment(parent, cargo),
        config,
        executor,
      });

      expect(readFileSync(currentLock, 'utf8')).toBe('current incompatible lock\n');
      expect(lease.lockfile.workspacePath).toBe('Cargo.lock');
      expect(lease.lockfile.digest).toMatch(/^[a-f0-9]{64}$/);
      expect(Object.isFrozen(lease.lockfile)).toBe(true);
      expect(lease.registryPackages).toEqual([]);
      expect(Object.isFrozen(lease.registryPackages)).toBe(true);
      expect(lstatSync(lease.lockfile.sourcePath).mode & 0o222).toBe(0);
      expect(readFileSync(lease.lockfile.sourcePath, 'utf8')).toContain('version = 4');
      expect(lease.baseline).toEqual(fixture.baseline);
      expect(lease.source).toEqual(fixture.source);
      lease.assertStable();

      await lease.cleanup();
      await lease.cleanup();
      expect(existsSync(scratchRoot)).toBe(false);
      expect(existsSync(parent)).toBe(true);
      expect(readFileSync(currentLock, 'utf8')).toBe('current incompatible lock\n');
    }, 15_000);
  }

  it('materializes a frozen lock from the private bare transaction repository', async () => {
    const fixture = repository();
    const parent = privateRoot('coding-harness-frozen-lock-bare-');
    const bare = join(parent, 'repository.git');
    git(fixture.root, ['clone', '--quiet', '--bare', '--no-hardlinks', '--local', '--', fixture.root, bare]);
    const cargo = cargoExecutable();
    const lease = await prepareFrozenCargoLock({
      ...preparationInput({ ...fixture, root: bare }, join(parent, 'lease'),
        createStructuredFrozenCargoLockExecutor({
          config: SECURE_HARNESS_CONFIG,
          offlineIsolator,
          sourceEnvironment: { PATH: process.env.PATH },
        })),
      cargoExecutable: cargo,
      cargoEnvironment: cargoEnvironment(parent, cargo),
    });

    expect(readFileSync(lease.lockfile.sourcePath, 'utf8')).toContain('version = 4');
    await lease.cleanup();
  });

  it('fails before execution when declared identities or scratch paths do not match', async () => {
    const fixture = repository();
    const parent = privateRoot('coding-harness-frozen-lock-identity-');
    const input = preparationInput(fixture, join(parent, 'lease'), inertExecutor());
    await expect(prepareFrozenCargoLock({
      ...input,
      source: { ...fixture.source, tree: 'f'.repeat(40) },
    })).rejects.toThrow(/SOURCE_TREE_MISMATCH/);
    expect(existsSync(join(parent, 'lease'))).toBe(false);

    await expect(prepareFrozenCargoLock({
      ...input,
      scratchRoot: join(fixture.root, 'overlapping-scratch'),
    })).rejects.toThrow(/SCRATCH_OVERLAP/);
  });

  it('rejects a source commit that changes Cargo resolution inputs', async () => {
    const fixture = repository(true);
    const parent = privateRoot('coding-harness-frozen-lock-manifest-');
    await expect(prepareFrozenCargoLock(
      preparationInput(fixture, join(parent, 'lease'), inertExecutor()),
    )).rejects.toThrow(/SOURCE_MANIFEST_MISMATCH/);
    expect(existsSync(join(parent, 'lease'))).toBe(false);
  });

  it('rejects network attestations that are not exactly offline and cleans only its lease', async () => {
    const fixture = repository();
    const parent = privateRoot('coding-harness-frozen-lock-network-');
    const sentinel = join(parent, 'caller-owned.txt');
    writeFileSync(sentinel, 'keep\n');
    const executor: FrozenCargoLockExecutor = {
      kind: 'native-offline',
      execute: async (request) => ({
        kind: 'native-offline',
        network: { mode: 'offline', allowedOrigins: ['https://example.invalid'] },
        commandDigest: frozenCargoLockCommandDigest(request),
        result: successfulResult(''),
      } as unknown as FrozenCargoLockExecution),
    };

    await expect(prepareFrozenCargoLock(
      preparationInput(fixture, join(parent, 'lease'), executor),
    )).rejects.toThrow(/NETWORK_MISMATCH/);
    expect(existsSync(join(parent, 'lease'))).toBe(false);
    expect(readFileSync(sentinel, 'utf8')).toBe('keep\n');
  });

  it('requires successful locked offline Cargo metadata and a stable lock', async () => {
    const fixture = repository();
    const parent = privateRoot('coding-harness-frozen-lock-metadata-');
    const executor: FrozenCargoLockExecutor = {
      kind: 'native-offline',
      execute: async (request) => {
        if (request.command.argv[0] === 'generate-lockfile') {
          writeFileSync(join(request.workspaceRoot, 'Cargo.lock'), 'version = 4\n');
          return attested(request, successfulResult(''));
        }
        expect(request.command.argv).toEqual([
          'metadata', '--locked', '--offline', '--filter-platform',
          'x86_64-unknown-linux-gnu', '--format-version', '1',
        ]);
        return attested(request, failedResult());
      },
    };

    await expect(prepareFrozenCargoLock(
      {
        ...preparationInput(fixture, join(parent, 'lease'), executor),
        targetTriple: 'x86_64-unknown-linux-gnu',
      },
    )).rejects.toThrow(/METADATA_FAILED/);
    expect(existsSync(join(parent, 'lease'))).toBe(false);
  });

  it('rejects an unexpected generated lock before running metadata', async () => {
    const fixture = repository();
    const parent = privateRoot('coding-harness-frozen-lock-digest-');
    const stages: string[] = [];
    const executor: FrozenCargoLockExecutor = {
      kind: 'native-offline',
      execute: async (request) => {
        stages.push(request.command.argv[0] ?? 'missing');
        writeFileSync(join(request.workspaceRoot, 'Cargo.lock'), 'version = 4\n');
        return attested(request, successfulResult(''));
      },
    };

    await expect(prepareFrozenCargoLock({
      ...preparationInput(fixture, join(parent, 'lease'), executor),
      expectedDigest: '0'.repeat(64),
    })).rejects.toThrow(/DIGEST_MISMATCH/);
    expect(stages).toEqual(['generate-lockfile']);
  });

  it('marks a failed preparation cleanup so no ancestor can erase the rejected subtree', async () => {
    const fixture = repository();
    const parent = privateRoot('coding-harness-frozen-lock-preserve-');
    const executor: FrozenCargoLockExecutor = {
      kind: 'native-offline',
      execute: async () => {
        chmodSync(parent, 0o755);
        throw new Error('CARGO_EXECUTION_FAILED');
      },
    };
    let failure: unknown;
    try {
      await prepareFrozenCargoLock(preparationInput(fixture, join(parent, 'lease'), executor));
    } catch (error) {
      failure = error;
    } finally {
      chmodSync(parent, 0o700);
    }

    expect(failure).toBeInstanceOf(ParentedResourceCleanupError);
    expect(cleanupRequiresAncestorPreservation(failure)).toBe(true);
    expect(existsSync(join(parent, 'lease'))).toBe(true);
  });

  it('detects mutation of the immutable returned lock', async () => {
    const fixture = repository();
    const parent = privateRoot('coding-harness-frozen-lock-tamper-');
    const cargo = cargoExecutable();
    const config = SECURE_HARNESS_CONFIG;
    const lease = await prepareFrozenCargoLock({
      ...preparationInput(fixture, join(parent, 'lease'), createStructuredFrozenCargoLockExecutor({
        config,
        offlineIsolator,
        sourceEnvironment: { PATH: process.env.PATH },
      })),
      cargoExecutable: cargo,
      cargoEnvironment: cargoEnvironment(parent, cargo),
      config,
    });
    chmodSync(lease.lockfile.sourcePath, 0o644);
    writeFileSync(lease.lockfile.sourcePath, 'tampered\n');
    expect(() => lease.assertStable()).toThrow(/IMMUTABILITY_INVALID|DIGEST_CHANGED/);
    await lease.cleanup();
    expect(existsSync(parent)).toBe(true);
  });
});

function repository(changeManifest = false): Readonly<{
  root: string;
  baseline: GitIdentity;
  source: GitIdentity;
}> {
  const root = privateRoot('coding-harness-frozen-lock-repo-');
  mkdirSync(join(root, 'src'), { mode: 0o700 });
  writeFileSync(join(root, '.gitignore'), '/Cargo.lock\n');
  writeFileSync(join(root, 'Cargo.toml'), [
    '[package]', 'name = "frozen-lock-fixture"', 'version = "0.1.0"',
    'edition = "2021"', '', '[lib]', 'path = "src/lib.rs"', '',
  ].join('\n'));
  writeFileSync(join(root, 'src/lib.rs'), 'pub fn baseline() -> bool { true }\n');
  git(root, ['init', '--quiet']);
  git(root, ['config', 'user.email', 'harness@example.invalid']);
  git(root, ['config', 'user.name', 'Harness Test']);
  git(root, ['add', '--', '.gitignore', 'Cargo.toml', 'src/lib.rs']);
  git(root, ['commit', '--quiet', '-m', 'baseline']);
  const baseline = identity(root);
  writeFileSync(join(root, 'src/lib.rs'), 'pub fn source() -> bool { true }\n');
  if (changeManifest) {
    writeFileSync(join(root, 'Cargo.toml'), `${readFileSync(join(root, 'Cargo.toml'), 'utf8')}publish = false\n`);
  }
  git(root, ['add', '--', 'Cargo.toml', 'src/lib.rs']);
  git(root, ['commit', '--quiet', '-m', 'source']);
  return Object.freeze({ root, baseline, source: identity(root) });
}

function preparationInput(
  fixture: Readonly<{ root: string; baseline: GitIdentity; source: GitIdentity }>,
  scratchRoot: string,
  executor: FrozenCargoLockExecutor,
) {
  const cargo = cargoExecutable();
  return {
    repositoryRoot: fixture.root,
    scratchRoot,
    baseline: fixture.baseline,
    source: fixture.source,
    cargoExecutable: cargo,
    cargoEnvironment: cargoEnvironment(dirname(scratchRoot), cargo),
    config: SECURE_HARNESS_CONFIG,
    executor,
  };
}

function identity(root: string): GitIdentity {
  return Object.freeze({
    commit: git(root, ['rev-parse', 'HEAD']),
    tree: git(root, ['rev-parse', 'HEAD^{tree}']),
  });
}

function cargoExecutable(): string {
  const located = spawnSync('rustup', ['which', 'cargo'], { encoding: 'utf8' });
  if (located.status !== 0) throw new Error(located.stderr);
  return realpathSync(located.stdout.trim());
}

function cargoEnvironment(parent: string, cargo: string): Record<string, string> {
  const home = join(parent, 'home');
  const cargoHome = join(parent, 'cargo-home');
  mkdirSync(home, { recursive: true, mode: 0o700 });
  mkdirSync(cargoHome, { recursive: true, mode: 0o700 });
  return {
    PATH: `${dirname(cargo)}:/usr/bin`,
    HOME: home,
    CARGO_HOME: cargoHome,
    CARGO_NET_OFFLINE: 'true',
    CARGO_INCREMENTAL: '0',
  };
}

function nativeAdapter(structured: FrozenCargoLockExecutor): FrozenCargoLockExecutor {
  return Object.freeze({
    kind: 'native-offline' as const,
    execute: async (request) => Object.freeze({
      ...await structured.execute(request),
      kind: 'native-offline' as const,
    }),
  });
}

function inertExecutor(): FrozenCargoLockExecutor {
  return Object.freeze({
    kind: 'native-offline' as const,
    execute: async (request) => attested(request, successfulResult('')),
  });
}

function attested(
  request: Parameters<FrozenCargoLockExecutor['execute']>[0],
  result: ProcessResult,
): FrozenCargoLockExecution {
  return Object.freeze({
    kind: 'native-offline',
    network: Object.freeze({ mode: 'offline', allowedOrigins: Object.freeze([]) as [] }),
    commandDigest: frozenCargoLockCommandDigest(request),
    result,
  });
}

function successfulResult(stdout: string): ProcessResult {
  return {
    success: true, exitCode: 0, signal: null, stdout, stderr: '',
    startedAt: '2026-08-25T12:00:00.000Z', durationMs: 1,
    timedOut: false, cancelled: false, outputLimitExceeded: false, spawnError: null,
  };
}

function failedResult(): ProcessResult {
  return { ...successfulResult(''), success: false, exitCode: 101, stderr: 'locked metadata failed' };
}

function git(cwd: string, args: string[]): string {
  const result = spawnSync('/usr/bin/git', args, { cwd, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr);
  return result.stdout.trim();
}

function privateRoot(prefix: string): string {
  const root = mkdtempSync(join(tmpdir(), prefix));
  roots.push(root);
  return root;
}
