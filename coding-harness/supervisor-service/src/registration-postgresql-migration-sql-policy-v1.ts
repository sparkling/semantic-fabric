// SPDX-License-Identifier: MIT

import { isProxy } from 'node:util/types';
import {
  deepFreezeV1,
  rawSha256HexV1,
  snapshotBytesV1,
  utf8TextV1,
} from './registration-postgresql-canonical-v1.js';
import {
  countPostgresMigrationWordsV1,
  hasPostgresMigrationWordsV1,
  postgresMigrationSqlInventoryFromTokensV1,
  scanPostgresMigrationSqlTextV1,
  splitPostgresMigrationSqlStatementsV1,
  type PostgresMigrationSqlInventoryV1,
  type PostgresMigrationSqlTokenV1,
} from './registration-postgresql-migration-sql-scanner-v1.js';

export type { PostgresMigrationSqlInventoryV1 }
  from './registration-postgresql-migration-sql-scanner-v1.js';

export type PostgresMigrationVersionV1 = 1 | 2;

const INVALID = 'PostgreSQL migration SQL is invalid';
const MAXIMUM_SQL_BYTES = 1_048_576;
const EXPECTED = Object.freeze({
  1: Object.freeze({
    bytes: 26_438,
    rawSha256: 'c923f0f725c009a65ef85bc1881b7ae5717a1eca148bbf5316aeee60bb4a31c1',
    tokenSha256: '7d9d01b829addebcb6ede5620fab8b2249e3870ae48a840afac9c73668b6be94',
    inventory: Object.freeze({
      statements: 52, createSchemas: 1, createDomains: 10, createTables: 8,
      namedConstraints: 70, checkConstraints: 25, primaryKeys: 8,
      uniqueConstraints: 16, foreignKeys: 21, createIndexes: 0,
      revokeTypeUsage: 8, grants: 0, createPolicies: 0, enableRls: 0, forceRls: 0,
      callableFunctions: Object.freeze(['octet_length', 'scale', 'substring']),
      callableFunctionCounts: Object.freeze({
        octet_length: 14, scale: 1, substring: 1, pg_has_role: 0,
      }),
      escapeByteaLiterals: 2,
    }),
  }),
  2: Object.freeze({
    bytes: 21_661,
    rawSha256: '1d620d95f630997785d0d3adf724e5befe458c0c41e0746f45713cb584b58765',
    tokenSha256: 'a787ab77dff4b1c92962e9c3e453c099f40e4f7e9ce641f37caeecd3de3ee2a4',
    inventory: Object.freeze({
      statements: 73, createSchemas: 0, createDomains: 0, createTables: 0,
      namedConstraints: 0, checkConstraints: 0, primaryKeys: 0,
      uniqueConstraints: 0, foreignKeys: 0, createIndexes: 0,
      revokeTypeUsage: 0, grants: 21, createPolicies: 38, enableRls: 7, forceRls: 7,
      callableFunctions: Object.freeze(['pg_has_role']),
      callableFunctionCounts: Object.freeze({
        octet_length: 0, scale: 0, substring: 0, pg_has_role: 38,
      }),
      escapeByteaLiterals: 0,
    }),
  }),
} as const);

export const POSTGRES_MIGRATION_SQL_PINS_V1 = deepFreezeV1({
  1: {
    bytes: EXPECTED[1].bytes,
    rawSha256: EXPECTED[1].rawSha256,
    tokenSha256: EXPECTED[1].tokenSha256,
  },
  2: {
    bytes: EXPECTED[2].bytes,
    rawSha256: EXPECTED[2].rawSha256,
    tokenSha256: EXPECTED[2].tokenSha256,
  },
});

export interface PostgresMigrationSqlPolicyReportV1 {
  readonly reportKind: 'postgresql-migration-sql-policy-review-v1';
  readonly authority: 'none';
  readonly readinessAuthorized: false;
  readonly executableAuthority: false;
  readonly rawByteLength: number;
  readonly rawSha256: string;
  readonly tokenSha256: string;
  readonly inventory: PostgresMigrationSqlInventoryV1;
}

export interface ParsedPostgresMigrationSqlV1 {
  readonly sqlKind: 'postgresql-migration-sql-v1';
  readonly version: PostgresMigrationVersionV1;
  readonly rawByteLength: number;
  readonly rawSha256: string;
  readonly authority: 'none';
  readonly readinessAuthorized: false;
}

const HANDLES = new WeakMap<object, Readonly<{ bytes: Uint8Array }>>();

/** Review-only lexical and inventory report. It never creates an executable authority. */
export function inspectPostgresMigrationSqlCandidateV1(
  value: unknown,
): PostgresMigrationSqlPolicyReportV1 {
  try {
    const bytes = snapshotBytesV1(value, 'PostgreSQL migration SQL', MAXIMUM_SQL_BYTES);
    const text = utf8TextV1(bytes, 'PostgreSQL migration SQL', MAXIMUM_SQL_BYTES);
    if (!text.endsWith('\n') || text.includes('\r') || text.includes('\0')
      || text.startsWith('\uFEFF') || /[\uD800-\uDFFF]/u.test(text)) throw new TypeError();
    const tokens = scanPostgresMigrationSqlTextV1(text);
    validateClosedConstructs(tokens);
    const inventory = postgresMigrationSqlInventoryFromTokensV1(tokens);
    const tokenSha256 = rawSha256HexV1(`${JSON.stringify(
      tokens.map(({ kind, value }) => [kind, value]),
    )}\n`);
    return deepFreezeV1({
      reportKind: 'postgresql-migration-sql-policy-review-v1' as const,
      authority: 'none' as const,
      readinessAuthorized: false as const,
      executableAuthority: false as const,
      rawByteLength: bytes.byteLength,
      rawSha256: rawSha256HexV1(bytes),
      tokenSha256,
      inventory,
    });
  } catch {
    throw new TypeError(INVALID);
  }
}

/** Closed semantic review policy, independent from the raw-byte digest pin. */
export function assertPostgresMigrationSqlPolicyV1(
  value: unknown,
  version: PostgresMigrationVersionV1,
): PostgresMigrationSqlPolicyReportV1 {
  try {
    const report = inspectPostgresMigrationSqlCandidateV1(value);
    const expected = expectedFor(version);
    if (report.tokenSha256 !== expected.tokenSha256
      || JSON.stringify(report.inventory) !== JSON.stringify(expected.inventory)) {
      throw new TypeError();
    }
    return report;
  } catch {
    throw new TypeError(INVALID);
  }
}

/** Exact raw bytes plus the independent review policy create the private SQL handle. */
export function parsePostgresMigrationSqlV1(
  value: unknown,
  version: PostgresMigrationVersionV1,
): ParsedPostgresMigrationSqlV1 {
  try {
    const expected = expectedFor(version);
    const bytes = snapshotBytesV1(
      value, 'PostgreSQL migration SQL', expected.bytes, expected.bytes,
    );
    if (rawSha256HexV1(bytes) !== expected.rawSha256) throw new TypeError();
    assertPostgresMigrationSqlPolicyV1(bytes, version);
    const handle = Object.freeze({
      sqlKind: 'postgresql-migration-sql-v1' as const,
      version,
      rawByteLength: expected.bytes,
      rawSha256: expected.rawSha256,
      authority: 'none' as const,
      readinessAuthorized: false as const,
    });
    HANDLES.set(handle, Object.freeze({ bytes }));
    return handle;
  } catch {
    throw new TypeError(INVALID);
  }
}

export function assertPostgresMigrationSqlHandleV1(
  value: unknown,
  version?: PostgresMigrationVersionV1,
): asserts value is ParsedPostgresMigrationSqlV1 {
  try {
    if (isProxy(value) || value === null || typeof value !== 'object'
      || !HANDLES.has(value)
      || (version !== undefined && (value as ParsedPostgresMigrationSqlV1).version !== version)) {
      throw new TypeError();
    }
  } catch {
    throw new TypeError(INVALID);
  }
}

export function copyPostgresMigrationSqlBytesV1(value: unknown): Uint8Array {
  assertPostgresMigrationSqlHandleV1(value);
  return Uint8Array.from(HANDLES.get(value)!.bytes);
}

function expectedFor(version: PostgresMigrationVersionV1): typeof EXPECTED[1] | typeof EXPECTED[2] {
  if (version !== 1 && version !== 2) throw new TypeError();
  return EXPECTED[version];
}

function validateClosedConstructs(tokens: readonly PostgresMigrationSqlTokenV1[]): void {
  const statements = splitPostgresMigrationSqlStatementsV1(tokens);
  for (const statement of statements) {
    const words = wordsFrom(statement);
    if (hasPostgresMigrationWordsV1(words, ['IF', 'NOT', 'EXISTS'])
      || hasPostgresMigrationWordsV1(words, ['ON', 'CONFLICT'])
      || hasPostgresMigrationWordsV1(words, ['SECURITY', 'DEFINER'])
      || hasPostgresMigrationWordsV1(words, ['SESSION', 'AUTHORIZATION'])
      || hasPostgresMigrationWordsV1(words, ['SET', 'ROLE'])
      || words.includes('PROGRAM')
      || !isClosedStatement(statement, words)) throw new TypeError();
    validateCallableTokens(statement);
    validateTypeCasts(statement);
  }
  if (tokens.some((token) => token.kind === 'dollar-string'
    || (token.kind === 'escape-string' && !/^\\\\x[0-9a-f]+$/i.test(token.value))
    || (token.kind === 'string' && /^\\x[0-9a-f]+$/i.test(token.value)))) {
    throw new TypeError();
  }
}

function isClosedStatement(
  tokens: readonly PostgresMigrationSqlTokenV1[], words: readonly string[],
): boolean {
  if (words[0] === 'CREATE') return isClosedCreate(tokens, words);
  if (words[0] === 'ALTER') return isClosedAlter(tokens, words);
  if (words[0] === 'GRANT') return isClosedGrant(tokens, words);
  if (words[0] === 'REVOKE') return isClosedRevoke(tokens, words);
  return false;
}

function isClosedCreate(
  tokens: readonly PostgresMigrationSqlTokenV1[], words: readonly string[],
): boolean {
  if (words[1] === 'SCHEMA') return equalWords(words, [
    'CREATE', 'SCHEMA', 'SF_SUPERVISOR_V1', 'AUTHORIZATION', 'SF_SUPERVISOR_OWNER_V1',
  ]);
  if (words[1] === 'DOMAIN') {
    return isQualifiedIdentifierAt(tokens, 2)
      && tokens[5]?.value === 'AS' && tokens[6]?.value === 'PG_CATALOG'
      && tokens[7]?.value === '.' && ['BYTEA', 'NUMERIC', 'TEXT'].includes(tokens[8]?.value ?? '')
      && countPostgresMigrationWordsV1(words, ['CONSTRAINT']) === 1
      && countPostgresMigrationWordsV1(words, ['CHECK']) === 1
      && !hasTopLevelComma(tokens) && !words.includes('PUBLIC');
  }
  if (words[1] === 'TABLE') {
    const forbidden = new Set([
      'AS', 'INHERITS', 'LIKE', 'OF', 'PARTITION', 'TABLESPACE', 'UNLOGGED', 'WITH',
    ]);
    return isQualifiedIdentifierAt(tokens, 2) && tokens[5]?.value === '('
      && tokens.at(-3)?.value === ')' && tokens.at(-3)?.depth === 0
      && tokens.at(-2)?.value === 'USING' && tokens.at(-1)?.value === 'HEAP'
      && countPostgresMigrationWordsV1(words, ['USING', 'HEAP']) === 1
      && !words.some((word) => forbidden.has(word)) && !words.includes('PUBLIC');
  }
  if (words[1] === 'POLICY') {
    const roles = new Set([
      'SF_SUPERVISOR_OWNER_V1', 'SF_SUPERVISOR_READINESS_LOGIN_V1',
      'SF_SUPERVISOR_RECOVERY_LOGIN_V1', 'SF_SUPERVISOR_WRITER_LOGIN_V1',
    ]);
    return tokens[2]?.kind === 'word' && tokens[3]?.value === 'ON'
      && isQualifiedIdentifierAt(tokens, 4)
      && words[3] === 'ON' && words[4] === 'SF_SUPERVISOR_V1'
      && words[6] === 'AS' && ['PERMISSIVE', 'RESTRICTIVE'].includes(words[7] ?? '')
      && words[8] === 'FOR' && ['INSERT', 'SELECT', 'UPDATE'].includes(words[9] ?? '')
      && words[10] === 'TO' && roles.has(words[11] ?? '')
      && ['USING', 'WITH'].includes(words[12] ?? '')
      && !hasTopLevelComma(tokens) && !words.includes('PUBLIC');
  }
  return false;
}

function isClosedAlter(
  tokens: readonly PostgresMigrationSqlTokenV1[], words: readonly string[],
): boolean {
  if (words[1] === 'DEFAULT') return equalWords(words, [
    'ALTER', 'DEFAULT', 'PRIVILEGES', 'FOR', 'ROLE', 'SF_SUPERVISOR_OWNER_V1',
    'REVOKE', 'EXECUTE', 'ON', 'FUNCTIONS', 'FROM', 'PUBLIC',
  ]) || equalWords(words, [
    'ALTER', 'DEFAULT', 'PRIVILEGES', 'FOR', 'ROLE', 'SF_SUPERVISOR_OWNER_V1',
    'REVOKE', 'USAGE', 'ON', 'TYPES', 'FROM', 'PUBLIC',
  ]);
  if (words[1] !== 'TABLE' || words[2] !== 'ONLY' || !isQualifiedIdentifierAt(tokens, 3)
    || hasTopLevelComma(tokens) || words.includes('PUBLIC')) return false;
  const action = words.slice(5);
  if (equalWords(action, ['ENABLE', 'ROW', 'LEVEL', 'SECURITY'])
    || equalWords(action, ['FORCE', 'ROW', 'LEVEL', 'SECURITY'])) return true;
  const deferred = ['DEFERRABLE', 'INITIALLY', 'DEFERRED'];
  return action[0] === 'ADD' && action[1] === 'CONSTRAINT'
    && action[2] !== undefined && action[3] === 'FOREIGN' && action[4] === 'KEY'
    && countPostgresMigrationWordsV1(action, ['REFERENCES', 'SF_SUPERVISOR_V1']) === 1
    && countPostgresMigrationWordsV1(
      action, ['ON', 'UPDATE', 'RESTRICT', 'ON', 'DELETE', 'RESTRICT'],
    ) === 1
    && (equalWords(action.slice(-3), deferred) || action.at(-1) === 'RESTRICT');
}

function isClosedGrant(
  tokens: readonly PostgresMigrationSqlTokenV1[], words: readonly string[],
): boolean {
  if (hasTopLevelComma(tokens) || words.includes('PUBLIC')) return false;
  if (words[1] === 'USAGE') return equalWords(words.slice(0, 6), [
    'GRANT', 'USAGE', 'ON', 'SCHEMA', 'SF_SUPERVISOR_V1', 'TO',
  ]) && words.length === 7;
  if (!['INSERT', 'SELECT', 'UPDATE'].includes(words[1] ?? '')) return false;
  const on = words.indexOf('ON', 2);
  if (on < 2 || words[on + 1] !== 'TABLE' || words[on + 2] !== 'SF_SUPERVISOR_V1'
    || words[on + 3] === undefined || words[on + 4] !== 'TO'
    || words[on + 5] === undefined || words.length !== on + 6) return false;
  const privilegeWords = new Set(['DELETE', 'EXECUTE', 'REFERENCES', 'TRIGGER', 'TRUNCATE']);
  return !words.slice(2, on).some((word) => privilegeWords.has(word));
}

function isClosedRevoke(
  tokens: readonly PostgresMigrationSqlTokenV1[], words: readonly string[],
): boolean {
  if (hasTopLevelComma(tokens) || words.filter((word) => word === 'PUBLIC').length !== 1
    || words.at(-2) !== 'FROM' || words.at(-1) !== 'PUBLIC') return false;
  if (equalWords(words, [
    'REVOKE', 'ALL', 'PRIVILEGES', 'ON', 'SCHEMA', 'SF_SUPERVISOR_V1', 'FROM', 'PUBLIC',
  ]) || equalWords(words, [
    'REVOKE', 'ALL', 'PRIVILEGES', 'ON', 'ALL', 'TABLES', 'IN', 'SCHEMA',
    'SF_SUPERVISOR_V1', 'FROM', 'PUBLIC',
  ])) return true;
  return words.length === 8 && words[1] === 'USAGE' && words[2] === 'ON'
    && words[3] === 'TYPE' && words[4] === 'SF_SUPERVISOR_V1'
    && words[5] !== undefined;
}

function validateTypeCasts(tokens: readonly PostgresMigrationSqlTokenV1[]): void {
  const types = new Set(['BYTEA', 'NAME', 'NUMERIC', 'TEXT']);
  for (let index = 0; index < tokens.length; index += 1) {
    if (tokens[index]?.value !== ':' || tokens[index + 1]?.value !== ':') continue;
    if (tokens[index + 2]?.value !== 'PG_CATALOG' || tokens[index + 3]?.value !== '.'
      || tokens[index + 4]?.kind !== 'word' || !types.has(tokens[index + 4]!.value)) {
      throw new TypeError();
    }
  }
}

function validateCallableTokens(tokens: readonly PostgresMigrationSqlTokenV1[]): void {
  const grouping = new Set(['AND', 'CHECK', 'INSERT', 'KEY', 'OR', 'SELECT', 'UNIQUE', 'UPDATE', 'USING']);
  const callables = new Set(['OCTET_LENGTH', 'PG_HAS_ROLE', 'SCALE', 'SUBSTRING']);
  for (let index = 0; index + 1 < tokens.length; index += 1) {
    const current = tokens[index];
    if (tokens[index + 1]?.value !== '(') continue;
    if (current?.kind === 'quoted-identifier') throw new TypeError();
    if (current?.kind !== 'word') continue;
    if (tokens[index - 1]?.value !== '.') {
      if (!grouping.has(current.value)) throw new TypeError();
      continue;
    }
    const schema = tokens[index - 2];
    const context = tokens[index - 3];
    if (schema?.value === 'PG_CATALOG') {
      if (!callables.has(current.value)) throw new TypeError();
    } else if (schema?.value !== 'SF_SUPERVISOR_V1'
      || !['REFERENCES', 'TABLE'].includes(context?.value ?? '')) throw new TypeError();
  }
}

function isQualifiedIdentifierAt(
  tokens: readonly PostgresMigrationSqlTokenV1[], index: number,
): boolean {
  return tokens[index]?.value === 'SF_SUPERVISOR_V1' && tokens[index + 1]?.value === '.'
    && tokens[index + 2]?.kind === 'word';
}

function hasTopLevelComma(tokens: readonly PostgresMigrationSqlTokenV1[]): boolean {
  return tokens.some((token) => token.kind === 'punct'
    && token.value === ',' && token.depth === 0);
}

function wordsFrom(tokens: readonly PostgresMigrationSqlTokenV1[]): readonly string[] {
  return tokens.filter((token) => token.kind === 'word').map((token) => token.value);
}

function equalWords(actual: readonly string[], expected: readonly string[]): boolean {
  return actual.length === expected.length
    && expected.every((word, index) => actual[index] === word);
}
