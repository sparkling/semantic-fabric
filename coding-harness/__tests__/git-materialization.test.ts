// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import {
  chmodSync,
  lstatSync,
  mkdtempSync,
  mkdirSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import {
  assertGitMaterializationSafe,
  assertRawIndexMatchesWorkingTree,
} from '../src/git-materialization.js';
import { runGitCommand } from '../src/git-process.js';

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('raw Git materialization boundary', () => {
  it('compares working bytes to raw index objects instead of clean-filter output', async () => {
    const root = repository();
    await expect(assertRawIndexMatchesWorkingTree({ workspaceRoot: root })).resolves.toBeUndefined();
    writeFileSync(join(root, 'source.txt'), 'working bytes changed\n');
    await expect(assertRawIndexMatchesWorkingTree({ workspaceRoot: root })).rejects.toThrow(
      /HARNESS_GIT_WORKTREE_BLOB_MISMATCH/,
    );
  });

  it('rejects executable materialization configuration and attribute surfaces', async () => {
    const root = repository();
    git(root, ['config', 'filter.inject.smudge', '/bin/true']);
    hardenStore(root);
    await expect(assertGitMaterializationSafe({ repositoryRoot: root })).rejects.toThrow(
      /MATERIALIZATION_CONFIG_FORBIDDEN/,
    );
    git(root, ['config', '--unset', 'filter.inject.smudge']);
    hardenStore(root);
    writeFileSync(join(root, '.git', 'info', 'attributes'), '*.txt filter=inject\n');
    await expect(assertGitMaterializationSafe({ repositoryRoot: root })).rejects.toThrow(
      /MATERIALIZATION_FILE_FORBIDDEN/,
    );
  });

  it('rejects per-worktree configuration before a checkout can run filters', async () => {
    const root = repository();
    git(root, ['config', 'extensions.worktreeConfig', 'true']);
    git(root, ['config', '--worktree', 'filter.inject.smudge', '/bin/true']);
    hardenStore(root);
    await expect(assertGitMaterializationSafe({ repositoryRoot: root })).rejects.toThrow(
      /MATERIALIZATION_CONFIG_FORBIDDEN/,
    );
  });

  it.each([
    ['repository config', (root: string) => chmodSync(join(root, '.git', 'config'), 0o666)],
    ['attribute parent', (root: string) => chmodSync(join(root, '.git', 'info'), 0o777)],
    ['object metadata parent', (root: string) =>
      chmodSync(join(root, '.git', 'objects', 'info'), 0o777)],
    ['loose object', (root: string) => {
      const object = git(root, ['rev-parse', 'HEAD^{tree}']);
      chmodSync(join(root, '.git', 'objects', object.slice(0, 2), object.slice(2)), 0o666);
    }],
  ])('rejects a cross-UID writable %s surface', async (_label, mutate) => {
    const root = repository();
    mutate(root);
    await expect(assertGitMaterializationSafe({
      repositoryRoot: root, requireProtectedAuthority: true,
    })).rejects.toThrow(
      /MATERIALIZATION_(?:OBJECT_)?AUTHORITY_UNPROTECTED/,
    );
  });

  it.each([true, false])('rejects %s active includeIf authority', async (active) => {
    const root = repository();
    const included = join(root, 'included.cfg');
    writeFileSync(included, '[core]\n\trepositoryformatversion = 0\n');
    const condition = active ? `gitdir:${join(root, '.git')}/` : 'gitdir:/never-active/';
    git(root, ['config', `includeIf.${condition}.path`, included]);
    hardenStore(root);
    await expect(assertGitMaterializationSafe({ repositoryRoot: root })).rejects.toThrow(
      /MATERIALIZATION_CONFIG_FORBIDDEN/,
    );
  });

  it.each([false, true])('rejects exact-commit attributes in a %s store', async (bare) => {
    const root = repository();
    writeFileSync(join(root, '.gitattributes'), '*.txt filter=inject\n');
    git(root, ['add', '--', '.gitattributes']);
    git(root, ['commit', '--quiet', '-m', 'attributes']);
    const attributed = git(root, ['rev-parse', 'HEAD']);
    git(root, ['rm', '--quiet', '--', '.gitattributes']);
    git(root, ['commit', '--quiet', '-m', 'remove attributes']);
    hardenStore(root);
    let store = root;
    if (bare) {
      store = mkdtempSync(join(tmpdir(), 'coding-harness-git-materialization-bare-'));
      roots.push(store); rmSync(store, { recursive: true });
      git(tmpdir(), ['clone', '--quiet', '--bare', root, store]);
      hardenStore(store, true);
    }
    await expect(assertGitMaterializationSafe({
      repositoryRoot: store, commits: [attributed],
    })).rejects.toThrow(/TRACKED_ATTRIBUTES_FORBIDDEN/);
  });

  it('disables replacement objects in shared Git reads and rejects replacement refs', async () => {
    const root = repository();
    const trusted = git(root, ['rev-parse', 'HEAD']);
    writeFileSync(join(root, 'source.txt'), 'replacement bytes\n');
    git(root, ['add', '--', 'source.txt']);
    git(root, ['commit', '--quiet', '-m', 'replacement']);
    const replacement = git(root, ['rev-parse', 'HEAD']);
    git(root, ['replace', trusted, replacement]);
    hardenStore(root);
    expect(git(root, ['show', `${trusted}:source.txt`])).toBe('replacement bytes');

    const protectedRead = await runGitCommand(root, ['show', `${trusted}:source.txt`]);
    expect(protectedRead.exitCode).toBe(0);
    expect(protectedRead.stdout).toBe('trusted bytes\n');
    await expect(assertGitMaterializationSafe({ repositoryRoot: root })).rejects.toThrow(
      /REPLACEMENT_REF_FORBIDDEN/,
    );
  });
});

function repository(): string {
  const root = mkdtempSync(join(tmpdir(), 'coding-harness-git-materialization-'));
  roots.push(root);
  mkdirSync(join(root, 'nested'));
  writeFileSync(join(root, 'source.txt'), 'trusted bytes\n');
  writeFileSync(join(root, 'nested', 'source.txt'), 'nested bytes\n');
  git(root, ['init', '--quiet']);
  git(root, ['config', 'user.email', 'harness@example.invalid']);
  git(root, ['config', 'user.name', 'Harness Test']);
  git(root, ['add', '--', 'source.txt', 'nested/source.txt']);
  git(root, ['commit', '--quiet', '-m', 'trusted']);
  hardenStore(root);
  return root;
}

function hardenStore(root: string, bare = false): void {
  const gitRoot = bare ? root : join(root, '.git');
  const visit = (path: string): void => {
    const stat = lstatSync(path); chmodSync(path, stat.mode & ~0o022);
    if (stat.isDirectory() && !stat.isSymbolicLink()) {
      for (const entry of readdirSync(path)) visit(join(path, entry));
    }
  };
  visit(gitRoot);
}

function git(root: string, args: readonly string[]): string {
  const result = spawnSync('/usr/bin/git', [...args], {
    cwd: root,
    env: {
      PATH: '/usr/bin:/bin', HOME: '/nonexistent', LANG: 'C', LC_ALL: 'C',
      GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null',
    },
    encoding: 'utf8',
  });
  if (result.status !== 0) throw new Error(result.stderr);
  return result.stdout.trim();
}
