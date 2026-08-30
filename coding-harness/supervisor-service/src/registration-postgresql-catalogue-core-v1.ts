// SPDX-License-Identifier: MIT

import type { CatalogueRecordV1 }
  from './registration-postgresql-catalogue-shape-v1.js';
import { POSTGRES_CATALOGUE_LIMITS_V1 }
  from './registration-postgresql-catalogue-scanner-v1.js';
import {
  constraintDefinitionV1,
  domainBaseTypeV1,
  domainCheckExpressionV1,
  tableCheckExpressionV1,
} from './registration-postgresql-catalogue-templates-v1.js';
import {
  catalogueArrayV1,
  catalogueBooleanV1,
  catalogueNumberV1,
  catalogueRecordV1,
  catalogueStringV1,
  recordsV1,
  requireSortedRecordsV1,
  requireV1,
  stringsV1,
} from './registration-postgresql-catalogue-values-v1.js';

const SCHEMA = 'sf_supervisor_v1';
const OWNER = 'sf_supervisor_owner_v1';
const SCOPE = ['project_authority_digest', 'project_scope_role'] as const;

export function validatePostgresCatalogueCoreV1(root: CatalogueRecordV1): void {
  requireV1(text(root, 'domain')
    === 'semantic-fabric/programme-capture/supervisor-postgresql-catalogue-oracle-v1', 'domain');
  requireV1(number(root, 'schemaVersion') === 1, 'version');
  requireV1(text(root, 'schemaName') === SCHEMA && text(root, 'ownerRole') === OWNER, 'identity');
  validateLimits(record(root, 'limits'));
  const domains = validateDomains(root);
  const relations = validateRelations(root);
  const columns = validateColumns(root, domains, relations);
  const constraints = validateConstraints(root, relations, columns);
  validateIndexes(root, constraints, columns);
  validateTriggers(root, constraints);
}

function validateLimits(value: CatalogueRecordV1): void {
  for (const [key, expected] of Object.entries(POSTGRES_CATALOGUE_LIMITS_V1)) {
    requireV1(number(value, key) === expected, 'limits');
  }
}

function validateDomains(root: CatalogueRecordV1): Map<string, CatalogueRecordV1> {
  const values = recordsV1(root, 'domains');
  requireV1(values.length === 10, 'domains');
  requireSortedRecordsV1(values, (value) => [text(value, 'schema'), text(value, 'name')], 'domains');
  const output = new Map<string, CatalogueRecordV1>();
  for (const value of values) {
    const name = text(value, 'name');
    const template = `${name}_check_v1`;
    const expected = domainBaseTypeV1(template);
    requireV1(text(value, 'schema') === SCHEMA && text(value, 'owner') === OWNER, 'domain owner');
    requireV1(text(value, 'baseTypeSchema') === 'pg_catalog'
      && text(value, 'baseTypeName') === expected.type, 'domain base');
    requireV1(value.typeModifier === null && boolean(value, 'notNull') === false
      && value.defaultTemplate === null && value.defaultExpression === null, 'domain defaults');
    requireV1(nullableText(value, 'collationSchema')
      === (expected.collation === null ? null : 'pg_catalog'), 'domain collation');
    requireV1(nullableText(value, 'collationName') === expected.collation, 'domain collation');
    const checks = arrayRecords(value, 'checks');
    requireV1(checks.length === 1, 'domain checks');
    const check = checks[0]!;
    requireV1(text(check, 'name') === template && text(check, 'template') === template
      && text(check, 'expression') === domainCheckExpressionV1(template), 'domain check');
    requireV1(!output.has(name), 'domain identity');
    output.set(name, value);
  }
  return output;
}

function validateRelations(root: CatalogueRecordV1): Map<string, CatalogueRecordV1> {
  const values = recordsV1(root, 'relations');
  requireV1(values.length === 8, 'relations');
  requireSortedRecordsV1(values, (value) => [text(value, 'schema'), text(value, 'name')], 'relations');
  const output = new Map<string, CatalogueRecordV1>();
  for (const value of values) {
    const name = text(value, 'name');
    const migration = name === 'schema_migrations';
    requireV1(text(value, 'schema') === SCHEMA && text(value, 'owner') === OWNER, 'relation owner');
    requireV1(text(value, 'kind') === 'table' && text(value, 'persistence') === 'permanent'
      && text(value, 'accessMethod') === 'heap' && text(value, 'replicaIdentity') === 'default',
    'relation shape');
    requireV1(boolean(value, 'rowSecurityEnabled') === !migration
      && boolean(value, 'rowSecurityForced') === !migration, 'relation RLS');
    const options = stringsV1(value.relOptions!, 'relation options');
    requireV1(new Set(options).size === options.length
      && options.every((item, index) => index === 0 || options[index - 1]! < item), 'relation options');
    requireV1(text(value, 'toastState') === 'linked', 'relation TOAST');
    requireV1(!output.has(name), 'relation identity');
    output.set(name, value);
  }
  requireV1(output.has('schema_migrations'), 'migration relation');
  return output;
}

function validateColumns(
  root: CatalogueRecordV1,
  domains: Map<string, CatalogueRecordV1>,
  relations: Map<string, CatalogueRecordV1>,
): Map<string, CatalogueRecordV1> {
  const values = recordsV1(root, 'columns');
  requireV1(values.length === 88, 'columns');
  requireSortedRecordsV1(values, (value) => [
    text(value, 'schema'), text(value, 'relation'), number(value, 'ordinal'),
  ], 'columns');
  const output = new Map<string, CatalogueRecordV1>();
  const ordinals = new Map<string, number>();
  for (const value of values) {
    const relation = text(value, 'relation');
    const name = text(value, 'name');
    requireV1(text(value, 'schema') === SCHEMA && relations.has(relation), 'column relation');
    const ordinal = number(value, 'ordinal');
    requireV1(ordinal === (ordinals.get(relation) ?? 0) + 1, 'column ordinal');
    ordinals.set(relation, ordinal);
    const type = resolveType(value, domains);
    requireV1(text(value, 'baseProjectionType') === type.projection, 'column projection');
    requireV1(nullableText(value, 'collationSchema')
      === (type.collation === null ? null : 'pg_catalog'), 'column collation');
    requireV1(nullableText(value, 'collationName') === type.collation, 'column collation');
    requireV1(value.defaultTemplate === null && value.defaultExpression === null
      && text(value, 'identityKind') === '' && text(value, 'generatedKind') === '', 'column defaults');
    const key = `${relation}.${name}`;
    requireV1(!output.has(key), 'column identity');
    output.set(key, value);
  }
  for (const relation of relations.keys()) {
    const scoped = relation !== 'schema_migrations';
    const relationColumns = values.filter((value) => text(value, 'relation') === relation);
    requireV1(scoped === (relationColumns.length >= 2
      && text(relationColumns[0]!, 'name') === SCOPE[0]
      && text(relationColumns[1]!, 'name') === SCOPE[1]), 'scope prefix');
  }
  return output;
}

function validateConstraints(
  root: CatalogueRecordV1,
  relations: Map<string, CatalogueRecordV1>,
  columns: Map<string, CatalogueRecordV1>,
): CatalogueRecordV1[] {
  const values = recordsV1(root, 'constraints');
  requireV1(values.length === 60, 'constraints');
  requireSortedRecordsV1(values, (value) => [
    text(value, 'schema'), text(value, 'relation'), text(value, 'name'),
  ], 'constraints');
  const kinds = new Map<string, number>();
  for (const value of values) {
    const relation = text(value, 'relation');
    const kind = text(value, 'kind');
    const local = stringArray(value, 'columns');
    requireV1(text(value, 'schema') === SCHEMA && relations.has(relation)
      && local.length > 0 && distinct(local)
      && local.every((column) => columns.has(`${relation}.${column}`)), 'constraint columns');
    requireV1(boolean(value, 'validated') && (!boolean(value, 'initiallyDeferred')
      || boolean(value, 'deferrable')), 'constraint state');
    kinds.set(kind, (kinds.get(kind) ?? 0) + 1);
    const input = definitionInput(value, kind, local);
    if (kind === 'check') {
      const template = nullableText(value, 'checkTemplate');
      requireV1(template === text(value, 'name') && input.expression === tableCheckExpressionV1(template),
        'check template');
    } else requireV1(value.checkTemplate === null && value.expression === null, 'constraint expression');
    requireV1(text(value, 'definition') === constraintDefinitionV1(input), 'constraint definition');
  }
  requireV1(JSON.stringify(Object.fromEntries(kinds))
    === JSON.stringify({ unique: 16, check: 15, 'primary-key': 8, 'foreign-key': 21 }),
  'constraint kinds');
  for (const value of values.filter((item) => text(item, 'kind') === 'foreign-key')) {
    const targetRelation = nullableText(value, 'referencedRelation')!;
    const targetColumns = nullableStringArray(value, 'referencedColumns')!;
    const targets = values.filter((candidate) => text(candidate, 'relation') === targetRelation
      && ['primary-key', 'unique'].includes(text(candidate, 'kind'))
      && JSON.stringify(stringArray(candidate, 'columns')) === JSON.stringify(targetColumns));
    requireV1(targets.length === 1, 'foreign key target');
  }
  for (const relation of relations.keys()) {
    requireV1(values.filter((value) => text(value, 'relation') === relation
      && text(value, 'kind') === 'primary-key').length === 1, 'primary key');
  }
  return values;
}

function validateIndexes(
  root: CatalogueRecordV1,
  constraints: CatalogueRecordV1[],
  columns: Map<string, CatalogueRecordV1>,
): void {
  const values = recordsV1(root, 'indexes');
  requireV1(values.length === 24, 'indexes');
  requireSortedRecordsV1(values, (value) => [text(value, 'schema'), text(value, 'name')], 'indexes');
  for (const value of values) {
    const relation = text(value, 'relation');
    const constraintName = text(value, 'constraintName');
    const matches = constraints.filter((item) => text(item, 'relation') === relation
      && text(item, 'name') === constraintName
      && ['primary-key', 'unique'].includes(text(item, 'kind')));
    requireV1(matches.length === 1 && text(value, 'schema') === SCHEMA
      && text(value, 'relationSchema') === SCHEMA && text(value, 'name') === constraintName,
    'index constraint');
    const constraint = matches[0]!;
    requireV1(text(value, 'accessMethod') === 'btree' && boolean(value, 'unique')
      && boolean(value, 'primary') === (text(constraint, 'kind') === 'primary-key')
      && boolean(value, 'immediate') && !boolean(value, 'nullsNotDistinct')
      && !boolean(value, 'clustered') && !boolean(value, 'replicaIdentity')
      && boolean(value, 'valid') && boolean(value, 'ready') && boolean(value, 'live'), 'index flags');
    requireV1(stringArray(value, 'includedColumns').length === 0
      && value.predicateTemplate === null && value.predicateExpression === null, 'index extras');
    const expectedColumns = stringArray(constraint, 'columns');
    const keys = arrayRecords(value, 'keys');
    requireV1(keys.length === expectedColumns.length, 'index keys');
    keys.forEach((key, index) => {
      const name = nullableText(key, 'column');
      const column = name === null ? undefined : columns.get(`${relation}.${name}`);
      requireV1(number(key, 'position') === index + 1 && name === expectedColumns[index]
        && key.expression === null && column !== undefined, 'index key');
      requireV1(nullableText(key, 'collationSchema') === nullableText(column!, 'collationSchema')
        && nullableText(key, 'collationName') === nullableText(column!, 'collationName'), 'index collation');
      requireV1(text(key, 'opclassSchema') === 'pg_catalog'
        && text(key, 'opclassName') === `${physicalType(column!)}_ops`
        && text(key, 'direction') === 'asc' && text(key, 'nulls') === 'nulls-last', 'index opclass');
    });
  }
  const backed = constraints.filter((value) => ['primary-key', 'unique'].includes(text(value, 'kind')));
  requireV1(backed.every((constraint) => values.filter((index) =>
    text(index, 'constraintName') === text(constraint, 'name')).length === 1), 'index closure');
}

function validateTriggers(root: CatalogueRecordV1, constraints: CatalogueRecordV1[]): void {
  const values = recordsV1(root, 'foreignKeyTriggers');
  requireV1(values.length === 84, 'triggers');
  requireSortedRecordsV1(values, (value) => [
    text(value, 'constraintSchema'), text(value, 'constraintName'),
    text(value, 'side'), text(value, 'event'),
  ], 'triggers');
  const foreignKeys = constraints.filter((value) => text(value, 'kind') === 'foreign-key');
  for (const constraint of foreignKeys) {
    const name = text(constraint, 'name');
    const triggers = values.filter((value) => text(value, 'constraintName') === name);
    requireV1(triggers.length === 4, 'trigger closure');
    const seen = new Set<string>();
    for (const value of triggers) {
      const side = text(value, 'side');
      const event = text(value, 'event');
      const key = `${side}.${event}`;
      requireV1(!seen.has(key) && text(value, 'constraintSchema') === SCHEMA, 'trigger identity');
      seen.add(key);
      const referencing = side === 'referencing';
      const functionName = referencing
        ? `RI_FKey_check_${event === 'insert' ? 'ins' : 'upd'}`
        : `RI_FKey_restrict_${event === 'delete' ? 'del' : 'upd'}`;
      requireV1((referencing ? ['insert', 'update'] : ['delete', 'update']).includes(event)
        && text(value, 'timing') === 'after' && text(value, 'orientation') === 'row'
        && text(value, 'triggerType') === (referencing ? 'foreign-key-check' : 'foreign-key-action')
        && boolean(value, 'internal') && text(value, 'functionSchema') === 'pg_catalog'
        && text(value, 'functionName') === functionName && text(value, 'enabled') === 'origin',
      'trigger form');
      requireV1(boolean(value, 'deferrable') === (referencing && boolean(constraint, 'deferrable'))
        && boolean(value, 'initiallyDeferred')
          === (referencing && boolean(constraint, 'initiallyDeferred')), 'trigger timing');
    }
  }
  requireV1(values.every((value) => foreignKeys.some((constraint) =>
    text(constraint, 'name') === text(value, 'constraintName'))), 'trigger owner');
}

function definitionInput(value: CatalogueRecordV1, kind: string, columns: string[]) {
  const referencedColumns = nullableStringArray(value, 'referencedColumns');
  if (kind === 'foreign-key') {
    requireV1(nullableText(value, 'referencedSchema') === SCHEMA
      && nullableText(value, 'referencedRelation') !== null
      && referencedColumns !== null && referencedColumns.length === columns.length
      && nullableText(value, 'matchType') === 'simple'
      && nullableText(value, 'updateAction') === 'restrict'
      && nullableText(value, 'deleteAction') === 'restrict', 'foreign key');
  } else requireV1(value.referencedSchema === null && value.referencedRelation === null
    && referencedColumns === null && value.matchType === null && value.updateAction === null
    && value.deleteAction === null && !boolean(value, 'deferrable')
    && !boolean(value, 'initiallyDeferred'), 'non foreign key');
  return {
    kind, columns,
    referencedSchema: nullableText(value, 'referencedSchema'),
    referencedRelation: nullableText(value, 'referencedRelation'), referencedColumns,
    updateAction: nullableText(value, 'updateAction'),
    deleteAction: nullableText(value, 'deleteAction'),
    deferrable: boolean(value, 'deferrable'),
    initiallyDeferred: boolean(value, 'initiallyDeferred'),
    expression: nullableText(value, 'expression'),
  };
}

function resolveType(value: CatalogueRecordV1, domains: Map<string, CatalogueRecordV1>) {
  const schema = text(value, 'typeSchema');
  const name = text(value, 'typeName');
  if (schema === SCHEMA) {
    const domain = domains.get(name);
    requireV1(domain !== undefined, 'column domain');
    const base = text(domain!, 'baseTypeName');
    return { projection: base === 'numeric' ? 'text' : base, collation: nullableText(domain!, 'collationName') };
  }
  requireV1(schema === 'pg_catalog' && ['bool', 'int2', 'int4', 'text'].includes(name), 'column type');
  const projection = name === 'bool' ? 'boolean'
    : name === 'int4' ? 'integer' : name === 'int2' ? 'text' : 'text';
  return { projection, collation: name === 'text' ? 'C' : null };
}

function physicalType(column: CatalogueRecordV1): string {
  const type = text(column, 'typeName');
  if (text(column, 'typeSchema') === 'pg_catalog') return type;
  if (type === 'uint64_v1') return 'numeric';
  if (['opaque_id_v1', 'project_scope_role_v1'].includes(type)) return 'text';
  return 'bytea';
}

function text(value: CatalogueRecordV1, key: string): string {
  return catalogueStringV1(value[key]!, key);
}
function number(value: CatalogueRecordV1, key: string): number {
  return catalogueNumberV1(value[key]!, key);
}
function boolean(value: CatalogueRecordV1, key: string): boolean {
  return catalogueBooleanV1(value[key]!, key);
}
function record(value: CatalogueRecordV1, key: string): CatalogueRecordV1 {
  return catalogueRecordV1(value[key]!, key);
}
function arrayRecords(value: CatalogueRecordV1, key: string): CatalogueRecordV1[] {
  return catalogueArrayV1(value[key]!, key).map((item) => catalogueRecordV1(item, key));
}
function nullableText(value: CatalogueRecordV1, key: string): string | null {
  return value[key] === null ? null : text(value, key);
}
function stringArray(value: CatalogueRecordV1, key: string): string[] {
  return stringsV1(value[key]!, key);
}
function nullableStringArray(value: CatalogueRecordV1, key: string): string[] | null {
  return value[key] === null ? null : stringArray(value, key);
}
function distinct(values: string[]): boolean {
  return new Set(values).size === values.length;
}
