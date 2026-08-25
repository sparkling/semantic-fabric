// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  closeSync,
  constants,
  existsSync,
  fstatSync,
  lstatSync,
  openSync,
  readSync,
  realpathSync,
  type BigIntStats,
} from 'node:fs';
import { basename, isAbsolute, join, resolve } from 'node:path';
import { normalizeWorkspacePath } from './contracts.js';
import { runGitCommand } from './git-process.js';
import { resolveWorkspacePath } from './workspace.js';

const GIT_OBJECT = /^[a-f0-9]{40,64}$/;
const INDEX_ENTRY = /^(100644|100755) ([a-f0-9]{40}|[a-f0-9]{64}) 0\t(.+)$/;
const DANGEROUS_CONFIG =
  '^(include(\\..*)?$|extensions\\.worktreeconfig$|filter\\.|diff\\..*\\.(command|textconv)$|merge\\..*\\.driver$|gpg\\.|commit\\.gpgsign$|user\\.signingkey$|core\\.(attributesfile|autocrlf|eol|safecrlf)$)';
const MAX_INDEX_BYTES = 20_000_000;
const MAX_TRACKED_FILES = 100_000;
const MAX_TRACKED_BYTES = 5_000_000_000;
const FORBIDDEN_GIT_FILES = Object.freeze([
  ['info', 'attributes'],
  ['info', 'grafts'],
  ['objects', 'info', 'alternates'],
] as const);

export async function assertGitMaterializationSafe(input: Readonly<{
  repositoryRoot: string;
  commits?: readonly string[];
  signal?: AbortSignal;
}>): Promise<void> {
  const root = canonicalDirectory(input.repositoryRoot);
  const config = await runGitCommand(
    root,
    ['config', '--null', '--get-regexp', DANGEROUS_CONFIG],
    { signal: input.signal, maxOutputBytes: 1_000_000 },
  );
  if (config.exitCode === 0) throw new Error('HARNESS_GIT_MATERIALIZATION_CONFIG_FORBIDDEN');
  if (config.exitCode !== 1) throw new Error('HARNESS_GIT_MATERIALIZATION_CONFIG_FAILED');
  const replacements = await gitChecked(
    root,
    ['for-each-ref', '--format=%(refname)', 'refs/replace/'],
    input.signal,
    1_000_000,
  );
  if (replacements.trim() !== '') throw new Error('HARNESS_GIT_REPLACEMENT_REF_FORBIDDEN');
  const common = await gitChecked(
    root,
    ['rev-parse', '--path-format=absolute', '--git-common-dir'],
    input.signal,
    4096,
  );
  const commonRoot = canonicalDirectory(common.trim());
  for (const parts of FORBIDDEN_GIT_FILES) {
    if (existsSync(join(commonRoot, ...parts))) {
      throw new Error(`HARNESS_GIT_MATERIALIZATION_FILE_FORBIDDEN:${parts.join('/')}`);
    }
  }
  const indexedPaths = parseNullPaths(await gitChecked(
    root,
    ['ls-files', '-z', '--'],
    input.signal,
    MAX_INDEX_BYTES,
  ));
  if (indexedPaths.some((path) => basename(path) === '.gitattributes')) {
    throw new Error('HARNESS_GIT_TRACKED_ATTRIBUTES_FORBIDDEN');
  }
  for (const commit of [...new Set(input.commits ?? [])]) {
    if (!GIT_OBJECT.test(commit)) throw new Error('HARNESS_GIT_MATERIALIZATION_COMMIT_INVALID');
    const paths = parseNullPaths(await gitChecked(
      root,
      ['ls-tree', '-r', '-z', '--name-only', commit],
      input.signal,
      MAX_INDEX_BYTES,
    ));
    if (paths.some((path) => basename(path) === '.gitattributes')) {
      throw new Error('HARNESS_GIT_TRACKED_ATTRIBUTES_FORBIDDEN');
    }
  }
}

export async function assertRawIndexMatchesWorkingTree(input: Readonly<{
  workspaceRoot: string;
  repositoryRoot?: string;
  environment?: Readonly<Record<string, string>>;
  signal?: AbortSignal;
}>): Promise<void> {
  const root = canonicalDirectory(input.workspaceRoot);
  const repositoryRoot = input.repositoryRoot === undefined
    ? root
    : canonicalDirectory(input.repositoryRoot);
  const listing = await gitCheckedResult(repositoryRoot, ['ls-files', '--stage', '-z', '--'], {
    signal: input.signal,
    environment: input.environment,
    maxOutputBytes: MAX_INDEX_BYTES,
  });
  const records = listing.split('\0').filter(Boolean);
  if (records.length === 0 || records.length > MAX_TRACKED_FILES) {
    throw new Error('HARNESS_GIT_RAW_INDEX_ENTRY_LIMIT');
  }
  let totalBytes = 0;
  for (const record of records) {
    const match = INDEX_ENTRY.exec(record);
    if (match === null) throw new Error('HARNESS_GIT_RAW_INDEX_ENTRY_INVALID');
    const path = normalizeWorkspacePath(match[3], 'raw Git index path');
    const absolute = resolveWorkspacePath(root, path, {
      requireRegularFile: true,
      rejectHardlinks: true,
    });
    const observed = rawGitBlobIdentity(absolute, match[2].length);
    totalBytes += observed.bytes;
    if (totalBytes > MAX_TRACKED_BYTES) throw new Error('HARNESS_GIT_RAW_INDEX_BYTE_LIMIT');
    if (observed.object !== match[2]) {
      throw new Error(`HARNESS_GIT_WORKTREE_BLOB_MISMATCH:${path}`);
    }
  }
}

function rawGitBlobIdentity(path: string, objectLength: number) {
  const pathStat = lstatSync(path, { bigint: true });
  if (!pathStat.isFile() || pathStat.isSymbolicLink() || pathStat.nlink !== 1n
    || realpathSync(path) !== path || pathStat.size > BigInt(MAX_TRACKED_BYTES)) {
    throw new Error('HARNESS_GIT_WORKTREE_FILE_INVALID');
  }
  const descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const before = fstatSync(descriptor, { bigint: true });
    const hash = createHash(objectLength === 40 ? 'sha1' : 'sha256');
    hash.update(`blob ${String(before.size)}\0`, 'utf8');
    const buffer = Buffer.allocUnsafe(1024 * 1024);
    let offset = 0n;
    while (offset < before.size) {
      const count = readSync(
        descriptor,
        buffer,
        0,
        Math.min(buffer.length, Number(before.size - offset)),
        Number(offset),
      );
      if (count === 0) break;
      hash.update(buffer.subarray(0, count));
      offset += BigInt(count);
    }
    const after = fstatSync(descriptor, { bigint: true });
    if (offset !== before.size || !sameIdentity(before, after)) {
      throw new Error('HARNESS_GIT_WORKTREE_FILE_CHANGED');
    }
    return Object.freeze({ object: hash.digest('hex'), bytes: Number(before.size) });
  } finally {
    closeSync(descriptor);
  }
}

async function gitChecked(
  root: string,
  args: readonly string[],
  signal: AbortSignal | undefined,
  maxOutputBytes: number,
): Promise<string> {
  return await gitCheckedResult(root, args, { signal, maxOutputBytes });
}

async function gitCheckedResult(
  root: string,
  args: readonly string[],
  options: Readonly<{
    signal?: AbortSignal;
    environment?: Readonly<Record<string, string>>;
    maxOutputBytes: number;
  }>,
): Promise<string> {
  const result = await runGitCommand(root, args, options);
  if (result.exitCode !== 0) throw new Error(`HARNESS_GIT_MATERIALIZATION_COMMAND_FAILED:${args[0]}`);
  return result.stdout;
}

function parseNullPaths(value: string): string[] {
  return value.split('\0').filter(Boolean)
    .map((path) => normalizeWorkspacePath(path, 'Git tree path'));
}

function canonicalDirectory(value: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) {
    throw new Error('HARNESS_GIT_MATERIALIZATION_ROOT_INVALID');
  }
  const stat = lstatSync(value);
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(value) !== value) {
    throw new Error('HARNESS_GIT_MATERIALIZATION_ROOT_INVALID');
  }
  return value;
}

function sameIdentity(left: BigIntStats, right: BigIntStats) {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.size === right.size && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}
