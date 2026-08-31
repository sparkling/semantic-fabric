// SPDX-License-Identifier: MIT

import { TextDecoder } from 'node:util';
import { compareRecords, parseProjectionRecords, sha256 }
  from './postgresql-public-acl-oracle-v1.mjs';
import {
  assert, bool, hex, nullableHex, nullableUnsigned, oid, signed, unsigned,
} from './postgresql-public-acl-oracle-wire-v1.mjs';

const ROLE_NAME = 'sf_public_acl_no_membership_witness_v1';
const DATABASE_NAME = 'sf_public_baseline';
const MARKER = '@@ADR0047-HAS-V1';
const MAX_TRANSCRIPT_BYTES = 16 * 1024 * 1024;
const MAX_FIXTURE_BYTES = 1024 * 1024;
const MAX_ROWS_PER_SECTION = 32_768;
const MAX_CHECKS = 65_536;
const SECTIONS = Object.freeze([
  ['ROLE', 19], ['AUTHORITY', 2], ['SCHEMA', 5], ['RELATION', 7],
  ['COLUMN', 11], ['ROUTINE', 8], ['TYPE', 11], ['LANGUAGE', 5],
  ['FDW', 5], ['SERVER', 5],
]);
const AUTHORITIES = Object.freeze([
  'acl-column', 'acl-database', 'acl-default', 'acl-fdw', 'acl-language',
  'acl-large-object', 'acl-parameter', 'acl-relation', 'acl-routine', 'acl-schema',
  'acl-server', 'acl-tablespace', 'acl-type', 'owned-database', 'owned-fdw',
  'owned-language', 'owned-large-object', 'owned-relation', 'owned-routine',
  'owned-schema', 'owned-server', 'owned-tablespace', 'owned-type',
  'predefined-member', 'predefined-set', 'predefined-usage',
]);
const TABLE_PRIVILEGES = Object.freeze([
  'DELETE', 'INSERT', 'REFERENCES', 'SELECT', 'TRIGGER', 'TRUNCATE', 'UPDATE',
]);
const SEQUENCE_PRIVILEGES = Object.freeze(['SELECT', 'UPDATE', 'USAGE']);
const COLUMN_PRIVILEGES = Object.freeze(['INSERT', 'REFERENCES', 'SELECT', 'UPDATE']);
const RELATION_KINDS = new Set([
  'table', 'partitioned-table', 'view', 'materialized-view', 'foreign-table', 'sequence',
]);
const ROUTINE_KINDS = new Set(['function', 'procedure', 'aggregate', 'window-function']);
const TYPE_KINDS = new Set([
  'array', 'base', 'composite', 'domain', 'enum', 'multirange', 'pseudo', 'range',
]);
const OBJECT_CLASSES = Object.freeze([
  'schema', 'relation', 'column', 'routine', 'type', 'language',
  'foreign-data-wrapper', 'foreign-server',
]);
const INVENTORY_KEYS = Object.freeze([
  'objectClass', 'objectOid', 'schemaName', 'objectName', 'subobjectName',
  'subobjectNumber', 'objectKind', 'routineIdentityArguments', 'privilege',
  'trueArray', 'elementObjectOid', 'elementSchemaName', 'elementObjectName',
]);

export function parseWitnessSession(value) {
  const bytes = Buffer.from(value);
  assert(bytes.byteLength <= MAX_TRANSCRIPT_BYTES, 'WITNESS_TRANSCRIPT_BYTES_INVALID');
  const text = decodeUtf8(bytes, 'WITNESS_TRANSCRIPT_UTF8_INVALID');
  assert(text.endsWith('\n') && !text.includes('\r') && !text.includes('\0'),
    'WITNESS_TRANSCRIPT_FRAMING_INVALID');
  const lines = text.slice(0, -1).split('\n');
  const raw = {};
  let cursor = 0;
  for (const [section, fieldCount] of SECTIONS) {
    assert(lines[cursor] === `${MARKER}/${section}/BEGIN@@`,
      'WITNESS_TRANSCRIPT_SECTION_BEGIN_INVALID');
    cursor += 1;
    const rows = [];
    const end = `${MARKER}/${section}/END@@`;
    while (cursor < lines.length && lines[cursor] !== end) {
      const cells = lines[cursor].split('\t');
      assert(cells.length === fieldCount, 'WITNESS_TRANSCRIPT_FIELD_COUNT_INVALID');
      cells.forEach((cell) => assert(/^[\x20-\x7e]*$/.test(cell),
        'WITNESS_TRANSCRIPT_CELL_ASCII_INVALID'));
      rows.push(cells);
      cursor += 1;
    }
    assert(lines[cursor] === end, 'WITNESS_TRANSCRIPT_SECTION_END_INVALID');
    assert(rows.length <= MAX_ROWS_PER_SECTION, 'WITNESS_TRANSCRIPT_SECTION_ROWS_INVALID');
    raw[section] = rows;
    cursor += 1;
  }
  assert(cursor === lines.length, 'WITNESS_TRANSCRIPT_TRAILING_DATA_INVALID');
  return { raw };
}

export function verifyWitnessSession(transcript, fixture, inventorySource) {
  const { raw } = parseWitnessSession(transcript);
  const expected = parseFixture(fixture);
  validateRole(raw.ROLE);
  validateAuthorities(raw.AUTHORITY);
  const observations = [
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
  assert(observations.length > 0 && observations.length <= MAX_CHECKS,
    'WITNESS_CHECK_COUNT_INVALID');
  validateObservationOrder(observations);
  validatePrivilegeGroups(observations);
  const inventory = parseWitnessInventory(inventorySource);
  const actualInventory = observations.map(toInventoryEntry);
  assert(JSON.stringify(actualInventory) === JSON.stringify(inventory.entries),
    'WITNESS_INVENTORY_MISMATCH');

  const expectedByKey = new Map(expected.map((record) => [recordKey(record), record]));
  const seenExpected = new Set();
  const relations = new Map(observations
    .filter((value) => value.objectClass === 'relation')
    .map((value) => [relationKey(value), value]));
  let plainTrueCount = 0;
  let grantOptionTrueCount = 0;
  for (const observation of observations) {
    const record = toRecord(observation);
    const key = recordKey(record);
    const direct = expectedByKey.has(key);
    let expectedPlain = direct;
    let expectedGrant = false;
    if (observation.objectClass === 'column') {
      const parent = relations.get(relationKey(observation));
      assert(parent !== undefined, 'WITNESS_COLUMN_PARENT_MISSING');
      assert(observation.parentPlain === parent.plain
        && observation.parentGrantOption === parent.grantOption,
      'WITNESS_COLUMN_PARENT_RESULT_INVALID');
      expectedPlain = parent.plain || direct;
      expectedGrant = parent.grantOption;
    }
    assert(observation.plain === expectedPlain, 'WITNESS_PLAIN_RESULT_INVALID');
    assert(observation.grantOption === expectedGrant && !observation.grantOption,
      'WITNESS_GRANT_OPTION_RESULT_INVALID');
    if (direct) seenExpected.add(key);
    if (observation.plain) plainTrueCount += 1;
    if (observation.grantOption) grantOptionTrueCount += 1;
  }
  assert(seenExpected.size === expected.length, 'WITNESS_FIXTURE_COVERAGE_INVALID');

  const encoded = Buffer.from(`${JSON.stringify(observations)}\n`, 'utf8');
  const classCounts = Object.fromEntries(OBJECT_CLASSES
    .map((key) => [key, { checks: 0, plainTrue: 0, plainFalse: 0 }]));
  for (const observation of observations) {
    const count = classCounts[observation.objectClass] ?? {
      checks: 0, plainTrue: 0, plainFalse: 0,
    };
    count.checks += 1;
    count[observation.plain ? 'plainTrue' : 'plainFalse'] += 1;
    classCounts[observation.objectClass] = count;
  }
  return {
    checkCount: observations.length,
    plainTrueCount,
    plainFalseCount: observations.length - plainTrueCount,
    grantOptionTrueCount,
    corroboratedAtoms: seenExpected.size,
    columnLocalAtoms: expected.filter((value) => value.objectClass === 'column').length,
    trueArrayAtoms: expected.filter((value) => value.objectClass === 'type'
      && value.objectKind === 'array').length,
    inventoryBytes: inventory.bytes.byteLength,
    inventorySha256: sha256(inventory.bytes),
    observationsBytes: encoded.byteLength,
    observationsSha256: sha256(encoded),
    classCounts,
  };
}

export function parseWitnessInventory(value) {
  const bytes = Buffer.from(value ?? []);
  assert(bytes.byteLength <= MAX_TRANSCRIPT_BYTES, 'WITNESS_INVENTORY_BYTES_INVALID');
  const source = decodeUtf8(bytes, 'WITNESS_INVENTORY_UTF8_INVALID');
  assert(source.endsWith('\n')
    && !source.includes('\r') && !source.includes('\0'),
  'WITNESS_INVENTORY_FRAMING_INVALID');
  let entries;
  try { entries = JSON.parse(source); }
  catch { throw new Error('WITNESS_INVENTORY_JSON_INVALID'); }
  assert(Array.isArray(entries) && entries.length > 0 && entries.length <= MAX_CHECKS,
    'WITNESS_INVENTORY_CARDINALITY_INVALID');
  entries.forEach(validateInventoryEntry);
  const observations = entries.map((entry) => ({
    ...entry, plain: false, grantOption: false, parentPlain: null,
    parentGrantOption: null,
  }));
  validateObservationOrder(observations);
  validatePrivilegeGroups(observations);
  assert(Buffer.from(`${JSON.stringify(entries)}\n`, 'utf8').equals(bytes),
    'WITNESS_INVENTORY_CANONICAL_INVALID');
  return { entries, bytes };
}

function validateRole(rows) {
  assert(rows.length === 1, 'WITNESS_ROLE_CARDINALITY_INVALID');
  const row = rows[0];
  assert(identifier(row[0]) === ROLE_NAME, 'WITNESS_ROLE_NAME_INVALID');
  row.slice(1, 8).forEach((value) => assert(bool(value) === false,
    'WITNESS_ROLE_ATTRIBUTE_INVALID'));
  assert(signed(row[8]) === -1, 'WITNESS_ROLE_CONNECTION_LIMIT_INVALID');
  row.slice(9, 12).forEach((value) => assert(bool(value) === true,
    'WITNESS_ROLE_NULL_STATE_INVALID'));
  row.slice(12, 15).forEach((value) => assert(unsigned(value) === 0,
    'WITNESS_ROLE_AUTHORITY_COUNT_INVALID'));
  assert(unsigned(row[15]) === 160_015, 'WITNESS_SERVER_VERSION_INVALID');
  assert(identifier(row[16]) === DATABASE_NAME && identifier(row[17]) === 'postgres'
    && identifier(row[18]) === 'postgres', 'WITNESS_SESSION_IDENTITY_INVALID');
}

function validateAuthorities(rows) {
  assert(rows.length === AUTHORITIES.length, 'WITNESS_AUTHORITY_CARDINALITY_INVALID');
  rows.forEach((row, index) => {
    assert(identifier(row[0]) === AUTHORITIES[index], 'WITNESS_AUTHORITY_NAME_INVALID');
    assert(unsigned(row[1]) === 0, 'WITNESS_AUTHORITY_COUNT_INVALID');
  });
}

function parseSchema(row) {
  return observation('schema', oid(row[0]), null, identifier(row[1]), null, null,
    'schema', null, privilege(row[2], ['CREATE', 'USAGE']), row[3], row[4]);
}

function parseRelation(row) {
  const kind = identifier(row[3]);
  assert(RELATION_KINDS.has(kind), 'WITNESS_RELATION_KIND_INVALID');
  const allowed = kind === 'sequence' ? SEQUENCE_PRIVILEGES : TABLE_PRIVILEGES;
  return observation('relation', oid(row[0]), identifier(row[1]), identifier(row[2]), null,
    null, kind, null, privilege(row[4], allowed), row[5], row[6]);
}

function parseColumn(row) {
  const number = signed(row[1]);
  assert(number > 0 && number <= 32_767, 'WITNESS_COLUMN_NUMBER_INVALID');
  const kind = identifier(row[5]);
  assert(RELATION_KINDS.has(kind) && kind !== 'sequence', 'WITNESS_COLUMN_KIND_INVALID');
  return observation('column', oid(row[0]), identifier(row[2]), identifier(row[3]),
    identifier(row[4]), number, kind, null, privilege(row[6], COLUMN_PRIVILEGES),
    row[7], row[8], {
      parentPlain: bool(row[9]), parentGrantOption: bool(row[10]),
    });
}

function parseRoutine(row) {
  const kind = identifier(row[4]);
  assert(ROUTINE_KINDS.has(kind), 'WITNESS_ROUTINE_KIND_INVALID');
  const identity = text(row[3], 196_608, true);
  return observation('routine', oid(row[0]), identifier(row[1]), identifier(row[2]), null,
    null, kind, identity, privilege(row[5], ['EXECUTE']), row[6], row[7]);
}

function parseType(row) {
  const kind = identifier(row[3]);
  assert(TYPE_KINDS.has(kind), 'WITNESS_TYPE_KIND_INVALID');
  const trueArray = bool(row[4]);
  const elementObjectOid = optionalOid(row[5]);
  const elementSchemaName = optionalIdentifier(row[6]);
  const elementObjectName = optionalIdentifier(row[7]);
  assert(trueArray === (kind === 'array')
    && (trueArray
      ? elementObjectOid !== null && elementSchemaName !== null && elementObjectName !== null
      : elementObjectOid === null && elementSchemaName === null && elementObjectName === null),
  'WITNESS_TYPE_ARRAY_METADATA_INVALID');
  return observation('type', oid(row[0]), identifier(row[1]), identifier(row[2]), null,
    null, kind, null, privilege(row[8], ['USAGE']), row[9], row[10], {
      trueArray, elementObjectOid, elementSchemaName, elementObjectName,
    });
}

function parseGlobal(row, objectClass, objectKind) {
  return observation(objectClass, oid(row[0]), null, identifier(row[1]), null, null,
    objectKind, null, privilege(row[2], ['USAGE']), row[3], row[4]);
}

function observation(objectClass, objectOid, schemaName, objectName, subobjectName,
  subobjectNumber, objectKind, routineIdentityArguments, privilegeName, plainCell,
  grantCell, extras = {}) {
  return {
    objectClass, objectOid, schemaName, objectName, subobjectName, subobjectNumber,
    objectKind, routineIdentityArguments, privilege: privilegeName,
    plain: bool(plainCell), grantOption: bool(grantCell),
    parentPlain: extras.parentPlain ?? null,
    parentGrantOption: extras.parentGrantOption ?? null,
    trueArray: extras.trueArray ?? null,
    elementObjectOid: extras.elementObjectOid ?? null,
    elementSchemaName: extras.elementSchemaName ?? null,
    elementObjectName: extras.elementObjectName ?? null,
  };
}

function validateObservationOrder(observations) {
  for (let index = 1; index < observations.length; index += 1) {
    const previousRank = OBJECT_CLASSES.indexOf(observations[index - 1].objectClass);
    const currentRank = OBJECT_CLASSES.indexOf(observations[index].objectClass);
    assert(previousRank >= 0 && previousRank <= currentRank,
      'WITNESS_OBSERVATION_CLASS_ORDER_INVALID');
    if (previousRank < currentRank) continue;
    const previous = toRecord(observations[index - 1]);
    const current = toRecord(observations[index]);
    assert(compareRecords(previous, current) < 0, 'WITNESS_OBSERVATION_ORDER_INVALID');
  }
}

function validatePrivilegeGroups(observations) {
  const groups = new Map();
  for (const value of observations) {
    const key = JSON.stringify([
      value.objectClass, value.schemaName, value.objectName, value.subobjectName,
      value.objectKind, value.routineIdentityArguments,
    ]);
    const group = groups.get(key) ?? [];
    group.push(value.privilege);
    groups.set(key, group);
  }
  for (const [key, actual] of groups) {
    const [objectClass, , , , objectKind] = JSON.parse(key);
    const expected = objectClass === 'schema' ? ['CREATE', 'USAGE']
      : objectClass === 'relation' ? (objectKind === 'sequence'
        ? SEQUENCE_PRIVILEGES : TABLE_PRIVILEGES)
      : objectClass === 'column' ? COLUMN_PRIVILEGES
      : objectClass === 'routine' ? ['EXECUTE'] : ['USAGE'];
    assert(JSON.stringify(actual) === JSON.stringify(expected),
      'WITNESS_PRIVILEGE_VOCABULARY_INVALID');
  }
}

function parseFixture(value) {
  const bytes = Buffer.from(value);
  assert(bytes.byteLength <= MAX_FIXTURE_BYTES, 'WITNESS_FIXTURE_BYTES_INVALID');
  const source = decodeUtf8(bytes, 'WITNESS_FIXTURE_UTF8_INVALID');
  assert(source.endsWith('\n')
    && !source.includes('\r') && !source.includes('\0'), 'WITNESS_FIXTURE_FRAMING_INVALID');
  let parsed;
  try { parsed = JSON.parse(source); } catch { throw new Error('WITNESS_FIXTURE_JSON_INVALID'); }
  assert(Array.isArray(parsed) && parsed.length > 0 && parsed.length <= 8_192,
    'WITNESS_FIXTURE_CARDINALITY_INVALID');
  const records = parseProjectionRecords(parsed.map((record) => JSON.stringify(record)));
  assert(Buffer.from(`${JSON.stringify(records)}\n`, 'utf8').equals(bytes),
    'WITNESS_FIXTURE_CANONICAL_INVALID');
  return records;
}

function toRecord(value) {
  return {
    objectClass: value.objectClass,
    schemaName: value.schemaName,
    objectName: value.objectName,
    subobjectName: value.subobjectName,
    objectKind: value.objectKind,
    routineIdentityArguments: value.routineIdentityArguments,
    privilege: value.privilege,
    grantable: false,
  };
}

function toInventoryEntry(value) {
  return {
    objectClass: value.objectClass,
    objectOid: value.objectOid,
    schemaName: value.schemaName,
    objectName: value.objectName,
    subobjectName: value.subobjectName,
    subobjectNumber: value.subobjectNumber,
    objectKind: value.objectKind,
    routineIdentityArguments: value.routineIdentityArguments,
    privilege: value.privilege,
    trueArray: value.trueArray,
    elementObjectOid: value.elementObjectOid,
    elementSchemaName: value.elementSchemaName,
    elementObjectName: value.elementObjectName,
  };
}

function validateInventoryEntry(value) {
  assert(value !== null && typeof value === 'object' && !Array.isArray(value)
    && Object.getPrototypeOf(value) === Object.prototype
    && JSON.stringify(Object.keys(value)) === JSON.stringify(INVENTORY_KEYS),
  'WITNESS_INVENTORY_SHAPE_INVALID');
  assert(Number.isInteger(value.objectOid) && value.objectOid > 0
    && value.objectOid <= 4_294_967_295, 'WITNESS_INVENTORY_OBJECT_OID_INVALID');
  assertIdentifierText(value.objectName);
  if (value.schemaName !== null) assertIdentifierText(value.schemaName);
  if (value.subobjectName !== null) assertIdentifierText(value.subobjectName);
  if (value.routineIdentityArguments !== null) {
    assert(typeof value.routineIdentityArguments === 'string'
      && !value.routineIdentityArguments.includes('\0')
      && !/[\uD800-\uDFFF]/u.test(value.routineIdentityArguments)
      && Buffer.byteLength(value.routineIdentityArguments, 'utf8') <= 196_608,
    'WITNESS_INVENTORY_ROUTINE_IDENTITY_INVALID');
  }
  const shape = value.objectClass === 'schema'
    ? value.schemaName === null && value.subobjectName === null
      && value.subobjectNumber === null
      && value.objectKind === 'schema' && value.routineIdentityArguments === null
    : value.objectClass === 'relation'
      ? value.schemaName !== null && value.subobjectName === null
        && value.subobjectNumber === null
        && RELATION_KINDS.has(value.objectKind) && value.routineIdentityArguments === null
      : value.objectClass === 'column'
        ? value.schemaName !== null && value.subobjectName !== null
          && Number.isInteger(value.subobjectNumber) && value.subobjectNumber > 0
          && value.subobjectNumber <= 32_767
          && RELATION_KINDS.has(value.objectKind) && value.objectKind !== 'sequence'
          && value.routineIdentityArguments === null
        : value.objectClass === 'routine'
          ? value.schemaName !== null && value.subobjectName === null
            && value.subobjectNumber === null
            && ROUTINE_KINDS.has(value.objectKind)
            && value.routineIdentityArguments !== null
          : value.objectClass === 'type'
            ? value.schemaName !== null && value.subobjectName === null
              && value.subobjectNumber === null
              && TYPE_KINDS.has(value.objectKind) && value.routineIdentityArguments === null
            : ['language', 'foreign-data-wrapper', 'foreign-server']
              .includes(value.objectClass)
              && value.schemaName === null && value.subobjectName === null
              && value.subobjectNumber === null
              && value.objectKind === value.objectClass
              && value.routineIdentityArguments === null;
  assert(shape, 'WITNESS_INVENTORY_IDENTITY_INVALID');
  const allowed = value.objectClass === 'schema' ? ['CREATE', 'USAGE']
    : value.objectClass === 'relation' ? (value.objectKind === 'sequence'
      ? SEQUENCE_PRIVILEGES : TABLE_PRIVILEGES)
    : value.objectClass === 'column' ? COLUMN_PRIVILEGES
    : value.objectClass === 'routine' ? ['EXECUTE'] : ['USAGE'];
  assert(allowed.includes(value.privilege), 'WITNESS_INVENTORY_PRIVILEGE_INVALID');
  const array = value.objectClass === 'type' && value.objectKind === 'array';
  assert(value.trueArray === (value.objectClass === 'type' ? array : null)
    && (array
      ? Number.isInteger(value.elementObjectOid) && value.elementObjectOid > 0
        && value.elementObjectOid <= 4_294_967_295
        && value.elementSchemaName !== null && value.elementObjectName !== null
      : value.elementObjectOid === null && value.elementSchemaName === null
        && value.elementObjectName === null),
  'WITNESS_INVENTORY_ARRAY_METADATA_INVALID');
  if (array) {
    assertIdentifierText(value.elementSchemaName);
    assertIdentifierText(value.elementObjectName);
  }
}

function assertIdentifierText(value) {
  assert(typeof value === 'string' && value.length > 0 && !value.includes('\0')
    && !/[\uD800-\uDFFF]/u.test(value) && Buffer.byteLength(value, 'utf8') <= 63,
  'WITNESS_INVENTORY_IDENTIFIER_INVALID');
}

function relationKey(value) {
  return JSON.stringify([
    value.objectOid, value.schemaName, value.objectName, value.objectKind, value.privilege,
  ]);
}

function recordKey(value) {
  return JSON.stringify(value);
}

function privilege(value, allowed) {
  const decoded = text(value, 32);
  assert(allowed.includes(decoded), 'WITNESS_PRIVILEGE_INVALID');
  return decoded;
}

function identifier(value) {
  return text(value, 63);
}

function optionalIdentifier(value) {
  const decoded = nullableHex(value);
  if (decoded === null) return null;
  assert(decoded.length > 0 && Buffer.byteLength(decoded, 'utf8') <= 63,
    'WITNESS_IDENTIFIER_INVALID');
  return decoded;
}

function optionalOid(value) {
  const decoded = nullableUnsigned(value);
  assert(decoded === null || (decoded > 0 && decoded <= 4_294_967_295),
    'WITNESS_OPTIONAL_OID_INVALID');
  return decoded;
}

function text(value, maximumBytes, allowEmpty = false) {
  const decoded = hex(value);
  assert((allowEmpty || decoded.length > 0)
    && Buffer.byteLength(decoded, 'utf8') <= maximumBytes, 'WITNESS_TEXT_INVALID');
  return decoded;
}

function decodeUtf8(value, code) {
  try { return new TextDecoder('utf-8', { fatal: true }).decode(value); }
  catch { throw new Error(code); }
}

export { AUTHORITIES, ROLE_NAME };
