// SPDX-License-Identifier: MIT

import { isProxy } from 'node:util/types';
import {
  deepFreezeV1,
  snapshotBytesV1,
} from './registration-postgresql-canonical-v1.js';
import {
  parsePostgresMigrationJsonBytesV1,
} from './registration-postgresql-migration-json-v1.js';

export const POSTGRES_MIGRATION_MANIFEST_BYTES_V1 = 1_299;
export const POSTGRES_MIGRATION_MANIFEST_SHA256_V1 =
  '72782ecae7d33a0149fb2ceb0a3219254fcd68137e499b811260f77b5cd70478';

const INVALID = 'PostgreSQL migration manifest is invalid';
const EXPECTED_TEXT = `{
  "domain": "semantic-fabric/programme-capture/supervisor-postgresql-migration-manifest-v1",
  "schemaVersion": 1,
  "authority": "none",
  "readinessAuthorized": false,
  "postgresqlServerVersion": "16.15",
  "postgresqlServerVersionNumber": 160015,
  "advisoryLockKey": "800874507948546278",
  "catalogContract": {
    "path": "migrations/catalog-contract-v1.json",
    "bytes": 232822,
    "sha256": "e7ce3572463587f4beed55c35c5a6b93810a270136cb963cf312b580fd1ace69"
  },
  "provisioningContract": {
    "path": "migrations/provisioning-contract-v1.json",
    "bytes": 8657,
    "sha256": "71e4bafda6f97f44b54f28903363fe4ff88f3199a2d08b4dc4bc9060c33e55a9"
  },
  "authoritySeed": {
    "domain": "semantic-fabric/programme-capture/supervisor-postgresql-authority-seed-v1",
    "bytes": 11303,
    "sha256": "b6d3b0f77a2b71cb10782152cb74a3f37320e220131a2d3ef29cc455c9a26e4c"
  },
  "migrations": [
    {
      "version": 1,
      "path": "migrations/0001-registration-state-v1.sql",
      "bytes": 26438,
      "sha256": "c923f0f725c009a65ef85bc1881b7ae5717a1eca148bbf5316aeee60bb4a31c1"
    },
    {
      "version": 2,
      "path": "migrations/0002-registration-rls-v1.sql",
      "bytes": 21661,
      "sha256": "1d620d95f630997785d0d3adf724e5befe458c0c41e0746f45713cb584b58765"
    }
  ]
}
`;
const EXPECTED_RECORD = deepFreezeV1(
  JSON.parse(EXPECTED_TEXT) as PostgresMigrationManifestRecordV1,
);
const HANDLES = new WeakMap<object, Readonly<{
  bytes: Uint8Array;
  record: PostgresMigrationManifestRecordV1;
}>>();

export interface PostgresMigrationManifestFileV1 {
  readonly path: string;
  readonly bytes: number;
  readonly sha256: string;
}
export interface PostgresMigrationManifestStepV1 extends PostgresMigrationManifestFileV1 {
  readonly version: 1 | 2;
}
export interface PostgresMigrationManifestRecordV1 {
  readonly domain: string;
  readonly schemaVersion: 1;
  readonly authority: 'none';
  readonly readinessAuthorized: false;
  readonly postgresqlServerVersion: '16.15';
  readonly postgresqlServerVersionNumber: 160015;
  readonly advisoryLockKey: '800874507948546278';
  readonly catalogContract: PostgresMigrationManifestFileV1;
  readonly provisioningContract: PostgresMigrationManifestFileV1;
  readonly authoritySeed: Readonly<{ domain: string; bytes: number; sha256: string }>;
  readonly migrations: readonly [
    PostgresMigrationManifestStepV1,
    PostgresMigrationManifestStepV1,
  ];
}
export interface ParsedPostgresMigrationManifestV1 {
  readonly manifestKind: 'postgresql-migration-manifest-v1';
  readonly rawByteLength: typeof POSTGRES_MIGRATION_MANIFEST_BYTES_V1;
  readonly rawSha256: typeof POSTGRES_MIGRATION_MANIFEST_SHA256_V1;
  readonly authority: 'none';
  readonly readinessAuthorized: false;
}

export function parsePostgresMigrationManifestV1(
  value: unknown,
): ParsedPostgresMigrationManifestV1 {
  try {
    const bytes = snapshotBytesV1(
      value, 'PostgreSQL migration manifest',
      POSTGRES_MIGRATION_MANIFEST_BYTES_V1,
      POSTGRES_MIGRATION_MANIFEST_BYTES_V1,
    );
    const parsed = parsePostgresMigrationJsonBytesV1(bytes, 'manifest');
    if (parsed.sha256 !== POSTGRES_MIGRATION_MANIFEST_SHA256_V1
      || JSON.stringify(parsed.record) !== JSON.stringify(EXPECTED_RECORD)) throw new TypeError();
    const handle = Object.freeze({
      manifestKind: 'postgresql-migration-manifest-v1' as const,
      rawByteLength: POSTGRES_MIGRATION_MANIFEST_BYTES_V1,
      rawSha256: POSTGRES_MIGRATION_MANIFEST_SHA256_V1,
      authority: 'none' as const,
      readinessAuthorized: false as const,
    });
    HANDLES.set(handle, Object.freeze({ bytes, record: EXPECTED_RECORD }));
    return handle;
  } catch {
    throw new TypeError(INVALID);
  }
}

export function assertPostgresMigrationManifestHandleV1(
  value: unknown,
): asserts value is ParsedPostgresMigrationManifestV1 {
  try {
    if (isProxy(value) || value === null || typeof value !== 'object'
      || !HANDLES.has(value)) throw new TypeError();
  } catch {
    throw new TypeError(INVALID);
  }
}

export function postgresMigrationManifestRecordV1(
  value: unknown,
): PostgresMigrationManifestRecordV1 {
  assertPostgresMigrationManifestHandleV1(value);
  return HANDLES.get(value)!.record;
}

export function copyPostgresMigrationManifestBytesV1(value: unknown): Uint8Array {
  assertPostgresMigrationManifestHandleV1(value);
  return Uint8Array.from(HANDLES.get(value)!.bytes);
}
