// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { isProxy } from 'node:util/types';
import {
  POSTGRES_PUBLIC_ACL_FIXTURE_LIMITS_V1,
  scanPostgresPublicAclFixtureBytesV1,
  snapshotPostgresPublicAclFixtureBytesV1,
  type PostgresPublicAclFixtureScanV1,
} from './registration-postgresql-public-acl-fixture-scanner-v1.js';

const INVALID = 'PostgreSQL PUBLIC ACL fixture is invalid';
const KEYS = Object.freeze([
  'objectClass', 'schemaName', 'objectName', 'subobjectName', 'objectKind',
  'routineIdentityArguments', 'privilege', 'grantable',
] as const);
const KINDS = Object.freeze({
  schema: Object.freeze(['schema']),
  relation: Object.freeze([
    'table', 'partitioned-table', 'view', 'materialized-view', 'foreign-table', 'sequence',
  ]),
  column: Object.freeze([
    'table', 'partitioned-table', 'view', 'materialized-view', 'foreign-table',
  ]),
  routine: Object.freeze(['function', 'procedure', 'aggregate', 'window-function']),
  type: Object.freeze([
    'base', 'composite', 'domain', 'enum', 'pseudo', 'range', 'multirange', 'array',
  ]),
  language: Object.freeze(['language']),
  'foreign-data-wrapper': Object.freeze(['foreign-data-wrapper']),
  'foreign-server': Object.freeze(['foreign-server']),
} as const);
const TABLE_PRIVILEGES = Object.freeze([
  'SELECT', 'INSERT', 'UPDATE', 'DELETE', 'TRUNCATE', 'REFERENCES', 'TRIGGER',
]);
const PRIVATE_BYTES = new WeakMap<object, Uint8Array>();

export interface PostgresPublicAclRecordV1 {
  readonly objectClass: keyof typeof KINDS;
  readonly schemaName: string | null;
  readonly objectName: string;
  readonly subobjectName: string | null;
  readonly objectKind: string;
  readonly routineIdentityArguments: string | null;
  readonly privilege: string;
  readonly grantable: false;
}

export interface ParsedPostgresPublicAclFixtureV1 {
  readonly rawByteLength: number;
  readonly rawSha256: string;
  readonly scan: Readonly<Omit<PostgresPublicAclFixtureScanV1, 'text'>>;
  readonly records: readonly PostgresPublicAclRecordV1[];
  readonly authority: 'none';
  readonly readinessAuthorized: false;
}

/** Parse the protected test oracle without creating runtime or migration authority. */
export function parsePostgresPublicAclFixtureV1(
  value: unknown,
): ParsedPostgresPublicAclFixtureV1 {
  try {
    const snapshot = snapshotPostgresPublicAclFixtureBytesV1(value);
    const scanned = scanPostgresPublicAclFixtureBytesV1(snapshot);
    let parsed: unknown;
    try { parsed = JSON.parse(scanned.text) as unknown; }
    catch { throw new TypeError(); }
    const records = reconstructRecords(parsed);
    if (scanned.records !== records.length || scanned.nodes !== 1 + (9 * records.length)) {
      throw new TypeError();
    }
    const replay = `${JSON.stringify(records)}\n`;
    if (replay !== scanned.text) throw new TypeError();
    const handle: ParsedPostgresPublicAclFixtureV1 = Object.freeze({
      rawByteLength: snapshot.byteLength,
      rawSha256: createHash('sha256').update(snapshot).digest('hex'),
      scan: Object.freeze({
        nodes: scanned.nodes,
        records: scanned.records,
        maximumDepth: scanned.maximumDepth,
      }),
      records,
      authority: 'none',
      readinessAuthorized: false,
    });
    PRIVATE_BYTES.set(handle, snapshot);
    return handle;
  } catch {
    throw new TypeError(INVALID);
  }
}

/** Return a fresh copy only for a handle minted by this test-only module. */
export function copyPostgresPublicAclFixtureBytesV1(value: unknown): Uint8Array {
  try {
    if (isProxy(value) || value === null || typeof value !== 'object'
      || !PRIVATE_BYTES.has(value)) {
      throw new TypeError();
    }
    return new Uint8Array(PRIVATE_BYTES.get(value)!);
  } catch {
    throw new TypeError(INVALID);
  }
}

function reconstructRecords(value: unknown): readonly PostgresPublicAclRecordV1[] {
  if (!Array.isArray(value) || Object.getPrototypeOf(value) !== Array.prototype
    || Reflect.ownKeys(value).length !== value.length + 1
    || value.length > POSTGRES_PUBLIC_ACL_FIXTURE_LIMITS_V1.maximumRootWidth) {
    throw new TypeError();
  }
  const records = value.map((entry) => reconstructRecord(entry));
  for (let index = 1; index < records.length; index += 1) {
    if (compareRecords(records[index - 1]!, records[index]!) >= 0) throw new TypeError();
  }
  return Object.freeze(records);
}

function reconstructRecord(value: unknown): PostgresPublicAclRecordV1 {
  if (value === null || typeof value !== 'object' || Array.isArray(value)
    || Object.getPrototypeOf(value) !== Object.prototype
    || !exactDataKeys(value, KEYS)) {
    throw new TypeError();
  }
  const input = value as Record<string, unknown>;
  const objectClass = input.objectClass;
  if (typeof objectClass !== 'string' || !hasOwn(KINDS, objectClass)) throw new TypeError();
  const objectKind = boundedText(input.objectKind, false);
  if (!(KINDS[objectClass] as readonly string[]).includes(objectKind)) throw new TypeError();
  const schemaBound = ['column', 'relation', 'routine', 'type'].includes(objectClass);
  const schemaName = schemaBound
    ? identifier(input.schemaName) : exactNull(input.schemaName);
  const subobjectName = objectClass === 'column'
    ? identifier(input.subobjectName) : exactNull(input.subobjectName);
  const routineIdentityArguments = objectClass === 'routine'
    ? boundedText(input.routineIdentityArguments, true) : exactNull(input.routineIdentityArguments);
  const privilege = boundedText(input.privilege, false);
  if (!privileges(objectClass, objectKind).includes(privilege)
    || input.grantable !== false) {
    throw new TypeError();
  }
  return Object.freeze({
    objectClass,
    schemaName,
    objectName: identifier(input.objectName),
    subobjectName,
    objectKind,
    routineIdentityArguments,
    privilege,
    grantable: false,
  });
}

function exactDataKeys(value: object, expected: readonly string[]): boolean {
  const keys = Reflect.ownKeys(value);
  if (keys.length !== expected.length || keys.some((key, index) => key !== expected[index])) {
    return false;
  }
  return keys.every((key) => {
    const descriptor = Object.getOwnPropertyDescriptor(value, key);
    return descriptor?.enumerable === true && 'value' in descriptor;
  });
}

function privileges(objectClass: keyof typeof KINDS, objectKind: string): readonly string[] {
  if (objectClass === 'schema') return ['CREATE', 'USAGE'];
  if (objectClass === 'relation') {
    return objectKind === 'sequence' ? ['SELECT', 'UPDATE', 'USAGE'] : TABLE_PRIVILEGES;
  }
  if (objectClass === 'column') return ['INSERT', 'SELECT', 'UPDATE', 'REFERENCES'];
  if (objectClass === 'routine') return ['EXECUTE'];
  return ['USAGE'];
}

function identifier(value: unknown): string {
  const result = boundedText(value, false);
  if (new TextEncoder().encode(result).byteLength > 63) throw new TypeError();
  return result;
}

function boundedText(value: unknown, allowEmpty: boolean): string {
  if (typeof value !== 'string' || (!allowEmpty && value.length === 0)
    || value.includes('\0')
    || new TextEncoder().encode(value).byteLength
      > POSTGRES_PUBLIC_ACL_FIXTURE_LIMITS_V1.maximumStringBytes) {
    throw new TypeError();
  }
  return value;
}

function exactNull(value: unknown): null {
  if (value !== null) throw new TypeError();
  return null;
}

function compareRecords(
  left: PostgresPublicAclRecordV1,
  right: PostgresPublicAclRecordV1,
): number {
  for (const key of KEYS) {
    const a = left[key];
    const b = right[key];
    if (a === b) continue;
    if (a === null) return -1;
    if (b === null) return 1;
    if (typeof a === 'boolean') return a ? 1 : -1;
    const order = compareBytes(new TextEncoder().encode(a), new TextEncoder().encode(b as string));
    if (order !== 0) return order;
  }
  return 0;
}

function compareBytes(left: Uint8Array, right: Uint8Array): number {
  const width = Math.min(left.length, right.length);
  for (let index = 0; index < width; index += 1) {
    if (left[index] !== right[index]) return left[index]! - right[index]!;
  }
  return left.length - right.length;
}

function hasOwn(value: object, key: PropertyKey): key is keyof typeof KINDS {
  return Object.prototype.hasOwnProperty.call(value, key);
}
