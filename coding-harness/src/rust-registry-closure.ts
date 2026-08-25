// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  chmodSync,
  closeSync,
  constants,
  fstatSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  readSync,
  readdirSync,
  realpathSync,
  writeSync,
  type BigIntStats,
} from 'node:fs';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { TextDecoder } from 'node:util';
import type { FrozenRegistryPackage } from './frozen-cargo-metadata.js';

const CRATES_IO_SOURCE = 'registry+https://github.com/rust-lang/crates.io-index';
const CRATE_NAME = /^[A-Za-z0-9_-]{1,64}$/;
const CRATE_VERSION = /^[0-9A-Za-z][0-9A-Za-z.+-]{0,127}$/;
const SHA256 = /^[a-f0-9]{64}$/;
const SPARSE_HEADER = Buffer.from([3, 2, 0, 0, 0]);
const SEALED_ETAG = 'etag: "semantic-fabric-locked"';
const INDEX_CONFIG_DIGEST = '5b943a2c6f7eb743f7308aba07bdbb47d9ae44aafecd832d7f15df186afbafb3';
const MAX_LOCK_BYTES = 10_000_000;
const MAX_INDEX_BYTES = 20_000_000;
const MAX_ENTRIES = 100_000;
const MAX_BYTES = 3_000_000_000;

interface LockedPackage extends FrozenRegistryPackage {
  readonly checksum: string;
}

export interface LockedRustRegistryClosure {
  readonly registryRoot: string;
  readonly evidence: Readonly<Record<string, string>>;
  assertStable(): void;
}

export function prepareLockedRustRegistryClosure(input: Readonly<{
  snapshotRegistryRoot: string;
  destinationRoot: string;
  registryKey: string;
  lockfilePath: string;
  lockfileDigest: string;
  packages: readonly FrozenRegistryPackage[];
  targetTriple: string;
  expectedContentDigest: string;
}>): LockedRustRegistryClosure {
  try {
    return prepareLockedRustRegistryClosureInner(input);
  } catch (error) {
    if (error instanceof Error && /^HARNESS_RUST_REGISTRY_[A-Z0-9_]+$/.test(error.message)) {
      throw error;
    }
    throw new Error('HARNESS_RUST_REGISTRY_IO_FAILED', { cause: error });
  }
}

function prepareLockedRustRegistryClosureInner(input: Readonly<{
  snapshotRegistryRoot: string;
  destinationRoot: string;
  registryKey: string;
  lockfilePath: string;
  lockfileDigest: string;
  packages: readonly FrozenRegistryPackage[];
  targetTriple: string;
  expectedContentDigest: string;
}>): LockedRustRegistryClosure {
  const snapshot = canonicalDirectory(input.snapshotRegistryRoot, 'HARNESS_RUST_REGISTRY_SNAPSHOT_INVALID');
  const destination = normalizedAbsolute(input.destinationRoot, 'HARNESS_RUST_REGISTRY_DESTINATION_INVALID');
  if (!/^[A-Za-z0-9._-]{8,128}$/.test(input.registryKey)
    || !/^[A-Za-z0-9_][A-Za-z0-9_.-]{2,127}$/.test(input.targetTriple)
    || !SHA256.test(input.lockfileDigest) || !SHA256.test(input.expectedContentDigest)) {
    throw new Error('HARNESS_RUST_REGISTRY_CONTRACT_INVALID');
  }
  const lockfile = stableFile(input.lockfilePath, MAX_LOCK_BYTES, 'HARNESS_RUST_REGISTRY_LOCK_INVALID');
  if (sha256(lockfile) !== input.lockfileDigest) throw new Error('HARNESS_RUST_REGISTRY_LOCK_MISMATCH');
  const lock = parseLock(lockfile.toString('utf8'));
  const selected = selectedPackages(lock, input.packages);
  const selectionDigest = sha256(Buffer.from(selected.map((pkg) =>
    `${pkg.name}\0${pkg.version}\0${pkg.checksum}\0`).join(''), 'utf8'));
  mkdirSync(destination, { mode: 0o700 });
  const cacheRoot = join(destination, 'cache', input.registryKey);
  const indexRoot = join(destination, 'index', input.registryKey);
  mkdirSync(cacheRoot, { recursive: true, mode: 0o700 });
  mkdirSync(join(indexRoot, '.cache'), { recursive: true, mode: 0o700 });

  const snapshotCache = canonicalDirectory(
    join(snapshot, 'cache', input.registryKey), 'HARNESS_RUST_REGISTRY_CACHE_INVALID',
  );
  const snapshotIndex = canonicalDirectory(
    join(snapshot, 'index', input.registryKey), 'HARNESS_RUST_REGISTRY_INDEX_INVALID',
  );
  copyExpectedFile(
    join(snapshotIndex, 'config.json'), join(indexRoot, 'config.json'), INDEX_CONFIG_DIGEST,
  );
  for (const pkg of selected) {
    const name = `${pkg.name}-${pkg.version}`;
    copyExpectedFile(
      join(snapshotCache, `${name}.crate`), join(cacheRoot, `${name}.crate`), pkg.checksum,
    );
  }
  const versions = groupVersions([...lock.values()].sort(comparePackage));
  for (const [name, entries] of versions) {
    const relativePath = sparseIndexPath(name);
    const raw = stableFile(
      join(snapshotIndex, '.cache', relativePath), MAX_INDEX_BYTES,
      'HARNESS_RUST_REGISTRY_INDEX_RECORD_INVALID',
    );
    writePrivateFile(
      join(indexRoot, '.cache', relativePath), selectSparseRecords(raw, name, entries),
    );
  }
  hardenTree(destination);
  const content = contentTreeDigest(destination);
  if (content.digest !== input.expectedContentDigest) {
    throw new Error('HARNESS_RUST_REGISTRY_CONTENT_MISMATCH');
  }
  const metadataDigest = metadataTreeDigest(destination);
  const assertStable = () => {
    if (metadataTreeDigest(destination) !== metadataDigest) {
      throw new Error('HARNESS_RUST_REGISTRY_CHANGED');
    }
  };
  assertStable();
  return Object.freeze({
    registryRoot: destination,
    evidence: Object.freeze({
      rustRegistryClosure: `${content.digest}:${String(content.entries)}:${String(content.bytes)}`,
      rustRegistryLock: `${input.lockfileDigest}:${String(lock.size)}:${String(selected.length)}`,
      rustRegistrySelection: `${input.targetTriple}:${selectionDigest}`,
      rustRegistryMetadata: metadataDigest,
    }),
    assertStable,
  });
}

function parseLock(raw: string): ReadonlyMap<string, LockedPackage> {
  if (!raw.startsWith('# This file is automatically @generated by Cargo.\n')
    || !/^version = 4$/m.test(raw)) throw new Error('HARNESS_RUST_REGISTRY_LOCK_INVALID');
  const packages = new Map<string, LockedPackage>();
  for (const block of raw.split('[[package]]').slice(1)) {
    const source = field(block, 'source', false);
    if (source !== CRATES_IO_SOURCE) continue;
    const name = field(block, 'name', true);
    const version = field(block, 'version', true);
    const checksum = field(block, 'checksum', true);
    if (!CRATE_NAME.test(name) || !CRATE_VERSION.test(version) || !SHA256.test(checksum)) {
      throw new Error('HARNESS_RUST_REGISTRY_LOCK_INVALID');
    }
    const key = packageKey(name, version);
    if (packages.has(key)) throw new Error('HARNESS_RUST_REGISTRY_LOCK_DUPLICATE');
    packages.set(key, Object.freeze({ name, version, checksum }));
  }
  if (packages.size === 0) throw new Error('HARNESS_RUST_REGISTRY_LOCK_EMPTY');
  return packages;
}

function field(block: string, name: string, required: boolean): string {
  const matches = [...block.matchAll(new RegExp(`^${name} = "([^"]+)"$`, 'gm'))];
  if (matches.length === 0 && !required) return '';
  if (matches.length !== 1 || matches[0]?.[1] === undefined) {
    throw new Error('HARNESS_RUST_REGISTRY_LOCK_INVALID');
  }
  return matches[0][1];
}

function selectedPackages(
  lock: ReadonlyMap<string, LockedPackage>,
  requested: readonly FrozenRegistryPackage[],
): readonly LockedPackage[] {
  if (!Array.isArray(requested) || requested.length === 0) {
    throw new Error('HARNESS_RUST_REGISTRY_SELECTION_INVALID');
  }
  const selected = new Map<string, LockedPackage>();
  for (const value of requested) {
    if (value === null || typeof value !== 'object' || !CRATE_NAME.test(value.name)
      || !CRATE_VERSION.test(value.version)) {
      throw new Error('HARNESS_RUST_REGISTRY_SELECTION_INVALID');
    }
    const key = packageKey(value.name, value.version);
    const pkg = lock.get(key);
    if (pkg === undefined || selected.has(key)) {
      throw new Error('HARNESS_RUST_REGISTRY_SELECTION_MISMATCH');
    }
    selected.set(key, pkg);
  }
  return Object.freeze([...selected.values()].sort(comparePackage));
}

function groupVersions(packages: readonly LockedPackage[]): ReadonlyMap<string, Map<string, string>> {
  const grouped = new Map<string, Map<string, string>>();
  for (const pkg of packages) {
    const versions = grouped.get(pkg.name) ?? new Map<string, string>();
    versions.set(pkg.version, pkg.checksum);
    grouped.set(pkg.name, versions);
  }
  return grouped;
}

function selectSparseRecords(
  raw: Buffer,
  name: string,
  versions: ReadonlyMap<string, string>,
): Buffer {
  if (raw.length <= SPARSE_HEADER.length
    || !raw.subarray(0, SPARSE_HEADER.length).equals(SPARSE_HEADER)) {
    throw new Error('HARNESS_RUST_REGISTRY_INDEX_RECORD_INVALID');
  }
  let decoded: string;
  try { decoded = new TextDecoder('utf-8', { fatal: true }).decode(raw.subarray(5)); } catch {
    throw new Error('HARNESS_RUST_REGISTRY_INDEX_RECORD_INVALID');
  }
  const chunks = decoded.split('\0');
  if (!/^etag: "[^"\0]{1,256}"$/.test(chunks[0] ?? '') || chunks.at(-1) !== ''
    || (chunks.length - 2) % 2 !== 0) {
    throw new Error('HARNESS_RUST_REGISTRY_INDEX_RECORD_INVALID');
  }
  const selected = new Map<string, string>();
  for (let index = 1; index + 1 < chunks.length; index += 2) {
    const version = chunks[index] ?? '';
    const json = chunks[index + 1] ?? '';
    if (!versions.has(version)) continue;
    let value: unknown;
    try { value = JSON.parse(json); } catch {
      throw new Error('HARNESS_RUST_REGISTRY_INDEX_RECORD_INVALID');
    }
    if (value === null || typeof value !== 'object' || Array.isArray(value)) {
      throw new Error('HARNESS_RUST_REGISTRY_INDEX_RECORD_INVALID');
    }
    const record = value as Record<string, unknown>;
    if (record.name !== name || record.vers !== version || record.cksum !== versions.get(version)
      || selected.has(version)) {
      throw new Error('HARNESS_RUST_REGISTRY_INDEX_RECORD_MISMATCH');
    }
    selected.set(version, json);
  }
  if (selected.size !== versions.size) throw new Error('HARNESS_RUST_REGISTRY_INDEX_RECORD_MISSING');
  const body = [SEALED_ETAG, ...[...selected].flatMap(([version, json]) => [version, json]), '']
    .join('\0');
  return Buffer.concat([SPARSE_HEADER, Buffer.from(body, 'utf8')]);
}

function sparseIndexPath(raw: string): string {
  const name = raw.toLowerCase();
  if (name.length === 1) return `1/${name}`;
  if (name.length === 2) return `2/${name}`;
  if (name.length === 3) return `3/${name[0]}/${name}`;
  return `${name.slice(0, 2)}/${name.slice(2, 4)}/${name}`;
}

function copyExpectedFile(source: string, target: string, expected: string): void {
  const copied = copyFile(source, target, false);
  if (copied.digest !== expected) throw new Error('HARNESS_RUST_REGISTRY_FILE_MISMATCH');
}

function copyFile(source: string, target: string, executable: boolean): { digest: string; bytes: number } {
  mkdirSync(dirname(target), { recursive: true, mode: 0o700 });
  const input = openSync(source, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  const output = openSync(target, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL, 0o600);
  try {
    const before = fstatSync(input, { bigint: true });
    if (!before.isFile() || before.isSymbolicLink() || before.nlink !== 1n
      || before.size < 1n || before.size > BigInt(MAX_BYTES)) {
      throw new Error('HARNESS_RUST_REGISTRY_FILE_INVALID');
    }
    const hash = createHash('sha256');
    const buffer = Buffer.allocUnsafe(1024 * 1024);
    let offset = 0n;
    while (offset < before.size) {
      const count = readSync(input, buffer, 0, Math.min(buffer.length, Number(before.size - offset)), Number(offset));
      if (count === 0) break;
      writeAll(output, buffer, count, Number(offset));
      hash.update(buffer.subarray(0, count));
      offset += BigInt(count);
    }
    const after = fstatSync(input, { bigint: true });
    if (offset !== before.size || !sameIdentity(before, after)) {
      throw new Error('HARNESS_RUST_REGISTRY_FILE_CHANGED');
    }
    chmodSync(target, executable ? 0o500 : 0o400);
    return { digest: hash.digest('hex'), bytes: Number(before.size) };
  } finally {
    closeSync(input);
    closeSync(output);
  }
}

function writePrivateFile(path: string, value: Buffer): void {
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const descriptor = openSync(path, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL, 0o400);
  try { writeAll(descriptor, value, value.length, 0); } finally { closeSync(descriptor); }
}

function stableFile(pathValue: string, maxBytes: number, error: string): Buffer {
  if (!isAbsolute(pathValue) || resolve(pathValue) !== pathValue || pathValue.includes('\0')) {
    throw new Error(error);
  }
  const before = lstatSync(pathValue, { bigint: true });
  if (!before.isFile() || before.isSymbolicLink() || before.nlink !== 1n
    || realpathSync(pathValue) !== pathValue || before.size < 1n || before.size > BigInt(maxBytes)) {
    throw new Error(error);
  }
  const value = readFileSync(pathValue);
  const after = lstatSync(pathValue, { bigint: true });
  if (BigInt(value.length) !== before.size || !sameIdentity(before, after)) throw new Error(error);
  return value;
}

function contentTreeDigest(root: string): { digest: string; entries: number; bytes: number } {
  const hash = createHash('sha256');
  let entries = 0;
  let bytes = 0;
  const visit = (directory: string) => {
    for (const name of readdirSync(directory).sort()) {
      const path = join(directory, name);
      const relativePath = relative(root, path).split(sep).join('/');
      const stat = lstatSync(path, { bigint: true });
      if (stat.isDirectory() && !stat.isSymbolicLink() && realpathSync(path) === path) {
        hash.update(`d\0${relativePath}\0\0`, 'utf8');
        entries += 1;
        visit(path);
      } else if (stat.isFile() && !stat.isSymbolicLink() && stat.nlink === 1n
        && realpathSync(path) === path) {
        const content = stableFile(path, MAX_BYTES, 'HARNESS_RUST_REGISTRY_COPY_INVALID');
        hash.update(`f\0${relativePath}\0${(stat.mode & 0o111n) !== 0n ? 'x' : 'r'}\0${String(content.length)}\0${sha256(content)}\0`, 'utf8');
        entries += 1;
        bytes += content.length;
      } else throw new Error('HARNESS_RUST_REGISTRY_COPY_INVALID');
      if (entries > MAX_ENTRIES || bytes > MAX_BYTES) throw new Error('HARNESS_RUST_REGISTRY_LIMIT');
    }
  };
  visit(root);
  return { digest: hash.digest('hex'), entries, bytes };
}

function hardenTree(root: string): void {
  const visit = (directory: string) => {
    for (const name of readdirSync(directory).sort()) {
      const path = join(directory, name);
      const stat = lstatSync(path);
      if (stat.isDirectory() && !stat.isSymbolicLink()) visit(path);
      else if (!stat.isFile() || stat.isSymbolicLink()) throw new Error('HARNESS_RUST_REGISTRY_COPY_INVALID');
    }
    chmodSync(directory, 0o700);
  };
  visit(root);
}

function metadataTreeDigest(root: string): string {
  const hash = createHash('sha256');
  const visit = (directory: string) => {
    for (const name of readdirSync(directory).sort()) {
      const path = join(directory, name);
      const stat = lstatSync(path, { bigint: true });
      const relativePath = relative(root, path).split(sep).join('/');
      if (stat.isDirectory() && !stat.isSymbolicLink() && realpathSync(path) === path) {
        metadataEntry(hash, 'd', relativePath, stat);
        visit(path);
      } else if (stat.isFile() && !stat.isSymbolicLink() && realpathSync(path) === path) {
        metadataEntry(hash, 'f', relativePath, stat);
      } else throw new Error('HARNESS_RUST_REGISTRY_COPY_INVALID');
    }
  };
  visit(root);
  return hash.digest('hex');
}

function metadataEntry(hash: ReturnType<typeof createHash>, kind: 'd' | 'f', path: string, stat: BigIntStats): void {
  hash.update([kind, path, String(stat.dev), String(stat.ino), String(stat.mode), String(stat.size),
    String(stat.mtimeNs), String(stat.ctimeNs), ''].join('\0'), 'utf8');
}

function hardPath(value: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) throw new Error();
  return value;
}

function canonicalDirectory(value: string, error: string): string {
  let path: string;
  try { path = hardPath(value); } catch { throw new Error(error); }
  const stat = lstatSync(path);
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(path) !== path) throw new Error(error);
  return path;
}

function normalizedAbsolute(value: string, error: string): string {
  try { return hardPath(value); } catch { throw new Error(error); }
}

function comparePackage(left: LockedPackage, right: LockedPackage): number {
  if (left.name !== right.name) return left.name < right.name ? -1 : 1;
  if (left.version === right.version) return 0;
  return left.version < right.version ? -1 : 1;
}

function packageKey(name: string, version: string): string { return `${name}\0${version}`; }
function sha256(value: Buffer): string { return createHash('sha256').update(value).digest('hex'); }
function sameIdentity(left: BigIntStats, right: BigIntStats): boolean {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.size === right.size && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}
function writeAll(descriptor: number, buffer: Buffer, length: number, position: number): void {
  let written = 0;
  while (written < length) written += writeSync(descriptor, buffer, written, length - written, position + written);
}
