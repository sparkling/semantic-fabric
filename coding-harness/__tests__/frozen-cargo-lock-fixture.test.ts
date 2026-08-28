// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import {
  existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, unlinkSync, writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';
import { afterEach, describe, expect, it } from 'vitest';
import {
  decodeFrozenCargoLockFixture,
  ISSUE_8_FROZEN_LOCK_FIXTURE_PATH,
  installPinnedCargoLock,
  LEGACY_FROZEN_LOCK_TASK_PATHS,
  readIssue8FrozenCargoLock,
  readTaskFrozenCargoLock,
} from '../src/frozen-cargo-lock-fixture.js';

const roots: string[] = [];
const EXPECTED_DIGEST = '72916782d4d8fb87b613f61debe2107c160e083ef4969c89c23c7596df5b637d';

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('controller-bound frozen Cargo lock fixture', () => {
  it('decodes the recovered historical lock bytes exactly', () => {
    const harnessRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
    const serialized = readFileSync(resolve(harnessRoot, 'config',
      'issue-8-baseline-Cargo.lock.gz.b64'), 'utf8');
    const lockfile = decodeFrozenCargoLockFixture(serialized, EXPECTED_DIGEST);

    expect(createHash('sha256').update(lockfile).digest('hex')).toBe(EXPECTED_DIGEST);
    expect(Buffer.byteLength(lockfile, 'utf8')).toBe(158_007);
    expect(lockfile).toContain('name = "combine"\nversion = "4.6.7"');
    expect(lockfile).toContain('name = "rand"\nversion = "0.8.7"');
  });

  it('reads only the exact committed blob, independent of working-tree mutation', async () => {
    const expected = '# lock fixture\nversion = 4\n';
    const fixture = gitFixture(expected);
    writeFileSync(fixture.path, `${gzipSync('tampered\n').toString('base64')}\n`);

    expect(await readIssue8FrozenCargoLock({
      controllerRepositoryRoot: fixture.root,
      controllerCommit: fixture.commit,
      expectedDigest: createHash('sha256').update(expected).digest('hex'),
    })).toBe(expected);
  });

  it('reads a task-bound tracked baseline lock as exact binary-safe bytes', async () => {
    const expected = '# tracked lock\nversion = 4\n';
    const fixture = trackedLockFixture(Buffer.from(expected));
    writeFileSync(fixture.path, 'working tree substitution\n');

    expect(await readTaskFrozenCargoLock({
      controllerRepositoryRoot: fixture.root,
      controllerCommit: fixture.commit,
      taskPath: 'coding-harness/config/m0-artifact-acceptance.json',
      baselineCommit: fixture.commit,
      expectedDigest: createHash('sha256').update(expected).digest('hex'),
    })).toBe(expected);
  });

  it('reads Cargo.lock from the baseline commit rather than a descendant controller', async () => {
    const expected = '# baseline lock\nversion = 4\n';
    const fixture = trackedLockFixture(Buffer.from(expected));
    writeFileSync(fixture.path, '# controller replacement\nversion = 4\n');
    git(fixture.root, ['add', '--', 'Cargo.lock']);
    git(fixture.root, ['commit', '--quiet', '-m', 'replace controller lock']);

    expect(await readTaskFrozenCargoLock({
      controllerRepositoryRoot: fixture.root,
      controllerCommit: git(fixture.root, ['rev-parse', 'HEAD']),
      taskPath: 'coding-harness/config/m0-artifact-acceptance.json',
      baselineCommit: fixture.commit,
      expectedDigest: createHash('sha256').update(expected).digest('hex'),
    })).toBe(expected);
  });

  it('fails closed on tracked blob type, size, digest, and UTF-8 violations', async () => {
    const tree = trackedLockTreeFixture();
    await expect(readTaskFrozenCargoLock({
      controllerRepositoryRoot: tree.root,
      controllerCommit: tree.commit,
      taskPath: 'coding-harness/config/m0-artifact-acceptance.json',
      baselineCommit: tree.commit,
      expectedDigest: 'a'.repeat(64),
    })).rejects.toThrow('HARNESS_FROZEN_LOCK_BASELINE_BLOB_INVALID');

    const oversized = trackedLockFixture(Buffer.alloc(10_000_001, 0x61));
    await expect(readTaskFrozenCargoLock({
      controllerRepositoryRoot: oversized.root,
      controllerCommit: oversized.commit,
      taskPath: 'coding-harness/config/m0-artifact-acceptance.json',
      baselineCommit: oversized.commit,
      expectedDigest: createHash('sha256').update(Buffer.alloc(10_000_001, 0x61)).digest('hex'),
    })).rejects.toThrow('HARNESS_FROZEN_LOCK_BASELINE_SIZE_INVALID');

    const ordinary = trackedLockFixture(Buffer.from('version = 4\n'));
    await expect(readTaskFrozenCargoLock({
      controllerRepositoryRoot: ordinary.root,
      controllerCommit: ordinary.commit,
      taskPath: 'coding-harness/config/m0-artifact-acceptance.json',
      baselineCommit: ordinary.commit,
      expectedDigest: 'b'.repeat(64),
    })).rejects.toThrow('HARNESS_FROZEN_LOCK_BASELINE_DIGEST_MISMATCH');

    const invalidUtf8 = trackedLockFixture(Buffer.from([0xff, 0x0a]));
    await expect(readTaskFrozenCargoLock({
      controllerRepositoryRoot: invalidUtf8.root,
      controllerCommit: invalidUtf8.commit,
      taskPath: 'coding-harness/config/m0-artifact-acceptance.json',
      baselineCommit: invalidUtf8.commit,
      expectedDigest: createHash('sha256').update(Buffer.from([0xff, 0x0a])).digest('hex'),
    })).rejects.toThrow('HARNESS_FROZEN_LOCK_BASELINE_UTF8_INVALID');

    const nul = trackedLockFixture(Buffer.from('version = 4\n\0'));
    await expect(readTaskFrozenCargoLock({
      controllerRepositoryRoot: nul.root,
      controllerCommit: nul.commit,
      taskPath: 'coding-harness/config/m0-artifact-acceptance.json',
      baselineCommit: nul.commit,
      expectedDigest: createHash('sha256').update(Buffer.from('version = 4\n\0')).digest('hex'),
    })).rejects.toThrow('HARNESS_FROZEN_LOCK_BASELINE_UTF8_INVALID');
  });

  it('permits only the exact historical task bindings to use the legacy fixture', async () => {
    const harnessRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
    const repositoryRoot = resolve(harnessRoot, '..');
    const controllerCommit = git(repositoryRoot, ['rev-parse', 'HEAD']);
    for (const taskPath of LEGACY_FROZEN_LOCK_TASK_PATHS) {
      const lockfile = await readTaskFrozenCargoLock({
        controllerRepositoryRoot: repositoryRoot,
        controllerCommit,
        taskPath,
        baselineCommit: 'd510fc952a8dc701d65b1a4f3ad25a8109b98669',
        expectedDigest: EXPECTED_DIGEST,
      });
      expect(createHash('sha256').update(lockfile).digest('hex')).toBe(EXPECTED_DIGEST);
    }
    await expect(readTaskFrozenCargoLock({
      controllerRepositoryRoot: repositoryRoot,
      controllerCommit,
      taskPath: 'coding-harness/config/programme-v5-copy-acceptance.json',
      baselineCommit: 'd510fc952a8dc701d65b1a4f3ad25a8109b98669',
      expectedDigest: EXPECTED_DIGEST,
    })).rejects.toThrow('HARNESS_FROZEN_LOCK_BASELINE_BLOB_INVALID');
  });

  it('rejects a missing tracked baseline lock for every non-legacy task', async () => {
    const fixture = gitFixture('version = 4\n');
    await expect(readTaskFrozenCargoLock({
      controllerRepositoryRoot: fixture.root,
      controllerCommit: fixture.commit,
      taskPath: 'coding-harness/config/m0-artifact-acceptance.json',
      baselineCommit: fixture.commit,
      expectedDigest: EXPECTED_DIGEST,
    })).rejects.toThrow('HARNESS_FROZEN_LOCK_BASELINE_BLOB_INVALID');
  });

  it('rejects missing, unrelated, and corrupt baseline objects before legacy fallback', async () => {
    const fixture = gitFixture('version = 4\n');
    await expect(readTaskFrozenCargoLock({
      controllerRepositoryRoot: fixture.root,
      controllerCommit: fixture.commit,
      taskPath: LEGACY_FROZEN_LOCK_TASK_PATHS[0]!,
      baselineCommit: 'f'.repeat(40),
      expectedDigest: EXPECTED_DIGEST,
    })).rejects.toThrow('HARNESS_FROZEN_LOCK_TASK_BINDING_INVALID');

    const unrelated = unrelatedCommitFixture();
    await expect(readTaskFrozenCargoLock({
      controllerRepositoryRoot: unrelated.root,
      controllerCommit: unrelated.controllerCommit,
      taskPath: LEGACY_FROZEN_LOCK_TASK_PATHS[0]!,
      baselineCommit: unrelated.baselineCommit,
      expectedDigest: EXPECTED_DIGEST,
    })).rejects.toThrow('HARNESS_FROZEN_LOCK_TASK_BINDING_INVALID');

    const corrupt = trackedLockFixture(Buffer.from('version = 4\n'));
    const blob = git(corrupt.root, ['rev-parse', `${corrupt.commit}:Cargo.lock`]);
    unlinkSync(join(corrupt.root, '.git', 'objects', blob.slice(0, 2), blob.slice(2)));
    await expect(readTaskFrozenCargoLock({
      controllerRepositoryRoot: corrupt.root,
      controllerCommit: corrupt.commit,
      taskPath: LEGACY_FROZEN_LOCK_TASK_PATHS[0]!,
      baselineCommit: corrupt.commit,
      expectedDigest: EXPECTED_DIGEST,
    })).rejects.toThrow('HARNESS_FROZEN_LOCK_BASELINE_BLOB_INVALID');
  });

  it('rejects a future replacement at an allowlisted legacy task path', async () => {
    const harnessRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
    const repositoryRoot = resolve(harnessRoot, '..');
    const clone = clonedRepositoryFixture(repositoryRoot);
    const taskPath = LEGACY_FROZEN_LOCK_TASK_PATHS[0]!;
    writeFileSync(join(clone.root, taskPath), '{"replacement":true}\n');
    git(clone.root, ['add', '--', taskPath]);
    git(clone.root, ['commit', '--quiet', '-m', 'replace legacy task']);

    await expect(readTaskFrozenCargoLock({
      controllerRepositoryRoot: clone.root,
      controllerCommit: git(clone.root, ['rev-parse', 'HEAD']),
      taskPath,
      baselineCommit: 'd510fc952a8dc701d65b1a4f3ad25a8109b98669',
      expectedDigest: EXPECTED_DIGEST,
    })).rejects.toThrow('HARNESS_FROZEN_LOCK_LEGACY_TASK_BLOB_INVALID');
  });

  it('rejects noncanonical encoding and a mismatched lock digest', async () => {
    expect(() => decodeFrozenCargoLockFixture('not-base64\n', 'a'.repeat(64)))
      .toThrow('HARNESS_FROZEN_LOCK_FIXTURE_INVALID');
    const fixture = gitFixture('version = 4\n');
    await expect(readIssue8FrozenCargoLock({
      controllerRepositoryRoot: fixture.root,
      controllerCommit: fixture.commit,
      expectedDigest: 'a'.repeat(64),
    })).rejects.toThrow('HARNESS_FROZEN_LOCK_FIXTURE_DIGEST_MISMATCH');
  });

  it('rejects missing and non-blob controller objects', async () => {
    const fixture = gitFixture('version = 4\n');
    unlinkSync(fixture.path);
    git(fixture.root, ['add', '--update', '--', ISSUE_8_FROZEN_LOCK_FIXTURE_PATH]);
    git(fixture.root, ['commit', '--quiet', '-m', 'remove fixture']);
    await expect(readIssue8FrozenCargoLock({
      controllerRepositoryRoot: fixture.root,
      controllerCommit: git(fixture.root, ['rev-parse', 'HEAD']),
      expectedDigest: 'a'.repeat(64),
    })).rejects.toThrow('HARNESS_FROZEN_LOCK_FIXTURE_BLOB_INVALID');

    mkdirSync(fixture.path, { mode: 0o700 });
    writeFileSync(join(fixture.path, 'entry'), 'tree\n');
    git(fixture.root, ['add', '--', ISSUE_8_FROZEN_LOCK_FIXTURE_PATH]);
    git(fixture.root, ['commit', '--quiet', '-m', 'replace fixture with tree']);
    await expect(readIssue8FrozenCargoLock({
      controllerRepositoryRoot: fixture.root,
      controllerCommit: git(fixture.root, ['rev-parse', 'HEAD']),
      expectedDigest: 'a'.repeat(64),
    })).rejects.toThrow('HARNESS_FROZEN_LOCK_FIXTURE_BLOB_INVALID');
  });

  it('rejects invalid gzip data, decompression overflow, and missing runtime contents', async () => {
    const invalidGzip = `${Buffer.from('not gzip').toString('base64')}\n`;
    expect(() => decodeFrozenCargoLockFixture(invalidGzip, 'a'.repeat(64)))
      .toThrow('HARNESS_FROZEN_LOCK_FIXTURE_INVALID');
    const oversized = `${gzipSync(Buffer.alloc(10_000_001, 0x61)).toString('base64')}\n`;
    expect(() => decodeFrozenCargoLockFixture(oversized, 'a'.repeat(64)))
      .toThrow('HARNESS_FROZEN_LOCK_FIXTURE_INVALID');

    const workspace = mkdtempSync(join(tmpdir(), 'coding-harness-lock-install-'));
    roots.push(workspace);
    await expect(installPinnedCargoLock(
      workspace, undefined as unknown as string, 'a'.repeat(64),
    )).rejects.toThrow('HARNESS_FROZEN_LOCK_FIXTURE_INVALID');
    expect(existsSync(join(workspace, 'Cargo.lock'))).toBe(false);
  });
});

function gitFixture(contents: string): Readonly<{ root: string; path: string; commit: string }> {
  const root = mkdtempSync(join(tmpdir(), 'coding-harness-lock-fixture-'));
  roots.push(root);
  const path = join(root, ISSUE_8_FROZEN_LOCK_FIXTURE_PATH);
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  writeFileSync(path, `${gzipSync(contents).toString('base64')}\n`);
  git(root, ['init', '--quiet']);
  git(root, ['config', 'user.email', 'harness@example.invalid']);
  git(root, ['config', 'user.name', 'Harness Test']);
  git(root, ['add', '--', ISSUE_8_FROZEN_LOCK_FIXTURE_PATH]);
  git(root, ['commit', '--quiet', '-m', 'fixture']);
  return Object.freeze({ root, path, commit: git(root, ['rev-parse', 'HEAD']) });
}

function trackedLockFixture(
  contents: Buffer,
): Readonly<{ root: string; path: string; commit: string }> {
  const root = initializedGitRoot('coding-harness-tracked-lock-');
  const path = join(root, 'Cargo.lock');
  writeFileSync(path, contents);
  git(root, ['add', '--', 'Cargo.lock']);
  git(root, ['commit', '--quiet', '-m', 'tracked lock']);
  return Object.freeze({ root, path, commit: git(root, ['rev-parse', 'HEAD']) });
}

function trackedLockTreeFixture(): Readonly<{ root: string; commit: string }> {
  const root = initializedGitRoot('coding-harness-lock-tree-');
  mkdirSync(join(root, 'Cargo.lock'));
  writeFileSync(join(root, 'Cargo.lock', 'entry'), 'tree\n');
  git(root, ['add', '--', 'Cargo.lock']);
  git(root, ['commit', '--quiet', '-m', 'lock tree']);
  return Object.freeze({ root, commit: git(root, ['rev-parse', 'HEAD']) });
}

function unrelatedCommitFixture(): Readonly<{
  root: string;
  baselineCommit: string;
  controllerCommit: string;
}> {
  const root = initializedGitRoot('coding-harness-unrelated-lock-');
  writeFileSync(join(root, 'Cargo.lock'), 'version = 4\n');
  git(root, ['add', '--', 'Cargo.lock']);
  git(root, ['commit', '--quiet', '-m', 'baseline']);
  const baselineCommit = git(root, ['rev-parse', 'HEAD']);
  git(root, ['checkout', '--quiet', '--orphan', 'unrelated-controller']);
  unlinkSync(join(root, 'Cargo.lock'));
  writeFileSync(join(root, 'controller.txt'), 'unrelated\n');
  git(root, ['add', '--all']);
  git(root, ['commit', '--quiet', '-m', 'controller']);
  return Object.freeze({
    root,
    baselineCommit,
    controllerCommit: git(root, ['rev-parse', 'HEAD']),
  });
}

function clonedRepositoryFixture(source: string): Readonly<{ root: string }> {
  const parent = mkdtempSync(join(tmpdir(), 'coding-harness-task-clone-'));
  roots.push(parent);
  const root = join(parent, 'repository');
  git(parent, ['clone', '--quiet', '--no-local', '--', source, root]);
  git(root, ['config', 'user.email', 'harness@example.invalid']);
  git(root, ['config', 'user.name', 'Harness Test']);
  return Object.freeze({ root });
}

function initializedGitRoot(prefix: string): string {
  const root = mkdtempSync(join(tmpdir(), prefix));
  roots.push(root);
  git(root, ['init', '--quiet']);
  git(root, ['config', 'user.email', 'harness@example.invalid']);
  git(root, ['config', 'user.name', 'Harness Test']);
  return root;
}

function git(cwd: string, args: string[]): string {
  const result = spawnSync('/usr/bin/git', args, { cwd, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr);
  return result.stdout.trim();
}
