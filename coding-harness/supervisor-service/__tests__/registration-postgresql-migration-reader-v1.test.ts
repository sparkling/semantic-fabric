// SPDX-License-Identifier: MIT

import { execFileSync } from 'node:child_process';
import {
  chmodSync, copyFileSync, linkSync, mkdirSync, mkdtempSync, renameSync,
  rmSync, symlinkSync, writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { rawSha256HexV1 }
  from '../src/registration-postgresql-canonical-v1.js';
import { readPostgresMigrationBundleV1 }
  from '../src/registration-postgresql-migration-reader-v1.js';

const SERVICE_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const SOURCE_MIGRATIONS = resolve(SERVICE_ROOT, 'migrations');
const FILES = Object.freeze([
  'manifest-v1.json',
  'catalog-contract-v1.json',
  'provisioning-contract-v1.json',
  '0001-registration-state-v1.sql',
  '0002-registration-rls-v1.sql',
]);
const EXPECTED = Object.freeze({
  manifest: Object.freeze({
    bytes: 1_299,
    sha256: '72782ecae7d33a0149fb2ceb0a3219254fcd68137e499b811260f77b5cd70478',
  }),
  catalogueContract: Object.freeze({
    bytes: 232_822,
    sha256: 'e7ce3572463587f4beed55c35c5a6b93810a270136cb963cf312b580fd1ace69',
  }),
  provisioningContract: Object.freeze({
    bytes: 8_657,
    sha256: '71e4bafda6f97f44b54f28903363fe4ff88f3199a2d08b4dc4bc9060c33e55a9',
  }),
  migration0001: Object.freeze({
    bytes: 26_438,
    sha256: 'c923f0f725c009a65ef85bc1881b7ae5717a1eca148bbf5316aeee60bb4a31c1',
  }),
  migration0002: Object.freeze({
    bytes: 21_661,
    sha256: '1d620d95f630997785d0d3adf724e5befe458c0c41e0746f45713cb584b58765',
  }),
});
const TEMP_ROOTS: string[] = [];

afterEach(() => {
  for (const path of TEMP_ROOTS.splice(0)) rmSync(path, { recursive: true, force: true });
});

describe('PostgreSQL fixed migration reader V1', () => {
  it('reads only the five pinned regular files and returns fresh private bytes', () => {
    const root = fixture();
    writeFileSync(resolve(root, 'migrations/unrelated.txt'), 'ignored');

    const first = readPostgresMigrationBundleV1(root);
    const second = readPostgresMigrationBundleV1(root);
    expect(Object.keys(first)).toEqual(Object.keys(EXPECTED));
    expect(Object.isFrozen(first)).toBe(true);
    for (const key of Object.keys(EXPECTED) as Array<keyof typeof EXPECTED>) {
      expect(first[key]).not.toBe(second[key]);
      expect(first[key]).toHaveLength(EXPECTED[key].bytes);
      expect(rawSha256HexV1(first[key])).toBe(EXPECTED[key].sha256);
    }
    first.manifest.fill(0);
    expect(rawSha256HexV1(second.manifest)).toBe(EXPECTED.manifest.sha256);
  });

  it('rejects lexical, ancestor, root, migrations, and file symlinks', () => {
    const root = fixture();
    expectInvalid('relative/root');

    const rootAlias = `${root}-alias`;
    symlinkSync(root, rootAlias, 'dir');
    expectInvalid(rootAlias);
    rmSync(rootAlias);

    const anchor = temporary('sf-pg-reader-ancestor-');
    const actualParent = resolve(anchor, 'actual');
    const service = resolve(actualParent, 'service');
    mkdirSync(actualParent);
    populate(service);
    const parentAlias = resolve(anchor, 'alias');
    symlinkSync(actualParent, parentAlias, 'dir');
    expectInvalid(resolve(parentAlias, 'service'));

    const migrations = resolve(root, 'migrations');
    renameSync(migrations, `${migrations}-actual`);
    symlinkSync(`${migrations}-actual`, migrations, 'dir');
    expectInvalid(root);

    rmSync(migrations);
    renameSync(`${migrations}-actual`, migrations);
    const manifest = resolve(migrations, 'manifest-v1.json');
    renameSync(manifest, `${manifest}-actual`);
    symlinkSync(`${manifest}-actual`, manifest, 'file');
    expectInvalid(root);
  });

  it('rejects hard links, FIFOs, writable directories, and writable files', () => {
    const hardLinkRoot = fixture();
    linkSync(
      resolve(hardLinkRoot, 'migrations/manifest-v1.json'),
      resolve(hardLinkRoot, 'migrations/manifest-hard-link.json'),
    );
    expectInvalid(hardLinkRoot);

    const fifoRoot = fixture();
    const fifo = resolve(fifoRoot, 'migrations/manifest-v1.json');
    rmSync(fifo);
    execFileSync('mkfifo', [fifo]);
    chmodSync(fifo, 0o644);
    expectInvalid(fifoRoot);

    const rootWritable = fixture();
    chmodSync(rootWritable, 0o775);
    expectInvalid(rootWritable);

    const migrationsWritable = fixture();
    chmodSync(resolve(migrationsWritable, 'migrations'), 0o775);
    expectInvalid(migrationsWritable);

    const fileWritable = fixture();
    chmodSync(resolve(fileWritable, 'migrations/manifest-v1.json'), 0o664);
    expectInvalid(fileWritable);
  });

  it('rejects missing, short, long, and same-length digest mutations', () => {
    const missing = fixture();
    rmSync(resolve(missing, 'migrations/manifest-v1.json'));
    expectInvalid(missing);

    for (const value of ['', 'x'.repeat(EXPECTED.manifest.bytes + 1)]) {
      const root = fixture();
      writeFileSync(resolve(root, 'migrations/manifest-v1.json'), value);
      chmodSync(resolve(root, 'migrations/manifest-v1.json'), 0o644);
      expectInvalid(root);
    }

    const mutated = fixture();
    const path = resolve(mutated, 'migrations/manifest-v1.json');
    const bytes = readPostgresMigrationBundleV1(mutated).manifest;
    bytes[0] = bytes[0] === 0x7b ? 0x5b : 0x7b;
    writeFileSync(path, bytes);
    chmodSync(path, 0o644);
    expectInvalid(mutated);
  });

  it('uses one sanitized failure and does not disclose caller paths', () => {
    const secretPath = resolve(temporary('sf-pg-reader-secret-'), 'not-present');
    let failure: unknown;
    try { readPostgresMigrationBundleV1(secretPath); } catch (error) { failure = error; }
    expect(failure).toBeInstanceOf(TypeError);
    expect((failure as Error).message).toBe('PostgreSQL migration bundle is invalid');
    expect((failure as Error).message).not.toContain(secretPath);
  });

  it('fails closed without O_NOFOLLOW and when any descriptor close fails', async () => {
    const root = fixture();
    vi.resetModules();
    vi.doMock('node:fs', async () => {
      const actual = await vi.importActual<typeof import('node:fs')>('node:fs');
      return {
        ...actual,
        constants: { ...actual.constants, O_NOFOLLOW: 0 },
      };
    });
    try {
      const isolated = await import('../src/registration-postgresql-migration-reader-v1.js');
      expect(() => isolated.readPostgresMigrationBundleV1(root))
        .toThrow('PostgreSQL migration bundle is invalid');
    } finally {
      vi.doUnmock('node:fs');
      vi.resetModules();
    }

    let rejectedClose = false;
    vi.doMock('node:fs', async () => {
      const actual = await vi.importActual<typeof import('node:fs')>('node:fs');
      return {
        ...actual,
        closeSync: (fd: number) => {
          actual.closeSync(fd);
          if (!rejectedClose) {
            rejectedClose = true;
            throw new Error('synthetic close failure');
          }
        },
      };
    });
    try {
      const isolated = await import('../src/registration-postgresql-migration-reader-v1.js');
      expect(() => isolated.readPostgresMigrationBundleV1(root))
        .toThrow('PostgreSQL migration bundle is invalid');
      expect(rejectedClose).toBe(true);
    } finally {
      vi.doUnmock('node:fs');
      vi.resetModules();
    }
  });
});

function temporary(prefix: string): string {
  const path = mkdtempSync(resolve(tmpdir(), prefix));
  TEMP_ROOTS.push(path);
  return path;
}

function fixture(): string {
  const root = temporary('sf-pg-reader-');
  populate(root);
  return root;
}

function populate(root: string): void {
  mkdirSync(resolve(root, 'migrations'), { recursive: true, mode: 0o755 });
  chmodSync(root, 0o755);
  chmodSync(resolve(root, 'migrations'), 0o755);
  for (const name of FILES) {
    const target = resolve(root, 'migrations', name);
    copyFileSync(resolve(SOURCE_MIGRATIONS, name), target);
    chmodSync(target, 0o644);
  }
}

function expectInvalid(root: string): void {
  expect(() => readPostgresMigrationBundleV1(root))
    .toThrow('PostgreSQL migration bundle is invalid');
}
