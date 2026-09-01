// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { describe, expect, it } from 'vitest';
import {
  assertPostgresMigrationLifecycleContractV1,
  copyPostgresMigrationLifecycleContractV1,
} from '../src/registration-postgresql-migration-lifecycle-v1.js';
import { inspectPostgresMigrationSqlCandidateV1 }
  from '../src/registration-postgresql-migration-sql-policy-v1.js';

const SERVICE_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const LIFECYCLE_SOURCE = resolve(
  SERVICE_ROOT, 'src/registration-postgresql-migration-lifecycle-v1.ts',
);
const EXECUTE_OPERATIONS = [
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
] as const;
const LIFECYCLE_NAMES = [
  ...EXECUTE_OPERATIONS,
  'checkout',
  'open',
  'release',
  'destroy',
  'discard-malformed',
] as const;
const EMPTY_APPLY_SCHEDULE = [
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
] as const;
const EXACT_NO_OP_SCHEDULE = [
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
] as const;
const CONTROL_SQL = {
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
} as const;

describe('PostgreSQL migration lifecycle contract V1', () => {
  it('pins the closed lifecycle and execute-operation reachability', () => {
    const contract = copyPostgresMigrationLifecycleContractV1();
    const reachableExecutes = new Set([
      ...EMPTY_APPLY_SCHEDULE,
      ...EXACT_NO_OP_SCHEDULE,
      'rollback',
    ].filter((name) => (EXECUTE_OPERATIONS as readonly string[]).includes(name)));

    expect(Reflect.ownKeys(contract)).toEqual([
      'contractKind', 'authority', 'readinessAuthorized', 'databaseAccessAuthorized',
      'migrationApplyAuthorized', 'executableAuthority', 'lifecycleNames',
      'executeOperations', 'emptyApplySchedule', 'exactNoOpSchedule', 'controlSql',
      'deadlines',
    ]);
    expect(contract).toMatchObject({
      contractKind: 'postgresql-migration-lifecycle-contract-v1',
      authority: 'none',
      readinessAuthorized: false,
      databaseAccessAuthorized: false,
      migrationApplyAuthorized: false,
      executableAuthority: false,
    });
    expect(contract.lifecycleNames).toEqual(LIFECYCLE_NAMES);
    expect(contract.executeOperations).toEqual(EXECUTE_OPERATIONS);
    expect(contract.lifecycleNames).toHaveLength(36);
    expect(contract.executeOperations).toHaveLength(31);
    expect(new Set(contract.lifecycleNames)).toHaveLength(36);
    expect(new Set(contract.executeOperations)).toHaveLength(31);
    expect([...reachableExecutes]).toEqual(EXECUTE_OPERATIONS);
    expect(contract.executeOperations.every(
      (name) => contract.lifecycleNames.includes(name),
    )).toBe(true);
  });

  it('pins both successful schedules in exact order', () => {
    const contract = copyPostgresMigrationLifecycleContractV1();

    expect(contract.emptyApplySchedule).toEqual(EMPTY_APPLY_SCHEDULE);
    expect(contract.exactNoOpSchedule).toEqual(EXACT_NO_OP_SCHEDULE);
    expect(contract.emptyApplySchedule).toHaveLength(32);
    expect(contract.exactNoOpSchedule).toHaveLength(24);
    expect(new Set(contract.emptyApplySchedule)).toHaveLength(32);
    expect(new Set(contract.exactNoOpSchedule)).toHaveLength(24);
    for (const schedule of [contract.emptyApplySchedule, contract.exactNoOpSchedule]) {
      expect(schedule.every((name) => contract.lifecycleNames.includes(name))).toBe(true);
      expect(schedule.slice(0, 2)).toEqual(['checkout', 'open']);
      expect(schedule.slice(-2)).toEqual(['commit', 'release']);
    }
  });

  it('pins exact LF-terminated control statement bytes without semicolons', () => {
    const { controlSql } = copyPostgresMigrationLifecycleContractV1();

    expect(controlSql).toEqual(CONTROL_SQL);
    for (const [operation, text] of Object.entries(controlSql)) {
      expect(new TextDecoder().decode(new TextEncoder().encode(text)), operation).toBe(text);
      expect(text.endsWith('\n'), operation).toBe(true);
      expect(text.includes('\r'), operation).toBe(false);
      expect(text.includes(';'), operation).toBe(false);
    }
  });

  it('derives fixed script and invocation ceilings from sealed statement counts', () => {
    const { deadlines } = copyPostgresMigrationLifecycleContractV1();
    const first = inspectPostgresMigrationSqlCandidateV1(new Uint8Array(readFileSync(
      resolve(SERVICE_ROOT, 'migrations/0001-registration-state-v1.sql'),
    ))).inventory.statements;
    const second = inspectPostgresMigrationSqlCandidateV1(new Uint8Array(readFileSync(
      resolve(SERVICE_ROOT, 'migrations/0002-registration-rls-v1.sql'),
    ))).inventory.statements;

    expect([first, second]).toEqual([52, 73]);
    expect(deadlines.migration0001StatementCount).toBe(first);
    expect(deadlines.migration0002StatementCount).toBe(second);
    expect(deadlines.migration0001Milliseconds).toBe(
      first * deadlines.statementTimeoutMilliseconds
        + deadlines.migrationProtocolMarginMilliseconds,
    );
    expect(deadlines.migration0002Milliseconds).toBe(
      second * deadlines.statementTimeoutMilliseconds
        + deadlines.migrationProtocolMarginMilliseconds,
    );
    const successfulNormalWork = deadlines.checkoutMilliseconds
      + deadlines.openMilliseconds
      + deadlines.emptyApplyOrdinaryExecuteCount * deadlines.ordinaryExecuteMilliseconds
      + deadlines.advisoryLockExecuteMilliseconds
      + deadlines.migration0001Milliseconds
      + deadlines.migration0002Milliseconds
      + deadlines.commitMilliseconds
      + deadlines.emptyApplySynchronousGapCount * deadlines.synchronousGapMilliseconds;
    expect(successfulNormalWork).toBe(5_030_000);
    expect(deadlines.successfulNormalWorkMilliseconds).toBe(successfulNormalWork);
    expect(deadlines.successfulWholeWithReleaseMilliseconds).toBe(5_040_000);
    expect(deadlines.normalWorkCutoffMilliseconds - successfulNormalWork).toBe(360_000);
    expect(deadlines.wholeInvocationMilliseconds
      - deadlines.successfulWholeWithReleaseMilliseconds).toBe(360_000);
    expect(deadlines.latestRejectedOrdinaryOrRollbackCount).toBe(26);
  });

  it('returns fresh, recursively frozen non-authorizing copies', () => {
    const first = copyPostgresMigrationLifecycleContractV1();
    const second = copyPostgresMigrationLifecycleContractV1();

    expect(first).not.toBe(second);
    for (const key of [
      'lifecycleNames', 'executeOperations', 'emptyApplySchedule', 'exactNoOpSchedule',
      'controlSql', 'deadlines',
    ] as const) {
      expect(first[key]).not.toBe(second[key]);
      expect(Object.isFrozen(first[key]), key).toBe(true);
    }
    expect(Object.isFrozen(first)).toBe(true);
    expect(assertPostgresMigrationLifecycleContractV1(structuredClone(first)))
      .toEqual(first);
  });

  it('copies through captured intrinsics after host globals are poisoned', () => {
    const iterator = Object.getOwnPropertyDescriptor(Array.prototype, Symbol.iterator)!;
    const freeze = Object.freeze;
    const create = Object.create;
    const defineProperty = Object.defineProperty;
    const ownKeys = Reflect.ownKeys;
    let copied: ReturnType<typeof copyPostgresMigrationLifecycleContractV1> | undefined;
    try {
      defineProperty(Array.prototype, Symbol.iterator, {
        ...iterator,
        value: () => { throw new Error('poisoned iterator'); },
      });
      Object.freeze = () => { throw new Error('poisoned freeze'); };
      Object.create = () => { throw new Error('poisoned create'); };
      Object.defineProperty = () => { throw new Error('poisoned defineProperty'); };
      Reflect.ownKeys = () => { throw new Error('poisoned ownKeys'); };
      copied = copyPostgresMigrationLifecycleContractV1();
    } finally {
      Object.freeze = freeze;
      Object.create = create;
      Object.defineProperty = defineProperty;
      Reflect.ownKeys = ownKeys;
      defineProperty(Array.prototype, Symbol.iterator, iterator);
    }
    expect(copied).toMatchObject({
      authority: 'none',
      executableAuthority: false,
    });
    expect(copied?.lifecycleNames).toEqual(LIFECYCLE_NAMES);
    expect(Object.isFrozen(copied)).toBe(true);
  });

  it('rejects authority, order, duplication, omission, SQL, and deadline mutants', () => {
    const original = copyPostgresMigrationLifecycleContractV1();
    const mutants: unknown[] = [];
    const mutate = (change: (value: any) => void): void => {
      const candidate = structuredClone(original);
      change(candidate);
      mutants.push(candidate);
    };
    mutate((value) => { value.authority = 'migration'; });
    mutate((value) => { value.executableAuthority = true; });
    mutate((value) => { value.lifecycleNames.pop(); });
    mutate((value) => { value.executeOperations[30] = value.executeOperations[0]; });
    mutate((value) => { [value.emptyApplySchedule[2], value.emptyApplySchedule[3]] =
      [value.emptyApplySchedule[3], value.emptyApplySchedule[2]]; });
    mutate((value) => { value.controlSql.begin = `${value.controlSql.begin.trimEnd()};\n`; });
    mutate((value) => {
      value.controlSql['set-statement-timeout'] =
        "SET LOCAL statement_timeout TO '30001ms'\n";
    });
    mutate((value) => { value.deadlines.ordinaryExecuteMilliseconds += 1; });
    mutate((value) => { value.deadlines.successfulNormalWorkMilliseconds -= 1; });
    mutate((value) => { value.extra = false; });
    const accessor = structuredClone(original);
    Object.defineProperty(accessor, 'authority', { enumerable: true, get: () => 'none' });
    mutants.push(accessor, new Proxy(structuredClone(original), {
      get: () => { throw new Error('trap'); },
    }));

    for (const mutant of mutants) {
      expect(() => assertPostgresMigrationLifecycleContractV1(mutant))
        .toThrow('PostgreSQL migration lifecycle contract is invalid');
    }
  });

  it('bundles and imports without assets, I/O modules, or public exports', async () => {
    const built = await build({
      entryPoints: [LIFECYCLE_SOURCE],
      bundle: true,
      format: 'esm',
      platform: 'node',
      target: 'node20',
      write: false,
      metafile: true,
      logLevel: 'silent',
    });
    expect(built.outputFiles).toHaveLength(1);
    const imported = await import(`data:text/javascript;base64,${
      Buffer.from(built.outputFiles[0]!.contents).toString('base64')
    }`);
    expect(imported.copyPostgresMigrationLifecycleContractV1()).toMatchObject({
      authority: 'none',
      databaseAccessAuthorized: false,
      migrationApplyAuthorized: false,
      executableAuthority: false,
    });
    expect(Object.values(built.metafile.inputs)).toHaveLength(1);
    expect(Object.values(built.metafile.outputs)[0]!.imports).toEqual([
      { path: 'node:util/types', kind: 'import-statement', external: true },
    ]);

    const publicModule = await import('../src/index.js');
    expect(publicModule).not.toHaveProperty('copyPostgresMigrationLifecycleContractV1');
    expect(publicModule).not.toHaveProperty('assertPostgresMigrationLifecycleContractV1');
  });
});
