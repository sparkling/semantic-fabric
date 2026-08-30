// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  chmodSync,
  existsSync,
  lstatSync,
  readFileSync,
  writeFileSync,
} from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { TextDecoder } from 'node:util';

const IMAGE_REFERENCE =
  'postgres@sha256:485935f94cc7165afa896978809c37b592dc07f0a37d2c8f645f12412d0212c8';
const IMAGE_CONFIGURATION =
  'sha256:80f4c7a5e91618546dce5b4fe60cf03b14c0f9efa7e40157278d122772ced8d2';
const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const PROJECTION_PATH = resolve(ROOT, '__tests__/fixtures/postgresql-16.15-public-acl-projection-v1.sql');
const FIXTURE_PATH = resolve(
  ROOT,
  '__tests__/fixtures/postgresql-16.15-clean-template0-public-object-acl-v1.json',
);
const MAX_CAPTURE_BYTES = 2 * 1024 * 1024;
const RECORD_KEYS = Object.freeze([
  'objectClass', 'schemaName', 'objectName', 'subobjectName', 'objectKind',
  'routineIdentityArguments', 'privilege', 'grantable',
]);
const CLASS_KINDS = Object.freeze({
  column: Object.freeze(['table', 'partitioned-table', 'view', 'materialized-view', 'foreign-table']),
  'foreign-data-wrapper': Object.freeze(['foreign-data-wrapper']),
  'foreign-server': Object.freeze(['foreign-server']),
  language: Object.freeze(['language']),
  relation: Object.freeze(['table', 'partitioned-table', 'view', 'materialized-view', 'foreign-table', 'sequence']),
  routine: Object.freeze(['function', 'procedure', 'aggregate', 'window-function']),
  schema: Object.freeze(['schema']),
  type: Object.freeze(['base', 'composite', 'domain', 'enum', 'pseudo', 'range', 'multirange', 'array']),
});
const TABLE_PRIVILEGES = Object.freeze([
  'SELECT', 'INSERT', 'UPDATE', 'DELETE', 'TRUNCATE', 'REFERENCES', 'TRIGGER',
]);
const SESSION_PREAMBLE = String.raw`
BEGIN ISOLATION LEVEL SERIALIZABLE READ ONLY DEFERRABLE;
SET LOCAL search_path TO pg_catalog;
SET LOCAL row_security TO on;
SET LOCAL quote_all_identifiers TO off;
SET LOCAL client_encoding TO 'UTF8';
`;
const PROFILE_QUERY = String.raw`
SELECT pg_catalog.json_build_object(
  'serverVersion', current_setting('server_version'),
  'serverVersionNumber', current_setting('server_version_num')::integer,
  'serverEncoding', current_setting('server_encoding'),
  'databaseName', current_database(),
  'databaseOwner', pg_catalog.pg_get_userbyid(d.datdba),
  'databaseEncoding', pg_catalog.pg_encoding_to_char(d.encoding),
  'databaseCollate', d.datcollate,
  'databaseCtype', d.datctype,
  'databaseIcuLocale', d.daticulocale,
  'databaseLocaleProvider', d.datlocprovider,
  'currentUser', current_user,
  'sessionUser', session_user,
  'clientEncoding', current_setting('client_encoding'),
  'searchPath', current_setting('search_path'),
  'rowSecurity', current_setting('row_security'),
  'quoteAllIdentifiers', current_setting('quote_all_identifiers'),
  'transactionIsolation', current_setting('transaction_isolation'),
  'transactionReadOnly', current_setting('transaction_read_only'),
  'transactionDeferrable', current_setting('transaction_deferrable'),
  'schemas', (
    SELECT pg_catalog.json_agg(n.nspname ORDER BY n.nspname COLLATE "C")
    FROM pg_catalog.pg_namespace AS n
  ),
  'extensions', (
    SELECT pg_catalog.json_agg(
      pg_catalog.json_build_object(
        'name', e.extname, 'version', e.extversion, 'schema', n.nspname,
        'relocatable', e.extrelocatable, 'configIsNull', e.extconfig IS NULL,
        'conditionIsNull', e.extcondition IS NULL
      )
      ORDER BY e.extname COLLATE "C"
    ) FROM pg_catalog.pg_extension AS e
      JOIN pg_catalog.pg_namespace AS n ON n.oid = e.extnamespace
  ),
  'extensionMembers', (
    SELECT pg_catalog.json_agg(
      pg_catalog.pg_describe_object(d.classid, d.objid, d.objsubid)
      ORDER BY pg_catalog.pg_describe_object(d.classid, d.objid, d.objsubid) COLLATE "C"
    )
    FROM pg_catalog.pg_depend AS d
    JOIN pg_catalog.pg_extension AS e ON e.oid = d.refobjid
    WHERE d.refclassid = 'pg_catalog.pg_extension'::regclass
      AND e.extname = 'plpgsql' AND d.deptype = 'e'
  ),
  'defaultAclCount', (SELECT count(*) FROM pg_catalog.pg_default_acl),
  'parameterAclCount', (SELECT count(*) FROM pg_catalog.pg_parameter_acl),
  'foreignDataWrapperCount', (SELECT count(*) FROM pg_catalog.pg_foreign_data_wrapper),
  'foreignServerCount', (SELECT count(*) FROM pg_catalog.pg_foreign_server),
  'userMappingCount', (SELECT count(*) FROM pg_catalog.pg_user_mapping),
  'largeObjectCount', (SELECT count(*) FROM pg_catalog.pg_largeobject_metadata),
  'largeObjectPublicAtomCount', (
    SELECT count(*) FROM pg_catalog.pg_largeobject_metadata AS l
    CROSS JOIN LATERAL pg_catalog.aclexplode(
      CASE WHEN l.lomacl IS NULL THEN pg_catalog.acldefault('L', l.lomowner)
        WHEN pg_catalog.cardinality(l.lomacl) = 0 THEN NULL::aclitem[]
        ELSE l.lomacl END
    ) AS a WHERE a.grantee = 0
  ),
  'nonNullTrueArrayAclCount', (
    SELECT count(*) FROM pg_catalog.pg_type AS t
    WHERE t.typelem <> 0
      AND t.typsubscript = 'pg_catalog.array_subscript_handler'::regproc
      AND t.typacl IS NOT NULL
  ),
  'missingTrueArrayElementCount', (
    SELECT count(*) FROM pg_catalog.pg_type AS t
    LEFT JOIN pg_catalog.pg_type AS element ON element.oid = t.typelem
    WHERE t.typelem <> 0
      AND t.typsubscript = 'pg_catalog.array_subscript_handler'::regproc
      AND element.oid IS NULL
  ),
  'missingTypeElementCount', (
    SELECT count(*) FROM pg_catalog.pg_type AS t
    LEFT JOIN pg_catalog.pg_type AS element ON element.oid = t.typelem
    WHERE t.typelem <> 0 AND element.oid IS NULL
  ),
  'missingTypeSubscriptHandlerCount', (
    SELECT count(*) FROM pg_catalog.pg_type AS t
    LEFT JOIN pg_catalog.pg_proc AS handler ON handler.oid = t.typsubscript
    WHERE t.typsubscript <> 0 AND handler.oid IS NULL
  ),
  'unsupportedProcedureKindCount', (
    SELECT count(*) FROM pg_catalog.pg_proc WHERE prokind NOT IN ('f', 'p', 'a', 'w')
  ),
  'unsupportedTypeKindCount', (
    SELECT count(*) FROM pg_catalog.pg_type WHERE typtype NOT IN ('b', 'c', 'd', 'e', 'p', 'r', 'm')
  ),
  'unsupportedElementHandlerCount', (
    SELECT count(*) FROM pg_catalog.pg_type AS t
    JOIN pg_catalog.pg_proc AS p ON p.oid = t.typsubscript
    JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
    WHERE t.typelem <> 0 AND NOT (
      n.nspname = 'pg_catalog'
      AND p.proname IN ('array_subscript_handler', 'raw_array_subscript_handler')
    )
  ),
  'unknownRelationKindCount', (
    SELECT count(*) FROM pg_catalog.pg_class
    WHERE relkind NOT IN ('r', 'i', 'S', 't', 'v', 'm', 'c', 'f', 'p', 'I')
  ),
  'nonAclRelationPublicAtomCount', (
    SELECT count(*) FROM pg_catalog.pg_class AS c
    CROSS JOIN LATERAL pg_catalog.aclexplode(
      CASE WHEN pg_catalog.cardinality(c.relacl) = 0
        THEN NULL::aclitem[] ELSE c.relacl END
    ) AS a
    WHERE c.relkind NOT IN ('r', 'p', 'v', 'm', 'f', 'S') AND a.grantee = 0
  ),
  'invalidColumnPublicAtomCount', (
    SELECT count(*) FROM pg_catalog.pg_attribute AS att
    LEFT JOIN pg_catalog.pg_class AS c ON c.oid = att.attrelid
    CROSS JOIN LATERAL pg_catalog.aclexplode(
      CASE WHEN pg_catalog.cardinality(att.attacl) = 0
        THEN NULL::aclitem[] ELSE att.attacl END
    ) AS a
    WHERE (att.attnum <= 0 OR att.attisdropped OR c.oid IS NULL
      OR c.relkind NOT IN ('r', 'p', 'v', 'm', 'f'))
      AND a.grantee = 0
  ),
  'missingClassNamespaceCount', (
    SELECT count(*) FROM pg_catalog.pg_class AS c
    LEFT JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
    WHERE n.oid IS NULL
  ),
  'missingAttributeRelationCount', (
    SELECT count(*) FROM pg_catalog.pg_attribute AS att
    LEFT JOIN pg_catalog.pg_class AS c ON c.oid = att.attrelid
    WHERE c.oid IS NULL
  ),
  'missingProcedureNamespaceCount', (
    SELECT count(*) FROM pg_catalog.pg_proc AS p
    LEFT JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
    WHERE n.oid IS NULL
  ),
  'missingTypeNamespaceCount', (
    SELECT count(*) FROM pg_catalog.pg_type AS t
    LEFT JOIN pg_catalog.pg_namespace AS n ON n.oid = t.typnamespace
    WHERE n.oid IS NULL
  ),
  'dedicatedSchemaCount', (
    SELECT count(*) FROM pg_catalog.pg_namespace WHERE nspname = 'sf_supervisor_v1'
  ),
  'publicRelationCount', (
    SELECT count(*) FROM pg_catalog.pg_class AS c
    JOIN pg_catalog.pg_namespace AS n ON n.oid = c.relnamespace
    WHERE n.nspname = 'public'
  ),
  'publicRoutineCount', (
    SELECT count(*) FROM pg_catalog.pg_proc AS p
    JOIN pg_catalog.pg_namespace AS n ON n.oid = p.pronamespace
    WHERE n.nspname = 'public'
  ),
  'publicTypeCount', (
    SELECT count(*) FROM pg_catalog.pg_type AS t
    JOIN pg_catalog.pg_namespace AS n ON n.oid = t.typnamespace
    WHERE n.nspname = 'public'
  ),
  'publicNamespaceDependentObjectCount', (
    SELECT count(*) FROM pg_catalog.pg_depend AS d
    JOIN pg_catalog.pg_namespace AS n ON n.oid = d.refobjid
    WHERE d.refclassid = 'pg_catalog.pg_namespace'::regclass
      AND n.nspname = 'public'
  )
)::text
FROM pg_catalog.pg_database AS d
WHERE d.datname = current_database();
`;
const EXPECTED_PROFILE = Object.freeze({
  serverVersion: '16.15 (Debian 16.15-1.pgdg13+2)',
  serverVersionNumber: 160_015,
  serverEncoding: 'UTF8',
  databaseName: 'sf_public_baseline',
  databaseOwner: 'postgres',
  databaseEncoding: 'UTF8',
  databaseCollate: 'C',
  databaseCtype: 'C',
  databaseIcuLocale: null,
  databaseLocaleProvider: 'c',
  currentUser: 'postgres',
  sessionUser: 'postgres',
  clientEncoding: 'UTF8',
  searchPath: 'pg_catalog',
  rowSecurity: 'on',
  quoteAllIdentifiers: 'off',
  transactionIsolation: 'serializable',
  transactionReadOnly: 'on',
  transactionDeferrable: 'on',
  schemas: ['information_schema', 'pg_catalog', 'pg_toast', 'public'],
  extensions: [{
    name: 'plpgsql', version: '1.0', schema: 'pg_catalog', relocatable: false,
    configIsNull: true, conditionIsNull: true,
  }],
  extensionMembers: [
    'function plpgsql_call_handler()',
    'function plpgsql_inline_handler(internal)',
    'function plpgsql_validator(oid)',
    'language plpgsql',
  ],
  defaultAclCount: 0,
  parameterAclCount: 0,
  foreignDataWrapperCount: 0,
  foreignServerCount: 0,
  userMappingCount: 0,
  largeObjectCount: 0,
  largeObjectPublicAtomCount: 0,
  nonNullTrueArrayAclCount: 0,
  missingTrueArrayElementCount: 0,
  missingTypeElementCount: 0,
  missingTypeSubscriptHandlerCount: 0,
  unsupportedProcedureKindCount: 0,
  unsupportedTypeKindCount: 0,
  unsupportedElementHandlerCount: 0,
  unknownRelationKindCount: 0,
  nonAclRelationPublicAtomCount: 0,
  invalidColumnPublicAtomCount: 0,
  missingClassNamespaceCount: 0,
  missingAttributeRelationCount: 0,
  missingProcedureNamespaceCount: 0,
  missingTypeNamespaceCount: 0,
  dedicatedSchemaCount: 0,
  publicRelationCount: 0,
  publicRoutineCount: 0,
  publicTypeCount: 0,
  publicNamespaceDependentObjectCount: 0,
});

const args = process.argv.slice(2);
const writeFixture = args.includes('--write-fixture');
const positional = args.filter((value) => value !== '--write-fixture');
assert(positional.length === 1 && args.length === positional.length + Number(writeFixture),
  'CAPTURE_ARGUMENTS_INVALID');
const containerName = positional[0];
assert(typeof containerName === 'string'
  && /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/.test(containerName),
'CAPTURE_CONTAINER_INVALID');

const container = parseSingleJson(run('docker', ['inspect', containerName]),
  'CAPTURE_CONTAINER_INSPECT_INVALID');
assert(container.Name === `/${containerName}` && container.Config?.Image === IMAGE_REFERENCE
  && container.Image === IMAGE_CONFIGURATION && container.State?.Running === true,
'CAPTURE_CONTAINER_IDENTITY_INVALID');
assert(noPublishedPorts(container.HostConfig?.PortBindings)
  && noPublishedPorts(container.NetworkSettings?.Ports),
'CAPTURE_CONTAINER_PORTS_INVALID');
const dataMounts = Array.isArray(container.Mounts)
  ? container.Mounts.filter((mount) => mount?.Destination === '/var/lib/postgresql/data') : [];
assert(dataMounts.length === 1 && dataMounts[0]?.Type === 'volume'
  && typeof dataMounts[0]?.Name === 'string' && dataMounts[0].Name.length > 0,
'CAPTURE_CONTAINER_VOLUME_INVALID');
const env = Array.isArray(container.Config?.Env) ? container.Config.Env : [];
assert(env.includes('POSTGRES_INITDB_ARGS=--locale=C --encoding=UTF8')
  && env.includes('POSTGRES_HOST_AUTH_METHOD=trust'),
'CAPTURE_CONTAINER_ENVIRONMENT_INVALID');

const image = parseSingleJson(run('docker', ['image', 'inspect', IMAGE_CONFIGURATION]),
  'CAPTURE_IMAGE_INSPECT_INVALID');
assert(image.Id === IMAGE_CONFIGURATION && image.Os === 'linux' && image.Architecture === 'amd64'
  && Array.isArray(image.RepoDigests) && image.RepoDigests.includes(IMAGE_REFERENCE),
'CAPTURE_IMAGE_IDENTITY_INVALID');

const projection = readRegular(PROJECTION_PATH, 'CAPTURE_PROJECTION_INVALID');
const session = Buffer.concat([
  Buffer.from(`${SESSION_PREAMBLE}${PROFILE_QUERY}`, 'utf8'),
  projection,
  Buffer.from('\nROLLBACK;\n', 'utf8'),
]);
const sessionText = runPsql(containerName, session);
assert(sessionText.endsWith('\n'), 'CAPTURE_ROWS_TERMINATOR_INVALID');
const sessionLines = sessionText.slice(0, -1).split('\n');
assert(sessionLines.length > 1, 'CAPTURE_SESSION_CARDINALITY_INVALID');
const profile = parseJson(sessionLines[0], 'CAPTURE_PROFILE_JSON_INVALID');
assert(JSON.stringify(profile) === JSON.stringify(EXPECTED_PROFILE), 'CAPTURE_PROFILE_INVALID');
const rowLines = sessionLines.slice(1);
assert(rowLines.length > 0 && rowLines.length <= 8_192, 'CAPTURE_ROWS_CARDINALITY_INVALID');
const rows = rowLines.map((line) => parseJson(line, 'CAPTURE_ROW_JSON_INVALID'));
rows.forEach(validateRecord);
for (let index = 1; index < rows.length; index += 1) {
  assert(compareRecords(rows[index - 1], rows[index]) < 0, 'CAPTURE_ROWS_ORDER_INVALID');
}
assert(1 + (rows.length * 9) <= 65_536, 'CAPTURE_ROWS_NODES_INVALID');

const fixture = Buffer.from(`${JSON.stringify(rows)}\n`, 'utf8');
assert(fixture.byteLength > 0 && fixture.byteLength <= 1_048_576,
  'CAPTURE_FIXTURE_BYTES_INVALID');
if (existsSync(FIXTURE_PATH)) {
  const current = readRegular(FIXTURE_PATH, 'CAPTURE_EXISTING_FIXTURE_INVALID');
  assert(current.equals(fixture), 'CAPTURE_EXISTING_FIXTURE_MISMATCH');
} else if (writeFixture) {
  writeFileSync(FIXTURE_PATH, fixture, { flag: 'wx', mode: 0o600 });
  chmodSync(FIXTURE_PATH, 0o644);
} else {
  assert(false, 'CAPTURE_FIXTURE_ABSENT_USE_WRITE_FIXTURE');
}

const classCounts = Object.fromEntries(
  Object.keys(CLASS_KINDS).map((name) => [
    name, rows.filter((row) => row.objectClass === name).length,
  ]),
);
process.stdout.write(`${JSON.stringify({
  schemaVersion: 1,
  profile: 'postgresql-16.15-clean-template0-public-object-acl-v1',
  image: IMAGE_REFERENCE,
  imageConfiguration: IMAGE_CONFIGURATION,
  platform: 'linux/amd64',
  dataVolumeNameSha256: sha256(Buffer.from(dataMounts[0].Name, 'utf8')),
  profileSha256: sha256(Buffer.from(`${JSON.stringify(profile)}\n`, 'utf8')),
  projectionBytes: projection.byteLength,
  projectionSha256: sha256(projection),
  recordCount: rows.length,
  recordsBytes: fixture.byteLength,
  recordsSha256: sha256(fixture),
  classCounts,
}, null, 2)}\n`);

function validateRecord(row) {
  assert(row !== null && typeof row === 'object' && !Array.isArray(row)
    && Object.getPrototypeOf(row) === Object.prototype
    && JSON.stringify(Object.keys(row)) === JSON.stringify(RECORD_KEYS),
  'CAPTURE_ROW_SHAPE_INVALID');
  const kinds = CLASS_KINDS[row.objectClass];
  assert(Array.isArray(kinds) && kinds.includes(row.objectKind), 'CAPTURE_ROW_KIND_INVALID');
  assertIdentifier(row.objectName, 'CAPTURE_ROW_OBJECT_NAME_INVALID');
  const schemaBound = ['column', 'relation', 'routine', 'type'].includes(row.objectClass);
  assert(schemaBound ? isIdentifier(row.schemaName) : row.schemaName === null,
    'CAPTURE_ROW_SCHEMA_NAME_INVALID');
  assert(row.objectClass === 'column' ? isIdentifier(row.subobjectName)
    : row.subobjectName === null, 'CAPTURE_ROW_SUBOBJECT_NAME_INVALID');
  assert(row.objectClass === 'routine'
    ? typeof row.routineIdentityArguments === 'string'
      && Buffer.byteLength(row.routineIdentityArguments, 'utf8') <= 196_608
    : row.routineIdentityArguments === null,
  'CAPTURE_ROW_ROUTINE_ARGUMENTS_INVALID');
  assert(row.grantable === false, 'CAPTURE_ROW_GRANTABLE_INVALID');
  const allowed = privilegeSet(row.objectClass, row.objectKind);
  assert(allowed.includes(row.privilege), 'CAPTURE_ROW_PRIVILEGE_INVALID');
}

function privilegeSet(objectClass, objectKind) {
  if (objectClass === 'schema') return ['CREATE', 'USAGE'];
  if (objectClass === 'relation') {
    return objectKind === 'sequence' ? ['SELECT', 'UPDATE', 'USAGE'] : TABLE_PRIVILEGES;
  }
  if (objectClass === 'column') return ['INSERT', 'SELECT', 'UPDATE', 'REFERENCES'];
  if (objectClass === 'routine') return ['EXECUTE'];
  return ['USAGE'];
}

function compareRecords(left, right) {
  for (const key of RECORD_KEYS) {
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

function isIdentifier(value) {
  return typeof value === 'string' && value.length > 0 && !value.includes('\0')
    && Buffer.byteLength(value, 'utf8') <= 63;
}

function assertIdentifier(value, code) {
  assert(isIdentifier(value), code);
}

function runPsql(containerNameValue, input) {
  return run('docker', [
    'exec', '-i', containerNameValue, 'psql', '-U', 'postgres',
    '-d', 'sf_public_baseline', '-X', '-q', '-A', '-t', '-v', 'ON_ERROR_STOP=1',
  ], input);
}

function run(file, commandArgs, input = undefined) {
  const result = spawnSync(file, commandArgs, {
    input,
    maxBuffer: MAX_CAPTURE_BYTES,
    shell: false,
  });
  assert(result.error === undefined && result.signal === null && result.status === 0,
    'CAPTURE_COMMAND_FAILED');
  assert(Buffer.isBuffer(result.stdout) && Buffer.isBuffer(result.stderr)
    && result.stderr.byteLength === 0, 'CAPTURE_COMMAND_OUTPUT_INVALID');
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(result.stdout);
  } catch {
    throw new Error('CAPTURE_COMMAND_UTF8_INVALID');
  }
}

function readRegular(path, code) {
  const stat = lstatSync(path);
  assert(stat.isFile() && !stat.isSymbolicLink() && stat.nlink === 1, code);
  return readFileSync(path);
}

function parseSingleJson(value, code) {
  const parsed = parseJson(value, code);
  assert(Array.isArray(parsed) && parsed.length === 1, code);
  return parsed[0];
}

function parseJson(value, code) {
  try {
    return JSON.parse(value);
  } catch {
    throw new Error(code);
  }
}

function nonemptyLines(value) {
  return value.split('\n').filter((line) => line.length > 0);
}

function noPublishedPorts(value) {
  if (value === null || value === undefined) return true;
  if (typeof value !== 'object' || Array.isArray(value)) return false;
  return Object.values(value).every((entry) => entry === null
    || (Array.isArray(entry) && entry.length === 0));
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex');
}

function assert(condition, code) {
  if (!condition) throw new Error(code);
}
