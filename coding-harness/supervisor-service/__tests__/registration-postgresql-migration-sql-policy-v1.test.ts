// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import {
  POSTGRES_MIGRATION_SQL_PINS_V1,
  assertPostgresMigrationSqlHandleV1,
  assertPostgresMigrationSqlPolicyV1,
  copyPostgresMigrationSqlBytesV1,
  inspectPostgresMigrationSqlCandidateV1,
  parsePostgresMigrationSqlV1,
} from '../src/registration-postgresql-migration-sql-policy-v1.js';
import {
  postgresMigrationSqlInventoryFromTokensV1,
  scanPostgresMigrationSqlTextV1,
} from '../src/registration-postgresql-migration-sql-scanner-v1.js';

const SQL_1_PATH = fileURLToPath(
  new URL('../migrations/0001-registration-state-v1.sql', import.meta.url),
);
const SQL_2_PATH = fileURLToPath(
  new URL('../migrations/0002-registration-rls-v1.sql', import.meta.url),
);
const SQL_1 = new Uint8Array(readFileSync(SQL_1_PATH));
const SQL_2 = new Uint8Array(readFileSync(SQL_2_PATH));

describe('PostgreSQL migration SQL policy V1', () => {
  it('pins exact framing, bytes, raw digests, and independent token digests', () => {
    for (const [version, bytes] of [[1, SQL_1], [2, SQL_2]] as const) {
      const pin = POSTGRES_MIGRATION_SQL_PINS_V1[version];
      const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
      const report = assertPostgresMigrationSqlPolicyV1(bytes, version);

      expect(bytes).toHaveLength(pin.bytes);
      expect(text.endsWith('\n')).toBe(true);
      expect(text).not.toContain('\r');
      expect(report).toMatchObject({
        reportKind: 'postgresql-migration-sql-policy-review-v1',
        authority: 'none',
        readinessAuthorized: false,
        executableAuthority: false,
        rawByteLength: pin.bytes,
        rawSha256: pin.rawSha256,
        tokenSha256: pin.tokenSha256,
      });
      expect(Object.isFrozen(report)).toBe(true);
      expect(Object.isFrozen(report.inventory)).toBe(true);
    }
  });

  it('pins the exact reviewed DDL, constraint, grant, policy, and RLS inventories', () => {
    expect(assertPostgresMigrationSqlPolicyV1(SQL_1, 1).inventory).toEqual({
      statements: 52,
      createSchemas: 1,
      createDomains: 10,
      createTables: 8,
      namedConstraints: 70,
      checkConstraints: 25,
      primaryKeys: 8,
      uniqueConstraints: 16,
      foreignKeys: 21,
      createIndexes: 0,
      revokeTypeUsage: 8,
      grants: 0,
      createPolicies: 0,
      enableRls: 0,
      forceRls: 0,
      callableFunctions: ['octet_length', 'scale', 'substring'],
      callableFunctionCounts: {
        octet_length: 14, scale: 1, substring: 1, pg_has_role: 0,
      },
      escapeByteaLiterals: 2,
    });
    expect(assertPostgresMigrationSqlPolicyV1(SQL_2, 2).inventory).toEqual({
      statements: 73,
      createSchemas: 0,
      createDomains: 0,
      createTables: 0,
      namedConstraints: 0,
      checkConstraints: 0,
      primaryKeys: 0,
      uniqueConstraints: 0,
      foreignKeys: 0,
      createIndexes: 0,
      revokeTypeUsage: 0,
      grants: 21,
      createPolicies: 38,
      enableRls: 7,
      forceRls: 7,
      callableFunctions: ['pg_has_role'],
      callableFunctionCounts: {
        octet_length: 0, scale: 0, substring: 0, pg_has_role: 38,
      },
      escapeByteaLiterals: 0,
    });
  });

  it('fixes table access methods and uses only GUC-independent bytea literals', () => {
    const text = new TextDecoder().decode(SQL_1);
    expect(text.match(/\) USING heap;/g)).toHaveLength(8);
    expect(text.match(/E'\\\\x[0-9a-f]+'::pg_catalog\.bytea/g)).toHaveLength(2);
    expect(text).not.toMatch(/(?<!E)'\\x[0-9a-f]+'::(?:pg_catalog\.)?bytea/);
  });

  it('bounds dollar-quoted bodies inside the scanner itself', () => {
    expect(scanPostgresMigrationSqlTextV1(`$q$${'x'.repeat(196_608)}$q$;`))
      .toHaveLength(2);
    expect(() => scanPostgresMigrationSqlTextV1(
      `$q$${'x'.repeat(196_609)}$q$;`,
    )).toThrow();
  });

  it('matches PostgreSQL dollar-bearing unquoted identifier tokenization', () => {
    expect(scanPostgresMigrationSqlTextV1('SELECT foo$$body$$;\n').map(
      ({ kind, value }) => [kind, value],
    )).toEqual([
      ['word', 'SELECT'], ['word', 'FOO$$BODY$$'], ['punct', ';'],
    ]);
  });

  it('does not count RLS inventory sequences across statement boundaries', () => {
    const tokens = scanPostgresMigrationSqlTextV1(
      'ENABLE;\nROW LEVEL SECURITY;\nFORCE;\nROW LEVEL SECURITY;\n',
    );
    const inventory = postgresMigrationSqlInventoryFromTokensV1(tokens);
    expect(inventory.enableRls).toBe(0);
    expect(inventory.forceRls).toBe(0);
  });

  it('makes semantic review independent from exact executable-byte authority', () => {
    const source = new TextDecoder().decode(SQL_1);
    const equivalent = sqlBytes(source.replace(
      'CREATE SCHEMA', 'create/* review-only comment */ schema',
    ));
    const report = assertPostgresMigrationSqlPolicyV1(equivalent, 1);

    expect(report.tokenSha256).toBe(POSTGRES_MIGRATION_SQL_PINS_V1[1].tokenSha256);
    expect(report.rawSha256).not.toBe(POSTGRES_MIGRATION_SQL_PINS_V1[1].rawSha256);
    expect(() => parsePostgresMigrationSqlV1(equivalent, 1))
      .toThrow('PostgreSQL migration SQL is invalid');
  });

  it('creates only privately branded exact-byte handles with fresh projections', () => {
    const source = new Uint8Array(SQL_1);
    const handle = parsePostgresMigrationSqlV1(source, 1);
    source.fill(0);
    const first = copyPostgresMigrationSqlBytesV1(handle);
    first.fill(0);
    const second = copyPostgresMigrationSqlBytesV1(handle);

    expect(handle).toEqual({
      sqlKind: 'postgresql-migration-sql-v1',
      version: 1,
      rawByteLength: POSTGRES_MIGRATION_SQL_PINS_V1[1].bytes,
      rawSha256: POSTGRES_MIGRATION_SQL_PINS_V1[1].rawSha256,
      authority: 'none',
      readinessAuthorized: false,
    });
    expect(Object.isFrozen(handle)).toBe(true);
    expect(second).toEqual(SQL_1);
    expect(second).not.toBe(first);
    expect(() => assertPostgresMigrationSqlHandleV1(handle, 1)).not.toThrow();
    expect(() => assertPostgresMigrationSqlHandleV1(handle, 2)).toThrow();
    expect(() => assertPostgresMigrationSqlHandleV1({ ...handle }, 1)).toThrow();
  });

  it('rejects every forbidden owned-object kind as an active CREATE statement', () => {
    const forbidden = [
      'AGGREGATE', 'CAST', 'COLLATION', 'CONVERSION', 'EXTENSION', 'FOREIGN TABLE',
      'FUNCTION', 'MATERIALIZED VIEW', 'OPERATOR', 'PARTITION', 'PROCEDURE',
      'PUBLICATION', 'RULE', 'SEQUENCE', 'SUBSCRIPTION', 'TEXT SEARCH CONFIGURATION',
      'TRIGGER', 'VIEW',
    ];
    for (const kind of forbidden) {
      expect(() => inspectPostgresMigrationSqlCandidateV1(
        sqlBytes(`CREATE ${kind} forbidden_v1;\n`),
      )).toThrow('PostgreSQL migration SQL is invalid');
    }
  });

  it.each([
    'BEGIN;\n',
    'COMMIT;\n',
    'ROLLBACK;\n',
    'SAVEPOINT hostile;\n',
    'PREPARE TRANSACTION \'hostile\';\n',
    'CREATE ROLE hostile;\n',
    'CREATE DATABASE hostile;\n',
    'CREATE OR REPLACE FUNCTION hostile() RETURNS int LANGUAGE sql AS \'SELECT 1\';\n',
    'COPY hostile TO PROGRAM \'true\';\n',
    'SET SESSION AUTHORIZATION hostile;\n',
    'SET ROLE hostile;\n',
    'DO $$ BEGIN END $$;\n',
    '\\set hostile true\n',
  ])('rejects forbidden transaction, authority, dynamic, and psql SQL: %s', (source) => {
    expect(() => inspectPostgresMigrationSqlCandidateV1(sqlBytes(source)))
      .toThrow('PostgreSQL migration SQL is invalid');
  });

  it.each([
    'ALTER ROLE hostile SUPERUSER;\n',
    'ALTER SCHEMA public OWNER TO hostile;\n',
    'GRANT USAGE ON SCHEMA public TO PUBLIC;\n',
    'CREATE TABLE sf_supervisor_v1.child PARTITION OF sf_supervisor_v1.parent '
      + 'FOR VALUES DEFAULT;\n',
    'ALTER TABLE ONLY sf_supervisor_v1.x ENABLE ROW LEVEL SECURITY, '
      + 'DISABLE TRIGGER ALL;\n',
    'GRANT SELECT, DELETE ON TABLE sf_supervisor_v1.x TO hostile;\n',
    'REVOKE ALL PRIVILEGES ON SCHEMA public, sf_supervisor_v1 FROM PUBLIC;\n',
    'CREATE DOMAIN sf_supervisor_v1.hostile AS pg_catalog.int4 CHECK (evil(VALUE));\n',
    'CREATE DOMAIN sf_supervisor_v1.hostile AS pg_catalog.text '
      + 'CONSTRAINT hostile_check CHECK ("evil"(VALUE));\n',
    'CREATE DOMAIN sf_supervisor_v1.hostile AS pg_catalog.text '
      + 'CONSTRAINT hostile_check CHECK (pg_catalog."octet_length"(VALUE));\n',
  ])('rejects widened DDL, privileges, public mutation, and callables: %s', (source) => {
    expect(() => inspectPostgresMigrationSqlCandidateV1(sqlBytes(source)))
      .toThrow('PostgreSQL migration SQL is invalid');
  });

  it('kills coherent identifier, callable, deletion, duplication, and reorder mutants', () => {
    const first = new TextDecoder().decode(SQL_1);
    const second = new TextDecoder().decode(SQL_2);
    const firstStatementEnd = first.indexOf(';\n') + 2;
    const firstStatement = first.slice(0, firstStatementEnd);
    const secondStatementEnd = second.indexOf(';\n') + 2;
    const secondStatement = second.slice(0, secondStatementEnd);
    const mutants = [
      [1, first.replace('authority_state', 'authority_stata')],
      [1, first.replace('pg_catalog.scale', 'pg_catalog.floor')],
      [1, first.slice(firstStatementEnd)],
      [1, firstStatement + first],
      [1, first.slice(firstStatementEnd) + firstStatement],
      [2, second.slice(secondStatementEnd) + secondStatement],
    ] as const;

    for (const [version, mutant] of mutants) {
      expect(() => assertPostgresMigrationSqlPolicyV1(sqlBytes(mutant), version))
        .toThrow('PostgreSQL migration SQL is invalid');
    }
  });
});

function sqlBytes(source: string): Uint8Array {
  return new TextEncoder().encode(source);
}
