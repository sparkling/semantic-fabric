// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
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
    await expect(assertGitMaterializationSafe({ repositoryRoot: root })).rejects.toThrow(
      /MATERIALIZATION_CONFIG_FORBIDDEN/,
    );
    git(root, ['config', '--unset', 'filter.inject.smudge']);
    writeFileSync(join(root, '.git', 'info', 'attributes'), '*.txt filter=inject\n');
    await expect(assertGitMaterializationSafe({ repositoryRoot: root })).rejects.toThrow(
      /MATERIALIZATION_FILE_FORBIDDEN/,
    );
  });

  it('rejects per-worktree configuration before a checkout can run filters', async () => {
    const root = repository();
    git(root, ['config', 'extensions.worktreeConfig', 'true']);
    git(root, ['config', '--worktree', 'filter.inject.smudge', '/bin/true']);
    await expect(assertGitMaterializationSafe({ repositoryRoot: root })).rejects.toThrow(
      /MATERIALIZATION_CONFIG_FORBIDDEN/,
    );
  });

  it('disables replacement objects in shared Git reads and rejects replacement refs', async () => {
    const root = repository();
    const trusted = git(root, ['rev-parse', 'HEAD']);
    writeFileSync(join(root, 'source.txt'), 'replacement bytes\n');
    git(root, ['add', '--', 'source.txt']);
    git(root, ['commit', '--quiet', '-m', 'replacement']);
    const replacement = git(root, ['rev-parse', 'HEAD']);
    git(root, ['replace', trusted, replacement]);
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
  return root;
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
