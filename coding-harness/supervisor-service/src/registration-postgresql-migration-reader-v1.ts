// SPDX-License-Identifier: MIT

import {
  accessSync, closeSync, constants, fstatSync, openSync, readSync,
  type BigIntStats,
} from 'node:fs';
import { isAbsolute, normalize } from 'node:path';
import { rawSha256HexV1 }
  from './registration-postgresql-canonical-v1.js';

const INVALID = 'PostgreSQL migration bundle is invalid';
const MAXIMUM_ROOT_BYTES = 4_096;
const MAXIMUM_ROOT_COMPONENTS = 64;
const FILES = Object.freeze([
  Object.freeze({
    key: 'manifest',
    name: 'manifest-v1.json',
    bytes: 1_299,
    sha256: '72782ecae7d33a0149fb2ceb0a3219254fcd68137e499b811260f77b5cd70478',
  }),
  Object.freeze({
    key: 'catalogueContract',
    name: 'catalog-contract-v1.json',
    bytes: 232_822,
    sha256: 'e7ce3572463587f4beed55c35c5a6b93810a270136cb963cf312b580fd1ace69',
  }),
  Object.freeze({
    key: 'provisioningContract',
    name: 'provisioning-contract-v1.json',
    bytes: 8_657,
    sha256: '71e4bafda6f97f44b54f28903363fe4ff88f3199a2d08b4dc4bc9060c33e55a9',
  }),
  Object.freeze({
    key: 'migration0001',
    name: '0001-registration-state-v1.sql',
    bytes: 26_438,
    sha256: 'c923f0f725c009a65ef85bc1881b7ae5717a1eca148bbf5316aeee60bb4a31c1',
  }),
  Object.freeze({
    key: 'migration0002',
    name: '0002-registration-rls-v1.sql',
    bytes: 21_661,
    sha256: '1d620d95f630997785d0d3adf724e5befe458c0c41e0746f45713cb584b58765',
  }),
] as const);

export interface PostgresMigrationBundleV1 {
  readonly manifest: Uint8Array;
  readonly catalogueContract: Uint8Array;
  readonly provisioningContract: Uint8Array;
  readonly migration0001: Uint8Array;
  readonly migration0002: Uint8Array;
}

type BundleKeyV1 = keyof PostgresMigrationBundleV1;
type MutableBundleV1 = { [K in BundleKeyV1]?: Uint8Array };
type DirectoryIdentityV1 = Readonly<{
  stat: BigIntStats;
  fd: number;
}>;

/**
 * Read the dormant migration inputs from a private service root.
 *
 * Linux openat-style component walks are expressed through held directory
 * descriptors and /proc/self/fd. There is deliberately no realpath, readdir,
 * caller-selected filename, or unsafe fallback.
 */
export function readPostgresMigrationBundleV1(
  serviceRoot: string,
): PostgresMigrationBundleV1 {
  const held: number[] = [];
  let bundle: PostgresMigrationBundleV1 | undefined;
  let failed = false;
  try {
    const runtime = secureRuntime();
    const components = rootComponents(serviceRoot);
    const anchor = openSync('/', runtime.directoryFlags);
    held.push(anchor);
    validateAncestor(fstatSync(anchor, { bigint: true }));

    let parent = anchor;
    let service: DirectoryIdentityV1 | undefined;
    for (let index = 0; index < components.length; index += 1) {
      const current = openDirectory(parent, components[index]!, runtime.directoryFlags, held);
      if (index === components.length - 1) {
        validatePrivateDirectory(current.stat);
        service = current;
      } else {
        validateAncestor(current.stat);
      }
      parent = current.fd;
    }
    if (!service) throw new TypeError();

    const migrations = openDirectory(
      service.fd, 'migrations', runtime.directoryFlags, held,
    );
    validatePrivateDirectory(migrations.stat, service.stat);

    const aliases = new Set<string>();
    const output: MutableBundleV1 = {};
    for (const pin of FILES) {
      output[pin.key] = readPinnedFile(
        migrations.fd, pin, runtime.fileFlags, migrations.stat, aliases, held,
      );
    }
    if (!sameIdentity(service.stat, fstatSync(service.fd, { bigint: true }))
      || !sameIdentity(migrations.stat, fstatSync(migrations.fd, { bigint: true }))) {
      throw new TypeError();
    }
    bundle = Object.freeze({
      manifest: output.manifest!,
      catalogueContract: output.catalogueContract!,
      provisioningContract: output.provisioningContract!,
      migration0001: output.migration0001!,
      migration0002: output.migration0002!,
    });
  } catch {
    failed = true;
  } finally {
    for (let index = held.length - 1; index >= 0; index -= 1) {
      try { closeSync(held[index]!); } catch { failed = true; }
    }
  }
  if (failed || !bundle) throw new TypeError(INVALID);
  return bundle;
}

function secureRuntime(): Readonly<{ directoryFlags: number; fileFlags: number }> {
  if (process.platform !== 'linux'
    || !Number.isInteger(constants.O_NOFOLLOW) || constants.O_NOFOLLOW <= 0
    || !Number.isInteger(constants.O_NONBLOCK) || constants.O_NONBLOCK <= 0
    || !Number.isInteger(constants.O_DIRECTORY) || constants.O_DIRECTORY <= 0) {
    throw new TypeError();
  }
  accessSync('/proc/self/fd', constants.R_OK | constants.X_OK);
  return Object.freeze({
    directoryFlags: constants.O_RDONLY | constants.O_DIRECTORY
      | constants.O_NOFOLLOW | constants.O_NONBLOCK,
    fileFlags: constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK,
  });
}

function rootComponents(value: unknown): string[] {
  if (typeof value !== 'string' || value.length === 0
    || Buffer.byteLength(value, 'utf8') > MAXIMUM_ROOT_BYTES
    || value.includes('\0') || !isAbsolute(value) || normalize(value) !== value
    || value === '/' || value.endsWith('/')) throw new TypeError();
  const components = value.slice(1).split('/');
  if (components.length === 0 || components.length > MAXIMUM_ROOT_COMPONENTS
    || components.some((part) => part.length === 0 || part === '.' || part === '..'
      || Buffer.byteLength(part, 'utf8') > 255)) throw new TypeError();
  return components;
}

function openDirectory(
  parentFd: number,
  component: string,
  flags: number,
  held: number[],
): DirectoryIdentityV1 {
  const fd = openSync(`/proc/self/fd/${parentFd}/${component}`, flags);
  held.push(fd);
  const stat = fstatSync(fd, { bigint: true });
  if (!stat.isDirectory()) throw new TypeError();
  return Object.freeze({ fd, stat });
}

function validateAncestor(stat: BigIntStats): void {
  if (!stat.isDirectory() || (stat.uid !== 0n && stat.uid !== currentUid())) {
    throw new TypeError();
  }
  const mode = Number(stat.mode & 0o7777n);
  const stickyRootDirectory = stat.uid === 0n
    && (mode & 0o002) !== 0 && (mode & 0o1000) !== 0;
  if ((mode & 0o002) !== 0 && !stickyRootDirectory) throw new TypeError();
  if ((mode & 0o020) !== 0 && !stickyRootDirectory
    && stat.uid !== currentUid()) throw new TypeError();
}

function validatePrivateDirectory(stat: BigIntStats, parent?: BigIntStats): void {
  if (!stat.isDirectory() || (stat.mode & 0o022n) !== 0n
    || (stat.uid !== 0n && stat.uid !== currentUid())
    || (parent && (stat.uid !== parent.uid || stat.gid !== parent.gid))) {
    throw new TypeError();
  }
}

function readPinnedFile(
  directoryFd: number,
  pin: typeof FILES[number],
  flags: number,
  directory: BigIntStats,
  aliases: Set<string>,
  held: number[],
): Uint8Array {
  const fd = openSync(`/proc/self/fd/${directoryFd}/${pin.name}`, flags);
  held.push(fd);
  const before = fstatSync(fd, { bigint: true });
  if (!before.isFile() || before.nlink !== 1n || (before.mode & 0o022n) !== 0n
    || before.uid !== directory.uid || before.gid !== directory.gid
    || before.size !== BigInt(pin.bytes)) throw new TypeError();
  const identity = `${before.dev}:${before.ino}`;
  if (aliases.has(identity)) throw new TypeError();
  aliases.add(identity);

  const bytes = new Uint8Array(pin.bytes);
  let offset = 0;
  while (offset < bytes.byteLength) {
    const count = readSync(fd, bytes, offset, bytes.byteLength - offset, null);
    if (!Number.isSafeInteger(count) || count <= 0) throw new TypeError();
    offset += count;
  }
  const after = fstatSync(fd, { bigint: true });
  if (!sameIdentity(before, after) || rawSha256HexV1(bytes) !== pin.sha256) {
    throw new TypeError();
  }
  return bytes;
}

function sameIdentity(before: BigIntStats, after: BigIntStats): boolean {
  return before.dev === after.dev && before.ino === after.ino
    && before.mode === after.mode && before.uid === after.uid && before.gid === after.gid
    && before.nlink === after.nlink && before.size === after.size
    && before.mtimeNs === after.mtimeNs && before.ctimeNs === after.ctimeNs;
}

function currentUid(): bigint {
  const getuid = process.getuid;
  if (typeof getuid !== 'function') throw new TypeError();
  const uid = getuid();
  if (!Number.isSafeInteger(uid) || uid < 0) throw new TypeError();
  return BigInt(uid);
}
