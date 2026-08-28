// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  chmodSync,
  closeSync,
  constants,
  fstatSync,
  lstatSync,
  openSync,
  readSync,
  readdirSync,
  realpathSync,
  type BigIntStats,
} from 'node:fs';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { deepFreeze, normalizeWorkspacePath } from './contracts.js';
import { runGitCommandBytes } from './git-process.js';
import {
  type CaptureTreeEntryV1,
  type CaptureTreeSnapshotV1,
} from './programme-capture-git-v1.js';
import { digestValue } from './receipts.js';

const MAX_SOURCE_BYTES = 5_000_000_000;
const MAX_INDEX_BYTES = 20_000_000;
const INDEX_RECORD = /^(100644|100755) ([a-f0-9]{40}|[a-f0-9]{64}) 0\t(.+)$/;
const OBJECT_SIZE_RECORD = /^([a-f0-9]{40}|[a-f0-9]{64}) blob (0|[1-9][0-9]*)$/;
const CONTROL_CHARACTER = /[\u0000-\u001f\u007f]/;

export interface PinnedSourceIdentityV1 {
  readonly path: string;
  readonly kind: 'directory' | 'file';
  readonly dev: bigint;
  readonly ino: bigint;
  readonly mode: bigint;
  readonly uid: bigint;
  readonly nlink: bigint;
  readonly size: bigint;
  readonly mtimeNs: bigint;
  readonly ctimeNs: bigint;
}

export interface ProgrammeCapturePrivateSourceFileV1 {
  readonly path: string;
  readonly gitMode: '100644' | '100755';
  readonly gitObjectId: string;
  readonly sha256: string;
  readonly byteLength: number;
}

export interface ProgrammeCapturePrivateSourceInventoryV1 {
  readonly files: readonly ProgrammeCapturePrivateSourceFileV1[];
  readonly directories: readonly string[];
  readonly digest: string;
  readonly totalBytes: number;
}

export function assertSupportedPrivateSourceTreeV1(
  snapshot: CaptureTreeSnapshotV1,
): void {
  for (const entry of snapshot.entries.values()) {
    const segments = entry.path.split('/');
    if (CONTROL_CHARACTER.test(entry.path)
      || segments.some((segment) => segment.toLowerCase() === '.git')
      || !((entry.type === 'tree' && entry.mode === '040000')
        || (entry.type === 'blob' && (entry.mode === '100644' || entry.mode === '100755')))) {
      throw new Error(`HARNESS_CAPTURE_PRIVATE_SOURCE_TREE_ENTRY_UNSUPPORTED:${entry.path}`);
    }
  }
  const actual = [...snapshot.entries.values()]
    .filter(({ type }) => type === 'tree').map(({ path }) => path).sort(compareUtf8);
  if (JSON.stringify(actual) !== JSON.stringify(privateSourceDirectoriesV1(snapshot))) {
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_TREE_DIRECTORY_MISMATCH');
  }
}

export function normalizePrivateSourceModesV1(
  root: string,
  snapshot: CaptureTreeSnapshotV1,
): void {
  for (const entry of privateSourceFiles(snapshot)) {
    const path = join(root, entry.path);
    const stat = lstatSync(path, { bigint: true });
    if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1n
      || realpathSync(path) !== path) {
      throw new Error(`HARNESS_CAPTURE_PRIVATE_SOURCE_FILE_INVALID:${entry.path}`);
    }
    chmodSync(path, entry.mode === '100755' ? 0o500 : 0o400);
  }
  for (const directory of privateSourceDirectoriesV1(snapshot)
    .sort((left, right) => right.split('/').length - left.split('/').length)) {
    chmodSync(join(root, directory), 0o500);
  }
  chmodSync(root, 0o500);
}

export function capturePrivateSourceInventoryV1(
  root: string,
  snapshot: CaptureTreeSnapshotV1,
  objectFormat: 'sha1' | 'sha256',
): ProgrammeCapturePrivateSourceInventoryV1 {
  assertExactSourcePaths(root, snapshot);
  const files: ProgrammeCapturePrivateSourceFileV1[] = [];
  let totalBytes = 0;
  for (const entry of privateSourceFiles(snapshot)) {
    const identity = fileIdentity(join(root, entry.path), entry, objectFormat);
    totalBytes += identity.byteLength;
    if (totalBytes > MAX_SOURCE_BYTES) {
      throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_BYTE_LIMIT');
    }
    files.push(identity);
  }
  const directories = privateSourceDirectoriesV1(snapshot);
  const body = { inventoryKind: 'private-source-inventory-v1', directories, files };
  return deepFreeze({ files, directories, digest: digestValue(body), totalBytes });
}

export async function assertPrivateSourceBlobSizesV1(
  store: string,
  snapshot: CaptureTreeSnapshotV1,
  signal?: AbortSignal,
): Promise<void> {
  const files = privateSourceFiles(snapshot);
  if (files.length === 0) return;
  const result = await runGitCommandBytes(
    store,
    ['cat-file', '--batch-check=%(objectname) %(objecttype) %(objectsize)'],
    {
      stdin: `${files.map(({ gitObjectId }) => gitObjectId).join('\n')}\n`,
      signal,
      maxOutputBytes: MAX_INDEX_BYTES,
    },
  );
  if (result.exitCode !== 0 || result.stderr !== '') {
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_SIZE_PREFLIGHT_FAILED');
  }
  parsePrivateSourceBlobSizesV1(result.stdout, files);
}

export function parsePrivateSourceBlobSizesV1(
  listing: Buffer,
  expected: readonly CaptureTreeEntryV1[],
): number {
  const records = decodeUtf8(listing, 'SIZE_PREFLIGHT_INVALID').split('\n');
  if (records.at(-1) !== '' || records.length - 1 !== expected.length) {
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_SIZE_PREFLIGHT_INVALID');
  }
  let totalBytes = 0;
  records.slice(0, -1).forEach((record, index) => {
    const match = OBJECT_SIZE_RECORD.exec(record);
    const byteLength = Number(match?.[2]);
    if (match === null || match[1] !== expected[index]?.gitObjectId
      || !Number.isSafeInteger(byteLength) || byteLength > MAX_SOURCE_BYTES) {
      throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_BYTE_LIMIT');
    }
    totalBytes += byteLength;
    if (totalBytes > MAX_SOURCE_BYTES) {
      throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_BYTE_LIMIT');
    }
  });
  return totalBytes;
}

export async function assertPrivateSourceIndexMatchesTreeV1(
  store: string,
  indexPath: string,
  snapshot: CaptureTreeSnapshotV1,
  signal?: AbortSignal,
): Promise<void> {
  const result = await runGitCommandBytes(store, ['ls-files', '--stage', '-z', '--'], {
    environment: { GIT_INDEX_FILE: indexPath }, signal, maxOutputBytes: 20_000_000,
  });
  if (result.exitCode !== 0 || result.stderr !== '') {
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_INDEX_READ_FAILED');
  }
  const text = decodeUtf8(result.stdout);
  const records = text.split('\0');
  if (records.at(-1) !== '') {
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_INDEX_INVALID');
  }
  const actual = records.slice(0, -1).map((record) => {
    const match = INDEX_RECORD.exec(record);
    if (match === null) throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_INDEX_INVALID');
    const path = normalizeWorkspacePath(match[3], 'private source index path');
    return `${match[1]} ${match[2]} 0\t${path}`;
  });
  const expected = privateSourceFiles(snapshot)
    .map(({ mode, gitObjectId, path }) => `${mode} ${gitObjectId} 0\t${path}`);
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_INDEX_TREE_MISMATCH');
  }
}

export function privateSourceDirectoriesV1(snapshot: CaptureTreeSnapshotV1): string[] {
  const directories = new Set<string>();
  for (const { path } of privateSourceFiles(snapshot)) {
    const parts = path.split('/');
    for (let index = 1; index < parts.length; index += 1) {
      directories.add(parts.slice(0, index).join('/'));
    }
  }
  return [...directories].sort(compareUtf8);
}

export function pinPrivateSourceDirectoryV1(
  path: string,
  mode: number,
  error: string,
): PinnedSourceIdentityV1 {
  return pinPath(path, mode, 'directory', error);
}

export function pinPrivateSourceFileV1(
  path: string,
  mode: number,
  error: string,
): PinnedSourceIdentityV1 {
  return pinPath(path, mode, 'file', error);
}

export function assertPrivateSourceIdentityV1(
  expected: PinnedSourceIdentityV1,
  error: string,
): void {
  let current: PinnedSourceIdentityV1;
  try {
    current = pinPath(
      expected.path,
      Number(expected.mode & 0o7777n),
      expected.kind,
      error,
    );
  } catch (cause) {
    throw new Error(`HARNESS_CAPTURE_PRIVATE_SOURCE_${error}`, { cause });
  }
  if (!sameIdentity(expected, current)) {
    throw new Error(`HARNESS_CAPTURE_PRIVATE_SOURCE_${error}`);
  }
}

export function assertPrivateSourceRootEntriesV1(root: string): void {
  const entries = readdirSync(root, { withFileTypes: true })
    .map(({ name }) => name).sort(compareUtf8);
  if (JSON.stringify(entries) !== JSON.stringify(['index', 'source'])) {
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_ROOT_ENTRY_MISMATCH');
  }
}

export function stablePrivateSourceFileDigestV1(path: string): string {
  const before = lstatSync(path, { bigint: true });
  if (!before.isFile() || before.isSymbolicLink() || before.nlink !== 1n
    || before.uid !== currentUid() || before.size > BigInt(MAX_INDEX_BYTES)
    || realpathSync(path) !== path) {
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_INDEX_INVALID');
  }
  return hashStableFile(path, before, 'sha256', undefined, 'INDEX_CHANGED');
}

function privateSourceFiles(snapshot: CaptureTreeSnapshotV1): CaptureTreeEntryV1[] {
  return [...snapshot.entries.values()].filter(({ type }) => type === 'blob')
    .sort((left, right) => compareUtf8(left.path, right.path));
}

function fileIdentity(
  path: string,
  entry: CaptureTreeEntryV1,
  objectFormat: 'sha1' | 'sha256',
): ProgrammeCapturePrivateSourceFileV1 {
  const expectedMode = entry.mode === '100755' ? 0o500n : 0o400n;
  const before = lstatSync(path, { bigint: true });
  if (!before.isFile() || before.isSymbolicLink() || before.nlink !== 1n
    || before.uid !== currentUid() || (before.mode & 0o7777n) !== expectedMode
    || before.size > BigInt(MAX_SOURCE_BYTES) || realpathSync(path) !== path) {
    throw new Error(`HARNESS_CAPTURE_PRIVATE_SOURCE_FILE_INVALID:${entry.path}`);
  }
  const sha256 = createHash('sha256');
  const git = createHash(objectFormat).update(`blob ${String(before.size)}\0`, 'utf8');
  const gitObjectId = hashStableFile(
    path, before, git, sha256, `FILE_CHANGED:${entry.path}`,
  );
  if (gitObjectId !== entry.gitObjectId) {
    throw new Error(`HARNESS_CAPTURE_PRIVATE_SOURCE_BLOB_MISMATCH:${entry.path}`);
  }
  return Object.freeze({
    path: entry.path,
    gitMode: entry.mode as '100644' | '100755',
    gitObjectId,
    sha256: sha256.digest('hex'),
    byteLength: Number(before.size),
  });
}

function hashStableFile(
  path: string,
  before: BigIntStats,
  primary: string | ReturnType<typeof createHash>,
  secondary: ReturnType<typeof createHash> | undefined,
  error: string,
): string {
  const hash = typeof primary === 'string' ? createHash(primary) : primary;
  const descriptor = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const opened = fstatSync(descriptor, { bigint: true });
    if (!sameIdentity(before, opened)) {
      throw new Error(`HARNESS_CAPTURE_PRIVATE_SOURCE_${error}`);
    }
    const buffer = Buffer.allocUnsafe(1024 * 1024);
    let offset = 0n;
    while (offset < opened.size) {
      const count = readSync(
        descriptor,
        buffer,
        0,
        Math.min(buffer.length, Number(opened.size - offset)),
        Number(offset),
      );
      if (count < 1) throw new Error(`HARNESS_CAPTURE_PRIVATE_SOURCE_${error}`);
      const chunk = buffer.subarray(0, count);
      hash.update(chunk);
      secondary?.update(chunk);
      offset += BigInt(count);
    }
    const after = fstatSync(descriptor, { bigint: true });
    const named = lstatSync(path, { bigint: true });
    if (offset !== opened.size || !sameIdentity(opened, after) || !sameIdentity(after, named)) {
      throw new Error(`HARNESS_CAPTURE_PRIVATE_SOURCE_${error}`);
    }
    return hash.digest('hex');
  } finally {
    closeSync(descriptor);
  }
}

function assertExactSourcePaths(root: string, snapshot: CaptureTreeSnapshotV1): void {
  const files: string[] = [];
  const directories: string[] = [];
  const visit = (directory: string): void => {
    const relativeDirectory = relative(root, directory).split(sep).join('/');
    const stat = lstatSync(directory, { bigint: true });
    if (!stat.isDirectory() || stat.isSymbolicLink() || stat.uid !== currentUid()
      || (stat.mode & 0o7777n) !== 0o500n || realpathSync(directory) !== directory) {
      throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_DIRECTORY_INVALID');
    }
    if (relativeDirectory !== '') directories.push(relativeDirectory);
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      const relativePath = relative(root, path).split(sep).join('/');
      normalizeWorkspacePath(relativePath, 'private source path');
      if (entry.isDirectory() && !entry.isSymbolicLink()) visit(path);
      else if (entry.isFile() && !entry.isSymbolicLink()) files.push(relativePath);
      else throw new Error(`HARNESS_CAPTURE_PRIVATE_SOURCE_ENTRY_INVALID:${relativePath}`);
    }
  };
  visit(root);
  files.sort(compareUtf8);
  directories.sort(compareUtf8);
  if (JSON.stringify(files) !== JSON.stringify(privateSourceFiles(snapshot).map(({ path }) => path))
    || JSON.stringify(directories) !== JSON.stringify(privateSourceDirectoriesV1(snapshot))) {
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_PATH_SET_MISMATCH');
  }
}

function pinPath(
  path: string,
  mode: number,
  kind: 'directory' | 'file',
  error: string,
): PinnedSourceIdentityV1 {
  requirePlatform();
  if (!isAbsolute(path) || resolve(path) !== path || path.includes('\0')) {
    throw new Error(`HARNESS_CAPTURE_PRIVATE_SOURCE_${error}`);
  }
  assertProtectedParentChain(path);
  const stat = lstatSync(path, { bigint: true });
  if ((kind === 'directory' ? !stat.isDirectory() : !stat.isFile()) || stat.isSymbolicLink()
    || stat.uid !== currentUid() || (kind === 'file' && stat.nlink !== 1n)
    || stat.nlink < 1n || (stat.mode & 0o7777n) !== BigInt(mode)
    || realpathSync(path) !== path) {
    throw new Error(`HARNESS_CAPTURE_PRIVATE_SOURCE_${error}`);
  }
  return Object.freeze({
    path, kind, dev: stat.dev, ino: stat.ino, mode: stat.mode, uid: stat.uid,
    nlink: stat.nlink, size: stat.size, mtimeNs: stat.mtimeNs, ctimeNs: stat.ctimeNs,
  });
}

function assertProtectedParentChain(path: string): void {
  let current = dirname(path);
  while (true) {
    const stat = lstatSync(current, { bigint: true });
    const writable = (stat.mode & 0o022n) !== 0n;
    const sticky = (stat.mode & 0o1000n) !== 0n;
    if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(current) !== current
      || (stat.uid !== 0n && stat.uid !== currentUid()) || (writable && !sticky)) {
      throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_PARENT_INVALID');
    }
    const parent = dirname(current);
    if (parent === current) return;
    current = parent;
  }
}

function decodeUtf8(bytes: Buffer, error = 'INDEX_INVALID'): string {
  try {
    const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
    if (!Buffer.from(text, 'utf8').equals(bytes)) throw new Error();
    return text;
  } catch (cause) {
    throw new Error(`HARNESS_CAPTURE_PRIVATE_SOURCE_${error}`, { cause });
  }
}

function sameIdentity(
  left: PinnedSourceIdentityV1 | BigIntStats,
  right: PinnedSourceIdentityV1 | BigIntStats,
): boolean {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.uid === right.uid && left.nlink === right.nlink && left.size === right.size
    && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}

function currentUid(): bigint {
  if (typeof process.getuid !== 'function') {
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_PLATFORM_UNAVAILABLE');
  }
  return BigInt(process.getuid());
}

function requirePlatform(): void {
  if (process.platform !== 'linux' || typeof constants.O_NOFOLLOW !== 'number'
    || typeof constants.O_DIRECTORY !== 'number') {
    throw new Error('HARNESS_CAPTURE_PRIVATE_SOURCE_PLATFORM_UNAVAILABLE');
  }
}

function compareUtf8(left: string, right: string): number {
  return Buffer.compare(Buffer.from(left, 'utf8'), Buffer.from(right, 'utf8'));
}
