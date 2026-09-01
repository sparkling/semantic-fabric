// SPDX-License-Identifier: MIT

import { Buffer } from 'node:buffer';
import { postgresAuthoritySeedInsertProjectionV1 }
  from './registration-postgresql-authority-seed-v1.js';
import {
  postgresMigrationPlanAuthoritiesV1,
  postgresMigrationPlanPreflightReceiptV1,
} from './registration-postgresql-migration-plan-v1.js';

const APPLY = Reflect.apply;
const ARRAY = Array;
const BIGINT = BigInt;
const BUFFER_FROM = Buffer.from;
const BUFFER_TO_STRING = Buffer.prototype.toString;
const CREATE = Object.create;
const DEFINE_PROPERTY = Object.defineProperty;
const FREEZE = Object.freeze;
const PLAIN_OBJECT_PROTOTYPE = Object.prototype;
const REGEXP_TEST = RegExp.prototype.test;
const U8 = Uint8Array;
const U8_SET = Uint8Array.prototype.set;
const INVALID = 'PostgreSQL migration INSERT contract is invalid';

export type PostgresMigrationInsertOperationV1 =
  | 'seed-authority-configuration-insert'
  | 'seed-authority-state-insert'
  | 'ledger-insert-version-1'
  | 'ledger-insert-version-2';

export type PostgresMigrationInsertValueV1 =
  null | boolean | number | string | Uint8Array;

export interface PostgresMigrationInsertValueSetV1 {
  readonly valueSetKind: 'postgresql-migration-insert-value-set-v1';
  readonly operation: PostgresMigrationInsertOperationV1;
  readonly authority: 'none';
  readonly readinessAuthorized: false;
  readonly databaseAccessAuthorized: false;
  readonly migrationApplyAuthorized: false;
  readonly executableAuthority: false;
  readonly sourcePlanReceiptSha256: string;
  readonly values: readonly PostgresMigrationInsertValueV1[];
}

export interface PostgresMigrationInsertCompletionV1 {
  readonly operation: PostgresMigrationInsertOperationV1;
  readonly resultKind: 'command-complete';
  readonly wireCommandTag: 'INSERT 0 1';
  readonly normalizedCommandKind: 'INSERT';
  readonly rowCount: 1;
  readonly rows: readonly never[];
}

export interface PostgresMigrationInsertCompletionContractV1 {
  readonly contractKind: 'postgresql-migration-insert-completion-contract-v1';
  readonly authority: 'none';
  readonly readinessAuthorized: false;
  readonly databaseAccessAuthorized: false;
  readonly migrationApplyAuthorized: false;
  readonly executableAuthority: false;
  readonly resultAdmissionAuthorized: false;
  readonly sourcePlanReceiptSha256: string;
  readonly completions: readonly PostgresMigrationInsertCompletionV1[];
}

interface InsertContextV1 {
  readonly receiptSha256: string;
  readonly authorities: ReturnType<typeof postgresMigrationPlanAuthoritiesV1>;
  readonly seed: ReturnType<typeof postgresAuthoritySeedInsertProjectionV1>;
}

export function copyPostgresAuthorityConfigurationInsertValuesV1(
  plan: unknown,
): PostgresMigrationInsertValueSetV1 {
  const context = insertContextV1(plan);
  const row = context.seed.authorityConfiguration;
  return valueSetV1('seed-authority-configuration-insert', context.receiptSha256, [
    digestBytesV1(row.projectAuthorityDigest),
    row.projectScopeRole,
    canonicalUint64V1(row.configurationEpoch),
    digestBytesV1(row.configurationDigest),
    digestBytesV1(row.genesisAuthorityHeadDigest),
    base64UrlBytesV1(row.serializedConfiguration, 7_231),
    digestBytesV1(row.serializedConfigurationSha256),
    row.projectPrincipalId,
    digestBytesV1(row.projectAuthenticationPolicyDigest),
    row.servicePrincipalId,
    canonicalUint64V1(row.serviceKeyEpoch),
    digestBytesV1(row.serviceKeyFingerprint),
    base64UrlBytesV1(row.serviceSigningSpkiDer, 44),
    digestBytesV1(row.genesisSemanticReceiptDigest),
  ]);
}

export function copyPostgresAuthorityStateInsertValuesV1(
  plan: unknown,
): PostgresMigrationInsertValueSetV1 {
  const context = insertContextV1(plan);
  const row = context.seed.authorityStateIdentity;
  return valueSetV1('seed-authority-state-insert', context.receiptSha256, [
    digestBytesV1(row.projectAuthorityDigest),
    row.projectScopeRole,
    row.singletonKey,
    canonicalUint64V1(row.activeConfigurationEpoch),
    digestBytesV1(row.activeConfigurationDigest),
    digestBytesV1(row.authorityHeadDigest),
    '0',
    '1',
    null,
  ]);
}

export function copyPostgresMigrationLedgerVersion1InsertValuesV1(
  plan: unknown,
): PostgresMigrationInsertValueSetV1 {
  const context = insertContextV1(plan);
  const { migration0001, catalogueContract, authoritySeed } = context.authorities;
  if (migration0001.version !== 1) throw new TypeError(INVALID);
  return valueSetV1('ledger-insert-version-1', context.receiptSha256, [
    migration0001.version,
    digestBytesV1(migration0001.rawSha256),
    digestBytesV1(catalogueContract.rawSha256),
    digestBytesV1(authoritySeed.rawSha256),
  ]);
}

export function copyPostgresMigrationLedgerVersion2InsertValuesV1(
  plan: unknown,
): PostgresMigrationInsertValueSetV1 {
  const context = insertContextV1(plan);
  const { migration0002, catalogueContract, authoritySeed } = context.authorities;
  if (migration0002.version !== 2) throw new TypeError(INVALID);
  return valueSetV1('ledger-insert-version-2', context.receiptSha256, [
    migration0002.version,
    digestBytesV1(migration0002.rawSha256),
    digestBytesV1(catalogueContract.rawSha256),
    digestBytesV1(authoritySeed.rawSha256),
  ]);
}

export function copyPostgresMigrationInsertCompletionContractV1(
  plan: unknown,
): PostgresMigrationInsertCompletionContractV1 {
  const receipt = postgresMigrationPlanPreflightReceiptV1(plan);
  const completions = copyArrayV1<PostgresMigrationInsertCompletionV1>([
    completionV1('seed-authority-configuration-insert'),
    completionV1('seed-authority-state-insert'),
    completionV1('ledger-insert-version-1'),
    completionV1('ledger-insert-version-2'),
  ]);
  return recordV1([
    ['contractKind', 'postgresql-migration-insert-completion-contract-v1'],
    ['authority', 'none'],
    ['readinessAuthorized', false],
    ['databaseAccessAuthorized', false],
    ['migrationApplyAuthorized', false],
    ['executableAuthority', false],
    ['resultAdmissionAuthorized', false],
    ['sourcePlanReceiptSha256', receipt.receiptSha256],
    ['completions', completions],
  ]);
}

function insertContextV1(plan: unknown): InsertContextV1 {
  const authorities = postgresMigrationPlanAuthoritiesV1(plan);
  const receipt = postgresMigrationPlanPreflightReceiptV1(plan);
  return {
    receiptSha256: receipt.receiptSha256,
    authorities,
    seed: postgresAuthoritySeedInsertProjectionV1(authorities.authoritySeed),
  };
}

function valueSetV1(
  operation: PostgresMigrationInsertOperationV1,
  receiptSha256: string,
  values: readonly PostgresMigrationInsertValueV1[],
): PostgresMigrationInsertValueSetV1 {
  return recordV1([
    ['valueSetKind', 'postgresql-migration-insert-value-set-v1'],
    ['operation', operation],
    ['authority', 'none'],
    ['readinessAuthorized', false],
    ['databaseAccessAuthorized', false],
    ['migrationApplyAuthorized', false],
    ['executableAuthority', false],
    ['sourcePlanReceiptSha256', receiptSha256],
    ['values', copyArrayV1(values)],
  ]);
}

function completionV1(
  operation: PostgresMigrationInsertOperationV1,
): PostgresMigrationInsertCompletionV1 {
  return recordV1([
    ['operation', operation],
    ['resultKind', 'command-complete'],
    ['wireCommandTag', 'INSERT 0 1'],
    ['normalizedCommandKind', 'INSERT'],
    ['rowCount', 1],
    ['rows', copyArrayV1<never>([])],
  ]);
}

function digestBytesV1(value: string): Uint8Array {
  if (!matchesV1(/^[a-f0-9]{64}$/, value) || matchesV1(/^0+$/, value)) {
    throw new TypeError(INVALID);
  }
  return decodedBytesV1(value, 'hex', 32, /^[a-f0-9]+$/);
}

function base64UrlBytesV1(value: string, exactBytes: number): Uint8Array {
  return decodedBytesV1(value, 'base64url', exactBytes, /^[A-Za-z0-9_-]+$/);
}

function decodedBytesV1(
  value: string,
  encoding: 'hex' | 'base64url',
  exactBytes: number,
  pattern: RegExp,
): Uint8Array {
  try {
    if (value.length === 0 || !APPLY(REGEXP_TEST, pattern, [value])) throw new TypeError();
    const decoded = BUFFER_FROM(value, encoding);
    if (decoded.byteLength !== exactBytes
      || APPLY(BUFFER_TO_STRING, decoded, [encoding]) !== value) throw new TypeError();
    const output = new U8(exactBytes);
    APPLY(U8_SET, output, [decoded]);
    return output;
  } catch {
    throw new TypeError(INVALID);
  }
}

function canonicalUint64V1(value: string): string {
  if (!matchesV1(/^(?:0|[1-9][0-9]{0,19})$/, value)
    || BIGINT(value) > 18_446_744_073_709_551_615n) throw new TypeError(INVALID);
  return value;
}

function matchesV1(pattern: RegExp, value: string): boolean {
  return APPLY(REGEXP_TEST, pattern, [value]) as boolean;
}

function copyArrayV1<T>(values: readonly T[]): readonly T[] {
  const output = new ARRAY<T>(values.length);
  for (let index = 0; index < values.length; index += 1) {
    DEFINE_PROPERTY(output, index, {
      value: values[index], enumerable: true, configurable: true, writable: true,
    });
  }
  return FREEZE(output);
}

function recordV1<T>(entries: readonly (readonly [string, unknown])[]): T {
  const record = CREATE(PLAIN_OBJECT_PROTOTYPE) as Record<string, unknown>;
  for (let index = 0; index < entries.length; index += 1) {
    const entry = entries[index]!;
    DEFINE_PROPERTY(record, entry[0], {
      value: entry[1], enumerable: true, configurable: true, writable: true,
    });
  }
  return FREEZE(record) as T;
}
