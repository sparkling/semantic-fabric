// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
// @ts-expect-error The test exercises the private JavaScript witness directly.
import { parseWitnessInventory, parseWitnessSession, verifyWitnessSession } from '../scripts/postgresql-public-acl-has-privilege-v1.mjs';
// @ts-expect-error The test exercises the private inventory wire directly.
import { parseWitnessInventorySession } from '../scripts/postgresql-public-acl-has-privilege-inventory-v1.mjs';

const SQL_PATH = resolve(
  import.meta.dirname, 'fixtures/postgresql-16.15-public-acl-has-privilege-witness-v1.sql',
);
const INVENTORY_SQL_PATH = resolve(
  import.meta.dirname, 'fixtures/postgresql-16.15-public-acl-has-privilege-inventory-v1.sql',
);
const ROLE = 'sf_public_acl_no_membership_witness_v1';
const SECTIONS = [
  'ROLE', 'AUTHORITY', 'SCHEMA', 'RELATION', 'COLUMN',
  'ROUTINE', 'TYPE', 'LANGUAGE', 'FDW', 'SERVER',
] as const;
const AUTHORITIES = [
  'acl-column', 'acl-database', 'acl-default', 'acl-fdw', 'acl-language',
  'acl-large-object', 'acl-parameter', 'acl-relation', 'acl-routine', 'acl-schema',
  'acl-server', 'acl-tablespace', 'acl-type', 'owned-database', 'owned-fdw',
  'owned-language', 'owned-large-object', 'owned-relation', 'owned-routine',
  'owned-schema', 'owned-server', 'owned-tablespace', 'owned-type',
  'predefined-member', 'predefined-set', 'predefined-usage',
] as const;
const TABLE_PRIVILEGES = [
  'DELETE', 'INSERT', 'REFERENCES', 'SELECT', 'TRIGGER', 'TRUNCATE', 'UPDATE',
] as const;
const COLUMN_PRIVILEGES = ['INSERT', 'REFERENCES', 'SELECT', 'UPDATE'] as const;
const RELATION_OID = 10;
const TYPE_OID = 20;
const ELEMENT_OID = 21;

interface AclRecord {
  readonly objectClass: string;
  readonly schemaName: string | null;
  readonly objectName: string;
  readonly subobjectName: string | null;
  readonly objectKind: string;
  readonly routineIdentityArguments: string | null;
  readonly privilege: string;
  readonly grantable: boolean;
}

describe('PostgreSQL 16.15 no-membership has_* privilege witness', () => {
  it('defines a read-only OID-based witness independent of the fixture bytes', () => {
    const sql = readFileSync(SQL_PATH, 'utf8');
    expect(sql).toContain('BEGIN ISOLATION LEVEL SERIALIZABLE READ ONLY DEFERRABLE');
    expect(sql).toContain('SET LOCAL search_path TO pg_catalog');
    expect(sql).toContain('SET LOCAL row_security TO on');
    for (const name of [
      'has_schema_privilege', 'has_table_privilege', 'has_sequence_privilege',
      'has_column_privilege', 'has_function_privilege', 'has_type_privilege',
      'has_language_privilege', 'has_foreign_data_wrapper_privilege',
      'has_server_privilege',
    ]) expect(sql).toContain(`pg_catalog.${name}`);
    expect(sql).toContain('WITH GRANT OPTION');
    expect(sql).toContain('has_column_privilege(w.oid, c.oid, a.attnum, p.name)');
    expect(sql).toContain("has_function_privilege(w.oid, p.oid, 'EXECUTE')");
    expect(sql).toContain("has_type_privilege(w.oid, t.oid, 'USAGE')");
    expect(sql).toContain('pg_catalog.pg_has_role');
    expect(sql).toContain('pg_catalog.aclexplode');
    expect(sql).not.toMatch(/\bSET\s+(?:LOCAL\s+)?ROLE\b/iu);
    expect(sql).not.toContain('postgresql-16.15-clean-template0-public-object-acl-v1.json');
    expect(sql).not.toContain('a108e05f9cfd6d6485a86fe198a87b3800e21986b5c62e6251519de6577d05be');
    expect((sql.match(/@@ADR0047-HAS-V1\/[A-Z_]+\/BEGIN@@/gu) ?? [])).toHaveLength(10);
    expect((sql.match(/@@ADR0047-HAS-V1\/[A-Z_]+\/END@@/gu) ?? [])).toHaveLength(10);
    expect(sql.endsWith('\nROLLBACK;\n')).toBe(true);
  });

  it('derives the complete candidate matrix without privilege inquiry or fixture input', () => {
    const sql = readFileSync(INVENTORY_SQL_PATH, 'utf8');
    expect(sql).toContain('BEGIN ISOLATION LEVEL SERIALIZABLE READ ONLY DEFERRABLE');
    expect(sql).not.toMatch(/has_[a-z_]+_privilege/iu);
    expect(sql).not.toContain('pg_catalog.aclexplode');
    expect(sql).not.toContain(ROLE);
    expect((sql.match(/@@ADR0047-HAS-INVENTORY-V1\/[A-Z_]+\/BEGIN@@/gu) ?? []))
      .toHaveLength(8);
    expect((sql.match(/@@ADR0047-HAS-INVENTORY-V1\/[A-Z_]+\/END@@/gu) ?? []))
      .toHaveLength(8);
    expect(sql.endsWith('\nROLLBACK;\n')).toBe(true);
    const parsed = parseWitnessInventorySession(Buffer.from(inventoryTranscript()));
    expect(parsed.canonical.equals(inventory())).toBe(true);
  });

  it('cleans up an uncertain role creation and grants evidence queries a bounded long timeout', () => {
    const runner = readFileSync(resolve(
      import.meta.dirname, '../scripts/verify-postgresql-public-acl-has-privilege-v1.mjs',
    ), 'utf8');
    const support = readFileSync(resolve(
      import.meta.dirname, '../scripts/postgresql-public-acl-replay-support-v2.mjs',
    ), 'utf8');
    expect(runner.indexOf('createAttempted = true;')).toBeLessThan(
      runner.indexOf("runPsql('postgres', CREATE_ROLE"),
    );
    expect(runner).toContain('if (roleCount() === 1)');
    expect(runner).toContain('assert(roleCount() === 0');
    expect(runner.match(/120_000/gu)).toHaveLength(2);
    expect(support).toContain('undefined, 300_000');
  });

  it('accepts the full column truth table and a true array', () => {
    const value = transcript();
    expect(parseWitnessSession(Buffer.from(value)).raw.COLUMN).toHaveLength(8);
    expect(verifyWitnessSession(Buffer.from(value), fixture(), inventory())).toMatchObject({
      checkCount: 16,
      plainTrueCount: 5,
      plainFalseCount: 11,
      grantOptionTrueCount: 0,
      corroboratedAtoms: 3,
      columnLocalAtoms: 1,
      trueArrayAtoms: 1,
    });
  });

  it('rejects framing, duplicate, boolean, grant, role and array mutants', () => {
    const valid = transcript();
    const relation = row('RELATION', [
      `${RELATION_OID}`, h('catalogue'), h('sample'), h('table'), h('SELECT'), 't', 'f',
    ]);
    const columnLocal = row('COLUMN', [
      `${RELATION_OID}`, '1', h('catalogue'), h('sample'), h('c'), h('table'), h('UPDATE'),
      't', 'f', 'f', 'f',
    ]);
    const columnAbsent = row('COLUMN', [
      `${RELATION_OID}`, '2', h('catalogue'), h('sample'), h('d'), h('table'), h('UPDATE'),
      'f', 'f', 'f', 'f',
    ]);
    const type = row('TYPE', [
      `${TYPE_OID}`, h('pg_catalog'), h('_int4'), h('array'), 't', `${ELEMENT_OID}`,
      h('pg_catalog'), h('int4'), h('USAGE'), 't', 'f',
    ]);
    const role = roleRow();
    const mutants = [
      valid.replace('@@ADR0047-HAS-V1/TYPE/END@@\n', ''),
      valid.replace(`${relation}\n`, `${relation}\n${relation}\n`),
      valid.replace(relation, relation.replace('\tt\tf', '\tf\tf')),
      valid.replace(columnLocal, columnLocal.replace('\tt\tf\tf\tf', '\tf\tf\tf\tf')),
      valid.replace(columnAbsent, columnAbsent.replace('\tf\tf\tf\tf', '\tt\tf\tf\tf')),
      valid.replace(type, type.replace(/\tt\tf$/u, '\tt\tt')),
      valid.replace(type, type.replace(`\tt\t${ELEMENT_OID}\t`, `\tf\t${ELEMENT_OID}\t`)),
      valid.replace(role, role.replace(`${h(ROLE)}\tf\tf`, `${h(ROLE)}\tf\tt`)),
      valid.slice(0, -1),
      valid.replace('\n', '\r\n'),
    ];
    mutants.forEach((mutant) => {
      expect(() => verifyWitnessSession(Buffer.from(mutant), fixture(), inventory())).toThrow();
    });
  });

  it('requires every isolation authority exactly once and equal to zero', () => {
    const valid = transcript();
    const first = row('AUTHORITY', [h(AUTHORITIES[0]), '0']);
    for (const mutant of [
      valid.replace(`${first}\n`, ''),
      valid.replace(first, `${first}\n${first}`),
      valid.replace(first, row('AUTHORITY', [h(AUTHORITIES[0]), '1'])),
      valid.replace(first, row('AUTHORITY', [h('unknown-authority'), '0'])),
    ]) expect(() => verifyWitnessSession(Buffer.from(mutant), fixture(), inventory())).toThrow();
  });

  it('requires the independently supplied negative-object and array identity inventory', () => {
    const valid = transcript();
    const negativeColumnRows = COLUMN_PRIVILEGES.map((privilege) => row('COLUMN', [
      `${RELATION_OID}`, '2', h('catalogue'), h('sample'), h('d'), h('table'), h(privilege),
      privilege === 'SELECT' ? 't' : 'f', 'f', privilege === 'SELECT' ? 't' : 'f', 'f',
    ]));
    const withoutNegativeObject = valid.replace(`${negativeColumnRows.join('\n')}\n`, '');
    const changedArrayElement = valid.replace(h('int4'), h('text'));
    expect(() => verifyWitnessSession(
      Buffer.from(withoutNegativeObject), fixture(), inventory(),
    )).toThrow('WITNESS_INVENTORY_MISMATCH');
    expect(() => verifyWitnessSession(
      Buffer.from(changedArrayElement), fixture(), inventory(),
    )).toThrow('WITNESS_INVENTORY_MISMATCH');
  });

  it('rejects hostile routine scalars and class-block interleaving in canonical inventory', () => {
    for (const identityArguments of ['\0', '\uD800']) {
      const hostile = [identity('routine', 30, 'pg_catalog', 'f', null, null,
        'function', identityArguments, 'EXECUTE', null, null, null, null)];
      expect(() => parseWitnessInventory(
        Buffer.from(`${JSON.stringify(hostile)}\n`, 'utf8'),
      )).toThrow('WITNESS_INVENTORY_ROUTINE_IDENTITY_INVALID');
    }
    const entries = JSON.parse(inventory().toString('utf8')) as Record<string, unknown>[];
    const type = entries.pop();
    expect(type).toBeDefined();
    const lateRelations = TABLE_PRIVILEGES.map((privilege) => identity(
      'relation', 11, 'catalogue', 'late', null, null, 'table', null,
      privilege, null, null, null, null,
    ));
    expect(() => parseWitnessInventory(Buffer.from(
      `${JSON.stringify([...entries, ...lateRelations, type])}\n`, 'utf8',
    ))).toThrow('WITNESS_OBSERVATION_CLASS_ORDER_INVALID');
  });
});

function transcript(): string {
  const rows = Object.fromEntries(
    SECTIONS.map((section) => [section, [] as string[]]),
  ) as Record<(typeof SECTIONS)[number], string[]>;
  rows.ROLE = [roleRow()];
  rows.AUTHORITY = AUTHORITIES.map((authority) => row('AUTHORITY', [h(authority), '0']));
  rows.RELATION = TABLE_PRIVILEGES.map((privilege) => row('RELATION', [
    `${RELATION_OID}`, h('catalogue'), h('sample'), h('table'), h(privilege),
    privilege === 'SELECT' ? 't' : 'f', 'f',
  ]));
  for (const [index, column] of ['c', 'd'].entries()) {
    for (const privilege of COLUMN_PRIVILEGES) {
      const parent = privilege === 'SELECT';
      const local = column === 'c' && privilege === 'UPDATE';
      rows.COLUMN.push(row('COLUMN', [
        `${RELATION_OID}`, `${index + 1}`, h('catalogue'), h('sample'), h(column),
        h('table'), h(privilege),
        parent || local ? 't' : 'f', 'f', parent ? 't' : 'f', 'f',
      ]));
    }
  }
  rows.TYPE = [row('TYPE', [
    `${TYPE_OID}`, h('pg_catalog'), h('_int4'), h('array'), 't', `${ELEMENT_OID}`,
    h('pg_catalog'), h('int4'), h('USAGE'), 't', 'f',
  ])];
  const lines: string[] = [];
  for (const section of SECTIONS) {
    lines.push(`@@ADR0047-HAS-V1/${section}/BEGIN@@`, ...rows[section],
      `@@ADR0047-HAS-V1/${section}/END@@`);
  }
  return `${lines.join('\n')}\n`;
}

function roleRow(): string {
  return row('ROLE', [
    h(ROLE), 'f', 'f', 'f', 'f', 'f', 'f', 'f', '-1',
    't', 't', 't', '0', '0', '0', '160015', h('sf_public_baseline'),
    h('postgres'), h('postgres'),
  ]);
}

function fixture(): Buffer {
  const records: AclRecord[] = [
    record('column', 'catalogue', 'sample', 'c', 'table', 'UPDATE'),
    record('relation', 'catalogue', 'sample', null, 'table', 'SELECT'),
    record('type', 'pg_catalog', '_int4', null, 'array', 'USAGE'),
  ];
  return Buffer.from(`${JSON.stringify(records)}\n`, 'utf8');
}

function inventory(): Buffer {
  const entries: Record<string, unknown>[] = [];
  for (const privilege of TABLE_PRIVILEGES) {
    entries.push(identity('relation', RELATION_OID, 'catalogue', 'sample', null, null,
      'table', null, privilege, null, null, null, null));
  }
  for (const [index, column] of ['c', 'd'].entries()) {
    for (const privilege of COLUMN_PRIVILEGES) {
      entries.push(identity('column', RELATION_OID, 'catalogue', 'sample', column,
        index + 1, 'table', null, privilege, null, null, null, null));
    }
  }
  entries.push(identity('type', TYPE_OID, 'pg_catalog', '_int4', null, null, 'array', null,
    'USAGE', true, ELEMENT_OID, 'pg_catalog', 'int4'));
  return Buffer.from(`${JSON.stringify(entries)}\n`, 'utf8');
}

function inventoryTranscript(): string {
  const sections = [
    'SCHEMA', 'RELATION', 'COLUMN', 'ROUTINE', 'TYPE', 'LANGUAGE', 'FDW', 'SERVER',
  ] as const;
  const rows = Object.fromEntries(
    sections.map((section) => [section, [] as string[]]),
  ) as Record<(typeof sections)[number], string[]>;
  rows.RELATION = TABLE_PRIVILEGES.map((privilege) => row('RELATION', [
    `${RELATION_OID}`, h('catalogue'), h('sample'), h('table'), h(privilege),
  ]));
  for (const [index, column] of ['c', 'd'].entries()) {
    for (const privilege of COLUMN_PRIVILEGES) {
      rows.COLUMN.push(row('COLUMN', [
        `${RELATION_OID}`, `${index + 1}`, h('catalogue'), h('sample'), h(column),
        h('table'), h(privilege),
      ]));
    }
  }
  rows.TYPE = [row('TYPE', [
    `${TYPE_OID}`, h('pg_catalog'), h('_int4'), h('array'), 't', `${ELEMENT_OID}`,
    h('pg_catalog'), h('int4'), h('USAGE'),
  ])];
  const lines: string[] = [];
  for (const section of sections) {
    lines.push(`@@ADR0047-HAS-INVENTORY-V1/${section}/BEGIN@@`, ...rows[section],
      `@@ADR0047-HAS-INVENTORY-V1/${section}/END@@`);
  }
  return `${lines.join('\n')}\n`;
}

function identity(
  objectClass: string, objectOid: number, schemaName: string | null, objectName: string,
  subobjectName: string | null, subobjectNumber: number | null, objectKind: string,
  routineIdentityArguments: string | null, privilege: string, trueArray: boolean | null,
  elementObjectOid: number | null, elementSchemaName: string | null,
  elementObjectName: string | null,
): Record<string, unknown> {
  return { objectClass, objectOid, schemaName, objectName, subobjectName, subobjectNumber,
    objectKind, routineIdentityArguments, privilege, trueArray, elementObjectOid,
    elementSchemaName, elementObjectName };
}

function record(
  objectClass: string, schemaName: string, objectName: string,
  subobjectName: string | null, objectKind: string, privilege: string,
): AclRecord {
  return { objectClass, schemaName, objectName, subobjectName, objectKind,
    routineIdentityArguments: null, privilege, grantable: false };
}

function row(_section: string, cells: readonly string[]): string {
  return cells.join('\t');
}

function h(value: string): string {
  return Buffer.from(value, 'utf8').toString('hex');
}
