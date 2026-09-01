// SPDX-License-Identifier: MIT

import { isProxy } from 'node:util/types';

const ARRAY_IS_ARRAY = Array.isArray;
const ARRAY = Array;
const CREATE = Object.create;
const DEFINE_PROPERTY = Object.defineProperty;
const FREEZE = Object.freeze;
const GET_OWN_PROPERTY_DESCRIPTOR = Object.getOwnPropertyDescriptor;
const GET_PROTOTYPE_OF = Object.getPrototypeOf;
const HAS_OWN = Object.hasOwn;
const OBJECT_IS = Object.is;
const OWN_KEYS = Reflect.ownKeys;
const ARRAY_PROTOTYPE = Array.prototype;
const PLAIN_OBJECT_PROTOTYPE = Object.prototype;
const TYPE_ERROR = TypeError;
const INVALID = 'PostgreSQL migration lifecycle contract is invalid';

const EXECUTE_OPERATIONS = FREEZE([
  'preflight-identity',
  'preflight-role-attributes',
  'preflight-role-membership',
  'begin',
  'set-search-path',
  'set-row-security',
  'set-lock-timeout',
  'set-statement-timeout',
  'set-idle-in-transaction-session-timeout',
  'set-synchronous-commit',
  'verify-settings',
  'advisory-lock',
  'set-local-role',
  'reverify-settings',
  'observe-dedicated-schema-state',
  'observe-provisioning-projection',
  'observe-public-acl-baseline',
  'observe-default-acl-absence',
  'migration-0001',
  'seed-authority-configuration-insert',
  'seed-authority-state-insert',
  'migration-0002',
  'ledger-insert-version-1',
  'ledger-insert-version-2',
  'replay-authority-configuration-row',
  'replay-authority-state-row',
  'compare-catalogue-projection',
  'compare-provisioning-projection',
  'commit',
  'observe-migration-ledger',
  'rollback',
] as const);
const LIFECYCLE_NAMES = FREEZE([
  ...EXECUTE_OPERATIONS,
  'checkout',
  'open',
  'release',
  'destroy',
  'discard-malformed',
] as const);
const EMPTY_APPLY_SCHEDULE = FREEZE([
  'checkout',
  'open',
  'preflight-identity',
  'preflight-role-attributes',
  'preflight-role-membership',
  'begin',
  'set-search-path',
  'set-row-security',
  'set-lock-timeout',
  'set-statement-timeout',
  'set-idle-in-transaction-session-timeout',
  'set-synchronous-commit',
  'verify-settings',
  'advisory-lock',
  'set-local-role',
  'reverify-settings',
  'observe-dedicated-schema-state',
  'observe-provisioning-projection',
  'observe-public-acl-baseline',
  'observe-default-acl-absence',
  'migration-0001',
  'seed-authority-configuration-insert',
  'seed-authority-state-insert',
  'migration-0002',
  'ledger-insert-version-1',
  'ledger-insert-version-2',
  'replay-authority-configuration-row',
  'replay-authority-state-row',
  'compare-catalogue-projection',
  'compare-provisioning-projection',
  'commit',
  'release',
] as const);
const EXACT_NO_OP_SCHEDULE = FREEZE([
  'checkout',
  'open',
  'preflight-identity',
  'preflight-role-attributes',
  'preflight-role-membership',
  'begin',
  'set-search-path',
  'set-row-security',
  'set-lock-timeout',
  'set-statement-timeout',
  'set-idle-in-transaction-session-timeout',
  'set-synchronous-commit',
  'verify-settings',
  'advisory-lock',
  'set-local-role',
  'reverify-settings',
  'observe-dedicated-schema-state',
  'observe-migration-ledger',
  'replay-authority-configuration-row',
  'replay-authority-state-row',
  'compare-catalogue-projection',
  'compare-provisioning-projection',
  'commit',
  'release',
] as const);
const CONTROL_SQL_KEYS = FREEZE([
  'begin',
  'set-search-path',
  'set-row-security',
  'set-lock-timeout',
  'set-statement-timeout',
  'set-idle-in-transaction-session-timeout',
  'set-synchronous-commit',
  'set-local-role',
  'commit',
  'rollback',
] as const);
const CONTROL_SQL = FREEZE({
  begin: 'BEGIN ISOLATION LEVEL READ COMMITTED READ WRITE\n',
  'set-search-path': 'SET LOCAL search_path TO pg_catalog\n',
  'set-row-security': 'SET LOCAL row_security TO on\n',
  'set-lock-timeout': "SET LOCAL lock_timeout TO '5000ms'\n",
  'set-statement-timeout': "SET LOCAL statement_timeout TO '30000ms'\n",
  'set-idle-in-transaction-session-timeout':
    "SET LOCAL idle_in_transaction_session_timeout TO '30000ms'\n",
  'set-synchronous-commit': 'SET LOCAL synchronous_commit TO on\n',
  'set-local-role': 'SET LOCAL ROLE sf_supervisor_owner_v1\n',
  commit: 'COMMIT\n',
  rollback: 'ROLLBACK\n',
} as const);

const STATEMENT_TIMEOUT_MILLISECONDS = 30_000;
const MIGRATION_PROTOCOL_MARGIN_MILLISECONDS = 10_000;
const MIGRATION_0001_STATEMENT_COUNT = 52;
const MIGRATION_0002_STATEMENT_COUNT = 73;
const MIGRATION_0001_MILLISECONDS = MIGRATION_0001_STATEMENT_COUNT
  * STATEMENT_TIMEOUT_MILLISECONDS + MIGRATION_PROTOCOL_MARGIN_MILLISECONDS;
const MIGRATION_0002_MILLISECONDS = MIGRATION_0002_STATEMENT_COUNT
  * STATEMENT_TIMEOUT_MILLISECONDS + MIGRATION_PROTOCOL_MARGIN_MILLISECONDS;
const EMPTY_APPLY_ORDINARY_EXECUTE_COUNT = 25;
const EMPTY_APPLY_SYNCHRONOUS_GAP_COUNT = 33;
const SUCCESSFUL_NORMAL_WORK_MILLISECONDS = 10_000 + 10_000
  + EMPTY_APPLY_ORDINARY_EXECUTE_COUNT * 40_000
  + 15_000 + MIGRATION_0001_MILLISECONDS + MIGRATION_0002_MILLISECONDS
  + 60_000 + EMPTY_APPLY_SYNCHRONOUS_GAP_COUNT * 5_000;
const SUCCESSFUL_WHOLE_WITH_RELEASE_MILLISECONDS =
  SUCCESSFUL_NORMAL_WORK_MILLISECONDS + 10_000;
const DEADLINE_KEYS = FREEZE([
  'checkoutMilliseconds',
  'openMilliseconds',
  'ordinaryExecuteMilliseconds',
  'advisoryLockExecuteMilliseconds',
  'statementTimeoutMilliseconds',
  'migrationProtocolMarginMilliseconds',
  'migration0001StatementCount',
  'migration0001Milliseconds',
  'migration0002StatementCount',
  'migration0002Milliseconds',
  'commitMilliseconds',
  'rollbackMilliseconds',
  'terminalMilliseconds',
  'synchronousGapMilliseconds',
  'normalWorkCutoffMilliseconds',
  'wholeInvocationMilliseconds',
  'emptyApplyOrdinaryExecuteCount',
  'emptyApplySynchronousGapCount',
  'latestRejectedOrdinaryOrRollbackCount',
  'successfulNormalWorkMilliseconds',
  'successfulWholeWithReleaseMilliseconds',
  'normalWorkMarginMilliseconds',
  'wholeInvocationMarginMilliseconds',
] as const);
const DEADLINES = FREEZE({
  checkoutMilliseconds: 10_000,
  openMilliseconds: 10_000,
  ordinaryExecuteMilliseconds: 40_000,
  advisoryLockExecuteMilliseconds: 15_000,
  statementTimeoutMilliseconds: STATEMENT_TIMEOUT_MILLISECONDS,
  migrationProtocolMarginMilliseconds: MIGRATION_PROTOCOL_MARGIN_MILLISECONDS,
  migration0001StatementCount: MIGRATION_0001_STATEMENT_COUNT,
  migration0001Milliseconds: MIGRATION_0001_MILLISECONDS,
  migration0002StatementCount: MIGRATION_0002_STATEMENT_COUNT,
  migration0002Milliseconds: MIGRATION_0002_MILLISECONDS,
  commitMilliseconds: 60_000,
  rollbackMilliseconds: 40_000,
  terminalMilliseconds: 10_000,
  synchronousGapMilliseconds: 5_000,
  normalWorkCutoffMilliseconds: 5_390_000,
  wholeInvocationMilliseconds: 5_400_000,
  emptyApplyOrdinaryExecuteCount: EMPTY_APPLY_ORDINARY_EXECUTE_COUNT,
  emptyApplySynchronousGapCount: EMPTY_APPLY_SYNCHRONOUS_GAP_COUNT,
  latestRejectedOrdinaryOrRollbackCount: 26,
  successfulNormalWorkMilliseconds: SUCCESSFUL_NORMAL_WORK_MILLISECONDS,
  successfulWholeWithReleaseMilliseconds: SUCCESSFUL_WHOLE_WITH_RELEASE_MILLISECONDS,
  normalWorkMarginMilliseconds: 5_390_000 - SUCCESSFUL_NORMAL_WORK_MILLISECONDS,
  wholeInvocationMarginMilliseconds:
    5_400_000 - SUCCESSFUL_WHOLE_WITH_RELEASE_MILLISECONDS,
} as const);
const ROOT_KEYS = FREEZE([
  'contractKind',
  'authority',
  'readinessAuthorized',
  'databaseAccessAuthorized',
  'migrationApplyAuthorized',
  'executableAuthority',
  'lifecycleNames',
  'executeOperations',
  'emptyApplySchedule',
  'exactNoOpSchedule',
  'controlSql',
  'deadlines',
] as const);

export type PostgresMigrationExecuteOperationV1 = typeof EXECUTE_OPERATIONS[number];
export type PostgresMigrationLifecycleNameV1 = typeof LIFECYCLE_NAMES[number];

export interface PostgresMigrationLifecycleContractV1 {
  readonly contractKind: 'postgresql-migration-lifecycle-contract-v1';
  readonly authority: 'none';
  readonly readinessAuthorized: false;
  readonly databaseAccessAuthorized: false;
  readonly migrationApplyAuthorized: false;
  readonly executableAuthority: false;
  readonly lifecycleNames: readonly PostgresMigrationLifecycleNameV1[];
  readonly executeOperations: readonly PostgresMigrationExecuteOperationV1[];
  readonly emptyApplySchedule: readonly PostgresMigrationLifecycleNameV1[];
  readonly exactNoOpSchedule: readonly PostgresMigrationLifecycleNameV1[];
  readonly controlSql: Readonly<Record<typeof CONTROL_SQL_KEYS[number], string>>;
  readonly deadlines: Readonly<Record<typeof DEADLINE_KEYS[number], number>>;
}

const CONTRACT: PostgresMigrationLifecycleContractV1 = FREEZE({
  contractKind: 'postgresql-migration-lifecycle-contract-v1',
  authority: 'none',
  readinessAuthorized: false,
  databaseAccessAuthorized: false,
  migrationApplyAuthorized: false,
  executableAuthority: false,
  lifecycleNames: LIFECYCLE_NAMES,
  executeOperations: EXECUTE_OPERATIONS,
  emptyApplySchedule: EMPTY_APPLY_SCHEDULE,
  exactNoOpSchedule: EXACT_NO_OP_SCHEDULE,
  controlSql: CONTROL_SQL,
  deadlines: DEADLINES,
});

/** Return fresh, frozen data that conveys no execution or database authority. */
export function copyPostgresMigrationLifecycleContractV1():
  PostgresMigrationLifecycleContractV1 {
  return FREEZE({
    contractKind: CONTRACT.contractKind,
    authority: CONTRACT.authority,
    readinessAuthorized: CONTRACT.readinessAuthorized,
    databaseAccessAuthorized: CONTRACT.databaseAccessAuthorized,
    migrationApplyAuthorized: CONTRACT.migrationApplyAuthorized,
    executableAuthority: CONTRACT.executableAuthority,
    lifecycleNames: copyArrayV1(CONTRACT.lifecycleNames),
    executeOperations: copyArrayV1(CONTRACT.executeOperations),
    emptyApplySchedule: copyArrayV1(CONTRACT.emptyApplySchedule),
    exactNoOpSchedule: copyArrayV1(CONTRACT.exactNoOpSchedule),
    controlSql: copyRecordV1(CONTRACT.controlSql, CONTROL_SQL_KEYS),
    deadlines: copyRecordV1(CONTRACT.deadlines, DEADLINE_KEYS),
  });
}

/** Validate exact lifecycle representation and return a fresh canonical copy. */
export function assertPostgresMigrationLifecycleContractV1(
  value: unknown,
): PostgresMigrationLifecycleContractV1 {
  try {
    assertExactRecordV1(value, ROOT_KEYS, CONTRACT, 6);
    const fields = recordValuesV1(value, ROOT_KEYS);
    assertExactArrayV1(fields[6], LIFECYCLE_NAMES);
    assertExactArrayV1(fields[7], EXECUTE_OPERATIONS);
    assertExactArrayV1(fields[8], EMPTY_APPLY_SCHEDULE);
    assertExactArrayV1(fields[9], EXACT_NO_OP_SCHEDULE);
    assertExactRecordV1(fields[10], CONTROL_SQL_KEYS, CONTROL_SQL);
    assertExactRecordV1(fields[11], DEADLINE_KEYS, DEADLINES);
  } catch {
    throw new TYPE_ERROR(INVALID);
  }
  return copyPostgresMigrationLifecycleContractV1();
}

function assertExactRecordV1(
  value: unknown,
  expectedKeys: readonly string[],
  expected: object,
  exactValueCount = expectedKeys.length,
): void {
  const values = recordValuesV1(value, expectedKeys);
  for (let index = 0; index < exactValueCount; index += 1) {
    const key = expectedKeys[index]!;
    const descriptor = GET_OWN_PROPERTY_DESCRIPTOR(expected, key);
    if (descriptor === undefined || !HAS_OWN(descriptor, 'value')
      || !OBJECT_IS(values[index], descriptor.value)) throw new TYPE_ERROR();
  }
}

function recordValuesV1(value: unknown, expectedKeys: readonly string[]): readonly unknown[] {
  if (isProxy(value) || value === null || typeof value !== 'object'
    || ARRAY_IS_ARRAY(value) || GET_PROTOTYPE_OF(value) !== PLAIN_OBJECT_PROTOTYPE) {
    throw new TYPE_ERROR();
  }
  const keys = OWN_KEYS(value);
  if (keys.length !== expectedKeys.length) throw new TYPE_ERROR();
  const values = new ARRAY<unknown>(expectedKeys.length);
  for (let index = 0; index < expectedKeys.length; index += 1) {
    const key = keys[index];
    const expectedKey = expectedKeys[index]!;
    if (key !== expectedKey) throw new TYPE_ERROR();
    const descriptor = GET_OWN_PROPERTY_DESCRIPTOR(value, expectedKey);
    if (descriptor === undefined || descriptor.enumerable !== true
      || !HAS_OWN(descriptor, 'value')) throw new TYPE_ERROR();
    DEFINE_PROPERTY(values, index, {
      value: descriptor.value,
      enumerable: true,
      configurable: true,
      writable: true,
    });
  }
  return values;
}

function copyArrayV1<T>(value: readonly T[]): readonly T[] {
  const copy = new ARRAY<T>(value.length);
  for (let index = 0; index < value.length; index += 1) {
    DEFINE_PROPERTY(copy, index, {
      value: value[index],
      enumerable: true,
      configurable: true,
      writable: true,
    });
  }
  return FREEZE(copy);
}

function copyRecordV1<T extends Readonly<Record<string, unknown>>>(
  value: T,
  keys: readonly string[],
): T {
  const copy = CREATE(PLAIN_OBJECT_PROTOTYPE) as Record<string, unknown>;
  for (let index = 0; index < keys.length; index += 1) {
    const key = keys[index]!;
    DEFINE_PROPERTY(copy, key, {
      value: value[key],
      enumerable: true,
      configurable: true,
      writable: true,
    });
  }
  return FREEZE(copy) as T;
}

function assertExactArrayV1(value: unknown, expected: readonly unknown[]): void {
  if (isProxy(value) || !ARRAY_IS_ARRAY(value) || GET_PROTOTYPE_OF(value) !== ARRAY_PROTOTYPE) {
    throw new TYPE_ERROR();
  }
  const keys = OWN_KEYS(value);
  if (keys.length !== expected.length + 1 || keys[keys.length - 1] !== 'length') {
    throw new TYPE_ERROR();
  }
  for (let index = 0; index < expected.length; index += 1) {
    const key = `${index}`;
    if (keys[index] !== key) throw new TYPE_ERROR();
    const descriptor = GET_OWN_PROPERTY_DESCRIPTOR(value, key);
    if (descriptor === undefined || descriptor.enumerable !== true
      || !HAS_OWN(descriptor, 'value') || !OBJECT_IS(descriptor.value, expected[index])) {
      throw new TYPE_ERROR();
    }
  }
  const length = GET_OWN_PROPERTY_DESCRIPTOR(value, 'length');
  if (length === undefined || length.enumerable !== false
    || !HAS_OWN(length, 'value') || length.value !== expected.length) throw new TYPE_ERROR();
}

if (MIGRATION_0001_MILLISECONDS !== 1_570_000
  || MIGRATION_0002_MILLISECONDS !== 2_200_000
  || SUCCESSFUL_NORMAL_WORK_MILLISECONDS !== 5_030_000
  || SUCCESSFUL_WHOLE_WITH_RELEASE_MILLISECONDS !== 5_040_000
  || DEADLINES.normalWorkMarginMilliseconds !== 360_000
  || DEADLINES.wholeInvocationMarginMilliseconds !== 360_000) {
  throw new TYPE_ERROR('PostgreSQL migration lifecycle deadline pins are invalid');
}
