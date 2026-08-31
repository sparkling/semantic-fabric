// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { lstatSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseWitnessInventorySession }
  from './postgresql-public-acl-has-privilege-inventory-v1.mjs';
import { verifyWitnessSession }
  from './postgresql-public-acl-has-privilege-v1.mjs';
import { sha256 } from './postgresql-public-acl-oracle-v1.mjs';
import { assert } from './postgresql-public-acl-oracle-wire-v1.mjs';

const IMAGE_REFERENCE =
  'postgres@sha256:485935f94cc7165afa896978809c37b592dc07f0a37d2c8f645f12412d0212c8';
const IMAGE_CONFIGURATION =
  'sha256:80f4c7a5e91618546dce5b4fe60cf03b14c0f9efa7e40157278d122772ced8d2';
const DATABASE_NAME = 'sf_public_baseline';
const ROLE_NAME = 'sf_public_acl_no_membership_witness_v1';
const PGDATA = '/var/lib/postgresql/data';
const OWNER_LABEL = 'org.semantic-fabric.postgresql-public-acl-replay.owner';
const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const INVENTORY_PATH = resolve(
  ROOT, '__tests__/fixtures/postgresql-16.15-public-acl-has-privilege-inventory-v1.sql',
);
const WITNESS_PATH = resolve(
  ROOT, '__tests__/fixtures/postgresql-16.15-public-acl-has-privilege-witness-v1.sql',
);
const FIXTURE_PATH = resolve(
  ROOT, '__tests__/fixtures/postgresql-16.15-clean-template0-public-object-acl-v1.json',
);
const CREATE_ROLE = Buffer.from(`CREATE ROLE ${ROLE_NAME}
  NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT
  NOREPLICATION NOBYPASSRLS CONNECTION LIMIT -1;\n`, 'utf8');
const DROP_ROLE = Buffer.from(`DROP ROLE ${ROLE_NAME};\n`, 'utf8');

const args = process.argv.slice(2);
assert(args.length === 1 && /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/.test(args[0]),
  'WITNESS_CONTAINER_ARGUMENT_INVALID');
const containerName = args[0];
let createAttempted = false;
let summary;
let primaryFailure;
try {
  summary = execute();
} catch (error) {
  primaryFailure = error;
}
let cleanupFailure;
if (createAttempted) {
  try {
    if (roleCount() === 1) {
      const dropped = runPsql('postgres', DROP_ROLE, 64 * 1024);
      assert(dropped.byteLength === 0, 'WITNESS_ROLE_DROP_OUTPUT_INVALID');
    }
    assert(roleCount() === 0, 'WITNESS_ROLE_POST_DROP_INVALID');
  } catch (error) {
    cleanupFailure = error;
  }
}
if (primaryFailure !== undefined || cleanupFailure !== undefined) {
  const codes = [primaryFailure, cleanupFailure].filter(Boolean).map(errorCode);
  throw new Error(codes.join(';'));
}
summary.cleanup = Object.freeze({
  preflightAbsent: true, created: true, dropped: true, postDropAbsent: true,
});
process.stdout.write(`${JSON.stringify(summary, null, 2)}\n`);

function execute() {
  const container = inspectOne('docker', ['inspect', containerName],
    'WITNESS_CONTAINER_INSPECT_INVALID');
  const image = inspectOne('docker', ['image', 'inspect', IMAGE_CONFIGURATION],
    'WITNESS_IMAGE_INSPECT_INVALID');
  validateImage(image);
  validateContainer(container, image);
  const mounts = container.Mounts.filter((mount) => mount?.Destination === PGDATA);
  const inventorySource = readRegular(INVENTORY_PATH, 512 * 1024,
    'WITNESS_INVENTORY_SOURCE_INVALID');
  const witnessSource = readRegular(WITNESS_PATH, 512 * 1024,
    'WITNESS_SOURCE_INVALID');
  const fixture = readRegular(FIXTURE_PATH, 1024 * 1024,
    'WITNESS_FIXTURE_SOURCE_INVALID');
  assert(roleCount() === 0, 'WITNESS_ROLE_PREEXISTING');
  const inventoryTranscript = runPsql(
    DATABASE_NAME, inventorySource, 16 * 1024 * 1024, 120_000,
  );
  let inventory;
  try { inventory = parseWitnessInventorySession(inventoryTranscript); }
  catch (error) { throw new Error('WITNESS_INVENTORY_EVIDENCE_INVALID', { cause: error }); }
  createAttempted = true;
  const created = runPsql('postgres', CREATE_ROLE, 64 * 1024);
  assert(created.byteLength === 0, 'WITNESS_ROLE_CREATE_OUTPUT_INVALID');
  const witnessTranscript = runPsql(
    DATABASE_NAME, witnessSource, 16 * 1024 * 1024, 120_000,
  );
  let result;
  try { result = verifyWitnessSession(witnessTranscript, fixture, inventory.canonical); }
  catch (error) { throw new Error('WITNESS_SEMANTIC_EVIDENCE_INVALID', { cause: error }); }
  return {
    schemaVersion: 1,
    witness: 'postgresql-16.15-public-acl-has-privilege-witness-v1',
    authority: 'test-only-non-runtime',
    roleName: ROLE_NAME,
    image: IMAGE_REFERENCE,
    imageConfiguration: IMAGE_CONFIGURATION,
    platform: 'linux/amd64',
    dataVolumeNameSha256: sha256(Buffer.from(mounts[0].Name, 'utf8')),
    inventorySourceBytes: inventorySource.byteLength,
    inventorySourceSha256: sha256(inventorySource),
    witnessSourceBytes: witnessSource.byteLength,
    witnessSourceSha256: sha256(witnessSource),
    fixtureBytes: fixture.byteLength,
    fixtureSha256: sha256(fixture),
    inventoryTranscriptBytes: inventoryTranscript.byteLength,
    inventoryTranscriptSha256: sha256(inventoryTranscript),
    witnessTranscriptBytes: witnessTranscript.byteLength,
    witnessTranscriptSha256: sha256(witnessTranscript),
    inventoryEntries: inventory.entries.length,
    ...result,
  };
}

function roleCount() {
  const query = Buffer.from(`COPY (
SELECT pg_catalog.count(*) FROM pg_catalog.pg_authid
WHERE rolname = '${ROLE_NAME}'
) TO STDOUT WITH (FORMAT text);\n`, 'utf8');
  const output = runPsql('postgres', query, 64 * 1024);
  assert(/^(?:0|1)\n$/.test(output.toString('ascii')), 'WITNESS_ROLE_COUNT_INVALID');
  return Number(output.toString('ascii').trim());
}

function validateContainer(value, image) {
  assert(value.Name === `/${containerName}` && value.Config?.Image === IMAGE_REFERENCE
    && value.Image === IMAGE_CONFIGURATION && value.State?.Running === true
    && value.HostConfig?.NetworkMode === 'none'
    && value.HostConfig?.PublishAllPorts === false, 'WITNESS_CONTAINER_IDENTITY_INVALID');
  assert(/^[0-9a-f]{32}$/.test(value.Config?.Labels?.[OWNER_LABEL] ?? ''),
    'WITNESS_CONTAINER_OWNERSHIP_INVALID');
  assert(JSON.stringify(value.Config?.Entrypoint) === JSON.stringify(image.Config?.Entrypoint)
    && JSON.stringify(value.Config?.Cmd) === JSON.stringify(image.Config?.Cmd),
  'WITNESS_CONTAINER_DEFAULT_COMMAND_INVALID');
  assert(noPublishedPorts(value.HostConfig?.PortBindings)
    && noPublishedPorts(value.NetworkSettings?.Ports), 'WITNESS_CONTAINER_PORTS_INVALID');
  assert((value.HostConfig?.Binds === null || value.HostConfig?.Binds?.length === 0)
    && emptyObject(value.HostConfig?.Tmpfs), 'WITNESS_CONTAINER_MOUNTS_INVALID');
  const mounts = Array.isArray(value.Mounts) ? value.Mounts : [];
  assert(mounts.length === 1 && mounts[0]?.Type === 'volume' && mounts[0]?.RW === true
    && mounts[0]?.Destination === PGDATA && /^[0-9a-f]{64}$/.test(mounts[0]?.Name ?? ''),
  'WITNESS_CONTAINER_VOLUME_INVALID');
  const configured = Array.isArray(value.HostConfig?.Mounts)
    ? value.HostConfig.Mounts.filter((mount) => mount?.Target === PGDATA) : [];
  assert(configured.length === 1 && configured[0]?.Type === 'volume'
    && (configured[0]?.Source === undefined || configured[0]?.Source === '')
    && (configured[0]?.ReadOnly === undefined || configured[0]?.ReadOnly === false),
  'WITNESS_CONTAINER_VOLUME_CONFIGURATION_INVALID');
  const expectedEnv = [...(image.Config?.Env ?? []),
    'POSTGRES_HOST_AUTH_METHOD=trust', 'POSTGRES_INITDB_ARGS=--locale=C --encoding=UTF8'];
  assert(sameStringBag(value.Config?.Env, expectedEnv),
  'WITNESS_CONTAINER_ENVIRONMENT_INVALID');
}

function validateImage(value) {
  assert(value.Id === IMAGE_CONFIGURATION && value.Os === 'linux'
    && value.Architecture === 'amd64' && Array.isArray(value.RepoDigests)
    && value.RepoDigests.includes(IMAGE_REFERENCE), 'WITNESS_IMAGE_IDENTITY_INVALID');
}

function runPsql(database, input, maxBuffer, timeout = 30_000) {
  return run('docker', [
    'exec', '-i', containerName, 'psql', '-U', 'postgres', '-d', database,
    '-X', '-q', '-A', '-t', '-v', 'ON_ERROR_STOP=1',
  ], input, maxBuffer, timeout, 'WITNESS_PSQL_FAILED');
}

function inspectOne(file, commandArgs, code) {
  const output = run(file, commandArgs, undefined, 2 * 1024 * 1024, 30_000, code);
  let parsed;
  try { parsed = JSON.parse(output.toString('utf8')); } catch { throw new Error(code); }
  assert(Array.isArray(parsed) && parsed.length === 1, code);
  return parsed[0];
}

function run(file, commandArgs, input, maxBuffer, timeout, code) {
  const result = spawnSync(file, commandArgs, {
    input, maxBuffer, shell: false, timeout,
  });
  assert(result.error === undefined && result.signal === null && result.status === 0, code);
  assert(Buffer.isBuffer(result.stdout) && Buffer.isBuffer(result.stderr)
    && result.stderr.byteLength === 0 && result.stdout.byteLength <= maxBuffer, code);
  return result.stdout;
}

function readRegular(path, maximumBytes, code) {
  const stat = lstatSync(path);
  assert(stat.isFile() && !stat.isSymbolicLink() && stat.nlink === 1
    && stat.size > 0 && stat.size <= maximumBytes, code);
  const value = readFileSync(path);
  assert(value.byteLength === stat.size, code);
  return value;
}

function noPublishedPorts(value) {
  if (value === null || value === undefined) return true;
  if (typeof value !== 'object' || Array.isArray(value)) return false;
  return Object.values(value).every((entry) => entry === null
    || (Array.isArray(entry) && entry.length === 0));
}

function emptyObject(value) {
  return value === null || value === undefined
    || (typeof value === 'object' && !Array.isArray(value) && Object.keys(value).length === 0);
}

function sameStringBag(left, right) {
  return Array.isArray(left) && Array.isArray(right)
    && JSON.stringify([...left].sort()) === JSON.stringify([...right].sort());
}

function errorCode(error) {
  return error instanceof Error && /^WITNESS_[A-Z0-9_]+$/.test(error.message)
    ? error.message : 'WITNESS_INTERNAL_ERROR';
}
