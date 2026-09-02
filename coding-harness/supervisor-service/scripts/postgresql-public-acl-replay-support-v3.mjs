// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { TextDecoder } from 'node:util';
import { assert } from './postgresql-public-acl-oracle-wire-v1.mjs';
import { sha256 } from './postgresql-public-acl-oracle-v1.mjs';

export const IMAGE_REFERENCE =
  'postgres@sha256:485935f94cc7165afa896978809c37b592dc07f0a37d2c8f645f12412d0212c8';
export const IMAGE_CONFIGURATION =
  'sha256:80f4c7a5e91618546dce5b4fe60cf03b14c0f9efa7e40157278d122772ced8d2';
export const DATABASE_NAME = 'sf_public_baseline';
export const OWNER_LABEL = 'org.semantic-fabric.postgresql-public-acl-replay.owner';
const PGDATA = '/var/lib/postgresql/data';
const MAX_DOCKER_OUTPUT = 2 * 1024 * 1024;
const MAX_CHILD_OUTPUT = 256 * 1024;

export function runOwnedReplayPair(root, childPaths) {
  validateChildPaths(childPaths);
  const owned = [];
  let result;
  let primaryFailure;
  try {
    const image = inspectImage(root);
    const token = randomBytes(16).toString('hex');
    result = [1, 2].map((sequence) => replayOnce(
      root, childPaths, owned, sequence, token, image,
    ));
  } catch (error) {
    primaryFailure = error;
  }
  const cleanupFailures = cleanupOwned(root, owned);
  if (primaryFailure !== undefined) {
    const suffix = cleanupFailures.length === 0 ? '' : `;${cleanupFailures.join(';')}`;
    throw new Error(`${errorCode(primaryFailure)}${suffix}`, { cause: primaryFailure });
  }
  assert(cleanupFailures.length === 0, cleanupFailures.join(';'));
  return result;
}

function validateChildPaths(value) {
  assert(value !== null && typeof value === 'object' && !Array.isArray(value)
    && Object.getPrototypeOf(value) === Object.prototype
    && JSON.stringify(Object.keys(value)) === JSON.stringify(value.witness === undefined
      ? ['capture', 'oracle'] : ['capture', 'oracle', 'witness'])
    && Object.values(value).every((path) => typeof path === 'string'
      && /^scripts\/[A-Za-z0-9.-]+\.mjs$/.test(path)), 'REPLAY_V3_CHILD_PATHS_INVALID');
}

function replayOnce(root, childPaths, owned, sequence, token, image) {
  const name = `sf-pgacl-v3-${process.pid}-${token.slice(0, 12)}-${sequence}`;
  assert(/^[a-z0-9][a-z0-9-]{0,62}$/.test(name), 'REPLAY_V3_CONTAINER_NAME_INVALID');
  assertContainerNameAbsent(root, name);
  const resource = { name, token, id: null, volumeName: null };
  owned.push(resource);
  const output = runOk(root, 'docker', [
    'run', '--detach', '--pull', 'never', '--platform', 'linux/amd64',
    '--name', name, '--network', 'none', '--label', `${OWNER_LABEL}=${token}`,
    '--env', 'POSTGRES_HOST_AUTH_METHOD=trust',
    '--env', 'POSTGRES_INITDB_ARGS=--locale=C --encoding=UTF8',
    '--mount', `type=volume,destination=${PGDATA}`, IMAGE_REFERENCE,
  ], undefined, 30_000, 128 * 1024, 'REPLAY_V3_CONTAINER_CREATE_FAILED');
  assert(/^[0-9a-f]{64}\n$/.test(decodeUtf8(output, 'REPLAY_V3_CONTAINER_ID_UTF8_INVALID')),
    'REPLAY_V3_CONTAINER_ID_INVALID');
  resource.id = output.toString('ascii').trim();
  const container = inspectContainer(root, resource.id);
  validateContainer(container, resource, image);
  resource.volumeName = container.Mounts[0].Name;
  waitUntilReady(root, resource.id);
  assert(runPsql(root, resource.id,
    `SELECT count(*)::text FROM pg_catalog.pg_database WHERE datname = '${DATABASE_NAME}';\n`)
    .equals(Buffer.from('0\n')), 'REPLAY_V3_DATABASE_PREEXISTED');
  const created = runOk(root, 'docker', [
    'exec', resource.id, 'createdb', '-U', 'postgres', '--maintenance-db=postgres',
    '--owner=postgres', '--template=template0', '--encoding=UTF8',
    '--locale-provider=libc', '--lc-collate=C', '--lc-ctype=C', DATABASE_NAME,
  ], undefined, 30_000, 64 * 1024, 'REPLAY_V3_DATABASE_CREATE_FAILED');
  assert(created.byteLength === 0, 'REPLAY_V3_DATABASE_CREATE_OUTPUT_INVALID');
  const metadata = runPsql(root, resource.id, String.raw`COPY (
SELECT datname, pg_catalog.pg_get_userbyid(datdba),
  pg_catalog.pg_encoding_to_char(encoding), datlocprovider, datcollate, datctype,
  daticulocale IS NULL
FROM pg_catalog.pg_database WHERE datname = 'sf_public_baseline'
) TO STDOUT WITH (FORMAT text);
`);
  assert(metadata.equals(Buffer.from('sf_public_baseline\tpostgres\tUTF8\tc\tC\tC\tt\n')),
    'REPLAY_V3_DATABASE_CONFIGURATION_INVALID');
  const capture = runChild(root, childPaths.capture, name, 'REPLAY_V3_CAPTURE');
  const oracle = runChild(root, childPaths.oracle, name, 'REPLAY_V3_ORACLE');
  const witness = childPaths.witness === undefined
    ? undefined : runChild(root, childPaths.witness, name, 'REPLAY_V3_WITNESS');
  const volumeHash = sha256(Buffer.from(resource.volumeName, 'utf8'));
  assert(capture.dataVolumeNameSha256 === volumeHash
    && oracle.dataVolumeNameSha256 === volumeHash
    && (witness === undefined || witness.dataVolumeNameSha256 === volumeHash),
  'REPLAY_V3_VOLUME_EVIDENCE_MISMATCH');
  return witness === undefined
    ? { sequence, volumeName: resource.volumeName, capture, oracle }
    : { sequence, volumeName: resource.volumeName, capture, oracle, witness };
}

function inspectImage(root) {
  const value = parseDockerSingleton(runOk(root, 'docker', [
    'image', 'inspect', IMAGE_REFERENCE,
  ], undefined, 30_000, MAX_DOCKER_OUTPUT, 'REPLAY_V3_IMAGE_INSPECT_FAILED'),
  'REPLAY_V3_IMAGE_INSPECT_JSON_INVALID');
  assert(value.Id === IMAGE_CONFIGURATION && value.Os === 'linux'
    && value.Architecture === 'amd64' && Array.isArray(value.RepoDigests)
    && value.RepoDigests.includes(IMAGE_REFERENCE)
    && value.Config !== null && typeof value.Config === 'object',
  'REPLAY_V3_IMAGE_IDENTITY_INVALID');
  return value;
}

function validateContainer(value, resource, image) {
  assert(value.Id === resource.id && value.Name === `/${resource.name}`
    && value.Config?.Image === IMAGE_REFERENCE && value.Image === IMAGE_CONFIGURATION
    && value.State?.Running === true && value.HostConfig?.NetworkMode === 'none'
    && value.HostConfig?.PublishAllPorts === false
    && noPublishedPorts(value.HostConfig?.PortBindings)
    && noPublishedPorts(value.NetworkSettings?.Ports), 'REPLAY_V3_CONTAINER_IDENTITY_INVALID');
  assert(value.Config?.Labels?.[OWNER_LABEL] === resource.token
    && JSON.stringify(value.Config?.Entrypoint) === JSON.stringify(image.Config?.Entrypoint)
    && JSON.stringify(value.Config?.Cmd) === JSON.stringify(image.Config?.Cmd),
  'REPLAY_V3_CONTAINER_DEFAULT_COMMAND_INVALID');
  const expectedEnv = [...(image.Config?.Env ?? []),
    'POSTGRES_HOST_AUTH_METHOD=trust', 'POSTGRES_INITDB_ARGS=--locale=C --encoding=UTF8'];
  assert(sameStringBag(value.Config?.Env, expectedEnv),
    'REPLAY_V3_CONTAINER_ENVIRONMENT_INVALID');
  assert((value.HostConfig?.Binds === null || value.HostConfig?.Binds?.length === 0)
    && emptyObject(value.HostConfig?.Tmpfs)
    && Array.isArray(value.Mounts) && value.Mounts.length === 1,
  'REPLAY_V3_CONTAINER_MOUNTS_INVALID');
  const mount = value.Mounts[0];
  assert(mount?.Type === 'volume' && mount.Destination === PGDATA && mount.RW === true
    && /^[0-9a-f]{64}$/.test(mount.Name ?? ''), 'REPLAY_V3_CONTAINER_VOLUME_INVALID');
  const configured = value.HostConfig?.Mounts;
  assert(Array.isArray(configured) && configured.length === 1
    && configured[0]?.Type === 'volume' && configured[0]?.Target === PGDATA
    && (configured[0]?.Source === undefined || configured[0]?.Source === '')
    && (configured[0]?.ReadOnly === undefined || configured[0]?.ReadOnly === false),
  'REPLAY_V3_CONTAINER_ANONYMOUS_VOLUME_INVALID');
}

function cleanupOwned(root, owned) {
  const failures = [];
  for (const resource of [...owned].reverse()) {
    try {
      const listed = runOk(root, 'docker', [
        'container', 'ls', '--all', '--filter', `name=^/${resource.name}$`,
        '--filter', `label=${OWNER_LABEL}=${resource.token}`, '--format', '{{.ID}}',
      ], undefined, 30_000, 64 * 1024, 'REPLAY_V3_CLEANUP_LIST_FAILED');
      const line = decodeUtf8(listed, 'REPLAY_V3_CLEANUP_LIST_UTF8_INVALID');
      if (line === '') {
        assert(resource.id === null && resource.volumeName === null,
          'REPLAY_V3_CLEANUP_CONTAINER_MISSING');
        continue;
      }
      assert(/^[0-9a-f]{12,64}\n$/.test(line), 'REPLAY_V3_CLEANUP_CONTAINER_LIST_INVALID');
      const container = inspectContainer(root, resource.name);
      assert(container.Name === `/${resource.name}`
        && container.Config?.Labels?.[OWNER_LABEL] === resource.token
        && (resource.id === null || container.Id === resource.id),
      'REPLAY_V3_CLEANUP_OWNERSHIP_INVALID');
      const mounts = container.Mounts;
      assert(Array.isArray(mounts) && mounts.length === 1 && mounts[0]?.Type === 'volume'
        && mounts[0]?.Destination === PGDATA
        && (resource.volumeName === null || mounts[0]?.Name === resource.volumeName),
      'REPLAY_V3_CLEANUP_VOLUME_OWNERSHIP_INVALID');
      resource.volumeName = mounts[0].Name;
      const removed = runOk(root, 'docker', ['container', 'rm', '--force', container.Id],
        undefined, 30_000, 64 * 1024, 'REPLAY_V3_CLEANUP_CONTAINER_REMOVE_FAILED');
      const removedText = decodeUtf8(removed, 'REPLAY_V3_CLEANUP_CONTAINER_REMOVE_UTF8_INVALID');
      assert(removedText === `${container.Id}\n` || removedText === `${resource.name}\n`,
        'REPLAY_V3_CLEANUP_CONTAINER_REMOVE_OUTPUT_INVALID');
      const volume = runOk(root, 'docker', ['volume', 'rm', resource.volumeName], undefined,
        30_000, 64 * 1024, 'REPLAY_V3_CLEANUP_VOLUME_REMOVE_FAILED');
      assert(decodeUtf8(volume, 'REPLAY_V3_CLEANUP_VOLUME_REMOVE_UTF8_INVALID')
        === `${resource.volumeName}\n`, 'REPLAY_V3_CLEANUP_VOLUME_REMOVE_OUTPUT_INVALID');
      assertContainerNameAbsent(root, resource.name);
      const remaining = runOk(root, 'docker', ['volume', 'ls', '--filter',
        `name=^${resource.volumeName}$`, '--format', '{{.Name}}'], undefined,
      30_000, 64 * 1024, 'REPLAY_V3_CLEANUP_VOLUME_POSTFLIGHT_FAILED');
      assert(remaining.byteLength === 0, 'REPLAY_V3_CLEANUP_VOLUME_REMAINED');
    } catch (error) {
      failures.push(errorCode(error));
    }
  }
  return failures;
}

function runChild(root, path, name, prefix) {
  const output = runOk(root, process.execPath, [path, name], undefined, 300_000,
    MAX_CHILD_OUTPUT, `${prefix}_COMMAND_FAILED`);
  return parseCanonicalJson(output, `${prefix}_JSON_INVALID`);
}

function runPsql(root, id, input) {
  return runOk(root, 'docker', [
    'exec', '-i', id, 'psql', '-U', 'postgres', '-d', 'postgres',
    '-X', '-q', '-A', '-t', '-v', 'ON_ERROR_STOP=1',
  ], Buffer.from(input, 'utf8'), 30_000, 128 * 1024, 'REPLAY_V3_PSQL_FAILED');
}

function waitUntilReady(root, id) {
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline) {
    const lifecycle = runRaw(root, 'docker', ['exec', id, 'cat', '/proc/1/comm'],
      undefined, 5_000, 64 * 1024, 'REPLAY_V3_LIFECYCLE_COMMAND_FAILED');
    assert(lifecycle.status === 0 && lifecycle.stderr.byteLength === 0
      && /^[A-Za-z0-9_.-]{1,15}\n$/.test(decodeUtf8(lifecycle.stdout,
        'REPLAY_V3_LIFECYCLE_OUTPUT_INVALID')), 'REPLAY_V3_LIFECYCLE_OUTPUT_INVALID');
    const readiness = runRaw(root, 'docker', [
      'exec', id, 'pg_isready', '-q', '-U', 'postgres', '-d', 'postgres',
    ], undefined, 5_000, 64 * 1024, 'REPLAY_V3_READINESS_COMMAND_FAILED');
    assert(readiness.stdout.byteLength === 0 && readiness.stderr.byteLength === 0
      && [0, 1, 2, 3].includes(readiness.status), 'REPLAY_V3_READINESS_OUTPUT_INVALID');
    if (finalPostgresServerReady(lifecycle, readiness)) return;
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 250);
  }
  assert(false, 'REPLAY_V3_READINESS_TIMEOUT');
}

export function finalPostgresServerReady(lifecycle, readiness) {
  return lifecycle.status === 0 && lifecycle.stderr.byteLength === 0
    && lifecycle.stdout.equals(Buffer.from('postgres\n')) && readiness.status === 0
    && readiness.stdout.byteLength === 0 && readiness.stderr.byteLength === 0;
}

function assertContainerNameAbsent(root, name) {
  const output = runOk(root, 'docker', [
    'container', 'ls', '--all', '--filter', `name=^/${name}$`, '--format', '{{.ID}}',
  ], undefined, 30_000, 64 * 1024, 'REPLAY_V3_CONTAINER_PREFLIGHT_FAILED');
  assert(output.byteLength === 0, 'REPLAY_V3_CONTAINER_NAME_COLLISION');
}

function inspectContainer(root, id) {
  return parseDockerSingleton(runOk(root, 'docker', ['container', 'inspect', id], undefined,
    30_000, MAX_DOCKER_OUTPUT, 'REPLAY_V3_CONTAINER_INSPECT_FAILED'),
  'REPLAY_V3_CONTAINER_INSPECT_JSON_INVALID');
}

function runOk(root, file, args, input, timeout, maxBuffer, code) {
  const result = runRaw(root, file, args, input, timeout, maxBuffer, code);
  assert(result.status === 0 && result.stderr.byteLength === 0, code);
  return result.stdout;
}

function runRaw(root, file, args, input, timeout, maxBuffer, code) {
  const result = spawnSync(file, args, { cwd: root, input, timeout, maxBuffer, shell: false });
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

function noPublishedPorts(value) {
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
  try { return new TextDecoder('utf-8', { fatal: true }).decode(value); }
  catch { throw new Error(code); }
}

function errorCode(error) {
  return error instanceof Error && /^REPLAY_V3_[A-Z0-9_]+$/.test(error.message)
    ? error.message : 'REPLAY_V3_INTERNAL_ERROR';
}
