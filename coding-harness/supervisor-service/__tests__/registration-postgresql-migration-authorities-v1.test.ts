// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import {
  POSTGRES_AUTHORITY_SEED_BYTES_V1,
  POSTGRES_AUTHORITY_SEED_SHA256_V1,
  assertPostgresAuthoritySeedHandleV1,
  copyPostgresAuthoritySeedBytesV1,
  parsePostgresAuthoritySeedCandidateV1,
  parseSealedPostgresAuthoritySeedV1,
} from '../src/registration-postgresql-authority-seed-v1.js';
import {
  POSTGRES_MIGRATION_MANIFEST_BYTES_V1,
  POSTGRES_MIGRATION_MANIFEST_SHA256_V1,
  assertPostgresMigrationManifestHandleV1,
  copyPostgresMigrationManifestBytesV1,
  parsePostgresMigrationManifestV1,
  postgresMigrationManifestRecordV1,
} from '../src/registration-postgresql-migration-manifest-v1.js';
import {
  POSTGRES_PROVISIONING_CONTRACT_BYTES_V1,
  POSTGRES_PROVISIONING_CONTRACT_SHA256_V1,
  assertPostgresProvisioningContractHandleV1,
  copyPostgresProvisioningContractBytesV1,
  parsePostgresProvisioningContractV1,
} from '../src/registration-postgresql-provisioning-contract-v1.js';
import { rawSha256HexV1 }
  from '../src/registration-postgresql-canonical-v1.js';

const PROVISIONING_PATH = fileURLToPath(
  new URL('../migrations/provisioning-contract-v1.json', import.meta.url),
);
const MANIFEST_PATH = fileURLToPath(
  new URL('../migrations/manifest-v1.json', import.meta.url),
);

describe('PostgreSQL migration semantic authorities V1', () => {
  it('pins and privately brands the exact provisioning contract', () => {
    const source = new Uint8Array(readFileSync(PROVISIONING_PATH));
    expect(source).toHaveLength(POSTGRES_PROVISIONING_CONTRACT_BYTES_V1);
    expect(rawSha256HexV1(source)).toBe(POSTGRES_PROVISIONING_CONTRACT_SHA256_V1);

    const handle = parsePostgresProvisioningContractV1(source);
    source.fill(0);
    expect(handle).toEqual({
      contractKind: 'postgresql-provisioning-contract-v1',
      rawByteLength: POSTGRES_PROVISIONING_CONTRACT_BYTES_V1,
      rawSha256: POSTGRES_PROVISIONING_CONTRACT_SHA256_V1,
      authority: 'none',
      readinessAuthorized: false,
    });
    expect(Object.isFrozen(handle)).toBe(true);
    expect(() => assertPostgresProvisioningContractHandleV1(handle)).not.toThrow();
    expect(copyPostgresProvisioningContractBytesV1(handle))
      .toEqual(new Uint8Array(readFileSync(PROVISIONING_PATH)));
  });

  it('pins and privately brands the exact manifest and its fixed references', () => {
    const source = new Uint8Array(readFileSync(MANIFEST_PATH));
    expect(source).toHaveLength(POSTGRES_MIGRATION_MANIFEST_BYTES_V1);
    expect(rawSha256HexV1(source)).toBe(POSTGRES_MIGRATION_MANIFEST_SHA256_V1);

    const handle = parsePostgresMigrationManifestV1(source);
    source.fill(0);
    const record = postgresMigrationManifestRecordV1(handle);
    expect(record).toMatchObject({
      authority: 'none',
      readinessAuthorized: false,
      postgresqlServerVersion: '16.15',
      postgresqlServerVersionNumber: 160_015,
      advisoryLockKey: '800874507948546278',
      catalogContract: {
        path: 'migrations/catalog-contract-v1.json',
        bytes: 232_822,
        sha256: 'e7ce3572463587f4beed55c35c5a6b93810a270136cb963cf312b580fd1ace69',
      },
      provisioningContract: {
        path: 'migrations/provisioning-contract-v1.json',
        bytes: 8_657,
        sha256: POSTGRES_PROVISIONING_CONTRACT_SHA256_V1,
      },
    });
    expect(Object.isFrozen(record)).toBe(true);
    expect(Object.isFrozen(record.migrations)).toBe(true);
    expect(() => assertPostgresMigrationManifestHandleV1(handle)).not.toThrow();
  });

  it('replays and independently validates the sealed in-memory seed', () => {
    const first = parseSealedPostgresAuthoritySeedV1();
    const bytes = copyPostgresAuthoritySeedBytesV1(first);
    expect(bytes).toHaveLength(POSTGRES_AUTHORITY_SEED_BYTES_V1);
    expect(rawSha256HexV1(bytes)).toBe(POSTGRES_AUTHORITY_SEED_SHA256_V1);

    const second = parsePostgresAuthoritySeedCandidateV1(bytes);
    bytes.fill(0);
    expect(copyPostgresAuthoritySeedBytesV1(first))
      .toEqual(copyPostgresAuthoritySeedBytesV1(second));
    expect(() => assertPostgresAuthoritySeedHandleV1(first)).not.toThrow();
    expect(Object.isFrozen(first)).toBe(true);
  });

  it('returns fresh byte copies without mutable aliases', () => {
    const provisioning = parsePostgresProvisioningContractV1(readFileSync(PROVISIONING_PATH));
    const manifest = parsePostgresMigrationManifestV1(readFileSync(MANIFEST_PATH));
    const seed = parseSealedPostgresAuthoritySeedV1();
    const projections = [
      () => copyPostgresProvisioningContractBytesV1(provisioning),
      () => copyPostgresMigrationManifestBytesV1(manifest),
      () => copyPostgresAuthoritySeedBytesV1(seed),
    ];

    for (const project of projections) {
      const first = project();
      const originalFirstByte = first[0];
      first.fill(0);
      const second = project();
      expect(second).not.toBe(first);
      expect(second[0]).toBe(originalFirstByte);
    }
  });

  it('rejects same-length mutations before a semantic handle can escape', () => {
    const provisioning = replaceAscii(
      new Uint8Array(readFileSync(PROVISIONING_PATH)), '"authority": "none"', '"authority": "fake"',
    );
    const manifest = replaceAscii(
      new Uint8Array(readFileSync(MANIFEST_PATH)), '"16.15"', '"16.14"',
    );
    const seed = copyPostgresAuthoritySeedBytesV1(parseSealedPostgresAuthoritySeedV1());
    seed[seed.length - 2] = 0x5b;

    expect(() => parsePostgresProvisioningContractV1(provisioning))
      .toThrow('PostgreSQL provisioning contract is invalid');
    expect(() => parsePostgresMigrationManifestV1(manifest))
      .toThrow('PostgreSQL migration manifest is invalid');
    expect(() => parsePostgresAuthoritySeedCandidateV1(seed))
      .toThrow('PostgreSQL authority seed is invalid');
  });

  it('keeps the three brands disjoint and rejects clones and proxies', () => {
    const provisioning = parsePostgresProvisioningContractV1(readFileSync(PROVISIONING_PATH));
    const manifest = parsePostgresMigrationManifestV1(readFileSync(MANIFEST_PATH));
    const seed = parseSealedPostgresAuthoritySeedV1();
    const proxy = new Proxy(manifest, { get: () => { throw new Error('trap'); } });

    expect(() => assertPostgresProvisioningContractHandleV1(manifest)).toThrow();
    expect(() => assertPostgresMigrationManifestHandleV1(seed)).toThrow();
    expect(() => assertPostgresAuthoritySeedHandleV1(provisioning)).toThrow();
    expect(() => assertPostgresMigrationManifestHandleV1({ ...manifest })).toThrow();
    expect(() => assertPostgresMigrationManifestHandleV1(proxy))
      .toThrow('PostgreSQL migration manifest is invalid');
  });
});

function replaceAscii(bytes: Uint8Array, from: string, to: string): Uint8Array {
  expect(from).toHaveLength(to.length);
  const text = new TextDecoder().decode(bytes);
  expect(text.split(from)).toHaveLength(2);
  return new TextEncoder().encode(text.replace(from, to));
}
