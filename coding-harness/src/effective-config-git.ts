// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  closeSync,
  constants,
  fstatSync,
  lstatSync,
  openSync,
  readFileSync,
  realpathSync,
  type BigIntStats,
} from 'node:fs';
import { isAbsolute, resolve } from 'node:path';
import { normalizeWorkspacePath, SHA256_PATTERN } from './contracts.js';
import type {
  ConfigurationRepositorySnapshot,
  SurfaceProvenance,
} from './effective-config.js';

const GIT_PATH = '/usr/bin/git' as const;
const GIT_OBJECT = /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/;
const MAX_GIT_OUTPUT_BYTES = 4_000_000;
const MAX_INDEX_BYTES = 64_000_000;
const MAX_WORKTREE_BLOB_BYTES = 1_000_000;
const REGULAR_INDEX_MODES = new Set(['100644', '100755']);
const BASE_GIT_ENVIRONMENT = Object.freeze({
  PATH: '/usr/bin:/bin',
  HOME: '/nonexistent',
  LANG: 'C',
  LC_ALL: 'C',
  GIT_CONFIG_NOSYSTEM: '1',
  GIT_CONFIG_GLOBAL: '/dev/null',
  GIT_ATTR_NOSYSTEM: '1',
  GIT_TERMINAL_PROMPT: '0',
  GIT_ASKPASS: '/bin/false',
  GIT_PAGER: 'cat',
  PAGER: 'cat',
  GIT_OPTIONAL_LOCKS: '0',
} as const);

type ObjectFormat = 'sha1' | 'sha256';

interface FileIdentity {
  readonly path: string;
  readonly device: string;
  readonly inode: string;
  readonly size: number;
  readonly digest: string;
}

interface RepositoryIdentity {
  readonly path: string;
  readonly device: string;
  readonly inode: string;
}

interface TreeEntry {
  readonly mode: string;
  readonly object: string;
}

interface SnapshotState {
  readonly snapshot: ConfigurationRepositorySnapshot;
  readonly repository: RepositoryIdentity;
  readonly objectFormat: ObjectFormat;
  readonly indexPath: string;
  readonly indexDevice: string;
  readonly indexInode: string;
}

const SNAPSHOT_STATES = new WeakMap<ConfigurationRepositorySnapshot, SnapshotState>();

export interface ConfigurationFileProvenance {
  readonly provenance: SurfaceProvenance;
  readonly trustworthy: boolean;
}

export function captureConfigurationGitSnapshot(
  repositoryRoot: string,
): ConfigurationRepositorySnapshot {
  const first = readSnapshot(repositoryRoot);
  const second = readSnapshot(repositoryRoot);
  if (!sameSnapshot(first, second)) throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_SNAPSHOT_CHANGED');
  SNAPSHOT_STATES.set(first.snapshot, first);
  return first.snapshot;
}

export function assertConfigurationGitSnapshot(
  repositoryRoot: string,
  expected: ConfigurationRepositorySnapshot,
): void {
  assertSnapshotAndGetState(repositoryRoot, expected);
}

export function configurationFileProvenance(
  repositoryRoot: string,
  path: string,
  rawBytes: Uint8Array,
  snapshot: ConfigurationRepositorySnapshot,
): ConfigurationFileProvenance {
  const state = assertSnapshotAndGetState(repositoryRoot, snapshot);
  const workspacePath = configurationPath(path);
  if (!(rawBytes instanceof Uint8Array) || rawBytes.byteLength > MAX_WORKTREE_BLOB_BYTES) {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_WORKTREE_BLOB_INVALID');
  }
  const indexEntry = readIndexEntry(repositoryRoot, workspacePath, snapshot, state);
  const headEntry = readHeadEntry(repositoryRoot, workspacePath, snapshot, state.objectFormat);
  const worktreeObject = hashGitBlob(rawBytes, state.objectFormat);

  let provenance: SurfaceProvenance;
  if (indexEntry !== null) {
    const indexMatchesHead = headEntry !== null
      && indexEntry.mode === headEntry.mode
      && indexEntry.object === headEntry.object;
    provenance = indexMatchesHead && worktreeObject === indexEntry.object
      ? 'tracked-clean'
      : 'tracked-dirty';
  } else if (headEntry !== null) {
    provenance = 'tracked-dirty';
  } else {
    const ignored = runGit(
      repositoryRoot,
      ['check-ignore', '--quiet', '--', workspacePath],
      state.indexPath,
      [0, 1],
    );
    provenance = ignored.status === 0 ? 'ignored' : 'untracked';
  }

  assertConfigurationGitSnapshot(repositoryRoot, snapshot);
  return Object.freeze({ provenance, trustworthy: true });
}

function readSnapshot(repositoryRoot: string): SnapshotState {
  const git = canonicalGitIdentity();
  const repository = canonicalRepository(repositoryRoot);
  const topLevel = singleLine(runGit(repository.path, ['rev-parse', '--show-toplevel']).stdout,
    'HARNESS_EFFECTIVE_CONFIG_GIT_TOPLEVEL_INVALID');
  if (topLevel !== repository.path) throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_ROOT_NOT_TOPLEVEL');
  const inside = singleLine(runGit(repository.path, ['rev-parse', '--is-inside-work-tree']).stdout,
    'HARNESS_EFFECTIVE_CONFIG_GIT_WORKTREE_INVALID');
  if (inside !== 'true') throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_WORKTREE_INVALID');
  const objectFormat = parseObjectFormat(singleLine(
    runGit(repository.path, ['rev-parse', '--show-object-format']).stdout,
    'HARNESS_EFFECTIVE_CONFIG_GIT_OBJECT_FORMAT_INVALID',
  ));
  const head = singleLine(
    runGit(repository.path, ['rev-parse', '--verify', 'HEAD^{commit}']).stdout,
    'HARNESS_EFFECTIVE_CONFIG_GIT_HEAD_INVALID',
  );
  assertObject(head, objectFormat, 'HARNESS_EFFECTIVE_CONFIG_GIT_HEAD_INVALID');
  const indexPath = singleLine(
    runGit(repository.path, ['rev-parse', '--path-format=absolute', '--git-path', 'index']).stdout,
    'HARNESS_EFFECTIVE_CONFIG_GIT_INDEX_PATH_INVALID',
  );
  const sharedIndex = runGit(repository.path, ['rev-parse', '--shared-index-path']).stdout;
  if (sharedIndex.length !== 0) throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_INDEX_SPECIAL');
  const unmerged = runGit(
    repository.path,
    ['ls-files', '--unmerged', '-z'],
    indexPath,
  ).stdout;
  if (unmerged.length !== 0) throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_INDEX_UNMERGED');
  const index = canonicalFile(indexPath, MAX_INDEX_BYTES, 'HARNESS_EFFECTIVE_CONFIG_GIT_INDEX_INVALID');
  const digest = snapshotDigest(repository.path, head, index.digest, git.digest);
  const snapshot: ConfigurationRepositorySnapshot = Object.freeze({
    repositoryRoot: repository.path,
    headCommit: head,
    indexTree: index.digest,
    gitExecutableDigest: git.digest,
    digest,
  });
  return Object.freeze({
    snapshot,
    repository,
    objectFormat,
    indexPath: index.path,
    indexDevice: index.device,
    indexInode: index.inode,
  });
}

function readIndexEntry(
  repositoryRoot: string,
  path: string,
  snapshot: ConfigurationRepositorySnapshot,
  state: SnapshotState,
): TreeEntry | null {
  const output = runGit(
    repositoryRoot,
    ['ls-files', '--stage', '-v', '-z', '--full-name', '--', path],
    state.indexPath,
  ).stdout;
  const records = nulRecords(output, 'HARNESS_EFFECTIVE_CONFIG_GIT_INDEX_ENTRY_INVALID');
  if (records.length === 0) return null;
  if (records.length !== 1) throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_INDEX_ENTRY_SPECIAL');
  const match = /^([^ ]) ([0-9]{6}) ([a-f0-9]{40,64}) ([0-3])\t([\s\S]+)$/.exec(records[0]!);
  if (match === null || match[5] !== path) {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_INDEX_ENTRY_INVALID');
  }
  if (match[1] !== 'H') throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_INDEX_FLAGS_UNSAFE');
  if (match[4] !== '0' || !REGULAR_INDEX_MODES.has(match[2]!)) {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_INDEX_ENTRY_SPECIAL');
  }
  assertObject(match[3]!, state.objectFormat, 'HARNESS_EFFECTIVE_CONFIG_GIT_INDEX_ENTRY_INVALID');
  return Object.freeze({ mode: match[2]!, object: match[3]! });
}

function readHeadEntry(
  repositoryRoot: string,
  path: string,
  snapshot: ConfigurationRepositorySnapshot,
  objectFormat: ObjectFormat,
): TreeEntry | null {
  const output = runGit(
    repositoryRoot,
    ['ls-tree', '-z', '--full-tree', snapshot.headCommit, '--', path],
  ).stdout;
  const records = nulRecords(output, 'HARNESS_EFFECTIVE_CONFIG_GIT_HEAD_ENTRY_INVALID');
  if (records.length === 0) return null;
  if (records.length !== 1) throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_HEAD_ENTRY_SPECIAL');
  const match = /^([0-9]{6}) ([^ ]+) ([a-f0-9]{40,64})\t([\s\S]+)$/.exec(records[0]!);
  if (match === null || match[2] !== 'blob' || match[4] !== path
    || !REGULAR_INDEX_MODES.has(match[1]!)) {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_HEAD_ENTRY_SPECIAL');
  }
  assertObject(match[3]!, objectFormat, 'HARNESS_EFFECTIVE_CONFIG_GIT_HEAD_ENTRY_INVALID');
  return Object.freeze({ mode: match[1]!, object: match[3]! });
}

function runGit(
  cwd: string,
  args: readonly string[],
  indexPath?: string,
  acceptedStatuses: readonly number[] = [0],
): Readonly<{ stdout: Buffer; status: number }> {
  assertCanonicalGitStat();
  const env: NodeJS.ProcessEnv = { ...BASE_GIT_ENVIRONMENT };
  if (indexPath !== undefined) env.GIT_INDEX_FILE = indexPath;
  const result = spawnSync(GIT_PATH, ['--no-optional-locks',
    '-c', 'core.hooksPath=/dev/null', '-c', 'core.fsmonitor=false', '-c', 'core.pager=cat',
    ...args], {
    cwd,
    env,
    shell: false,
    stdio: ['ignore', 'pipe', 'pipe'],
    encoding: null,
    timeout: 10_000,
    maxBuffer: MAX_GIT_OUTPUT_BYTES,
  });
  if (result.error !== undefined || result.signal !== null || result.status === null
    || !acceptedStatuses.includes(result.status)) {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_COMMAND_FAILED');
  }
  return Object.freeze({ stdout: result.stdout, status: result.status });
}

function canonicalGitIdentity(): FileIdentity {
  const identity = canonicalFile(GIT_PATH, MAX_INDEX_BYTES, 'HARNESS_EFFECTIVE_CONFIG_GIT_BINARY_INVALID');
  const stat = lstatSync(GIT_PATH, { bigint: true });
  if (stat.uid !== 0n || (stat.mode & 0o022n) !== 0n || (stat.mode & 0o111n) === 0n) {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_BINARY_UNTRUSTED');
  }
  return identity;
}

function assertCanonicalGitStat(): void {
  try {
    const stat = lstatSync(GIT_PATH, { bigint: true });
    if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1n || stat.uid !== 0n
      || (stat.mode & 0o022n) !== 0n || (stat.mode & 0o111n) === 0n
      || realpathSync(GIT_PATH) !== GIT_PATH) throw new Error();
  } catch {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_BINARY_UNTRUSTED');
  }
}

function canonicalRepository(path: string): RepositoryIdentity {
  try {
    if (!isAbsolute(path) || resolve(path) !== path || path.includes('\0')) throw new Error();
    const stat = lstatSync(path, { bigint: true });
    if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(path) !== path) throw new Error();
    return Object.freeze({ path, device: String(stat.dev), inode: String(stat.ino) });
  } catch {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_REPOSITORY_ROOT_INVALID');
  }
}

function canonicalFile(path: string, maximumBytes: number, errorCode: string): FileIdentity {
  let descriptor: number | undefined;
  try {
    if (!isAbsolute(path) || resolve(path) !== path || path.includes('\0') || realpathSync(path) !== path) {
      throw new Error();
    }
    descriptor = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW);
    const before = fstatSync(descriptor, { bigint: true });
    if (!before.isFile() || before.isSymbolicLink() || before.nlink !== 1n
      || before.size > BigInt(maximumBytes) || before.size > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error();
    const content = readFileSync(descriptor);
    const after = fstatSync(descriptor, { bigint: true });
    if (!sameFileStat(before, after) || content.byteLength !== Number(before.size)
      || realpathSync(path) !== path) throw new Error();
    return Object.freeze({
      path,
      device: String(before.dev),
      inode: String(before.ino),
      size: content.byteLength,
      digest: createHash('sha256').update(content).digest('hex'),
    });
  } catch {
    throw new Error(errorCode);
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
  }
}

function sameFileStat(left: BigIntStats, right: BigIntStats): boolean {
  return left.dev === right.dev && left.ino === right.ino && left.size === right.size
    && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}

function configurationPath(path: string): string {
  const normalized = normalizeWorkspacePath(path, 'configuration path');
  if (/[\x00-\x1f\x7f]/.test(normalized)) {
    throw new TypeError('configuration path contains control characters');
  }
  return normalized;
}

function singleLine(output: Buffer, errorCode: string): string {
  const text = output.toString('utf8');
  const value = text.endsWith('\n') ? text.slice(0, -1) : text;
  if (value.length === 0 || /[\r\n\0]/.test(value)) throw new Error(errorCode);
  return value;
}

function nulRecords(output: Buffer, errorCode: string): string[] {
  if (output.length === 0) return [];
  if (output.at(-1) !== 0) throw new Error(errorCode);
  return output.subarray(0, -1).toString('utf8').split('\0');
}

function parseObjectFormat(value: string): ObjectFormat {
  if (value !== 'sha1' && value !== 'sha256') {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_OBJECT_FORMAT_INVALID');
  }
  return value;
}

function assertObject(value: string, format: ObjectFormat, errorCode: string): void {
  if (!GIT_OBJECT.test(value) || value.length !== (format === 'sha1' ? 40 : 64)) {
    throw new Error(errorCode);
  }
}

function hashGitBlob(bytes: Uint8Array, format: ObjectFormat): string {
  const header = Buffer.from(`blob ${bytes.byteLength}\0`, 'utf8');
  return createHash(format).update(header).update(bytes).digest('hex');
}

function snapshotDigest(root: string, head: string, index: string, git: string): string {
  return createHash('sha256').update(JSON.stringify([root, head, index, git])).digest('hex');
}

function validateSnapshot(value: ConfigurationRepositorySnapshot): void {
  if (value === null || typeof value !== 'object'
    || value.repositoryRoot.length === 0 || value.repositoryRoot.includes('\0')
    || !GIT_OBJECT.test(value.headCommit)
    || !SHA256_PATTERN.test(value.indexTree)
    || !SHA256_PATTERN.test(value.gitExecutableDigest)
    || !SHA256_PATTERN.test(value.digest)) {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_SNAPSHOT_INVALID');
  }
  if (value.digest !== snapshotDigest(
    value.repositoryRoot, value.headCommit, value.indexTree, value.gitExecutableDigest,
  )) throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_SNAPSHOT_INVALID');
}

function sameSnapshot(
  left: SnapshotState,
  right: SnapshotState,
): boolean {
  return left.snapshot.digest === right.snapshot.digest
    && left.repository.device === right.repository.device
    && left.repository.inode === right.repository.inode
    && left.objectFormat === right.objectFormat
    && left.indexPath === right.indexPath
    && left.indexDevice === right.indexDevice
    && left.indexInode === right.indexInode;
}

function assertSnapshotAndGetState(
  repositoryRoot: string,
  expected: ConfigurationRepositorySnapshot,
): SnapshotState {
  validateSnapshot(expected);
  if (repositoryRoot !== expected.repositoryRoot) {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_SNAPSHOT_ROOT_MISMATCH');
  }
  const current = readSnapshot(repositoryRoot);
  const bound = SNAPSHOT_STATES.get(expected);
  if (current.snapshot.digest !== expected.digest || (bound !== undefined && !sameSnapshot(current, bound))) {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_GIT_SNAPSHOT_CHANGED');
  }
  return bound ?? current;
}
