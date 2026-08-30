// SPDX-License-Identifier: MIT

import { isProxy } from 'node:util/types';

export type CatalogueJsonPrimitiveV1 = null | boolean | number | string;
export type CatalogueJsonValueV1 = CatalogueJsonPrimitiveV1
  | readonly CatalogueJsonValueV1[] | CatalogueRecordV1;
export type CatalogueRecordV1 = {
  readonly [key: string]: CatalogueJsonValueV1;
};

type Shape =
  | Readonly<{ kind: 'string' | 'identifier' | 'integer' | 'boolean' | 'null' }>
  | Readonly<{ kind: 'enum'; values: readonly CatalogueJsonPrimitiveV1[] }>
  | Readonly<{ kind: 'array'; item: Shape }>
  | Readonly<{ kind: 'record'; fields: readonly (readonly [string, Shape])[] }>
  | Readonly<{ kind: 'union'; choices: readonly Shape[] }>;

const string = shape('string');
const identifier = shape('identifier');
const integer = shape('integer');
const boolean = shape('boolean');
const nil = shape('null');
const nullableString = union(string, nil);
const nullableIdentifier = union(identifier, nil);
const identifiers = array(identifier);
const strings = array(string);

const privilege = record([
  ['grantorRole', identifier], ['granteeKind', enumeration('role', 'public')],
  ['granteeRole', nullableIdentifier],
  ['privilege', enumeration(
    'CREATE', 'DELETE', 'EXECUTE', 'INSERT', 'REFERENCES',
    'SELECT', 'TRIGGER', 'TRUNCATE', 'UPDATE', 'USAGE',
  )],
  ['grantable', boolean],
]);
const privileges = array(privilege);
const policyRole = record([
  ['kind', enumeration('role', 'public')], ['name', nullableIdentifier],
]);
const policyArguments = record([
  ['scopeRole', nullableIdentifier], ['capabilityRole', nullableIdentifier],
  ['sessionLogin', nullableIdentifier], ['ownerRole', nullableIdentifier],
]);
const nullablePolicyArguments = union(policyArguments, nil);

const domainCheck = record([
  ['name', identifier], ['template', identifier], ['expression', string],
]);
const domain = record([
  ['schema', identifier], ['name', identifier], ['owner', identifier],
  ['baseTypeSchema', identifier], ['baseTypeName', identifier],
  ['typeModifier', union(integer, nil)], ['collationSchema', nullableIdentifier],
  ['collationName', nullableIdentifier], ['notNull', boolean],
  ['defaultTemplate', nullableString], ['defaultExpression', nullableString],
  ['checks', array(domainCheck)],
]);
const relation = record([
  ['schema', identifier], ['name', identifier], ['kind', enumeration('table')],
  ['persistence', enumeration('permanent')], ['owner', identifier],
  ['accessMethod', enumeration('heap')], ['rowSecurityEnabled', boolean],
  ['rowSecurityForced', boolean], ['replicaIdentity', enumeration('default')],
  ['relOptions', strings], ['toastState', enumeration('absent', 'linked')],
]);
const column = record([
  ['schema', identifier], ['relation', identifier], ['ordinal', integer],
  ['name', identifier], ['typeSchema', identifier], ['typeName', identifier],
  ['baseProjectionType', enumeration('boolean', 'bytea', 'integer', 'text')],
  ['notNull', boolean], ['defaultTemplate', nullableString],
  ['defaultExpression', nullableString], ['identityKind', string],
  ['generatedKind', string], ['collationSchema', nullableIdentifier],
  ['collationName', nullableIdentifier],
]);
const constraint = record([
  ['schema', identifier], ['relation', identifier], ['name', identifier],
  ['kind', enumeration('check', 'foreign-key', 'primary-key', 'unique')],
  ['columns', identifiers], ['referencedSchema', nullableIdentifier],
  ['referencedRelation', nullableIdentifier],
  ['referencedColumns', union(identifiers, nil)],
  ['matchType', union(enumeration('simple'), nil)],
  ['updateAction', union(enumeration('restrict'), nil)],
  ['deleteAction', union(enumeration('restrict'), nil)], ['deferrable', boolean],
  ['initiallyDeferred', boolean], ['validated', boolean], ['definition', string],
  ['checkTemplate', nullableIdentifier], ['expression', nullableString],
]);
const indexKey = record([
  ['position', integer], ['column', nullableIdentifier], ['expression', nullableString],
  ['collationSchema', nullableIdentifier], ['collationName', nullableIdentifier],
  ['opclassSchema', identifier], ['opclassName', identifier],
  ['direction', enumeration('asc')], ['nulls', enumeration('nulls-last')],
]);
const index = record([
  ['schema', identifier], ['name', identifier], ['relationSchema', identifier],
  ['relation', identifier], ['constraintName', identifier],
  ['accessMethod', enumeration('btree')], ['unique', boolean], ['primary', boolean],
  ['immediate', boolean], ['nullsNotDistinct', boolean], ['clustered', boolean],
  ['replicaIdentity', boolean], ['valid', boolean], ['ready', boolean], ['live', boolean],
  ['keys', array(indexKey)], ['includedColumns', identifiers],
  ['predicateTemplate', nullableString], ['predicateExpression', nullableString],
]);
const trigger = record([
  ['constraintSchema', identifier], ['constraintName', identifier],
  ['side', enumeration('referenced', 'referencing')],
  ['event', enumeration('delete', 'insert', 'update')], ['timing', enumeration('after')],
  ['orientation', enumeration('row')],
  ['triggerType', enumeration('foreign-key-action', 'foreign-key-check')],
  ['internal', boolean], ['functionSchema', identifier], ['functionName', identifier],
  ['deferrable', boolean], ['initiallyDeferred', boolean],
  ['enabled', enumeration('origin')],
]);
const policy = record([
  ['schema', identifier], ['relation', identifier], ['name', identifier],
  ['permissive', boolean], ['command', enumeration('insert', 'select', 'update')],
  ['roles', array(policyRole)], ['usingTemplate', nullableString],
  ['usingArguments', nullablePolicyArguments], ['usingExpression', nullableString],
  ['withCheckTemplate', nullableString], ['withCheckArguments', nullablePolicyArguments],
  ['withCheckExpression', nullableString],
]);
const objectAcl = record([
  ['objectKind', enumeration('domain', 'table', 'type')],
  ['schema', nullableIdentifier], ['object', identifier], ['owner', identifier],
  ['aclState', enumeration('explicit', 'null')], ['privileges', privileges],
]);
const columnAcl = record([
  ['schema', identifier], ['relation', identifier], ['column', identifier],
  ['aclState', enumeration('explicit', 'null')], ['privileges', privileges],
]);
const defaultAcl = record([
  ['owner', identifier], ['schema', nullableIdentifier],
  ['objectClass', enumeration('function', 'schema', 'sequence', 'table', 'type')],
  ['rowState', enumeration('absent', 'explicit')], ['privileges', privileges],
]);

const queryRoot = record([
  ['schema', identifier], ['relation', identifier], ['alias', identifier],
]);
const queryParameter = record([
  ['position', integer], ['name', identifier],
  ['baseType', enumeration('bytea', 'text')],
]);
const queryColumnPair = record([
  ['leftColumn', identifier], ['rightColumn', identifier],
]);
const queryJoin = record([
  ['kind', enumeration('left')], ['leftAlias', identifier],
  ['rightSchema', identifier], ['rightRelation', identifier],
  ['rightAlias', identifier], ['columnPairs', array(queryColumnPair)],
]);
const queryPredicate = record([
  ['sourceAlias', identifier], ['column', identifier], ['operator', enumeration('equals')],
  ['operandKind', enumeration('literal', 'parameter')], ['operand', union(integer, string)],
]);
const queryProjection = record([
  ['sourceAlias', identifier], ['column', identifier],
  ['cast', enumeration('bytea', 'text')], ['outputAlias', identifier],
]);
const exactQuery = record([
  ['name', string], ['root', queryRoot], ['parameters', array(queryParameter)],
  ['joins', array(queryJoin)], ['predicates', array(queryPredicate)],
  ['projection', array(queryProjection)], ['maximumRows', integer],
]);

const implicitObjects = record([
  ['allowedDerivedKinds', strings], ['forbiddenOwnedKinds', strings],
  ['rules', record([
    ['arrayTypes', string], ['compositeRowTypes', string],
    ['constraintIndexes', string], ['foreignKeyTriggers', string],
    ['toastObjects', string],
  ])],
]);

const root = record([
  ['domain', string], ['schemaVersion', integer], ['schemaName', identifier],
  ['ownerRole', identifier], ['limits', record([
    ['maximumBytes', integer], ['maximumDepth', integer], ['maximumNodes', integer],
    ['maximumRecords', integer], ['maximumCollectionWidth', integer],
    ['maximumObjectKeys', integer], ['maximumStringBytes', integer],
    ['maximumIdentifierBytes', integer],
  ])],
  ['schemas', array(record([
    ['name', identifier], ['owner', identifier],
    ['aclState', enumeration('explicit', 'null')], ['privileges', privileges],
  ]))],
  ['domains', array(domain)], ['relations', array(relation)], ['columns', array(column)],
  ['constraints', array(constraint)], ['indexes', array(index)],
  ['foreignKeyTriggers', array(trigger)], ['policies', array(policy)],
  ['objectAcls', array(objectAcl)], ['columnAcls', array(columnAcl)],
  ['defaultAcls', array(defaultAcl)], ['implicitObjects', implicitObjects],
  ['exactQueries', array(exactQuery)],
]);

export const POSTGRES_CATALOGUE_ROOT_KEYS_V1 = Object.freeze(
  root.fields.map(([key]) => key),
);

export function reconstructPostgresCatalogueShapeV1(value: unknown): CatalogueRecordV1 {
  return deepFreeze(reconstruct(value, root) as CatalogueRecordV1);
}

function reconstruct(value: unknown, expected: Shape): CatalogueJsonValueV1 {
  if (expected.kind === 'union') {
    for (const choice of expected.choices) {
      try { return reconstruct(value, choice); } catch { /* try the next closed shape */ }
    }
    throw new TypeError('invalid catalogue value');
  }
  if (expected.kind === 'enum') {
    if (!expected.values.includes(value as CatalogueJsonPrimitiveV1)) throw new TypeError();
    return value as CatalogueJsonPrimitiveV1;
  }
  if (expected.kind === 'null') {
    if (value !== null) throw new TypeError();
    return null;
  }
  if (expected.kind === 'string' || expected.kind === 'identifier') {
    if (typeof value !== 'string' || !/^[\x20-\x7e]*$/.test(value)) throw new TypeError();
    if (expected.kind === 'identifier' && !/^[A-Za-z_][A-Za-z0-9_$]{0,62}$/.test(value)) {
      throw new TypeError();
    }
    return value;
  }
  if (expected.kind === 'integer') {
    if (typeof value !== 'number' || !Number.isSafeInteger(value) || Object.is(value, -0)) {
      throw new TypeError();
    }
    return value;
  }
  if (expected.kind === 'boolean') {
    if (typeof value !== 'boolean') throw new TypeError();
    return value;
  }
  if (expected.kind === 'array') {
    if (isProxy(value) || !Array.isArray(value)
      || Object.getPrototypeOf(value) !== Array.prototype
      || Reflect.ownKeys(value).length !== value.length + 1) {
      throw new TypeError();
    }
    return Array.from({ length: value.length }, (_, index) => {
      const descriptor = Object.getOwnPropertyDescriptor(value, index);
      if (!descriptor?.enumerable || !('value' in descriptor)) throw new TypeError();
      return reconstruct(descriptor.value, expected.item);
    });
  }
  if (expected.kind !== 'record') throw new TypeError();
  if (isProxy(value) || value === null || typeof value !== 'object' || Array.isArray(value)
    || Object.getPrototypeOf(value) !== Object.prototype) throw new TypeError();
  const source = value as Record<string, unknown>;
  const ownKeys = Reflect.ownKeys(source);
  if (ownKeys.some((key) => typeof key !== 'string')) throw new TypeError();
  const actual = ownKeys as string[];
  const wanted = expected.fields.map(([key]) => key);
  if (actual.length !== wanted.length || actual.some((key, index) => key !== wanted[index])) {
    throw new TypeError();
  }
  return Object.fromEntries(expected.fields.map(([key, child]) => [
    key, reconstruct(dataValue(source, key), child),
  ]));
}

function dataValue(source: Record<string, unknown>, key: string): unknown {
  const descriptor = Object.getOwnPropertyDescriptor(source, key);
  if (!descriptor?.enumerable || !('value' in descriptor)) throw new TypeError();
  return descriptor.value;
}

function deepFreeze<T>(value: T): T {
  if (value !== null && typeof value === 'object' && !Object.isFrozen(value)) {
    for (const nested of Object.values(value as Record<string, unknown>)) deepFreeze(nested);
    Object.freeze(value);
  }
  return value;
}

function shape(kind: 'string' | 'identifier' | 'integer' | 'boolean' | 'null'): Shape {
  return Object.freeze({ kind });
}

function enumeration(...values: readonly CatalogueJsonPrimitiveV1[]): Shape {
  return Object.freeze({ kind: 'enum', values: Object.freeze([...values]) });
}

function array(item: Shape): Shape {
  return Object.freeze({ kind: 'array', item });
}

function record(fields: readonly (readonly [string, Shape])[]): Extract<Shape, { kind: 'record' }> {
  return Object.freeze({ kind: 'record', fields: Object.freeze([...fields]) });
}

function union(...choices: Shape[]): Shape {
  return Object.freeze({ kind: 'union', choices: Object.freeze(choices) });
}
