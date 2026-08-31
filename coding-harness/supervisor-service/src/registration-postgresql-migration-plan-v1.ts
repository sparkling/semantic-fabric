// SPDX-License-Identifier: MIT

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { isProxy } from 'node:util/types';
import {
  canonicalDigestHexV1,
  canonicalJsonV1,
  deepFreezeV1,
  snapshotClosedGraphV1,
} from './registration-postgresql-canonical-v1.js';
import {
  assertPostgresCatalogueDigestV1,
  parsePostgresCatalogueContractV1,
  type ParsedPostgresCatalogueContractV1,
} from './registration-postgresql-catalogue-contract-v1.js';
import {
  copyPostgresAuthoritySeedBytesV1,
  POSTGRES_AUTHORITY_SEED_DOMAIN_V1,
  parseSealedPostgresAuthoritySeedV1,
  type ParsedPostgresAuthoritySeedV1,
} from './registration-postgresql-authority-seed-v1.js';
import {
  parsePostgresMigrationManifestV1,
  postgresMigrationManifestRecordV1,
  type ParsedPostgresMigrationManifestV1,
} from './registration-postgresql-migration-manifest-v1.js';
import { readPostgresMigrationBundleV1 }
  from './registration-postgresql-migration-reader-v1.js';
import {
  copyPostgresMigrationSqlBytesV1,
  parsePostgresMigrationSqlV1,
  type ParsedPostgresMigrationSqlV1,
} from './registration-postgresql-migration-sql-policy-v1.js';
import {
  parsePostgresProvisioningContractV1,
  type ParsedPostgresProvisioningContractV1,
} from './registration-postgresql-provisioning-contract-v1.js';

const INVALID_PLAN = 'PostgreSQL migration Plan is invalid';
const INVALID_RECEIPT = 'PostgreSQL migration preflight receipt is invalid';
const SERVICE_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
declare const PLAN_IDENTITY: unique symbol;

export interface SealedPostgresMigrationPlanV1 {
  readonly [PLAN_IDENTITY]: true;
  readonly planKind: 'sealed-postgresql-migration-plan-v1';
  readonly authority: 'none';
  readonly readinessAuthorized: false;
  readonly databaseAccessAuthorized: false;
  readonly migrationApplyAuthorized: false;
  readonly preflightReceiptSha256: string;
}

export interface PostgresMigrationPlanAuthoritiesV1 {
  readonly manifest: ParsedPostgresMigrationManifestV1;
  readonly catalogueContract: ParsedPostgresCatalogueContractV1;
  readonly provisioningContract: ParsedPostgresProvisioningContractV1;
  readonly authoritySeed: ParsedPostgresAuthoritySeedV1;
  readonly migration0001: ParsedPostgresMigrationSqlV1;
  readonly migration0002: ParsedPostgresMigrationSqlV1;
}

export type PostgresMigrationPlanStepV1 = Readonly<
  | {
    stepKind: 'migration-sql';
    migrationVersion: 1 | 2;
    bytes: Uint8Array;
  }
  | {
    stepKind: 'authority-seed';
    bytes: Uint8Array;
  }
>;

export interface PostgresMigrationPlanPreflightReceiptV1 {
  readonly schemaVersion: 1;
  readonly receiptKind: 'postgresql-migration-plan-preflight-v1';
  readonly planKind: 'sealed-postgresql-migration-plan-v1';
  readonly authority: 'none';
  readonly readinessAuthorized: false;
  readonly databaseAccessAuthorized: false;
  readonly migrationApplyAuthorized: false;
  readonly postgresqlServerVersion: '16.15';
  readonly postgresqlServerVersionNumber: 160015;
  readonly advisoryLockKey: '800874507948546278';
  readonly artifacts: Readonly<Record<
    | 'manifest' | 'catalogueContract' | 'provisioningContract'
    | 'authoritySeed' | 'migration0001' | 'migration0002',
    Readonly<{ bytes: number; sha256: string }>
  >>;
  readonly receiptSha256: string;
}

interface PlanStateV1 {
  readonly authorities: PostgresMigrationPlanAuthoritiesV1;
  readonly receipt: PostgresMigrationPlanPreflightReceiptV1;
}

const RECEIPT_BODY = deepFreezeV1({
  schemaVersion: 1 as const,
  receiptKind: 'postgresql-migration-plan-preflight-v1' as const,
  planKind: 'sealed-postgresql-migration-plan-v1' as const,
  authority: 'none' as const,
  readinessAuthorized: false as const,
  databaseAccessAuthorized: false as const,
  migrationApplyAuthorized: false as const,
  postgresqlServerVersion: '16.15' as const,
  postgresqlServerVersionNumber: 160_015 as const,
  advisoryLockKey: '800874507948546278' as const,
  artifacts: {
    manifest: {
      bytes: 1_299,
      sha256: '72782ecae7d33a0149fb2ceb0a3219254fcd68137e499b811260f77b5cd70478',
    },
    catalogueContract: {
      bytes: 232_822,
      sha256: 'e7ce3572463587f4beed55c35c5a6b93810a270136cb963cf312b580fd1ace69',
    },
    provisioningContract: {
      bytes: 8_657,
      sha256: '71e4bafda6f97f44b54f28903363fe4ff88f3199a2d08b4dc4bc9060c33e55a9',
    },
    authoritySeed: {
      bytes: 11_303,
      sha256: 'b6d3b0f77a2b71cb10782152cb74a3f37320e220131a2d3ef29cc455c9a26e4c',
    },
    migration0001: {
      bytes: 26_438,
      sha256: 'c923f0f725c009a65ef85bc1881b7ae5717a1eca148bbf5316aeee60bb4a31c1',
    },
    migration0002: {
      bytes: 21_661,
      sha256: '1d620d95f630997785d0d3adf724e5befe458c0c41e0746f45713cb584b58765',
    },
  },
});
export const POSTGRES_MIGRATION_PLAN_PREFLIGHT_SHA256_V1 =
  '2ff788e1d5af841ea6be0f1d22635a0a584e176d2c17537b9c0533afeebd434d';
if (canonicalDigestHexV1(RECEIPT_BODY)
  !== POSTGRES_MIGRATION_PLAN_PREFLIGHT_SHA256_V1) {
  throw new TypeError('PostgreSQL migration preflight receipt pin is invalid');
}
const RECEIPT: PostgresMigrationPlanPreflightReceiptV1 = deepFreezeV1({
  ...RECEIPT_BODY,
  receiptSha256: POSTGRES_MIGRATION_PLAN_PREFLIGHT_SHA256_V1,
});
const RECEIPT_CANONICAL = canonicalJsonV1(RECEIPT);
const PLANS = new WeakMap<object, PlanStateV1>();

/** Load, bind, and brand the fixed dormant migration inputs. No database is opened. */
export function loadSealedPostgresMigrationPlanV1(): SealedPostgresMigrationPlanV1 {
  try {
    const bundle = readPostgresMigrationBundleV1(SERVICE_ROOT);
    const manifest = parsePostgresMigrationManifestV1(bundle.manifest);
    const manifestRecord = postgresMigrationManifestRecordV1(manifest);
    const catalogueContract = parsePostgresCatalogueContractV1(bundle.catalogueContract);
    const provisioningContract = parsePostgresProvisioningContractV1(
      bundle.provisioningContract,
    );
    const authoritySeed = parseSealedPostgresAuthoritySeedV1();
    const migration0001 = parsePostgresMigrationSqlV1(bundle.migration0001, 1);
    const migration0002 = parsePostgresMigrationSqlV1(bundle.migration0002, 2);

    assertPostgresCatalogueDigestV1(
      catalogueContract, manifestRecord.catalogContract.sha256,
    );
    assertAtomicManifestBinding(
      manifestRecord, catalogueContract, provisioningContract,
      authoritySeed, migration0001, migration0002,
    );
    const authorities: PostgresMigrationPlanAuthoritiesV1 = Object.freeze({
      manifest,
      catalogueContract,
      provisioningContract,
      authoritySeed,
      migration0001,
      migration0002,
    });
    assertReceiptBinding(authorities);
    const plan = Object.freeze({
      planKind: 'sealed-postgresql-migration-plan-v1' as const,
      authority: 'none' as const,
      readinessAuthorized: false as const,
      databaseAccessAuthorized: false as const,
      migrationApplyAuthorized: false as const,
      preflightReceiptSha256: POSTGRES_MIGRATION_PLAN_PREFLIGHT_SHA256_V1,
    }) as unknown as SealedPostgresMigrationPlanV1;
    PLANS.set(plan, Object.freeze({ authorities, receipt: RECEIPT }));
    return plan;
  } catch {
    throw new TypeError(INVALID_PLAN);
  }
}

export function assertSealedPostgresMigrationPlanV1(
  value: unknown,
): asserts value is SealedPostgresMigrationPlanV1 {
  try {
    if (isProxy(value) || value === null || typeof value !== 'object'
      || !PLANS.has(value)) throw new TypeError();
  } catch {
    throw new TypeError(INVALID_PLAN);
  }
}

export function postgresMigrationPlanAuthoritiesV1(
  value: unknown,
): PostgresMigrationPlanAuthoritiesV1 {
  assertSealedPostgresMigrationPlanV1(value);
  return PLANS.get(value)!.authorities;
}

export function copySealedPostgresMigrationPlanStepsV1(
  value: unknown,
): readonly PostgresMigrationPlanStepV1[] {
  const authorities = postgresMigrationPlanAuthoritiesV1(value);
  return Object.freeze([
    Object.freeze({
      stepKind: 'migration-sql' as const,
      migrationVersion: 1 as const,
      bytes: copyPostgresMigrationSqlBytesV1(authorities.migration0001),
    }),
    Object.freeze({
      stepKind: 'authority-seed' as const,
      bytes: copyPostgresAuthoritySeedBytesV1(authorities.authoritySeed),
    }),
    Object.freeze({
      stepKind: 'migration-sql' as const,
      migrationVersion: 2 as const,
      bytes: copyPostgresMigrationSqlBytesV1(authorities.migration0002),
    }),
  ]);
}

export function postgresMigrationPlanPreflightReceiptV1(
  value: unknown,
): PostgresMigrationPlanPreflightReceiptV1 {
  assertSealedPostgresMigrationPlanV1(value);
  return PLANS.get(value)!.receipt;
}

export function parsePostgresMigrationPlanPreflightReceiptV1(
  value: unknown,
): PostgresMigrationPlanPreflightReceiptV1 {
  try {
    const snapshot = snapshotClosedGraphV1(value, 'PostgreSQL migration receipt');
    if (canonicalJsonV1(snapshot) !== RECEIPT_CANONICAL) throw new TypeError();
    return RECEIPT;
  } catch {
    throw new TypeError(INVALID_RECEIPT);
  }
}

/** Replay re-opens only the fixed files and still grants no database/apply authority. */
export function replayPostgresMigrationPlanPreflightReceiptV1(
  value: unknown,
): SealedPostgresMigrationPlanV1 {
  const receipt = parsePostgresMigrationPlanPreflightReceiptV1(value);
  const plan = loadSealedPostgresMigrationPlanV1();
  if (plan.preflightReceiptSha256 !== receipt.receiptSha256) {
    throw new TypeError(INVALID_RECEIPT);
  }
  return plan;
}

function assertAtomicManifestBinding(
  manifest: ReturnType<typeof postgresMigrationManifestRecordV1>,
  catalogue: ParsedPostgresCatalogueContractV1,
  provisioning: ParsedPostgresProvisioningContractV1,
  seed: ParsedPostgresAuthoritySeedV1,
  migration0001: ParsedPostgresMigrationSqlV1,
  migration0002: ParsedPostgresMigrationSqlV1,
): void {
  if (manifest.catalogContract.path !== 'migrations/catalog-contract-v1.json'
    || manifest.catalogContract.bytes !== catalogue.rawByteLength
    || manifest.catalogContract.sha256 !== catalogue.rawSha256
    || manifest.provisioningContract.path !== 'migrations/provisioning-contract-v1.json'
    || manifest.provisioningContract.bytes !== provisioning.rawByteLength
    || manifest.provisioningContract.sha256 !== provisioning.rawSha256
    || manifest.authoritySeed.domain !== POSTGRES_AUTHORITY_SEED_DOMAIN_V1
    || manifest.authoritySeed.bytes !== seed.rawByteLength
    || manifest.authoritySeed.sha256 !== seed.rawSha256
    || manifest.migrations[0]?.path !== 'migrations/0001-registration-state-v1.sql'
    || manifest.migrations[0]?.version !== migration0001.version
    || manifest.migrations[0]?.bytes !== migration0001.rawByteLength
    || manifest.migrations[0]?.sha256 !== migration0001.rawSha256
    || manifest.migrations[1]?.path !== 'migrations/0002-registration-rls-v1.sql'
    || manifest.migrations[1]?.version !== migration0002.version
    || manifest.migrations[1]?.bytes !== migration0002.rawByteLength
    || manifest.migrations[1]?.sha256 !== migration0002.rawSha256) {
    throw new TypeError();
  }
}

function assertReceiptBinding(authorities: PostgresMigrationPlanAuthoritiesV1): void {
  const artifacts = RECEIPT_BODY.artifacts;
  const actual = Object.freeze({
    manifest: authorities.manifest,
    catalogueContract: authorities.catalogueContract,
    provisioningContract: authorities.provisioningContract,
    authoritySeed: authorities.authoritySeed,
    migration0001: authorities.migration0001,
    migration0002: authorities.migration0002,
  });
  for (const key of Object.keys(actual) as Array<keyof typeof actual>) {
    if (actual[key].rawByteLength !== artifacts[key].bytes
      || actual[key].rawSha256 !== artifacts[key].sha256) throw new TypeError();
  }
}
