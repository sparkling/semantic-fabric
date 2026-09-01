// SPDX-License-Identifier: MIT

import {
  copyPostgresMigrationCommandCatalogueV1,
  type PostgresMigrationCommandDescriptorV1,
} from './registration-postgresql-migration-command-catalogue-v1.js';
import { postgresMigrationPlanAuthoritiesV1 }
  from './registration-postgresql-migration-plan-v1.js';
import type { CatalogueRecordV1 }
  from './registration-postgresql-catalogue-shape-v1.js';

const INVALID = 'PostgreSQL migration command DDL is invalid';
const EXPECTED_INSERT_COMMANDS = 4;
const APPLY = Reflect.apply;
const ARRAY_IS_ARRAY = Array.isArray;
const ARRAY_PUSH = Array.prototype.push;
const ARRAY_SORT = Array.prototype.sort;
const NUMBER = Number;
const NUMBER_IS_SAFE_INTEGER = Number.isSafeInteger;
const REGEXP_EXEC = RegExp.prototype.exec;
const STRING_SPLIT = String.prototype.split;
const INSERT_PATTERN = /^INSERT INTO ([a-z_][a-z0-9_$]*)\.([a-z_][a-z0-9_$]*) \(\n([\s\S]*?)\n\) VALUES \(\n([\s\S]*?)\n\)\n$/;
const COLUMN_PATTERN = /^  ([a-z_][a-z0-9_$]*)$/;
const CAST_PATTERN = /^  \$([1-9][0-9]*)::pg_catalog\.([a-z_][a-z0-9_$]*)$/;

interface InsertShapeV1 {
  readonly schema: string;
  readonly relation: string;
  readonly columns: readonly string[];
  readonly casts: readonly string[];
}

/** Private static evidence check; it performs no I/O and grants no authority. */
export function assertPostgresMigrationCommandDdlV1(plan: unknown): void {
  try {
    const authorities = postgresMigrationPlanAuthoritiesV1(plan);
    const contract = authorities.catalogueContract.contract;
    const catalogue = copyPostgresMigrationCommandCatalogueV1(plan);
    if (!ARRAY_IS_ARRAY(catalogue.insertCommands)
      || catalogue.insertCommands.length !== EXPECTED_INSERT_COMMANDS) throw new TypeError();
    const operations = new Set<string>();
    for (let index = 0; index < catalogue.insertCommands.length; index += 1) {
      const command = catalogue.insertCommands[index]!;
      if (typeof command.operation !== 'string' || operations.has(command.operation)) {
        throw new TypeError();
      }
      operations.add(command.operation);
      assertInsertV1(contract, command);
    }
    if (operations.size !== EXPECTED_INSERT_COMMANDS) throw new TypeError();
  } catch {
    throw new TypeError(INVALID);
  }
}

function assertInsertV1(
  contract: CatalogueRecordV1,
  command: PostgresMigrationCommandDescriptorV1,
): void {
  if (command.descriptorKind !== 'postgresql-migration-command-descriptor-v1'
    || !ARRAY_IS_ARRAY(command.parameters)) throw new TypeError();
  const shape = parseInsertV1(command.text);
  if (!hasOneRelationV1(contract, shape.schema, shape.relation)) throw new TypeError();
  const columns = relationColumnsV1(contract, shape.schema, shape.relation);
  if (columns.length === 0 || shape.columns.length !== columns.length
    || shape.casts.length !== columns.length
    || command.parameters.length !== columns.length) throw new TypeError();
  for (let index = 0; index < columns.length; index += 1) {
    const column = columns[index]!;
    const baseType = resolvedBaseTypeV1(contract, column);
    const parameter = command.parameters[index]!;
    if (integerV1(column, 'ordinal') !== index + 1
      || textV1(column, 'name') !== shape.columns[index]
      || shape.casts[index] !== baseType
      || parameter.position !== index + 1
      || parameter.baseType !== baseType) throw new TypeError();
  }
}

function parseInsertV1(text: unknown): InsertShapeV1 {
  if (typeof text !== 'string') throw new TypeError();
  const match = APPLY(REGEXP_EXEC, INSERT_PATTERN, [text]) as RegExpExecArray | null;
  if (match === null) throw new TypeError();
  const columnLines = splitV1(match[3]!, ',\n');
  const castLines = splitV1(match[4]!, ',\n');
  const columns: string[] = [];
  const casts: string[] = [];
  for (let index = 0; index < columnLines.length; index += 1) {
    const column = APPLY(
      REGEXP_EXEC, COLUMN_PATTERN, [columnLines[index]!],
    ) as RegExpExecArray | null;
    if (column === null) throw new TypeError();
    APPLY(ARRAY_PUSH, columns, [column[1]!]);
  }
  for (let index = 0; index < castLines.length; index += 1) {
    const cast = APPLY(
      REGEXP_EXEC, CAST_PATTERN, [castLines[index]!],
    ) as RegExpExecArray | null;
    if (cast === null || NUMBER(cast[1]) !== index + 1) throw new TypeError();
    APPLY(ARRAY_PUSH, casts, [cast[2]!]);
  }
  return {
    schema: match[1]!, relation: match[2]!, columns, casts,
  };
}

function hasOneRelationV1(
  contract: CatalogueRecordV1, schema: string, relation: string,
): boolean {
  const relations = recordsV1(contract.relations);
  let matches = 0;
  for (let index = 0; index < relations.length; index += 1) {
    const candidate = relations[index]!;
    if (candidate.schema === schema && candidate.name === relation
      && candidate.kind === 'table') matches += 1;
  }
  return matches === 1;
}

function relationColumnsV1(
  contract: CatalogueRecordV1, schema: string, relation: string,
): CatalogueRecordV1[] {
  const all = recordsV1(contract.columns);
  const columns: CatalogueRecordV1[] = [];
  for (let index = 0; index < all.length; index += 1) {
    const column = all[index]!;
    if (column.schema === schema && column.relation === relation) {
      APPLY(ARRAY_PUSH, columns, [column]);
    }
  }
  APPLY(ARRAY_SORT, columns, [
    (left: CatalogueRecordV1, right: CatalogueRecordV1) =>
      integerV1(left, 'ordinal') - integerV1(right, 'ordinal'),
  ]);
  return columns;
}

function resolvedBaseTypeV1(
  contract: CatalogueRecordV1, column: CatalogueRecordV1,
): string {
  const typeSchema = textV1(column, 'typeSchema');
  const typeName = textV1(column, 'typeName');
  if (typeSchema === 'pg_catalog') return typeName;
  const domains = recordsV1(contract.domains);
  let baseType: string | undefined;
  for (let index = 0; index < domains.length; index += 1) {
    const domain = domains[index]!;
    if (domain.schema !== typeSchema || domain.name !== typeName) continue;
    if (baseType !== undefined || domain.baseTypeSchema !== 'pg_catalog') throw new TypeError();
    baseType = textV1(domain, 'baseTypeName');
  }
  if (baseType === undefined) throw new TypeError();
  return baseType;
}

function recordsV1(value: unknown): readonly CatalogueRecordV1[] {
  if (!ARRAY_IS_ARRAY(value)) throw new TypeError();
  return value as readonly CatalogueRecordV1[];
}

function textV1(value: CatalogueRecordV1, key: string): string {
  const item = value[key];
  if (typeof item !== 'string') throw new TypeError();
  return item;
}

function integerV1(value: CatalogueRecordV1, key: string): number {
  const item = value[key];
  if (!NUMBER_IS_SAFE_INTEGER(item)) throw new TypeError();
  return item as number;
}

function splitV1(value: string, separator: string): string[] {
  return APPLY(STRING_SPLIT, value, [separator]) as string[];
}
