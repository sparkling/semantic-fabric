// SPDX-License-Identifier: MIT

import type { CatalogueRecordV1 }
  from './registration-postgresql-catalogue-shape-v1.js';
import { POSTGRES_EXACT_RESULT_RAW_ROW_KEYS_V1 }
  from './registration-postgresql-row-codecs-v1.js';
import {
  catalogueArrayV1,
  catalogueNumberV1,
  catalogueRecordV1,
  catalogueStringV1,
  recordsV1,
  requireV1,
} from './registration-postgresql-catalogue-values-v1.js';

const SCHEMA = 'sf_supervisor_v1';
const SCOPE = ['project_authority_digest', 'project_scope_role'] as const;
const JOIN_EDGES = Object.freeze([
  Object.freeze({
    leftAlias: 'result', rightRelation: 'semantic_events', rightAlias: 'current_event',
    leftColumn: 'current_event_digest', rightColumn: 'event_digest',
  }),
  Object.freeze({
    leftAlias: 'result', rightRelation: 'registration_runs', rightAlias: 'run',
    leftColumn: 'run_id', rightColumn: 'run_id',
  }),
  Object.freeze({
    leftAlias: 'run', rightRelation: 'semantic_events', rightAlias: 'original_event',
    leftColumn: 'original_registration_event_digest', rightColumn: 'event_digest',
  }),
]);

export function validatePostgresCatalogueQueryV1(root: CatalogueRecordV1): void {
  const queries = recordsV1(root, 'exactQueries');
  requireV1(queries.length === 1, 'exact query');
  const query = queries[0]!;
  requireV1(text(query, 'name') === 'registration-exact-result-v1'
    && number(query, 'maximumRows') === 2, 'query identity');
  const relations = new Set(recordsV1(root, 'relations').map((value) => text(value, 'name')));
  const columnRecords = recordsV1(root, 'columns');
  const columns = new Map(columnRecords.map((value) => [
    `${text(value, 'relation')}.${text(value, 'name')}`, value,
  ]));
  const queryRoot = record(query, 'root');
  const rootRelation = text(queryRoot, 'relation');
  const rootAlias = text(queryRoot, 'alias');
  requireV1(text(queryRoot, 'schema') === SCHEMA
    && rootRelation === 'registration_results' && rootAlias === 'result'
    && relations.has(rootRelation), 'query root');

  const aliases = new Map<string, string>([[rootAlias, rootRelation]]);
  const parameters = arrayRecords(query, 'parameters');
  requireV1(parameters.length === 2, 'query parameters');
  const expectedParameters = ['project_authority_digest', 'semantic_request_digest'];
  parameters.forEach((value, index) => requireV1(number(value, 'position') === index + 1
    && text(value, 'name') === expectedParameters[index]
    && text(value, 'baseType') === 'bytea', 'query parameter order'));

  const joins = arrayRecords(query, 'joins');
  requireV1(joins.length === 3, 'query joins');
  joins.forEach((join, index) => {
    const expected = JOIN_EDGES[index]!;
    const leftAlias = text(join, 'leftAlias');
    const rightAlias = text(join, 'rightAlias');
    const rightRelation = text(join, 'rightRelation');
    requireV1(text(join, 'kind') === 'left' && leftAlias === expected.leftAlias
      && rightAlias === expected.rightAlias && rightRelation === expected.rightRelation
      && aliases.has(leftAlias)
      && !aliases.has(rightAlias) && text(join, 'rightSchema') === SCHEMA
      && relations.has(rightRelation), 'query join');
    const pairs = arrayRecords(join, 'columnPairs');
    requireV1(pairs.length === 3
      && text(pairs[0]!, 'leftColumn') === SCOPE[0]
      && text(pairs[0]!, 'rightColumn') === SCOPE[0]
      && text(pairs[1]!, 'leftColumn') === SCOPE[1]
      && text(pairs[1]!, 'rightColumn') === SCOPE[1]
      && text(pairs[2]!, 'leftColumn') === expected.leftColumn
      && text(pairs[2]!, 'rightColumn') === expected.rightColumn, 'query scope join');
    const leftRelation = aliases.get(leftAlias)!;
    requireV1(pairs.every((pair) => columns.has(`${leftRelation}.${text(pair, 'leftColumn')}`)
      && columns.has(`${rightRelation}.${text(pair, 'rightColumn')}`)), 'query join columns');
    aliases.set(rightAlias, rightRelation);
  });

  const predicates = arrayRecords(query, 'predicates');
  requireV1(predicates.length === 3, 'query predicates');
  const usedParameters = new Set<number>();
  let literalScope = 0;
  const expectedPredicates = Object.freeze([
    Object.freeze({ column: SCOPE[0], operandKind: 'parameter', operand: 1 }),
    Object.freeze({
      column: SCOPE[1], operandKind: 'literal',
      operand: 'sf_supervisor_project_scope_v1',
    }),
    Object.freeze({ column: 'semantic_request_digest', operandKind: 'parameter', operand: 2 }),
  ]);
  predicates.forEach((predicate, index) => {
    const expected = expectedPredicates[index]!;
    const alias = text(predicate, 'sourceAlias');
    const column = text(predicate, 'column');
    requireV1(alias === rootAlias && columns.has(`${rootRelation}.${column}`)
      && text(predicate, 'operator') === 'equals' && column === expected.column
      && text(predicate, 'operandKind') === expected.operandKind
      && predicate.operand === expected.operand, 'query predicate');
    if (text(predicate, 'operandKind') === 'parameter') {
      const position = number(predicate, 'operand');
      const parameter = parameters[position - 1];
      requireV1(parameter !== undefined && text(parameter, 'name') === column
        && !usedParameters.has(position), 'query parameter binding');
      usedParameters.add(position);
    } else {
      requireV1(column === SCOPE[1]
        && text(predicate, 'operand') === 'sf_supervisor_project_scope_v1', 'query literal');
      literalScope += 1;
    }
  });
  requireV1(usedParameters.size === 2 && literalScope === 1, 'query predicate closure');

  const projection = arrayRecords(query, 'projection');
  requireV1(projection.length === POSTGRES_EXACT_RESULT_RAW_ROW_KEYS_V1.length,
    'query projection');
  projection.forEach((value, index) => {
    const alias = text(value, 'sourceAlias');
    const relation = aliases.get(alias);
    const column = relation === undefined
      ? undefined : columns.get(`${relation}.${text(value, 'column')}`);
    requireV1(column !== undefined
      && text(value, 'cast') === text(column!, 'baseProjectionType')
      && text(value, 'outputAlias') === projectionAliasV1(
        alias, text(value, 'column'), text(value, 'cast'),
      )
      && text(value, 'outputAlias') === POSTGRES_EXACT_RESULT_RAW_ROW_KEYS_V1[index],
    'query projection binding');
  });
  requireV1(new Set(projection.map((value) => text(value, 'outputAlias'))).size === 27,
    'query aliases');
}

function projectionAliasV1(sourceAlias: string, column: string, cast: string): string {
  if (sourceAlias.endsWith('_event') && column.startsWith('event_')) {
    return `${sourceAlias.slice(0, -'_event'.length)}_${column}`;
  }
  const textSuffix = cast === 'text'
    && ((sourceAlias === 'result' && column === 'response_status')
      || (sourceAlias === 'original_event' && column === 'global_sequence'))
    ? '_text' : '';
  return `${sourceAlias}_${column}${textSuffix}`;
}

function text(value: CatalogueRecordV1, key: string): string {
  return catalogueStringV1(value[key]!, key);
}
function number(value: CatalogueRecordV1, key: string): number {
  return catalogueNumberV1(value[key]!, key);
}
function record(value: CatalogueRecordV1, key: string): CatalogueRecordV1 {
  return catalogueRecordV1(value[key]!, key);
}
function arrayRecords(value: CatalogueRecordV1, key: string): CatalogueRecordV1[] {
  return catalogueArrayV1(value[key]!, key).map((item) => catalogueRecordV1(item, key));
}
