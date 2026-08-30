// SPDX-License-Identifier: MIT
import { spawnSync } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import { lstatSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { TextDecoder } from 'node:util';
const IMAGE_REFERENCE =
  'postgres@sha256:485935f94cc7165afa896978809c37b592dc07f0a37d2c8f645f12412d0212c8';
const IMAGE_CONFIGURATION =
  'sha256:80f4c7a5e91618546dce5b4fe60cf03b14c0f9efa7e40157278d122772ced8d2';
const DATABASE_NAME = 'sf_public_baseline';
const PGDATA = '/var/lib/postgresql/data';
const OWNER_LABEL = 'org.semantic-fabric.postgresql-public-acl-replay.owner';
const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const RECEIPT_PATH = resolve(
  ROOT, '__tests__/fixtures/postgresql-16.15-public-acl-capture-receipt-v1.json',
);
const CAPTURE_PATH = resolve(ROOT, 'scripts/capture-postgresql-public-acl-baseline-v1.mjs');
const ORACLE_PATH = resolve(ROOT, 'scripts/verify-postgresql-public-acl-oracle-v1.mjs');
const SOURCE_PATHS = Object.freeze({
  projection: '__tests__/fixtures/postgresql-16.15-public-acl-projection-v1.sql',
  rawOracle: '__tests__/fixtures/postgresql-16.15-public-acl-completeness-oracle-v1.sql',
  captureRunner: 'scripts/capture-postgresql-public-acl-baseline-v1.mjs',
  oracleWire: 'scripts/postgresql-public-acl-oracle-wire-v1.mjs',
  oracleDeriver: 'scripts/postgresql-public-acl-oracle-v1.mjs',
  oracleRunner: 'scripts/verify-postgresql-public-acl-oracle-v1.mjs',
  replayRunner: 'scripts/replay-postgresql-public-acl-baseline-v1.mjs',
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
const CLASS_NAMES = Object.freeze([
  'column', 'foreign-data-wrapper', 'foreign-server', 'language', 'relation',
  'routine', 'schema', 'type',
]);
const MAX_DOCKER_OUTPUT = 2 * 1024 * 1024;
const MAX_CHILD_OUTPUT = 256 * 1024;
const owned = [];
assert(process.argv.length === 2, 'REPLAY_ARGUMENTS_INVALID');
let summary;
let primaryFailure;
try {
  const receiptSource = readRegular(RECEIPT_PATH, 64 * 1024, 'REPLAY_RECEIPT_FILE_INVALID');
  const receipt = parseCanonicalJson(receiptSource, 'REPLAY_RECEIPT_JSON_INVALID');
  validateReceipt(receipt);
  verifyCommittedFiles(receipt);
  const image = inspectImage();
  const token = randomBytes(16).toString('hex');
  const runs = [1, 2].map((sequence) => replayOnce(sequence, token, image, receipt));
  assert(runs[0].volumeName !== runs[1].volumeName
    && runs[0].capture.dataVolumeNameSha256 !== runs[1].capture.dataVolumeNameSha256,
  'REPLAY_VOLUMES_NOT_DISTINCT');
  assert(JSON.stringify(normalizeCounts(runs[0].capture.classCounts))
    === JSON.stringify(normalizeCounts(runs[1].capture.classCounts)),
  'REPLAY_CAPTURE_CLASS_COUNTS_NONDETERMINISTIC');
  assert(JSON.stringify(normalizeCounts(runs[0].oracle.classCounts))
    === JSON.stringify(normalizeCounts(runs[1].oracle.classCounts)),
  'REPLAY_ORACLE_CLASS_COUNTS_NONDETERMINISTIC');
  summary = buildSummary(receipt, receiptSource, runs);
} catch (error) {
  primaryFailure = error;
}
const cleanupFailures = cleanupOwned();
if (primaryFailure !== undefined) {
  const suffix = cleanupFailures.length === 0 ? '' : `;${cleanupFailures.join(';')}`;
  throw new Error(`${errorCode(primaryFailure)}${suffix}`, { cause: primaryFailure });
}
assert(cleanupFailures.length === 0, cleanupFailures.join(';'));
process.stdout.write(`${JSON.stringify(summary, null, 2)}\n`);
function replayOnce(sequence, token, image, receipt) {
  const name = `sf-pgacl-replay-${process.pid}-${token.slice(0, 12)}-${sequence}`;
  assert(/^[a-z0-9][a-z0-9-]{0,62}$/.test(name), 'REPLAY_CONTAINER_NAME_INVALID');
  assertContainerNameAbsent(name);
  const resource = { name, token, id: null, volumeName: null };
  owned.push(resource);
  const output = runOk('docker', [
    'run', '--detach', '--pull', 'never', '--platform', 'linux/amd64',
    '--name', name, '--network', 'none', '--label', `${OWNER_LABEL}=${token}`,
    '--env', 'POSTGRES_HOST_AUTH_METHOD=trust',
    '--env', 'POSTGRES_INITDB_ARGS=--locale=C --encoding=UTF8',
    '--mount', `type=volume,destination=${PGDATA}`, IMAGE_REFERENCE,
  ], undefined, 30_000, 128 * 1024, 'REPLAY_CONTAINER_CREATE_FAILED');
  assert(/^[0-9a-f]{64}\n$/.test(decodeUtf8(output, 'REPLAY_CONTAINER_ID_UTF8_INVALID')),
    'REPLAY_CONTAINER_ID_INVALID');
  resource.id = output.toString('ascii').trim();
  const container = inspectContainer(resource.id);
  validateContainer(container, resource, image);
  resource.volumeName = container.Mounts[0].Name;
  waitUntilReady(resource.id);
  assert(runPsql(resource.id,
    `SELECT count(*)::text FROM pg_catalog.pg_database WHERE datname = '${DATABASE_NAME}';\n`)
    .equals(Buffer.from('0\n')), 'REPLAY_DATABASE_PREEXISTED');
  const created = runOk('docker', [
    'exec', resource.id, 'createdb', '-U', 'postgres', '--maintenance-db=postgres',
    '--owner=postgres', '--template=template0', '--encoding=UTF8',
    '--locale-provider=libc', '--lc-collate=C', '--lc-ctype=C', DATABASE_NAME,
  ], undefined, 30_000, 64 * 1024, 'REPLAY_DATABASE_CREATE_FAILED');
  assert(created.byteLength === 0, 'REPLAY_DATABASE_CREATE_OUTPUT_INVALID');
  const metadata = runPsql(resource.id, String.raw`COPY (
SELECT datname, pg_catalog.pg_get_userbyid(datdba),
  pg_catalog.pg_encoding_to_char(encoding), datlocprovider, datcollate, datctype,
  daticulocale IS NULL
FROM pg_catalog.pg_database
WHERE datname = 'sf_public_baseline'
) TO STDOUT WITH (FORMAT text);
`);
  assert(metadata.equals(Buffer.from('sf_public_baseline\tpostgres\tUTF8\tc\tC\tC\tt\n')),
    'REPLAY_DATABASE_CONFIGURATION_INVALID');
  const capture = runChild(CAPTURE_PATH, name, 'REPLAY_CAPTURE');
  const oracle = runChild(ORACLE_PATH, name, 'REPLAY_ORACLE');
  validateCapture(capture, receipt);
  validateOracle(oracle, receipt);
  const volumeHash = sha256(Buffer.from(resource.volumeName, 'utf8'));
  assert(capture.dataVolumeNameSha256 === volumeHash
    && oracle.dataVolumeNameSha256 === volumeHash, 'REPLAY_VOLUME_EVIDENCE_MISMATCH');
  assert(capture.recordCount === oracle.recordCount
    && capture.recordsBytes === oracle.recordsBytes
    && capture.recordsSha256 === oracle.recordsSha256,
  'REPLAY_RESULT_EVIDENCE_MISMATCH');
  assert(JSON.stringify(normalizeCounts(capture.classCounts))
    === JSON.stringify(normalizeCounts(oracle.classCounts)),
  'REPLAY_CLASS_COUNTS_MISMATCH');
  return { sequence, volumeName: resource.volumeName, capture, oracle };
}
function validateReceipt(value) {
  exactObject(value, [
    'schemaVersion', 'authority', 'captureDate', 'image', 'database', 'sources',
    'profile', 'result', 'runs', 'replay',
  ], 'REPLAY_RECEIPT_SHAPE_INVALID');
  assert(value.schemaVersion === 'semantic-fabric.postgresql-public-acl-capture-receipt/v1'
    && value.authority === 'test-only-non-runtime'
    && /^\d{4}-\d{2}-\d{2}$/.test(value.captureDate), 'REPLAY_RECEIPT_IDENTITY_INVALID');
  exactObject(value.image, [
    'reference', 'manifestMediaType', 'platformManifestDigest', 'configurationDigest',
    'platform', 'serverVersion',
  ], 'REPLAY_RECEIPT_IMAGE_SHAPE_INVALID');
  assert(value.image.reference === IMAGE_REFERENCE
    && value.image.manifestMediaType === 'application/vnd.oci.image.manifest.v1+json'
    && value.image.platformManifestDigest === IMAGE_REFERENCE.slice('postgres@'.length)
    && value.image.configurationDigest === IMAGE_CONFIGURATION
    && value.image.platform === 'linux/amd64'
    && value.image.serverVersion === '16.15 (Debian 16.15-1.pgdg13+2)',
  'REPLAY_RECEIPT_IMAGE_INVALID');
  exactObject(value.database, [
    'name', 'owner', 'template', 'encoding', 'localeProvider', 'collation', 'ctype',
    'initdbArguments',
  ], 'REPLAY_RECEIPT_DATABASE_SHAPE_INVALID');
  assert(JSON.stringify(value.database) === JSON.stringify({
    name: DATABASE_NAME, owner: 'postgres', template: 'template0', encoding: 'UTF8',
    localeProvider: 'c', collation: 'C', ctype: 'C',
    initdbArguments: '--locale=C --encoding=UTF8',
  }), 'REPLAY_RECEIPT_DATABASE_INVALID');
  exactObject(value.sources, Object.keys(SOURCE_PATHS), 'REPLAY_RECEIPT_SOURCES_SHAPE_INVALID');
  for (const [key, path] of Object.entries(SOURCE_PATHS)) {
    const source = value.sources[key];
    exactObject(source, ['path', 'bytes', 'sha256'], 'REPLAY_RECEIPT_SOURCE_SHAPE_INVALID');
    assert(source.path === path && positiveInteger(source.bytes) && digest(source.sha256),
      'REPLAY_RECEIPT_SOURCE_INVALID');
  }
  exactObject(value.profile, [
    'sha256', 'defaultAclRows', 'parameterAclRows', 'foreignDataWrappers',
    'foreignServers', 'userMappings', 'largeObjects', 'dedicatedSchemaRows',
    'publicDependentObjects',
  ], 'REPLAY_RECEIPT_PROFILE_SHAPE_INVALID');
  assert(digest(value.profile.sha256)
    && Object.entries(value.profile).every(([key, item]) => key === 'sha256' || item === 0),
  'REPLAY_RECEIPT_PROFILE_INVALID');
  exactObject(value.result, ['fixturePath', 'records', 'nodes', 'bytes', 'sha256'],
    'REPLAY_RECEIPT_RESULT_SHAPE_INVALID');
  assert(value.result.fixturePath
    === '__tests__/fixtures/postgresql-16.15-clean-template0-public-object-acl-v1.json'
    && positiveInteger(value.result.records) && value.result.nodes === 1 + value.result.records * 9
    && positiveInteger(value.result.bytes) && digest(value.result.sha256),
  'REPLAY_RECEIPT_RESULT_INVALID');
  assert(Array.isArray(value.runs) && value.runs.length === 2, 'REPLAY_RECEIPT_RUNS_INVALID');
  for (const [index, run] of value.runs.entries()) {
    exactObject(run, [
      'sequence', 'networkMode', 'publishedPorts', 'dataVolumeNameSha256',
      'profileSha256',
      'rawTranscriptBytes', 'rawTranscriptSha256', 'sessionTranscriptBytes',
      'sessionTranscriptSha256', 'recordCount', 'recordsBytes', 'recordsSha256',
    ], 'REPLAY_RECEIPT_RUN_SHAPE_INVALID');
    assert(run.sequence === index + 1 && run.networkMode === 'none'
      && run.publishedPorts === false && digest(run.dataVolumeNameSha256)
      && run.profileSha256 === value.profile.sha256
      && positiveInteger(run.rawTranscriptBytes) && digest(run.rawTranscriptSha256)
      && positiveInteger(run.sessionTranscriptBytes) && digest(run.sessionTranscriptSha256)
      && run.recordCount === value.result.records && run.recordsBytes === value.result.bytes
      && run.recordsSha256 === value.result.sha256,
    'REPLAY_RECEIPT_RUN_INVALID');
  }
  const [first, second] = value.runs;
  assert(first.dataVolumeNameSha256 !== second.dataVolumeNameSha256
    && first.rawTranscriptBytes === second.rawTranscriptBytes
    && first.rawTranscriptSha256 === second.rawTranscriptSha256
    && first.sessionTranscriptBytes === second.sessionTranscriptBytes
    && first.sessionTranscriptSha256 === second.sessionTranscriptSha256,
  'REPLAY_RECEIPT_RUN_DETERMINISM_INVALID');
  exactObject(value.replay, [
    'minimumRuns', 'requiresDistinctAnonymousDataVolumes', 'requiresNoPublishedPorts',
    'runnerArgv', 'captureArgv', 'oracleArgv',
  ], 'REPLAY_RECEIPT_REPLAY_SHAPE_INVALID');
  assert(value.replay.minimumRuns === 2
    && value.replay.requiresDistinctAnonymousDataVolumes === true
    && value.replay.requiresNoPublishedPorts === true
    && JSON.stringify(value.replay.runnerArgv) === JSON.stringify([
      'node', 'scripts/replay-postgresql-public-acl-baseline-v1.mjs',
    ])
    && JSON.stringify(value.replay.captureArgv) === JSON.stringify([
      'node', 'scripts/capture-postgresql-public-acl-baseline-v1.mjs', 'CONTAINER_NAME',
    ])
    && JSON.stringify(value.replay.oracleArgv) === JSON.stringify([
      'node', 'scripts/verify-postgresql-public-acl-oracle-v1.mjs', 'CONTAINER_NAME',
    ]), 'REPLAY_RECEIPT_REPLAY_INVALID');
}
function verifyCommittedFiles(receipt) {
  for (const [key, relativePath] of Object.entries(SOURCE_PATHS)) {
    const source = readRegular(resolve(ROOT, relativePath), 256 * 1024,
      'REPLAY_COMMITTED_SOURCE_FILE_INVALID');
    assert(source.byteLength === receipt.sources[key].bytes
      && sha256(source) === receipt.sources[key].sha256,
    'REPLAY_COMMITTED_SOURCE_PIN_MISMATCH');
  }
  const fixture = readRegular(resolve(ROOT, receipt.result.fixturePath), 1_048_576,
    'REPLAY_COMMITTED_FIXTURE_FILE_INVALID');
  assert(fixture.byteLength === receipt.result.bytes
    && sha256(fixture) === receipt.result.sha256, 'REPLAY_COMMITTED_FIXTURE_PIN_MISMATCH');
}
function inspectImage() {
  const value = parseDockerSingleton(runOk('docker', ['image', 'inspect', IMAGE_REFERENCE],
    undefined, 30_000, MAX_DOCKER_OUTPUT, 'REPLAY_IMAGE_INSPECT_FAILED'),
  'REPLAY_IMAGE_INSPECT_JSON_INVALID');
  assert(value.Id === IMAGE_CONFIGURATION && value.Os === 'linux' && value.Architecture === 'amd64'
    && Array.isArray(value.RepoDigests) && value.RepoDigests.includes(IMAGE_REFERENCE)
    && value.Config !== null && typeof value.Config === 'object', 'REPLAY_IMAGE_IDENTITY_INVALID');
  return value;
}
function validateContainer(value, resource, image) {
  assert(value.Id === resource.id && value.Name === `/${resource.name}`
    && value.Config?.Image === IMAGE_REFERENCE && value.Image === IMAGE_CONFIGURATION
    && value.State?.Running === true && value.HostConfig?.NetworkMode === 'none'
    && value.HostConfig?.PublishAllPorts === false
    && noPublishedPorts(value.HostConfig?.PortBindings)
    && noPublishedPorts(value.NetworkSettings?.Ports), 'REPLAY_CONTAINER_IDENTITY_INVALID');
  assert(value.Config?.Labels?.[OWNER_LABEL] === resource.token
    && JSON.stringify(value.Config?.Entrypoint) === JSON.stringify(image.Config?.Entrypoint)
    && JSON.stringify(value.Config?.Cmd) === JSON.stringify(image.Config?.Cmd),
  'REPLAY_CONTAINER_DEFAULT_COMMAND_INVALID');
  const expectedEnv = [...(image.Config?.Env ?? []),
    'POSTGRES_HOST_AUTH_METHOD=trust', 'POSTGRES_INITDB_ARGS=--locale=C --encoding=UTF8'];
  assert(sameStringBag(value.Config?.Env, expectedEnv), 'REPLAY_CONTAINER_ENVIRONMENT_INVALID');
  assert((value.HostConfig?.Binds === null || value.HostConfig?.Binds?.length === 0)
    && emptyObject(value.HostConfig?.Tmpfs)
    && Array.isArray(value.Mounts) && value.Mounts.length === 1,
  'REPLAY_CONTAINER_MOUNTS_INVALID');
  const mount = value.Mounts[0];
  assert(mount?.Type === 'volume' && mount.Destination === PGDATA && mount.RW === true
    && /^[0-9a-f]{64}$/.test(mount.Name ?? ''), 'REPLAY_CONTAINER_VOLUME_INVALID');
  const configured = value.HostConfig?.Mounts;
  assert(Array.isArray(configured) && configured.length === 1
    && configured[0]?.Type === 'volume' && configured[0]?.Target === PGDATA
    && (configured[0]?.Source === undefined || configured[0]?.Source === '')
    && (configured[0]?.ReadOnly === undefined || configured[0]?.ReadOnly === false),
  'REPLAY_CONTAINER_ANONYMOUS_VOLUME_INVALID');
}
function validateCapture(value, receipt) {
  exactObject(value, CAPTURE_KEYS, 'REPLAY_CAPTURE_SHAPE_INVALID');
  assert(value.schemaVersion === 1
    && value.profile === 'postgresql-16.15-clean-template0-public-object-acl-v1'
    && value.image === IMAGE_REFERENCE && value.imageConfiguration === IMAGE_CONFIGURATION
    && value.platform === 'linux/amd64' && digest(value.dataVolumeNameSha256)
    && value.profileSha256 === receipt.profile.sha256
    && value.projectionBytes === receipt.sources.projection.bytes
    && value.projectionSha256 === receipt.sources.projection.sha256
    && value.recordCount === receipt.result.records && value.recordsBytes === receipt.result.bytes
    && value.recordsSha256 === receipt.result.sha256,
  'REPLAY_CAPTURE_EVIDENCE_INVALID');
  validateCounts(value.classCounts, value.recordCount, true, 'REPLAY_CAPTURE_CLASS_COUNTS_INVALID');
}
function validateOracle(value, receipt) {
  exactObject(value, ORACLE_KEYS, 'REPLAY_ORACLE_SHAPE_INVALID');
  assert(value.schemaVersion === 1
    && value.oracle === 'postgresql-16.15-public-acl-completeness-oracle-v1'
    && value.image === IMAGE_REFERENCE && value.imageConfiguration === IMAGE_CONFIGURATION
    && value.platform === 'linux/amd64' && digest(value.dataVolumeNameSha256)
    && value.oracleSourceBytes === receipt.sources.rawOracle.bytes
    && value.oracleSourceSha256 === receipt.sources.rawOracle.sha256
    && value.projectionSourceBytes === receipt.sources.projection.bytes
    && value.projectionSourceSha256 === receipt.sources.projection.sha256
    && receipt.runs.every((run) => value.rawTranscriptBytes === run.rawTranscriptBytes
      && value.rawTranscriptSha256 === run.rawTranscriptSha256
      && value.sessionTranscriptBytes === run.sessionTranscriptBytes
      && value.sessionTranscriptSha256 === run.sessionTranscriptSha256)
    && value.recordCount === receipt.result.records && value.recordsBytes === receipt.result.bytes
    && value.recordsSha256 === receipt.result.sha256,
  'REPLAY_ORACLE_EVIDENCE_INVALID');
  validateCounts(value.classCounts, value.recordCount, false, 'REPLAY_ORACLE_CLASS_COUNTS_INVALID');
}
function buildSummary(receipt, receiptSource, runs) {
  const transcript = receipt.runs[0];
  return {
    schemaVersion: 1,
    replay: 'postgresql-16.15-public-acl-baseline-v1',
    authority: 'test-only-non-runtime',
    receiptBytes: receiptSource.byteLength,
    receiptSha256: sha256(receiptSource),
    image: IMAGE_REFERENCE,
    imageConfiguration: IMAGE_CONFIGURATION,
    platform: 'linux/amd64',
    runs: runs.map((run) => ({
      sequence: run.sequence,
      networkMode: 'none',
      publishedPorts: false,
      dataVolumeNameSha256: run.capture.dataVolumeNameSha256,
      profileSha256: run.capture.profileSha256,
      rawTranscriptBytes: run.oracle.rawTranscriptBytes,
      rawTranscriptSha256: run.oracle.rawTranscriptSha256,
      sessionTranscriptBytes: run.oracle.sessionTranscriptBytes,
      sessionTranscriptSha256: run.oracle.sessionTranscriptSha256,
      recordCount: run.capture.recordCount,
      recordsBytes: run.capture.recordsBytes,
      recordsSha256: run.capture.recordsSha256,
    })),
    deterministic: {
      profileSha256: receipt.profile.sha256,
      projectionSourceBytes: receipt.sources.projection.bytes,
      projectionSourceSha256: receipt.sources.projection.sha256,
      rawOracleSourceBytes: receipt.sources.rawOracle.bytes,
      rawOracleSourceSha256: receipt.sources.rawOracle.sha256,
      rawTranscriptBytes: transcript.rawTranscriptBytes,
      rawTranscriptSha256: transcript.rawTranscriptSha256,
      sessionTranscriptBytes: transcript.sessionTranscriptBytes,
      sessionTranscriptSha256: transcript.sessionTranscriptSha256,
      recordCount: receipt.result.records,
      recordNodes: receipt.result.nodes,
      recordsBytes: receipt.result.bytes,
      recordsSha256: receipt.result.sha256,
    },
  };
}
function runChild(path, name, prefix) {
  const output = runOk(process.execPath, [path, name], undefined, 180_000, MAX_CHILD_OUTPUT,
    `${prefix}_COMMAND_FAILED`);
  return parseCanonicalJson(output, `${prefix}_JSON_INVALID`);
}
function runPsql(id, input) {
  return runOk('docker', [
    'exec', '-i', id, 'psql', '-U', 'postgres', '-d', 'postgres',
    '-X', '-q', '-A', '-t', '-v', 'ON_ERROR_STOP=1',
  ], Buffer.from(input, 'utf8'), 30_000, 128 * 1024, 'REPLAY_PSQL_FAILED');
}
function waitUntilReady(id) {
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline) {
    const result = runRaw('docker', [
      'exec', id, 'pg_isready', '-q', '-U', 'postgres', '-d', 'postgres',
    ], undefined, 5_000, 64 * 1024, 'REPLAY_READINESS_COMMAND_FAILED');
    assert(result.stdout.byteLength === 0 && result.stderr.byteLength === 0
      && [0, 1, 2, 3].includes(result.status), 'REPLAY_READINESS_OUTPUT_INVALID');
    if (result.status === 0) return;
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 250);
  }
  assert(false, 'REPLAY_READINESS_TIMEOUT');
}
function assertContainerNameAbsent(name) {
  const output = runOk('docker', [
    'container', 'ls', '--all', '--filter', `name=^/${name}$`, '--format', '{{.ID}}',
  ], undefined, 30_000, 64 * 1024, 'REPLAY_CONTAINER_PREFLIGHT_FAILED');
  assert(output.byteLength === 0, 'REPLAY_CONTAINER_NAME_COLLISION');
}
function inspectContainer(id) {
  return parseDockerSingleton(runOk('docker', ['container', 'inspect', id], undefined,
    30_000, MAX_DOCKER_OUTPUT, 'REPLAY_CONTAINER_INSPECT_FAILED'),
  'REPLAY_CONTAINER_INSPECT_JSON_INVALID');
}
function cleanupOwned() {
  const failures = [];
  for (const resource of [...owned].reverse()) {
    try {
      const listed = runOk('docker', [
        'container', 'ls', '--all', '--filter', `name=^/${resource.name}$`,
        '--filter', `label=${OWNER_LABEL}=${resource.token}`, '--format', '{{.ID}}',
      ], undefined, 30_000, 64 * 1024, 'REPLAY_CLEANUP_LIST_FAILED');
      const line = decodeUtf8(listed, 'REPLAY_CLEANUP_LIST_UTF8_INVALID');
      assert(/^[0-9a-f]{12,64}\n$/.test(line), 'REPLAY_CLEANUP_CONTAINER_MISSING');
      const container = inspectContainer(resource.name);
      assert(container.Name === `/${resource.name}` && container.Config?.Labels?.[OWNER_LABEL]
        === resource.token && (resource.id === null || container.Id === resource.id),
      'REPLAY_CLEANUP_OWNERSHIP_INVALID');
      const mounts = container.Mounts;
      assert(Array.isArray(mounts) && mounts.length === 1 && mounts[0]?.Type === 'volume'
        && mounts[0]?.Destination === PGDATA
        && (resource.volumeName === null || mounts[0]?.Name === resource.volumeName),
      'REPLAY_CLEANUP_VOLUME_OWNERSHIP_INVALID');
      resource.volumeName = mounts[0].Name;
      const removed = runOk('docker', ['container', 'rm', '--force', container.Id], undefined,
        30_000, 64 * 1024, 'REPLAY_CLEANUP_CONTAINER_REMOVE_FAILED');
      const removedName = decodeUtf8(removed, 'REPLAY_CLEANUP_CONTAINER_REMOVE_UTF8_INVALID');
      assert(removedName === `${container.Id}\n` || removedName === `${resource.name}\n`,
        'REPLAY_CLEANUP_CONTAINER_REMOVE_OUTPUT_INVALID');
      const volume = runOk('docker', ['volume', 'rm', resource.volumeName], undefined,
        30_000, 64 * 1024, 'REPLAY_CLEANUP_VOLUME_REMOVE_FAILED');
      assert(decodeUtf8(volume, 'REPLAY_CLEANUP_VOLUME_REMOVE_UTF8_INVALID')
        === `${resource.volumeName}\n`, 'REPLAY_CLEANUP_VOLUME_REMOVE_OUTPUT_INVALID');
    } catch (error) {
      failures.push(errorCode(error));
    }
  }
  return failures;
}
function runOk(file, args, input, timeout, maxBuffer, code) {
  const result = runRaw(file, args, input, timeout, maxBuffer, code);
  assert(result.status === 0 && result.stderr.byteLength === 0, code);
  return result.stdout;
}
function runRaw(file, args, input, timeout, maxBuffer, code) {
  const result = spawnSync(file, args, { cwd: ROOT, input, timeout, maxBuffer, shell: false });
  assert(result.error === undefined && result.signal === null
    && Number.isInteger(result.status) && Buffer.isBuffer(result.stdout)
    && Buffer.isBuffer(result.stderr), code);
  return result;
}
function parseDockerSingleton(source, code) {
  let value;
  try { value = JSON.parse(decodeUtf8(source, code)); } catch { throw new Error(code); }
  assert(Array.isArray(value) && value.length === 1 && value[0] !== null
    && typeof value[0] === 'object' && !Array.isArray(value[0]), code);
  return value[0];
}
function parseCanonicalJson(source, code) {
  const text = decodeUtf8(source, code);
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
  assert(keys.length > 0 && keys.every((key) => CLASS_NAMES.includes(key))
    && (!complete || JSON.stringify(keys) === JSON.stringify(CLASS_NAMES))
    && keys.every((key) => Number.isSafeInteger(value[key]) && value[key] >= 0)
    && Object.values(value).reduce((sum, item) => sum + item, 0) === total, code);
}
function normalizeCounts(value) {
  return Object.fromEntries(CLASS_NAMES.map((key) => [key, value[key] ?? 0]));
}
function exactObject(value, keys, code) {
  assert(value !== null && typeof value === 'object' && !Array.isArray(value)
    && Object.getPrototypeOf(value) === Object.prototype
    && JSON.stringify(Object.keys(value)) === JSON.stringify(keys), code);
}
function noPublishedPorts(value) {
  if (value === null || value === undefined) return true;
  return emptyObject(value) || (typeof value === 'object' && !Array.isArray(value)
    && Object.values(value).every((entry) => entry === null
      || (Array.isArray(entry) && entry.length === 0)));
}
function emptyObject(value) {
  return value === null || value === undefined
    || (typeof value === 'object' && !Array.isArray(value) && Object.keys(value).length === 0);
}
function sameStringBag(actual, expected) {
  return Array.isArray(actual) && actual.every((item) => typeof item === 'string')
    && JSON.stringify([...actual].sort()) === JSON.stringify([...expected].sort());
}
function decodeUtf8(value, code) {
  try { return new TextDecoder('utf-8', { fatal: true }).decode(value); } catch { throw new Error(code); }
}
function positiveInteger(value) { return Number.isSafeInteger(value) && value > 0; }
function digest(value) { return typeof value === 'string' && /^[0-9a-f]{64}$/.test(value); }
function sha256(value) { return createHash('sha256').update(value).digest('hex'); }
function errorCode(error) {
  return error instanceof Error && /^[A-Z0-9_;]+$/.test(error.message)
    ? error.message : 'REPLAY_UNEXPECTED_FAILURE';
}
function assert(condition, code) { if (!condition) throw new Error(code); }
