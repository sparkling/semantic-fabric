// SPDX-License-Identifier: MIT

import { lstatSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { TextDecoder } from 'node:util';
import { assert } from './postgresql-public-acl-oracle-wire-v1.mjs';
import { sha256 } from './postgresql-public-acl-oracle-v1.mjs';
import {
  DATABASE_NAME, IMAGE_CONFIGURATION, IMAGE_REFERENCE, runOwnedReplayPair,
} from './postgresql-public-acl-replay-support-v2.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const RECEIPT_PATH = resolve(
  ROOT, '__tests__/fixtures/postgresql-16.15-public-acl-capture-receipt-v2.json',
);
const V1_RECEIPT_PATH = resolve(
  ROOT, '__tests__/fixtures/postgresql-16.15-public-acl-capture-receipt-v1.json',
);
const CHILD_PATHS = Object.freeze({
  capture: 'scripts/capture-postgresql-public-acl-baseline-v1.mjs',
  oracle: 'scripts/verify-postgresql-public-acl-oracle-v1.mjs',
  witness: 'scripts/verify-postgresql-public-acl-has-privilege-v1.mjs',
});
const SOURCE_PATHS = Object.freeze({
  projection: '__tests__/fixtures/postgresql-16.15-public-acl-projection-v1.sql',
  rawOracle: '__tests__/fixtures/postgresql-16.15-public-acl-completeness-oracle-v1.sql',
  captureRunner: 'scripts/capture-postgresql-public-acl-baseline-v1.mjs',
  oracleWire: 'scripts/postgresql-public-acl-oracle-wire-v1.mjs',
  oracleDeriver: 'scripts/postgresql-public-acl-oracle-v1.mjs',
  oracleRunner: 'scripts/verify-postgresql-public-acl-oracle-v1.mjs',
  v1ReplayRunner: 'scripts/replay-postgresql-public-acl-baseline-v1.mjs',
  witnessInventorySql:
    '__tests__/fixtures/postgresql-16.15-public-acl-has-privilege-inventory-v1.sql',
  witnessSql:
    '__tests__/fixtures/postgresql-16.15-public-acl-has-privilege-witness-v1.sql',
  witnessInventoryParser: 'scripts/postgresql-public-acl-has-privilege-inventory-v1.mjs',
  witnessVerifier: 'scripts/postgresql-public-acl-has-privilege-v1.mjs',
  witnessRunner: 'scripts/verify-postgresql-public-acl-has-privilege-v1.mjs',
  v2ReplaySupport: 'scripts/postgresql-public-acl-replay-support-v2.mjs',
  v2ReplayRunner: 'scripts/replay-postgresql-public-acl-baseline-v2.mjs',
});
const CAPTURE_KEYS = Object.freeze([
  'schemaVersion', 'profile', 'image', 'imageConfiguration', 'platform',
  'dataVolumeNameSha256', 'profileSha256', 'projectionBytes', 'projectionSha256',
  'recordCount', 'recordsBytes', 'recordsSha256', 'classCounts',
]);
const ORACLE_KEYS = Object.freeze([
  'schemaVersion', 'oracle', 'image', 'imageConfiguration', 'platform',
  'dataVolumeNameSha256', 'oracleSourceBytes', 'oracleSourceSha256',
  'projectionSourceBytes', 'projectionSourceSha256', 'rawTranscriptBytes',
  'rawTranscriptSha256', 'sessionTranscriptBytes', 'sessionTranscriptSha256',
  'recordCount', 'recordsBytes', 'recordsSha256', 'classCounts',
]);
const WITNESS_KEYS = Object.freeze([
  'schemaVersion', 'witness', 'authority', 'roleName', 'image', 'imageConfiguration',
  'platform', 'dataVolumeNameSha256', 'inventorySourceBytes', 'inventorySourceSha256',
  'witnessSourceBytes', 'witnessSourceSha256', 'fixtureBytes', 'fixtureSha256',
  'inventoryTranscriptBytes', 'inventoryTranscriptSha256', 'witnessTranscriptBytes',
  'witnessTranscriptSha256', 'inventoryEntries', 'checkCount', 'plainTrueCount',
  'plainFalseCount', 'grantOptionTrueCount', 'corroboratedAtoms', 'columnLocalAtoms',
  'trueArrayAtoms', 'inventoryBytes', 'inventorySha256', 'observationsBytes',
  'observationsSha256', 'classCounts', 'cleanup',
]);
const CLASS_NAMES = Object.freeze([
  'schema', 'relation', 'column', 'routine', 'type', 'language',
  'foreign-data-wrapper', 'foreign-server',
]);
const V1_RECEIPT_BYTES = 4_835;
const V1_RECEIPT_SHA256 =
  '14fbd3ff2d2b50d3a8adbe0b51dc921eb926cd644a4a765183723518ec4fd08b';
const V1_SOURCE_KEYS = Object.freeze({
  projection: 'projection',
  rawOracle: 'rawOracle',
  captureRunner: 'captureRunner',
  oracleWire: 'oracleWire',
  oracleDeriver: 'oracleDeriver',
  oracleRunner: 'oracleRunner',
  v1ReplayRunner: 'replayRunner',
});

if (typeof process.argv[1] === 'string'
  && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();

function main() {
  assert(process.argv.length === 2, 'REPLAY_V2_ARGUMENTS_INVALID');
  const receiptSource = readRegular(RECEIPT_PATH, 128 * 1024,
    'REPLAY_V2_RECEIPT_FILE_INVALID');
  const receipt = parseCanonicalJson(receiptSource, 'REPLAY_V2_RECEIPT_JSON_INVALID');
  validateReceipt(receipt);
  verifyCommittedFiles(receipt);
  const runs = runOwnedReplayPair(ROOT, CHILD_PATHS);
  runs.forEach((run) => validateRun(run, receipt));
  assert(runs[0].volumeName !== runs[1].volumeName
    && runs[0].capture.dataVolumeNameSha256 !== runs[1].capture.dataVolumeNameSha256,
  'REPLAY_V2_VOLUMES_NOT_DISTINCT');
  assert(JSON.stringify(deterministicRun(runs[0])) === JSON.stringify(deterministicRun(runs[1])),
    'REPLAY_V2_RUNS_NONDETERMINISTIC');
  assert(JSON.stringify(normalizeCounts(runs[0].capture.classCounts))
    === JSON.stringify(normalizeCounts(runs[1].capture.classCounts)),
  'REPLAY_V2_CAPTURE_CLASS_COUNTS_NONDETERMINISTIC');
  assert(JSON.stringify(normalizeCounts(runs[0].oracle.classCounts))
    === JSON.stringify(normalizeCounts(runs[1].oracle.classCounts)),
  'REPLAY_V2_ORACLE_CLASS_COUNTS_NONDETERMINISTIC');
  process.stdout.write(`${JSON.stringify(buildSummary(receipt, receiptSource, runs), null, 2)}\n`);
}

export function validateReceipt(value) {
  exactObject(value, [
    'schemaVersion', 'authority', 'captureDate', 'predecessor', 'image', 'database',
    'sources', 'profile', 'result', 'witness', 'runs', 'replay',
  ], 'REPLAY_V2_RECEIPT_SHAPE_INVALID');
  assert(value.schemaVersion === 'semantic-fabric.postgresql-public-acl-capture-receipt/v2'
    && value.authority === 'test-only-non-runtime'
    && /^\d{4}-\d{2}-\d{2}$/.test(value.captureDate),
  'REPLAY_V2_RECEIPT_IDENTITY_INVALID');
  exactObject(value.predecessor, ['path', 'schemaVersion', 'bytes', 'sha256'],
    'REPLAY_V2_PREDECESSOR_SHAPE_INVALID');
  assert(value.predecessor.path
    === '__tests__/fixtures/postgresql-16.15-public-acl-capture-receipt-v1.json'
    && value.predecessor.schemaVersion
      === 'semantic-fabric.postgresql-public-acl-capture-receipt/v1'
    && value.predecessor.bytes === V1_RECEIPT_BYTES
    && value.predecessor.sha256 === V1_RECEIPT_SHA256,
  'REPLAY_V2_PREDECESSOR_INVALID');
  validateImageAndDatabase(value);
  exactObject(value.sources, Object.keys(SOURCE_PATHS), 'REPLAY_V2_SOURCES_SHAPE_INVALID');
  for (const [key, path] of Object.entries(SOURCE_PATHS)) {
    const source = value.sources[key];
    exactObject(source, ['path', 'bytes', 'sha256'], 'REPLAY_V2_SOURCE_SHAPE_INVALID');
    assert(source.path === path && positiveInteger(source.bytes) && digest(source.sha256),
      'REPLAY_V2_SOURCE_INVALID');
  }
  exactObject(value.profile, [
    'sha256', 'defaultAclRows', 'parameterAclRows', 'foreignDataWrappers',
    'foreignServers', 'userMappings', 'largeObjects', 'dedicatedSchemaRows',
    'publicDependentObjects',
  ], 'REPLAY_V2_PROFILE_SHAPE_INVALID');
  assert(digest(value.profile.sha256)
    && Object.entries(value.profile).every(([key, item]) => key === 'sha256' || item === 0),
  'REPLAY_V2_PROFILE_INVALID');
  exactObject(value.result, ['fixturePath', 'records', 'nodes', 'bytes', 'sha256'],
    'REPLAY_V2_RESULT_SHAPE_INVALID');
  assert(value.result.fixturePath
    === '__tests__/fixtures/postgresql-16.15-clean-template0-public-object-acl-v1.json'
    && positiveInteger(value.result.records) && value.result.nodes === 1 + value.result.records * 9
    && positiveInteger(value.result.bytes) && digest(value.result.sha256),
  'REPLAY_V2_RESULT_INVALID');
  validateWitnessAuthority(value.witness, value.result);
  assert(Array.isArray(value.runs) && value.runs.length === 2, 'REPLAY_V2_RUNS_INVALID');
  value.runs.forEach((run, index) => validateReceiptRun(run, index, value));
  assert(value.runs[0].dataVolumeNameSha256 !== value.runs[1].dataVolumeNameSha256
    && JSON.stringify(receiptDeterministicRun(value.runs[0]))
      === JSON.stringify(receiptDeterministicRun(value.runs[1])),
  'REPLAY_V2_RECEIPT_RUN_DETERMINISM_INVALID');
  exactObject(value.replay, [
    'minimumRuns', 'requiresDistinctAnonymousDataVolumes', 'requiresNoPublishedPorts',
    'runnerArgv', 'captureArgv', 'oracleArgv', 'witnessArgv',
  ], 'REPLAY_V2_REPLAY_SHAPE_INVALID');
  assert(value.replay.minimumRuns === 2
    && value.replay.requiresDistinctAnonymousDataVolumes === true
    && value.replay.requiresNoPublishedPorts === true
    && JSON.stringify(value.replay.runnerArgv) === JSON.stringify([
      'node', 'scripts/replay-postgresql-public-acl-baseline-v2.mjs',
    ]) && JSON.stringify(value.replay.captureArgv) === JSON.stringify([
      'node', CHILD_PATHS.capture, 'CONTAINER_NAME',
    ]) && JSON.stringify(value.replay.oracleArgv) === JSON.stringify([
      'node', CHILD_PATHS.oracle, 'CONTAINER_NAME',
    ]) && JSON.stringify(value.replay.witnessArgv) === JSON.stringify([
      'node', CHILD_PATHS.witness, 'CONTAINER_NAME',
    ]), 'REPLAY_V2_REPLAY_INVALID');
}

function validateImageAndDatabase(value) {
  exactObject(value.image, [
    'reference', 'manifestMediaType', 'platformManifestDigest', 'configurationDigest',
    'platform', 'serverVersion',
  ], 'REPLAY_V2_IMAGE_SHAPE_INVALID');
  assert(value.image.reference === IMAGE_REFERENCE
    && value.image.manifestMediaType === 'application/vnd.oci.image.manifest.v1+json'
    && value.image.platformManifestDigest === IMAGE_REFERENCE.slice('postgres@'.length)
    && value.image.configurationDigest === IMAGE_CONFIGURATION
    && value.image.platform === 'linux/amd64'
    && value.image.serverVersion === '16.15 (Debian 16.15-1.pgdg13+2)',
  'REPLAY_V2_IMAGE_INVALID');
  exactObject(value.database, [
    'name', 'owner', 'template', 'encoding', 'localeProvider', 'collation', 'ctype',
    'initdbArguments',
  ], 'REPLAY_V2_DATABASE_SHAPE_INVALID');
  assert(JSON.stringify(value.database) === JSON.stringify({
    name: DATABASE_NAME, owner: 'postgres', template: 'template0', encoding: 'UTF8',
    localeProvider: 'c', collation: 'C', ctype: 'C',
    initdbArguments: '--locale=C --encoding=UTF8',
  }), 'REPLAY_V2_DATABASE_INVALID');
}

function validateWitnessAuthority(value, result) {
  exactObject(value, [
    'name', 'roleName', 'inventoryEntries', 'checkCount', 'plainTrueCount',
    'plainFalseCount', 'grantOptionTrueCount', 'corroboratedAtoms', 'columnLocalAtoms',
    'trueArrayAtoms', 'inventoryBytes', 'inventorySha256', 'observationsBytes',
    'observationsSha256', 'classCounts',
  ], 'REPLAY_V2_WITNESS_AUTHORITY_SHAPE_INVALID');
  const classTotals = validateWitnessClassCounts(value.classCounts, value.checkCount);
  assert(value.name === 'postgresql-16.15-public-acl-has-privilege-witness-v1'
    && value.roleName === 'sf_public_acl_no_membership_witness_v1'
    && value.inventoryEntries === value.checkCount && positiveInteger(value.checkCount)
    && nonnegativeInteger(value.plainTrueCount)
    && nonnegativeInteger(value.plainFalseCount)
    && value.plainTrueCount + value.plainFalseCount === value.checkCount
    && nonnegativeInteger(value.grantOptionTrueCount) && value.grantOptionTrueCount === 0
    && value.corroboratedAtoms === result.records
    && nonnegativeInteger(value.columnLocalAtoms)
    && nonnegativeInteger(value.trueArrayAtoms)
    && value.columnLocalAtoms <= value.corroboratedAtoms
    && value.trueArrayAtoms <= value.corroboratedAtoms
    && classTotals.plainTrue === value.plainTrueCount
    && classTotals.plainFalse === value.plainFalseCount
    && positiveInteger(value.inventoryBytes) && digest(value.inventorySha256)
    && positiveInteger(value.observationsBytes) && digest(value.observationsSha256),
  'REPLAY_V2_WITNESS_AUTHORITY_INVALID');
}

function validateReceiptRun(run, index, value) {
  exactObject(run, [
    'sequence', 'networkMode', 'publishedPorts', 'dataVolumeNameSha256',
    'profileSha256', 'rawTranscriptBytes', 'rawTranscriptSha256',
    'sessionTranscriptBytes', 'sessionTranscriptSha256', 'recordCount', 'recordsBytes',
    'recordsSha256', 'inventoryTranscriptBytes', 'inventoryTranscriptSha256',
    'witnessTranscriptBytes', 'witnessTranscriptSha256',
  ], 'REPLAY_V2_RECEIPT_RUN_SHAPE_INVALID');
  assert(run.sequence === index + 1 && run.networkMode === 'none'
    && run.publishedPorts === false && digest(run.dataVolumeNameSha256)
    && run.profileSha256 === value.profile.sha256
    && positiveInteger(run.rawTranscriptBytes) && digest(run.rawTranscriptSha256)
    && positiveInteger(run.sessionTranscriptBytes) && digest(run.sessionTranscriptSha256)
    && run.recordCount === value.result.records && run.recordsBytes === value.result.bytes
    && run.recordsSha256 === value.result.sha256
    && positiveInteger(run.inventoryTranscriptBytes) && digest(run.inventoryTranscriptSha256)
    && positiveInteger(run.witnessTranscriptBytes) && digest(run.witnessTranscriptSha256),
  'REPLAY_V2_RECEIPT_RUN_INVALID');
}

function verifyCommittedFiles(receipt) {
  const predecessor = readRegular(V1_RECEIPT_PATH, 64 * 1024,
    'REPLAY_V2_PREDECESSOR_FILE_INVALID');
  assert(predecessor.byteLength === V1_RECEIPT_BYTES
    && sha256(predecessor) === V1_RECEIPT_SHA256,
  'REPLAY_V2_PREDECESSOR_PIN_MISMATCH');
  const parsed = parseCanonicalJson(predecessor, 'REPLAY_V2_PREDECESSOR_JSON_INVALID');
  assert(parsed.schemaVersion === receipt.predecessor.schemaVersion
    && JSON.stringify(receipt.image) === JSON.stringify(parsed.image)
    && JSON.stringify(receipt.database) === JSON.stringify(parsed.database)
    && JSON.stringify(receipt.profile) === JSON.stringify(parsed.profile)
    && JSON.stringify(receipt.result) === JSON.stringify(parsed.result),
  'REPLAY_V2_PREDECESSOR_CONTENT_INVALID');
  for (const [v2Key, v1Key] of Object.entries(V1_SOURCE_KEYS)) {
    assert(JSON.stringify(receipt.sources[v2Key]) === JSON.stringify(parsed.sources[v1Key]),
      'REPLAY_V2_PREDECESSOR_SOURCE_INVALID');
  }
  for (const [key, path] of Object.entries(SOURCE_PATHS)) {
    const source = readRegular(resolve(ROOT, path), 512 * 1024,
      'REPLAY_V2_COMMITTED_SOURCE_FILE_INVALID');
    assert(source.byteLength === receipt.sources[key].bytes
      && sha256(source) === receipt.sources[key].sha256,
    'REPLAY_V2_COMMITTED_SOURCE_PIN_MISMATCH');
  }
  const fixture = readRegular(resolve(ROOT, receipt.result.fixturePath), 1_048_576,
    'REPLAY_V2_COMMITTED_FIXTURE_FILE_INVALID');
  assert(fixture.byteLength === receipt.result.bytes
    && sha256(fixture) === receipt.result.sha256, 'REPLAY_V2_COMMITTED_FIXTURE_PIN_MISMATCH');
}

function validateRun(run, receipt) {
  validateCapture(run.capture, receipt);
  validateOracle(run.oracle, receipt);
  validateWitness(run.witness, receipt);
  assert(run.capture.dataVolumeNameSha256 === run.oracle.dataVolumeNameSha256
    && run.capture.dataVolumeNameSha256 === run.witness.dataVolumeNameSha256
    && run.capture.recordCount === run.oracle.recordCount
    && run.capture.recordsBytes === run.oracle.recordsBytes
    && run.capture.recordsSha256 === run.oracle.recordsSha256
    && run.witness.fixtureBytes === run.capture.recordsBytes
    && run.witness.fixtureSha256 === run.capture.recordsSha256,
  'REPLAY_V2_RUN_EVIDENCE_MISMATCH');
  assert(JSON.stringify(normalizeCounts(run.capture.classCounts))
    === JSON.stringify(normalizeCounts(run.oracle.classCounts)),
  'REPLAY_V2_RUN_CLASS_COUNTS_MISMATCH');
}

function validateCapture(value, receipt) {
  exactObject(value, CAPTURE_KEYS, 'REPLAY_V2_CAPTURE_SHAPE_INVALID');
  assert(value.schemaVersion === 1
    && value.profile === 'postgresql-16.15-clean-template0-public-object-acl-v1'
    && commonImage(value) && value.profileSha256 === receipt.profile.sha256
    && value.projectionBytes === receipt.sources.projection.bytes
    && value.projectionSha256 === receipt.sources.projection.sha256
    && value.recordCount === receipt.result.records && value.recordsBytes === receipt.result.bytes
    && value.recordsSha256 === receipt.result.sha256,
  'REPLAY_V2_CAPTURE_INVALID');
  validateCounts(value.classCounts, value.recordCount, true,
    'REPLAY_V2_CAPTURE_CLASS_COUNTS_INVALID');
}

function validateOracle(value, receipt) {
  exactObject(value, ORACLE_KEYS, 'REPLAY_V2_ORACLE_SHAPE_INVALID');
  const expected = receipt.runs[0];
  assert(value.schemaVersion === 1
    && value.oracle === 'postgresql-16.15-public-acl-completeness-oracle-v1'
    && commonImage(value) && value.oracleSourceBytes === receipt.sources.rawOracle.bytes
    && value.oracleSourceSha256 === receipt.sources.rawOracle.sha256
    && value.projectionSourceBytes === receipt.sources.projection.bytes
    && value.projectionSourceSha256 === receipt.sources.projection.sha256
    && value.rawTranscriptBytes === expected.rawTranscriptBytes
    && value.rawTranscriptSha256 === expected.rawTranscriptSha256
    && value.sessionTranscriptBytes === expected.sessionTranscriptBytes
    && value.sessionTranscriptSha256 === expected.sessionTranscriptSha256
    && value.recordCount === receipt.result.records && value.recordsBytes === receipt.result.bytes
    && value.recordsSha256 === receipt.result.sha256,
  'REPLAY_V2_ORACLE_INVALID');
  validateCounts(value.classCounts, value.recordCount, false,
    'REPLAY_V2_ORACLE_CLASS_COUNTS_INVALID');
}

function validateWitness(value, receipt) {
  exactObject(value, WITNESS_KEYS, 'REPLAY_V2_WITNESS_SHAPE_INVALID');
  const expected = receipt.runs[0];
  const authority = receipt.witness;
  assert(value.schemaVersion === 1 && value.witness === authority.name
    && value.authority === 'test-only-non-runtime' && value.roleName === authority.roleName
    && commonImage(value)
    && value.inventorySourceBytes === receipt.sources.witnessInventorySql.bytes
    && value.inventorySourceSha256 === receipt.sources.witnessInventorySql.sha256
    && value.witnessSourceBytes === receipt.sources.witnessSql.bytes
    && value.witnessSourceSha256 === receipt.sources.witnessSql.sha256
    && value.fixtureBytes === receipt.result.bytes && value.fixtureSha256 === receipt.result.sha256
    && value.inventoryTranscriptBytes === expected.inventoryTranscriptBytes
    && value.inventoryTranscriptSha256 === expected.inventoryTranscriptSha256
    && value.witnessTranscriptBytes === expected.witnessTranscriptBytes
    && value.witnessTranscriptSha256 === expected.witnessTranscriptSha256,
  'REPLAY_V2_WITNESS_SOURCE_OR_TRANSCRIPT_INVALID');
  for (const key of [
    'inventoryEntries', 'checkCount', 'plainTrueCount', 'plainFalseCount',
    'grantOptionTrueCount', 'corroboratedAtoms', 'columnLocalAtoms', 'trueArrayAtoms',
    'inventoryBytes', 'inventorySha256', 'observationsBytes', 'observationsSha256',
  ]) assert(value[key] === authority[key], 'REPLAY_V2_WITNESS_RESULT_INVALID');
  assert(JSON.stringify(value.classCounts) === JSON.stringify(authority.classCounts)
    && JSON.stringify(value.cleanup) === JSON.stringify({
      preflightAbsent: true, created: true, dropped: true, postDropAbsent: true,
    }), 'REPLAY_V2_WITNESS_CONTROL_INVALID');
}

function buildSummary(receipt, receiptSource, runs) {
  return {
    schemaVersion: 1,
    replay: 'postgresql-16.15-public-acl-baseline-v2',
    authority: 'test-only-non-runtime',
    receiptBytes: receiptSource.byteLength,
    receiptSha256: sha256(receiptSource),
    predecessor: receipt.predecessor,
    image: IMAGE_REFERENCE,
    imageConfiguration: IMAGE_CONFIGURATION,
    platform: 'linux/amd64',
    runs: runs.map((run) => ({
      sequence: run.sequence,
      dataVolumeNameSha256: run.capture.dataVolumeNameSha256,
      ...deterministicRun(run),
    })),
    deterministic: {
      ...receipt.witness,
      rawTranscriptBytes: receipt.runs[0].rawTranscriptBytes,
      rawTranscriptSha256: receipt.runs[0].rawTranscriptSha256,
      sessionTranscriptBytes: receipt.runs[0].sessionTranscriptBytes,
      sessionTranscriptSha256: receipt.runs[0].sessionTranscriptSha256,
      inventoryTranscriptBytes: receipt.runs[0].inventoryTranscriptBytes,
      inventoryTranscriptSha256: receipt.runs[0].inventoryTranscriptSha256,
      witnessTranscriptBytes: receipt.runs[0].witnessTranscriptBytes,
      witnessTranscriptSha256: receipt.runs[0].witnessTranscriptSha256,
      records: receipt.result.records,
      recordsBytes: receipt.result.bytes,
      recordsSha256: receipt.result.sha256,
    },
  };
}

function deterministicRun(run) {
  return {
    networkMode: 'none', publishedPorts: false,
    profileSha256: run.capture.profileSha256,
    rawTranscriptBytes: run.oracle.rawTranscriptBytes,
    rawTranscriptSha256: run.oracle.rawTranscriptSha256,
    sessionTranscriptBytes: run.oracle.sessionTranscriptBytes,
    sessionTranscriptSha256: run.oracle.sessionTranscriptSha256,
    recordCount: run.capture.recordCount, recordsBytes: run.capture.recordsBytes,
    recordsSha256: run.capture.recordsSha256,
    inventoryTranscriptBytes: run.witness.inventoryTranscriptBytes,
    inventoryTranscriptSha256: run.witness.inventoryTranscriptSha256,
    witnessTranscriptBytes: run.witness.witnessTranscriptBytes,
    witnessTranscriptSha256: run.witness.witnessTranscriptSha256,
  };
}

function receiptDeterministicRun(run) {
  const { sequence: _sequence, dataVolumeNameSha256: _volume, ...rest } = run;
  return rest;
}

function validateWitnessClassCounts(value, total) {
  exactObject(value, CLASS_NAMES, 'REPLAY_V2_WITNESS_CLASS_COUNTS_SHAPE_INVALID');
  const totals = { checks: 0, plainTrue: 0, plainFalse: 0 };
  for (const item of Object.values(value)) {
    exactObject(item, ['checks', 'plainTrue', 'plainFalse'],
      'REPLAY_V2_WITNESS_CLASS_COUNT_SHAPE_INVALID');
    assert(nonnegativeInteger(item.checks) && nonnegativeInteger(item.plainTrue)
      && nonnegativeInteger(item.plainFalse)
      && item.plainTrue + item.plainFalse === item.checks,
    'REPLAY_V2_WITNESS_CLASS_COUNT_INVALID');
    totals.checks += item.checks;
    totals.plainTrue += item.plainTrue;
    totals.plainFalse += item.plainFalse;
  }
  assert(totals.checks === total, 'REPLAY_V2_WITNESS_CLASS_COUNT_TOTAL_INVALID');
  return totals;
}

function commonImage(value) {
  return value.image === IMAGE_REFERENCE && value.imageConfiguration === IMAGE_CONFIGURATION
    && value.platform === 'linux/amd64' && digest(value.dataVolumeNameSha256);
}

function exactObject(value, keys, code) {
  assert(value !== null && typeof value === 'object' && !Array.isArray(value)
    && Object.getPrototypeOf(value) === Object.prototype
    && JSON.stringify(Object.keys(value)) === JSON.stringify(keys), code);
}

function validateCounts(value, total, complete, code) {
  assert(value !== null && typeof value === 'object' && !Array.isArray(value)
    && Object.getPrototypeOf(value) === Object.prototype, code);
  const keys = Object.keys(value);
  assert(keys.length > 0 && keys.every((key) => CLASS_NAMES.includes(key))
    && (!complete || JSON.stringify(keys) === JSON.stringify([...CLASS_NAMES].sort()))
    && keys.every((key) => nonnegativeInteger(value[key]))
    && Object.values(value).reduce((sum, item) => sum + item, 0) === total, code);
}

function normalizeCounts(value) {
  return Object.fromEntries([...CLASS_NAMES].sort().map((key) => [key, value[key] ?? 0]));
}

function parseCanonicalJson(source, code) {
  const text = new TextDecoder('utf-8', { fatal: true }).decode(source);
  assert(text.endsWith('\n') && !text.includes('\r') && !text.includes('\0'), code);
  let value;
  try { value = JSON.parse(text); } catch { throw new Error(code); }
  assert(`${JSON.stringify(value, null, 2)}\n` === text, code);
  return value;
}

function readRegular(path, maximum, code) {
  const stat = lstatSync(path);
  assert(stat.isFile() && !stat.isSymbolicLink() && stat.nlink === 1
    && stat.size > 0 && stat.size <= maximum, code);
  const value = readFileSync(path);
  assert(value.byteLength === stat.size, code);
  return value;
}

function digest(value) { return typeof value === 'string' && /^[0-9a-f]{64}$/.test(value); }
function positiveInteger(value) { return Number.isSafeInteger(value) && value > 0; }
function nonnegativeInteger(value) { return Number.isSafeInteger(value) && value >= 0; }
