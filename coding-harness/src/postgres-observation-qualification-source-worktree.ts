// SPDX-License-Identifier: MIT

import { lstatSync, mkdtempSync, realpathSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { isAbsolute, join, resolve } from 'node:path';
import { runGitCommand } from './git-process.js';

const GIT_OBJECT = /^[a-f0-9]{40}$/;

export async function withPostgresQualificationSourceWorktree<T>(
  repositoryRoot: string,
  commit: string,
  action: (sourceRoot: string) => Promise<T>,
): Promise<T> {
  const root = canonicalRoot(repositoryRoot);
  if (!GIT_OBJECT.test(commit)) {
    throw new Error('POSTGRES_QUALIFICATION_SOURCE_COMMIT_INVALID');
  }
  const temporaryRoot = mkdtempSync(join(tmpdir(), 'semantic-fabric-pgq-source-'));
  const sourceRoot = join(temporaryRoot, 'source');
  let added = false;
  let actionError: unknown;
  let output: T | undefined;
  try {
    const result = await runGitCommand(root, [
      'worktree', 'add', '--quiet', '--detach', sourceRoot, commit,
    ], { timeoutMs: 60_000, maxOutputBytes: 64 * 1024 });
    if (result.exitCode !== 0 || result.stdout !== '' || result.stderr !== '') {
      throw new Error('POSTGRES_QUALIFICATION_SOURCE_WORKTREE_ADD_FAILED');
    }
    added = true;
    if (realpathSync(sourceRoot) !== sourceRoot || !lstatSync(sourceRoot).isDirectory()) {
      throw new Error('POSTGRES_QUALIFICATION_SOURCE_WORKTREE_INVALID');
    }
    output = await action(sourceRoot);
  } catch (error) {
    actionError = error;
  }
  let cleanupError: unknown;
  if (added) {
    try {
      const result = await runGitCommand(root, [
        'worktree', 'remove', '--force', sourceRoot,
      ], { timeoutMs: 60_000, maxOutputBytes: 64 * 1024 });
      if (result.exitCode !== 0 || result.stdout !== '' || result.stderr !== '') {
        throw new Error('POSTGRES_QUALIFICATION_SOURCE_WORKTREE_REMOVE_FAILED');
      }
    } catch (error) {
      cleanupError = error;
    }
  }
  // Preserve a worktree whose Git deregistration failed. Removing its files first
  // would leave a stale registration while destroying the path needed to repair it.
  if (!added || cleanupError === undefined) {
    rmSync(temporaryRoot, { recursive: true, force: true });
  }
  if (actionError !== undefined && cleanupError !== undefined) {
    throw new AggregateError(
      [actionError, cleanupError], 'POSTGRES_QUALIFICATION_SOURCE_AND_CLEANUP_FAILED',
    );
  }
  if (actionError !== undefined) throw actionError;
  if (cleanupError !== undefined) throw cleanupError;
  if (output === undefined) throw new Error('POSTGRES_QUALIFICATION_SOURCE_ACTION_INCOMPLETE');
  return output;
}

function canonicalRoot(root: string): string {
  if (!isAbsolute(root) || resolve(root) !== root || realpathSync(root) !== root
    || !lstatSync(root).isDirectory()) {
    throw new Error('POSTGRES_QUALIFICATION_REPOSITORY_ROOT_INVALID');
  }
  return root;
}
