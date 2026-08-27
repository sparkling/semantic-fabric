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
  readIssue8FrozenCargoLock,
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

function git(cwd: string, args: string[]): string {
  const result = spawnSync('/usr/bin/git', args, { cwd, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr);
  return result.stdout.trim();
}
