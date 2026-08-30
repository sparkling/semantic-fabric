// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  NULL, assert, bool, hex, nullableHex, nullableUnsigned, oid, signed, unsigned,
  validString,
} from './postgresql-public-acl-oracle-wire-v1.mjs';
const KEYS = Object.freeze([
  'objectClass', 'schemaName', 'objectName', 'subobjectName', 'objectKind',
  'routineIdentityArguments', 'privilege', 'grantable',
]);
const RELATION_KINDS = Object.freeze({
  r: 'table', p: 'partitioned-table', v: 'view', m: 'materialized-view',
  f: 'foreign-table', S: 'sequence',
});
const TYPE_KINDS = Object.freeze({
  b: 'base', c: 'composite', d: 'domain', e: 'enum', p: 'pseudo',
  r: 'range', m: 'multirange',
});
const KNOWN_RELKINDS = new Set(['r', 'i', 'S', 't', 'v', 'm', 'c', 'f', 'p', 'I']);
const TABLE_PRIVILEGES = Object.freeze([
  'SELECT', 'INSERT', 'UPDATE', 'DELETE', 'TRUNCATE', 'REFERENCES', 'TRIGGER',
]);
const CONTROL_KEYS = Object.freeze([
  'schemaBase', 'schemaAclItems', 'schemaAclAtoms',
  'relationBase', 'relationAclItems', 'relationAclAtoms',
  'columnBase', 'columnAclItems', 'columnAclAtoms',
  'routineBase', 'routineAclItems', 'routineAclAtoms', 'routineArgumentItems',
  'typeBase', 'typeAclItems', 'typeAclAtoms',
  'languageBase', 'languageAclItems', 'languageAclAtoms',
  'fdwBase', 'fdwAclItems', 'fdwAclAtoms',
  'serverBase', 'serverAclItems', 'serverAclAtoms',
  'largeObjectBase', 'largeObjectAclItems', 'largeObjectAclAtoms', 'defaultAclBase',
  'parameterAclBase', 'userMappingBase', 'publicNamespaceDependBase',
]);

export function deriveOracleRecords(raw, options = {}) {
  const clean = options.enforceCleanProfile !== false;
  const schemas = parseSimple(raw.SCHEMA, simpleConfig('schema'));
  const relations = parseSimple(raw.RELATION, simpleConfig('relation'));
  const columns = parseSimple(raw.COLUMN, simpleConfig('column'));
  const routines = parseRoutines(raw.ROUTINE);
  const types = parseSimple(raw.TYPE, simpleConfig('type'));
  const languages = parseSimple(raw.LANGUAGE, simpleConfig('language'));
  const fdws = parseSimple(raw.FDW, simpleConfig('fdw'));
  const servers = parseSimple(raw.SERVER, simpleConfig('server'));
  const largeObjects = parseSimple(raw.LARGE_OBJECT, simpleConfig('largeObject'));
  const control = parseControl(raw.CONTROL);
  const catalogues = { schemas, relations, columns, routines, types, languages, fdws,
    servers, largeObjects };
  validateControl(catalogues, control);
  validateReferences(catalogues);
  if (clean) validateCleanProfile(catalogues, control);

  const records = [];
  for (const value of schemas.values()) {
    if (!['public', 'sf_supervisor_v1'].includes(value.name)) {
      appendAcl(records, value, 'n', recordBase('schema', null, value.name, null,
        'schema', null));
    }
  }
  for (const value of relations.values()) {
    assert(KNOWN_RELKINDS.has(value.kind), 'ORACLE_RELATION_KIND_UNKNOWN');
    const objectKind = RELATION_KINDS[value.kind];
    if (objectKind === undefined) {
      assert(value.acl === null || value.acl.length === 0,
        'ORACLE_UNSUPPORTED_RELATION_ACL_INVALID');
      continue;
    }
    const schema = schemas.get(String(value.namespaceOid));
    if (excludedNamespace(schema.name)) continue;
    appendAcl(records, value, value.kind === 'S' ? 's' : 'r',
      recordBase('relation', schema.name, value.name, null, objectKind, null));
  }
  for (const value of columns.values()) {
    const parent = relations.get(String(value.relationOid));
    const objectKind = RELATION_KINDS[parent.kind];
    const projectable = value.number > 0 && !value.dropped
      && objectKind !== undefined && objectKind !== 'sequence';
    if (!projectable) {
      assert(value.acl === null || value.acl.length === 0,
        'ORACLE_INVALID_COLUMN_ACL_INVALID');
      continue;
    }
    const schema = schemas.get(String(parent.namespaceOid));
    if (excludedNamespace(schema.name)) continue;
    appendAcl(records, { ...value, owner: parent.owner }, 'c',
      recordBase('column', schema.name, parent.name, value.name, objectKind, null));
  }
  for (const value of routines.values()) {
    const objectKind = ({ f: 'function', p: 'procedure', a: 'aggregate',
      w: 'window-function' })[value.kind];
    assert(objectKind !== undefined, 'ORACLE_ROUTINE_KIND_UNKNOWN');
    const schema = schemas.get(String(value.namespaceOid));
    if (excludedNamespace(schema.name)) continue;
    appendAcl(records, value, 'f', recordBase('routine', schema.name, value.name, null,
      objectKind, routineIdentity(value)));
  }
  for (const value of types.values()) {
    const schema = schemas.get(String(value.namespaceOid));
    const source = typeAuthority(value, catalogues);
    const objectKind = source.array ? 'array' : TYPE_KINDS[value.kind];
    assert(objectKind !== undefined, 'ORACLE_TYPE_KIND_UNKNOWN');
    if (excludedNamespace(schema.name)) continue;
    appendAcl(records, source.authority, 'T',
      recordBase('type', schema.name, value.name, null, objectKind, null));
  }
  for (const value of languages.values()) {
    appendAcl(records, value, 'l', recordBase('language', null, value.name, null,
      'language', null));
  }
  for (const value of fdws.values()) {
    appendAcl(records, value, 'F', recordBase('foreign-data-wrapper', null, value.name,
      null, 'foreign-data-wrapper', null));
  }
  for (const value of servers.values()) {
    appendAcl(records, value, 'S', recordBase('foreign-server', null, value.name, null,
      'foreign-server', null));
  }
  for (const value of largeObjects.values()) {
    const publicAtoms = effectiveAcl(value, 'L').filter((atom) => atom.grantee === 0);
    assert(publicAtoms.length === 0, 'ORACLE_LARGE_OBJECT_PUBLIC_ACL_INVALID');
  }
  validateRecords(records);
  records.sort(compareRecords);
  return records;
}

export function parseProjectionRecords(lines) {
  const records = lines.map((line) => {
    let value;
    try { value = JSON.parse(line); } catch { throw new Error('ORACLE_PROJECTION_JSON_INVALID'); }
    validateRecord(value);
    return value;
  });
  for (let index = 1; index < records.length; index += 1) {
    assert(compareRecords(records[index - 1], records[index]) < 0,
      'ORACLE_PROJECTION_ORDER_INVALID');
  }
  return records;
}

export function compareRecordBags(expected, actual) {
  const left = bag(expected);
  const right = bag(actual);
  assert(left.size === right.size, 'ORACLE_RECORD_BAG_KEYS_MISMATCH');
  for (const [key, count] of left) {
    assert(right.get(key) === count, 'ORACLE_RECORD_BAG_MULTIPLICITY_MISMATCH');
  }
}

export function canonicalFixture(records) {
  return Buffer.from(`${JSON.stringify(records)}\n`, 'utf8');
}

export function compareRecords(left, right) {
  for (const key of KEYS) {
    const a = left[key];
    const b = right[key];
    if (a === b) continue;
    if (a === null) return -1;
    if (b === null) return 1;
    if (typeof a === 'boolean') return a ? 1 : -1;
    const order = Buffer.compare(Buffer.from(a, 'utf8'), Buffer.from(b, 'utf8'));
    if (order !== 0) return order;
  }
  return 0;
}

export function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}

function simpleConfig(section) {
  const configs = {
    schema: { base: [0, 1, 2], acl: 3, map: (v) => ({ oid: oid(v[0]), name: hex(v[1]), owner: oid(v[2]) }) },
    relation: { base: [0, 1, 2, 3, 4], acl: 5, map: (v) => ({ oid: oid(v[0]), namespaceOid: oid(v[1]), name: hex(v[2]), kind: hex(v[3]), owner: oid(v[4]) }) },
    column: { base: [0, 1, 2, 3], acl: 4, key: (v) => `${v[0]}:${v[1]}`, map: (v) => ({ relationOid: oid(v[0]), number: signed(v[1]), name: hex(v[2]), dropped: bool(v[3]) }) },
    type: { base: [0, 1, 2, 3, 4, 5, 6], acl: 7, map: (v) => ({ oid: oid(v[0]), namespaceOid: oid(v[1]), name: hex(v[2]), kind: hex(v[3]), owner: oid(v[4]), elementOid: oid(v[5], true), handlerOid: oid(v[6], true) }) },
    language: { base: [0, 1, 2], acl: 3, map: (v) => ({ oid: oid(v[0]), name: hex(v[1]), owner: oid(v[2]) }) },
    fdw: { base: [0, 1, 2], acl: 3, map: (v) => ({ oid: oid(v[0]), name: hex(v[1]), owner: oid(v[2]) }) },
    server: { base: [0, 1, 2, 3], acl: 4, map: (v) => ({ oid: oid(v[0]), name: hex(v[1]), owner: oid(v[2]), fdwOid: oid(v[3]) }) },
    largeObject: { base: [0, 1, 2], acl: 3, map: (v) => ({ oid: oid(v[0]), name: nullableHex(v[1]), owner: oid(v[2]) }) },
  };
  return configs[section];
}

function parseSimple(rows, config) {
  const objects = new Map();
  for (const row of rows) {
    const key = config.key?.(row) ?? row[0];
    const base = config.base.map((index) => row[index]).join('\t');
    const existing = objects.get(key);
    if (existing !== undefined) assert(existing.base === base, 'ORACLE_BASE_FACT_CONFLICT');
    const value = existing ?? { base, value: config.map(row), acl: new Map(), aclState: null };
    collectAcl(value, row, config.acl);
    objects.set(key, value);
  }
  for (const [key, entry] of objects) {
    const acl = finishAcl(entry);
    entry.value.acl = acl.atoms;
    entry.value.aclItemCount = acl.itemCount;
    objects.set(key, entry.value);
  }
  return objects;
}

function parseRoutines(rows) {
  const objects = new Map();
  for (const row of rows) {
    const baseFields = row.slice(0, 11).join('\t');
    const existing = objects.get(row[0]);
    if (existing !== undefined) assert(existing.base === baseFields, 'ORACLE_ROUTINE_FACT_CONFLICT');
    const entry = existing ?? {
      base: baseFields,
      value: { oid: oid(row[0]), namespaceOid: oid(row[1]), name: hex(row[2]),
        kind: hex(row[3]), owner: oid(row[4]), aggregateKind: nullableHex(row[5]),
        directArgs: nullableUnsigned(row[6]), allTypesNull: bool(row[7]),
        modesNull: bool(row[8]), namesNull: bool(row[9]), args: [] },
      argCount: unsigned(row[10]), args: new Map(), acl: new Map(), aclState: null,
    };
    collectArgument(entry, row);
    collectAcl(entry, row, 17);
    objects.set(row[0], entry);
  }
  for (const [key, entry] of objects) {
    assert(entry.args.size === entry.argCount, 'ORACLE_ROUTINE_ARGUMENT_COUNT_INVALID');
    entry.value.args = dense(entry.args, entry.argCount, 'ORACLE_ROUTINE_ARGUMENT_ORDINAL_INVALID');
    const acl = finishAcl(entry);
    entry.value.acl = acl.atoms;
    entry.value.aclItemCount = acl.itemCount;
    objects.set(key, entry.value);
  }
  return objects;
}

function collectArgument(entry, row) {
  const ordinal = nullableUnsigned(row[11]);
  if (entry.argCount === 0) {
    assert(ordinal === null && row.slice(12, 17).every((value) => value === NULL),
      'ORACLE_ROUTINE_ARGUMENT_SENTINEL_INVALID');
    return;
  }
  assert(ordinal !== null && ordinal > 0 && ordinal <= entry.argCount,
    'ORACLE_ROUTINE_ARGUMENT_ORDINAL_INVALID');
  const value = { mode: nullableHex(row[12]), name: nullableHex(row[13]),
    quotedName: nullableHex(row[14]), typeOid: oid(row[15]), type: hex(row[16]) };
  const encoded = JSON.stringify(value);
  const existing = entry.args.get(ordinal);
  if (existing !== undefined) assert(existing.encoded === encoded, 'ORACLE_ROUTINE_ARGUMENT_CONFLICT');
  entry.args.set(ordinal, { encoded, value });
}

function collectAcl(entry, row, offset) {
  const isNull = bool(row[offset]);
  const itemCount = nullableUnsigned(row[offset + 1]);
  const atomCount = unsigned(row[offset + 2]);
  const atomCells = row.slice(offset + 3, offset + 8);
  const state = `${isNull}:${itemCount === null ? NULL : itemCount}:${atomCount}`;
  if (entry.aclState !== null) assert(entry.aclState === state, 'ORACLE_ACL_STATE_CONFLICT');
  entry.aclState = state;
  if (atomCount === 0) {
    assert((isNull ? itemCount === null : itemCount !== null)
      && atomCells.every((value) => value === NULL),
      'ORACLE_ACL_SENTINEL_INVALID');
    return;
  }
  assert(!isNull && itemCount !== null && itemCount > 0, 'ORACLE_ACL_COUNT_INVALID');
  const ordinal = unsigned(atomCells[0]);
  assert(ordinal > 0 && ordinal <= atomCount, 'ORACLE_ACL_ORDINAL_INVALID');
  const atom = { grantor: oid(atomCells[1]), grantee: oid(atomCells[2], true),
    privilege: hex(atomCells[3]), grantable: bool(atomCells[4]) };
  const encoded = JSON.stringify(atom);
  const existing = entry.acl.get(ordinal);
  if (existing !== undefined) assert(existing.encoded === encoded, 'ORACLE_ACL_ATOM_CONFLICT');
  entry.acl.set(ordinal, { encoded, value: atom });
}

function finishAcl(entry) {
  assert(entry.aclState !== null, 'ORACLE_ACL_STATE_MISSING');
  const [nullText, itemText, atomText] = entry.aclState.split(':');
  const itemCount = itemText === NULL ? null : Number(itemText);
  const atoms = dense(entry.acl, Number(atomText), 'ORACLE_ACL_ORDINAL_DENSITY_INVALID');
  return { atoms: nullText === 'true' ? null : atoms, itemCount };
}

function dense(values, count, code) {
  const result = [];
  for (let ordinal = 1; ordinal <= count; ordinal += 1) {
    const entry = values.get(ordinal);
    assert(entry !== undefined, code);
    result.push(entry.value);
  }
  return result;
}

function parseControl(rows) {
  assert(rows.length === 1, 'ORACLE_CONTROL_CARDINALITY_INVALID');
  return Object.fromEntries(CONTROL_KEYS.map((key, index) => [key, unsigned(rows[0][index])]));
}

function validateControl(c, control) {
  const checks = [
    ['schemas', 'schemaBase', 'schemaAclItems', 'schemaAclAtoms'],
    ['relations', 'relationBase', 'relationAclItems', 'relationAclAtoms'],
    ['columns', 'columnBase', 'columnAclItems', 'columnAclAtoms'],
    ['routines', 'routineBase', 'routineAclItems', 'routineAclAtoms'],
    ['types', 'typeBase', 'typeAclItems', 'typeAclAtoms'],
    ['languages', 'languageBase', 'languageAclItems', 'languageAclAtoms'],
    ['fdws', 'fdwBase', 'fdwAclItems', 'fdwAclAtoms'],
    ['servers', 'serverBase', 'serverAclItems', 'serverAclAtoms'],
    ['largeObjects', 'largeObjectBase', 'largeObjectAclItems', 'largeObjectAclAtoms'],
  ];
  for (const [catalogue, baseKey, itemKey, atomKey] of checks) {
    const values = [...c[catalogue].values()];
    assert(values.length === control[baseKey], 'ORACLE_CONTROL_BASE_COUNT_INVALID');
    assert(values.reduce((sum, value) => sum + (value.aclItemCount ?? 0), 0)
      === control[itemKey], 'ORACLE_CONTROL_ACL_ITEM_COUNT_INVALID');
    assert(values.reduce((sum, value) => sum + (value.acl?.length ?? 0), 0)
      === control[atomKey], 'ORACLE_CONTROL_ACL_ATOM_COUNT_INVALID');
  }
  assert([...c.routines.values()].reduce((sum, value) => sum + value.args.length, 0)
    === control.routineArgumentItems, 'ORACLE_CONTROL_ARGUMENT_COUNT_INVALID');
}

function validateReferences(c) {
  for (const value of c.relations.values()) requireRef(c.schemas, value.namespaceOid);
  for (const value of c.routines.values()) {
    requireRef(c.schemas, value.namespaceOid);
    for (const argument of value.args) requireRef(c.types, argument.typeOid);
  }
  for (const value of c.types.values()) {
    requireRef(c.schemas, value.namespaceOid);
    if (value.elementOid !== 0) requireRef(c.types, value.elementOid);
    if (value.handlerOid !== 0) requireRef(c.routines, value.handlerOid);
  }
  for (const value of c.columns.values()) requireRef(c.relations, value.relationOid);
  for (const value of c.servers.values()) requireRef(c.fdws, value.fdwOid);
}

function validateCleanProfile(c, control) {
  assert(control.defaultAclBase === 0 && control.parameterAclBase === 0
    && control.userMappingBase === 0 && control.publicNamespaceDependBase === 0,
  'ORACLE_CLEAN_CONTROL_INVALID');
  assert(c.fdws.size === 0 && c.servers.size === 0 && c.largeObjects.size === 0,
    'ORACLE_CLEAN_GLOBAL_CATALOGUE_INVALID');
  const schemaNames = new Set([...c.schemas.values()].map((value) => value.name));
  assert(schemaNames.has('public') && !schemaNames.has('sf_supervisor_v1'),
    'ORACLE_CLEAN_SCHEMA_PROFILE_INVALID');
  for (const group of [c.relations, c.routines, c.types]) {
    for (const value of group.values()) {
      assert(c.schemas.get(String(value.namespaceOid)).name !== 'public',
        'ORACLE_CLEAN_PUBLIC_OBJECT_INVALID');
    }
  }
}

function typeAuthority(value, c) {
  assert(TYPE_KINDS[value.kind] !== undefined, 'ORACLE_TYPE_KIND_UNKNOWN');
  if (value.elementOid === 0) return { array: false, authority: value };
  const handler = c.routines.get(String(value.handlerOid));
  const handlerSchema = c.schemas.get(String(handler.namespaceOid));
  const handlerIdentity = routineIdentity(handler);
  if (handlerSchema.name === 'pg_catalog' && handler.name === 'array_subscript_handler'
      && handler.kind === 'f' && handlerIdentity === 'internal') {
    assert(value.acl === null, 'ORACLE_ARRAY_OWN_ACL_INVALID');
    return { array: true, authority: c.types.get(String(value.elementOid)) };
  }
  assert(handlerSchema.name === 'pg_catalog' && handler.name === 'raw_array_subscript_handler'
    && handler.kind === 'f' && handlerIdentity === 'internal',
  'ORACLE_ELEMENT_HANDLER_INVALID');
  return { array: false, authority: value };
}

function routineIdentity(value) {
  assert(value.modesNull ? value.args.every((arg) => arg.mode === null)
    : value.args.every((arg) => arg.mode !== null), 'ORACLE_ROUTINE_ARGUMENT_MODES_INVALID');
  assert(!value.namesNull || value.args.every((arg) => arg.name === null
    && arg.quotedName === null), 'ORACLE_ROUTINE_ARGUMENT_NAMES_INVALID');
  const args = [];
  for (const arg of value.args) {
    const mode = arg.mode ?? 'i';
    const prefix = ({ i: value.kind === 'p' ? 'IN ' : '', o: 'OUT ',
      b: 'INOUT ', v: 'VARIADIC ', t: '' })[mode];
    assert(prefix !== undefined, 'ORACLE_ROUTINE_ARGUMENT_MODE_INVALID');
    assert(arg.name === null ? arg.quotedName === null : arg.quotedName !== null,
      'ORACLE_ROUTINE_ARGUMENT_NAME_INVALID');
    if (mode === 't') continue;
    const name = arg.name === null || arg.name.length === 0 ? '' : `${arg.quotedName} `;
    args.push(`${prefix}${name}${arg.type}`);
  }
  if (value.kind !== 'a') {
    assert(value.aggregateKind === null && value.directArgs === null,
      'ORACLE_ROUTINE_AGGREGATE_METADATA_INVALID');
    return args.join(', ');
  }
  assert(['n', 'o', 'h'].includes(value.aggregateKind) && value.directArgs !== null
    && value.directArgs <= args.length, 'ORACLE_ROUTINE_AGGREGATE_METADATA_INVALID');
  if (value.aggregateKind === 'n') return args.join(', ');
  if (value.aggregateKind === 'h') {
    assert(args.length > 0, 'ORACLE_ROUTINE_HYPOTHETICAL_ARGUMENT_INVALID');
    return `${args.join(', ')} ORDER BY ${args.at(-1)}`;
  }
  const direct = args.slice(0, value.directArgs).join(', ');
  const ordered = args.slice(value.directArgs).join(', ');
  return `${direct}${direct.length > 0 ? ' ' : ''}ORDER BY ${ordered}`;
}

function appendAcl(records, source, code, base) {
  for (const atom of effectiveAcl(source, code)) {
    if (atom.grantee !== 0) continue;
    assert(atom.grantable === false, 'ORACLE_PUBLIC_GRANTABLE_INVALID');
    records.push({ ...base, privilege: atom.privilege, grantable: false });
  }
}

function effectiveAcl(source, code) {
  if (source.acl !== null) return source.acl;
  const owner = (privilegesFor(code)).map((privilege) => ({
    grantor: source.owner, grantee: source.owner, privilege, grantable: false,
  }));
  const publicPrivileges = ({ f: ['EXECUTE'], T: ['USAGE'], l: ['USAGE'] })[code] ?? [];
  return [...owner, ...publicPrivileges.map((privilege) => ({
    grantor: source.owner, grantee: 0, privilege, grantable: false,
  }))];
}

function privilegesFor(code) {
  const values = { n: ['CREATE', 'USAGE'], r: TABLE_PRIVILEGES,
    s: ['SELECT', 'UPDATE', 'USAGE'], c: [], f: ['EXECUTE'], T: ['USAGE'],
    l: ['USAGE'], F: ['USAGE'], S: ['USAGE'], L: ['SELECT', 'UPDATE'] }[code];
  assert(values !== undefined, 'ORACLE_ACL_DEFAULT_CODE_INVALID');
  return values;
}

function recordBase(objectClass, schemaName, objectName, subobjectName, objectKind,
  routineIdentityArguments) {
  return { objectClass, schemaName, objectName, subobjectName, objectKind,
    routineIdentityArguments };
}

function validateRecords(records) {
  const identities = new Set();
  for (const value of records) {
    validateRecord(value);
    const key = JSON.stringify(value);
    assert(!identities.has(key), 'ORACLE_RECORD_IDENTITY_DUPLICATE');
    identities.add(key);
  }
  assert(records.length > 0 && records.length <= 8_192, 'ORACLE_RECORD_COUNT_INVALID');
  assert(1 + records.length * 9 <= 65_536, 'ORACLE_RECORD_NODE_COUNT_INVALID');
}

function validateRecord(value) {
  assert(value !== null && typeof value === 'object' && !Array.isArray(value)
    && Object.getPrototypeOf(value) === Object.prototype
    && JSON.stringify(Object.keys(value)) === JSON.stringify(KEYS),
  'ORACLE_RECORD_SHAPE_INVALID');
  for (const key of ['objectClass', 'objectName', 'objectKind', 'privilege']) validString(value[key]);
  for (const key of ['schemaName', 'subobjectName', 'routineIdentityArguments']) {
    if (value[key] !== null) validString(value[key]);
  }
  assert(value.grantable === false, 'ORACLE_RECORD_GRANTABLE_INVALID');
  const allowed = value.objectClass === 'schema' ? ['CREATE', 'USAGE']
    : value.objectClass === 'relation' ? (value.objectKind === 'sequence'
      ? ['SELECT', 'UPDATE', 'USAGE'] : TABLE_PRIVILEGES)
      : value.objectClass === 'column' ? ['INSERT', 'SELECT', 'UPDATE', 'REFERENCES']
      : value.objectClass === 'routine' ? ['EXECUTE'] : ['USAGE'];
  assert(allowed.includes(value.privilege), 'ORACLE_RECORD_PRIVILEGE_INVALID');
}

function bag(records) {
  const value = new Map();
  for (const record of records) {
    const key = JSON.stringify(record);
    value.set(key, (value.get(key) ?? 0) + 1);
  }
  return value;
}

function excludedNamespace(value) {
  return value === 'public' || value === 'sf_supervisor_v1';
}

function requireRef(values, key) {
  assert(values.has(String(key)), 'ORACLE_OID_REFERENCE_INVALID');
}
