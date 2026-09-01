// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  copyPostgresPublicAclFixtureBytesV1,
  parsePostgresPublicAclFixtureV1,
} from './registration-postgresql-public-acl-fixture-reader-v1.js';

const ROOT = resolve(import.meta.dirname, '..');
const FIXTURE = resolve(
  import.meta.dirname,
  'fixtures/postgresql-16.15-clean-template0-public-object-acl-v1.json',
);
const PROJECTION = resolve(
  import.meta.dirname,
  'fixtures/postgresql-16.15-public-acl-projection-v1.sql',
);
const CAPTURE = resolve(ROOT, 'scripts/capture-postgresql-public-acl-baseline-v1.mjs');
const EXPECTED = Object.freeze({
  bytes: 860_988,
  records: 4_059,
  nodes: 36_532,
  sha256: 'a108e05f9cfd6d6485a86fe198a87b3800e21986b5c62e6251519de6577d05be',
  projectionBytes: 6_859,
  projectionSha256: '0e3ad724f4ce85191564c245c51dd7665b6d9aa704c355067a0056cdbfe95232',
});
const KEYS = Object.freeze([
  'objectClass', 'schemaName', 'objectName', 'subobjectName', 'objectKind',
  'routineIdentityArguments', 'privilege', 'grantable',
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

describe('PostgreSQL 16.15 PUBLIC ACL baseline', () => {
  const fixture = new Uint8Array(readFileSync(FIXTURE));
  const fixtureHandle = parsePostgresPublicAclFixtureV1(fixture);
  const fixtureBytes = copyPostgresPublicAclFixtureBytesV1(fixtureHandle);
  const text = new TextDecoder('utf-8', { fatal: true }).decode(fixtureBytes);
  const rows = fixtureHandle.records;

  it('pins the independently replayed compact fixture bytes', () => {
    expect(fixtureBytes.byteLength).toBe(EXPECTED.bytes);
    expect(sha256(fixtureBytes)).toBe(EXPECTED.sha256);
    expect(text.startsWith('[')).toBe(true);
    expect(text.endsWith(']\n')).toBe(true);
    expect(text.includes('\r')).toBe(false);
    expect(Buffer.from(text, 'utf8').equals(fixtureBytes)).toBe(true);
    expect(Array.isArray(rows)).toBe(true);
    expect(rows).toHaveLength(EXPECTED.records);
    expect(`${JSON.stringify(rows)}\n`).toBe(text);
    expect(1 + ((rows as unknown[]).length * 9)).toBe(EXPECTED.nodes);
  });

  it('has one closed vocabulary and strict unsigned-UTF8 tuple order', () => {
    const records = rows as PublicAclRecordV1[];
    records.forEach(validateRecord);
    for (let index = 1; index < records.length; index += 1) {
      expect(compare(records[index - 1]!, records[index]!)).toBeLessThan(0);
    }
    expect(count(records, 'objectClass')).toEqual({
      column: 16,
      language: 4,
      relation: 189,
      routine: 3_235,
      schema: 2,
      type: 613,
    });
    expect(count(records, 'objectKind')).toEqual({
      table: 77,
      language: 4,
      view: 128,
      function: 3_063,
      aggregate: 157,
      'window-function': 15,
      schema: 2,
      array: 294,
      composite: 209,
      domain: 5,
      base: 68,
      pseudo: 25,
      multirange: 6,
      range: 6,
    });
    expect(count(records, 'privilege')).toEqual({
      SELECT: 204,
      USAGE: 619,
      UPDATE: 1,
      EXECUTE: 3_235,
    });
  });

  it('pins the reviewed projection without embedding the expected result pin', () => {
    const projection = readFileSync(PROJECTION);
    const capture = readFileSync(CAPTURE, 'utf8');
    expect(projection.byteLength).toBe(EXPECTED.projectionBytes);
    expect(sha256(projection)).toBe(EXPECTED.projectionSha256);
    const sql = projection.toString('utf8');
    expect(sql).toContain("CASE WHEN c.relkind = 'S' THEN 's' ELSE 'r' END");
    expect(sql).toContain("pg_catalog.acldefault('T', element.typowner)");
    expect(sql).toContain("'pg_catalog.array_subscript_handler'::regproc");
    expect(sql).not.toMatch(/\bCOALESCE\s*\(/u);
    expect((sql.match(/pg_catalog\.cardinality\([^)]*acl\) = 0/gu) ?? [])).toHaveLength(9);
    expect(capture).toContain('SET LOCAL quote_all_identifiers TO off');
    expect(capture).toContain("'nonNullTrueArrayAclCount'");
    expect((capture.match(/pg_catalog\.cardinality\([^)]*acl\) = 0/gu) ?? []))
      .toHaveLength(3);
    expect(sql).toContain('UNION ALL');
    expect(sql).not.toContain('pg_default_acl');
    expect(sql).not.toContain('pg_init_privs');
    expect(sql).not.toContain(EXPECTED.sha256);
    expect(capture).not.toContain(EXPECTED.sha256);
  });

  it('keeps capture evidence outside the non-deployable Node oracle artifact', () => {
    const artifact = JSON.parse(
      readFileSync(resolve(ROOT, '.service/artifact.json'), 'utf8'),
    ) as { buildInputs: Record<string, string>; sourceInputs: Record<string, string> };
    const artifactPaths = [...Object.keys(artifact.buildInputs), ...Object.keys(artifact.sourceInputs)];
    const evidencePaths = [
      '__tests__/registration-postgresql-public-acl-baseline-v1.test.ts',
      '__tests__/registration-postgresql-public-acl-completeness-oracle-v1.test.ts',
      '__tests__/registration-postgresql-public-acl-fixture-hostile-v1.test.ts',
      '__tests__/registration-postgresql-public-acl-fixture-limits-v1.test.ts',
      '__tests__/registration-postgresql-public-acl-fixture-reader-v1.ts',
      '__tests__/registration-postgresql-public-acl-fixture-scanner-v1.ts',
      '__tests__/fixtures/postgresql-16.15-clean-template0-public-object-acl-v1.json',
      '__tests__/fixtures/postgresql-16.15-public-acl-projection-v1.sql',
      '__tests__/fixtures/postgresql-16.15-public-acl-completeness-oracle-v1.sql',
      '__tests__/fixtures/postgresql-16.15-public-acl-capture-receipt-v1.json',
      'scripts/capture-postgresql-public-acl-baseline-v1.mjs',
      'scripts/postgresql-public-acl-oracle-wire-v1.mjs',
      'scripts/postgresql-public-acl-oracle-v1.mjs',
      'scripts/replay-postgresql-public-acl-baseline-v1.mjs',
      'scripts/verify-postgresql-public-acl-oracle-v1.mjs',
    ];
    evidencePaths.forEach((path) => expect(artifactPaths).not.toContain(path));
  });

  it('gates the owned replay on both exact supported Node runtimes', () => {
    const workflow = readFileSync(resolve(ROOT, '../../.github/workflows/ci.yml'), 'utf8');
    const start = workflow.indexOf('  postgresql-public-acl-replay:');
    const end = workflow.indexOf('\n  build:', start);
    expect(start).toBeGreaterThan(-1);
    expect(end).toBeGreaterThan(start);
    const job = workflow.slice(start, end);
    expect(job.match(/- node: '[^']+'/gu)).toEqual(["- node: '20.0.0'", "- node: '24.14.1'"]);
    expect(job.match(/docker pull postgres@sha256:/gu)).toHaveLength(1);
    expect(job.split(
      'node coding-harness/supervisor-service/scripts/replay-postgresql-public-acl-baseline-v1.mjs',
    )).toHaveLength(2);
  });
});

function validateRecord(value: PublicAclRecordV1): void {
  expect(value).not.toBeNull();
  expect(Array.isArray(value)).toBe(false);
  expect(Object.getPrototypeOf(value)).toBe(Object.prototype);
  expect(Object.keys(value)).toEqual(KEYS);
  expect(value.grantable).toBe(false);
  expect(isIdentifier(value.objectName)).toBe(true);
  const schemaBound = ['column', 'relation', 'routine', 'type'].includes(value.objectClass);
  expect(schemaBound ? isIdentifier(value.schemaName) : value.schemaName === null).toBe(true);
  expect(value.objectClass === 'column'
    ? isIdentifier(value.subobjectName) : value.subobjectName === null).toBe(true);
  expect(value.objectClass === 'routine'
    ? typeof value.routineIdentityArguments === 'string'
      && Buffer.byteLength(value.routineIdentityArguments, 'utf8') <= 196_608
    : value.routineIdentityArguments === null).toBe(true);
  expect(validKind(value.objectClass, value.objectKind)).toBe(true);
  expect(validPrivilege(value)).toBe(true);
}

function validKind(objectClass: string, objectKind: string): boolean {
  const kinds: Readonly<Record<string, readonly string[]>> = {
    column: ['table', 'partitioned-table', 'view', 'materialized-view', 'foreign-table'],
    'foreign-data-wrapper': ['foreign-data-wrapper'],
    'foreign-server': ['foreign-server'],
    language: ['language'],
    relation: ['table', 'partitioned-table', 'view', 'materialized-view', 'foreign-table', 'sequence'],
    routine: ['function', 'procedure', 'aggregate', 'window-function'],
    schema: ['schema'],
    type: ['base', 'composite', 'domain', 'enum', 'pseudo', 'range', 'multirange', 'array'],
  };
  return kinds[objectClass]?.includes(objectKind) === true;
}

function validPrivilege(value: PublicAclRecordV1): boolean {
  if (value.objectClass === 'schema') return ['CREATE', 'USAGE'].includes(value.privilege);
  if (value.objectClass === 'relation') {
    return (value.objectKind === 'sequence'
      ? ['SELECT', 'UPDATE', 'USAGE']
      : ['SELECT', 'INSERT', 'UPDATE', 'DELETE', 'TRUNCATE', 'REFERENCES', 'TRIGGER'])
      .includes(value.privilege);
  }
  if (value.objectClass === 'column') {
    return ['INSERT', 'SELECT', 'UPDATE', 'REFERENCES'].includes(value.privilege);
  }
  if (value.objectClass === 'routine') return value.privilege === 'EXECUTE';
  return value.privilege === 'USAGE';
}

function compare(left: PublicAclRecordV1, right: PublicAclRecordV1): number {
  for (const key of KEYS) {
    const a = left[key as keyof PublicAclRecordV1];
    const b = right[key as keyof PublicAclRecordV1];
    if (a === b) continue;
    if (a === null) return -1;
    if (b === null) return 1;
    if (typeof a === 'boolean') return a ? 1 : -1;
    const order = Buffer.compare(Buffer.from(a, 'utf8'), Buffer.from(b as string, 'utf8'));
    if (order !== 0) return order;
  }
  return 0;
}

function isIdentifier(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && !value.includes('\0')
    && Buffer.byteLength(value, 'utf8') <= 63;
}

function count(
  rowsValue: readonly PublicAclRecordV1[],
  key: 'objectClass' | 'objectKind' | 'privilege',
): Record<string, number> {
  const counts: Record<string, number> = {};
  rowsValue.forEach((row) => { counts[row[key]] = (counts[row[key]] ?? 0) + 1; });
  return counts;
}

function sha256(value: Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}
