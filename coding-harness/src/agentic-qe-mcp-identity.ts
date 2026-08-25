// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  closeSync,
  constants,
  fstatSync,
  lstatSync,
  openSync,
  readFileSync,
  readdirSync,
  readlinkSync,
  realpathSync,
} from 'node:fs';
import type { BigIntStats } from 'node:fs';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';

export interface StableExecutableIdentity {
  readonly path: string;
  readonly sha256: string;
  readonly dev: bigint;
  readonly ino: bigint;
  readonly mode: bigint;
  readonly size: bigint;
  readonly mtimeNs: bigint;
  readonly ctimeNs: bigint;
}

export interface AgenticQePackageIdentity {
  readonly root: string;
  readonly entryPath: string;
  readonly manifestPath: string;
  readonly name: 'agentic-qe';
  readonly version: string;
  readonly entrySha256: string;
  readonly treeSha256: string;
  readonly fileCount: number;
  readonly totalBytes: number;
}

export interface PackageIdentityLimits {
  readonly maxFiles: number;
  readonly maxBytes: number;
}

interface FileCapture {
  readonly bytes: Buffer;
  readonly sha256: string;
  readonly stat: BigIntStats;
}

interface TreeRecord {
  readonly path: string;
  readonly kind: 'directory' | 'file' | 'symlink';
  readonly mode: string;
  readonly uid: string;
  readonly gid: string;
  readonly size: string;
  readonly mtimeNs: string;
  readonly ctimeNs: string;
  readonly nlink: string;
  readonly content?: string;
  readonly target?: string;
}

const PACKAGE_NAME = 'agentic-qe';
const PACKAGE_BIN = './dist/mcp/bundle.js';
const VERSION = /^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$/;

export function captureStableExecutable(
  value: string,
  label: 'NODE' | 'BWRAP',
): StableExecutableIdentity {
  const path = canonicalAbsolute(value, `HARNESS_AGENTIC_QE_${label}_INVALID`);
  const capture = captureFile(path, 256_000_000, true);
  const stat = capture.stat;
  const uid = process.getuid?.() ?? Number(stat.uid);
  if (stat.nlink !== 1n
    || (stat.mode & 0o111n) === 0n
    || (stat.mode & 0o022n) !== 0n
    || (Number(stat.uid) !== 0 && Number(stat.uid) !== uid)) {
    throw new Error(`HARNESS_AGENTIC_QE_${label}_UNTRUSTED`);
  }
  return Object.freeze({
    path,
    sha256: capture.sha256,
    dev: stat.dev,
    ino: stat.ino,
    mode: stat.mode,
    size: stat.size,
    mtimeNs: stat.mtimeNs,
    ctimeNs: stat.ctimeNs,
  });
}

export function assertStableExecutable(
  expected: StableExecutableIdentity,
  label: 'NODE' | 'BWRAP',
): void {
  const current = captureStableExecutable(expected.path, label);
  if (current.sha256 !== expected.sha256
    || current.dev !== expected.dev
    || current.ino !== expected.ino
    || current.mode !== expected.mode
    || current.size !== expected.size
    || current.mtimeNs !== expected.mtimeNs
    || current.ctimeNs !== expected.ctimeNs) {
    throw new Error(`HARNESS_AGENTIC_QE_${label}_CHANGED`);
  }
}

export function captureAgenticQePackage(
  rootValue: string,
  entryValue: string,
  limits: PackageIdentityLimits,
): AgenticQePackageIdentity {
  const root = canonicalAbsolute(rootValue, 'HARNESS_AGENTIC_QE_PACKAGE_ROOT_INVALID');
  const rootStat = lstatSync(root, { bigint: true });
  if (!rootStat.isDirectory()) throw new Error('HARNESS_AGENTIC_QE_PACKAGE_ROOT_INVALID');
  assertPackageOwnerMode(rootStat, 'HARNESS_AGENTIC_QE_PACKAGE_ROOT_UNTRUSTED');
  const entryPath = canonicalAbsolute(entryValue, 'HARNESS_AGENTIC_QE_ENTRY_INVALID');
  if (!contains(root, entryPath)) throw new Error('HARNESS_AGENTIC_QE_ENTRY_OUTSIDE_PACKAGE');
  const manifestPath = join(root, 'package.json');
  if (realpathSync(manifestPath) !== manifestPath) {
    throw new Error('HARNESS_AGENTIC_QE_PACKAGE_MANIFEST_INVALID');
  }
  const manifestCapture = captureFile(manifestPath, 1_000_000, false);
  const manifest = parsePackageManifest(manifestCapture.bytes);
  const binEntry = manifest.bin['aqe-mcp'];
  if (typeof binEntry !== 'string') {
    throw new Error('HARNESS_AGENTIC_QE_PACKAGE_BINDING_INVALID');
  }
  const declaredEntry = resolve(root, binEntry);
  if (manifest.name !== PACKAGE_NAME
    || binEntry !== PACKAGE_BIN
    || declaredEntry !== entryPath
    || realpathSync(declaredEntry) !== entryPath) {
    throw new Error('HARNESS_AGENTIC_QE_PACKAGE_BINDING_INVALID');
  }
  const tree = captureTree(root, limits);
  const entrySha256 = captureFile(entryPath, limits.maxBytes, false).sha256;
  return Object.freeze({
    root,
    entryPath,
    manifestPath,
    name: PACKAGE_NAME,
    version: manifest.version,
    entrySha256,
    treeSha256: tree.sha256,
    fileCount: tree.fileCount,
    totalBytes: tree.totalBytes,
  });
}

export function assertAgenticQePackageStable(
  expected: AgenticQePackageIdentity,
  limits: PackageIdentityLimits,
): void {
  const current = captureAgenticQePackage(expected.root, expected.entryPath, limits);
  if (current.version !== expected.version
    || current.entrySha256 !== expected.entrySha256
    || current.treeSha256 !== expected.treeSha256
    || current.fileCount !== expected.fileCount
    || current.totalBytes !== expected.totalBytes) {
    throw new Error('HARNESS_AGENTIC_QE_PACKAGE_CHANGED');
  }
}

function captureTree(
  root: string,
  limits: PackageIdentityLimits,
): Readonly<{ sha256: string; fileCount: number; totalBytes: number }> {
  if (!Number.isSafeInteger(limits.maxFiles) || limits.maxFiles < 2
    || !Number.isSafeInteger(limits.maxBytes) || limits.maxBytes < 1) {
    throw new TypeError('HARNESS_AGENTIC_QE_PACKAGE_LIMIT_INVALID');
  }
  const records: TreeRecord[] = [];
  const hardlinks = new Map<string, { expected: bigint; observed: number }>();
  let fileCount = 0;
  let totalBytes = 0;

  const visit = (directory: string): void => {
    const before = lstatSync(directory, { bigint: true });
    assertPackageOwnerMode(before, 'HARNESS_AGENTIC_QE_PACKAGE_ENTRY_UNTRUSTED');
    const entries = readdirSync(directory).sort();
    for (const name of entries) {
      const path = join(directory, name);
      const relativePath = relative(root, path).split(sep).join('/');
      const stat = lstatSync(path, { bigint: true });
      assertPackageOwnerMode(
        stat,
        'HARNESS_AGENTIC_QE_PACKAGE_ENTRY_UNTRUSTED',
        stat.isSymbolicLink(),
      );
      if (stat.isDirectory()) {
        records.push(treeRecord(relativePath, 'directory', stat));
        visit(path);
      } else if (stat.isFile()) {
        fileCount += 1;
        totalBytes += Number(stat.size);
        if (fileCount > limits.maxFiles || totalBytes > limits.maxBytes) {
          throw new Error('HARNESS_AGENTIC_QE_PACKAGE_LIMIT_EXCEEDED');
        }
        const capture = captureFile(path, limits.maxBytes, false);
        records.push({
          ...treeRecord(relativePath, 'file', capture.stat),
          content: capture.sha256,
        });
        const key = `${stat.dev}:${stat.ino}`;
        const hardlink = hardlinks.get(key) ?? { expected: stat.nlink, observed: 0 };
        hardlink.observed += 1;
        hardlinks.set(key, hardlink);
      } else if (stat.isSymbolicLink()) {
        const target = readlinkSync(path);
        const resolvedTarget = realpathSync(path);
        if (!contains(root, resolvedTarget)) {
          throw new Error('HARNESS_AGENTIC_QE_PACKAGE_SYMLINK_ESCAPE');
        }
        records.push({ ...treeRecord(relativePath, 'symlink', stat), target });
      } else {
        throw new Error('HARNESS_AGENTIC_QE_PACKAGE_ENTRY_UNTRUSTED');
      }
    }
    const after = lstatSync(directory, { bigint: true });
    if (!sameDirectory(before, after)) throw new Error('HARNESS_AGENTIC_QE_PACKAGE_CHANGED');
  };
  visit(root);
  if ([...hardlinks.values()].some(({ expected, observed }) => expected !== BigInt(observed))) {
    throw new Error('HARNESS_AGENTIC_QE_PACKAGE_HARDLINK_ESCAPE');
  }
  const sha256 = createHash('sha256')
    .update(JSON.stringify({ root, records }))
    .digest('hex');
  return Object.freeze({ sha256, fileCount, totalBytes });
}

function captureFile(path: string, maxBytes: number, executable: boolean): FileCapture {
  const pathStat = lstatSync(path, { bigint: true });
  if (!pathStat.isFile() || pathStat.isSymbolicLink()
    || realpathSync(path) !== path || pathStat.nlink < 1n
    || (executable && pathStat.size < 1n) || pathStat.size > BigInt(maxBytes)) {
    throw new Error(executable
      ? 'HARNESS_AGENTIC_QE_EXECUTABLE_UNTRUSTED'
      : 'HARNESS_AGENTIC_QE_PACKAGE_ENTRY_UNTRUSTED');
  }
  const descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const before = fstatSync(descriptor, { bigint: true });
    const bytes = readFileSync(descriptor);
    const after = fstatSync(descriptor, { bigint: true });
    if (!sameFile(before, after) || before.size !== BigInt(bytes.length)) {
      throw new Error('HARNESS_AGENTIC_QE_IDENTITY_CHANGED_DURING_READ');
    }
    return Object.freeze({
      bytes,
      sha256: createHash('sha256').update(bytes).digest('hex'),
      stat: before,
    });
  } finally {
    closeSync(descriptor);
  }
}

function parsePackageManifest(bytes: Buffer): {
  name: unknown;
  version: string;
  bin: Record<string, unknown>;
} {
  let parsed: unknown;
  try {
    parsed = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
  } catch {
    throw new Error('HARNESS_AGENTIC_QE_PACKAGE_MANIFEST_INVALID');
  }
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
    throw new Error('HARNESS_AGENTIC_QE_PACKAGE_MANIFEST_INVALID');
  }
  const record = parsed as Record<string, unknown>;
  if (typeof record.version !== 'string' || !VERSION.test(record.version)
    || record.bin === null || typeof record.bin !== 'object' || Array.isArray(record.bin)) {
    throw new Error('HARNESS_AGENTIC_QE_PACKAGE_MANIFEST_INVALID');
  }
  return {
    name: record.name,
    version: record.version,
    bin: record.bin as Record<string, unknown>,
  };
}

function treeRecord(
  path: string,
  kind: TreeRecord['kind'],
  stat: BigIntStats,
): TreeRecord {
  return {
    path,
    kind,
    mode: stat.mode.toString(),
    uid: stat.uid.toString(),
    gid: stat.gid.toString(),
    size: stat.size.toString(),
    mtimeNs: stat.mtimeNs.toString(),
    ctimeNs: stat.ctimeNs.toString(),
    nlink: stat.nlink.toString(),
  };
}

function assertPackageOwnerMode(
  stat: BigIntStats,
  error: string,
  ignoreMode = false,
): void {
  const uid = BigInt(process.getuid?.() ?? Number(stat.uid));
  if ((!ignoreMode && (stat.mode & 0o002n) !== 0n)
    || (stat.uid !== 0n && stat.uid !== uid)) {
    throw new Error(error);
  }
}

function canonicalAbsolute(value: string, error: string): string {
  if (typeof value !== 'string' || !isAbsolute(value) || resolve(value) !== value
    || value.includes('\0') || realpathSync(value) !== value) {
    throw new Error(error);
  }
  return value;
}

function contains(root: string, child: string): boolean {
  const delta = relative(root, child);
  return delta === '' || (delta !== '..' && !delta.startsWith(`..${sep}`) && !isAbsolute(delta));
}

function sameFile(
  left: BigIntStats,
  right: BigIntStats,
): boolean {
  return left.isFile() && right.isFile()
    && left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.uid === right.uid && left.gid === right.gid && left.nlink === right.nlink
    && left.size === right.size && left.mtimeNs === right.mtimeNs
    && left.ctimeNs === right.ctimeNs;
}

function sameDirectory(
  left: BigIntStats,
  right: BigIntStats,
): boolean {
  return left.isDirectory() && right.isDirectory()
    && left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.uid === right.uid && left.gid === right.gid
    && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}
