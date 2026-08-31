// SPDX-License-Identifier: MIT

import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  PUBLIC_ACL_OBJECT_CLASSES_V1, buildPublicAclProjectionBranchDeletionMutantsV1,
  buildPublicAclProjectionRecordSetMutantsV1,
} from './postgresql-public-acl-projection-branch-mutations-v1.mjs';
import {
  canonicalFixture, compareRecordBags, compareRecords, deriveOracleRecords,
  parseProjectionRecords, sha256,
} from './postgresql-public-acl-oracle-v1.mjs';
import { assert, hex, parseOracleSession } from './postgresql-public-acl-oracle-wire-v1.mjs';
import {
  decodeMutationUtf8, inspectMutationContainer, readMutationSource, runMutationPsql,
} from './postgresql-public-acl-mutation-replay-support-v1.mjs';
import {
  IMAGE_CONFIGURATION, IMAGE_REFERENCE, runOwnedReplayPair,
} from './postgresql-public-acl-replay-support-v2.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const SELF_PATH = 'scripts/verify-postgresql-public-acl-projection-branch-mutations-v1.mjs';
const MUTATOR_PATH = 'scripts/postgresql-public-acl-projection-branch-mutations-v1.mjs';
const SUPPORT_PATH = 'scripts/postgresql-public-acl-mutation-replay-support-v1.mjs';
const PROJECTION_PATH = '__tests__/fixtures/postgresql-16.15-public-acl-projection-v1.sql';
const RAW_ORACLE_PATH =
  '__tests__/fixtures/postgresql-16.15-public-acl-completeness-oracle-v1.sql';
const PROJECTION_PIN = Object.freeze({
  bytes: 6_859,
  sha256: '0e3ad724f4ce85191564c245c51dd7665b6d9aa704c355067a0056cdbfe95232',
});
const RAW_ORACLE_PIN = Object.freeze({
  bytes: 16_037,
  sha256: '6a1cf204ca8c5a3aa7a70da4f5c8c46cd15998b745d2eb648b77568e6c912722',
});
const VERIFIER = 'postgresql-16.15-public-acl-projection-mutations-v1';
const FDW_NAME = 'sf_public_acl_mutation_fdw_v1';
const SERVER_NAME = 'sf_public_acl_mutation_server_v1';
const SEED_SQL = `CREATE FOREIGN DATA WRAPPER sf_public_acl_mutation_fdw_v1
  NO HANDLER NO VALIDATOR;
CREATE SERVER sf_public_acl_mutation_server_v1
  FOREIGN DATA WRAPPER sf_public_acl_mutation_fdw_v1;
GRANT USAGE ON FOREIGN DATA WRAPPER sf_public_acl_mutation_fdw_v1 TO PUBLIC;
GRANT USAGE ON FOREIGN SERVER sf_public_acl_mutation_server_v1 TO PUBLIC;
`;
const READ_ONLY_BEGIN = 'BEGIN ISOLATION LEVEL SERIALIZABLE READ ONLY DEFERRABLE;\n';
const ROLLBACK = 'ROLLBACK;\n';
const RAW_END = '@@ADR0047-RAW-V1/CONTROL/END@@\n';
const PROJECTION_BEGIN = '@@ADR0047-PROJECTION/BEGIN@@\n';
const PROJECTION_END = '@@ADR0047-PROJECTION/END@@\n';
const MAX_TRANSCRIPT_BYTES = 16 * 1024 * 1024;
const CHILD_PATHS = Object.freeze({
  capture: 'scripts/capture-postgresql-public-acl-baseline-v1.mjs',
  oracle: 'scripts/verify-postgresql-public-acl-oracle-v1.mjs',
  witness: SELF_PATH,
});

if (typeof process.argv[1] === 'string'
  && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();

function main() {
  if (process.argv.length === 2) runOuter();
  else if (process.argv.length === 3) runChild(process.argv[2]);
  else throw new Error('ACL_MUTATION_ARGUMENTS_INVALID');
}

function runOuter() {
  const runs = runOwnedReplayPair(ROOT, CHILD_PATHS);
  assert(Array.isArray(runs) && runs.length === 2, 'ACL_MUTATION_RUNS_INVALID');
  runs.forEach((run, index) => validateOwnedRun(run, index + 1));
  const first = withoutVolume(runs[0].witness);
  const second = withoutVolume(runs[1].witness);
  assert(JSON.stringify(first) === JSON.stringify(second),
    'ACL_MUTATION_RUNS_NONDETERMINISTIC');
  assert(runs[0].witness.dataVolumeNameSha256
    !== runs[1].witness.dataVolumeNameSha256, 'ACL_MUTATION_VOLUMES_NOT_DISTINCT');
  process.stdout.write(`${JSON.stringify({
    schemaVersion: 1,
    replay: VERIFIER,
    authority: 'test-only-non-runtime',
    image: IMAGE_REFERENCE,
    imageConfiguration: IMAGE_CONFIGURATION,
    platform: 'linux/amd64',
    ownedReplay: {
      runs: 2, networkMode: 'none', publishedPorts: false,
      distinctAnonymousVolumes: true, cleanupVerified: true,
    },
    cleanControl: {
      recordCount: runs[0].capture.recordCount,
      recordsBytes: runs[0].capture.recordsBytes,
      recordsSha256: runs[0].capture.recordsSha256,
    },
    runs: runs.map((run) => ({
      sequence: run.sequence,
      dataVolumeNameSha256: run.witness.dataVolumeNameSha256,
      sessionTranscriptBytes: run.witness.sessionTranscriptBytes,
      sessionTranscriptSha256: run.witness.sessionTranscriptSha256,
      killedMutants: run.witness.mutations.length,
    })),
    deterministic: first,
  }, null, 2)}\n`);
}

function runChild(containerName) {
  const container = inspectMutationContainer(containerName);
  const sources = readSources();
  const projectionText = decodeMutationUtf8(sources.projection.bytes,
    'ACL_MUTATION_PROJECTION_UTF8_INVALID');
  const rawOracleText = decodeMutationUtf8(sources.rawOracle.bytes,
    'ACL_MUTATION_RAW_ORACLE_UTF8_INVALID');
  const branch = buildPublicAclProjectionBranchDeletionMutantsV1(projectionText);
  const recordSet = buildPublicAclProjectionRecordSetMutantsV1(projectionText);
  assert(branch.normalizedSource === recordSet.normalizedSource,
    'ACL_MUTATION_NORMALIZED_SOURCES_MISMATCH');
  const catalogue = Object.freeze({ branch, recordSet });
  assertAbsent(container.id, 'ACL_MUTATION_SEED_PREEXISTED');
  let result;
  let primaryFailure;
  try {
    const session = buildSession(rawOracleText, projectionText, catalogue);
    const transcript = runPsql(container.id, session, MAX_TRANSCRIPT_BYTES, 'session');
    result = analyzeTranscript(transcript, catalogue, sources, container.volumeName);
  } catch (error) {
    primaryFailure = error;
  }
  let postflightFailure;
  try { assertAbsent(container.id, 'ACL_MUTATION_SEED_POSTFLIGHT_INVALID'); }
  catch (error) { postflightFailure = error; }
  if (primaryFailure !== undefined) {
    throw new Error(`${errorCode(primaryFailure)}${postflightFailure === undefined
      ? '' : `;${errorCode(postflightFailure)}`}`, { cause: primaryFailure });
  }
  if (postflightFailure !== undefined) throw postflightFailure;
  result.cleanup = Object.freeze({
    transactionRolledBack: true,
    foreignDataWrapperAbsent: true,
    foreignServerAbsent: true,
  });
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

function buildSession(rawOracle, projection, catalogue) {
  assert(count(rawOracle, READ_ONLY_BEGIN) === 1 && rawOracle.endsWith(ROLLBACK),
    'ACL_MUTATION_RAW_ORACLE_ENVELOPE_INVALID');
  const transaction = `BEGIN ISOLATION LEVEL SERIALIZABLE;\n${SEED_SQL}`;
  const raw = rawOracle.replace(READ_ONLY_BEGIN, transaction).slice(0, -ROLLBACK.length);
  const sections = [
    ['original', projection], ['normalized', catalogue.branch.normalizedSource],
    ...mutationSections(catalogue),
  ];
  return `${raw}${sections.map(([id, sql]) => `${markerCommand(id, 'BEGIN')}${sql}`
    + `${markerCommand(id, 'END')}`)
    .join('')}${ROLLBACK}`;
}

function analyzeTranscript(transcript, catalogue, sources, volumeName) {
  const parsed = splitTranscript(transcript, [
    'original', 'normalized', ...mutationSections(catalogue).map(([id]) => id),
  ]);
  const oracleSession = parseOracleSession(Buffer.from(
    `${parsed.raw}${PROJECTION_BEGIN}${parsed.sections.original}${PROJECTION_END}`, 'utf8',
  ));
  const expected = deriveOracleRecords(oracleSession.raw, { enforceCleanProfile: false });
  validateSeed(oracleSession.raw, expected);
  const expectedBytes = canonicalFixture(expected);
  const original = parseProjectionRecords(lines(parsed.sections.original));
  const normalized = parseProjectionRecords(lines(parsed.sections.normalized));
  requireExactRecords(expected, original, expectedBytes, 'ACL_MUTATION_ORIGINAL_CONTROL_INVALID');
  requireExactRecords(expected, normalized, expectedBytes, 'ACL_MUTATION_NORMALIZED_CONTROL_INVALID');
  const classCounts = countClasses(expected);
  PUBLIC_ACL_OBJECT_CLASSES_V1.forEach((objectClass) => assert(classCounts[objectClass] > 0,
    'ACL_MUTATION_CLASS_NOT_OBSERVED'));
  validateSentinel(expected, catalogue.recordSet.sentinel);
  const branchMutations = catalogue.branch.mutants.map((mutant, index) => {
    const id = `delete-${mutant.objectClass}-branch`;
    const actual = parseProjectionRecords(lines(parsed.sections[id]));
    const survivors = expected.filter((record) => record.objectClass !== mutant.objectClass);
    requireExactRecords(survivors, actual, canonicalFixture(survivors),
      'ACL_MUTATION_SURVIVING_RECORDS_INVALID');
    return mutationEvidence(index + 1, id, 'branch-deletion', mutant.objectClass,
      mutant.source, expected, actual, 'ORACLE_RECORD_BAG_KEYS_MISMATCH');
  });
  const recordSetMutations = catalogue.recordSet.mutants.map((mutant, index) => {
    const actual = parseProjectionRecords(lines(parsed.sections[mutant.id]));
    const exact = exactRecordSetOutput(mutant.id, expected, catalogue.recordSet.sentinel);
    requireExactRecords(exact, actual, canonicalFixture(exact),
      'ACL_MUTATION_RECORD_SET_OUTPUT_INVALID');
    const rejection = mutant.id === 'substitute-first-atom'
      ? 'ORACLE_RECORD_BAG_MULTIPLICITY_MISMATCH' : 'ORACLE_RECORD_BAG_KEYS_MISMATCH';
    return mutationEvidence(branchMutations.length + index + 1, mutant.id,
      'record-set-sensitivity', null, mutant.source, expected, actual, rejection);
  });
  const mutations = Object.freeze([...branchMutations, ...recordSetMutations]);
  const sourceSummary = Object.fromEntries(Object.entries(sources).map(([key, value]) => [
    key, Object.freeze({ bytes: value.bytes.byteLength, sha256: sha256(value.bytes) }),
  ]));
  return {
    schemaVersion: 1, verifier: VERIFIER, authority: 'test-only-non-runtime',
    image: IMAGE_REFERENCE, imageConfiguration: IMAGE_CONFIGURATION, platform: 'linux/amd64',
    dataVolumeNameSha256: sha256(Buffer.from(volumeName, 'utf8')),
    sources: Object.freeze(sourceSummary),
    seed: Object.freeze({
      foreignDataWrapperName: FDW_NAME, foreignServerName: SERVER_NAME, publicAclAtoms: 2,
    }),
    rawTranscriptBytes: Buffer.byteLength(parsed.raw, 'utf8'),
    rawTranscriptSha256: sha256(Buffer.from(parsed.raw, 'utf8')),
    sessionTranscriptBytes: transcript.byteLength, sessionTranscriptSha256: sha256(transcript),
    control: Object.freeze({
      recordCount: expected.length, recordsBytes: expectedBytes.byteLength,
      recordsSha256: sha256(expectedBytes), normalizedEquivalent: true,
      classCounts: Object.freeze(classCounts),
    }),
    mutations,
  };
}

function mutationSections(catalogue) {
  return [
    ...catalogue.branch.mutants.map((mutant) => [
      `delete-${mutant.objectClass}-branch`, mutant.source,
    ]),
    ...catalogue.recordSet.mutants.map((mutant) => [mutant.id, mutant.source]),
  ];
}

function validateSentinel(expected, sentinel) {
  assert(expected.length > 0 && !expected.some((record) => compareRecords(record, sentinel) === 0)
    && compareRecords(expected.at(-1), sentinel) < 0, 'ACL_MUTATION_SENTINEL_INVALID');
}

function exactRecordSetOutput(id, expected, sentinel) {
  if (id === 'return-zero') return [];
  if (id === 'omit-first-atom') return expected.slice(1);
  if (id === 'add-sentinel-atom') return [...expected, sentinel];
  if (id === 'substitute-first-atom') return [...expected.slice(1), sentinel];
  throw new Error('ACL_MUTATION_RECORD_SET_ID_INVALID');
}

function mutationEvidence(sequence, id, kind, objectClass, source, expected, actual, rejection) {
  let observed = null;
  try { compareRecordBags(expected, actual); }
  catch (error) { observed = errorCode(error); }
  assert(observed === rejection, 'ACL_MUTATION_ORACLE_REJECTION_INVALID');
  const sourceBytes = Buffer.from(source, 'utf8');
  const recordsBytes = canonicalFixture(actual);
  return Object.freeze({
    sequence, id, kind, objectClass,
    sourceBytes: sourceBytes.byteLength, sourceSha256: sha256(sourceBytes),
    executed: true, parsed: true, oracleRejection: observed,
    recordDelta: actual.length - expected.length, recordCount: actual.length,
    recordsBytes: recordsBytes.byteLength, recordsSha256: sha256(recordsBytes),
  });
}

function splitTranscript(source, sectionIds) {
  const text = decodeMutationUtf8(source, 'ACL_MUTATION_TRANSCRIPT_UTF8_INVALID');
  assert(source.byteLength <= MAX_TRANSCRIPT_BYTES && text.endsWith('\n')
    && !text.includes('\r') && !text.includes('\0'), 'ACL_MUTATION_TRANSCRIPT_FRAMING_INVALID');
  const rawEnd = text.indexOf(RAW_END);
  assert(rawEnd >= 0 && text.indexOf(RAW_END, rawEnd + 1) === -1,
    'ACL_MUTATION_RAW_TRANSCRIPT_BOUNDARY_INVALID');
  const raw = text.slice(0, rawEnd + RAW_END.length);
  const sections = Object.create(null);
  let cursor = raw.length;
  for (const id of sectionIds) {
    const begin = `${markerText(id, 'BEGIN')}\n`;
    const end = `${markerText(id, 'END')}\n`;
    assert(text.startsWith(begin, cursor), 'ACL_MUTATION_SECTION_BEGIN_INVALID');
    cursor += begin.length;
    const endOffset = text.indexOf(end, cursor);
    assert(endOffset >= cursor && text.indexOf(end, endOffset + 1) === -1,
      'ACL_MUTATION_SECTION_END_INVALID');
    sections[id] = text.slice(cursor, endOffset);
    cursor = endOffset + end.length;
  }
  assert(cursor === text.length, 'ACL_MUTATION_TRANSCRIPT_TRAILING_INVALID');
  return { raw, sections };
}

function validateSeed(raw, records) {
  const fdw = raw.FDW.filter((row) => hex(row[1]) === FDW_NAME && row[8] === '0');
  const server = raw.SERVER.filter((row) => hex(row[1]) === SERVER_NAME && row[9] === '0');
  assert(raw.FDW.length === 2 && raw.SERVER.length === 2
    && raw.CONTROL.length === 1 && raw.CONTROL[0].slice(19, 25).join(',') === '1,2,2,1,2,2'
    && fdw.length === 1 && server.length === 1 && server[0][3] === fdw[0][0]
    && hex(fdw[0][9]) === 'USAGE' && fdw[0][10] === 'f'
    && hex(server[0][10]) === 'USAGE' && server[0][11] === 'f'
    && raw.CONTROL[0][30] === '0',
  'ACL_MUTATION_SEED_RAW_INVALID');
  const seeded = records.filter((record) => [FDW_NAME, SERVER_NAME].includes(record.objectName));
  assert(seeded.length === 2 && seeded.every((record) => record.privilege === 'USAGE'
    && record.grantable === false), 'ACL_MUTATION_SEED_RECORDS_INVALID');
}

function validateOwnedRun(run, sequence) {
  assert(run?.sequence === sequence && run.capture !== null && run.oracle !== null
    && run.witness !== null, 'ACL_MUTATION_OWNED_RUN_INVALID');
  const baseline = [run.capture.recordCount, run.capture.recordsBytes, run.capture.recordsSha256];
  assert(JSON.stringify(baseline) === JSON.stringify([
    run.oracle.recordCount, run.oracle.recordsBytes, run.oracle.recordsSha256,
  ]) && run.capture.projectionBytes === PROJECTION_PIN.bytes
    && run.capture.projectionSha256 === PROJECTION_PIN.sha256
    && run.oracle.projectionSourceBytes === PROJECTION_PIN.bytes
    && run.oracle.projectionSourceSha256 === PROJECTION_PIN.sha256
    && run.oracle.oracleSourceBytes === RAW_ORACLE_PIN.bytes
    && run.oracle.oracleSourceSha256 === RAW_ORACLE_PIN.sha256,
  'ACL_MUTATION_CLEAN_CONTROL_INVALID');
  validateChildSummary(run.witness);
  assert(run.capture.dataVolumeNameSha256 === run.oracle.dataVolumeNameSha256
    && run.oracle.dataVolumeNameSha256 === run.witness.dataVolumeNameSha256,
  'ACL_MUTATION_VOLUME_EVIDENCE_INVALID');
}

function validateChildSummary(value) {
  exactObject(value, [
    'schemaVersion', 'verifier', 'authority', 'image', 'imageConfiguration', 'platform',
    'dataVolumeNameSha256', 'sources', 'seed', 'rawTranscriptBytes', 'rawTranscriptSha256',
    'sessionTranscriptBytes', 'sessionTranscriptSha256', 'control', 'mutations', 'cleanup',
  ], 'ACL_MUTATION_CHILD_SHAPE_INVALID');
  assert(value.schemaVersion === 1 && value.verifier === VERIFIER
    && value.authority === 'test-only-non-runtime' && value.image === IMAGE_REFERENCE
    && value.imageConfiguration === IMAGE_CONFIGURATION && value.platform === 'linux/amd64'
    && digest(value.dataVolumeNameSha256), 'ACL_MUTATION_CHILD_IDENTITY_INVALID');
  exactObject(value.sources, ['projection', 'rawOracle', 'mutator', 'replaySupport', 'verifier'],
    'ACL_MUTATION_CHILD_SOURCES_INVALID');
  Object.values(value.sources).forEach((source) => {
    exactObject(source, ['bytes', 'sha256'], 'ACL_MUTATION_CHILD_SOURCE_INVALID');
    assert(positive(source.bytes) && digest(source.sha256), 'ACL_MUTATION_CHILD_SOURCE_INVALID');
  });
  assert(value.sources.projection.bytes === PROJECTION_PIN.bytes
    && value.sources.projection.sha256 === PROJECTION_PIN.sha256
    && value.sources.rawOracle.bytes === RAW_ORACLE_PIN.bytes
    && value.sources.rawOracle.sha256 === RAW_ORACLE_PIN.sha256,
  'ACL_MUTATION_CHILD_SOURCE_PIN_INVALID');
  exactObject(value.seed,
    ['foreignDataWrapperName', 'foreignServerName', 'publicAclAtoms'],
    'ACL_MUTATION_CHILD_SEED_INVALID');
  assert(value.seed.foreignDataWrapperName === FDW_NAME
    && value.seed.foreignServerName === SERVER_NAME && value.seed.publicAclAtoms === 2
    && positive(value.rawTranscriptBytes) && digest(value.rawTranscriptSha256)
    && positive(value.sessionTranscriptBytes) && digest(value.sessionTranscriptSha256),
  'ACL_MUTATION_CHILD_EVIDENCE_INVALID');
  exactObject(value.control,
    ['recordCount', 'recordsBytes', 'recordsSha256', 'normalizedEquivalent', 'classCounts'],
    'ACL_MUTATION_CHILD_CONTROL_INVALID');
  assert(value.control.normalizedEquivalent === true && positive(value.control.recordCount)
    && positive(value.control.recordsBytes) && digest(value.control.recordsSha256)
    && JSON.stringify(Object.keys(value.control.classCounts))
      === JSON.stringify(PUBLIC_ACL_OBJECT_CLASSES_V1)
    && Object.values(value.control.classCounts).every(positive)
    && Object.values(value.control.classCounts).reduce((sum, countValue) => sum + countValue, 0)
      === value.control.recordCount, 'ACL_MUTATION_CHILD_CONTROL_INVALID');
  const expectedMutations = [
    ...PUBLIC_ACL_OBJECT_CLASSES_V1.map((objectClass) => ({
      id: `delete-${objectClass}-branch`, kind: 'branch-deletion', objectClass,
      rejection: 'ORACLE_RECORD_BAG_KEYS_MISMATCH',
      delta: -value.control.classCounts[objectClass],
    })),
    { id: 'return-zero', kind: 'record-set-sensitivity', objectClass: null,
      rejection: 'ORACLE_RECORD_BAG_KEYS_MISMATCH', delta: -value.control.recordCount },
    { id: 'omit-first-atom', kind: 'record-set-sensitivity', objectClass: null,
      rejection: 'ORACLE_RECORD_BAG_KEYS_MISMATCH', delta: -1 },
    { id: 'add-sentinel-atom', kind: 'record-set-sensitivity', objectClass: null,
      rejection: 'ORACLE_RECORD_BAG_KEYS_MISMATCH', delta: 1 },
    { id: 'substitute-first-atom', kind: 'record-set-sensitivity', objectClass: null,
      rejection: 'ORACLE_RECORD_BAG_MULTIPLICITY_MISMATCH', delta: 0 },
  ];
  assert(Array.isArray(value.mutations) && value.mutations.length === expectedMutations.length,
    'ACL_MUTATION_CHILD_MUTATIONS_INVALID');
  value.mutations.forEach((mutation, index) => {
    const expectedMutation = expectedMutations[index];
    assert(mutation.sequence === index + 1 && mutation.id === expectedMutation.id
      && mutation.kind === expectedMutation.kind
      && mutation.objectClass === expectedMutation.objectClass
      && mutation.executed === true && mutation.parsed === true
      && mutation.oracleRejection === expectedMutation.rejection
      && mutation.recordDelta === expectedMutation.delta && nonnegative(mutation.recordCount)
      && mutation.recordCount === value.control.recordCount + mutation.recordDelta
      && positive(mutation.sourceBytes) && positive(mutation.recordsBytes)
      && digest(mutation.sourceSha256) && digest(mutation.recordsSha256),
    'ACL_MUTATION_CHILD_MUTATION_INVALID');
  });
  value.mutations.forEach((mutation) => exactObject(mutation, [
    'sequence', 'id', 'kind', 'objectClass', 'sourceBytes', 'sourceSha256', 'executed', 'parsed',
    'oracleRejection', 'recordDelta', 'recordCount', 'recordsBytes', 'recordsSha256',
  ], 'ACL_MUTATION_CHILD_MUTATION_SHAPE_INVALID'));
  exactObject(value.cleanup,
    ['transactionRolledBack', 'foreignDataWrapperAbsent', 'foreignServerAbsent'],
    'ACL_MUTATION_CHILD_CLEANUP_INVALID');
  assert(value.cleanup?.transactionRolledBack === true
    && value.cleanup?.foreignDataWrapperAbsent === true
    && value.cleanup?.foreignServerAbsent === true,
  'ACL_MUTATION_CHILD_CLEANUP_INVALID');
}

function readSources() {
  const projection = readMutationSource(ROOT, PROJECTION_PATH, 64 * 1024);
  const rawOracle = readMutationSource(ROOT, RAW_ORACLE_PATH, 64 * 1024);
  assert(projection.byteLength === PROJECTION_PIN.bytes && sha256(projection) === PROJECTION_PIN.sha256,
    'ACL_MUTATION_PROJECTION_PIN_INVALID');
  assert(rawOracle.byteLength === RAW_ORACLE_PIN.bytes && sha256(rawOracle) === RAW_ORACLE_PIN.sha256,
    'ACL_MUTATION_RAW_ORACLE_PIN_INVALID');
  return Object.freeze({
    projection: Object.freeze({ bytes: projection }), rawOracle: Object.freeze({ bytes: rawOracle }),
    mutator: Object.freeze({ bytes: readMutationSource(ROOT, MUTATOR_PATH, 64 * 1024) }),
    replaySupport: Object.freeze({ bytes: readMutationSource(ROOT, SUPPORT_PATH, 64 * 1024) }),
    verifier: Object.freeze({ bytes: readMutationSource(ROOT, SELF_PATH, 64 * 1024) }),
  });
}

function assertAbsent(id, code) {
  const output = runPsql(id, `SELECT (SELECT count(*) FROM pg_catalog.pg_foreign_data_wrapper
WHERE fdwname = '${FDW_NAME}')::text || E'\\t' ||
(SELECT count(*) FROM pg_catalog.pg_foreign_server WHERE srvname = '${SERVER_NAME}')::text;\n`,
  64 * 1024, 'probe');
  assert(output.equals(Buffer.from('0\t0\n')), code);
}

function runPsql(id, source, maxBuffer, operation) {
  return runMutationPsql(ROOT, id, Buffer.from(source, 'utf8'), maxBuffer, operation);
}

function requireExactRecords(expected, actual, bytes, code) {
  try {
    compareRecordBags(expected, actual);
    assert(bytes.equals(canonicalFixture(actual)), code);
  } catch (error) { throw new Error(code, { cause: error }); }
}

function lines(text) {
  if (text === '') return [];
  assert(text.endsWith('\n') && text.length > 1, 'ACL_MUTATION_PROJECTION_FRAMING_INVALID');
  return text.slice(0, -1).split('\n');
}

function countClasses(records) {
  const result = Object.fromEntries(PUBLIC_ACL_OBJECT_CLASSES_V1.map((value) => [value, 0]));
  records.forEach((record) => { result[record.objectClass] += 1; });
  return result;
}

function markerText(id, edge) {
  assert(/^(?:original|normalized|delete-[a-z-]+-branch|return-zero|omit-first-atom|add-sentinel-atom|substitute-first-atom)$/.test(id),
    'ACL_MUTATION_SECTION_ID_INVALID');
  return `@@ADR0047-MUTATION-V1/${id}/${edge}@@`;
}

function markerCommand(id, edge) {
  return `\\echo ${markerText(id, edge)}\n`;
}

function withoutVolume(value) {
  const { dataVolumeNameSha256: _volume, ...deterministic } = value;
  return deterministic;
}

function exactObject(value, keys, code) {
  assert(value !== null && typeof value === 'object' && !Array.isArray(value)
    && Object.getPrototypeOf(value) === Object.prototype
    && JSON.stringify(Object.keys(value)) === JSON.stringify(keys), code);
}

function count(value, needle) {
  return value.split(needle).length - 1;
}

function digest(value) {
  return typeof value === 'string' && /^[0-9a-f]{64}$/.test(value);
}

function positive(value) {
  return Number.isSafeInteger(value) && value > 0;
}

function nonnegative(value) {
  return Number.isSafeInteger(value) && value >= 0;
}

function errorCode(error) {
  return error instanceof Error && /^(?:ACL_MUTATION|ORACLE)_[A-Z0-9_]+$/.test(error.message)
    ? error.message : 'ACL_MUTATION_INTERNAL_ERROR';
}
