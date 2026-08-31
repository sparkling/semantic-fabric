// SPDX-License-Identifier: MIT

import { deepFreezeV1 } from './registration-postgresql-canonical-v1.js';

const MAXIMUM_TOKENS = 65_536;
const MAXIMUM_COMMENT_DEPTH = 32;
const MAXIMUM_QUOTED_VALUE_BYTES = 196_608;
const UTF8_ENCODER = new TextEncoder();

export interface PostgresMigrationSqlTokenV1 {
  readonly kind: 'word' | 'number' | 'string' | 'escape-string'
    | 'quoted-identifier' | 'dollar-string' | 'punct';
  readonly value: string;
  readonly depth: number;
}

export interface PostgresMigrationSqlInventoryV1 {
  readonly statements: number;
  readonly createSchemas: number;
  readonly createDomains: number;
  readonly createTables: number;
  readonly namedConstraints: number;
  readonly checkConstraints: number;
  readonly primaryKeys: number;
  readonly uniqueConstraints: number;
  readonly foreignKeys: number;
  readonly createIndexes: number;
  readonly revokeTypeUsage: number;
  readonly grants: number;
  readonly createPolicies: number;
  readonly enableRls: number;
  readonly forceRls: number;
  readonly callableFunctions: readonly string[];
  readonly callableFunctionCounts: Readonly<{
    readonly octet_length: number;
    readonly scale: number;
    readonly substring: number;
    readonly pg_has_role: number;
  }>;
  readonly escapeByteaLiterals: number;
}

export function scanPostgresMigrationSqlTextV1(
  source: string,
): readonly PostgresMigrationSqlTokenV1[] {
  const tokens: PostgresMigrationSqlTokenV1[] = [];
  let cursor = 0;
  let depth = 0;
  const add = (
    kind: PostgresMigrationSqlTokenV1['kind'], value: string, tokenDepth = depth,
  ): void => {
    if (tokens.length >= MAXIMUM_TOKENS) throw new TypeError();
    tokens.push(Object.freeze({ kind, value, depth: tokenDepth }));
  };
  while (cursor < source.length) {
    const character = source[cursor]!;
    if (/\s/u.test(character)) { cursor += 1; continue; }
    if (source.startsWith('--', cursor)) {
      const newline = source.indexOf('\n', cursor + 2);
      cursor = newline === -1 ? source.length : newline;
      continue;
    }
    if (source.startsWith('/*', cursor)) {
      cursor = skipBlockComment(source, cursor);
      continue;
    }
    if ((character === 'E' || character === 'e') && source[cursor + 1] === "'") {
      const result = quoted(source, cursor + 1, "'", true);
      add('escape-string', result.value);
      cursor = result.end;
      continue;
    }
    if (character === "'") {
      const result = quoted(source, cursor, "'", false);
      add('string', result.value);
      cursor = result.end;
      continue;
    }
    if (character === '"') {
      const result = quoted(source, cursor, '"', false);
      add('quoted-identifier', result.value);
      cursor = result.end;
      continue;
    }
    if (character === '$') {
      const delimiter = source.slice(cursor).match(/^\$(?:[A-Za-z_][A-Za-z0-9_]*)?\$/u)?.[0];
      if (delimiter !== undefined) {
        const bodyStart = cursor + delimiter.length;
        const close = source.indexOf(delimiter, bodyStart);
        if (close < 0) throw new TypeError();
        if (close - bodyStart > MAXIMUM_QUOTED_VALUE_BYTES) throw new TypeError();
        const value = source.slice(bodyStart, close);
        if (UTF8_ENCODER.encode(value).byteLength > MAXIMUM_QUOTED_VALUE_BYTES) {
          throw new TypeError();
        }
        add('dollar-string', value);
        cursor = close + delimiter.length;
        continue;
      }
    }
    const word = source.slice(cursor).match(/^[A-Za-z_][A-Za-z0-9_$]*/u)?.[0];
    if (word !== undefined) {
      add('word', word.toUpperCase());
      cursor += word.length;
      continue;
    }
    const number = source.slice(cursor).match(/^[0-9]+(?:\.[0-9]+)?/u)?.[0];
    if (number !== undefined) { add('number', number); cursor += number.length; continue; }
    if (character.charCodeAt(0) > 0x7f || character === '\\') throw new TypeError();
    if (character === '(') { add('punct', character); depth += 1; cursor += 1; continue; }
    if (character === ')') {
      if (depth === 0) throw new TypeError();
      depth -= 1;
      add('punct', character);
      cursor += 1;
      continue;
    }
    add('punct', character);
    cursor += 1;
  }
  if (depth !== 0 || tokens.length === 0) throw new TypeError();
  return Object.freeze(tokens);
}

export function splitPostgresMigrationSqlStatementsV1(
  tokens: readonly PostgresMigrationSqlTokenV1[],
): readonly (readonly PostgresMigrationSqlTokenV1[])[] {
  const output: PostgresMigrationSqlTokenV1[][] = [];
  let current: PostgresMigrationSqlTokenV1[] = [];
  for (const token of tokens) {
    if (token.kind === 'punct' && token.value === ';' && token.depth === 0) {
      if (current.length === 0) throw new TypeError();
      output.push(current);
      current = [];
    } else current.push(token);
  }
  if (current.length !== 0 || output.length === 0) throw new TypeError();
  return Object.freeze(output.map((statement) => Object.freeze(statement)));
}

export function postgresMigrationSqlInventoryFromTokensV1(
  tokens: readonly PostgresMigrationSqlTokenV1[],
): PostgresMigrationSqlInventoryV1 {
  const statements = splitPostgresMigrationSqlStatementsV1(tokens);
  const constraintWords = statements.filter((statement) => {
    const statementWords = wordsFrom(statement);
    return hasPostgresMigrationWordsV1(statementWords, ['CREATE', 'DOMAIN'])
      || hasPostgresMigrationWordsV1(statementWords, ['CREATE', 'TABLE'])
      || hasPostgresMigrationWordsV1(statementWords, ['ADD', 'CONSTRAINT']);
  }).flatMap((statement) => wordsFrom(statement));
  const functions = new Set<string>();
  const functionCounts = {
    octet_length: 0, scale: 0, substring: 0, pg_has_role: 0,
  };
  for (let index = 0; index + 3 < tokens.length; index += 1) {
    if (tokens[index]?.kind === 'word' && tokens[index]?.value === 'PG_CATALOG'
      && tokens[index + 1]?.value === '.' && tokens[index + 2]?.kind === 'word'
      && tokens[index + 3]?.value === '(') {
      functions.add(tokens[index + 2]!.value.toLowerCase());
      const name = tokens[index + 2]!.value.toLowerCase() as keyof typeof functionCounts;
      if (Object.hasOwn(functionCounts, name)) functionCounts[name] += 1;
    }
  }
  const countStatements = (...prefix: string[]): number => statements.filter((statement) => (
    prefix.every((word, index) => wordsFrom(statement)[index] === word)
  )).length;
  const countStatementWords = (...sequence: string[]): number => statements.reduce(
    (count, statement) => count
      + countPostgresMigrationWordsV1(wordsFrom(statement), sequence),
    0,
  );
  return deepFreezeV1({
    statements: statements.length,
    createSchemas: countStatements('CREATE', 'SCHEMA'),
    createDomains: countStatements('CREATE', 'DOMAIN'),
    createTables: countStatements('CREATE', 'TABLE'),
    namedConstraints: constraintWords.filter((word) => word === 'CONSTRAINT').length,
    checkConstraints: constraintWords.filter((word) => word === 'CHECK').length,
    primaryKeys: countPostgresMigrationWordsV1(constraintWords, ['PRIMARY', 'KEY']),
    uniqueConstraints: constraintWords.filter((word) => word === 'UNIQUE').length,
    foreignKeys: countPostgresMigrationWordsV1(constraintWords, ['FOREIGN', 'KEY']),
    createIndexes: countStatements('CREATE', 'INDEX'),
    revokeTypeUsage: statements.filter((statement) => hasPostgresMigrationWordsV1(
      wordsFrom(statement), ['REVOKE', 'USAGE', 'ON', 'TYPE'],
    )).length,
    grants: countStatements('GRANT'),
    createPolicies: countStatements('CREATE', 'POLICY'),
    enableRls: countStatementWords('ENABLE', 'ROW', 'LEVEL', 'SECURITY'),
    forceRls: countStatementWords('FORCE', 'ROW', 'LEVEL', 'SECURITY'),
    callableFunctions: Object.freeze([...functions].sort()),
    callableFunctionCounts: Object.freeze(functionCounts),
    escapeByteaLiterals: tokens.filter((token) => token.kind === 'escape-string').length,
  });
}

export function countPostgresMigrationWordsV1(
  words: readonly string[], sequence: readonly string[],
): number {
  let count = 0;
  for (let index = 0; index + sequence.length <= words.length; index += 1) {
    if (sequence.every((word, offset) => words[index + offset] === word)) count += 1;
  }
  return count;
}

export function hasPostgresMigrationWordsV1(
  words: readonly string[], sequence: readonly string[],
): boolean {
  return countPostgresMigrationWordsV1(words, sequence) > 0;
}

function skipBlockComment(source: string, start: number): number {
  let cursor = start + 2;
  let depth = 1;
  while (cursor < source.length && depth > 0) {
    if (source.startsWith('/*', cursor)) {
      if (++depth > MAXIMUM_COMMENT_DEPTH) throw new TypeError();
      cursor += 2;
    } else if (source.startsWith('*/', cursor)) { depth -= 1; cursor += 2; }
    else cursor += 1;
  }
  if (depth !== 0) throw new TypeError();
  return cursor;
}

function quoted(
  source: string, start: number, delimiter: "'" | '"', backslashEscapes: boolean,
): Readonly<{ value: string; end: number }> {
  let cursor = start + 1;
  let value = '';
  let valueBytes = 0;
  const append = (part: string): void => {
    valueBytes += UTF8_ENCODER.encode(part).byteLength;
    if (valueBytes > MAXIMUM_QUOTED_VALUE_BYTES) throw new TypeError();
    value += part;
  };
  while (cursor < source.length) {
    if (backslashEscapes && source[cursor] === '\\') {
      if (cursor + 1 >= source.length) throw new TypeError();
      const escapedWidth = (source.codePointAt(cursor + 1) ?? 0) > 0xffff ? 2 : 1;
      const part = source.slice(cursor, cursor + 1 + escapedWidth);
      append(part);
      cursor += part.length;
    } else if (source[cursor] !== delimiter) {
      const width = (source.codePointAt(cursor) ?? 0) > 0xffff ? 2 : 1;
      const part = source.slice(cursor, cursor + width);
      append(part);
      cursor += part.length;
    } else if (source[cursor + 1] === delimiter) {
      append(delimiter);
      cursor += 2;
    } else return Object.freeze({ value, end: cursor + 1 });
  }
  throw new TypeError();
}

function wordsFrom(tokens: readonly PostgresMigrationSqlTokenV1[]): readonly string[] {
  return tokens.filter((token) => token.kind === 'word').map((token) => token.value);
}
