// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { lstatSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  canonicalFixture, compareRecordBags, deriveOracleRecords, parseProjectionRecords, sha256,
} from './postgresql-public-acl-oracle-v1.mjs';
import { assert, parseOracleSession } from './postgresql-public-acl-oracle-wire-v1.mjs';

const IMAGE_REFERENCE =
  'postgres@sha256:485935f94cc7165afa896978809c37b592dc07f0a37d2c8f645f12412d0212c8';
const IMAGE_CONFIGURATION =
  'sha256:80f4c7a5e91618546dce5b4fe60cf03b14c0f9efa7e40157278d122772ced8d2';
const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const ORACLE_PATH = resolve(
  ROOT, '__tests__/fixtures/postgresql-16.15-public-acl-completeness-oracle-v1.sql',
);
const PROJECTION_PATH = resolve(
  ROOT, '__tests__/fixtures/postgresql-16.15-public-acl-projection-v1.sql',
);
const ROLLBACK = Buffer.from('ROLLBACK;\n', 'utf8');
const PROJECTION_BEGIN = Buffer.from("SELECT '@@ADR0047-PROJECTION/BEGIN@@';\n", 'utf8');
const PROJECTION_END = Buffer.from("\nSELECT '@@ADR0047-PROJECTION/END@@';\nROLLBACK;\n", 'utf8');

const args = process.argv.slice(2);
assert(args.length === 1 && /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/.test(args[0]),
  'ORACLE_CONTAINER_ARGUMENT_INVALID');
const containerName = args[0];
const container = parseSingleJson(run('docker', ['inspect', containerName]),
  'ORACLE_CONTAINER_INSPECT_INVALID');
assert(container.Name === `/${containerName}` && container.Config?.Image === IMAGE_REFERENCE
  && container.Image === IMAGE_CONFIGURATION && container.State?.Running === true,
'ORACLE_CONTAINER_IDENTITY_INVALID');
assert(noPublishedPorts(container.HostConfig?.PortBindings)
  && noPublishedPorts(container.NetworkSettings?.Ports), 'ORACLE_CONTAINER_PORTS_INVALID');
const dataMounts = Array.isArray(container.Mounts)
  ? container.Mounts.filter((mount) => mount?.Destination === '/var/lib/postgresql/data') : [];
assert(dataMounts.length === 1 && dataMounts[0]?.Type === 'volume'
  && typeof dataMounts[0]?.Name === 'string' && dataMounts[0].Name.length > 0,
'ORACLE_CONTAINER_VOLUME_INVALID');
const env = Array.isArray(container.Config?.Env) ? container.Config.Env : [];
assert(env.includes('POSTGRES_INITDB_ARGS=--locale=C --encoding=UTF8')
  && env.includes('POSTGRES_HOST_AUTH_METHOD=trust'), 'ORACLE_CONTAINER_ENVIRONMENT_INVALID');

const image = parseSingleJson(run('docker', ['image', 'inspect', IMAGE_CONFIGURATION]),
  'ORACLE_IMAGE_INSPECT_INVALID');
assert(image.Id === IMAGE_CONFIGURATION && image.Os === 'linux' && image.Architecture === 'amd64'
  && Array.isArray(image.RepoDigests) && image.RepoDigests.includes(IMAGE_REFERENCE),
'ORACLE_IMAGE_IDENTITY_INVALID');

const oracleSource = readRegular(ORACLE_PATH, 'ORACLE_SOURCE_INVALID');
const projectionSource = readRegular(PROJECTION_PATH, 'ORACLE_PROJECTION_SOURCE_INVALID');
assert(oracleSource.subarray(-ROLLBACK.byteLength).equals(ROLLBACK),
  'ORACLE_SOURCE_ROLLBACK_INVALID');
assertIndependentSql(oracleSource.toString('utf8'));
const session = Buffer.concat([
  oracleSource.subarray(0, -ROLLBACK.byteLength), PROJECTION_BEGIN,
  projectionSource, PROJECTION_END,
]);
const transcript = runPsql(containerName, session);
const parsed = parseOracleSession(transcript);
const expected = deriveOracleRecords(parsed.raw);
const actual = parseProjectionRecords(parsed.projection);
compareRecordBags(expected, actual);
const expectedFixture = canonicalFixture(expected);
const actualFixture = canonicalFixture(actual);
assert(expectedFixture.equals(actualFixture), 'ORACLE_RECORD_ORDER_OR_BYTES_MISMATCH');
assert(expectedFixture.byteLength <= 1_048_576, 'ORACLE_RECORD_BYTES_INVALID');
const projectionMarker = Buffer.from('@@ADR0047-PROJECTION/BEGIN@@\n', 'utf8');
const markerOffset = transcript.indexOf(projectionMarker);
assert(markerOffset > 0, 'ORACLE_RAW_TRANSCRIPT_BOUNDARY_INVALID');
const classCounts = Object.create(null);
for (const row of expected) classCounts[row.objectClass] = (classCounts[row.objectClass] ?? 0) + 1;

process.stdout.write(`${JSON.stringify({
  schemaVersion: 1,
  oracle: 'postgresql-16.15-public-acl-completeness-oracle-v1',
  image: IMAGE_REFERENCE,
  imageConfiguration: IMAGE_CONFIGURATION,
  platform: 'linux/amd64',
  dataVolumeNameSha256: sha256(Buffer.from(dataMounts[0].Name, 'utf8')),
  oracleSourceBytes: oracleSource.byteLength,
  oracleSourceSha256: sha256(oracleSource),
  projectionSourceBytes: projectionSource.byteLength,
  projectionSourceSha256: sha256(projectionSource),
  rawTranscriptBytes: markerOffset,
  rawTranscriptSha256: sha256(transcript.subarray(0, markerOffset)),
  sessionTranscriptBytes: transcript.byteLength,
  sessionTranscriptSha256: sha256(transcript),
  recordCount: expected.length,
  recordsBytes: expectedFixture.byteLength,
  recordsSha256: sha256(expectedFixture),
  classCounts,
}, null, 2)}\n`);

function assertIndependentSql(value) {
  const forbidden = [
    /json(?:b)?_build_object/iu, /acl(?:default)/iu, /coalesce/iu,
    /grantee\s*=\s*0/iu, /\bunion\b/iu, /pg_get_function_identity_arguments/iu,
  ];
  forbidden.forEach((pattern) => assert(!pattern.test(value), 'ORACLE_SOURCE_INDEPENDENCE_INVALID'));
}

function runPsql(name, input) {
  return run('docker', [
    'exec', '-i', name, 'psql', '-U', 'postgres', '-d', 'sf_public_baseline',
    '-X', '-q', '-A', '-t', '-v', 'ON_ERROR_STOP=1',
  ], input, 16 * 1024 * 1024);
}

function run(file, commandArgs, input = undefined, maxBuffer = 2 * 1024 * 1024) {
  const result = spawnSync(file, commandArgs, { input, maxBuffer, shell: false });
  assert(result.error === undefined && result.signal === null && result.status === 0,
    'ORACLE_COMMAND_FAILED');
  assert(Buffer.isBuffer(result.stdout) && Buffer.isBuffer(result.stderr)
    && result.stderr.byteLength === 0, 'ORACLE_COMMAND_OUTPUT_INVALID');
  return result.stdout;
}

function readRegular(path, code) {
  const stat = lstatSync(path);
  assert(stat.isFile() && !stat.isSymbolicLink() && stat.nlink === 1, code);
  return readFileSync(path);
}

function parseSingleJson(value, code) {
  let parsed;
  try { parsed = JSON.parse(value.toString('utf8')); } catch { throw new Error(code); }
  assert(Array.isArray(parsed) && parsed.length === 1, code);
  return parsed[0];
}

function noPublishedPorts(value) {
  if (value === null || value === undefined) return true;
  if (typeof value !== 'object' || Array.isArray(value)) return false;
  return Object.values(value).every((entry) => entry === null
    || (Array.isArray(entry) && entry.length === 0));
}
