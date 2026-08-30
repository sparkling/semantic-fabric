// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
// @ts-expect-error The test exercises the private JavaScript oracle directly.
import { canonicalFixture, compareRecordBags, deriveOracleRecords, parseProjectionRecords } from '../scripts/postgresql-public-acl-oracle-v1.mjs';
// @ts-expect-error The test exercises the private JavaScript wire parser directly.
import { bool, hex, nullableHex, nullableUnsigned, oid, parseOracleSession, signed, unsigned } from '../scripts/postgresql-public-acl-oracle-wire-v1.mjs';

const ROOT = resolve(import.meta.dirname, '..');
const SQL_PATH = resolve(
  import.meta.dirname, 'fixtures/postgresql-16.15-public-acl-completeness-oracle-v1.sql',
);
const RECEIPT_PATH = resolve(
  import.meta.dirname, 'fixtures/postgresql-16.15-public-acl-capture-receipt-v1.json',
);
const SCRIPT_PATHS = [
  resolve(ROOT, 'scripts/postgresql-public-acl-oracle-wire-v1.mjs'),
  resolve(ROOT, 'scripts/postgresql-public-acl-oracle-v1.mjs'),
  resolve(ROOT, 'scripts/verify-postgresql-public-acl-oracle-v1.mjs'),
];
const SECTIONS = Object.freeze([
  ['SCHEMA', 11], ['RELATION', 13], ['COLUMN', 12], ['ROUTINE', 25],
  ['TYPE', 15], ['LANGUAGE', 11], ['FDW', 11], ['SERVER', 12],
  ['LARGE_OBJECT', 11], ['CONTROL', 32],
]);

interface PublicAclRecordV1 {
  readonly objectClass: string;
  readonly schemaName: string | null;
  readonly objectName: string;
  readonly subobjectName: string | null;
  readonly objectKind: string;
  readonly routineIdentityArguments: string | null;
  readonly privilege: string;
  readonly grantable: boolean;
}

describe('PostgreSQL 16.15 independent PUBLIC ACL completeness oracle', () => {
  it('keeps the raw SQL independent from the production projection and result pins', () => {
    const sql = readFileSync(SQL_PATH, 'utf8');
    expect(sql).not.toMatch(/json(?:b)?_build_object/iu);
    expect(sql).not.toMatch(/acl(?:default)/iu);
    expect(sql).not.toMatch(/coalesce/iu);
    expect(sql).not.toMatch(/grantee\s*=\s*0/iu);
    expect(sql).not.toMatch(/\bunion\b/iu);
    expect(sql).not.toContain('pg_get_function_identity_arguments');
    expect(sql).not.toContain('postgresql-16.15-clean-template0-public-object-acl-v1.json');
    expect(sql).not.toContain('a108e05f9cfd6d6485a86fe198a87b3800e21986b5c62e6251519de6577d05be');
    expect(sql).not.toMatch(/\b4059\b/u);
    for (const path of SCRIPT_PATHS) {
      const source = readFileSync(path, 'utf8');
      expect(source).not.toContain('a108e05f9cfd6d6485a86fe198a87b3800e21986b5c62e6251519de6577d05be');
      expect(source).not.toContain('postgresql-16.15-clean-template0-public-object-acl-v1.json');
    }
    expect((sql.match(/@@ADR0047-RAW-V1\/[A-Z_]+\/BEGIN@@/gu) ?? [])).toHaveLength(10);
    expect((sql.match(/@@ADR0047-RAW-V1\/[A-Z_]+\/END@@/gu) ?? [])).toHaveLength(10);
    expect(sql.endsWith('\nROLLBACK;\n')).toBe(true);
  });

  it('keeps each oracle component below the repository line ceiling', () => {
    for (const path of [SQL_PATH, ...SCRIPT_PATHS]) {
      const source = readFileSync(path, 'utf8');
      expect(source.split('\n').length - Number(source.endsWith('\n'))).toBeLessThan(500);
    }
  });

  it('binds two distinct-volume replays to every source and the exact fixture', () => {
    const receipt = objectValue(JSON.parse(readFileSync(RECEIPT_PATH, 'utf8')));
    expectExactKeys(receipt, ['schemaVersion', 'authority', 'captureDate', 'image',
      'database', 'sources', 'profile', 'result', 'runs', 'replay']);
    expect([receipt.schemaVersion, receipt.authority, receipt.captureDate]).toEqual([
      'semantic-fabric.postgresql-public-acl-capture-receipt/v1',
      'test-only-non-runtime', '2026-08-31',
    ]);
    expect(receipt.image).toEqual({
      reference: 'postgres@sha256:485935f94cc7165afa896978809c37b592dc07f0a37d2c8f645f12412d0212c8',
      manifestMediaType: 'application/vnd.oci.image.manifest.v1+json',
      platformManifestDigest: 'sha256:485935f94cc7165afa896978809c37b592dc07f0a37d2c8f645f12412d0212c8',
      configurationDigest: 'sha256:80f4c7a5e91618546dce5b4fe60cf03b14c0f9efa7e40157278d122772ced8d2',
      platform: 'linux/amd64', serverVersion: '16.15 (Debian 16.15-1.pgdg13+2)',
    });
    expect(receipt.database).toEqual({
      name: 'sf_public_baseline', owner: 'postgres', template: 'template0', encoding: 'UTF8',
      localeProvider: 'c', collation: 'C', ctype: 'C',
      initdbArguments: '--locale=C --encoding=UTF8',
    });
    const expectedSources: Readonly<Record<string, string>> = {
      projection: '__tests__/fixtures/postgresql-16.15-public-acl-projection-v1.sql',
      rawOracle: '__tests__/fixtures/postgresql-16.15-public-acl-completeness-oracle-v1.sql',
      captureRunner: 'scripts/capture-postgresql-public-acl-baseline-v1.mjs',
      oracleWire: 'scripts/postgresql-public-acl-oracle-wire-v1.mjs',
      oracleDeriver: 'scripts/postgresql-public-acl-oracle-v1.mjs',
      oracleRunner: 'scripts/verify-postgresql-public-acl-oracle-v1.mjs',
      replayRunner: 'scripts/replay-postgresql-public-acl-baseline-v1.mjs',
    };
    const sources = objectValue(receipt.sources);
    expectExactKeys(sources, Object.keys(expectedSources));
    for (const [key, path] of Object.entries(expectedSources)) {
      const source = objectValue(sources[key]);
      expectExactKeys(source, ['path', 'bytes', 'sha256']);
      expect(source.path).toBe(path);
      expect(path.startsWith('/') || path.includes('\\') || path.split('/').includes('..')).toBe(false);
      const bytes = readFileSync(resolve(ROOT, path));
      expect(bytes.byteLength).toBe(source.bytes);
      expect(sha256(bytes)).toBe(source.sha256);
      expectDigest(source.sha256);
    }
    expect(receipt.profile).toEqual({
      sha256: '15d6ff996e0cf5cec2fd269898c6ec470f35d2b8e25da6f2535daa95324f92c7',
      defaultAclRows: 0, parameterAclRows: 0, foreignDataWrappers: 0, foreignServers: 0,
      userMappings: 0, largeObjects: 0, dedicatedSchemaRows: 0, publicDependentObjects: 0,
    });
    const result = objectValue(receipt.result);
    expect(result).toEqual({
      fixturePath: '__tests__/fixtures/postgresql-16.15-clean-template0-public-object-acl-v1.json',
      records: 4_059, nodes: 36_532, bytes: 860_988,
      sha256: 'a108e05f9cfd6d6485a86fe198a87b3800e21986b5c62e6251519de6577d05be',
    });
    const fixture = readFileSync(resolve(ROOT, result.fixturePath as string));
    expect(fixture.byteLength).toBe(result.bytes);
    expect(sha256(fixture)).toBe(result.sha256);
    expect(JSON.parse(fixture.toString('utf8'))).toHaveLength(4_059);
    const runs = receipt.runs as Array<Record<string, unknown>>;
    expect(runs).toHaveLength(2);
    for (const [index, run] of runs.entries()) {
      expectExactKeys(run, ['sequence', 'networkMode', 'publishedPorts',
        'dataVolumeNameSha256', 'profileSha256', 'rawTranscriptBytes', 'rawTranscriptSha256',
        'sessionTranscriptBytes', 'sessionTranscriptSha256',
        'recordCount', 'recordsBytes', 'recordsSha256']);
      expect([run.sequence, run.networkMode, run.publishedPorts]).toEqual([index + 1, 'none', false]);
      ['dataVolumeNameSha256', 'profileSha256', 'rawTranscriptSha256',
        'sessionTranscriptSha256', 'recordsSha256']
        .forEach((key) => expectDigest(run[key]));
      expect([run.profileSha256, run.recordCount, run.recordsBytes, run.recordsSha256]).toEqual([
        (receipt.profile as Record<string, unknown>).sha256,
        result.records, result.bytes, result.sha256,
      ]);
      expect(Number.isSafeInteger(run.rawTranscriptBytes)).toBe(true);
      expect(Number.isSafeInteger(run.sessionTranscriptBytes)).toBe(true);
    }
    expect(new Set(runs.map((run) => run.dataVolumeNameSha256)).size).toBe(2);
    for (const key of ['rawTranscriptBytes', 'rawTranscriptSha256',
      'sessionTranscriptBytes', 'sessionTranscriptSha256']) {
      expect(runs[0]?.[key]).toBe(runs[1]?.[key]);
    }
    expect(receipt.replay).toEqual({
      minimumRuns: 2, requiresDistinctAnonymousDataVolumes: true,
      requiresNoPublishedPorts: true,
      runnerArgv: ['node', 'scripts/replay-postgresql-public-acl-baseline-v1.mjs'],
      captureArgv: ['node', 'scripts/capture-postgresql-public-acl-baseline-v1.mjs', 'CONTAINER_NAME'],
      oracleArgv: ['node', 'scripts/verify-postgresql-public-acl-oracle-v1.mjs', 'CONTAINER_NAME'],
    });
  });

  it('requires the exact section order, field counts, projection envelope and final LF', () => {
    const valid = transcript();
    expect(parseOracleSession(Buffer.from(valid)).projection).toHaveLength(1);
    const mutants = [
      valid.replace('@@ADR0047-RAW-V1/SCHEMA/BEGIN@@\n', ''),
      valid.replace('@@ADR0047-RAW-V1/TYPE/END@@', '@@ADR0047-RAW-V1/FDW/END@@'),
      valid.replace(controlRow(), controlRow().split('\t').slice(1).join('\t')),
      valid.replace('@@ADR0047-PROJECTION/BEGIN@@', '@@ADR0047-PROJECTION/END@@'),
      valid.slice(0, -1),
      valid.replace('\n', '\r\n'),
    ];
    mutants.forEach((mutant) => expect(() => parseOracleSession(Buffer.from(mutant))).toThrow());
  });

  it('rejects noncanonical scalar cells and invalid UTF-8 text', () => {
    for (const value of ['', '00', '+1', '01', '-1', '1.0']) {
      expect(() => unsigned(value)).toThrow();
    }
    expect(unsigned('0')).toBe(0);
    expect(signed('-1')).toBe(-1);
    expect(() => signed('-0')).toThrow();
    expect(() => oid('0')).toThrow();
    expect(oid('0', true)).toBe(0);
    expect(bool('t')).toBe(true);
    expect(() => bool('true')).toThrow();
    expect(nullableUnsigned(String.raw`\N`)).toBeNull();
    expect(nullableHex(String.raw`\N`)).toBeNull();
    expect(hex('616263')).toBe('abc');
    for (const value of ['A0', '0', 'zz', 'eda080']) expect(() => hex(value)).toThrow();
  });

  it('rejects malformed, NUL, lone-surrogate and out-of-order production records', () => {
    const first = record({ objectName: 'a' });
    const second = record({ objectName: 'b' });
    expect(parseProjectionRecords([JSON.stringify(first), JSON.stringify(second)]))
      .toEqual([first, second]);
    expect(() => parseProjectionRecords([JSON.stringify(second), JSON.stringify(first)]))
      .toThrow('ORACLE_PROJECTION_ORDER_INVALID');
    expect(() => parseProjectionRecords([JSON.stringify(record({ objectName: 'a\0b' }))]))
      .toThrow('ORACLE_STRING_INVALID');
    expect(() => parseProjectionRecords(['{"objectClass":"schema","schemaName":null,'
      + '"objectName":"\\ud800","subobjectName":null,"objectKind":"schema",'
      + '"routineIdentityArguments":null,"privilege":"USAGE","grantable":false}']))
      .toThrow('ORACLE_STRING_INVALID');
    expect(() => parseProjectionRecords(['{}'])).toThrow('ORACLE_RECORD_SHAPE_INVALID');
  });

  it('compares multiplicity in both directions and canonicalizes only after validation', () => {
    const left = record({ objectName: 'left' });
    const right = record({ objectName: 'right' });
    expect(() => compareRecordBags([left, right], [left, right])).not.toThrow();
    expect(() => compareRecordBags([left, left], [left])).toThrow();
    expect(() => compareRecordBags([left], [right])).toThrow();
    const fixture = canonicalFixture([left]);
    expect(fixture.equals(Buffer.from(`${JSON.stringify([left])}\n`))).toBe(true);
    expect(sha256(fixture)).toMatch(/^[0-9a-f]{64}$/u);
  });

  it('projects supported stock-zero kinds from synthetic raw catalogue facts', () => {
    const records = deriveOracleRecords(
      syntheticRaw(), { enforceCleanProfile: false },
    ) as PublicAclRecordV1[];
    expect(new Set(records.map((value) => value.objectKind))).toEqual(new Set([
      'table', 'partitioned-table', 'view', 'materialized-view', 'foreign-table',
      'sequence', 'function', 'procedure', 'aggregate', 'window-function',
      'base', 'enum', 'array', 'pseudo', 'foreign-data-wrapper', 'foreign-server', 'language',
    ]));
    expect(records.some((value) => value.objectClass === 'column')).toBe(true);
    expect(records.some((value) => value.objectName === 'unsupported_index')).toBe(false);
    expect(records.find((value) => value.objectName === '_scalar')?.objectKind).toBe('base');
    expect(records.find((value) => value.objectName === '_element')?.objectKind).toBe('array');
    expect(records.find((value) => value.objectName === 'procedure_value')
      ?.routineIdentityArguments).toBe('IN internal');
    expect(records.find((value) => value.objectName === 'table_function_value')
      ?.routineIdentityArguments).toBe('');
  });

  it('fails closed on unsupported, unresolved, invalid-column and global ACL mutants', () => {
    const mutants: ReadonlyArray<readonly [(raw: Record<string, string[][]>) => void, string]> = [
      [(raw) => { raw.RELATION![0]![3] = encode('z'); }, 'ORACLE_RELATION_KIND_UNKNOWN'],
      [(raw) => { replaceAcl(raw.RELATION![6]!, 5, publicAcl('SELECT')); },
        'ORACLE_UNSUPPORTED_RELATION_ACL_INVALID'],
      [(raw) => { raw.COLUMN![0]![1] = '0'; }, 'ORACLE_INVALID_COLUMN_ACL_INVALID'],
      [(raw) => { raw.COLUMN![0]![3] = 't'; }, 'ORACLE_INVALID_COLUMN_ACL_INVALID'],
      [(raw) => { raw.TYPE![1]![6] = '999'; }, 'ORACLE_OID_REFERENCE_INVALID'],
      [(raw) => { replaceAcl(raw.TYPE![1]!, 7, publicAcl('USAGE')); },
        'ORACLE_ARRAY_OWN_ACL_INVALID'],
      [(raw) => { raw.SERVER![0]![3] = '999'; }, 'ORACLE_OID_REFERENCE_INVALID'],
      [(raw) => { raw.RELATION![0]![12] = 't'; }, 'ORACLE_PUBLIC_GRANTABLE_INVALID'],
      [(raw) => { raw.LARGE_OBJECT!.push(['900', nil(), '10', ...publicAcl('SELECT')]); },
        'ORACLE_LARGE_OBJECT_PUBLIC_ACL_INVALID'],
    ];
    for (const [mutate, code] of mutants) {
      const raw = syntheticRaw();
      mutate(raw);
      raw.CONTROL = [controlFor(raw)];
      expect(() => deriveOracleRecords(raw, { enforceCleanProfile: false })).toThrow(code);
    }
  });

  it('distinguishes raw-null, explicit-empty, element authority and grantor collisions', () => {
    const defaults = deriveOracleRecords(
      syntheticRaw(), { enforceCleanProfile: false },
    ) as PublicAclRecordV1[];
    const emptyRoutine = syntheticRaw();
    replaceAcl(emptyRoutine.ROUTINE![2]!, 17, emptyAcl());
    emptyRoutine.CONTROL = [controlFor(emptyRoutine)];
    const withoutRoutine = deriveOracleRecords(
      emptyRoutine, { enforceCleanProfile: false },
    ) as PublicAclRecordV1[];
    expect(defaults.some((value) => value.objectName === 'function_value')).toBe(true);
    expect(withoutRoutine.some((value) => value.objectName === 'function_value')).toBe(false);

    const emptyElement = syntheticRaw();
    replaceAcl(emptyElement.TYPE![0]!, 7, emptyAcl());
    emptyElement.CONTROL = [controlFor(emptyElement)];
    const withoutElement = deriveOracleRecords(
      emptyElement, { enforceCleanProfile: false },
    ) as PublicAclRecordV1[];
    expect(withoutElement.some((value) => ['element', '_element'].includes(value.objectName)))
      .toBe(false);

    const collision = syntheticRaw();
    const first = collision.RELATION![0]!.slice(0, 5);
    collision.RELATION!.splice(0, 1,
      [...first, 'f', '2', '2', '1', '10', '0', encode('SELECT'), 'f'],
      [...first, 'f', '2', '2', '2', '11', '0', encode('SELECT'), 'f']);
    collision.CONTROL = [controlFor(collision)];
    expect(() => deriveOracleRecords(collision, { enforceCleanProfile: false }))
      .toThrow('ORACLE_RECORD_IDENTITY_DUPLICATE');
  });
});

function transcript(): string {
  const lines = [];
  for (const [section] of SECTIONS) {
    lines.push(`@@ADR0047-RAW-V1/${section}/BEGIN@@`);
    if (section === 'CONTROL') lines.push(controlRow());
    lines.push(`@@ADR0047-RAW-V1/${section}/END@@`);
  }
  lines.push('@@ADR0047-PROJECTION/BEGIN@@', JSON.stringify(record()),
    '@@ADR0047-PROJECTION/END@@');
  return `${lines.join('\n')}\n`;
}

function controlRow(): string {
  return new Array(32).fill('0').join('\t');
}

function record(overrides: Partial<PublicAclRecordV1> = {}): PublicAclRecordV1 {
  return {
    objectClass: 'schema', schemaName: null, objectName: 'catalogue',
    subobjectName: null, objectKind: 'schema', routineIdentityArguments: null,
    privilege: 'USAGE', grantable: false, ...overrides,
  };
}

function syntheticRaw(): Record<string, string[][]> {
  const raw: Record<string, string[][]> = {
    SCHEMA: [schemaRow(11, 'pg_catalog', emptyAcl())],
    RELATION: [
      relationRow(20, 'table', 'r', 'SELECT'),
      relationRow(21, 'partitioned', 'p', 'SELECT'),
      relationRow(22, 'view', 'v', 'SELECT'),
      relationRow(23, 'materialized', 'm', 'SELECT'),
      relationRow(24, 'foreign_table', 'f', 'SELECT'),
      relationRow(25, 'sequence', 'S', 'USAGE'),
      relationRow(26, 'unsupported_index', 'i', null),
    ],
    COLUMN: [columnRow(20, 1, 'value', 'SELECT')],
    ROUTINE: [
      routineRow(30, 'array_subscript_handler', 'f', null, [argument(104, 'internal')]),
      routineRow(31, 'raw_array_subscript_handler', 'f', null, [argument(104, 'internal')]),
      routineRow(32, 'function_value', 'f', 'EXECUTE'),
      routineRow(33, 'procedure_value', 'p', 'EXECUTE', [argument(104, 'internal')]),
      routineRow(34, 'aggregate_value', 'a', 'EXECUTE', [], 'n', 0),
      routineRow(35, 'window_value', 'w', 'EXECUTE'),
      routineRow(36, 'table_function_value', 'f', 'EXECUTE', [
        argument(104, 'internal', 't'),
      ]),
    ],
    TYPE: [
      typeRow(100, 'element', 'b', 0, 0, null),
      typeRow(101, '_element', 'b', 100, 30, null),
      typeRow(102, 'enum_value', 'e', 0, 0, 'USAGE'),
      typeRow(103, '_scalar', 'b', 100, 31, 'USAGE'),
      typeRow(104, 'internal', 'p', 0, 0, null),
    ],
    LANGUAGE: [globalRow(50, 'language_value', 'USAGE')],
    FDW: [globalRow(40, 'fdw_value', 'USAGE')],
    SERVER: [serverRow(41, 'server_value', 40, 'USAGE')],
    LARGE_OBJECT: [],
    CONTROL: [],
  };
  raw.CONTROL = [controlFor(raw)];
  return raw;
}

function schemaRow(oidValue: number, name: string, acl: string[]): string[] {
  return [String(oidValue), encode(name), '10', ...acl];
}

function relationRow(
  oidValue: number, name: string, kind: string, privilege: string | null,
): string[] {
  return [String(oidValue), '11', encode(name), encode(kind), '10',
    ...(privilege === null ? nullAcl() : publicAcl(privilege))];
}

function columnRow(
  relationOid: number, number: number, name: string, privilege: string,
): string[] {
  return [String(relationOid), String(number), encode(name), 'f', ...publicAcl(privilege)];
}

interface SyntheticArgument {
  readonly typeOid: number;
  readonly type: string;
  readonly mode: string | null;
}

function argument(typeOid: number, type: string, mode: string | null = null): SyntheticArgument {
  return { typeOid, type, mode };
}

function routineRow(
  oidValue: number,
  name: string,
  kind: string,
  privilege: string | null,
  args: readonly SyntheticArgument[] = [],
  aggregateKind: string | null = null,
  directArgs: number | null = null,
): string[] {
  const arg = args[0];
  return [
    String(oidValue), '11', encode(name), encode(kind), '10', nullable(aggregateKind, encode),
    directArgs === null ? nil() : String(directArgs),
    args.some((value) => value.mode === 't') ? 'f' : 't',
    args.every((value) => value.mode === null) ? 't' : 'f', 't', String(args.length),
    arg === undefined ? nil() : '1',
    arg?.mode === null || arg === undefined ? nil() : encode(arg.mode), nil(), nil(),
    arg === undefined ? nil() : String(arg.typeOid),
    arg === undefined ? nil() : encode(arg.type),
    ...(privilege === null ? nullAcl() : publicAcl(privilege)),
  ];
}

function typeRow(
  oidValue: number,
  name: string,
  kind: string,
  elementOid: number,
  handlerOid: number,
  privilege: string | null,
): string[] {
  return [String(oidValue), '11', encode(name), encode(kind), '10', String(elementOid),
    String(handlerOid), ...(privilege === null ? nullAcl() : publicAcl(privilege))];
}

function globalRow(oidValue: number, name: string, privilege: string): string[] {
  return [String(oidValue), encode(name), '10', ...publicAcl(privilege)];
}

function serverRow(
  oidValue: number, name: string, fdwOid: number, privilege: string,
): string[] {
  return [String(oidValue), encode(name), '10', String(fdwOid), ...publicAcl(privilege)];
}

function publicAcl(privilege: string): string[] {
  return ['f', '1', '1', '1', '10', '0', encode(privilege), 'f'];
}

function nullAcl(): string[] {
  return ['t', nil(), '0', nil(), nil(), nil(), nil(), nil()];
}

function emptyAcl(): string[] {
  return ['f', '0', '0', nil(), nil(), nil(), nil(), nil()];
}

function replaceAcl(row: string[], offset: number, acl: string[]): void {
  row.splice(offset, 8, ...acl);
}

function controlFor(raw: Record<string, string[][]>): string[] {
  const configs: ReadonlyArray<readonly [string, number, (row: string[]) => string]> = [
    ['SCHEMA', 3, (row) => row[0]!], ['RELATION', 5, (row) => row[0]!],
    ['COLUMN', 4, (row) => `${row[0]}:${row[1]}`], ['ROUTINE', 17, (row) => row[0]!],
    ['TYPE', 7, (row) => row[0]!], ['LANGUAGE', 3, (row) => row[0]!],
    ['FDW', 3, (row) => row[0]!], ['SERVER', 4, (row) => row[0]!],
    ['LARGE_OBJECT', 3, (row) => row[0]!],
  ];
  const values: number[] = [];
  for (const [section, offset, key] of configs) {
    const objects = new Map<string, string[]>();
    raw[section]!.forEach((row) => objects.set(key(row), row));
    const rows = [...objects.values()];
    values.push(rows.length,
      rows.reduce((sum, row) => sum + (row[offset + 1] === nil() ? 0 : Number(row[offset + 1])), 0),
      rows.reduce((sum, row) => sum + Number(row[offset + 2]), 0));
    if (section === 'ROUTINE') {
      values.push(rows.reduce((sum, row) => sum + Number(row[10]), 0));
    }
  }
  values.push(0, 0, 0, 0);
  return values.map(String);
}

function nullable(value: string | null, convert: (input: string) => string): string {
  return value === null ? nil() : convert(value);
}

function encode(value: string): string {
  return Buffer.from(value, 'utf8').toString('hex');
}

function nil(): string {
  return String.raw`\N`;
}

function objectValue(value: unknown): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)
    || Object.getPrototypeOf(value) !== Object.prototype) throw new Error('TEST_OBJECT_INVALID');
  return value as Record<string, unknown>;
}

function expectExactKeys(value: Record<string, unknown>, keys: readonly string[]): void {
  expect(Object.keys(value)).toEqual(keys);
}

function expectDigest(value: unknown): void {
  expect(value).toMatch(/^[0-9a-f]{64}$/u);
}

function sha256(value: Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}
