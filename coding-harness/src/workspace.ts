// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { lstatSync, readFileSync, realpathSync } from 'node:fs';
import { dirname, relative, resolve, sep } from 'node:path';
import { normalizeWorkspacePath } from './contracts.js';

export interface WorkspacePathOptions {
  allowRoot?: boolean;
  allowMissingLeaf?: boolean;
  requireDirectory?: boolean;
  requireRegularFile?: boolean;
  rejectHardlinks?: boolean;
}

export function resolveWorkspacePath(
  workspaceRoot: string,
  input: string,
  options: WorkspacePathOptions = {},
): string {
  const relativePath = normalizeWorkspacePath(input, 'workspace path', options.allowRoot ?? false);
  const rootStat = lstatSync(workspaceRoot);
  if (!rootStat.isDirectory() || rootStat.isSymbolicLink()) {
    throw new Error('workspace root must be a real directory');
  }
  const canonicalRoot = realpathSync(workspaceRoot);
  const absolute = relativePath === '.' ? workspaceRoot : resolve(workspaceRoot, relativePath);
  assertInside(canonicalRoot, absolute);

  const parts = relativePath === '.' ? [] : relativePath.split('/');
  let cursor = workspaceRoot;
  for (let index = 0; index < parts.length; index += 1) {
    cursor = resolve(cursor, parts[index]);
    let stat;
    try {
      stat = lstatSync(cursor);
    } catch (error) {
      const isLeaf = index === parts.length - 1;
      if (isLeaf && options.allowMissingLeaf && isMissing(error)) return absolute;
      throw error;
    }
    if (stat.isSymbolicLink()) throw new Error(`workspace path crosses symlink: ${relativePath}`);
  }

  const stat = lstatSync(absolute);
  const canonical = realpathSync(absolute);
  assertInside(canonicalRoot, canonical);
  if (options.requireDirectory && !stat.isDirectory()) throw new Error(`${relativePath} is not a directory`);
  if (options.requireRegularFile && !stat.isFile()) throw new Error(`${relativePath} is not a regular file`);
  if (options.rejectHardlinks && stat.isFile() && stat.nlink > 1) {
    throw new Error(`${relativePath} is hard-linked and cannot be proven worktree-local`);
  }
  return absolute;
}

export function resolveMutablePath(workspaceRoot: string, relativePath: string): string {
  try {
    return resolveWorkspacePath(workspaceRoot, relativePath, {
      requireRegularFile: true,
      rejectHardlinks: true,
    });
  } catch (error) {
    if (!isMissing(error)) throw error;
    const parent = dirname(relativePath);
    resolveWorkspacePath(workspaceRoot, parent === '.' ? '.' : parent, {
      allowRoot: true,
      requireDirectory: true,
    });
    return resolveWorkspacePath(workspaceRoot, relativePath, { allowMissingLeaf: true });
  }
}

export function sha256File(path: string): string {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

export function countFileLines(path: string): number {
  const content = readFileSync(path);
  if (content.length === 0) return 0;
  let lines = 1;
  for (const byte of content) if (byte === 10) lines += 1;
  if (content.at(-1) === 10) lines -= 1;
  return lines;
}

function assertInside(root: string, candidate: string): void {
  const delta = relative(root, candidate);
  if (delta === '..' || delta.startsWith(`..${sep}`) || resolve(root, delta) !== resolve(candidate)) {
    throw new Error('workspace path escapes the workspace root');
  }
}

function isMissing(error: unknown): boolean {
  return error instanceof Error && 'code' in error && (error as NodeJS.ErrnoException).code === 'ENOENT';
}
