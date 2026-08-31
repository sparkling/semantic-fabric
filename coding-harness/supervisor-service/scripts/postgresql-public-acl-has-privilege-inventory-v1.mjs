// SPDX-License-Identifier: MIT

import { TextDecoder } from 'node:util';
import { parseWitnessInventory } from './postgresql-public-acl-has-privilege-v1.mjs';
import {
  assert, bool, hex, nullableHex, nullableUnsigned, oid, signed,
} from './postgresql-public-acl-oracle-wire-v1.mjs';

const MARKER = '@@ADR0047-HAS-INVENTORY-V1';
const MAX_BYTES = 16 * 1024 * 1024;
const MAX_ROWS = 32_768;
const SECTIONS = Object.freeze([
  ['SCHEMA', 3], ['RELATION', 5], ['COLUMN', 7], ['ROUTINE', 6],
  ['TYPE', 9], ['LANGUAGE', 3], ['FDW', 3], ['SERVER', 3],
]);

export function parseWitnessInventorySession(value) {
  const bytes = Buffer.from(value);
  assert(bytes.byteLength <= MAX_BYTES, 'WITNESS_INVENTORY_TRANSCRIPT_BYTES_INVALID');
  const text = decodeUtf8(bytes);
  assert(text.endsWith('\n') && !text.includes('\r') && !text.includes('\0'),
    'WITNESS_INVENTORY_TRANSCRIPT_FRAMING_INVALID');
  const lines = text.slice(0, -1).split('\n');
  const raw = {};
  let cursor = 0;
  for (const [section, fields] of SECTIONS) {
    assert(lines[cursor] === `${MARKER}/${section}/BEGIN@@`,
      'WITNESS_INVENTORY_TRANSCRIPT_SECTION_BEGIN_INVALID');
    cursor += 1;
    const rows = [];
    const end = `${MARKER}/${section}/END@@`;
    while (cursor < lines.length && lines[cursor] !== end) {
      const cells = lines[cursor].split('\t');
      assert(cells.length === fields,
        'WITNESS_INVENTORY_TRANSCRIPT_FIELD_COUNT_INVALID');
      cells.forEach((cell) => assert(/^[\x20-\x7e]*$/.test(cell),
        'WITNESS_INVENTORY_TRANSCRIPT_CELL_ASCII_INVALID'));
      rows.push(cells);
      cursor += 1;
    }
    assert(lines[cursor] === end, 'WITNESS_INVENTORY_TRANSCRIPT_SECTION_END_INVALID');
    assert(rows.length <= MAX_ROWS, 'WITNESS_INVENTORY_TRANSCRIPT_SECTION_ROWS_INVALID');
    raw[section] = rows;
    cursor += 1;
  }
  assert(cursor === lines.length, 'WITNESS_INVENTORY_TRANSCRIPT_TRAILING_DATA_INVALID');
  const entries = [
    ...raw.SCHEMA.map(parseSchema),
    ...raw.RELATION.map(parseRelation),
    ...raw.COLUMN.map(parseColumn),
    ...raw.ROUTINE.map(parseRoutine),
    ...raw.TYPE.map(parseType),
    ...raw.LANGUAGE.map((row) => parseGlobal(row, 'language', 'language')),
    ...raw.FDW.map((row) => parseGlobal(row, 'foreign-data-wrapper',
      'foreign-data-wrapper')),
    ...raw.SERVER.map((row) => parseGlobal(row, 'foreign-server', 'foreign-server')),
  ];
  const canonical = Buffer.from(`${JSON.stringify(entries)}\n`, 'utf8');
  parseWitnessInventory(canonical);
  return { raw, entries, canonical };
}

function parseSchema(row) {
  return entry('schema', oid(row[0]), null, hex(row[1]), null, null, 'schema', null,
    hex(row[2]));
}

function parseRelation(row) {
  return entry('relation', oid(row[0]), hex(row[1]), hex(row[2]), null, null,
    hex(row[3]), null, hex(row[4]));
}

function parseColumn(row) {
  const number = signed(row[1]);
  assert(number > 0 && number <= 32_767, 'WITNESS_INVENTORY_COLUMN_NUMBER_INVALID');
  return entry('column', oid(row[0]), hex(row[2]), hex(row[3]), hex(row[4]), number,
    hex(row[5]), null, hex(row[6]));
}

function parseRoutine(row) {
  return entry('routine', oid(row[0]), hex(row[1]), hex(row[2]), null, null,
    hex(row[4]), hex(row[3]), hex(row[5]));
}

function parseType(row) {
  const trueArray = bool(row[4]);
  const elementObjectOid = optionalOid(row[5]);
  return entry('type', oid(row[0]), hex(row[1]), hex(row[2]), null, null, hex(row[3]),
    null, hex(row[8]), trueArray, elementObjectOid, nullableHex(row[6]),
    nullableHex(row[7]));
}

function parseGlobal(row, objectClass, objectKind) {
  return entry(objectClass, oid(row[0]), null, hex(row[1]), null, null, objectKind,
    null, hex(row[2]));
}

function entry(objectClass, objectOid, schemaName, objectName, subobjectName,
  subobjectNumber, objectKind, routineIdentityArguments, privilege, trueArray = null,
  elementObjectOid = null, elementSchemaName = null, elementObjectName = null) {
  return {
    objectClass, objectOid, schemaName, objectName, subobjectName, subobjectNumber,
    objectKind, routineIdentityArguments, privilege, trueArray, elementObjectOid,
    elementSchemaName, elementObjectName,
  };
}

function optionalOid(value) {
  const result = nullableUnsigned(value);
  assert(result === null || (result > 0 && result <= 4_294_967_295),
    'WITNESS_INVENTORY_OPTIONAL_OID_INVALID');
  return result;
}

function decodeUtf8(value) {
  try { return new TextDecoder('utf-8', { fatal: true }).decode(value); }
  catch { throw new Error('WITNESS_INVENTORY_TRANSCRIPT_UTF8_INVALID'); }
}
