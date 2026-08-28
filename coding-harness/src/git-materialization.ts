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
  readdirSync,
  realpathSync,
  type BigIntStats,
} from 'node:fs';
import { basename, dirname, isAbsolute, join, resolve } from 'node:path';
import { normalizeWorkspacePath } from './contracts.js';
import { runGitCommand } from './git-process.js';
import { resolveWorkspacePath } from './workspace.js';

const GIT_OBJECT = /^[a-f0-9]{40,64}$/;
const INDEX_ENTRY = /^(100644|100755) ([a-f0-9]{40}|[a-f0-9]{64}) 0\t(.+)$/;
const DANGEROUS_CONFIG =
  '^(include(\\..*)?$|includeif(\\..*)?$|extensions\\.worktreeconfig$|filter\\.|diff\\..*\\.(command|textconv)$|merge\\..*\\.driver$|gpg\\.|commit\\.gpgsign$|user\\.signingkey$|core\\.(attributesfile|autocrlf|eol|safecrlf)$)';
const MAX_INDEX_BYTES = 20_000_000;
const MAX_OBJECT_AUTHORITY_NODES = 1_000_000;
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
  requireProtectedAuthority?: boolean;
  signal?: AbortSignal;
}>): Promise<void> {
  const protectedAuthority = input.requireProtectedAuthority === true;
  const root = protectedAuthority
    ? canonicalProtectedDirectory(input.repositoryRoot)
    : canonicalDirectory(input.repositoryRoot);
  const commonPath = (await gitChecked(
    root,
    ['rev-parse', '--path-format=absolute', '--git-common-dir'],
    input.signal,
    4096,
  )).trim();
  const commonRoot = protectedAuthority
    ? canonicalProtectedDirectory(commonPath)
    : canonicalDirectory(commonPath);
  if (protectedAuthority) {
    const gitDirectory = canonicalProtectedDirectory((await gitChecked(
      root,
      ['rev-parse', '--path-format=absolute', '--absolute-git-dir'],
      input.signal,
      4096,
    )).trim());
    const objectRoot = canonicalProtectedDirectory((await gitChecked(
      root,
      ['rev-parse', '--path-format=absolute', '--git-path', 'objects'],
      input.signal,
      4096,
    )).trim());
    if (objectRoot !== join(commonRoot, 'objects')) {
      throw new Error('HARNESS_GIT_MATERIALIZATION_OBJECT_AUTHORITY_UNPROTECTED');
    }
    assertProtectedObjectAuthority(objectRoot);
    assertProtectedControlFile(join(commonRoot, 'config'), true);
    assertProtectedControlFile(join(gitDirectory, 'config.worktree'), false);
  }
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
  for (const parts of FORBIDDEN_GIT_FILES) {
    const forbidden = join(commonRoot, ...parts);
    if (protectedAuthority) assertProtectedCreationParent(forbidden);
    if (existsSync(forbidden)) {
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

function canonicalProtectedDirectory(value: string): string {
  const path = canonicalDirectory(value);
  const stat = lstatSync(path);
  const uid = currentUid(stat.uid);
  if ((stat.mode & 0o022) !== 0 || (stat.uid !== 0 && stat.uid !== uid)) {
    throw new Error('HARNESS_GIT_MATERIALIZATION_AUTHORITY_UNPROTECTED');
  }
  assertProtectedParentChain(path, uid);
  return path;
}

function assertProtectedControlFile(path: string, required: boolean): void {
  assertProtectedCreationParent(path);
  if (!existsSync(path)) {
    if (required) throw new Error('HARNESS_GIT_MATERIALIZATION_AUTHORITY_UNPROTECTED');
    return;
  }
  const stat = lstatSync(path);
  const uid = currentUid(stat.uid);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1
    || realpathSync(path) !== path || (stat.mode & 0o022) !== 0
    || (stat.uid !== 0 && stat.uid !== uid)) {
    throw new Error('HARNESS_GIT_MATERIALIZATION_AUTHORITY_UNPROTECTED');
  }
}

function assertProtectedObjectAuthority(root: string): void {
  const pending = [root];
  let visited = 0;
  while (pending.length > 0) {
    const directory = pending.pop()!;
    for (const name of readdirSync(directory)) {
      visited += 1;
      if (visited > MAX_OBJECT_AUTHORITY_NODES) {
        throw new Error('HARNESS_GIT_MATERIALIZATION_OBJECT_AUTHORITY_LIMIT');
      }
      const path = join(directory, name);
      const stat = lstatSync(path);
      const uid = currentUid(stat.uid);
      if (stat.isSymbolicLink() || realpathSync(path) !== path
        || (stat.mode & 0o022) !== 0 || (stat.uid !== 0 && stat.uid !== uid)) {
        throw new Error('HARNESS_GIT_MATERIALIZATION_OBJECT_AUTHORITY_UNPROTECTED');
      }
      if (stat.isDirectory()) pending.push(path);
      else if (!stat.isFile()) {
        throw new Error('HARNESS_GIT_MATERIALIZATION_OBJECT_AUTHORITY_UNPROTECTED');
      }
    }
  }
}

function assertProtectedCreationParent(path: string): void {
  let parent = dirname(path);
  while (!existsSync(parent)) {
    const next = dirname(parent);
    if (next === parent) {
      throw new Error('HARNESS_GIT_MATERIALIZATION_AUTHORITY_UNPROTECTED');
    }
    parent = next;
  }
  canonicalProtectedDirectory(parent);
}

function assertProtectedParentChain(path: string, uid: number): void {
  let current = dirname(path);
  while (true) {
    const stat = lstatSync(current);
    const writable = (stat.mode & 0o022) !== 0;
    const sticky = (stat.mode & 0o1000) !== 0;
    if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(current) !== current
      || (stat.uid !== 0 && stat.uid !== uid) || (writable && !sticky)) {
      throw new Error('HARNESS_GIT_MATERIALIZATION_AUTHORITY_UNPROTECTED');
    }
    const parent = dirname(current);
    if (parent === current) return;
    current = parent;
  }
}

function currentUid(fallback: number): number {
  return typeof process.getuid === 'function' ? process.getuid() : fallback;
}

function sameIdentity(left: BigIntStats, right: BigIntStats) {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.size === right.size && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}
