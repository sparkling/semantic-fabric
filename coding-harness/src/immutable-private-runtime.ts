// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  chmodSync,
  closeSync,
  constants,
  copyFileSync,
  existsSync,
  fstatSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readSync,
  readdirSync,
  realpathSync,
  rmSync,
  statSync,
  symlinkSync,
} from 'node:fs';
import { dirname, isAbsolute, join, posix, relative, resolve, sep } from 'node:path';
import { SHA256_PATTERN, deepFreeze } from './contracts.js';
import {
  assertImmutablePrivateTreeOverridesStable,
  captureImmutablePrivateTreeOverrides,
  immutablePrivateTreeOverrideCount,
  immutablePrivateTreeOverrideMetadata,
  writeImmutablePrivateTreeOverride,
  type ImmutablePrivateTreeOverrideManifestSpec,
  type ImmutablePrivateTreeOverrideSnapshot,
} from './immutable-private-tree-overlay.js';

export interface ImmutablePrivateFileSpec {
  readonly key: string;
  readonly sourcePath: string;
  readonly relativePath: string;
  readonly executable: boolean;
  readonly expectedDigest?: string;
  readonly maxBytes?: number;
  readonly sourcePolicy?: 'immutable' | 'same-principal-cooperative-snapshot';
}

export interface ImmutablePrivateTreeSpec {
  readonly key: string;
  readonly sourceRoot: string;
  readonly relativePath: string;
  readonly maxFiles: number;
  readonly maxBytes: number;
  readonly overrideManifest?: ImmutablePrivateTreeOverrideManifestSpec;
}

export interface ImmutablePrivateLinkSpec {
  readonly relativePath: string;
  readonly targetPath: string;
}

export interface ImmutablePrivateFileIdentity {
  readonly sourcePath: string;
  readonly path: string;
  readonly digest: string;
}

export interface ImmutablePrivateTreeIdentity {
  readonly sourceRoot: string;
  readonly path: string;
  readonly digest: string;
  readonly fileCount: number;
  readonly totalBytes: number;
}

export interface ImmutablePrivateRuntime {
  readonly root: string;
  readonly files: Readonly<Record<string, ImmutablePrivateFileIdentity>>;
  readonly trees: Readonly<Record<string, ImmutablePrivateTreeIdentity>>;
  cleanup(): void;
}

interface TreeFile {
  readonly relativePath: string;
  readonly digest: string;
  readonly executable: boolean;
  readonly size: number;
}

interface TreeSnapshot {
  readonly directories: readonly string[];
  readonly files: readonly TreeFile[];
  readonly digest: string;
  readonly totalBytes: number;
}

const DEFAULT_MAX_FILE_BYTES = 500_000_000;
const KEY = /^[A-Za-z][A-Za-z0-9_-]{0,63}$/;

export function createImmutablePrivateRuntime(input: Readonly<{
  parent: string;
  prefix: string;
  files?: readonly ImmutablePrivateFileSpec[];
  trees?: readonly ImmutablePrivateTreeSpec[];
  directories?: readonly string[];
  links?: readonly ImmutablePrivateLinkSpec[];
}>): ImmutablePrivateRuntime {
  const parent = canonicalDirectory(input.parent, 'PARENT');
  if (!/^[a-z0-9][a-z0-9-]{0,47}-$/.test(input.prefix)) {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_PREFIX_INVALID');
  }
  const root = mkdtempSync(join(parent, input.prefix));
  chmodSync(root, 0o700);
  let cleaned = false;
  try {
    const occupied = new Set<string>();
    const fileIdentities: Record<string, ImmutablePrivateFileIdentity> = {};
    const copiedFiles: Array<Readonly<{
      spec: ImmutablePrivateFileSpec;
      sourcePolicy: NonNullable<ImmutablePrivateFileSpec['sourcePolicy']>;
    }>> = [];
    for (const spec of input.files ?? []) {
      uniqueKey(spec.key, fileIdentities);
      const target = targetPath(root, spec.relativePath, occupied);
      mkdirSync(dirname(target), { recursive: true, mode: 0o700 });
      const sourcePolicy = fileSourcePolicy(spec.sourcePolicy);
      const expectedDigest = spec.expectedDigest ?? stableFileDigest(
        spec.sourcePath,
        spec.executable,
        spec.maxBytes ?? DEFAULT_MAX_FILE_BYTES,
        sourcePolicy,
      );
      fileIdentities[spec.key] = copyStableFile(
        spec.sourcePath,
        target,
        spec.executable,
        spec.maxBytes ?? DEFAULT_MAX_FILE_BYTES,
        expectedDigest,
        sourcePolicy,
      );
      copiedFiles.push({ spec, sourcePolicy });
    }
    const treeIdentities: Record<string, ImmutablePrivateTreeIdentity> = {};
    for (const spec of input.trees ?? []) {
      uniqueKey(spec.key, treeIdentities);
      const treeTarget = targetPath(root, spec.relativePath, occupied);
      const bounds = { maxFiles: spec.maxFiles, maxBytes: spec.maxBytes };
      const overridesBefore = captureImmutablePrivateTreeOverrides(spec.overrideManifest, bounds);
      const before = captureTree(spec, overridesBefore);
      mkdirSync(treeTarget, { recursive: true, mode: 0o700 });
      for (const directory of before.directories) {
        mkdirSync(join(treeTarget, directory), { recursive: true, mode: 0o700 });
      }
      for (const file of before.files) {
        const target = join(treeTarget, file.relativePath);
        occupied.add(relative(root, target));
        mkdirSync(dirname(target), { recursive: true, mode: 0o700 });
        const override = immutablePrivateTreeOverrideMetadata(
          overridesBefore, file.relativePath,
        );
        if (override === undefined) {
          copyStableFile(
            join(spec.sourceRoot, file.relativePath),
            target,
            file.executable,
            Math.max(1, file.size),
            file.digest,
          );
        } else {
          writeImmutablePrivateTreeOverride(target, overridesBefore, file.relativePath);
        }
      }
      const overridesAfter = captureImmutablePrivateTreeOverrides(spec.overrideManifest, bounds);
      const after = captureTree(spec, overridesAfter);
      if (after.digest !== before.digest) throw new Error('HARNESS_IMMUTABLE_RUNTIME_TREE_CHANGED');
      assertImmutablePrivateTreeOverridesStable(overridesBefore, overridesAfter);
      const materialized = captureTree(
        { ...spec, sourceRoot: treeTarget, overrideManifest: undefined },
        captureImmutablePrivateTreeOverrides(undefined, bounds),
      );
      if (materialized.digest !== before.digest
        || materialized.totalBytes !== before.totalBytes
        || materialized.files.length !== before.files.length) {
        throw new Error('HARNESS_IMMUTABLE_RUNTIME_COPY_INVALID');
      }
      treeIdentities[spec.key] = Object.freeze({
        sourceRoot: spec.sourceRoot,
        path: treeTarget,
        digest: before.digest,
        fileCount: before.files.length,
        totalBytes: before.totalBytes,
      });
    }
    for (const { spec, sourcePolicy } of copiedFiles) {
      const copied = fileIdentities[spec.key];
      const currentDigest = stableFileDigest(
        spec.sourcePath, spec.executable,
        spec.maxBytes ?? DEFAULT_MAX_FILE_BYTES, sourcePolicy,
      );
      if (copied === undefined || currentDigest !== copied.digest) {
        throw new Error('HARNESS_IMMUTABLE_RUNTIME_SOURCE_CHANGED');
      }
    }
    for (const directory of input.directories ?? []) {
      mkdirSync(targetPath(root, directory, occupied), { recursive: true, mode: 0o700 });
    }
    for (const link of input.links ?? []) {
      const destination = targetPath(root, link.relativePath, occupied);
      const target = canonicalDirectory(link.targetPath, 'LINK_TARGET');
      const stat = lstatSync(target);
      if ((stat.mode & 0o022) !== 0) throw new Error('HARNESS_IMMUTABLE_RUNTIME_LINK_TARGET_UNTRUSTED');
      mkdirSync(dirname(destination), { recursive: true, mode: 0o700 });
      symlinkSync(target, destination, 'dir');
    }
    sealDirectories(root);
    const cleanup = () => {
      if (cleaned) return;
      cleaned = true;
      unsealDirectories(root);
      rmSync(root, { recursive: true, force: true });
    };
    return Object.freeze({
      root,
      files: deepFreeze(fileIdentities),
      trees: deepFreeze(treeIdentities),
      cleanup,
    });
  } catch (error) {
    if (existsSync(root)) {
      unsealDirectories(root);
      rmSync(root, { recursive: true, force: true });
    }
    throw error;
  }
}

function copyStableFile(
  sourcePath: string,
  target: string,
  executable: boolean,
  maxBytes: number,
  expectedDigest?: string,
  sourcePolicy: NonNullable<ImmutablePrivateFileSpec['sourcePolicy']> = 'immutable',
): ImmutablePrivateFileIdentity {
  if (!Number.isSafeInteger(maxBytes) || maxBytes < 1 || maxBytes > DEFAULT_MAX_FILE_BYTES
    || (expectedDigest !== undefined && !SHA256_PATTERN.test(expectedDigest))) {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_FILE_LIMIT_INVALID');
  }
  const before = trustedFile(sourcePath, executable, maxBytes, sourcePolicy);
  const descriptor = openSync(sourcePath, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const opened = fstatSync(descriptor, { bigint: true });
    if (!sameFile(before, opened)) throw new Error('HARNESS_IMMUTABLE_RUNTIME_SOURCE_CHANGED');
    copyFileSync(`/proc/${process.pid}/fd/${descriptor}`, target, constants.COPYFILE_EXCL);
    const after = fstatSync(descriptor, { bigint: true });
    if (!sameFile(opened, after)) throw new Error('HARNESS_IMMUTABLE_RUNTIME_SOURCE_CHANGED');
    const targetStat = lstatSync(target, { bigint: true });
    if (!targetStat.isFile() || targetStat.isSymbolicLink() || targetStat.nlink !== 1n
      || targetStat.size !== opened.size || realpathSync(target) !== target) {
      throw new Error('HARNESS_IMMUTABLE_RUNTIME_COPY_INVALID');
    }
    chmodSync(target, executable ? 0o500 : 0o400);
    const digest = stableFileDigest(target, executable, maxBytes);
    if (expectedDigest !== undefined && digest !== expectedDigest) {
      throw new Error('HARNESS_IMMUTABLE_RUNTIME_DIGEST_MISMATCH');
    }
    return Object.freeze({ sourcePath, path: target, digest });
  } finally {
    closeSync(descriptor);
  }
}

function captureTree(
  spec: ImmutablePrivateTreeSpec,
  overrides: ImmutablePrivateTreeOverrideSnapshot,
): TreeSnapshot {
  if (!Number.isSafeInteger(spec.maxFiles) || spec.maxFiles < 1
    || !Number.isSafeInteger(spec.maxBytes) || spec.maxBytes < 1) {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_TREE_LIMIT_INVALID');
  }
  const root = canonicalDirectory(spec.sourceRoot, 'TREE_ROOT');
  const directories: string[] = [];
  const files: TreeFile[] = [];
  const consumedOverrides = new Set<string>();
  let totalBytes = 0;
  const visit = (directory: string) => {
    trustedDirectory(directory);
    for (const entry of readdirSync(directory, { withFileTypes: true })
      .sort((left, right) => left.name.localeCompare(right.name))) {
      const path = join(directory, entry.name);
      const relativePath = relative(root, path).split(sep).join('/');
      const override = immutablePrivateTreeOverrideMetadata(overrides, relativePath);
      if (override !== undefined) {
        if (!entry.isFile() || entry.isSymbolicLink()) {
          throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_TARGET_NOT_REGULAR');
        }
        consumedOverrides.add(relativePath);
        totalBytes += override.size;
        files.push({
          relativePath,
          digest: override.digest,
          executable: override.executable,
          size: override.size,
        });
        if (files.length > spec.maxFiles || totalBytes > spec.maxBytes) {
          throw new Error('HARNESS_IMMUTABLE_RUNTIME_TREE_LIMIT_EXCEEDED');
        }
      } else if (entry.isDirectory() && !entry.isSymbolicLink()) {
        directories.push(relativePath);
        visit(path);
      } else if (entry.isFile() && !entry.isSymbolicLink()) {
        const stat = lstatSync(path);
        const identity = trustedFile(path, (stat.mode & 0o111) !== 0, DEFAULT_MAX_FILE_BYTES);
        totalBytes += Number(identity.size);
        files.push({
          relativePath,
          digest: stableFileDigest(path, (stat.mode & 0o111) !== 0, DEFAULT_MAX_FILE_BYTES),
          executable: (stat.mode & 0o111) !== 0,
          size: Number(identity.size),
        });
        if (files.length > spec.maxFiles || totalBytes > spec.maxBytes) {
          throw new Error('HARNESS_IMMUTABLE_RUNTIME_TREE_LIMIT_EXCEEDED');
        }
      } else {
        throw new Error('HARNESS_IMMUTABLE_RUNTIME_TREE_UNTRUSTED');
      }
    }
  };
  visit(root);
  if (consumedOverrides.size !== immutablePrivateTreeOverrideCount(overrides)) {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_OVERLAY_TARGET_MISSING');
  }
  const body = { directories, files };
  return Object.freeze({
    ...body,
    digest: createHash('sha256').update(JSON.stringify(body)).digest('hex'),
    totalBytes,
  });
}

function descriptorDigest(descriptor: number): string {
  const stat = fstatSync(descriptor, { bigint: true });
  const hash = createHash('sha256');
  const buffer = Buffer.allocUnsafe(1024 * 1024);
  let offset = 0n;
  while (offset < stat.size) {
    const count = readSync(
      descriptor, buffer, 0,
      Math.min(buffer.length, Number(stat.size - offset)), Number(offset),
    );
    if (count === 0) break;
    hash.update(buffer.subarray(0, count));
    offset += BigInt(count);
  }
  if (offset !== stat.size) throw new Error('HARNESS_IMMUTABLE_RUNTIME_FILE_CHANGED');
  return hash.digest('hex');
}

function stableFileDigest(
  path: string,
  executable: boolean,
  maxBytes: number,
  sourcePolicy: NonNullable<ImmutablePrivateFileSpec['sourcePolicy']> = 'immutable',
): string {
  const before = trustedFile(path, executable, maxBytes, sourcePolicy);
  const descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const opened = fstatSync(descriptor, { bigint: true });
    if (!sameFile(before, opened)) throw new Error('HARNESS_IMMUTABLE_RUNTIME_SOURCE_CHANGED');
    const digest = descriptorDigest(descriptor);
    const after = fstatSync(descriptor, { bigint: true });
    if (!sameFile(opened, after)) throw new Error('HARNESS_IMMUTABLE_RUNTIME_SOURCE_CHANGED');
    return digest;
  } finally {
    closeSync(descriptor);
  }
}

function trustedFile(
  path: string,
  executable: boolean,
  maxBytes: number,
  sourcePolicy: NonNullable<ImmutablePrivateFileSpec['sourcePolicy']> = 'immutable',
) {
  const value = canonicalPath(path, 'SOURCE');
  const stat = lstatSync(value, { bigint: true });
  const uid = BigInt(process.getuid?.() ?? Number(stat.uid));
  const gid = BigInt(process.getgid?.() ?? Number(stat.gid));
  const ownershipInvalid = sourcePolicy === 'same-principal-cooperative-snapshot'
    ? stat.uid !== uid || stat.gid !== gid || (stat.mode & 0o002n) !== 0n
    : (stat.mode & 0o022n) !== 0n || (stat.uid !== 0n && stat.uid !== uid);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1n
    || stat.size < 1n || stat.size > BigInt(maxBytes) || ownershipInvalid
    || (executable && (stat.mode & 0o111n) === 0n)) {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_SOURCE_UNTRUSTED');
  }
  return stat;
}

function fileSourcePolicy(
  value: ImmutablePrivateFileSpec['sourcePolicy'],
): NonNullable<ImmutablePrivateFileSpec['sourcePolicy']> {
  if (value === undefined || value === 'immutable') return 'immutable';
  if (value === 'same-principal-cooperative-snapshot') return value;
  throw new Error('HARNESS_IMMUTABLE_RUNTIME_SOURCE_POLICY_INVALID');
}

function trustedDirectory(path: string): void {
  const value = canonicalDirectory(path, 'TREE_DIRECTORY');
  const stat = lstatSync(value);
  const uid = process.getuid?.() ?? stat.uid;
  if ((stat.mode & 0o022) !== 0 || (stat.uid !== 0 && stat.uid !== uid)) {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_TREE_UNTRUSTED');
  }
}

function sameFile(left: import('node:fs').BigIntStats, right: import('node:fs').BigIntStats): boolean {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.nlink === right.nlink && left.uid === right.uid && left.gid === right.gid
    && left.size === right.size && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}

function targetPath(root: string, raw: string, occupied: Set<string>): string {
  const value = normalizedRelative(raw);
  if (occupied.has(value) || [...occupied].some((path) =>
    path.startsWith(`${value}/`) || value.startsWith(`${path}/`))) {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_TARGET_COLLISION');
  }
  occupied.add(value);
  return join(root, value);
}

function normalizedRelative(value: string): string {
  if (typeof value !== 'string' || value === '' || value.includes('\\') || value.includes('\0')
    || isAbsolute(value) || posix.normalize(value) !== value
    || value.split('/').some((segment) => segment === '' || segment === '.' || segment === '..')) {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_TARGET_INVALID');
  }
  return value;
}

function uniqueKey(key: string, values: Record<string, unknown>): void {
  if (!KEY.test(key) || Object.prototype.hasOwnProperty.call(values, key)) {
    throw new Error('HARNESS_IMMUTABLE_RUNTIME_KEY_INVALID');
  }
}

function canonicalDirectory(value: string, label: string): string {
  const path = canonicalPath(value, label);
  if (!statSync(path).isDirectory()) throw new Error(`HARNESS_IMMUTABLE_RUNTIME_${label}_INVALID`);
  return path;
}

function canonicalPath(value: string, label: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) {
    throw new Error(`HARNESS_IMMUTABLE_RUNTIME_${label}_INVALID`);
  }
  const stat = lstatSync(value);
  if (stat.isSymbolicLink() || realpathSync(value) !== value) {
    throw new Error(`HARNESS_IMMUTABLE_RUNTIME_${label}_INVALID`);
  }
  return value;
}

function sealDirectories(root: string): void {
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    if (entry.isDirectory() && !entry.isSymbolicLink()) sealDirectories(join(root, entry.name));
  }
  chmodSync(root, 0o500);
}

function unsealDirectories(root: string): void {
  chmodSync(root, 0o700);
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    if (entry.isDirectory() && !entry.isSymbolicLink()) unsealDirectories(join(root, entry.name));
  }
}
