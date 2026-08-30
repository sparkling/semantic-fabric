// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import {
  assertPostgresCatalogueDigestV1,
  parsePostgresCatalogueContractV1,
} from '../src/registration-postgresql-catalogue-contract-v1.js';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const contractBytes = readFileSync(resolve(root, 'migrations/catalog-contract-v1.json'));
const contractText = contractBytes.toString('utf8');
const INVALID = 'PostgreSQL catalogue contract is invalid';
const EXPECTED_SHA256 =
  'e7ce3572463587f4beed55c35c5a6b93810a270136cb963cf312b580fd1ace69';

type Catalogue = Record<string, any>;
type Mutate = (value: Catalogue) => void;

describe('PostgreSQL catalogue semantic and cross-reference KATs V1', () => {
  it('independently reproduces the reviewed root-line wire format', () => {
    expect(Buffer.from(encodeRootLines(loadCatalogue()))).toEqual(contractBytes);
  });

  it.each([
    'maximumBytes',
    'maximumDepth',
    'maximumNodes',
    'maximumRecords',
    'maximumCollectionWidth',
    'maximumObjectKeys',
    'maximumStringBytes',
    'maximumIdentifierBytes',
  ])('rejects encoded %s drift from the frozen parser ceiling', (name) => {
    expectRejected((value) => { value.limits[name] += 1; });
  });

  it.each([
    ['reordered domains', (value: Catalogue) => swap(value.domains, 0, 1)],
    ['duplicate relation identity', (value: Catalogue) => {
      value.relations[1].name = value.relations[0].name;
    }],
    ['duplicate column identity', (value: Catalogue) => {
      const columns = value.columns.filter((item: Catalogue) =>
        item.relation === 'authority_configurations');
      columns[1].name = columns[0].name;
    }],
    ['reordered constraints', (value: Catalogue) => swap(value.constraints, 0, 1)],
    ['duplicate policy identity', (value: Catalogue) => {
      value.policies[1].name = value.policies[0].name;
    }],
  ] satisfies ReadonlyArray<readonly [string, Mutate]>)('rejects %s', (_label, mutate) => {
    expectRejected(mutate);
  });

  it.each([
    ['dangling foreign-key relation', (value: Catalogue) => {
      const foreignKey = named(value.constraints, 'publication_outbox_event_fk_v1');
      foreignKey.referencedRelation = 'missing_relation_v1';
      rewriteForeignKeyDefinition(foreignKey);
    }],
    ['relinked foreign key without a unique target', (value: Catalogue) => {
      const foreignKey = named(value.constraints, 'publication_outbox_event_fk_v1');
      foreignKey.referencedRelation = 'registration_results';
      rewriteForeignKeyDefinition(foreignKey);
    }],
    ['foreign-key target-column cardinality drift', (value: Catalogue) => {
      const foreignKey = named(value.constraints, 'publication_outbox_event_fk_v1');
      foreignKey.referencedColumns.pop();
      rewriteForeignKeyDefinition(foreignKey);
    }],
    ['foreign-key definition drift', (value: Catalogue) => {
      named(value.constraints, 'publication_outbox_event_fk_v1').definition += ' DEFERRABLE';
    }],
  ] satisfies ReadonlyArray<readonly [string, Mutate]>)('rejects %s', (_label, mutate) => {
    expectRejected(mutate);
  });

  it.each([
    ['index-to-constraint linkage drift', (value: Catalogue) => {
      value.indexes[0].constraintName = value.indexes[1].constraintName;
    }],
    ['index opclass drift', (value: Catalogue) => {
      value.indexes[0].keys[0].opclassName = 'text_ops';
    }],
    ['index validity weakening', (value: Catalogue) => {
      value.indexes[0].valid = false;
    }],
    ['index order drift', (value: Catalogue) => swap(value.indexes, 0, 1)],
    ['trigger function drift', (value: Catalogue) => {
      value.foreignKeyTriggers[0].functionName = 'RI_FKey_restrict_upd';
    }],
    ['trigger internality weakening', (value: Catalogue) => {
      value.foreignKeyTriggers[0].internal = false;
    }],
    ['missing foreign-key trigger', (value: Catalogue) => {
      value.foreignKeyTriggers.pop();
    }],
    ['duplicate trigger identity', (value: Catalogue) => {
      value.foreignKeyTriggers[0].event = value.foreignKeyTriggers[1].event;
    }],
  ] satisfies ReadonlyArray<readonly [string, Mutate]>)('rejects %s', (_label, mutate) => {
    expectRejected(mutate);
  });

  it.each([
    ['RLS disablement', (value: Catalogue) => {
      named(value.relations, 'registration_results').rowSecurityEnabled = false;
    }],
    ['forced-RLS disablement', (value: Catalogue) => {
      named(value.relations, 'registration_results').rowSecurityForced = false;
    }],
    ['TOAST linkage drift', (value: Catalogue) => {
      named(value.relations, 'registration_results').toastState = 'absent';
    }],
    ['physical-to-projection type drift', (value: Catalogue) => {
      const column = columnNamed(value, 'registration_results', 'response_status');
      column.baseProjectionType = 'bytea';
    }],
    ['text collation drift', (value: Catalogue) => {
      const column = columnNamed(value, 'registration_results', 'response_content_type');
      column.collationName = null;
      column.collationSchema = null;
    }],
  ] satisfies ReadonlyArray<readonly [string, Mutate]>)('rejects %s', (_label, mutate) => {
    expectRejected(mutate);
  });
});

describe('PostgreSQL catalogue policy and ACL KATs V1', () => {
  it.each([
    ['policy role drift', (value: Catalogue) => {
      value.policies[0].roles[0].name = 'sf_supervisor_writer_login_v1';
    }],
    ['policy PUBLIC widening', (value: Catalogue) => {
      value.policies[0].roles[0].kind = 'public';
      value.policies[0].roles[0].name = null;
    }],
    ['policy template drift', (value: Catalogue) => {
      value.policies[0].usingTemplate = 'scope-capability-v1';
    }],
    ['policy argument drift', (value: Catalogue) => {
      value.policies[0].usingArguments.scopeRole = 'sf_supervisor_writer_login_v1';
    }],
    ['policy expression drift', (value: Catalogue) => {
      value.policies[0].usingExpression += ' ';
    }],
    ['policy permissiveness drift', (value: Catalogue) => {
      value.policies[0].permissive = false;
    }],
  ] satisfies ReadonlyArray<readonly [string, Mutate]>)('rejects %s', (_label, mutate) => {
    expectRejected(mutate);
  });

  it.each([
    ['schema ACL PUBLIC widening', (value: Catalogue) => {
      const atom = value.schemas[0].privileges[0];
      atom.granteeKind = 'public';
      atom.granteeRole = null;
    }],
    ['object ACL PUBLIC widening', (value: Catalogue) => {
      const atom = value.objectAcls[0].privileges[0];
      atom.granteeKind = 'public';
      atom.granteeRole = null;
    }],
    ['object ACL role drift', (value: Catalogue) => {
      value.objectAcls[0].privileges[0].granteeRole = 'sf_supervisor_writer_login_v1';
    }],
    ['column grant-option widening', (value: Catalogue) => {
      value.columnAcls[0].privileges[0].grantable = true;
    }],
    ['default ACL grantor drift', (value: Catalogue) => {
      value.defaultAcls[0].privileges[0].grantorRole = 'sf_supervisor_writer_login_v1';
    }],
  ] satisfies ReadonlyArray<readonly [string, Mutate]>)('rejects %s', (_label, mutate) => {
    expectRejected(mutate);
  });
});

describe('PostgreSQL exact-result query KATs V1', () => {
  it.each([
    ['root relation drift', (value: Catalogue) => {
      query(value).root.relation = 'semantic_events';
    }],
    ['parameter order drift', (value: Catalogue) => {
      swap(query(value).parameters, 0, 1);
    }],
    ['parameter type drift', (value: Catalogue) => {
      query(value).parameters[0].baseType = 'text';
    }],
    ['join order drift', (value: Catalogue) => {
      swap(query(value).joins, 0, 1);
    }],
    ['join target drift', (value: Catalogue) => {
      query(value).joins[0].rightRelation = 'registration_runs';
    }],
    ['join scope weakening', (value: Catalogue) => {
      query(value).joins[0].columnPairs.shift();
    }],
    ['predicate order drift', (value: Catalogue) => {
      swap(query(value).predicates, 0, 2);
    }],
    ['predicate parameter reuse', (value: Catalogue) => {
      query(value).predicates[2].operand = 1;
    }],
    ['missing scope predicate', (value: Catalogue) => {
      query(value).predicates.splice(1, 1);
    }],
    ['projection order drift', (value: Catalogue) => {
      swap(query(value).projection, 0, 1);
    }],
    ['projection source drift', (value: Catalogue) => {
      query(value).projection[0].sourceAlias = 'run';
    }],
    ['projection cast drift', (value: Catalogue) => {
      query(value).projection[0].cast = 'text';
    }],
    ['projection alias drift', (value: Catalogue) => {
      query(value).projection[1].outputAlias = query(value).projection[0].outputAlias;
    }],
    ['projection cardinality weakening', (value: Catalogue) => {
      query(value).projection.pop();
    }],
    ['row-cardinality weakening', (value: Catalogue) => {
      query(value).maximumRows = 3;
    }],
  ] satisfies ReadonlyArray<readonly [string, Mutate]>)('rejects %s', (_label, mutate) => {
    expectRejected(mutate);
  });
});

describe('PostgreSQL catalogue exact-byte authority V1', () => {
  it('lets generic relational validation pass a valid count-neutral FK relink but rejects its digest', () => {
    const value = loadCatalogue();
    const foreignKey = named(value.constraints, 'publication_outbox_event_fk_v1');
    foreignKey.referencedRelation = 'registration_results';
    foreignKey.referencedColumns[2] = 'current_event_digest';
    rewriteForeignKeyDefinition(foreignKey);

    const parsed = parsePostgresCatalogueContractV1(encodeRootLines(value));

    expect(parsed.scan).toEqual({ nodes: 9_125, records: 963, maximumDepth: 8 });
    expect(parsed.rawSha256).not.toBe(EXPECTED_SHA256);
    expect(() => assertPostgresCatalogueDigestV1(parsed, EXPECTED_SHA256))
      .toThrowError(new TypeError(INVALID));
  });
});

function loadCatalogue(): Catalogue {
  return JSON.parse(contractText) as Catalogue;
}

function encodeRootLines(value: Catalogue): Uint8Array {
  const keys = Object.keys(value);
  const lines = keys.map((key, index) => {
    const comma = index === keys.length - 1 ? '' : ',';
    return `  ${JSON.stringify(key)}: ${JSON.stringify(value[key])}${comma}`;
  });
  return new TextEncoder().encode(`{\n${lines.join('\n')}\n}\n`);
}

function expectRejected(mutate: Mutate): void {
  const value = loadCatalogue();
  mutate(value);
  expect(() => parsePostgresCatalogueContractV1(encodeRootLines(value)))
    .toThrowError(new TypeError(INVALID));
}

function named(values: Catalogue[], name: string): Catalogue {
  const value = values.find((candidate) => candidate.name === name);
  if (value === undefined) throw new TypeError(`missing test record ${name}`);
  return value;
}

function columnNamed(value: Catalogue, relation: string, name: string): Catalogue {
  const column = value.columns.find((candidate: Catalogue) =>
    candidate.relation === relation && candidate.name === name);
  if (column === undefined) throw new TypeError(`missing test column ${relation}.${name}`);
  return column;
}

function query(value: Catalogue): Catalogue {
  return value.exactQueries[0];
}

function swap(values: unknown[], left: number, right: number): void {
  [values[left], values[right]] = [values[right], values[left]];
}

function rewriteForeignKeyDefinition(value: Catalogue): void {
  value.definition = `FOREIGN KEY (${value.columns.join(', ')}) REFERENCES `
    + `${value.referencedSchema}.${value.referencedRelation}`
    + `(${value.referencedColumns.join(', ')}) ON UPDATE RESTRICT ON DELETE RESTRICT`;
  if (value.deferrable) value.definition += ' DEFERRABLE';
  if (value.initiallyDeferred) value.definition += ' INITIALLY DEFERRED';
}
