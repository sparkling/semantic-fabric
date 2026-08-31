// SPDX-License-Identifier: MIT

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  buildPublicAclProjectionFinalWhereMutantsV1,
} from './postgresql-public-acl-projection-final-where-mutations-v1.mjs';
import {
  FINAL_WHERE_MUTATION_BATCHES_V1, analyzeFinalWhereBatchTranscriptV1,
  combineFinalWhereMutationBatchReceiptsV1, validateFinalWhereMutationReceiptV1,
} from './postgresql-public-acl-final-where-mutation-oracle-v1.mjs';
import { sha256 } from './postgresql-public-acl-oracle-v1.mjs';
import { assert } from './postgresql-public-acl-oracle-wire-v1.mjs';
import {
  decodeMutationUtf8, inspectMutationContainer, readMutationSource, runMutationPsql,
} from './postgresql-public-acl-mutation-replay-support-v1.mjs';
import {
  IMAGE_CONFIGURATION, IMAGE_REFERENCE, runOwnedReplayPair,
} from './postgresql-public-acl-replay-support-v2.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const SELF_PATH = 'scripts/verify-postgresql-public-acl-projection-final-where-mutations-v1.mjs';
const MUTATOR_PATH = 'scripts/postgresql-public-acl-projection-final-where-mutations-v1.mjs';
const ORACLE_PATH = 'scripts/postgresql-public-acl-final-where-mutation-oracle-v1.mjs';
const SUPPORT_PATH = 'scripts/postgresql-public-acl-mutation-replay-support-v1.mjs';
const PROJECTION_PATH = '__tests__/fixtures/postgresql-16.15-public-acl-projection-v1.sql';
const RAW_ORACLE_PATH = '__tests__/fixtures/postgresql-16.15-public-acl-completeness-oracle-v1.sql';
const PROJECTION_PIN = Object.freeze({ bytes: 6_859,
  sha256: '0e3ad724f4ce85191564c245c51dd7665b6d9aa704c355067a0056cdbfe95232' });
const RAW_ORACLE_PIN = Object.freeze({ bytes: 16_037,
  sha256: '6a1cf204ca8c5a3aa7a70da4f5c8c46cd15998b745d2eb648b77568e6c912722' });
const VERIFIER = 'postgresql-16.15-public-acl-projection-final-where-mutations-v1';
const READ_ONLY_BEGIN = 'BEGIN ISOLATION LEVEL SERIALIZABLE READ ONLY DEFERRABLE;\n';
const ROLLBACK = 'ROLLBACK;\n';
const MAX_TRANSCRIPT_BYTES = 16 * 1024 * 1024;
const CHILD_PATHS = Object.freeze({
  capture: 'scripts/capture-postgresql-public-acl-baseline-v1.mjs',
  oracle: 'scripts/verify-postgresql-public-acl-oracle-v1.mjs', witness: SELF_PATH,
});

const VISIBLE_SEED_SQL = `CREATE FOREIGN DATA WRAPPER sf_public_acl_mutation_fdw_v1
  NO HANDLER NO VALIDATOR;
CREATE SERVER sf_public_acl_mutation_server_v1
  FOREIGN DATA WRAPPER sf_public_acl_mutation_fdw_v1;
GRANT USAGE ON FOREIGN DATA WRAPPER sf_public_acl_mutation_fdw_v1 TO PUBLIC;
GRANT USAGE ON FOREIGN SERVER sf_public_acl_mutation_server_v1 TO PUBLIC;
`;

const HIDDEN_SEED_SQL = `CREATE ROLE sf_acl_private_role_v1 NOLOGIN;
CREATE SCHEMA sf_acl_hidden_schema_v1 AUTHORIZATION postgres;
REVOKE ALL PRIVILEGES ON SCHEMA sf_acl_hidden_schema_v1 FROM PUBLIC;
GRANT USAGE ON SCHEMA sf_acl_hidden_schema_v1 TO sf_acl_private_role_v1;
CREATE SCHEMA sf_supervisor_v1 AUTHORIZATION postgres;
GRANT USAGE ON SCHEMA sf_supervisor_v1 TO PUBLIC;
CREATE TABLE sf_acl_hidden_schema_v1.sf_acl_private_table_v1 (
  private_col integer, dropped_col integer);
REVOKE ALL PRIVILEGES ON TABLE sf_acl_hidden_schema_v1.sf_acl_private_table_v1 FROM PUBLIC;
GRANT SELECT ON TABLE sf_acl_hidden_schema_v1.sf_acl_private_table_v1 TO sf_acl_private_role_v1;
GRANT SELECT (private_col) ON sf_acl_hidden_schema_v1.sf_acl_private_table_v1
  TO sf_acl_private_role_v1;
GRANT SELECT (ctid) ON sf_acl_hidden_schema_v1.sf_acl_private_table_v1 TO PUBLIC;
GRANT SELECT (dropped_col) ON sf_acl_hidden_schema_v1.sf_acl_private_table_v1 TO PUBLIC;
ALTER TABLE sf_acl_hidden_schema_v1.sf_acl_private_table_v1 DROP COLUMN dropped_col;
REVOKE ALL PRIVILEGES ON TYPE sf_acl_hidden_schema_v1.sf_acl_private_table_v1 FROM PUBLIC;
GRANT USAGE ON TYPE sf_acl_hidden_schema_v1.sf_acl_private_table_v1 TO sf_acl_private_role_v1;
CREATE TABLE public.sf_acl_public_table_v1 (public_col integer);
GRANT SELECT ON TABLE public.sf_acl_public_table_v1 TO PUBLIC;
GRANT UPDATE (public_col) ON public.sf_acl_public_table_v1 TO PUBLIC;
CREATE FUNCTION sf_acl_hidden_schema_v1.sf_acl_private_function_v1()
  RETURNS integer LANGUAGE sql AS $$SELECT 1$$;
REVOKE ALL PRIVILEGES ON FUNCTION sf_acl_hidden_schema_v1.sf_acl_private_function_v1() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION sf_acl_hidden_schema_v1.sf_acl_private_function_v1()
  TO sf_acl_private_role_v1;
CREATE FUNCTION public.sf_acl_public_function_v1() RETURNS integer LANGUAGE sql AS $$SELECT 1$$;
CREATE DOMAIN sf_acl_hidden_schema_v1.sf_acl_private_domain_v1 AS integer;
REVOKE ALL PRIVILEGES ON TYPE sf_acl_hidden_schema_v1.sf_acl_private_domain_v1 FROM PUBLIC;
GRANT USAGE ON TYPE sf_acl_hidden_schema_v1.sf_acl_private_domain_v1 TO sf_acl_private_role_v1;
CREATE DOMAIN public.sf_acl_public_domain_v1 AS integer;
CREATE TRUSTED PROCEDURAL LANGUAGE sf_acl_hidden_language_v1
  HANDLER pg_catalog.plpgsql_call_handler INLINE pg_catalog.plpgsql_inline_handler
  VALIDATOR pg_catalog.plpgsql_validator;
REVOKE ALL PRIVILEGES ON LANGUAGE sf_acl_hidden_language_v1 FROM PUBLIC;
GRANT USAGE ON LANGUAGE sf_acl_hidden_language_v1 TO sf_acl_private_role_v1;
CREATE FOREIGN DATA WRAPPER sf_acl_hidden_fdw_v1 NO HANDLER NO VALIDATOR;
GRANT USAGE ON FOREIGN DATA WRAPPER sf_acl_hidden_fdw_v1 TO sf_acl_private_role_v1;
CREATE SERVER sf_acl_hidden_server_v1 FOREIGN DATA WRAPPER sf_acl_hidden_fdw_v1;
GRANT USAGE ON FOREIGN SERVER sf_acl_hidden_server_v1 TO sf_acl_private_role_v1;
`;

const ROLE_OID = "(SELECT oid FROM pg_catalog.pg_roles WHERE rolname = 'sf_acl_private_role_v1')";
const SEED_WITNESSES = Object.freeze([
  ['schemaPrivate', `SELECT count(*) FROM pg_catalog.pg_namespace n CROSS JOIN LATERAL
    pg_catalog.aclexplode(n.nspacl) a WHERE n.nspname='sf_acl_hidden_schema_v1'
    AND a.grantee=${ROLE_OID}`],
  ['schemaExcluded', `SELECT count(*) FROM pg_catalog.pg_namespace n CROSS JOIN LATERAL
    pg_catalog.aclexplode(n.nspacl) a WHERE n.nspname='sf_supervisor_v1' AND a.grantee=0`],
  ['relationPrivate', `SELECT count(*) FROM pg_catalog.pg_class c CROSS JOIN LATERAL
    pg_catalog.aclexplode(c.relacl) a WHERE c.relname='sf_acl_private_table_v1'
    AND a.grantee=${ROLE_OID}`],
  ['relationExcluded', `SELECT count(*) FROM pg_catalog.pg_class c CROSS JOIN LATERAL
    pg_catalog.aclexplode(c.relacl) a WHERE c.relname='sf_acl_public_table_v1' AND a.grantee=0`],
  ['columnSystem', `SELECT count(*) FROM pg_catalog.pg_attribute x JOIN pg_catalog.pg_class c
    ON c.oid=x.attrelid CROSS JOIN LATERAL pg_catalog.aclexplode(x.attacl) a
    WHERE c.relname='sf_acl_private_table_v1' AND x.attnum<0 AND a.grantee=0`],
  ['columnDropped', `SELECT count(*) FROM pg_catalog.pg_attribute x JOIN pg_catalog.pg_class c
    ON c.oid=x.attrelid CROSS JOIN LATERAL pg_catalog.aclexplode(x.attacl) a
    WHERE c.relname='sf_acl_private_table_v1' AND x.attisdropped AND a.grantee=0`],
  ['columnPrivate', `SELECT count(*) FROM pg_catalog.pg_attribute x JOIN pg_catalog.pg_class c
    ON c.oid=x.attrelid CROSS JOIN LATERAL pg_catalog.aclexplode(x.attacl) a
    WHERE c.relname='sf_acl_private_table_v1' AND x.attname='private_col'
    AND a.grantee=${ROLE_OID}`],
  ['columnExcluded', `SELECT count(*) FROM pg_catalog.pg_attribute x JOIN pg_catalog.pg_class c
    ON c.oid=x.attrelid CROSS JOIN LATERAL pg_catalog.aclexplode(x.attacl) a
    WHERE c.relname='sf_acl_public_table_v1' AND x.attname='public_col' AND a.grantee=0`],
  ['routinePrivate', `SELECT count(*) FROM pg_catalog.pg_proc p CROSS JOIN LATERAL
    pg_catalog.aclexplode(p.proacl) a WHERE p.proname='sf_acl_private_function_v1'
    AND a.grantee=${ROLE_OID}`],
  ['routineExcluded', `SELECT count(*) FROM pg_catalog.pg_proc p CROSS JOIN LATERAL
    pg_catalog.aclexplode(COALESCE(p.proacl,pg_catalog.acldefault('f',p.proowner))) a
    WHERE p.proname='sf_acl_public_function_v1' AND a.grantee=0`],
  ['typePrivate', `SELECT count(*) FROM pg_catalog.pg_type t CROSS JOIN LATERAL
    pg_catalog.aclexplode(t.typacl) a WHERE t.typname='sf_acl_private_domain_v1'
    AND a.grantee=${ROLE_OID}`],
  ['typeExcluded', `SELECT count(*) FROM pg_catalog.pg_type t CROSS JOIN LATERAL
    pg_catalog.aclexplode(COALESCE(t.typacl,pg_catalog.acldefault('T',t.typowner))) a
    WHERE t.typname='sf_acl_public_domain_v1' AND a.grantee=0`],
  ['languagePrivate', `SELECT count(*) FROM pg_catalog.pg_language l CROSS JOIN LATERAL
    pg_catalog.aclexplode(l.lanacl) a WHERE l.lanname='sf_acl_hidden_language_v1'
    AND a.grantee=${ROLE_OID}`],
  ['fdwPrivate', `SELECT count(*) FROM pg_catalog.pg_foreign_data_wrapper f CROSS JOIN LATERAL
    pg_catalog.aclexplode(f.fdwacl) a WHERE f.fdwname='sf_acl_hidden_fdw_v1'
    AND a.grantee=${ROLE_OID}`],
  ['serverPrivate', `SELECT count(*) FROM pg_catalog.pg_foreign_server s CROSS JOIN LATERAL
    pg_catalog.aclexplode(s.srvacl) a WHERE s.srvname='sf_acl_hidden_server_v1'
    AND a.grantee=${ROLE_OID}`],
]);

const GUARD_WITNESSES = Object.freeze([
  ['relationUnsupported', `SELECT count(*) FROM pg_catalog.pg_class c
    JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace CROSS JOIN LATERAL
    pg_catalog.aclexplode(c.relacl) a WHERE c.relkind NOT IN ('r','p','v','m','f','S')
    AND a.grantee=0 AND n.nspname NOT IN ('public','sf_supervisor_v1')`],
  ['columnUnsupportedParent', `SELECT count(*) FROM pg_catalog.pg_attribute x
    JOIN pg_catalog.pg_class c ON c.oid=x.attrelid JOIN pg_catalog.pg_namespace n
    ON n.oid=c.relnamespace CROSS JOIN LATERAL pg_catalog.aclexplode(x.attacl) a
    WHERE x.attnum>0 AND NOT x.attisdropped AND c.relkind NOT IN ('r','p','v','m','f')
    AND a.grantee=0 AND n.nspname NOT IN ('public','sf_supervisor_v1')`],
  ['routineUnsupported', `SELECT count(*) FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n
    ON n.oid=p.pronamespace CROSS JOIN LATERAL pg_catalog.aclexplode(CASE
    WHEN p.proacl IS NULL THEN pg_catalog.acldefault('f',p.proowner)
    WHEN pg_catalog.cardinality(p.proacl)=0 THEN NULL::aclitem[] ELSE p.proacl END) a
    WHERE p.prokind NOT IN ('f','p','a','w') AND a.grantee=0
    AND n.nspname NOT IN ('public','sf_supervisor_v1')`],
  ['typeUnsupported', `SELECT count(*) FROM pg_catalog.pg_type t JOIN pg_catalog.pg_namespace n
    ON n.oid=t.typnamespace CROSS JOIN LATERAL pg_catalog.aclexplode(CASE
    WHEN t.typacl IS NULL THEN pg_catalog.acldefault('T',t.typowner)
    WHEN pg_catalog.cardinality(t.typacl)=0 THEN NULL::aclitem[] ELSE t.typacl END) a
    WHERE t.typtype NOT IN ('b','c','d','e','p','r','m') AND a.grantee=0
    AND n.nspname NOT IN ('public','sf_supervisor_v1')`],
]);

if (typeof process.argv[1] === 'string'
  && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();

function main() {
  if (process.argv.length === 2) runOuter();
  else if (process.argv.length === 3) runChild(process.argv[2]);
  else throw new Error('ACL_FINAL_WHERE_ARGUMENTS_INVALID');
}

function runOuter() {
  const runs = runOwnedReplayPair(ROOT, CHILD_PATHS);
  assert(Array.isArray(runs) && runs.length === 2, 'ACL_FINAL_WHERE_RUNS_INVALID');
  runs.forEach((run, index) => validateOwnedRun(run, index + 1));
  const deterministic = withoutVolume(runs[0].witness);
  assert(JSON.stringify(deterministic) === JSON.stringify(withoutVolume(runs[1].witness))
    && runs[0].witness.dataVolumeNameSha256 !== runs[1].witness.dataVolumeNameSha256,
  'ACL_FINAL_WHERE_RUNS_NONDETERMINISTIC');
  process.stdout.write(`${JSON.stringify({
    schemaVersion: 1, replay: VERIFIER, authority: 'test-only-non-runtime',
    image: IMAGE_REFERENCE, imageConfiguration: IMAGE_CONFIGURATION, platform: 'linux/amd64',
    ownedReplay: { runs: 2, networkMode: 'none', publishedPorts: false,
      distinctAnonymousVolumes: true, cleanupVerified: true },
    classification: { executed: 19, killed: 15, guardEquivalent: 4, unresolved: 0 },
    runs: runs.map((run) => ({ sequence: run.sequence,
      dataVolumeNameSha256: run.witness.dataVolumeNameSha256,
      batchTranscriptBytes: run.witness.proof.batches.map(({ transcriptBytes }) => transcriptBytes),
      batchTranscriptSha256: run.witness.proof.batches.map(({ transcriptSha256 }) => transcriptSha256),
    })), deterministic,
  }, null, 2)}\n`);
}

function runChild(containerName) {
  const container = inspectMutationContainer(containerName);
  const sources = readSources();
  const projection = decodeMutationUtf8(sources.projection.bytes,
    'ACL_FINAL_WHERE_PROJECTION_UTF8_INVALID');
  const rawOracle = decodeMutationUtf8(sources.rawOracle.bytes,
    'ACL_FINAL_WHERE_RAW_ORACLE_UTF8_INVALID');
  const catalogue = buildPublicAclProjectionFinalWhereMutantsV1(projection);
  assertAbsent(container.id, 'ACL_FINAL_WHERE_SEED_PREEXISTED');
  let proof;
  let primaryFailure;
  try {
    const batches = FINAL_WHERE_MUTATION_BATCHES_V1.map((batch) => {
      const session = buildSession(rawOracle, projection, catalogue, batch);
      return analyzeFinalWhereBatchTranscriptV1(
        runPsql(container.id, session, MAX_TRANSCRIPT_BYTES, 'session'), batch, catalogue);
    });
    proof = combineFinalWhereMutationBatchReceiptsV1(batches);
  } catch (error) { primaryFailure = error; }
  let postflightFailure;
  try { assertAbsent(container.id, 'ACL_FINAL_WHERE_SEED_POSTFLIGHT_INVALID'); }
  catch (error) { postflightFailure = error; }
  if (primaryFailure !== undefined) throw new Error(`${errorCode(primaryFailure)}${
    postflightFailure === undefined ? '' : `;${errorCode(postflightFailure)}`}`,
  { cause: primaryFailure });
  if (postflightFailure !== undefined) throw postflightFailure;
  const result = {
    schemaVersion: 1, verifier: VERIFIER, authority: 'test-only-non-runtime',
    image: IMAGE_REFERENCE, imageConfiguration: IMAGE_CONFIGURATION, platform: 'linux/amd64',
    dataVolumeNameSha256: sha256(Buffer.from(container.volumeName, 'utf8')),
    sources: sourceSummary(sources), proof,
    cleanup: { transactionsRolledBack: 2, hiddenObjectsAbsent: true },
  };
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

function buildSession(rawOracle, projection, catalogue, batch) {
  assert(rawOracle.split(READ_ONLY_BEGIN).length === 2 && rawOracle.endsWith(ROLLBACK),
    'ACL_FINAL_WHERE_RAW_ORACLE_ENVELOPE_INVALID');
  const raw = rawOracle.replace(READ_ONLY_BEGIN,
    `BEGIN ISOLATION LEVEL SERIALIZABLE;\n${VISIBLE_SEED_SQL}`).slice(0, -ROLLBACK.length);
  const byId = new Map(catalogue.mutants.map((mutant) => [mutant.id, mutant]));
  const sections = [['original', projection], ...batch.mutantIds.map((id) => {
    const mutant = byId.get(id); assert(mutant !== undefined, 'ACL_FINAL_WHERE_BATCH_ID_INVALID');
    return [id, mutant.source];
  })];
  return `${raw}${HIDDEN_SEED_SQL}${witnessSection('seed-witness', SEED_WITNESSES)}`
    + `${witnessSection('guard-witness', GUARD_WITNESSES)}${sections.map(([id, sql]) =>
      `${markerCommand(id, 'BEGIN')}${sql}${markerCommand(id, 'END')}`).join('')}${ROLLBACK}`;
}

function witnessSection(id, queries) {
  const pairs = queries.map(([key, query]) => `  '${key}', (${query})`).join(',\n');
  return `${markerCommand(id, 'BEGIN')}COPY (SELECT pg_catalog.json_build_object(\n${pairs}\n)::text)`
    + ` TO STDOUT;\n${markerCommand(id, 'END')}`;
}

function validateOwnedRun(run, sequence) {
  assert(run?.sequence === sequence && run.capture !== null && run.oracle !== null
    && run.witness !== null, 'ACL_FINAL_WHERE_OWNED_RUN_INVALID');
  const baseline = [run.capture.recordCount, run.capture.recordsBytes, run.capture.recordsSha256];
  assert(JSON.stringify(baseline) === JSON.stringify([
    run.oracle.recordCount, run.oracle.recordsBytes, run.oracle.recordsSha256,
  ]) && run.capture.projectionBytes === PROJECTION_PIN.bytes
    && run.capture.projectionSha256 === PROJECTION_PIN.sha256
    && run.oracle.projectionSourceBytes === PROJECTION_PIN.bytes
    && run.oracle.projectionSourceSha256 === PROJECTION_PIN.sha256
    && run.oracle.oracleSourceBytes === RAW_ORACLE_PIN.bytes
    && run.oracle.oracleSourceSha256 === RAW_ORACLE_PIN.sha256,
  'ACL_FINAL_WHERE_CLEAN_CONTROL_INVALID');
  validateChild(run.witness);
  assert(run.capture.dataVolumeNameSha256 === run.oracle.dataVolumeNameSha256
    && run.oracle.dataVolumeNameSha256 === run.witness.dataVolumeNameSha256,
  'ACL_FINAL_WHERE_VOLUME_EVIDENCE_INVALID');
}

function validateChild(value) {
  exactObject(value, ['schemaVersion', 'verifier', 'authority', 'image', 'imageConfiguration',
    'platform', 'dataVolumeNameSha256', 'sources', 'proof', 'cleanup'],
  'ACL_FINAL_WHERE_CHILD_INVALID');
  assert(value.schemaVersion === 1 && value.verifier === VERIFIER
    && value.authority === 'test-only-non-runtime' && value.image === IMAGE_REFERENCE
    && value.imageConfiguration === IMAGE_CONFIGURATION && value.platform === 'linux/amd64'
    && digest(value.dataVolumeNameSha256), 'ACL_FINAL_WHERE_CHILD_INVALID');
  exactObject(value.sources, ['projection', 'rawOracle', 'mutator', 'oracle', 'replaySupport',
    'verifier'], 'ACL_FINAL_WHERE_CHILD_SOURCES_INVALID');
  Object.values(value.sources).forEach((source) => {
    exactObject(source, ['bytes', 'sha256'], 'ACL_FINAL_WHERE_CHILD_SOURCE_INVALID');
    assert(positive(source.bytes) && digest(source.sha256), 'ACL_FINAL_WHERE_CHILD_SOURCE_INVALID');
  });
  assert(value.sources.projection.bytes === PROJECTION_PIN.bytes
    && value.sources.projection.sha256 === PROJECTION_PIN.sha256
    && value.sources.rawOracle.bytes === RAW_ORACLE_PIN.bytes
    && value.sources.rawOracle.sha256 === RAW_ORACLE_PIN.sha256,
  'ACL_FINAL_WHERE_CHILD_SOURCE_PIN_INVALID');
  validateFinalWhereMutationReceiptV1(value.proof);
  exactObject(value.cleanup, ['transactionsRolledBack', 'hiddenObjectsAbsent'],
    'ACL_FINAL_WHERE_CHILD_CLEANUP_INVALID');
  assert(value.cleanup.transactionsRolledBack === 2 && value.cleanup.hiddenObjectsAbsent === true,
    'ACL_FINAL_WHERE_CHILD_CLEANUP_INVALID');
}

function readSources() {
  const projection = readMutationSource(ROOT, PROJECTION_PATH, 64 * 1024);
  const rawOracle = readMutationSource(ROOT, RAW_ORACLE_PATH, 64 * 1024);
  assert(projection.byteLength === PROJECTION_PIN.bytes && sha256(projection) === PROJECTION_PIN.sha256,
    'ACL_FINAL_WHERE_PROJECTION_PIN_INVALID');
  assert(rawOracle.byteLength === RAW_ORACLE_PIN.bytes && sha256(rawOracle) === RAW_ORACLE_PIN.sha256,
    'ACL_FINAL_WHERE_RAW_ORACLE_PIN_INVALID');
  return Object.freeze({ projection: { bytes: projection }, rawOracle: { bytes: rawOracle },
    mutator: { bytes: readMutationSource(ROOT, MUTATOR_PATH, 64 * 1024) },
    oracle: { bytes: readMutationSource(ROOT, ORACLE_PATH, 64 * 1024) },
    replaySupport: { bytes: readMutationSource(ROOT, SUPPORT_PATH, 64 * 1024) },
    verifier: { bytes: readMutationSource(ROOT, SELF_PATH, 64 * 1024) } });
}

function sourceSummary(sources) {
  return Object.fromEntries(Object.entries(sources).map(([key, { bytes }]) =>
    [key, { bytes: bytes.byteLength, sha256: sha256(bytes) }]));
}

function assertAbsent(id, code) {
  const source = `SELECT ((SELECT count(*) FROM pg_catalog.pg_roles WHERE rolname='sf_acl_private_role_v1')
    +(SELECT count(*) FROM pg_catalog.pg_namespace WHERE nspname IN
      ('sf_acl_hidden_schema_v1','sf_supervisor_v1'))
    +(SELECT count(*) FROM pg_catalog.pg_class WHERE relname IN
      ('sf_acl_private_table_v1','sf_acl_public_table_v1'))
    +(SELECT count(*) FROM pg_catalog.pg_proc WHERE proname IN
      ('sf_acl_private_function_v1','sf_acl_public_function_v1'))
    +(SELECT count(*) FROM pg_catalog.pg_type WHERE typname IN
      ('sf_acl_private_domain_v1','sf_acl_public_domain_v1'))
    +(SELECT count(*) FROM pg_catalog.pg_language WHERE lanname='sf_acl_hidden_language_v1')
    +(SELECT count(*) FROM pg_catalog.pg_foreign_data_wrapper WHERE fdwname IN
      ('sf_public_acl_mutation_fdw_v1','sf_acl_hidden_fdw_v1'))
    +(SELECT count(*) FROM pg_catalog.pg_foreign_server WHERE srvname IN
      ('sf_public_acl_mutation_server_v1','sf_acl_hidden_server_v1')))::text;\n`;
  assert(runPsql(id, source, 64 * 1024, 'probe').equals(Buffer.from('0\n')), code);
}

function runPsql(id, source, maxBuffer, operation) {
  return runMutationPsql(ROOT, id, Buffer.from(source, 'utf8'), maxBuffer, operation);
}

function markerCommand(id, edge) {
  assert(/^[a-z0-9-]+$/.test(id) && (edge === 'BEGIN' || edge === 'END'),
    'ACL_FINAL_WHERE_SECTION_ID_INVALID');
  return `\\echo @@ADR0047-FINAL-WHERE-V1/${id}/${edge}@@\n`;
}

function withoutVolume(value) {
  const { dataVolumeNameSha256: _volume, ...result } = value; return result;
}

function exactObject(value, keys, code) {
  assert(value !== null && typeof value === 'object' && !Array.isArray(value)
    && Object.getPrototypeOf(value) === Object.prototype
    && JSON.stringify(Object.keys(value)) === JSON.stringify(keys), code);
}
function digest(value) { return typeof value === 'string' && /^[0-9a-f]{64}$/.test(value); }
function positive(value) { return Number.isSafeInteger(value) && value > 0; }
function errorCode(error) {
  return error instanceof Error && /^ACL_FINAL_WHERE_[A-Z0-9_]+$/.test(error.message)
    ? error.message : 'ACL_FINAL_WHERE_INTERNAL_ERROR';
}
