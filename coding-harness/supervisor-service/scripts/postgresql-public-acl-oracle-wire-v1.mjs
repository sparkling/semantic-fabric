// SPDX-License-Identifier: MIT

import { TextDecoder } from 'node:util';

export const NULL = String.raw`\N`;
const RAW_PREFIX = '@@ADR0047-RAW-V1';
const PROJECTION_BEGIN = '@@ADR0047-PROJECTION/BEGIN@@';
const PROJECTION_END = '@@ADR0047-PROJECTION/END@@';
const SECTIONS = Object.freeze([
  ['SCHEMA', 11], ['RELATION', 13], ['COLUMN', 12], ['ROUTINE', 25],
  ['TYPE', 15], ['LANGUAGE', 11], ['FDW', 11], ['SERVER', 12],
  ['LARGE_OBJECT', 11], ['CONTROL', 32],
]);

export function parseOracleSession(value) {
  const text = decodeUtf8(value, 'ORACLE_TRANSCRIPT_UTF8_INVALID');
  assert(Buffer.byteLength(text, 'utf8') <= 16 * 1024 * 1024,
    'ORACLE_TRANSCRIPT_BYTES_INVALID');
  assert(text.endsWith('\n') && !text.includes('\r') && !text.includes('\0'),
    'ORACLE_TRANSCRIPT_FRAMING_INVALID');
  const lines = text.slice(0, -1).split('\n');
  const raw = {};
  let cursor = 0;
  for (const [section, fields] of SECTIONS) {
    assert(lines[cursor] === `${RAW_PREFIX}/${section}/BEGIN@@`,
      'ORACLE_TRANSCRIPT_SECTION_BEGIN_INVALID');
    cursor += 1;
    const rows = [];
    const end = `${RAW_PREFIX}/${section}/END@@`;
    while (cursor < lines.length && lines[cursor] !== end) {
      const cells = lines[cursor].split('\t');
      assert(cells.length === fields, 'ORACLE_TRANSCRIPT_FIELD_COUNT_INVALID');
      cells.forEach(assertAsciiCell);
      rows.push(cells);
      cursor += 1;
    }
    assert(lines[cursor] === end, 'ORACLE_TRANSCRIPT_SECTION_END_INVALID');
    assert(rows.length <= 32_768, 'ORACLE_TRANSCRIPT_SECTION_ROWS_INVALID');
    raw[section] = rows;
    cursor += 1;
  }
  assert(lines[cursor] === PROJECTION_BEGIN, 'ORACLE_PROJECTION_BEGIN_INVALID');
  cursor += 1;
  const projection = [];
  while (cursor < lines.length && lines[cursor] !== PROJECTION_END) {
    assert(lines[cursor].length > 0, 'ORACLE_PROJECTION_LINE_INVALID');
    projection.push(lines[cursor]);
    cursor += 1;
  }
  assert(lines[cursor] === PROJECTION_END && cursor + 1 === lines.length,
    'ORACLE_PROJECTION_END_INVALID');
  assert(projection.length > 0 && projection.length <= 8_192,
    'ORACLE_PROJECTION_CARDINALITY_INVALID');
  return { raw, projection };
}

export function unsigned(value) {
  assert(/^(?:0|[1-9][0-9]*)$/.test(value), 'ORACLE_DECIMAL_INVALID');
  const result = Number(value);
  assert(Number.isSafeInteger(result), 'ORACLE_DECIMAL_RANGE_INVALID');
  return result;
}

export function nullableUnsigned(value) {
  return value === NULL ? null : unsigned(value);
}

export function signed(value) {
  assert(/^(?:0|-?[1-9][0-9]*)$/.test(value), 'ORACLE_SIGNED_DECIMAL_INVALID');
  const result = Number(value);
  assert(Number.isSafeInteger(result), 'ORACLE_SIGNED_DECIMAL_RANGE_INVALID');
  return result;
}

export function oid(value, allowZero = false) {
  const result = unsigned(value);
  assert(allowZero ? result <= 4_294_967_295 : result > 0 && result <= 4_294_967_295,
    'ORACLE_OID_INVALID');
  return result;
}

export function bool(value) {
  assert(value === 't' || value === 'f', 'ORACLE_BOOLEAN_INVALID');
  return value === 't';
}

export function nullableHex(value) {
  return value === NULL ? null : hex(value);
}

export function hex(value) {
  assert(/^(?:[0-9a-f]{2})*$/.test(value), 'ORACLE_HEX_INVALID');
  const bytes = Buffer.from(value, 'hex');
  const text = decodeUtf8(bytes, 'ORACLE_HEX_UTF8_INVALID');
  assert(Buffer.from(text, 'utf8').equals(bytes), 'ORACLE_HEX_ROUNDTRIP_INVALID');
  validString(text);
  return text;
}

export function validString(value) {
  assert(typeof value === 'string' && !value.includes('\0') && !/[\uD800-\uDFFF]/u.test(value),
    'ORACLE_STRING_INVALID');
}

export function assert(condition, code) {
  if (!condition) throw new Error(code);
}

function assertAsciiCell(value) {
  assert(/^[\x20-\x7e]*$/.test(value), 'ORACLE_TRANSCRIPT_CELL_ASCII_INVALID');
}

function decodeUtf8(value, code) {
  try { return new TextDecoder('utf-8', { fatal: true }).decode(value); }
  catch { throw new Error(code); }
}
