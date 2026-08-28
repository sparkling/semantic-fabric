// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { lstatSync, realpathSync } from 'node:fs';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { normalizeWorkspacePath } from './contracts.js';
import { runGitCommandBytes } from './git-process.js';

const GIT_OBJECT = /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/;
const TREE_RECORD = /^([0-9]{6}) (blob|tree|commit) ([a-f0-9]{40}|[a-f0-9]{64})\t(.+)$/;
const MAX_TREE_LISTING_BYTES = 20_000_000;
const MAX_TREE_ENTRIES = 100_000;

export interface CaptureBlobIdentityV1 {
  readonly path: string;
  readonly gitBlobId: string;
  readonly sha256: string;
  readonly byteLength: number;
}

interface PinnedDirectory {
  readonly path: string;
  readonly dev: bigint;
  readonly ino: bigint;
  readonly mode: bigint;
  readonly uid: bigint;
}

export interface CaptureControllerStoreV1 {
  readonly path: string;
  readonly bare: boolean;
  readonly objectFormat: 'sha1' | 'sha256';
  readonly root: PinnedDirectory;
  readonly gitDirectory: PinnedDirectory;
  readonly commonDirectory: PinnedDirectory;
  readonly objectDirectory: PinnedDirectory;
}

export interface CaptureTreeEntryV1 {
  readonly mode: string;
  readonly type: string;
  readonly gitObjectId: string;
  readonly path: string;
}

export interface CaptureTreeSnapshotV1 {
  readonly entries: ReadonlyMap<string, CaptureTreeEntryV1>;
  readonly listingDigest: string;
}

export async function openCaptureControllerStoreV1(
  path: string,
  signal?: AbortSignal,
): Promise<CaptureControllerStoreV1> {
  const root = pinDirectory(path);
  const bareValue = await gitValue(path, ['rev-parse', '--is-bare-repository'], signal);
  if (bareValue !== 'true' && bareValue !== 'false') {
    throw new Error('HARNESS_CAPTURE_CONTROLLER_STORE_LAYOUT_INVALID');
  }
  const bare = bareValue === 'true';
  const objectFormatValue = await gitValue(
    path, ['rev-parse', '--show-object-format=storage'], signal,
  );
  if (objectFormatValue !== 'sha1' && objectFormatValue !== 'sha256') {
    throw new Error('HARNESS_CAPTURE_CONTROLLER_OBJECT_FORMAT_INVALID');
  }
  const gitDirectory = pinDirectory(await gitValue(
    path, ['rev-parse', '--path-format=absolute', '--absolute-git-dir'], signal,
  ));
  const commonDirectory = pinDirectory(await gitValue(
    path, ['rev-parse', '--path-format=absolute', '--git-common-dir'], signal,
  ));
  const objectDirectory = pinDirectory(await gitValue(
    path, ['rev-parse', '--path-format=absolute', '--git-path', 'objects'], signal,
  ));
  if (bare) {
    if (gitDirectory.path !== root.path || commonDirectory.path !== root.path) {
      throw new Error('HARNESS_CAPTURE_CONTROLLER_STORE_ROOT_MISMATCH');
    }
  } else {
    const topLevel = await gitValue(path, ['rev-parse', '--show-toplevel'], signal);
    if (topLevel !== root.path || gitDirectory.path !== join(root.path, '.git')
      || commonDirectory.path !== gitDirectory.path) {
      throw new Error('HARNESS_CAPTURE_CONTROLLER_STORE_ROOT_MISMATCH');
    }
  }
  if (objectDirectory.path !== join(commonDirectory.path, 'objects')) {
    throw new Error('HARNESS_CAPTURE_CONTROLLER_OBJECT_STORE_INVALID');
  }
  const store = Object.freeze({
    path: root.path,
    bare,
    objectFormat: objectFormatValue,
    root,
    gitDirectory,
    commonDirectory,
    objectDirectory,
  });
  assertPinnedDirectory(root);
  return store;
}

export async function assertCaptureControllerStoreStableV1(
  expected: CaptureControllerStoreV1,
  signal?: AbortSignal,
): Promise<void> {
  const current = await openCaptureControllerStoreV1(expected.path, signal);
  if (current.bare !== expected.bare
    || current.objectFormat !== expected.objectFormat
    || !sameDirectory(current.root, expected.root)
    || !sameDirectory(current.gitDirectory, expected.gitDirectory)
    || !sameDirectory(current.commonDirectory, expected.commonDirectory)
    || !sameDirectory(current.objectDirectory, expected.objectDirectory)) {
    throw new Error('HARNESS_CAPTURE_CONTROLLER_STORE_CHANGED');
  }
}

export async function readCaptureCommitTreeV1(
  root: string,
  commit: string,
  tree: string,
  objectFormat: 'sha1' | 'sha256',
  signal?: AbortSignal,
): Promise<CaptureTreeSnapshotV1> {
  const objectLength = objectFormat === 'sha1' ? 40 : 64;
  if (commit.length !== objectLength || tree.length !== objectLength) {
    throw new Error('HARNESS_CAPTURE_CONTROLLER_OBJECT_FORMAT_MISMATCH');
  }
  const listing = await gitBytes(
    root,
    ['ls-tree', '-t', '-r', '-z', '--full-tree', commit],
    signal,
    MAX_TREE_LISTING_BYTES,
  );
  const snapshot = parseCaptureTreeListingV1(listing, objectLength);
  const treeObjects = [...new Set([
    tree,
    ...[...snapshot.entries.values()]
      .filter(({ type }) => type === 'tree')
      .map(({ gitObjectId }) => gitObjectId),
  ])];
  const objects = await readVerifiedObjects(
    root,
    [
      { id: commit, type: 'commit' },
      ...treeObjects.map((id) => ({ id, type: 'tree' as const })),
    ],
    MAX_TREE_LISTING_BYTES,
    MAX_TREE_LISTING_BYTES,
    objectFormat,
    signal,
  );
  const commitBody = objects.get(commit);
  if (commitBody === undefined || firstLine(commitBody) !== `tree ${tree}`) {
    throw new Error('HARNESS_CAPTURE_COMMIT_TREE_BINDING_INVALID');
  }
  return snapshot;
}

export function parseCaptureTreeListingV1(
  listing: Buffer,
  objectLength: number,
): CaptureTreeSnapshotV1 {
  const text = decodeUtf8(listing, 'tree');
  const records = text.split('\0');
  if (records.at(-1) !== '' || records.length - 1 > MAX_TREE_ENTRIES) {
    throw new Error('HARNESS_CAPTURE_COMMIT_TREE_INVALID');
  }
  const entries = new Map<string, CaptureTreeEntryV1>();
  for (const record of records.slice(0, -1)) {
    const match = TREE_RECORD.exec(record);
    if (match === null || match[3].length !== objectLength) {
      throw new Error('HARNESS_CAPTURE_COMMIT_TREE_INVALID');
    }
    const path = normalizeWorkspacePath(match[4], 'capture commit tree path');
    if (entries.has(path)) throw new Error('HARNESS_CAPTURE_COMMIT_TREE_DUPLICATE_PATH');
    entries.set(path, Object.freeze({
      mode: match[1],
      type: match[2],
      gitObjectId: match[3],
      path,
    }));
  }
  return Object.freeze({
    entries,
    listingDigest: createHash('sha256').update(listing).digest('hex'),
  });
}

export async function readCaptureCommitBlobsV1(
  root: string,
  tree: CaptureTreeSnapshotV1,
  paths: readonly string[],
  maximumBlobBytes: number,
  maximumTotalBytes: number,
  objectFormat: 'sha1' | 'sha256',
  signal?: AbortSignal,
): Promise<ReadonlyArray<Readonly<{ bytes: Buffer; identity: CaptureBlobIdentityV1 }>>> {
  const normalizedPaths = paths.map((path) => normalizeWorkspacePath(
    path, 'capture commit blob path',
  ));
  if (normalizedPaths.length === 0 || new Set(normalizedPaths).size !== normalizedPaths.length) {
    throw new Error('HARNESS_CAPTURE_COMMIT_BLOB_PATHS_INVALID');
  }
  const entries = normalizedPaths.map((path) => {
    const entry = tree.entries.get(path);
    if (entry === undefined || entry.type !== 'blob' || entry.mode !== '100644') {
      throw new Error(`HARNESS_CAPTURE_COMMIT_BLOB_INVALID:${path}`);
    }
    return entry;
  });
  const objects = await readVerifiedObjects(
    root,
    entries.map(({ gitObjectId }) => ({ id: gitObjectId, type: 'blob' as const })),
    maximumBlobBytes,
    maximumTotalBytes,
    objectFormat,
    signal,
  );
  return Object.freeze(entries.map((entry) => {
    const bytes = objects.get(entry.gitObjectId);
    if (bytes === undefined) throw new Error('HARNESS_CAPTURE_COMMIT_BLOB_BATCH_INVALID');
    return Object.freeze({
      bytes,
      identity: Object.freeze({
        path: entry.path,
        gitBlobId: entry.gitObjectId,
        sha256: createHash('sha256').update(bytes).digest('hex'),
        byteLength: bytes.length,
      }),
    });
  }));
}

export type GitObjectTypeV1 = 'blob' | 'tree' | 'commit';
export interface GitObjectRequestV1 {
  readonly id: string;
  readonly type: GitObjectTypeV1;
}

async function readVerifiedObjects(
  root: string,
  requested: readonly GitObjectRequestV1[],
  maximumObjectBytes: number,
  maximumTotalBytes: number,
  objectFormat: 'sha1' | 'sha256',
  signal?: AbortSignal,
): Promise<ReadonlyMap<string, Buffer>> {
  const unique = new Map<string, GitObjectTypeV1>();
  const objectLength = objectFormat === 'sha1' ? 40 : 64;
  for (const { id, type } of requested) {
    if (!GIT_OBJECT.test(id) || id.length !== objectLength
      || (unique.has(id) && unique.get(id) !== type)) {
      throw new Error('HARNESS_CAPTURE_GIT_OBJECT_REQUEST_INVALID');
    }
    unique.set(id, type);
  }
  const requests = [...unique.entries()];
  const result = await runGitCommandBytes(root, ['cat-file', '--batch'], {
    signal,
    stdin: `${requests.map(([id]) => id).join('\n')}\n`,
    maxOutputBytes: maximumTotalBytes + (requests.length * 200) + 4096,
  });
  if (result.exitCode !== 0) throw new Error('HARNESS_CAPTURE_GIT_COMMAND_FAILED:cat-file');
  if (result.stderr !== '') throw new Error('HARNESS_CAPTURE_GIT_COMMAND_STDERR:cat-file');
  return parseVerifiedGitBatchV1(
    result.stdout,
    requests.map(([id, type]) => ({ id, type })),
    maximumObjectBytes,
    maximumTotalBytes,
    objectFormat,
  );
}

export function parseVerifiedGitBatchV1(
  stdout: Buffer,
  requests: readonly GitObjectRequestV1[],
  maximumObjectBytes: number,
  maximumTotalBytes: number,
  objectFormat: 'sha1' | 'sha256',
): ReadonlyMap<string, Buffer> {
  const objects = new Map<string, Buffer>();
  if (new Set(requests.map(({ id }) => id)).size !== requests.length) {
    throw new Error('HARNESS_CAPTURE_GIT_OBJECT_REQUEST_INVALID');
  }
  const objectLength = objectFormat === 'sha1' ? 40 : 64;
  let offset = 0;
  let totalBytes = 0;
  for (const { id: expectedId, type: expectedType } of requests) {
    if (!GIT_OBJECT.test(expectedId) || expectedId.length !== objectLength) {
      throw new Error('HARNESS_CAPTURE_GIT_OBJECT_REQUEST_INVALID');
    }
    const lineEnd = stdout.indexOf(0x0a, offset);
    if (lineEnd === -1) throw new Error('HARNESS_CAPTURE_GIT_OBJECT_BATCH_INVALID');
    const headerBytes = stdout.subarray(offset, lineEnd);
    if (headerBytes.some((byte) => byte > 0x7f)) {
      throw new Error('HARNESS_CAPTURE_GIT_OBJECT_BATCH_INVALID');
    }
    const header = headerBytes.toString('ascii');
    const match = /^([a-f0-9]{40}|[a-f0-9]{64}) (blob|tree|commit) (0|[1-9][0-9]*)$/.exec(header);
    if (match === null || match[1] !== expectedId || match[2] !== expectedType) {
      throw new Error('HARNESS_CAPTURE_GIT_OBJECT_BATCH_INVALID');
    }
    const byteLength = Number(match[3]);
    totalBytes += byteLength;
    if (!Number.isSafeInteger(byteLength) || byteLength > maximumObjectBytes
      || totalBytes > maximumTotalBytes) {
      throw new Error('HARNESS_CAPTURE_GIT_OBJECT_SIZE_INVALID');
    }
    const start = lineEnd + 1;
    const end = start + byteLength;
    if (end >= stdout.length || stdout[end] !== 0x0a) {
      throw new Error('HARNESS_CAPTURE_GIT_OBJECT_SIZE_CHANGED');
    }
    const body = Buffer.from(stdout.subarray(start, end));
    if (gitObjectId(expectedType, body, objectFormat) !== expectedId) {
      throw new Error('HARNESS_CAPTURE_GIT_OBJECT_ID_MISMATCH');
    }
    objects.set(expectedId, body);
    offset = end + 1;
  }
  if (offset !== stdout.length) {
    throw new Error('HARNESS_CAPTURE_GIT_OBJECT_BATCH_TRAILING_BYTES');
  }
  return objects;
}

function gitObjectId(
  type: GitObjectTypeV1,
  body: Buffer,
  objectFormat: 'sha1' | 'sha256',
): string {
  return createHash(objectFormat)
    .update(`${type} ${body.length}\0`, 'utf8')
    .update(body)
    .digest('hex');
}

async function gitValue(root: string, args: readonly string[], signal?: AbortSignal) {
  return (await gitBytes(root, args, signal, 4096)).toString('utf8').trim();
}

async function gitBytes(
  root: string,
  args: readonly string[],
  signal: AbortSignal | undefined,
  maxOutputBytes: number,
): Promise<Buffer> {
  const result = await runGitCommandBytes(root, args, { signal, maxOutputBytes });
  if (result.exitCode !== 0) {
    throw new Error(`HARNESS_CAPTURE_GIT_COMMAND_FAILED:${args[0] ?? 'unknown'}`);
  }
  if (result.stderr !== '') {
    throw new Error(`HARNESS_CAPTURE_GIT_COMMAND_STDERR:${args[0] ?? 'unknown'}`);
  }
  return result.stdout;
}

function decodeUtf8(bytes: Buffer, label: string): string {
  let text: string;
  try {
    text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch {
    throw new Error(`HARNESS_CAPTURE_${label.toUpperCase()}_NOT_UTF-8`);
  }
  if (!Buffer.from(text, 'utf8').equals(bytes)) {
    throw new Error(`HARNESS_CAPTURE_${label.toUpperCase()}_NOT_CANONICAL_UTF-8`);
  }
  return text;
}

function firstLine(bytes: Buffer): string {
  const newline = bytes.indexOf(0x0a);
  if (newline < 0) throw new Error('HARNESS_CAPTURE_GIT_OBJECT_INVALID');
  return bytes.subarray(0, newline).toString('ascii');
}

function pinDirectory(path: string): PinnedDirectory {
  if (!isAbsolute(path) || resolve(path) !== path || path.includes('\0')) {
    throw new Error('HARNESS_CAPTURE_CONTROLLER_STORE_INVALID');
  }
  assertProtectedParentChain(path);
  const stat = lstatSync(path, { bigint: true });
  const currentUid = BigInt(typeof process.getuid === 'function' ? process.getuid() : Number(stat.uid));
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(path) !== path
    || (stat.mode & 0o022n) !== 0n || (stat.uid !== 0n && stat.uid !== currentUid)) {
    throw new Error('HARNESS_CAPTURE_CONTROLLER_STORE_INVALID');
  }
  return Object.freeze({
    path,
    dev: stat.dev,
    ino: stat.ino,
    mode: stat.mode,
    uid: stat.uid,
  });
}

function assertProtectedParentChain(path: string): void {
  let current = dirname(path);
  const currentUid = BigInt(typeof process.getuid === 'function' ? process.getuid() : 0);
  while (true) {
    const stat = lstatSync(current, { bigint: true });
    const writableByOthers = (stat.mode & 0o022n) !== 0n;
    const sticky = (stat.mode & 0o1000n) !== 0n;
    if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(current) !== current
      || (stat.uid !== 0n && stat.uid !== currentUid) || (writableByOthers && !sticky)) {
      throw new Error('HARNESS_CAPTURE_CONTROLLER_STORE_INVALID');
    }
    const parent = dirname(current);
    if (parent === current) return;
    current = parent;
  }
}

function assertPinnedDirectory(expected: PinnedDirectory): void {
  if (!sameDirectory(pinDirectory(expected.path), expected)) {
    throw new Error('HARNESS_CAPTURE_CONTROLLER_STORE_CHANGED');
  }
}

function sameDirectory(left: PinnedDirectory, right: PinnedDirectory): boolean {
  return left.path === right.path && left.dev === right.dev && left.ino === right.ino
    && left.mode === right.mode && left.uid === right.uid;
}
