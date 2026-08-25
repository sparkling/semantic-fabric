// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  chmodSync,
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { prepareLockedRustRegistryClosure } from '../src/rust-registry-closure.js';

const REGISTRY_KEY = 'index.crates.io-1949cf8c6b5b557f';
const CRATES_IO_SOURCE = 'registry+https://github.com/rust-lang/crates.io-index';
const PACKAGE = Object.freeze({ name: 'alpha', version: '1.2.3' });
const ARCHIVE = Buffer.from('alpha crate archive\n', 'utf8');
const ARCHIVE_DIGEST = sha256(ARCHIVE);
const OTHER_DIGEST = 'f'.repeat(64);
const SECOND_PACKAGE = Object.freeze({ name: 'beta', version: '4.5.6' });
const SECOND_DIGEST = 'e'.repeat(64);
const EXPECTED_CONTENT_DIGEST = 'a063431174e652ad0b985a767085f74b0f3dd8f14ac9dfc231f517c49cbe649c';
const EXPECTED_ASYMMETRIC_CONTENT_DIGEST =
  '474417144387985628d861eaac701457150a6a3b96dc6af3b5ea4515b512317a';
const EXPECTED_ENTRIES = 10;
const EXPECTED_BYTES = 284;
const SPARSE_HEADER = Buffer.from([3, 2, 0, 0, 0]);
const INDEX_CONFIG = Buffer.from([
  '{',
  '  "dl": "https://static.crates.io/crates",',
  '  "api": "https://crates.io"',
  '}',
  '',
].join('\n'), 'utf8');
const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) {
    makeTreeWritable(root);
    rmSync(root, { recursive: true, force: true });
  }
});

describe('dependency-scoped locked Rust registry closure', () => {
  it('copies and seals exactly the selected package', () => {
    const fixture = registryFixture();
    const closure = prepare(fixture);

    expect(closure.evidence.rustRegistryClosure).toBe(
      `${EXPECTED_CONTENT_DIGEST}:${EXPECTED_ENTRIES}:${EXPECTED_BYTES}`,
    );
    expect(closure.evidence.rustRegistryLock).toBe(`${fixture.lockfileDigest}:1:1`);
    expect(closure.evidence.rustRegistrySelection).toBe(
      `x86_64-unknown-linux-gnu:${sha256(Buffer.from(
        `${PACKAGE.name}\0${PACKAGE.version}\0${ARCHIVE_DIGEST}\0`, 'utf8',
      ))}`,
    );
    expect(readFileSync(join(
      closure.registryRoot,
      'cache',
      REGISTRY_KEY,
      `${PACKAGE.name}-${PACKAGE.version}.crate`,
    ))).toEqual(ARCHIVE);
    const selectedIndex = readFileSync(join(
      closure.registryRoot,
      'index',
      REGISTRY_KEY,
      '.cache',
      'al',
      'ph',
      PACKAGE.name,
    ));
    expect(selectedIndex.subarray(0, SPARSE_HEADER.length)).toEqual(SPARSE_HEADER);
    expect(selectedIndex.toString('utf8')).toContain('etag: "semantic-fabric-locked"');
    expect(selectedIndex.toString('utf8')).toContain(PACKAGE.version);
    closure.assertStable();
  });

  it('ignores an unrelated sparse version without changing the content digest', () => {
    const fixture = registryFixture({
      sparseRecords: [
        packageRecord(PACKAGE.version, ARCHIVE_DIGEST),
        packageRecord('9.9.9', OTHER_DIGEST),
      ],
    });
    const closure = prepare(fixture);
    const selectedIndex = readFileSync(join(
      closure.registryRoot,
      'index',
      REGISTRY_KEY,
      '.cache',
      'al',
      'ph',
      PACKAGE.name,
    )).toString('utf8');

    expect(closure.evidence.rustRegistryClosure).toBe(
      `${EXPECTED_CONTENT_DIGEST}:${EXPECTED_ENTRIES}:${EXPECTED_BYTES}`,
    );
    expect(selectedIndex).not.toContain('9.9.9');
    expect(selectedIndex).not.toContain(OTHER_DIGEST);
  });

  it('retains every locked index record but only selected target archives', () => {
    const fixture = registryFixture({ additionalLockedPackage: true });
    const closure = prepare(fixture, {
      expectedContentDigest: EXPECTED_ASYMMETRIC_CONTENT_DIGEST,
    });

    expect(existsSync(join(
      closure.registryRoot, 'cache', REGISTRY_KEY,
      `${SECOND_PACKAGE.name}-${SECOND_PACKAGE.version}.crate`,
    ))).toBe(false);
    expect(readFileSync(join(
      closure.registryRoot, 'index', REGISTRY_KEY, '.cache', 'be', 'ta', SECOND_PACKAGE.name,
    ), 'utf8')).toContain(SECOND_PACKAGE.version);
    expect(closure.evidence.rustRegistryLock).toBe(`${fixture.lockfileDigest}:2:1`);
  });

  it('rejects an archive whose checksum does not match the frozen lock', () => {
    const fixture = registryFixture();
    writeFileSync(fixture.archivePath, 'tampered archive\n', 'utf8');

    expect(() => prepare(fixture)).toThrow('HARNESS_RUST_REGISTRY_FILE_MISMATCH');
  });

  it('converts a missing selected archive into a finite harness error', () => {
    const fixture = registryFixture();
    rmSync(fixture.archivePath);

    expect(() => prepare(fixture)).toThrow('HARNESS_RUST_REGISTRY_IO_FAILED');
  });

  it('rejects a sparse index missing the exact locked version', () => {
    const fixture = registryFixture({
      sparseRecords: [packageRecord('9.9.9', OTHER_DIGEST)],
    });

    expect(() => prepare(fixture)).toThrow('HARNESS_RUST_REGISTRY_INDEX_RECORD_MISSING');
  });

  it('rejects duplicate sparse records for the exact locked version', () => {
    const selected = packageRecord(PACKAGE.version, ARCHIVE_DIGEST);
    const fixture = registryFixture({ sparseRecords: [selected, selected] });

    expect(() => prepare(fixture)).toThrow('HARNESS_RUST_REGISTRY_INDEX_RECORD_MISMATCH');
  });

  it('rejects lock digest and package-selection mismatches', () => {
    const fixture = registryFixture();
    expect(() => prepare(fixture, {
      destinationRoot: join(fixture.root, 'wrong-lock'),
      lockfileDigest: '0'.repeat(64),
    })).toThrow('HARNESS_RUST_REGISTRY_LOCK_MISMATCH');

    expect(() => prepare(fixture, {
      destinationRoot: join(fixture.root, 'wrong-selection'),
      packages: [{ name: PACKAGE.name, version: '1.2.4' }],
    })).toThrow('HARNESS_RUST_REGISTRY_SELECTION_MISMATCH');
  });
});

interface RegistryFixture {
  readonly root: string;
  readonly snapshotRegistryRoot: string;
  readonly destinationRoot: string;
  readonly archivePath: string;
  readonly lockfilePath: string;
  readonly lockfileDigest: string;
}

function registryFixture(options: Readonly<{
  sparseRecords?: readonly SparseRecord[];
  additionalLockedPackage?: boolean;
}> = {}): RegistryFixture {
  const root = mkdtempSync(join(tmpdir(), 'coding-harness-rust-registry-'));
  roots.push(root);
  const snapshotRegistryRoot = join(root, 'snapshot');
  const cacheRoot = join(snapshotRegistryRoot, 'cache', REGISTRY_KEY);
  const indexRoot = join(snapshotRegistryRoot, 'index', REGISTRY_KEY);
  const sparsePath = join(indexRoot, '.cache', 'al', 'ph', PACKAGE.name);
  mkdirSync(cacheRoot, { recursive: true, mode: 0o700 });
  mkdirSync(join(indexRoot, '.cache', 'al', 'ph'), { recursive: true, mode: 0o700 });
  const archivePath = join(cacheRoot, `${PACKAGE.name}-${PACKAGE.version}.crate`);
  writeFileSync(archivePath, ARCHIVE, { mode: 0o600 });
  writeFileSync(join(indexRoot, 'config.json'), INDEX_CONFIG, { mode: 0o600 });
  writeFileSync(sparsePath, sparseCache(
    options.sparseRecords ?? [packageRecord(PACKAGE.version, ARCHIVE_DIGEST)],
  ), { mode: 0o600 });
  if (options.additionalLockedPackage === true) {
    const secondPath = join(indexRoot, '.cache', 'be', 'ta', SECOND_PACKAGE.name);
    mkdirSync(join(indexRoot, '.cache', 'be', 'ta'), { recursive: true, mode: 0o700 });
    writeFileSync(secondPath, sparseCache([
      packageRecord(SECOND_PACKAGE.version, SECOND_DIGEST, SECOND_PACKAGE.name),
    ]), { mode: 0o600 });
  }
  const lockfilePath = join(root, 'Cargo.lock');
  const lockfile = cargoLock(options.additionalLockedPackage === true);
  writeFileSync(lockfilePath, lockfile, { mode: 0o600 });
  return Object.freeze({
    root,
    snapshotRegistryRoot,
    destinationRoot: join(root, 'locked'),
    archivePath,
    lockfilePath,
    lockfileDigest: sha256(lockfile),
  });
}

function prepare(
  fixture: RegistryFixture,
  overrides: Partial<Parameters<typeof prepareLockedRustRegistryClosure>[0]> = {},
) {
  return prepareLockedRustRegistryClosure({
    snapshotRegistryRoot: fixture.snapshotRegistryRoot,
    destinationRoot: fixture.destinationRoot,
    registryKey: REGISTRY_KEY,
    lockfilePath: fixture.lockfilePath,
    lockfileDigest: fixture.lockfileDigest,
    packages: [PACKAGE],
    targetTriple: 'x86_64-unknown-linux-gnu',
    expectedContentDigest: EXPECTED_CONTENT_DIGEST,
    ...overrides,
  });
}

interface SparseRecord {
  readonly version: string;
  readonly json: string;
}

function packageRecord(version: string, checksum: string, name = PACKAGE.name): SparseRecord {
  return Object.freeze({
    version,
    json: JSON.stringify({
      name,
      vers: version,
      deps: [],
      cksum: checksum,
      features: {},
      yanked: false,
    }),
  });
}

function sparseCache(records: readonly SparseRecord[]): Buffer {
  const body = [
    'etag: "fixture"',
    ...records.flatMap(({ version, json }) => [version, json]),
    '',
  ].join('\0');
  return Buffer.concat([SPARSE_HEADER, Buffer.from(body, 'utf8')]);
}

function cargoLock(additionalLockedPackage = false): Buffer {
  const lines = [
    '# This file is automatically @generated by Cargo.',
    '# It is not intended for manual editing.',
    'version = 4',
    '',
    ...packageBlock(PACKAGE.name, PACKAGE.version, ARCHIVE_DIGEST),
  ];
  if (additionalLockedPackage) {
    lines.push(...packageBlock(SECOND_PACKAGE.name, SECOND_PACKAGE.version, SECOND_DIGEST));
  }
  return Buffer.from(lines.join('\n'), 'utf8');
}

function packageBlock(name: string, version: string, checksum: string): string[] {
  return [
    '[[package]]', `name = "${name}"`, `version = "${version}"`,
    `source = "${CRATES_IO_SOURCE}"`, `checksum = "${checksum}"`, '',
  ];
}

function sha256(value: Buffer): string {
  return createHash('sha256').update(value).digest('hex');
}

function makeTreeWritable(path: string): void {
  const stat = lstatSync(path);
  if (!stat.isDirectory()) {
    chmodSync(path, 0o600);
    return;
  }
  chmodSync(path, 0o700);
  for (const name of readdirSync(path)) makeTreeWritable(join(path, name));
}
