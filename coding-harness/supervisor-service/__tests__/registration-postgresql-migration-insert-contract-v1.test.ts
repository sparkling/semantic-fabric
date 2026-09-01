// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { postgresAuthoritySeedInsertProjectionV1 }
  from '../src/registration-postgresql-authority-seed-v1.js';
import {
  copyPostgresAuthorityConfigurationInsertValuesV1,
  copyPostgresAuthorityStateInsertValuesV1,
  copyPostgresMigrationInsertCompletionContractV1,
  copyPostgresMigrationLedgerVersion1InsertValuesV1,
  copyPostgresMigrationLedgerVersion2InsertValuesV1,
} from '../src/registration-postgresql-migration-insert-contract-v1.js';
import {
  loadSealedPostgresMigrationPlanV1,
  postgresMigrationPlanAuthoritiesV1,
  postgresMigrationPlanPreflightReceiptV1,
} from '../src/registration-postgresql-migration-plan-v1.js';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const SOURCE_PATH = resolve(
  ROOT, 'src/registration-postgresql-migration-insert-contract-v1.ts',
);
const CATALOGUE_SOURCE_PATH = resolve(
  ROOT, 'src/registration-postgresql-migration-command-catalogue-v1.ts',
);
const OPERATIONS = [
  'seed-authority-configuration-insert',
  'seed-authority-state-insert',
  'ledger-insert-version-1',
  'ledger-insert-version-2',
] as const;
type Operation = typeof OPERATIONS[number];
type ValueSet = Readonly<{ operation: Operation; values: readonly unknown[] }>;
const CONFIGURATION_EXPECTED = [
  'b32:9ba3490c3ff9becc86163de81360b6aa1ea64e9e2f7098ec72daefa5a66b77bf',
  'sf_supervisor_project_scope_v1', '0',
  'b32:90ce861103ea0bcacebc37ee97f502fa5080ab5e9ab1b542c15393b96fac4c02',
  'b32:80b825d44f308c1ef66d92129f2c34e5b05e88ee839ec7ca91bf2301f2013146',
  'b7231:4152175917e9441505ac17611dbd064181defe6fbe4620bd550d773fcbf59d48',
  'b32:4152175917e9441505ac17611dbd064181defe6fbe4620bd550d773fcbf59d48',
  'project_client_20260829',
  'b32:39872817331eed587ca8150e5c6a9712e3f0e9fbfc4acfab1622d01849af95d5',
  'supervisor_service_20260829', '1',
  'b32:06e3fd8fda29bb60ab59557de61edb0aecdb231134be30e75b455f8e1b792fa9',
  'b44:302a300506032b6570032100d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a',
  'b32:8ab65ed0931fcb6b341cec2e22ce75ce131954c40822987815a3904eb5cebd91',
] as const;
const STATE_EXPECTED = [
  'b32:9ba3490c3ff9becc86163de81360b6aa1ea64e9e2f7098ec72daefa5a66b77bf',
  'sf_supervisor_project_scope_v1', true, '0',
  'b32:90ce861103ea0bcacebc37ee97f502fa5080ab5e9ab1b542c15393b96fac4c02',
  'b32:80b825d44f308c1ef66d92129f2c34e5b05e88ee839ec7ca91bf2301f2013146',
  '0', '1', null,
] as const;
const LEDGER_V1_EXPECTED = [
  1,
  'b32:c923f0f725c009a65ef85bc1881b7ae5717a1eca148bbf5316aeee60bb4a31c1',
  'b32:e7ce3572463587f4beed55c35c5a6b93810a270136cb963cf312b580fd1ace69',
  'b32:b6d3b0f77a2b71cb10782152cb74a3f37320e220131a2d3ef29cc455c9a26e4c',
] as const;
const LEDGER_V2_EXPECTED = [
  2,
  'b32:1d620d95f630997785d0d3adf724e5befe458c0c41e0746f45713cb584b58765',
  'b32:e7ce3572463587f4beed55c35c5a6b93810a270136cb963cf312b580fd1ace69',
  'b32:b6d3b0f77a2b71cb10782152cb74a3f37320e220131a2d3ef29cc455c9a26e4c',
] as const;
const EXPECTED_VALUES = [
  CONFIGURATION_EXPECTED, STATE_EXPECTED, LEDGER_V1_EXPECTED, LEDGER_V2_EXPECTED,
] as const;

describe('PostgreSQL migration INSERT contract V1', () => {
  it('copies four exact non-authorizing Plan-bound value sets', () => {
    const plan = loadSealedPostgresMigrationPlanV1();
    const receipt = postgresMigrationPlanPreflightReceiptV1(plan);
    valueSets(plan).forEach((set, index) => {
      expect(Reflect.ownKeys(set)).toEqual([
        'valueSetKind', 'operation', 'authority', 'readinessAuthorized',
        'databaseAccessAuthorized', 'migrationApplyAuthorized', 'executableAuthority',
        'sourcePlanReceiptSha256', 'values',
      ]);
      expect(set).toMatchObject({
        valueSetKind: 'postgresql-migration-insert-value-set-v1',
        operation: OPERATIONS[index], authority: 'none', readinessAuthorized: false,
        databaseAccessAuthorized: false, migrationApplyAuthorized: false,
        executableAuthority: false, sourcePlanReceiptSha256: receipt.receiptSha256,
      });
      expect(set.values.map(normalizeValue)).toEqual(EXPECTED_VALUES[index]);
    });
  });

  it('keeps numeric domains canonical decimal strings and ledger versions integers', () => {
    const sets = valueSets(loadSealedPostgresMigrationPlanV1());
    for (const [setIndex, positions] of [[0, [2, 10]], [1, [3, 6, 7]]] as const) {
      positions.forEach((position) => {
        const value = sets[setIndex]!.values[position];
        expect(typeof value).toBe('string');
        expect(value).toMatch(/^(?:0|[1-9][0-9]{0,19})$/);
        expect(typeof value).not.toBe('number');
        expect(typeof value).not.toBe('bigint');
      });
    }
    expect(sets[1]!.values.slice(6)).toEqual(['0', '1', null]);
    expect(sets[2]!.values[0]).toBe(1);
    expect(sets[3]!.values[0]).toBe(2);
    expect(Number.isSafeInteger(sets[2]!.values[0])).toBe(true);
    expect(Number.isSafeInteger(sets[3]!.values[0])).toBe(true);
  });

  it('copies separate raw-wire and normalized completion evidence without admission', () => {
    const plan = loadSealedPostgresMigrationPlanV1();
    const receipt = postgresMigrationPlanPreflightReceiptV1(plan);
    const contract = copyPostgresMigrationInsertCompletionContractV1(plan);
    expect(Reflect.ownKeys(contract)).toEqual([
      'contractKind', 'authority', 'readinessAuthorized', 'databaseAccessAuthorized',
      'migrationApplyAuthorized', 'executableAuthority', 'resultAdmissionAuthorized',
      'sourcePlanReceiptSha256', 'completions',
    ]);
    expect(contract).toMatchObject({
      contractKind: 'postgresql-migration-insert-completion-contract-v1',
      authority: 'none', readinessAuthorized: false, databaseAccessAuthorized: false,
      migrationApplyAuthorized: false, executableAuthority: false,
      resultAdmissionAuthorized: false, sourcePlanReceiptSha256: receipt.receiptSha256,
    });
    expect(contract.completions.map((entry) => ({ ...entry, rows: [...entry.rows] })))
      .toEqual(OPERATIONS.map((operation) => ({
        operation, resultKind: 'command-complete', wireCommandTag: 'INSERT 0 1',
        normalizedCommandKind: 'INSERT', rowCount: 1, rows: [],
      })));
    contract.completions.forEach((entry) => {
      expect(Reflect.ownKeys(entry)).toEqual([
        'operation', 'resultKind', 'wireCommandTag', 'normalizedCommandKind',
        'rowCount', 'rows',
      ]);
      expect(Object.isFrozen(entry.rows)).toBe(true);
    });
  });

  it('returns fresh frozen intrinsic containers and fresh whole-buffer byte leaves', () => {
    const plan = loadSealedPostgresMigrationPlanV1();
    const first = valueSets(plan);
    const second = valueSets(plan);
    const firstBytes = first.flatMap((set) => inspectGraph(set));
    const secondBytes = second.flatMap((set) => inspectGraph(set));
    expect(new Set(firstBytes).size).toBe(firstBytes.length);
    expect(new Set([...firstBytes, ...secondBytes]).size)
      .toBe(firstBytes.length + secondBytes.length);
    for (const bytes of [...firstBytes, ...secondBytes]) {
      expect(Object.getPrototypeOf(bytes)).toBe(Uint8Array.prototype);
      expect(bytes.byteOffset).toBe(0);
      expect(bytes.buffer.byteLength).toBe(bytes.byteLength);
      expect(Object.isFrozen(bytes)).toBe(false);
    }
    const mutable = firstBytes[0]!;
    const original = mutable[0]!;
    mutable[0] = original ^ 0xff;
    expect(inspectGraph(valueSets(plan)[0]!)[0]![0]).toBe(original);

    const completionA = copyPostgresMigrationInsertCompletionContractV1(plan);
    const completionB = copyPostgresMigrationInsertCompletionContractV1(plan);
    inspectGraph(completionA);
    completionA.completions.forEach((entry, index) => {
      expect(entry).not.toBe(completionB.completions[index]);
      expect(entry.rows).not.toBe(completionB.completions[index]!.rows);
    });
  });

  it('uses only the narrow branded validated immutable seed projection', () => {
    const plan = loadSealedPostgresMigrationPlanV1();
    const seed = postgresMigrationPlanAuthoritiesV1(plan).authoritySeed;
    const projection = postgresAuthoritySeedInsertProjectionV1(seed);
    expect(Reflect.ownKeys(projection)).toEqual([
      'authorityConfiguration', 'authorityStateIdentity',
    ]);
    expect(Object.isFrozen(projection)).toBe(true);
    expect(Object.isFrozen(projection.authorityConfiguration)).toBe(true);
    expect(Object.isFrozen(projection.authorityStateIdentity)).toBe(true);
    expect(projection.authorityConfiguration).toMatchObject({
      projectScopeRole: 'sf_supervisor_project_scope_v1',
      configurationEpoch: '0', serviceKeyEpoch: '1',
    });
    expect(projection.authorityStateIdentity).toMatchObject({
      projectScopeRole: 'sf_supervisor_project_scope_v1', singletonKey: true,
      activeConfigurationEpoch: '0',
    });
    expect(() => postgresAuthoritySeedInsertProjectionV1(structuredClone(seed)))
      .toThrow('PostgreSQL authority seed is invalid');
  });

  it('rejects clones, proxies, raw bytes, and unbranded Plan lookalikes', () => {
    const plan = loadSealedPostgresMigrationPlanV1();
    const candidates = [
      structuredClone(plan),
      new Proxy(plan, { get: () => { throw new Error('trap'); } }),
      new Uint8Array([1, 2, 3]), null, { ...structuredClone(plan) },
    ];
    const apis = [
      copyPostgresAuthorityConfigurationInsertValuesV1,
      copyPostgresAuthorityStateInsertValuesV1,
      copyPostgresMigrationLedgerVersion1InsertValuesV1,
      copyPostgresMigrationLedgerVersion2InsertValuesV1,
      copyPostgresMigrationInsertCompletionContractV1,
    ];
    candidates.forEach((candidate) => apis.forEach((api) => {
      expect(() => api(candidate)).toThrow('PostgreSQL migration Plan is invalid');
    }));
  });

  it('copies through captured intrinsics after host globals are poisoned', () => {
    const plan = loadSealedPostgresMigrationPlanV1();
    const freeze = Object.freeze;
    const create = Object.create;
    const defineProperty = Object.defineProperty;
    const bufferFrom = Buffer.from;
    const bigint = globalThis.BigInt;
    const regexpTest = RegExp.prototype.test;
    const typedSet = Uint8Array.prototype.set;
    const iterator = Object.getOwnPropertyDescriptor(Array.prototype, Symbol.iterator)!;
    let set: ValueSet | undefined;
    let completion: ReturnType<typeof copyPostgresMigrationInsertCompletionContractV1> | undefined;
    try {
      defineProperty(Array.prototype, Symbol.iterator, {
        ...iterator, value: () => { throw new Error('poisoned iterator'); },
      });
      Object.freeze = () => { throw new Error('poisoned freeze'); };
      Object.create = () => { throw new Error('poisoned create'); };
      Object.defineProperty = () => { throw new Error('poisoned defineProperty'); };
      (Buffer as any).from = () => { throw new Error('poisoned Buffer.from'); };
      (globalThis as any).BigInt = () => { throw new Error('poisoned BigInt'); };
      RegExp.prototype.test = () => { throw new Error('poisoned RegExp.test'); };
      Uint8Array.prototype.set = () => { throw new Error('poisoned Uint8Array.set'); };
      set = copyPostgresAuthorityConfigurationInsertValuesV1(plan);
      completion = copyPostgresMigrationInsertCompletionContractV1(plan);
    } finally {
      Object.freeze = freeze;
      Object.create = create;
      Object.defineProperty = defineProperty;
      (Buffer as any).from = bufferFrom;
      (globalThis as any).BigInt = bigint;
      RegExp.prototype.test = regexpTest;
      Uint8Array.prototype.set = typedSet;
      defineProperty(Array.prototype, Symbol.iterator, iterator);
    }
    expect(set?.values.map(normalizeValue)).toEqual(CONFIGURATION_EXPECTED);
    expect(completion?.completions).toHaveLength(4);
    inspectGraph(set);
    inspectGraph(completion);
  });

  it('is private, import-safe, SQL-free, driver-free, and separate from descriptors', async () => {
    const source = readFileSync(SOURCE_PATH, 'utf8');
    const catalogueSource = readFileSync(CATALOGUE_SOURCE_PATH, 'utf8');
    expect(source).not.toMatch(/node:fs|node:path|node:net|\bpg\b|\bPromise\b|\bexecute\b|\bstore\b/);
    expect(source).not.toMatch(/\b(?:SELECT|INSERT INTO|UPDATE|DELETE FROM)\b/);
    expect(source).not.toContain('registration-postgresql-migration-command-catalogue');
    expect(catalogueSource).not.toContain('registration-postgresql-migration-insert-contract');
    const publicModule = await import('../src/index.js');
    for (const key of [
      'copyPostgresAuthorityConfigurationInsertValuesV1',
      'copyPostgresAuthorityStateInsertValuesV1',
      'copyPostgresMigrationLedgerVersion1InsertValuesV1',
      'copyPostgresMigrationLedgerVersion2InsertValuesV1',
      'copyPostgresMigrationInsertCompletionContractV1',
    ]) expect(publicModule).not.toHaveProperty(key);
  });
});

function valueSets(plan: unknown): readonly ValueSet[] {
  return [
    copyPostgresAuthorityConfigurationInsertValuesV1(plan),
    copyPostgresAuthorityStateInsertValuesV1(plan),
    copyPostgresMigrationLedgerVersion1InsertValuesV1(plan),
    copyPostgresMigrationLedgerVersion2InsertValuesV1(plan),
  ];
}

function normalizeValue(value: unknown): unknown {
  if (!(value instanceof Uint8Array)) return value;
  if (value.byteLength === 7_231) {
    return `b7231:${createHash('sha256').update(value).digest('hex')}`;
  }
  return `b${value.byteLength}:${Buffer.from(value).toString('hex')}`;
}

function inspectGraph(value: unknown, seen = new Set<object>()): Uint8Array[] {
  expect(typeof value).not.toBe('function');
  if (value instanceof Uint8Array) return [value];
  if (value === null || typeof value !== 'object' || seen.has(value)) return [];
  seen.add(value);
  expect(Object.isFrozen(value)).toBe(true);
  expect(Object.getPrototypeOf(value))
    .toBe(Array.isArray(value) ? Array.prototype : Object.prototype);
  const bytes: Uint8Array[] = [];
  for (const key of Reflect.ownKeys(value)) {
    if (key === 'length') continue;
    expect(typeof key).toBe('string');
    expect(['text', 'sql', 'driver', 'store', 'execute', 'handle', 'result'])
      .not.toContain(key);
    bytes.push(...inspectGraph((value as Record<string, unknown>)[key as string], seen));
  }
  return bytes;
}
