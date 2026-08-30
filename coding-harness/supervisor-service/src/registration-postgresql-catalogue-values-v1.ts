// SPDX-License-Identifier: MIT

import type {
  CatalogueJsonValueV1,
  CatalogueRecordV1,
} from './registration-postgresql-catalogue-shape-v1.js';

export function catalogueRecordV1(value: CatalogueJsonValueV1, label: string): CatalogueRecordV1 {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) invalid(label);
  return value as CatalogueRecordV1;
}

export function catalogueArrayV1(
  value: CatalogueJsonValueV1,
  label: string,
): readonly CatalogueJsonValueV1[] {
  if (!Array.isArray(value)) invalid(label);
  return value as readonly CatalogueJsonValueV1[];
}

export function catalogueStringV1(value: CatalogueJsonValueV1, label: string): string {
  if (typeof value !== 'string') invalid(label);
  return value as string;
}

export function catalogueNumberV1(value: CatalogueJsonValueV1, label: string): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value)) invalid(label);
  return value as number;
}

export function catalogueBooleanV1(value: CatalogueJsonValueV1, label: string): boolean {
  if (typeof value !== 'boolean') invalid(label);
  return value as boolean;
}

export function recordsV1(root: CatalogueRecordV1, key: string): CatalogueRecordV1[] {
  return catalogueArrayV1(root[key]!, key).map((value) => catalogueRecordV1(value, key));
}

export function stringsV1(value: CatalogueJsonValueV1, label: string): string[] {
  return catalogueArrayV1(value, label).map((item) => catalogueStringV1(item, label));
}

export function requireV1(condition: unknown, _label: string): asserts condition {
  if (!condition) invalid(_label);
}

export function requireEqualJsonV1(
  actual: CatalogueJsonValueV1,
  expected: CatalogueJsonValueV1,
  label: string,
): void {
  requireV1(JSON.stringify(actual) === JSON.stringify(expected), label);
}

export function requireSortedRecordsV1(
  values: readonly CatalogueRecordV1[],
  identity: (value: CatalogueRecordV1) => readonly (string | number | boolean | null)[],
  label: string,
): void {
  let previous: readonly (string | number | boolean | null)[] | undefined;
  for (const value of values) {
    const current = identity(value);
    if (previous !== undefined) requireV1(compareTuple(previous, current) < 0, label);
    previous = current;
  }
}

export function canonicalRootLineJsonV1(
  value: CatalogueRecordV1,
  rootKeys: readonly string[],
): string {
  return `{\n${rootKeys.map((key, index) => {
    const comma = index === rootKeys.length - 1 ? '' : ',';
    return `  ${JSON.stringify(key)}: ${JSON.stringify(value[key])}${comma}\n`;
  }).join('')}}\n`;
}

function compareTuple(
  left: readonly (string | number | boolean | null)[],
  right: readonly (string | number | boolean | null)[],
): number {
  const length = Math.min(left.length, right.length);
  for (let index = 0; index < length; index += 1) {
    const compared = compareScalar(left[index]!, right[index]!);
    if (compared !== 0) return compared;
  }
  return Math.sign(left.length - right.length);
}

function compareScalar(
  left: string | number | boolean | null,
  right: string | number | boolean | null,
): number {
  if (left === right) return 0;
  const rank = (value: typeof left): number => {
    if (value === null) return 0;
    if (typeof value === 'boolean') return 1;
    if (typeof value === 'number') return 2;
    return 3;
  };
  const rankDelta = rank(left) - rank(right);
  if (rankDelta !== 0) return Math.sign(rankDelta);
  if (typeof left === 'boolean' && typeof right === 'boolean') return left ? 1 : -1;
  if (typeof left === 'number' && typeof right === 'number') return left < right ? -1 : 1;
  if (typeof left === 'string' && typeof right === 'string') return left < right ? -1 : 1;
  throw new TypeError('PostgreSQL catalogue contract is invalid');
}

function invalid(_label: string): never {
  throw new TypeError('PostgreSQL catalogue contract is invalid');
}
