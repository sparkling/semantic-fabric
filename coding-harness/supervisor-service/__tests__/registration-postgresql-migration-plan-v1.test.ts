// SPDX-License-Identifier: MIT

import {
  chmodSync, cpSync, mkdirSync, mkdtempSync, renameSync, rmSync,
} from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { build } from 'esbuild';
import { afterEach, describe, expect, it } from 'vitest';

const SERVICE_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const PLAN_SOURCE = resolve(
  SERVICE_ROOT, 'src/registration-postgresql-migration-plan-v1.ts',
);
const TEMP_ROOTS: string[] = [];

interface PlanModuleV1 {
  readonly loadSealedPostgresMigrationPlanV1: () => Record<string, unknown>;
  readonly assertSealedPostgresMigrationPlanV1: (value: unknown) => void;
  readonly postgresMigrationPlanAuthoritiesV1: (value: unknown) => Record<string, unknown>;
  readonly copySealedPostgresMigrationPlanStepsV1: (
    value: unknown,
  ) => readonly Readonly<{ stepKind: string; migrationVersion?: number; bytes: Uint8Array }>[];
  readonly postgresMigrationPlanPreflightReceiptV1: (
    value: unknown,
  ) => Record<string, unknown>;
  readonly parsePostgresMigrationPlanPreflightReceiptV1: (
    value: unknown,
  ) => Record<string, unknown>;
  readonly replayPostgresMigrationPlanPreflightReceiptV1: (
    value: unknown,
  ) => Record<string, unknown>;
}

afterEach(() => {
  for (const path of TEMP_ROOTS.splice(0)) rmSync(path, { recursive: true, force: true });
});

describe('sealed PostgreSQL migration Plan V1', () => {
  it('imports without I/O, then loads only from the module-fixed service root', async () => {
    const fixture = await bundledPlanFixture(false);
    expect(() => fixture.module.loadSealedPostgresMigrationPlanV1())
      .toThrow('PostgreSQL migration Plan is invalid');

    installMigrations(fixture.root);
    const plan = fixture.module.loadSealedPostgresMigrationPlanV1();
    expect(plan).toEqual({
      planKind: 'sealed-postgresql-migration-plan-v1',
      authority: 'none',
      readinessAuthorized: false,
      databaseAccessAuthorized: false,
      migrationApplyAuthorized: false,
      preflightReceiptSha256:
        '2ff788e1d5af841ea6be0f1d22635a0a584e176d2c17537b9c0533afeebd434d',
    });
    expect(Object.isFrozen(plan)).toBe(true);
    expect(Reflect.ownKeys(plan)).toEqual(Object.keys(plan));
  });

  it('brands the Plan without traps and rejects clones, proxies, and semantic handles', async () => {
    const { module } = await bundledPlanFixture(true);
    const plan = module.loadSealedPostgresMigrationPlanV1();
    const authorities = module.postgresMigrationPlanAuthoritiesV1(plan);
    const proxy = new Proxy(plan, { get: () => { throw new Error('trap'); } });

    expect(() => module.assertSealedPostgresMigrationPlanV1(plan)).not.toThrow();
    expect(() => module.assertSealedPostgresMigrationPlanV1({ ...plan })).toThrow();
    expect(() => module.assertSealedPostgresMigrationPlanV1(proxy))
      .toThrow('PostgreSQL migration Plan is invalid');
    for (const authority of Object.values(authorities)) {
      expect(() => module.assertSealedPostgresMigrationPlanV1(authority)).toThrow();
      expect(Object.isFrozen(authority)).toBe(true);
    }
    expect(Object.isFrozen(authorities)).toBe(true);
  });

  it('projects the exact three-step order through fresh mutable byte copies', async () => {
    const { module } = await bundledPlanFixture(true);
    const plan = module.loadSealedPostgresMigrationPlanV1();
    const first = module.copySealedPostgresMigrationPlanStepsV1(plan);
    const second = module.copySealedPostgresMigrationPlanStepsV1(plan);

    expect(first.map(({ stepKind, migrationVersion, bytes }) => ({
      stepKind, migrationVersion, bytes: bytes.byteLength,
    }))).toEqual([
      { stepKind: 'migration-sql', migrationVersion: 1, bytes: 26_438 },
      { stepKind: 'authority-seed', migrationVersion: undefined, bytes: 11_303 },
      { stepKind: 'migration-sql', migrationVersion: 2, bytes: 21_661 },
    ]);
    expect(Object.isFrozen(first)).toBe(true);
    expect(first.every(Object.isFrozen)).toBe(true);
    expect(first[0]!.bytes).not.toBe(second[0]!.bytes);
    first[0]!.bytes.fill(0);
    expect(second[0]!.bytes.some((byte) => byte !== 0)).toBe(true);
  });

  it('emits a replayable pathless receipt without rereading files', async () => {
    const fixture = await bundledPlanFixture(true);
    const plan = fixture.module.loadSealedPostgresMigrationPlanV1();
    const receipt = fixture.module.postgresMigrationPlanPreflightReceiptV1(plan);
    const encoded = JSON.stringify(receipt);
    const authorities = fixture.module.postgresMigrationPlanAuthoritiesV1(plan) as Record<
      string, { rawByteLength: number; rawSha256: string }
    >;
    const artifacts = receipt.artifacts as Record<string, { bytes: number; sha256: string }>;

    expect(receipt).toMatchObject({
      schemaVersion: 1,
      receiptKind: 'postgresql-migration-plan-preflight-v1',
      planKind: 'sealed-postgresql-migration-plan-v1',
      authority: 'none',
      readinessAuthorized: false,
      databaseAccessAuthorized: false,
      migrationApplyAuthorized: false,
      postgresqlServerVersion: '16.15',
      postgresqlServerVersionNumber: 160_015,
      advisoryLockKey: '800874507948546278',
      receiptSha256: plan.preflightReceiptSha256,
    });
    expect(Object.isFrozen(receipt)).toBe(true);
    for (const [key, pin] of Object.entries(artifacts)) {
      expect(authorities[key]).toMatchObject({
        rawByteLength: pin.bytes,
        rawSha256: pin.sha256,
      });
    }
    expect(encoded).not.toContain(fixture.root);
    expect(encoded).not.toMatch(/migrations\/|\.sql|credential|password|timestamp|environment/i);

    const migrations = resolve(fixture.root, 'migrations');
    renameSync(migrations, `${migrations}-hidden`);
    expect(fixture.module.postgresMigrationPlanPreflightReceiptV1(plan)).toBe(receipt);
    expect(() => fixture.module.replayPostgresMigrationPlanPreflightReceiptV1(receipt))
      .toThrow('PostgreSQL migration Plan is invalid');
    renameSync(`${migrations}-hidden`, migrations);
    expect(fixture.module.replayPostgresMigrationPlanPreflightReceiptV1(receipt))
      .toMatchObject({ preflightReceiptSha256: plan.preflightReceiptSha256 });
  });

  it('parses exact cloned receipts without candidate serialization', async () => {
    const { module } = await bundledPlanFixture(true);
    const plan = module.loadSealedPostgresMigrationPlanV1();
    const receipt = module.postgresMigrationPlanPreflightReceiptV1(plan);
    const clone = structuredClone(receipt);
    const frozenClone = freezeGraph(structuredClone(receipt));

    expect(module.parsePostgresMigrationPlanPreflightReceiptV1(receipt)).toBe(receipt);
    expect(module.parsePostgresMigrationPlanPreflightReceiptV1(clone)).toBe(receipt);
    expect(module.parsePostgresMigrationPlanPreflightReceiptV1(frozenClone)).toBe(receipt);

    const stringify = JSON.stringify;
    const ownKeys = Reflect.ownKeys;
    const getPrototypeOf = Object.getPrototypeOf;
    const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
    let replacementCalls = 0;
    const failSerialization = (): never => {
      replacementCalls += 1;
      throw new Error('candidate serialization');
    };
    let parsed: Record<string, unknown> | undefined;
    try {
      JSON.stringify = failSerialization;
      Reflect.ownKeys = failSerialization;
      Object.getPrototypeOf = failSerialization;
      Object.getOwnPropertyDescriptor = failSerialization;
      parsed = module.parsePostgresMigrationPlanPreflightReceiptV1(clone);
    } finally {
      JSON.stringify = stringify;
      Reflect.ownKeys = ownKeys;
      Object.getPrototypeOf = getPrototypeOf;
      Object.getOwnPropertyDescriptor = getOwnPropertyDescriptor;
    }
    expect(parsed).toBe(receipt);
    expect(replacementCalls).toBe(0);
  });

  it('rejects reordered records and bounded primitive substitutions', async () => {
    const { module } = await bundledPlanFixture(true);
    const receipt = module.postgresMigrationPlanPreflightReceiptV1(
      module.loadSealedPostgresMigrationPlanV1(),
    );
    const clone = structuredClone(receipt);
    const artifacts = clone.artifacts as Record<string, unknown>;
    const reorderedArtifacts = structuredClone(clone);
    reorderedArtifacts.artifacts = reverseRecord(
      reorderedArtifacts.artifacts as Record<string, unknown>,
    );
    const reorderedPin = structuredClone(clone);
    const reorderedPins = reorderedPin.artifacts as Record<string, unknown>;
    reorderedPins.manifest = reverseRecord(
      reorderedPins.manifest as Record<string, unknown>,
    );
    const pinArray = structuredClone(clone);
    (pinArray.artifacts as Record<string, unknown>).manifest = [
      1_299,
      '72782ecae7d33a0149fb2ceb0a3219254fcd68137e499b811260f77b5cd70478',
    ];

    for (const hostile of [
      reverseRecord(clone),
      reorderedArtifacts,
      reorderedPin,
      replaceOwnKey(clone, 'receiptSha256', 'x'.repeat(30)),
      { ...clone, authority: 'migration' },
      { ...clone, authority: 'a'.repeat(65) },
      { ...clone, authority: 'é'.repeat(33) },
      { ...clone, schemaVersion: Number.MAX_SAFE_INTEGER + 1 },
      { ...clone, extra: true },
      { ...clone, receiptSha256: '0'.repeat(64) },
      { ...clone, artifacts: Object.values(artifacts) },
      { ...clone, artifacts: new Uint8Array(0) },
      pinArray,
    ]) {
      expect(() => module.parsePostgresMigrationPlanPreflightReceiptV1(hostile))
        .toThrow('PostgreSQL migration preflight receipt is invalid');
    }
  });

  it('rejects hostile identities and descriptors without invoking traps or getters', async () => {
    const { module } = await bundledPlanFixture(true);
    const receipt = module.postgresMigrationPlanPreflightReceiptV1(
      module.loadSealedPostgresMigrationPlanV1(),
    );
    const clone = structuredClone(receipt);
    let getterCalls = 0;
    let trapCalls = 0;
    const accessor = structuredClone(clone);
    Object.defineProperty(accessor, 'authority', {
      enumerable: true,
      configurable: true,
      get: () => {
        getterCalls += 1;
        throw new Error('getter');
      },
    });
    const nestedAccessor = structuredClone(clone);
    const nestedManifest = (nestedAccessor.artifacts as Record<string, unknown>)
      .manifest as Record<string, unknown>;
    Object.defineProperty(nestedManifest, 'bytes', {
      enumerable: true,
      configurable: true,
      get: () => {
        getterCalls += 1;
        throw new Error('nested getter');
      },
    });
    const nonEnumerable = structuredClone(clone);
    Object.defineProperty(nonEnumerable, 'authority', {
      value: 'none', enumerable: false, configurable: true, writable: true,
    });
    const symbol = structuredClone(clone);
    Object.defineProperty(symbol, Symbol('extra'), { value: true, enumerable: true });
    const traps = {
      get: () => { trapCalls += 1; throw new Error('get trap'); },
      getOwnPropertyDescriptor: () => {
        trapCalls += 1;
        throw new Error('descriptor trap');
      },
      getPrototypeOf: () => { trapCalls += 1; throw new Error('prototype trap'); },
      ownKeys: () => { trapCalls += 1; throw new Error('ownKeys trap'); },
    };
    const rootProxy = new Proxy(clone, traps);
    const nestedProxy = structuredClone(clone);
    nestedProxy.artifacts = new Proxy(
      nestedProxy.artifacts as Record<string, unknown>, traps,
    );
    const revoked = Proxy.revocable(clone, traps);
    revoked.revoke();

    for (const hostile of [
      accessor,
      nestedAccessor,
      nonEnumerable,
      symbol,
      rootProxy,
      nestedProxy,
      revoked.proxy,
      Object.assign(Object.create(null), clone),
      Object.assign(Object.create({ inherited: true }), clone),
      Object.assign([], clone),
      Object.assign(new String('receipt'), clone),
      Object.assign(() => undefined, clone),
    ]) {
      expect(() => module.parsePostgresMigrationPlanPreflightReceiptV1(hostile))
        .toThrow('PostgreSQL migration preflight receipt is invalid');
    }
    expect(getterCalls).toBe(0);
    expect(trapCalls).toBe(0);
  });
});

function freezeGraph(value: unknown): unknown {
  if (value !== null && typeof value === 'object') {
    for (const key of Reflect.ownKeys(value)) {
      const descriptor = Object.getOwnPropertyDescriptor(value, key);
      if (descriptor && 'value' in descriptor) freezeGraph(descriptor.value);
    }
    Object.freeze(value);
  }
  return value;
}

function reverseRecord(value: Record<string, unknown>): Record<string, unknown> {
  return Object.fromEntries(Object.entries(value).reverse());
}

function replaceOwnKey(
  value: Record<string, unknown>,
  current: string,
  replacement: string,
): Record<string, unknown> {
  return Object.fromEntries(Object.entries(value).map(([key, item]) => [
    key === current ? replacement : key,
    item,
  ]));
}

async function bundledPlanFixture(withMigrations: boolean): Promise<{
  root: string;
  module: PlanModuleV1;
}> {
  const root = mkdtempSync(resolve(SERVICE_ROOT, '__tests__/sf-pg-plan-'));
  TEMP_ROOTS.push(root);
  mkdirSync(resolve(root, 'src'), { mode: 0o755 });
  chmodSync(root, 0o755);
  const outfile = resolve(root, 'src/plan.mjs');
  await build({
    entryPoints: [PLAN_SOURCE], outfile, bundle: true, platform: 'node',
    format: 'esm', target: 'node20', treeShaking: true, logLevel: 'silent',
  });
  if (withMigrations) installMigrations(root);
  const imported = await import(pathToFileURL(outfile).href);
  return { root, module: imported as PlanModuleV1 };
}

function installMigrations(root: string): void {
  const target = resolve(root, 'migrations');
  cpSync(resolve(SERVICE_ROOT, 'migrations'), target, { recursive: true });
  chmodSync(target, 0o755);
  for (const name of [
    'manifest-v1.json', 'catalog-contract-v1.json', 'provisioning-contract-v1.json',
    '0001-registration-state-v1.sql', '0002-registration-rls-v1.sql',
  ]) chmodSync(resolve(target, name), 0o644);
}
