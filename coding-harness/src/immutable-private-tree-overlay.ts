// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  closeSync,
  constants,
  fchmodSync,
  fstatSync,
  fsyncSync,
  lstatSync,
  openSync,
  readSync,
  realpathSync,
  writeSync,
  type BigIntStats,
} from 'node:fs';
import { dirname, isAbsolute, join, posix, resolve } from 'node:path';
import { gunzipSync } from 'node:zlib';
import { SHA256_PATTERN } from './contracts.js';

export interface ImmutablePrivateTreeOverrideManifestSpec {
  readonly sourcePath: string;
  readonly expectedDigest: string;
  readonly expectedBytes: number;
}

export interface ImmutablePrivateTreeOverrideBounds {
  readonly maxFiles: number;
  readonly maxBytes: number;
}

export interface ImmutablePrivateTreeOverrideFile {
  readonly relativePath: string;
  readonly digest: string;
  readonly executable: false;
  readonly size: number;
  readonly bytes: Buffer;
}

export interface ImmutablePrivateTreeOverrideSnapshot {
  readonly files: ReadonlyMap<string, ImmutablePrivateTreeOverrideFile>;
  readonly sourceDigest: string;
}

interface OverlayManifest {
  readonly schemaVersion: 1;
  readonly files: readonly OverlayManifestFile[];
}

interface OverlayManifestFile {
  readonly targetPath: string;
  readonly blobPath: string;
  readonly compression: 'gzip';
  readonly compressedSha256: string;
  readonly compressedBytes: number;
  readonly decodedSha256: string;
  readonly decodedBytes: number;
  readonly executable: false;
}

interface ProtectedSource {
  readonly bytes: Buffer;
  readonly identity: Readonly<Record<string, string | number>>;
}

const MAX_MANIFEST_BYTES = 1_048_576;
const MAX_BLOB_BYTES = 64 * 1024 * 1024;
const MAX_DECODED_BYTES = 500_000_000;
const MAX_OVERRIDE_FILES = 64;
const MANIFEST_KEYS = ['schemaVersion', 'files'] as const;
const FILE_KEYS = [
  'targetPath', 'blobPath', 'compression', 'compressedSha256', 'compressedBytes',
  'decodedSha256', 'decodedBytes', 'executable',
] as const;

export function captureImmutablePrivateTreeOverrides(
  spec: ImmutablePrivateTreeOverrideManifestSpec | undefined,
  bounds: ImmutablePrivateTreeOverrideBounds,
): ImmutablePrivateTreeOverrideSnapshot {
  if (spec === undefined) return Object.freeze({ files: new Map(), sourceDigest: '' });
  validateManifestSpec(spec);
  validateBounds(bounds);
  const manifestSource = readProtectedSource(spec.sourcePath, spec.expectedBytes);
  assertSourceBytes(
    manifestSource.bytes, spec.expectedBytes, spec.expectedDigest,
    'MANIFEST_SIZE_MISMATCH', 'MANIFEST_DIGEST_MISMATCH',
  );
  const manifest = parseManifest(manifestSource.bytes);
  const decodedTotal = manifest.files.reduce((total, file) => total + file.decodedBytes, 0);
  if (manifest.files.length > bounds.maxFiles || !Number.isSafeInteger(decodedTotal)
    || decodedTotal > bounds.maxBytes) {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_TREE_LIMIT_EXCEEDED');
  }
  const files = new Map<string, ImmutablePrivateTreeOverrideFile>();
  const blobPaths = new Set<string>();
  const identities: Readonly<Record<string, string | number>>[] = [manifestSource.identity];
  for (const entry of manifest.files) {
    const relativePath = normalizedRelative(entry.targetPath, 'TARGET');
    if (files.has(relativePath)) {
      throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_TARGET_DUPLICATE');
    }
    const blobRelative = normalizedRelative(entry.blobPath, 'BLOB_PATH');
    if (blobPaths.has(blobRelative)) {
      throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_BLOB_DUPLICATE');
    }
    blobPaths.add(blobRelative);
    const blobSource = readProtectedSource(
      join(dirname(spec.sourcePath), blobRelative), entry.compressedBytes,
    );
    assertSourceBytes(
      blobSource.bytes, entry.compressedBytes, entry.compressedSha256,
      'BLOB_SIZE_MISMATCH', 'BLOB_DIGEST_MISMATCH',
    );
    const decoded = boundedGunzip(blobSource.bytes, entry.decodedBytes);
    if (decoded.byteLength !== entry.decodedBytes) {
      throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_DECODED_SIZE_MISMATCH');
    }
    if (sha256(decoded) !== entry.decodedSha256) {
      throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_DECODED_DIGEST_MISMATCH');
    }
    identities.push(blobSource.identity);
    files.set(relativePath, Object.freeze({
      relativePath,
      digest: entry.decodedSha256,
      executable: false,
      size: entry.decodedBytes,
      bytes: decoded,
    }));
  }
  return Object.freeze({
    files,
    sourceDigest: sha256(Buffer.from(JSON.stringify(identities))),
  });
}

function validateBounds(bounds: ImmutablePrivateTreeOverrideBounds): void {
  if (!Number.isSafeInteger(bounds.maxFiles) || bounds.maxFiles < 1
    || !Number.isSafeInteger(bounds.maxBytes) || bounds.maxBytes < 1) {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_TREE_LIMIT_INVALID');
  }
}

export function assertImmutablePrivateTreeOverridesStable(
  before: ImmutablePrivateTreeOverrideSnapshot,
  after: ImmutablePrivateTreeOverrideSnapshot,
): void {
  if (before.sourceDigest !== after.sourceDigest) {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_SOURCE_CHANGED');
  }
}

export function writeImmutablePrivateTreeOverride(
  target: string,
  file: ImmutablePrivateTreeOverrideFile,
): void {
  const descriptor = openSync(
    target,
    constants.O_RDWR | constants.O_CREAT | constants.O_EXCL | (constants.O_NOFOLLOW ?? 0),
    0o400,
  );
  try {
    const opened = fstatSync(descriptor, { bigint: true });
    if (!opened.isFile() || opened.nlink !== 1n || opened.size !== 0n) {
      throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_COPY_INVALID');
    }
    let offset = 0;
    while (offset < file.bytes.byteLength) {
      const count = writeSync(
        descriptor, file.bytes, offset, file.bytes.byteLength - offset, offset,
      );
      if (count === 0) throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_COPY_INVALID');
      offset += count;
    }
    fchmodSync(descriptor, 0o400);
    fsyncSync(descriptor);
    const after = fstatSync(descriptor, { bigint: true });
    if (!sameObject(opened, after) || after.size !== BigInt(file.size)
      || (after.mode & 0o777n) !== 0o400n
      || sha256(readDescriptor(descriptor, file.size)) !== file.digest) {
      throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_COPY_INVALID');
    }
  } finally {
    closeSync(descriptor);
  }
  const targetStat = lstatSync(target, { bigint: true });
  if (!targetStat.isFile() || targetStat.isSymbolicLink() || targetStat.nlink !== 1n
    || targetStat.size !== BigInt(file.size) || realpathSync(target) !== target) {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_COPY_INVALID');
  }
}

function validateManifestSpec(spec: ImmutablePrivateTreeOverrideManifestSpec): void {
  if (!isAbsolute(spec.sourcePath) || resolve(spec.sourcePath) !== spec.sourcePath
    || spec.sourcePath.includes('\0') || !SHA256_PATTERN.test(spec.expectedDigest)
    || !boundedInteger(spec.expectedBytes, MAX_MANIFEST_BYTES)) {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_MANIFEST_SPEC_INVALID');
  }
}

function parseManifest(bytes: Buffer): OverlayManifest {
  let value: unknown;
  let text: string;
  try {
    text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
    value = JSON.parse(text) as unknown;
  } catch {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_MANIFEST_INVALID');
  }
  if (!isRecord(value) || !exactKeys(value, MANIFEST_KEYS)
    || value.schemaVersion !== 1 || !Array.isArray(value.files)
    || value.files.length < 1 || value.files.length > MAX_OVERRIDE_FILES
    || text !== `${JSON.stringify(value, null, 2)}\n`) {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_MANIFEST_INVALID');
  }
  const files = value.files.map(parseManifestFile);
  const sorted = [...files].sort((left, right) => compareUtf8(
    left.targetPath, right.targetPath,
  ));
  if (files.some((entry, index) => entry.targetPath !== sorted[index]?.targetPath)) {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_MANIFEST_ORDER_INVALID');
  }
  return Object.freeze({ schemaVersion: 1, files: Object.freeze(files) });
}

function parseManifestFile(value: unknown): OverlayManifestFile {
  if (!isRecord(value) || !exactKeys(value, FILE_KEYS)
    || typeof value.targetPath !== 'string' || typeof value.blobPath !== 'string'
    || value.compression !== 'gzip'
    || typeof value.compressedSha256 !== 'string'
    || !SHA256_PATTERN.test(value.compressedSha256)
    || !boundedInteger(value.compressedBytes, MAX_BLOB_BYTES)
    || typeof value.decodedSha256 !== 'string'
    || !SHA256_PATTERN.test(value.decodedSha256)
    || !boundedInteger(value.decodedBytes, MAX_DECODED_BYTES)
    || value.executable !== false) {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_MANIFEST_INVALID');
  }
  return Object.freeze({
    targetPath: value.targetPath,
    blobPath: value.blobPath,
    compression: 'gzip',
    compressedSha256: value.compressedSha256,
    compressedBytes: value.compressedBytes,
    decodedSha256: value.decodedSha256,
    decodedBytes: value.decodedBytes,
    executable: false,
  });
}

function readProtectedSource(path: string, maxBytes: number): ProtectedSource {
  if (!isAbsolute(path) || resolve(path) !== path || path.includes('\0')) {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_SOURCE_INVALID');
  }
  let before: BigIntStats;
  try {
    before = lstatSync(path, { bigint: true });
    if (before.isSymbolicLink() || realpathSync(path) !== path) throw new Error('invalid');
  } catch {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_SOURCE_INVALID');
  }
  const uid = BigInt(process.getuid?.() ?? Number(before.uid));
  if (!before.isFile() || before.nlink !== 1n
    || (before.uid !== 0n && before.uid !== uid) || (before.mode & 0o022n) !== 0n
    || (before.mode & 0o111n) !== 0n
    || before.size < 1n || before.size > BigInt(maxBytes)) {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_SOURCE_UNTRUSTED');
  }
  let descriptor: number;
  try {
    descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  } catch {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_SOURCE_INVALID');
  }
  try {
    const opened = fstatSync(descriptor, { bigint: true });
    if (!sameFile(before, opened)) {
      throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_SOURCE_CHANGED');
    }
    const bytes = readDescriptor(descriptor, Number(opened.size));
    const after = fstatSync(descriptor, { bigint: true });
    if (!sameFile(opened, after)) {
      throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_SOURCE_CHANGED');
    }
    return Object.freeze({ bytes, identity: sourceIdentity(path, after, sha256(bytes)) });
  } finally {
    closeSync(descriptor);
  }
}

function assertSourceBytes(
  bytes: Buffer,
  expectedBytes: number,
  expectedDigest: string,
  sizeError: string,
  digestError: string,
): void {
  if (bytes.byteLength !== expectedBytes) {
    throw new Error(`HARNESS_IMMUTABLE_RUNTIME_OVERLAY_${sizeError}`);
  }
  if (sha256(bytes) !== expectedDigest) {
    throw new Error(`HARNESS_IMMUTABLE_RUNTIME_OVERLAY_${digestError}`);
  }
}

function boundedGunzip(bytes: Buffer, maxOutputLength: number): Buffer {
  try {
    return gunzipSync(bytes, { maxOutputLength });
  } catch {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_DECOMPRESSION_FAILED');
  }
}

function readDescriptor(descriptor: number, size: number): Buffer {
  const bytes = Buffer.allocUnsafe(size);
  let offset = 0;
  while (offset < size) {
    const count = readSync(descriptor, bytes, offset, size - offset, offset);
    if (count === 0) throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_SOURCE_CHANGED');
    offset += count;
  }
  return bytes;
}

function sourceIdentity(
  path: string,
  stat: BigIntStats,
  digest: string,
): Readonly<Record<string, string | number>> {
  return Object.freeze({
    path,
    dev: stat.dev.toString(),
    ino: stat.ino.toString(),
    mode: Number(stat.mode & 0o7777n),
    nlink: stat.nlink.toString(),
    uid: stat.uid.toString(),
    gid: stat.gid.toString(),
    size: stat.size.toString(),
    mtimeNs: stat.mtimeNs.toString(),
    ctimeNs: stat.ctimeNs.toString(),
    digest,
  });
}

function sameFile(left: BigIntStats, right: BigIntStats): boolean {
  return sameObject(left, right) && left.mode === right.mode && left.size === right.size
    && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}

function sameObject(left: BigIntStats, right: BigIntStats): boolean {
  return left.dev === right.dev && left.ino === right.ino && left.nlink === right.nlink
    && left.uid === right.uid && left.gid === right.gid;
}

function normalizedRelative(value: string, label: string): string {
  if (value === '' || value.includes('\\') || value.includes('\0') || isAbsolute(value)
    || posix.normalize(value) !== value
    || value.split('/').some((segment) => segment === '' || segment === '.' || segment === '..')) {
    throw new Error(`HARNESS_IMMUTABLE_RUNTIME_OVERLAY_${label}_INVALID`);
  }
  return value;
}

function boundedInteger(value: unknown, max: number): value is number {
  return Number.isSafeInteger(value) && Number(value) > 0 && Number(value) <= max;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function exactKeys(value: Record<string, unknown>, keys: readonly string[]): boolean {
  const actual = Object.keys(value);
  return actual.length === keys.length && actual.every((key, index) => key === keys[index]);
}

function compareUtf8(left: string, right: string): number {
  return Buffer.compare(Buffer.from(left, 'utf8'), Buffer.from(right, 'utf8'));
}

function sha256(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex');
}
