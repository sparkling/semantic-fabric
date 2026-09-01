// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { copyPostgresMigrationCommandCatalogueV1 }
  from '../src/registration-postgresql-migration-command-catalogue-v1.js';
import { copyPostgresMigrationLifecycleContractV1 }
  from '../src/registration-postgresql-migration-lifecycle-v1.js';
import {
  loadSealedPostgresMigrationPlanV1,
  postgresMigrationPlanPreflightReceiptV1,
} from '../src/registration-postgresql-migration-plan-v1.js';

const SERVICE_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const SOURCE = resolve(
  SERVICE_ROOT, 'src/registration-postgresql-migration-command-catalogue-v1.ts',
);
const ROOT_KEYS = [
  'catalogueKind', 'authority', 'readinessAuthorized', 'databaseAccessAuthorized',
  'migrationApplyAuthorized', 'executableAuthority', 'statementCatalogueComplete',
  'resultContractsSealed', 'sourcePlanReceiptSha256', 'sourcePins', 'controlCommands',
  'insertCommands',
] as const;
const CONTROL_OPERATIONS = [
  'begin', 'set-search-path', 'set-row-security', 'set-lock-timeout',
  'set-statement-timeout', 'set-idle-in-transaction-session-timeout',
  'set-synchronous-commit', 'set-local-role', 'commit', 'rollback',
] as const;
const INSERT_OPERATIONS = [
  'seed-authority-configuration-insert',
  'seed-authority-state-insert',
  'ledger-insert-version-1',
  'ledger-insert-version-2',
] as const;
const TABLES = [
  'sf_supervisor_v1.authority_configurations',
  'sf_supervisor_v1.authority_state',
  'sf_supervisor_v1.schema_migrations',
  'sf_supervisor_v1.schema_migrations',
] as const;
const COLUMNS = [
  [
    'project_authority_digest', 'project_scope_role', 'configuration_epoch',
    'configuration_digest', 'genesis_authority_head_digest',
    'serialized_configuration', 'serialized_configuration_sha256',
    'project_principal_id', 'project_authentication_policy_digest',
    'service_principal_id', 'service_key_epoch', 'service_key_fingerprint',
    'service_signing_spki_der', 'genesis_semantic_receipt_digest',
  ],
  [
    'project_authority_digest', 'project_scope_role', 'singleton_key',
    'active_configuration_epoch', 'active_configuration_digest',
    'authority_head_digest', 'last_global_sequence', 'next_global_sequence',
    'last_event_digest',
  ],
  [
    'migration_version', 'script_sha256', 'catalog_contract_sha256',
    'authority_seed_sha256',
  ],
  [
    'migration_version', 'script_sha256', 'catalog_contract_sha256',
    'authority_seed_sha256',
  ],
] as const;
const SQL_PINS = [
  [801, '0c94d380f405c2f66ec8f94e7ef630718046107d92f1428b5d4db6846d47d2a6'],
  [500, '34d377aa7994206ea3d06dda78a9faeb66a6eaab1709aac5045a3f4fcbe05e16'],
  [245, '362a5285760f16f6a662e42b599d3422a64745eb6e03493bfd08f9e450733d03'],
  [245, '362a5285760f16f6a662e42b599d3422a64745eb6e03493bfd08f9e450733d03'],
] as const;
const CONFIGURATION_PARAMETERS = [
  ['bytea', 'authoritySeed.authorityConfiguration.projectAuthorityDigest', 'sha256-hex'],
  ['text', 'authoritySeed.authorityConfiguration.projectScopeRole', 'utf8-text'],
  ['numeric', 'authoritySeed.authorityConfiguration.configurationEpoch', 'canonical-uint64-decimal'],
  ['bytea', 'authoritySeed.authorityConfiguration.configurationDigest', 'sha256-hex'],
  ['bytea', 'authoritySeed.authorityConfiguration.genesisAuthorityHeadDigest', 'sha256-hex'],
  ['bytea', 'authoritySeed.authorityConfiguration.serializedConfiguration', 'base64url'],
  ['bytea', 'authoritySeed.authorityConfiguration.serializedConfigurationSha256', 'sha256-hex'],
  ['text', 'authoritySeed.authorityConfiguration.projectPrincipalId', 'utf8-text'],
  ['bytea', 'authoritySeed.authorityConfiguration.projectAuthenticationPolicyDigest', 'sha256-hex'],
  ['text', 'authoritySeed.authorityConfiguration.servicePrincipalId', 'utf8-text'],
  ['numeric', 'authoritySeed.authorityConfiguration.serviceKeyEpoch', 'canonical-uint64-decimal'],
  ['bytea', 'authoritySeed.authorityConfiguration.serviceKeyFingerprint', 'sha256-hex'],
  ['bytea', 'authoritySeed.authorityConfiguration.serviceSigningSpkiDer', 'base64url'],
  ['bytea', 'authoritySeed.authorityConfiguration.genesisSemanticReceiptDigest', 'sha256-hex'],
] as const;
const STATE_PARAMETERS = [
  ['bytea', 'authoritySeed.authorityStateIdentity.projectAuthorityDigest', 'sha256-hex'],
  ['text', 'authoritySeed.authorityStateIdentity.projectScopeRole', 'utf8-text'],
  ['bool', 'authoritySeed.authorityStateIdentity.singletonKey', 'boolean'],
  ['numeric', 'authoritySeed.authorityStateIdentity.activeConfigurationEpoch', 'canonical-uint64-decimal'],
  ['bytea', 'authoritySeed.authorityStateIdentity.activeConfigurationDigest', 'sha256-hex'],
  ['bytea', 'authoritySeed.authorityStateIdentity.authorityHeadDigest', 'sha256-hex'],
  ['numeric', 'compiledGenesis.lastGlobalSequence', 'canonical-uint64-decimal'],
  ['numeric', 'compiledGenesis.nextGlobalSequence', 'canonical-uint64-decimal'],
  ['bytea', 'compiledGenesis.lastEventDigest', 'null'],
] as const;
const LEDGER_V1_PARAMETERS = [
  ['int4', 'migration0001.migrationVersion', 'safe-integer'],
  ['bytea', 'migration0001.rawSha256', 'sha256-hex'],
  ['bytea', 'catalogueContract.rawSha256', 'sha256-hex'],
  ['bytea', 'authoritySeed.rawSha256', 'sha256-hex'],
] as const;
const LEDGER_V2_PARAMETERS = [
  ['int4', 'migration0002.migrationVersion', 'safe-integer'],
  ['bytea', 'migration0002.rawSha256', 'sha256-hex'],
  ['bytea', 'catalogueContract.rawSha256', 'sha256-hex'],
  ['bytea', 'authoritySeed.rawSha256', 'sha256-hex'],
] as const;
const PARAMETER_VECTORS = [
  CONFIGURATION_PARAMETERS, STATE_PARAMETERS, LEDGER_V1_PARAMETERS, LEDGER_V2_PARAMETERS,
] as const;

function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}

function inspectGraph(value: unknown, seen = new Set<object>()): void {
  expect(typeof value).not.toBe('function');
  expect(value).not.toBeInstanceOf(Uint8Array);
  if (value === null || typeof value !== 'object' || seen.has(value)) return;
  seen.add(value);
  expect(Object.isFrozen(value)).toBe(true);
  if (Array.isArray(value)) expect(Object.getPrototypeOf(value)).toBe(Array.prototype);
  else expect(Object.getPrototypeOf(value)).toBe(Object.prototype);
  for (const key of Reflect.ownKeys(value)) {
    if (key === 'length') continue;
    expect(typeof key).toBe('string');
    expect(['values', 'result', 'results', 'driver', 'store', 'execute', 'handle'])
      .not.toContain(key);
    inspectGraph((value as Record<string, unknown>)[key as string], seen);
  }
}

describe('PostgreSQL migration command catalogue V1', () => {
  it('requires a branded plan and copies exact receipt pins without authority', () => {
    const plan = loadSealedPostgresMigrationPlanV1();
    const receipt = postgresMigrationPlanPreflightReceiptV1(plan);
    const catalogue = copyPostgresMigrationCommandCatalogueV1(plan);

    expect(Reflect.ownKeys(catalogue)).toEqual(ROOT_KEYS);
    expect(catalogue).toMatchObject({
      catalogueKind: 'postgresql-migration-command-catalogue-v1',
      authority: 'none',
      readinessAuthorized: false,
      databaseAccessAuthorized: false,
      migrationApplyAuthorized: false,
      executableAuthority: false,
      statementCatalogueComplete: false,
      resultContractsSealed: false,
      sourcePlanReceiptSha256:
        '2ff788e1d5af841ea6be0f1d22635a0a584e176d2c17537b9c0533afeebd434d',
    });
    expect(catalogue.sourcePlanReceiptSha256).toBe(receipt.receiptSha256);
    expect(Reflect.ownKeys(catalogue.sourcePins)).toEqual([
      'authoritySeed', 'catalogueContract', 'migration0001', 'migration0002',
    ]);
    expect(catalogue.sourcePins).toEqual({
      authoritySeed: {
        bytes: 11_303,
        sha256: 'b6d3b0f77a2b71cb10782152cb74a3f37320e220131a2d3ef29cc455c9a26e4c',
      },
      catalogueContract: {
        bytes: 232_822,
        sha256: 'e7ce3572463587f4beed55c35c5a6b93810a270136cb963cf312b580fd1ace69',
      },
      migration0001: {
        bytes: 26_438,
        sha256: 'c923f0f725c009a65ef85bc1881b7ae5717a1eca148bbf5316aeee60bb4a31c1',
      },
      migration0002: {
        bytes: 21_661,
        sha256: '1d620d95f630997785d0d3adf724e5befe458c0c41e0746f45713cb584b58765',
      },
    });
    for (const key of Reflect.ownKeys(catalogue.sourcePins)) {
      expect(catalogue.sourcePins[key as keyof typeof catalogue.sourcePins])
        .not.toBe(receipt.artifacts[key as keyof typeof catalogue.sourcePins]);
    }
  });

  it('reuses all lifecycle control SQL bytes in lifecycle order', () => {
    const plan = loadSealedPostgresMigrationPlanV1();
    const lifecycle = copyPostgresMigrationLifecycleContractV1();
    const first = copyPostgresMigrationCommandCatalogueV1(plan);
    const second = copyPostgresMigrationCommandCatalogueV1(plan);

    expect(first.controlCommands.map(({ operation }) => operation)).toEqual(CONTROL_OPERATIONS);
    first.controlCommands.forEach((command, index) => {
      const operation = CONTROL_OPERATIONS[index]!;
      expect(Reflect.ownKeys(command)).toEqual([
        'descriptorKind', 'operation', 'text', 'parameters',
      ]);
      expect(command.descriptorKind)
        .toBe('postgresql-migration-command-descriptor-v1');
      expect(command.text).toBe(lifecycle.controlSql[operation]);
      expect(command.parameters).toEqual([]);
      expect(command).not.toBe(second.controlCommands[index]);
      expect(command.parameters).not.toBe(second.controlCommands[index]!.parameters);
    });
  });

  it('pins exact INSERT bytes, tables, columns, casts, and parameter provenance', () => {
    const { insertCommands } = copyPostgresMigrationCommandCatalogueV1(
      loadSealedPostgresMigrationPlanV1(),
    );
    expect(insertCommands.map(({ operation }) => operation)).toEqual(INSERT_OPERATIONS);
    insertCommands.forEach((command, commandIndex) => {
      const [bytes, digest] = SQL_PINS[commandIndex]!;
      expect(Buffer.byteLength(command.text, 'utf8')).toBe(bytes);
      expect(sha256(command.text)).toBe(digest);
      expect(command.text.startsWith(`INSERT INTO ${TABLES[commandIndex]} (\n`)).toBe(true);
      expect(command.text.endsWith('\n')).toBe(true);
      expect(command.text).not.toMatch(/\r|;|--|\/\*|\bRETURNING\b|\bON\s+CONFLICT\b/i);
      const columnBlock = command.text.match(/\(\n([\s\S]+?)\n\) VALUES \(/)?.[1];
      expect(columnBlock?.split(',\n').map((column) => column.trim()))
        .toEqual(COLUMNS[commandIndex]);
      const casts = [...command.text.matchAll(/\$(\d+)::pg_catalog\.(\w+)/g)]
        .map((match) => [Number(match[1]), match[2]]);
      const expected = PARAMETER_VECTORS[commandIndex]!;
      expect(casts).toEqual(expected.map(([baseType], index) => [index + 1, baseType]));
      expect(command.parameters.map(({ position }) => position))
        .toEqual(expected.map((_, index) => index + 1));
      expect(command.parameters.map(({ baseType, source, representation }) =>
        [baseType, source, representation])).toEqual(expected);
    });
    expect(insertCommands[2]!.text).toBe(insertCommands[3]!.text);
    expect(insertCommands[2]!.parameters[0]!.source)
      .not.toBe(insertCommands[3]!.parameters[0]!.source);
    expect(insertCommands[2]!.parameters[1]!.source)
      .not.toBe(insertCommands[3]!.parameters[1]!.source);
  });

  it('returns fresh, deeply frozen intrinsic data without values or capabilities', () => {
    const plan = loadSealedPostgresMigrationPlanV1();
    const first = copyPostgresMigrationCommandCatalogueV1(plan);
    const second = copyPostgresMigrationCommandCatalogueV1(plan);

    expect(first).not.toBe(second);
    expect(first.sourcePins).not.toBe(second.sourcePins);
    for (const key of Reflect.ownKeys(first.sourcePins)) {
      expect(first.sourcePins[key as keyof typeof first.sourcePins])
        .not.toBe(second.sourcePins[key as keyof typeof second.sourcePins]);
    }
    expect(first.controlCommands).not.toBe(second.controlCommands);
    expect(first.insertCommands).not.toBe(second.insertCommands);
    first.insertCommands.forEach((command, index) => {
      expect(command).not.toBe(second.insertCommands[index]);
      expect(command.parameters).not.toBe(second.insertCommands[index]!.parameters);
      command.parameters.forEach((parameter, parameterIndex) => {
        expect(parameter).not.toBe(second.insertCommands[index]!.parameters[parameterIndex]);
      });
    });
    inspectGraph(first);
    inspectGraph(second);
    const clone = structuredClone(first) as any;
    delete clone.insertCommands[0].parameters[0];
    expect(copyPostgresMigrationCommandCatalogueV1(plan)).toEqual(first);
  });

  it('copies through captured intrinsics after host globals are poisoned', () => {
    const plan = loadSealedPostgresMigrationPlanV1();
    const iterator = Object.getOwnPropertyDescriptor(Array.prototype, Symbol.iterator)!;
    const freeze = Object.freeze;
    const create = Object.create;
    const defineProperty = Object.defineProperty;
    let copied: ReturnType<typeof copyPostgresMigrationCommandCatalogueV1> | undefined;
    try {
      defineProperty(Array.prototype, Symbol.iterator, {
        ...iterator,
        value: () => { throw new Error('poisoned iterator'); },
      });
      Object.freeze = () => { throw new Error('poisoned freeze'); };
      Object.create = () => { throw new Error('poisoned create'); };
      Object.defineProperty = () => { throw new Error('poisoned defineProperty'); };
      copied = copyPostgresMigrationCommandCatalogueV1(plan);
    } finally {
      Object.freeze = freeze;
      Object.create = create;
      Object.defineProperty = defineProperty;
      defineProperty(Array.prototype, Symbol.iterator, iterator);
    }
    expect(copied).toMatchObject({ authority: 'none', executableAuthority: false });
    expect(copied?.insertCommands.map(({ operation }) => operation))
      .toEqual(INSERT_OPERATIONS);
    inspectGraph(copied);
  });

  it('rejects clones, proxies, bytes, and unbranded lookalikes', () => {
    const plan = loadSealedPostgresMigrationPlanV1();
    const lookalike = structuredClone(plan);
    for (const candidate of [
      lookalike,
      new Proxy(plan, { get: () => { throw new Error('trap'); } }),
      new Uint8Array([1, 2, 3]),
      null,
      { ...lookalike, preflightReceiptSha256: plan.preflightReceiptSha256 },
    ]) {
      expect(() => copyPostgresMigrationCommandCatalogueV1(candidate))
        .toThrow('PostgreSQL migration Plan is invalid');
    }
  });

  it('owns no filesystem loader and remains absent from the public bundle', async () => {
    const source = readFileSync(SOURCE, 'utf8');
    expect(source).not.toMatch(/node:fs|readFile|loadSealedPostgresMigrationPlanV1/);
    expect(source.match(/^import[\s\S]*?;$/gm)).toHaveLength(2);
    const publicModule = await import('../src/index.js');
    expect(publicModule).not.toHaveProperty('copyPostgresMigrationCommandCatalogueV1');
  });
});
