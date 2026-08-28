// SPDX-License-Identifier: MIT

import {
  closeSync, constants, fstatSync, fsyncSync, lstatSync, mkdirSync, openSync,
  readSync, realpathSync, writeSync, type BigIntStats,
} from 'node:fs';
import { basename, dirname, isAbsolute, join, resolve } from 'node:path';

const MAX_ARTIFACT_BYTES = 100_000_000;
export const PROGRAMME_V6_CLAIM_AUTHORITY_ROOT =
  '/home/claude/.semantic-fabric-harness';

export type ProgrammeV6ArtifactSuffix =
  'policy-review' | 'execution-claim' | 'execution' | 'replay';

export function programmeV6ArtifactPath(
  repositoryRoot: string,
  runId: string,
  suffix: ProgrammeV6ArtifactSuffix,
): string {
  if (!/^[A-Za-z0-9_-]{8,160}$/.test(runId)) {
    throw new Error('HARNESS_PROGRAMME_V6_ARTIFACT_RUN_ID_INVALID');
  }
  const name = suffix === 'execution' ? `${runId}.json` : `${runId}.${suffix}.json`;
  return join(exactRepository(repositoryRoot), 'coding-harness', '.metaharness', 'runs', name);
}

export function requireProgrammeV6ArtifactPath(
  repositoryRoot: string,
  runId: string,
  suffix: ProgrammeV6ArtifactSuffix,
  value: string,
): string {
  const expected = programmeV6ArtifactPath(repositoryRoot, runId, suffix);
  if (!isAbsolute(value) || resolve(value) !== value || value !== expected || value.includes('\0')) {
    throw new Error('HARNESS_PROGRAMME_V6_ARTIFACT_PATH_INVALID');
  }
  return value;
}

export function readProgrammeV6PrivateArtifact(
  repositoryRoot: string,
  path: string,
  maximumBytes = MAX_ARTIFACT_BYTES,
): string {
  return readPrivateArtifact(prepareRunsDirectory(repositoryRoot, false), path, maximumBytes);
}

export function writeProgrammeV6PrivateArtifact(
  repositoryRoot: string,
  path: string,
  contents: string,
  maximumBytes = MAX_ARTIFACT_BYTES,
): void {
  writePrivateArtifact(prepareRunsDirectory(repositoryRoot, true), path, contents, maximumBytes);
}

export function programmeV6AuthorityClaimPath(
  claimKeyDigest: string,
  authorityRoot = PROGRAMME_V6_CLAIM_AUTHORITY_ROOT,
): string {
  if (!/^[a-f0-9]{64}$/.test(claimKeyDigest) || claimKeyDigest === '0'.repeat(64)) {
    throw new Error('HARNESS_PROGRAMME_V6_CLAIM_KEY_INVALID');
  }
  return join(authorityRootPath(authorityRoot), 'programme-v6-claims', `${claimKeyDigest}.json`);
}

export function readProgrammeV6AuthorityClaim(
  path: string,
  authorityRoot = PROGRAMME_V6_CLAIM_AUTHORITY_ROOT,
  maximumBytes = 100_000,
): string {
  return readPrivateArtifact(prepareClaimDirectory(authorityRoot, false), path, maximumBytes);
}

export function writeProgrammeV6AuthorityClaim(
  path: string,
  contents: string,
  authorityRoot = PROGRAMME_V6_CLAIM_AUTHORITY_ROOT,
  maximumBytes = 100_000,
): void {
  writePrivateArtifact(prepareClaimDirectory(authorityRoot, true), path, contents, maximumBytes);
}

export function assertProgrammeV6ArtifactAbsent(path: string, error: string): void {
  try { lstatSync(path); }
  catch (caught) {
    if ((caught as NodeJS.ErrnoException).code === 'ENOENT') return;
    throw caught;
  }
  throw new Error(error);
}

function readPrivateArtifact(directoryPath: string, path: string, maximumBytes: number): string {
  if (dirname(path) !== directoryPath) {
    throw new Error('HARNESS_PROGRAMME_V6_ARTIFACT_PATH_INVALID');
  }
  const directory = openDirectory(directoryPath);
  try {
    const directoryIdentity = fstatSync(directory, { bigint: true });
    assertDirectoryIdentity(directoryPath, directoryIdentity);
    const anchoredPath = `/proc/self/fd/${directory}/${basename(path)}`;
    const pathStat = trustedFileStat(anchoredPath, maximumBytes);
    const descriptor = openSync(anchoredPath, constants.O_RDONLY | noFollow());
    try {
      const before = fstatSync(descriptor, { bigint: true });
      if (!sameFile(pathStat, before)) throw new Error('HARNESS_PROGRAMME_V6_ARTIFACT_CHANGED');
      const bytes = Buffer.allocUnsafe(Number(before.size));
      let offset = 0;
      while (offset < bytes.length) {
        const count = readSync(descriptor, bytes, offset, bytes.length - offset, offset);
        if (count === 0) throw new Error('HARNESS_PROGRAMME_V6_ARTIFACT_CHANGED');
        offset += count;
      }
      const after = fstatSync(descriptor, { bigint: true });
      if (!sameFile(before, after)
        || !sameFile(after, lstatSync(anchoredPath, { bigint: true }))) {
        throw new Error('HARNESS_PROGRAMME_V6_ARTIFACT_CHANGED');
      }
      assertDirectoryIdentity(directoryPath, directoryIdentity, directory);
      try { return new TextDecoder('utf-8', { fatal: true }).decode(bytes); }
      catch { throw new Error('HARNESS_PROGRAMME_V6_ARTIFACT_UTF8_INVALID'); }
    } finally { closeSync(descriptor); }
  } finally { closeSync(directory); }
}

function writePrivateArtifact(
  directoryPath: string,
  path: string,
  contents: string,
  maximumBytes: number,
): void {
  if (dirname(path) !== directoryPath) {
    throw new Error('HARNESS_PROGRAMME_V6_ARTIFACT_PATH_INVALID');
  }
  const bytes = Buffer.from(contents, 'utf8');
  if (bytes.length < 1 || bytes.length > maximumBytes) {
    throw new Error('HARNESS_PROGRAMME_V6_ARTIFACT_SIZE_INVALID');
  }
  const directory = openDirectory(directoryPath);
  try {
    const directoryIdentity = fstatSync(directory, { bigint: true });
    assertDirectoryIdentity(directoryPath, directoryIdentity);
    const anchoredPath = `/proc/self/fd/${directory}/${basename(path)}`;
    assertAbsent(anchoredPath);
    const descriptor = openSync(
      anchoredPath,
      constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | noFollow(),
      0o600,
    );
    try {
      let offset = 0;
      while (offset < bytes.length) {
        offset += writeSync(descriptor, bytes, offset, bytes.length - offset, offset);
      }
      fsyncSync(descriptor);
    } finally { closeSync(descriptor); }
    fsyncSync(directory);
    assertDirectoryIdentity(directoryPath, directoryIdentity, directory);
  } finally { closeSync(directory); }
  if (readPrivateArtifact(directoryPath, path, maximumBytes) !== contents) {
    throw new Error('HARNESS_PROGRAMME_V6_ARTIFACT_CHANGED');
  }
}

function prepareRunsDirectory(repositoryRoot: string, create: boolean): string {
  const repository = exactRepository(repositoryRoot);
  const harness = exactDirectory(join(repository, 'coding-harness'), false, 'DIRECTORY');
  const root = exactDirectory(join(harness, '.metaharness'), create, 'RESULT_ROOT');
  return exactDirectory(join(root, 'runs'), create, 'RESULTS_ROOT');
}

function prepareClaimDirectory(authorityRoot: string, create: boolean): string {
  const root = exactDirectory(
    authorityRootPath(authorityRoot), create, 'CLAIM_AUTHORITY_ROOT',
  );
  return exactDirectory(join(root, 'programme-v6-claims'), create, 'CLAIM_AUTHORITY');
}

function authorityRootPath(value: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) {
    throw new Error('HARNESS_PROGRAMME_V6_CLAIM_AUTHORITY_ROOT_INVALID');
  }
  return value;
}

function exactRepository(value: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) {
    throw new Error('HARNESS_PROGRAMME_V6_DIRECTORY_INVALID');
  }
  return exactDirectory(value, false, 'DIRECTORY');
}

function exactDirectory(path: string, create: boolean, label: string): string {
  try { lstatSync(path); }
  catch (caught) {
    if ((caught as NodeJS.ErrnoException).code !== 'ENOENT' || !create) throw caught;
    exactDirectory(dirname(path), false, 'DIRECTORY');
    mkdirSync(path, { mode: 0o700 });
  }
  const stat = lstatSync(path);
  const uid = process.getuid?.() ?? stat.uid;
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(path) !== path
    || stat.uid !== uid || (label !== 'DIRECTORY' && (stat.mode & 0o077) !== 0)) {
    throw new Error(`HARNESS_PROGRAMME_V6_${label}_INVALID`);
  }
  if (create) {
    fsyncDirectory(dirname(path));
    fsyncDirectory(path);
  }
  return path;
}

function fsyncDirectory(path: string): void {
  const descriptor = openDirectory(path);
  try { fsyncSync(descriptor); }
  finally { closeSync(descriptor); }
}

function openDirectory(path: string): number {
  return openSync(path, constants.O_RDONLY | (constants.O_DIRECTORY ?? 0) | noFollow());
}

function trustedFileStat(path: string, maximumBytes: number) {
  const stat = lstatSync(path, { bigint: true });
  if (!stat.isFile() || stat.isSymbolicLink() || stat.uid !== BigInt(process.getuid?.() ?? 0)
    || stat.nlink !== 1n || (stat.mode & 0o777n) !== 0o600n
    || stat.size < 1n || stat.size > BigInt(maximumBytes)) {
    throw new Error('HARNESS_PROGRAMME_V6_ARTIFACT_INVALID');
  }
  return stat;
}

function assertAbsent(path: string): void {
  try { lstatSync(path); }
  catch (caught) {
    if ((caught as NodeJS.ErrnoException).code === 'ENOENT') return;
    throw caught;
  }
  throw new Error('HARNESS_PROGRAMME_V6_RECEIPT_EXISTS');
}

function assertDirectoryIdentity(path: string, expected: BigIntStats, descriptor?: number): void {
  const current = descriptor === undefined ? expected : fstatSync(descriptor, { bigint: true });
  if (!sameDirectory(expected, current)
    || !sameDirectory(expected, lstatSync(path, { bigint: true }))) {
    throw new Error('HARNESS_PROGRAMME_V6_RESULTS_ROOT_CHANGED');
  }
}

function sameDirectory(left: BigIntStats, right: BigIntStats): boolean {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.uid === right.uid && left.nlink === right.nlink;
}

function sameFile(left: BigIntStats, right: BigIntStats): boolean {
  return sameDirectory(left, right) && left.size === right.size
    && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}

function noFollow(): number {
  return constants.O_NOFOLLOW ?? 0;
}
