// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  cpSync, existsSync, mkdtempSync, mkdirSync, readFileSync, renameSync, rmSync, statSync,
  symlinkSync, writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { GitWorktreeSet } from '../src/git-worktrees.js';

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function git(cwd: string, args: string[]): string {
  const result = spawnSync('git', args, { cwd, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr);
  return result.stdout.trim();
}

function repository(): { root: string; commit: string; patch: string } {
  const root = mkdtempSync(join(tmpdir(), 'coding-harness-git-'));
  roots.push(root);
  mkdirSync(join(root, 'src'));
  writeFileSync(join(root, 'src/file.txt'), 'before\n');
  writeFileSync(join(root, 'protected.txt'), 'oracle\n');
  git(root, ['init', '--quiet']);
  git(root, ['config', 'user.email', 'harness@example.invalid']);
  git(root, ['config', 'user.name', 'Harness Test']);
  git(root, ['add', '--', 'src/file.txt', 'protected.txt']);
  git(root, ['commit', '--quiet', '-m', 'fixture']);
  const commit = git(root, ['rev-parse', 'HEAD']);
  writeFileSync(join(root, 'src/file.txt'), 'after\n');
  const patch = `${git(root, ['diff', '--binary', '--', 'src/file.txt'])}\n`;
  git(root, ['reset', '--hard', '--quiet', 'HEAD']);
  return { root, commit, patch };
}

describe('isolated Git candidate and evaluator worktrees', () => {
  it('rejects a symlinked repository root at construction', () => {
    const fixture = repository();
    const top = mkdtempSync(join(tmpdir(), 'coding-harness-repository-link-'));
    roots.push(top);
    const linkedRoot = join(top, 'repository');
    symlinkSync(fixture.root, linkedRoot, 'dir');

    expect(() => new GitWorktreeSet({
      repositoryRoot: linkedRoot,
      runRoot: join(top, 'run'),
    })).toThrow(/REPOSITORY_ROOT_INVALID/);
  });

  it('rejects a repository replacement before its first Git use', async () => {
    const fixture = repository();
    const parent = mkdtempSync(join(tmpdir(), 'coding-harness-repository-race-'));
    roots.push(parent);
    const runRoot = join(parent, 'run');
    const worktrees = new GitWorktreeSet({ repositoryRoot: fixture.root, runRoot });
    const movedRoot = `${fixture.root}-original`;
    roots.push(movedRoot);
    renameSync(fixture.root, movedRoot);
    mkdirSync(fixture.root, { mode: 0o700 });

    await expect(worktrees.prepare(fixture.commit, fixture.commit)).rejects.toThrow(
      /REPOSITORY_ROOT_CHANGED/,
    );
    expect(existsSync(runRoot)).toBe(false);
  });

  it('fails closed if the repository root changes before cleanup', async () => {
    const fixture = repository();
    const parent = mkdtempSync(join(tmpdir(), 'coding-harness-repository-cleanup-race-'));
    roots.push(parent);
    const runRoot = join(parent, 'run');
    const worktrees = new GitWorktreeSet({ repositoryRoot: fixture.root, runRoot });
    await worktrees.prepare(fixture.commit, fixture.commit);
    const movedRoot = `${fixture.root}-original`;
    roots.push(movedRoot);
    renameSync(fixture.root, movedRoot);
    mkdirSync(fixture.root, { mode: 0o700 });
    writeFileSync(join(fixture.root, 'sentinel.txt'), 'replacement repository\n');

    await expect(worktrees.dispose()).rejects.toThrow(/REPOSITORY_ROOT_CHANGED/);
    expect(readFileSync(join(fixture.root, 'sentinel.txt'), 'utf8')).toBe(
      'replacement repository\n',
    );
    expect(existsSync(runRoot)).toBe(true);
  });

  it('rejects candidate identity after a prepared repository is replaced by a copy', async () => {
    const fixture = repository();
    const parent = mkdtempSync(join(tmpdir(), 'coding-harness-repository-copy-race-'));
    roots.push(parent);
    const runRoot = join(parent, 'run');
    const worktrees = new GitWorktreeSet({ repositoryRoot: fixture.root, runRoot });
    await worktrees.prepare(fixture.commit, fixture.commit);
    const copiedRoot = `${fixture.root}-copy`;
    const movedRoot = `${fixture.root}-original`;
    roots.push(movedRoot);
    cpSync(fixture.root, copiedRoot, { recursive: true });
    renameSync(fixture.root, movedRoot);
    renameSync(copiedRoot, fixture.root);

    await expect(worktrees.candidateIdentity()).rejects.toThrow(/REPOSITORY_ROOT_CHANGED/);
    expect(existsSync(runRoot)).toBe(true);
  });

  it('admits only exact mutable paths and returns the post-patch tree identity', async () => {
    const fixture = repository();
    const runRoot = join(mkdtempSync(join(tmpdir(), 'coding-harness-run-parent-')), 'run');
    roots.push(runRoot.slice(0, -4));
    const worktrees = new GitWorktreeSet({ repositoryRoot: fixture.root, runRoot });
    const prepared = await worktrees.prepare(fixture.commit, fixture.commit);

    expect(statSync(runRoot).mode & 0o077).toBe(0);
    expect(prepared.candidate).toEqual(prepared.baseline);
    expect(new Set(Object.values(prepared.verifierRoots)).size).toBe(3);
    const admitted = await worktrees.admitAndApply(fixture.patch, ['src/file.txt']);
    expect(admitted.admittedPaths).toEqual(['src/file.txt']);
    expect(admitted.candidate.tree).not.toBe(prepared.baseline.tree);
    expect(admitted.patchDigest).toMatch(/^[a-f0-9]{64}$/);
    for (const stage of ['public', 'independent', 'regression'] as const) {
      expect(await worktrees.verifierIdentity(stage)).toEqual(admitted.candidate);
    }

    await worktrees.resetCandidate();
    expect((await worktrees.candidateIdentity()).tree).toBe(prepared.baseline.tree);
    await worktrees.dispose();
    expect(existsSync(runRoot)).toBe(false);
  });

  it('rejects and never deletes a pre-existing run root', () => {
    const fixture = repository();
    const parent = mkdtempSync(join(tmpdir(), 'coding-harness-preexisting-parent-'));
    roots.push(parent);
    const runRoot = join(parent, 'run');
    mkdirSync(runRoot, { mode: 0o700 });
    writeFileSync(join(runRoot, 'sentinel.txt'), 'owned by caller\n');

    expect(() => new GitWorktreeSet({ repositoryRoot: fixture.root, runRoot })).toThrow(
      /ROOT_NOT_ABSENT/,
    );
    expect(readFileSync(join(runRoot, 'sentinel.txt'), 'utf8')).toBe('owned by caller\n');
  });

  it('rejects a run root beneath a non-canonical parent', () => {
    const fixture = repository();
    const top = mkdtempSync(join(tmpdir(), 'coding-harness-parent-link-'));
    roots.push(top);
    const canonicalParent = join(top, 'canonical');
    const linkedParent = join(top, 'linked');
    mkdirSync(canonicalParent, { mode: 0o700 });
    symlinkSync(canonicalParent, linkedParent, 'dir');

    expect(() => new GitWorktreeSet({
      repositoryRoot: fixture.root,
      runRoot: join(linkedParent, 'run'),
    })).toThrow(/PARENT_UNTRUSTED/);
  });

  it('refuses creation after its trusted parent is replaced', async () => {
    const fixture = repository();
    const top = mkdtempSync(join(tmpdir(), 'coding-harness-parent-swap-'));
    roots.push(top);
    const parent = join(top, 'parent');
    mkdirSync(parent, { mode: 0o700 });
    const runRoot = join(parent, 'run');
    const worktrees = new GitWorktreeSet({ repositoryRoot: fixture.root, runRoot });
    renameSync(parent, join(top, 'original-parent'));
    mkdirSync(parent, { mode: 0o700 });

    await expect(worktrees.prepare(fixture.commit, fixture.commit)).rejects.toThrow(
      /PARENT_CHANGED/,
    );
    expect(existsSync(runRoot)).toBe(false);
  });

  it('fails closed when its run root is replaced before cleanup', async () => {
    const fixture = repository();
    const parent = mkdtempSync(join(tmpdir(), 'coding-harness-root-swap-'));
    roots.push(parent);
    const runRoot = join(parent, 'run');
    const movedRoot = join(parent, 'original-run');
    const worktrees = new GitWorktreeSet({ repositoryRoot: fixture.root, runRoot });
    await worktrees.prepare(fixture.commit, fixture.commit);
    renameSync(runRoot, movedRoot);
    mkdirSync(runRoot, { mode: 0o700 });
    writeFileSync(join(runRoot, 'sentinel.txt'), 'replacement\n');

    await expect(worktrees.dispose()).rejects.toThrow(/ROOT_OWNERSHIP_CHANGED/);
    expect(readFileSync(join(runRoot, 'sentinel.txt'), 'utf8')).toBe('replacement\n');
  });

  it('rejects an undeclared patch before applying it', async () => {
    const fixture = repository();
    const runRoot = join(mkdtempSync(join(tmpdir(), 'coding-harness-run-parent-')), 'run');
    roots.push(runRoot.slice(0, -4));
    const worktrees = new GitWorktreeSet({ repositoryRoot: fixture.root, runRoot });
    await worktrees.prepare(fixture.commit, fixture.commit);

    await expect(worktrees.admitAndApply(fixture.patch, ['other.txt'])).rejects.toThrow(
      /PATCH_PATH_NOT_DECLARED/,
    );
    expect((await worktrees.candidateIdentity()).tree).toBe(
      (await worktrees.baselineIdentity()).tree,
    );
    await worktrees.dispose();
  });

  it('installs and re-establishes a read-only lock overlay in every lane', async () => {
    const fixture = repository();
    const parent = mkdtempSync(join(tmpdir(), 'coding-harness-overlay-parent-'));
    roots.push(parent);
    const source = join(parent, 'Cargo.lock.source');
    writeFileSync(source, 'version = 4\n');
    const digest = createHash('sha256').update(readFileSync(source)).digest('hex');
    const worktrees = new GitWorktreeSet({
      repositoryRoot: fixture.root,
      runRoot: join(parent, 'run'),
    });
    const prepared = await worktrees.prepare(fixture.commit, fixture.commit);

    await worktrees.installFrozenOverlay(source, 'Cargo.lock', digest);
    worktrees.verifyFrozenOverlay('Cargo.lock', digest);
    for (const laneRoot of [
      prepared.evaluatorRoot,
      prepared.candidateRoot,
      ...Object.values(prepared.verifierRoots),
    ]) {
      expect(readFileSync(join(laneRoot, 'Cargo.lock'), 'utf8')).toBe('version = 4\n');
      expect(statSync(join(laneRoot, 'Cargo.lock')).mode & 0o222).toBe(0);
    }

    await worktrees.resetCandidate();
    await worktrees.installFrozenOverlay(source, 'Cargo.lock', digest);
    worktrees.verifyFrozenOverlay('Cargo.lock', digest);
    await worktrees.dispose();
  });
});
