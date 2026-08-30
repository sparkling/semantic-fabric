// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  chmodSync,
  existsSync,
  linkSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { afterAll, afterEach, beforeAll, describe, expect, it } from 'vitest';
import { PROGRAMME_CAPTURE_TASK_PROTECTED_PATHS_V1 } from '../src/programme-capture-config-v1.js';
import {
  programmeCaptureRunClaimPathV1,
  reserveProgrammeCaptureRunClaimV1,
  type ProgrammeCaptureRunClaimAuthorityInputV1,
} from '../src/programme-capture-claim-io-v1.js';
import {
  disposeUnusedProgrammeCapturePrivateSourceV1,
  prepareProgrammeCapturePrivateSourceV1,
  verifyProgrammeCapturePrivateSourceV1,
  type ProgrammeCapturePrivateSourceHandleV1,
} from '../src/programme-capture-private-source-v1.js';
import { parsePrivateSourceBlobSizesV1 }
  from '../src/programme-capture-private-source-fs-v1.js';
import {
  PROGRAMME_CAPTURE_OUTPUT_PATH,
  PROGRAMME_CAPTURE_PROFILE_PATH,
  PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS,
  PROGRAMME_CAPTURE_SCENARIOS_PATH,
} from '../src/programme-capture-task-v1.js';

const roots: string[] = [];
const TASK_PATH = 'coding-harness/config/programme-v5-acceptance.json';
const PROJECT_AUTHORITY = '1'.repeat(64);
const RUNNER_IDENTITY = '2'.repeat(64);
const harnessRoot = resolve(import.meta.dirname, '..');
let controller: Readonly<{ root: string; commit: string }>;

beforeAll(() => { controller = controllerRepository(); });
afterAll(() => removeTree(controller.root));
afterEach(() => { for (const root of roots.splice(0)) removeTree(root); });

describe('programme capture V1 private exact-commit source', () => {
  it('materializes, seals, re-verifies, and disposes only the claimed commit', async () => {
    const prepared = await prepare();
    const expected = `private source fixture: ${PROGRAMME_CAPTURE_PROFILE_PATH}\n`;
    writeFileSync(join(controller.root, PROGRAMME_CAPTURE_PROFILE_PATH), 'ambient mutation\n');

    expect(readFileSync(join(prepared.handle.sourceRoot, PROGRAMME_CAPTURE_PROFILE_PATH), 'utf8'))
      .toBe(expected);
    expect(existsSync(join(prepared.handle.sourceRoot, '.git'))).toBe(false);
    expect(existsSync(join(prepared.handle.sourceRoot, PROGRAMME_CAPTURE_OUTPUT_PATH))).toBe(false);
    expect(lstatSync(prepared.handle.sourceRoot).mode & 0o7777).toBe(0o500);
    expect(lstatSync(dirname(prepared.handle.sourceRoot)).mode & 0o7777).toBe(0o500);
    expect(prepared.handle.view).toMatchObject({
      evidenceKind: 'private-source-materialization-view-v1',
      sourceRoot: prepared.handle.sourceRoot,
      outputAbsent: true,
      hostAdmission: 'not-evaluated',
      runnerLeaseAcquired: false,
      attemptStartAuthorized: false,
      captureAuthorized: false,
    });
    expect(prepared.handle.view.fileCount).toBeGreaterThan(100);
    await expect(verifyProgrammeCapturePrivateSourceV1({
      handle: prepared.handle, claimAuthority: prepared.authority,
    })).resolves.toBe(prepared.handle.view);

    await disposeUnusedProgrammeCapturePrivateSourceV1(prepared.handle);
    expect(existsSync(prepared.handle.sourceRoot)).toBe(false);
    await expect(verifyProgrammeCapturePrivateSourceV1({
      handle: prepared.handle, claimAuthority: prepared.authority,
    })).rejects.toThrow(/HANDLE_INVALID/);
  }, 15_000);

  it('works from a bare controller store without consulting a worktree', async () => {
    const bare = temporary('capture-private-source-bare-');
    removeTree(bare);
    git(tmpdir(), ['clone', '--quiet', '--bare', controller.root, bare]);
    hardenGitStore(bare, true);
    const prepared = await prepare({ controllerStore: bare });
    expect(readFileSync(join(prepared.handle.sourceRoot, PROGRAMME_CAPTURE_PROFILE_PATH), 'utf8'))
      .toBe(`private source fixture: ${PROGRAMME_CAPTURE_PROFILE_PATH}\n`);
    await disposeUnusedProgrammeCapturePrivateSourceV1(prepared.handle);
  }, 15_000);

  it('makes the claim-keyed materialization slot create-new', async () => {
    const prepared = await prepare();
    await expect(prepareProgrammeCapturePrivateSourceV1({
      claimAuthority: prepared.authority,
      runtimeParent: prepared.runtimeParent,
    })).rejects.toThrow(/PRIVATE_SOURCE_SPENT/);
  });

  it('keeps the source root outside claim and controller authority', async () => {
    const authority = claimInput();
    await reserveProgrammeCaptureRunClaimV1(authority);
    await expect(prepareProgrammeCapturePrivateSourceV1({
      claimAuthority: authority, runtimeParent: authority.authorityRoot,
    })).rejects.toThrow(/PLACEMENT_INVALID/);
    await expect(prepareProgrammeCapturePrivateSourceV1({
      claimAuthority: authority, runtimeParent: authority.controllerStore,
    })).rejects.toThrow(/PLACEMENT_INVALID/);
  });

  it.each([
    ['tracked bytes', (item: Prepared) => mutateFile(item, PROGRAMME_CAPTURE_PROFILE_PATH)],
    ['tracked mode', (item: Prepared) => chmodSync(
      join(item.handle.sourceRoot, PROGRAMME_CAPTURE_PROFILE_PATH), 0o600,
    )],
    ['extra file', (item: Prepared) => addFile(item, 'extra.txt')],
    ['extra directory', (item: Prepared) => addDirectory(item, 'extra-dir')],
    ['output injection', (item: Prepared) => addFile(item, PROGRAMME_CAPTURE_OUTPUT_PATH)],
    ['symlink', (item: Prepared) => replaceWithSymlink(item, 'Cargo.toml')],
    ['hardlink', (item: Prepared) => replaceWithHardlink(item, 'Cargo.toml')],
    ['index bytes', (item: Prepared) => mutateIndex(item)],
    ['root entries', (item: Prepared) => addRootEntry(item)],
    ['root replacement', (item: Prepared) => replaceRoot(item)],
  ] as const)('rejects %s mutation and preserves the poisoned tree', async (_label, mutate) => {
    const prepared = await prepare();
    mutate(prepared);
    await expect(verifyProgrammeCapturePrivateSourceV1({
      handle: prepared.handle, claimAuthority: prepared.authority,
    })).rejects.toThrow(/PRIVATE_SOURCE/);
    await expect(disposeUnusedProgrammeCapturePrivateSourceV1(prepared.handle))
      .rejects.toThrow(/PRIVATE_SOURCE/);
    expect(existsSync(dirname(prepared.handle.sourceRoot))).toBe(true);
  });

  it('rejects missing, replaced, and caller-forged claim authority', async () => {
    const missing = await prepare();
    rmSync(claimPath(missing.authority));
    await expect(verifyProgrammeCapturePrivateSourceV1({
      handle: missing.handle, claimAuthority: missing.authority,
    })).rejects.toThrow(/CLAIM_MISSING/);

    const replaced = await prepare();
    rmSync(claimPath(replaced.authority));
    await reserveProgrammeCaptureRunClaimV1({
      ...replaced.authority, expectedRunnerIdentityDigest: '3'.repeat(64),
    });
    await expect(verifyProgrammeCapturePrivateSourceV1({
      handle: replaced.handle, claimAuthority: replaced.authority,
    })).rejects.toThrow(/AUTHORITY_MISMATCH/);

    const forged = await prepare();
    await expect(verifyProgrammeCapturePrivateSourceV1({
      handle: { sourceRoot: forged.handle.sourceRoot, view: forged.handle.view },
      claimAuthority: forged.authority,
    })).rejects.toThrow(/HANDLE_INVALID/);
    await expect(verifyProgrammeCapturePrivateSourceV1({
      handle: forged.handle,
      claimAuthority: { ...forged.authority, runId: 'capture_private_source_forged' },
    })).rejects.toThrow(/AUTHORITY_MISMATCH/);

    let getterCalled = false;
    const accessorAuthority = { ...forged.authority };
    Object.defineProperty(accessorAuthority, 'runId', {
      enumerable: true,
      get: () => { getterCalled = true; return forged.authority.runId; },
    });
    await expect(verifyProgrammeCapturePrivateSourceV1({
      handle: forged.handle, claimAuthority: accessorAuthority,
    })).rejects.toThrow(/plain own-key object/);
    expect(getterCalled).toBe(false);
  }, 15_000);

  it('rejects unsupported commit symlinks before checkout', async () => {
    const root = cloneController('capture-private-source-symlink-');
    symlinkSync('Cargo.toml', join(root, 'unsafe-link'));
    git(root, ['add', '--all']);
    git(root, ['commit', '--quiet', '-m', 'add unsupported symlink'], identityEnvironment());
    hardenGitStore(root);
    const authority = claimInput({
      controllerStore: root,
      controllerCommit: git(root, ['rev-parse', 'HEAD']).trim(),
    });
    await reserveProgrammeCaptureRunClaimV1(authority);
    await expect(prepareProgrammeCapturePrivateSourceV1({
      claimAuthority: authority, runtimeParent: runtimeParent(),
    })).rejects.toThrow(/TREE_ENTRY_UNSUPPORTED/);
  });

  it.each([
    ['primary filter', false, 'filter'],
    ['bare filter', true, 'filter'],
    ['tracked attributes', false, 'tracked'],
    ['info attributes', false, 'info'],
  ] as const)('rejects %s authority before checkout', async (_label, bare, capability) => {
    const root = cloneController('capture-private-source-capability-');
    const commit = git(root, ['rev-parse', 'HEAD']).trim();
    let store = root;
    if (bare) {
      store = temporary('capture-private-source-capability-bare-');
      removeTree(store);
      git(tmpdir(), ['clone', '--quiet', '--bare', root, store]);
      hardenGitStore(store, true);
    }
    const authority = claimInput({ controllerStore: store, controllerCommit: commit });
    await reserveProgrammeCaptureRunClaimV1(authority);
    const marker = join(temporary('capture-private-source-filter-marker-'), 'ran');
    if (capability === 'filter') {
      git(store, ['config', 'filter.capture-side-effect.smudge',
        `/bin/sh -c 'touch ${marker}; cat'`]);
      writeFixture(bare ? store : join(store, '.git'),
        'info/attributes', '* filter=capture-side-effect\n');
    } else if (capability === 'tracked') {
      writeFixture(store, '.gitattributes', '* filter=ambient\n');
      git(store, ['add', '--', '.gitattributes']);
      hardenGitStore(store, bare);
    } else if (capability === 'info') {
      writeFixture(bare ? store : join(store, '.git'), 'info/attributes', '* filter=ambient\n');
    }
    await expect(prepareProgrammeCapturePrivateSourceV1({
      claimAuthority: authority, runtimeParent: runtimeParent(),
    })).rejects.toThrow(/MATERIALIZATION|ATTRIBUTES/);
    expect(existsSync(marker)).toBe(false);
  });

  it('preflights exact, oversized, and repeated-path blob sizes before writes', () => {
    const object = 'a'.repeat(40);
    const entry = (path: string) => ({ mode: '100644', type: 'blob', gitObjectId: object, path });
    expect(parsePrivateSourceBlobSizesV1(
      Buffer.from(`${object} blob 5000000000\n`), [entry('exact')],
    )).toBe(5_000_000_000);
    expect(() => parsePrivateSourceBlobSizesV1(
      Buffer.from(`${object} blob 5000000001\n`), [entry('oversized')],
    )).toThrow(/BYTE_LIMIT/);
    expect(() => parsePrivateSourceBlobSizesV1(Buffer.from(
      `${object} blob 2500000001\n${object} blob 2500000001\n`,
    ), [entry('first'), entry('second')])).toThrow(/BYTE_LIMIT/);
  });

  it('keeps private handle state opaque under WeakMap prototype replacement', async () => {
    const originalDelete = WeakMap.prototype.delete;
    const originalGet = WeakMap.prototype.get;
    const originalSet = WeakMap.prototype.set;
    let leaked: unknown;
    let forgedRejected = false;
    try {
      WeakMap.prototype.set = function (_key, value) { leaked = value; return this; };
      WeakMap.prototype.get = function () { return leaked; };
      WeakMap.prototype.delete = function () { return true; };
      const prepared = await prepare();
      try {
        await verifyProgrammeCapturePrivateSourceV1({
          handle: { ...prepared.handle }, claimAuthority: prepared.authority,
        });
      } catch (error) {
        forgedRejected = /HANDLE_INVALID/.test(String(error));
      }
      await disposeUnusedProgrammeCapturePrivateSourceV1(prepared.handle);
    } finally {
      WeakMap.prototype.delete = originalDelete;
      WeakMap.prototype.get = originalGet;
      WeakMap.prototype.set = originalSet;
    }
    expect(leaked).toBeUndefined();
    expect(forgedRejected).toBe(true);
  });

  it('contains only bounded Git materialization and cleanup capability', () => {
    const sources = [
      'src/programme-capture-private-source-v1.ts',
      'src/programme-capture-private-source-fs-v1.ts',
    ].map((path) => readFileSync(resolve(harnessRoot, path), 'utf8')).join('\n');
    expect(sources).toContain("['read-tree'");
    expect(sources).toContain("'checkout-index'");
    expect(sources).toContain("['ls-files', '--stage'");
    expect(sources).not.toMatch(
      /node:(?:child_process|net|http|https|tls|dgram|worker_threads)|native-client|provider|cargo|rustc|npm/,
    );
    expect(sources).not.toMatch(/capture-baseline|check-baseline|receipt_file|release|retry/);
  });
});

interface Prepared {
  readonly authority: ProgrammeCaptureRunClaimAuthorityInputV1;
  readonly runtimeParent: string;
  readonly handle: ProgrammeCapturePrivateSourceHandleV1;
}

async function prepare(
  override: Partial<ProgrammeCaptureRunClaimAuthorityInputV1> = {},
): Promise<Prepared> {
  const authority = claimInput(override);
  await reserveProgrammeCaptureRunClaimV1(authority);
  const parent = runtimeParent();
  const handle = await prepareProgrammeCapturePrivateSourceV1({
    claimAuthority: authority, runtimeParent: parent,
  });
  return { authority, runtimeParent: parent, handle };
}

function claimInput(
  override: Partial<ProgrammeCaptureRunClaimAuthorityInputV1> = {},
): ProgrammeCaptureRunClaimAuthorityInputV1 {
  return {
    authorityRoot: authorityRoot(),
    projectAuthorityDigest: PROJECT_AUTHORITY,
    runId: 'capture_private_source_20260828_0001',
    controllerStore: controller.root,
    controllerCommit: controller.commit,
    taskPath: TASK_PATH,
    expectedRunnerIdentityDigest: RUNNER_IDENTITY,
    ...override,
  };
}

function claimPath(authority: ProgrammeCaptureRunClaimAuthorityInputV1): string {
  return programmeCaptureRunClaimPathV1({
    authorityRoot: authority.authorityRoot,
    projectAuthorityDigest: authority.projectAuthorityDigest,
    runId: authority.runId,
  });
}

function controllerRepository(): Readonly<{ root: string; commit: string }> {
  const root = mkdtempSync(join(tmpdir(), 'capture-private-source-controller-'));
  const values = new Map<string, Buffer>();
  for (const path of PROGRAMME_CAPTURE_TASK_PROTECTED_PATHS_V1) {
    values.set(path, Buffer.from(`private source fixture: ${path}\n`));
  }
  const binding = (path: string) => ({ path, sha256: sha256(values.get(path)!) });
  const task = captureTask(binding);
  writeFixture(root, 'coding-harness/.harness/manifest.json', readFileSync(
    new URL('../.harness/manifest.json', import.meta.url),
  ));
  writeFixture(root, TASK_PATH, `${JSON.stringify(task)}\n`);
  for (const [path, bytes] of values) writeFixture(root, path, bytes);
  writeFixture(root, 'tools/executable', '#!/bin/false\n');
  chmodSync(join(root, 'tools/executable'), 0o755);
  git(root, ['init', '--quiet']);
  git(root, ['add', '--all']);
  git(root, ['commit', '--quiet', '-m', 'capture source fixture'], identityEnvironment());
  hardenGitStore(root);
  return { root, commit: git(root, ['rev-parse', 'HEAD']).trim() };
}

function captureTask(binding: (path: string) => { path: string; sha256: string }) {
  const command = (argv: string, timeoutMs: number) => ({
    commandId: `${argv}_0001`,
    command: {
      tool: 'sf-performance-receipt', executable: 'target/release/sf-performance-receipt',
      argv: [argv], cwd: '.', env: {}, timeoutMs, maxOutputBytes: 1_048_576,
    },
  });
  return {
    schemaVersion: 1, taskKind: 'controlled-performance-baseline',
    taskId: 'capture_private_source_20260828',
    workItem: 'completion-programme:m0-performance-baseline',
    objective: 'Materialize one exact claimed commit without authorizing execution.',
    invariants: ['Source bytes come only from the pinned controller commit.'],
    exclusions: ['No runner, attempt, build, capture, or receipt is authorized.'],
    authority: 'development-only-no-promotion',
    inputs: {
      runnerProfile: binding(PROGRAMME_CAPTURE_PROFILE_PATH),
      scenarios: binding(PROGRAMME_CAPTURE_SCENARIOS_PATH),
      cargoLock: binding('Cargo.lock'), workloadSha256: 'd'.repeat(64),
      sources: PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS.map(binding),
    },
    commands: { capture: command('capture-baseline', 1_800_000), verify: command('check-baseline', 60_000) },
    output: {
      path: PROGRAMME_CAPTURE_OUTPUT_PATH, mode: 'create-new',
      mediaType: 'text/tab-separated-values; charset=utf-8', maximumBytes: 1_048_576,
    },
    policy: {
      measurementNetwork: 'offline', modelTransport: 'native-first-party-only',
      nativeHosts: ['codex', 'claude-code'], dualReview: { preCapture: true, postCapture: true },
      maximumMeasurementAttempts: 1, automaticMeasurementRetries: 0, automaticRepairs: 0,
      modelMeasurementOverlap: 'forbidden', coreEvidence: 'fail-closed',
    },
    routing: { tags: ['controlled-capture', 'performance'], difficulty: 1, evolutionEligible: false },
  };
}

function mutateFile(item: Prepared, path: string): void {
  const target = join(item.handle.sourceRoot, path);
  chmodSync(target, 0o600);
  writeFileSync(target, 'mutated\n');
}
function addFile(item: Prepared, path: string): void {
  const target = join(item.handle.sourceRoot, path);
  makeWritable(dirname(target), item.handle.sourceRoot);
  mkdirSync(dirname(target), { recursive: true, mode: 0o700 });
  writeFileSync(target, 'injected\n');
}
function addDirectory(item: Prepared, path: string): void {
  chmodSync(item.handle.sourceRoot, 0o700);
  mkdirSync(join(item.handle.sourceRoot, path), { mode: 0o700 });
}
function replaceWithSymlink(item: Prepared, path: string): void {
  chmodSync(item.handle.sourceRoot, 0o700);
  rmSync(join(item.handle.sourceRoot, path));
  symlinkSync(PROGRAMME_CAPTURE_PROFILE_PATH, join(item.handle.sourceRoot, path));
}
function replaceWithHardlink(item: Prepared, path: string): void {
  chmodSync(item.handle.sourceRoot, 0o700);
  rmSync(join(item.handle.sourceRoot, path));
  linkSync(
    join(item.handle.sourceRoot, PROGRAMME_CAPTURE_PROFILE_PATH),
    join(item.handle.sourceRoot, path),
  );
}
function mutateIndex(item: Prepared): void {
  const index = join(dirname(item.handle.sourceRoot), 'index');
  chmodSync(index, 0o600);
  writeFileSync(index, 'forged index\n');
}
function addRootEntry(item: Prepared): void {
  const root = dirname(item.handle.sourceRoot);
  chmodSync(root, 0o700);
  writeFileSync(join(root, 'extra'), 'injected\n');
}
function replaceRoot(item: Prepared): void {
  const root = dirname(item.handle.sourceRoot);
  renameSync(root, `${root}.replaced`);
  mkdirSync(root, { mode: 0o500 });
}

function makeWritable(path: string, sourceRoot: string): void {
  let current = path;
  while (current.startsWith(sourceRoot)) {
    chmodSync(current, 0o700);
    if (current === sourceRoot) return;
    current = dirname(current);
  }
}
function runtimeParent(): string { const root = temporary('capture-private-runtime-'); chmodSync(root, 0o700); return root; }
function authorityRoot(): string { const root = temporary('capture-private-authority-'); chmodSync(root, 0o700); return root; }
function temporary(prefix: string): string { const root = mkdtempSync(join(tmpdir(), prefix)); roots.push(root); return root; }
function cloneController(prefix: string): string {
  const root = temporary(prefix);
  removeTree(root);
  git(tmpdir(), ['clone', '--quiet', controller.root, root]);
  chmodSync(root, 0o755); hardenGitStore(root);
  return root;
}
function hardenGitStore(root: string, bare = false): void {
  const gitRoot = bare ? root : join(root, '.git');
  const visit = (path: string): void => {
    const stat = lstatSync(path); chmodSync(path, stat.mode & ~0o022);
    if (stat.isDirectory() && !stat.isSymbolicLink()) {
      for (const entry of readdirSync(path)) visit(join(path, entry));
    }
  };
  visit(gitRoot);
}
function writeFixture(root: string, path: string, value: string | Buffer): void {
  const target = join(root, path); mkdirSync(dirname(target), { recursive: true }); writeFileSync(target, value);
}
function git(cwd: string, args: readonly string[], env = process.env): string {
  const result = spawnSync('/usr/bin/git', args, { cwd, env, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr); return result.stdout;
}
function identityEnvironment(): NodeJS.ProcessEnv {
  return {
    ...process.env, GIT_AUTHOR_NAME: 'Harness Test', GIT_AUTHOR_EMAIL: 'harness@example.invalid',
    GIT_AUTHOR_DATE: '2000-01-01T00:00:00Z', GIT_COMMITTER_NAME: 'Harness Test',
    GIT_COMMITTER_EMAIL: 'harness@example.invalid', GIT_COMMITTER_DATE: '2000-01-01T00:00:00Z',
  };
}
function sha256(value: Buffer): string { return createHash('sha256').update(value).digest('hex'); }
function removeTree(root: string): void {
  if (!existsSync(root)) return;
  const visit = (path: string): void => {
    const stat = lstatSync(path);
    if (!stat.isDirectory() || stat.isSymbolicLink()) return;
    chmodSync(path, 0o700);
    for (const entry of readdirSync(path, { withFileTypes: true })) {
      if (entry.isDirectory() && !entry.isSymbolicLink()) visit(join(path, entry.name));
    }
  };
  visit(root); rmSync(root, { recursive: true, force: true });
}
