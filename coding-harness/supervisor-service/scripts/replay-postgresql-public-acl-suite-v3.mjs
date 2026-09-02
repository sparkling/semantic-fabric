// SPDX-License-Identifier: MIT

import { lstatSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { TextDecoder } from 'node:util';
import { assert } from './postgresql-public-acl-oracle-wire-v1.mjs';
import { sha256 } from './postgresql-public-acl-oracle-v1.mjs';
import { IMAGE_CONFIGURATION, IMAGE_REFERENCE, runOwnedReplayPair }
  from './postgresql-public-acl-replay-support-v3.mjs';
import { validateReceipt as validateV2Receipt }
  from './replay-postgresql-public-acl-baseline-v2.mjs';
import { validateFinalWhereMutationReceiptV1 }
  from './postgresql-public-acl-final-where-mutation-oracle-v1.mjs';
import { PUBLIC_ACL_OBJECT_CLASSES_V1 }
  from './postgresql-public-acl-projection-branch-mutations-v1.mjs';

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const CONTRACT_PATH = resolve(ROOT,
  '__tests__/fixtures/postgresql-16.15-public-acl-replay-contract-v3.json');
const V1_RECEIPT_PATH = resolve(ROOT,
  '__tests__/fixtures/postgresql-16.15-public-acl-capture-receipt-v1.json');
const V2_RECEIPT_PATH = resolve(ROOT,
  '__tests__/fixtures/postgresql-16.15-public-acl-capture-receipt-v2.json');
const V1_PIN = Object.freeze({ bytes: 4_835,
  sha256: '14fbd3ff2d2b50d3a8adbe0b51dc921eb926cd644a4a765183723518ec4fd08b' });
const V2_PIN = Object.freeze({ bytes: 8_816,
  sha256: '48d54b635ff6bafc6bdb4ffcb1bb9d74c8357e932e22f7b6453bb54cb0d698e8' });
const PROFILE_PATHS = Object.freeze({
  'baseline-v1': Object.freeze({
    capture: 'scripts/capture-postgresql-public-acl-baseline-v1.mjs',
    oracle: 'scripts/verify-postgresql-public-acl-oracle-v1.mjs',
  }),
  'baseline-v2': Object.freeze({
    capture: 'scripts/capture-postgresql-public-acl-baseline-v1.mjs',
    oracle: 'scripts/verify-postgresql-public-acl-oracle-v1.mjs',
    witness: 'scripts/verify-postgresql-public-acl-has-privilege-v1.mjs',
  }),
  branch: Object.freeze({
    capture: 'scripts/capture-postgresql-public-acl-baseline-v1.mjs',
    oracle: 'scripts/verify-postgresql-public-acl-oracle-v1.mjs',
    witness: 'scripts/verify-postgresql-public-acl-projection-branch-mutations-v1.mjs',
  }),
  'final-where': Object.freeze({
    capture: 'scripts/capture-postgresql-public-acl-baseline-v1.mjs',
    oracle: 'scripts/verify-postgresql-public-acl-oracle-v1.mjs',
    witness: 'scripts/verify-postgresql-public-acl-projection-final-where-mutations-v1.mjs',
  }),
});
const SOURCE_PATHS = Object.freeze({
  projection: '__tests__/fixtures/postgresql-16.15-public-acl-projection-v1.sql',
  rawOracle: '__tests__/fixtures/postgresql-16.15-public-acl-completeness-oracle-v1.sql',
  captureRunner: 'scripts/capture-postgresql-public-acl-baseline-v1.mjs',
  oracleRunner: 'scripts/verify-postgresql-public-acl-oracle-v1.mjs',
  witnessRunner: 'scripts/verify-postgresql-public-acl-has-privilege-v1.mjs',
  branchMutator: 'scripts/postgresql-public-acl-projection-branch-mutations-v1.mjs',
  mutationReplaySupport: 'scripts/postgresql-public-acl-mutation-replay-support-v1.mjs',
  branchVerifier: 'scripts/verify-postgresql-public-acl-projection-branch-mutations-v1.mjs',
  finalWhereMutator:
    'scripts/postgresql-public-acl-projection-final-where-mutations-v1.mjs',
  finalWhereOracle: 'scripts/postgresql-public-acl-final-where-mutation-oracle-v1.mjs',
  finalWhereVerifier:
    'scripts/verify-postgresql-public-acl-projection-final-where-mutations-v1.mjs',
  v3ReplaySupport: 'scripts/postgresql-public-acl-replay-support-v3.mjs',
  v3ReplayRunner: 'scripts/replay-postgresql-public-acl-suite-v3.mjs',
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

if (typeof process.argv[1] === 'string'
  && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
function main() {
  assert(process.argv.length === 3 && Object.hasOwn(PROFILE_PATHS, process.argv[2]),
    'REPLAY_V3_ARGUMENTS_INVALID');
  const contractSource = readRegular(CONTRACT_PATH, 128 * 1024,
    'REPLAY_V3_CONTRACT_FILE_INVALID');
  const contract = parseCanonicalJson(contractSource, 'REPLAY_V3_CONTRACT_JSON_INVALID');
  validateReplayContract(contract);
  const { v1, v2 } = verifyCommittedFiles(contract);
  const profile = process.argv[2];
  const runs = runOwnedReplayPair(ROOT, PROFILE_PATHS[profile]);
  validateRuns(profile, runs, contract, v1, v2);
  const summary = buildSummary(profile, runs, contract, contractSource);
  process.stdout.write(`${JSON.stringify(summary, null, 2)}\n`);
}
export function buildSummary(profile, runs, contract, contractSource) {
  const deterministic = deterministicRun(runs[0]);
  assert(JSON.stringify(deterministic) === JSON.stringify(deterministicRun(runs[1])),
    'REPLAY_V3_RUNS_NONDETERMINISTIC');
  const deterministicSource = Buffer.from(`${JSON.stringify(deterministic)}\n`, 'utf8');
  const summary = {
    schemaVersion: 1,
    replay: 'postgresql-16.15-public-acl-suite-v3',
    profile,
    authority: 'test-only-non-runtime',
    contractBytes: contractSource.byteLength,
    contractSha256: sha256(contractSource),
    image: IMAGE_REFERENCE,
    imageConfiguration: IMAGE_CONFIGURATION,
    platform: 'linux/amd64',
    historicalPredecessors: contract.predecessors,
    runs: runs.map((run) => ({
      sequence: run.sequence,
      networkMode: 'none',
      publishedPorts: false,
      dataVolumeNameSha256: run.capture.dataVolumeNameSha256,
    })),
    deterministicBytes: deterministicSource.byteLength,
    deterministicSha256: sha256(deterministicSource),
  };
  const serialized = JSON.stringify(summary);
  assert(!serialized.includes('"volumeName"')
    && runs.every((run) => !serialized.includes(run.volumeName)),
  'REPLAY_V3_RAW_VOLUME_IDENTITY_LEAKED');
  return summary;
}
export function validateReplayContract(value) {
  exactObject(value, [
    'schemaVersion', 'authority', 'contractDate', 'predecessors', 'historicalFailure',
    'image', 'sources', 'lifecycle', 'replay',
  ], 'REPLAY_V3_CONTRACT_SHAPE_INVALID');
  assert(value.schemaVersion === 'semantic-fabric.postgresql-public-acl-replay-contract/v3'
    && value.authority === 'test-only-non-runtime' && value.contractDate === '2026-09-02',
  'REPLAY_V3_CONTRACT_IDENTITY_INVALID');
  assert(JSON.stringify(value.predecessors) === JSON.stringify([
    predecessor(1, V1_PIN), predecessor(2, V2_PIN),
  ]), 'REPLAY_V3_PREDECESSORS_INVALID');
  assert(JSON.stringify(value.historicalFailure) === JSON.stringify({
    runId: 33_612_211_004,
    headSha: 'a2a20e33239bf5d8210b75b51189d96f5cb1ecfe',
    diagnosis: 'temporary-init-server-readiness-race',
    jobs: [
      { node: '24.14.1', jobId: 100_189_827_960, error: 'REPLAY_PSQL_FAILED' },
      { node: '20.0.0', jobId: 100_189_828_074, error: 'REPLAY_DATABASE_CREATE_FAILED' },
    ],
    replayPolicy: 'historical-runners-not-invoked',
  }), 'REPLAY_V3_HISTORICAL_FAILURE_INVALID');
  validateImage(value.image);
  exactObject(value.sources, Object.keys(SOURCE_PATHS), 'REPLAY_V3_SOURCES_SHAPE_INVALID');
  for (const [key, path] of Object.entries(SOURCE_PATHS)) {
    const source = value.sources[key];
    exactObject(source, ['path', 'bytes', 'sha256'], 'REPLAY_V3_SOURCE_SHAPE_INVALID');
    assert(source.path === path && positive(source.bytes) && digest(source.sha256),
      'REPLAY_V3_SOURCE_INVALID');
  }
  assert(JSON.stringify(value.lifecycle) === JSON.stringify({
    pid1CommPath: '/proc/1/comm', expectedPid1Comm: 'postgres\n',
    readinessArgv: ['pg_isready', '-q', '-U', 'postgres', '-d', 'postgres'],
    acceptance: 'pid1-and-readiness', timeoutMilliseconds: 60_000,
  }), 'REPLAY_V3_LIFECYCLE_INVALID');
  validateReplay(value.replay);
}
function predecessor(version, pin) {
  return {
    path: `__tests__/fixtures/postgresql-16.15-public-acl-capture-receipt-v${version}.json`,
    schemaVersion: `semantic-fabric.postgresql-public-acl-capture-receipt/v${version}`,
    ...pin,
    disposition: 'preserved-historical-no-rerun',
  };
}
function validateImage(value) {
  exactObject(value, [
    'reference', 'manifestMediaType', 'platformManifestDigest', 'configurationDigest',
    'platform', 'serverVersion',
  ], 'REPLAY_V3_IMAGE_SHAPE_INVALID');
  assert(value.reference === IMAGE_REFERENCE
    && value.manifestMediaType === 'application/vnd.oci.image.manifest.v1+json'
    && value.platformManifestDigest === IMAGE_REFERENCE.slice('postgres@'.length)
    && value.configurationDigest === IMAGE_CONFIGURATION && value.platform === 'linux/amd64'
    && value.serverVersion === '16.15 (Debian 16.15-1.pgdg13+2)',
  'REPLAY_V3_IMAGE_INVALID');
}
function validateReplay(value) {
  exactObject(value, [
    'minimumRunsPerProfile', 'totalMinimumRuns', 'requiresDistinctAnonymousDataVolumes',
    'requiresNoPublishedPorts', 'requiresNetworkMode', 'profiles',
  ], 'REPLAY_V3_REPLAY_SHAPE_INVALID');
  assert(value.minimumRunsPerProfile === 2 && value.totalMinimumRuns === 8
    && value.requiresDistinctAnonymousDataVolumes === true
    && value.requiresNoPublishedPorts === true && value.requiresNetworkMode === 'none'
    && Array.isArray(value.profiles) && value.profiles.length === 4,
  'REPLAY_V3_REPLAY_INVALID');
  const expected = Object.entries(PROFILE_PATHS).map(([name, childPaths]) => ({
    name, minimumRuns: 2, childPaths,
    runnerArgv: ['node', 'scripts/replay-postgresql-public-acl-suite-v3.mjs', name],
  }));
  assert(JSON.stringify(value.profiles) === JSON.stringify(expected),
    'REPLAY_V3_PROFILES_INVALID');
}
function verifyCommittedFiles(receipt) {
  const v1Source = readRegular(V1_RECEIPT_PATH, 64 * 1024,
    'REPLAY_V3_V1_RECEIPT_FILE_INVALID');
  const v2Source = readRegular(V2_RECEIPT_PATH, 128 * 1024,
    'REPLAY_V3_V2_RECEIPT_FILE_INVALID');
  assert(v1Source.byteLength === V1_PIN.bytes && sha256(v1Source) === V1_PIN.sha256,
    'REPLAY_V3_V1_PREDECESSOR_PIN_MISMATCH');
  assert(v2Source.byteLength === V2_PIN.bytes && sha256(v2Source) === V2_PIN.sha256,
    'REPLAY_V3_V2_PREDECESSOR_PIN_MISMATCH');
  const v1 = parseCanonicalJson(v1Source, 'REPLAY_V3_V1_RECEIPT_JSON_INVALID');
  const v2 = parseCanonicalJson(v2Source, 'REPLAY_V3_V2_RECEIPT_JSON_INVALID');
  validateV2Receipt(v2);
  assert(JSON.stringify(v2.predecessor) === JSON.stringify({
    path: receipt.predecessors[0].path, schemaVersion: receipt.predecessors[0].schemaVersion,
    bytes: receipt.predecessors[0].bytes, sha256: receipt.predecessors[0].sha256,
  }), 'REPLAY_V3_PREDECESSOR_CHAIN_INVALID');
  [v1, v2].forEach(verifyPredecessorClosure);
  for (const [key, path] of Object.entries(SOURCE_PATHS)) {
    const source = readRegular(resolve(ROOT, path), 512 * 1024, 'REPLAY_V3_SOURCE_FILE_INVALID');
    assert(source.byteLength === receipt.sources[key].bytes
      && sha256(source) === receipt.sources[key].sha256, 'REPLAY_V3_SOURCE_PIN_MISMATCH');
  }
  return { v1, v2 };
}
export function verifyPredecessorClosure(value) {
  Object.values(value.sources).forEach((pin) => {
    const source = readRegular(resolve(ROOT, pin.path), 512 * 1024,
      'REPLAY_V3_PREDECESSOR_SOURCE_FILE_INVALID');
    assert(source.byteLength === pin.bytes && sha256(source) === pin.sha256,
      'REPLAY_V3_PREDECESSOR_SOURCE_PIN_MISMATCH');
  });
  const fixture = readRegular(resolve(ROOT, value.result.fixturePath), 2 * 1024 * 1024,
    'REPLAY_V3_PREDECESSOR_FIXTURE_FILE_INVALID');
  assert(fixture.byteLength === value.result.bytes && sha256(fixture) === value.result.sha256,
    'REPLAY_V3_PREDECESSOR_FIXTURE_PIN_MISMATCH');
}
function validateRuns(profile, runs, receipt, v1, v2) {
  assert(Array.isArray(runs) && runs.length === 2, 'REPLAY_V3_RUN_COUNT_INVALID');
  runs.forEach((run, index) => validateRun(profile, run, index + 1, receipt, v1, v2));
  const volumes = runs.map((run) => run.capture.dataVolumeNameSha256);
  assert(runs[0].volumeName !== runs[1].volumeName && volumes[0] !== volumes[1],
    'REPLAY_V3_VOLUMES_NOT_DISTINCT');
}
export function validateRun(profile, run, sequence, receipt, v1, v2) {
  exactObject(run, profile === 'baseline-v1'
    ? ['sequence', 'volumeName', 'capture', 'oracle']
    : ['sequence', 'volumeName', 'capture', 'oracle', 'witness'],
  'REPLAY_V3_RUN_SHAPE_INVALID');
  assert(run.sequence === sequence && /^[0-9a-f]{64}$/.test(run.volumeName),
    'REPLAY_V3_RUN_IDENTITY_INVALID');
  const baseline = profile === 'baseline-v1' ? v1 : v2;
  validateCapture(run.capture, baseline);
  validateOracle(run.oracle, baseline);
  assert(run.capture.dataVolumeNameSha256 === run.oracle.dataVolumeNameSha256,
    'REPLAY_V3_RUN_VOLUME_MISMATCH');
  assert(JSON.stringify(normalizeCounts(run.capture.classCounts))
    === JSON.stringify(normalizeCounts(run.oracle.classCounts)),
  'REPLAY_V3_RUN_CLASS_COUNTS_MISMATCH');
  if (profile === 'baseline-v1') return;
  assert(run.witness.dataVolumeNameSha256 === run.capture.dataVolumeNameSha256,
    'REPLAY_V3_WITNESS_VOLUME_MISMATCH');
  if (profile === 'baseline-v2') validateWitness(run.witness, v2);
  else if (profile === 'branch') validateBranch(run.witness, receipt);
  else validateFinalWhere(run.witness, receipt);
}
function validateCapture(value, receipt) {
  exactObject(value, CAPTURE_KEYS, 'REPLAY_V3_CAPTURE_SHAPE_INVALID');
  assert(value.schemaVersion === 1
    && value.profile === 'postgresql-16.15-clean-template0-public-object-acl-v1'
    && commonImage(value) && value.profileSha256 === receipt.profile.sha256
    && value.projectionBytes === receipt.sources.projection.bytes
    && value.projectionSha256 === receipt.sources.projection.sha256
    && value.recordCount === receipt.result.records && value.recordsBytes === receipt.result.bytes
    && value.recordsSha256 === receipt.result.sha256,
  'REPLAY_V3_CAPTURE_INVALID');
  validateCounts(value.classCounts, value.recordCount, true,
    'REPLAY_V3_CAPTURE_CLASS_COUNTS_INVALID');
}
function validateOracle(value, receipt) {
  exactObject(value, ORACLE_KEYS, 'REPLAY_V3_ORACLE_SHAPE_INVALID');
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
  'REPLAY_V3_ORACLE_INVALID');
  validateCounts(value.classCounts, value.recordCount, false,
    'REPLAY_V3_ORACLE_CLASS_COUNTS_INVALID');
}
export function validateWitness(value, receipt) {
  exactObject(value, WITNESS_KEYS, 'REPLAY_V3_WITNESS_SHAPE_INVALID');
  const authority = receipt.witness;
  const expected = receipt.runs[0];
  assert(value.schemaVersion === 1 && value.witness === authority.name
    && value.authority === 'test-only-non-runtime' && value.roleName === authority.roleName
    && commonImage(value)
    && value.inventorySourceBytes === receipt.sources.witnessInventorySql.bytes
    && value.inventorySourceSha256 === receipt.sources.witnessInventorySql.sha256
    && value.witnessSourceBytes === receipt.sources.witnessSql.bytes
    && value.witnessSourceSha256 === receipt.sources.witnessSql.sha256
    && value.fixtureBytes === receipt.result.bytes
    && value.fixtureSha256 === receipt.result.sha256
    && value.inventoryTranscriptBytes === expected.inventoryTranscriptBytes
    && value.inventoryTranscriptSha256 === expected.inventoryTranscriptSha256
    && value.witnessTranscriptBytes === expected.witnessTranscriptBytes
    && value.witnessTranscriptSha256 === expected.witnessTranscriptSha256,
  'REPLAY_V3_WITNESS_SOURCE_OR_TRANSCRIPT_INVALID');
  assert(JSON.stringify(value.classCounts) === JSON.stringify(authority.classCounts)
    && JSON.stringify(value.cleanup) === JSON.stringify({
      preflightAbsent: true, created: true, dropped: true, postDropAbsent: true,
    }), 'REPLAY_V3_WITNESS_INVALID');
  for (const key of [
    'inventoryEntries', 'checkCount', 'plainTrueCount', 'plainFalseCount',
    'grantOptionTrueCount', 'corroboratedAtoms', 'columnLocalAtoms', 'trueArrayAtoms',
    'inventoryBytes', 'inventorySha256', 'observationsBytes', 'observationsSha256',
  ]) assert(value[key] === authority[key], 'REPLAY_V3_WITNESS_RESULT_INVALID');
}
function validateBranch(value, receipt) {
  exactObject(value, [
    'schemaVersion', 'verifier', 'authority', 'image', 'imageConfiguration', 'platform',
    'dataVolumeNameSha256', 'sources', 'seed', 'rawTranscriptBytes', 'rawTranscriptSha256',
    'sessionTranscriptBytes', 'sessionTranscriptSha256', 'control', 'mutations', 'cleanup',
  ], 'REPLAY_V3_BRANCH_SHAPE_INVALID');
  exactObject(value.seed, ['foreignDataWrapperName', 'foreignServerName', 'publicAclAtoms'],
    'REPLAY_V3_BRANCH_SEED_SHAPE_INVALID');
  exactObject(value.control,
    ['recordCount', 'recordsBytes', 'recordsSha256', 'normalizedEquivalent', 'classCounts'],
    'REPLAY_V3_BRANCH_CONTROL_SHAPE_INVALID');
  const expectedMutations = branchMutationExpectations(value.control);
  assert(value.schemaVersion === 1
    && value.verifier === 'postgresql-16.15-public-acl-projection-mutations-v1'
    && value.authority === 'test-only-non-runtime' && commonImage(value)
    && JSON.stringify(value.seed) === JSON.stringify({
      foreignDataWrapperName: 'sf_public_acl_mutation_fdw_v1',
      foreignServerName: 'sf_public_acl_mutation_server_v1', publicAclAtoms: 2,
    })
    && value.rawTranscriptBytes === 1_124_809
    && value.rawTranscriptSha256
      === '004d74db066cd41b78a739cf8030211ebe450ddcd5fee810e6b744e66dbf2c7f'
    && JSON.stringify(value.control) === JSON.stringify({
      recordCount: 4_061, recordsBytes: 861_437,
      recordsSha256: '2ded27910e4aae2d44242ca7ea5c4ebd9c30ce10ad541a1c6f916bdbddf34935',
      normalizedEquivalent: true,
      classCounts: { schema: 2, relation: 189, column: 16, routine: 3_235,
        type: 613, language: 4, 'foreign-data-wrapper': 1, 'foreign-server': 1 },
    })
    && value.sessionTranscriptBytes === 12_584_275
    && value.sessionTranscriptSha256
      === 'b2db44b9feed4ea7c6dd4b294abfa8a3334fb8139d9817ada9d30851cd88da22'
    && Array.isArray(value.mutations) && value.mutations.length === expectedMutations.length
    && value.mutations.every((item, index) => {
      const expected = expectedMutations[index];
      exactObject(item, [
        'sequence', 'id', 'kind', 'objectClass', 'sourceBytes', 'sourceSha256', 'executed',
        'parsed', 'oracleRejection', 'recordDelta', 'recordCount', 'recordsBytes',
        'recordsSha256',
      ], 'REPLAY_V3_BRANCH_MUTATION_SHAPE_INVALID');
      return item.sequence === index + 1 && item.id === expected.id && item.kind === expected.kind
        && item.objectClass === expected.objectClass
        && item.oracleRejection === expected.rejection && item.recordDelta === expected.delta
        && item.recordCount === value.control.recordCount + expected.delta
        && item.executed === true && item.parsed === true
        && positive(item.sourceBytes) && digest(item.sourceSha256)
        && Number.isSafeInteger(item.recordDelta)
        && Number.isSafeInteger(item.recordCount) && item.recordCount >= 0
        && positive(item.recordsBytes) && digest(item.recordsSha256);
    })
    && JSON.stringify(value.cleanup) === JSON.stringify({
      transactionRolledBack: true, foreignDataWrapperAbsent: true, foreignServerAbsent: true,
    }), 'REPLAY_V3_BRANCH_INVALID');
  validateMutationSources(value.sources, receipt, 'branch');
}
function validateFinalWhere(value, receipt) {
  exactObject(value, ['schemaVersion', 'verifier', 'authority', 'image', 'imageConfiguration',
    'platform', 'dataVolumeNameSha256', 'sources', 'proof', 'cleanup'],
  'REPLAY_V3_FINAL_WHERE_SHAPE_INVALID');
  validateFinalWhereMutationReceiptV1(value.proof);
  exactObject(value.cleanup, ['transactionsRolledBack', 'hiddenObjectsAbsent'],
    'REPLAY_V3_FINAL_WHERE_CLEANUP_SHAPE_INVALID');
  assert(value.schemaVersion === 1
    && value.verifier === 'postgresql-16.15-public-acl-projection-final-where-mutations-v1'
    && value.authority === 'test-only-non-runtime' && commonImage(value)
    && value.proof?.executed === 19 && value.proof?.killed === 15
    && value.proof?.guardEquivalent === 4 && value.proof?.unresolved === 0
    && Array.isArray(value.proof?.batches) && value.proof.batches.length === 2
    && JSON.stringify(value.proof.batches.map((batch) => batch.transcriptBytes))
      === JSON.stringify([11_963_849, 11_608_234])
    && JSON.stringify(value.proof.batches.map((batch) => batch.transcriptSha256))
      === JSON.stringify([
        '2df643f972d3bd4fc9d62dea2466421c444ce54b7c8cf660b8dc63becac7c50f',
        '688a5ed3ee5af795339aaa20631c258627b0c362b0637a6c2c2251feb0626522',
      ])
    && JSON.stringify(value.cleanup) === JSON.stringify({
      transactionsRolledBack: 2, hiddenObjectsAbsent: true,
    }), 'REPLAY_V3_FINAL_WHERE_INVALID');
  validateMutationSources(value.sources, receipt, 'final-where');
}
function validateMutationSources(value, receipt, profile) {
  const mapping = profile === 'branch' ? {
    projection: 'projection', rawOracle: 'rawOracle', mutator: 'branchMutator',
    replaySupport: 'mutationReplaySupport', verifier: 'branchVerifier',
  } : {
    projection: 'projection', rawOracle: 'rawOracle', mutator: 'finalWhereMutator',
    oracle: 'finalWhereOracle', replaySupport: 'mutationReplaySupport',
    verifier: 'finalWhereVerifier',
  };
  assert(value !== null && typeof value === 'object'
    && JSON.stringify(Object.keys(value)) === JSON.stringify(Object.keys(mapping)),
  'REPLAY_V3_MUTATION_SOURCES_SHAPE_INVALID');
  for (const [actual, expected] of Object.entries(mapping)) {
    assert(JSON.stringify(value[actual]) === JSON.stringify({
      bytes: receipt.sources[expected].bytes, sha256: receipt.sources[expected].sha256,
    }), 'REPLAY_V3_MUTATION_SOURCE_INVALID');
  }
}
function branchMutationExpectations(control) {
  return [
    ...PUBLIC_ACL_OBJECT_CLASSES_V1.map((objectClass) => ({
      id: `delete-${objectClass}-branch`, kind: 'branch-deletion', objectClass,
      rejection: 'ORACLE_RECORD_BAG_KEYS_MISMATCH',
      delta: -(control.classCounts?.[objectClass] ?? Number.NaN),
    })),
    { id: 'return-zero', kind: 'record-set-sensitivity', objectClass: null,
      rejection: 'ORACLE_RECORD_BAG_KEYS_MISMATCH', delta: -control.recordCount },
    { id: 'omit-first-atom', kind: 'record-set-sensitivity', objectClass: null,
      rejection: 'ORACLE_RECORD_BAG_KEYS_MISMATCH', delta: -1 },
    { id: 'add-sentinel-atom', kind: 'record-set-sensitivity', objectClass: null,
      rejection: 'ORACLE_RECORD_BAG_KEYS_MISMATCH', delta: 1 },
    { id: 'substitute-first-atom', kind: 'record-set-sensitivity', objectClass: null,
      rejection: 'ORACLE_RECORD_BAG_MULTIPLICITY_MISMATCH', delta: 0 },
  ];
}
function deterministicRun(run) {
  return {
    capture: withoutVolume(run.capture), oracle: withoutVolume(run.oracle),
    ...(run.witness === undefined ? {} : { witness: withoutVolume(run.witness) }),
  };
}
function withoutVolume(value) {
  const { dataVolumeNameSha256: _volume, ...rest } = value;
  return rest;
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
function validateCounts(value, total, complete, code) {
  assert(value !== null && typeof value === 'object' && !Array.isArray(value)
    && Object.getPrototypeOf(value) === Object.prototype, code);
  const keys = Object.keys(value);
  assert(keys.length > 0 && keys.every((key) => PUBLIC_ACL_OBJECT_CLASSES_V1.includes(key))
    && (!complete || JSON.stringify(keys) === JSON.stringify([...PUBLIC_ACL_OBJECT_CLASSES_V1].sort()))
    && keys.every((key) => Number.isSafeInteger(value[key]) && value[key] >= 0)
    && Object.values(value).reduce((sum, count) => sum + count, 0) === total, code);
}
function normalizeCounts(value) {
  return Object.fromEntries([...PUBLIC_ACL_OBJECT_CLASSES_V1].sort()
    .map((key) => [key, value[key] ?? 0]));
}
function positive(value) { return Number.isSafeInteger(value) && value > 0; }
function digest(value) { return typeof value === 'string' && /^[0-9a-f]{64}$/.test(value); }
