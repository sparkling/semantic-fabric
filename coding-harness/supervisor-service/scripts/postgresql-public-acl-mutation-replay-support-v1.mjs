// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import {
  closeSync, constants, fstatSync, openSync, readFileSync, realpathSync,
} from 'node:fs';
import { isAbsolute, relative, resolve, sep } from 'node:path';
import { TextDecoder } from 'node:util';
import { assert } from './postgresql-public-acl-oracle-wire-v1.mjs';
import {
  DATABASE_NAME, IMAGE_CONFIGURATION, IMAGE_REFERENCE, OWNER_LABEL,
} from './postgresql-public-acl-replay-support-v2.mjs';

const PGDATA = '/var/lib/postgresql/data';
const MAX_MUTATION_SOURCE_BYTES = 1024 * 1024;
const MUTATION_PARENT_CHILD_TIMEOUT_MS = 300_000;
const MUTATION_CONTAINER_INSPECT_TIMEOUT_MS = 15_000;
const MUTATION_PSQL_TIMEOUTS_V1 = Object.freeze({ probe: 15_000, session: 60_000 });
const BOUNDED_EXTERNAL_MS = (2 * MUTATION_CONTAINER_INSPECT_TIMEOUT_MS)
  + (2 * MUTATION_PSQL_TIMEOUTS_V1.probe) + MUTATION_PSQL_TIMEOUTS_V1.session;
export const MUTATION_REPLAY_TIMEOUTS_V1 = Object.freeze({
  containerInspectMs: MUTATION_CONTAINER_INSPECT_TIMEOUT_MS,
  probePsqlMs: MUTATION_PSQL_TIMEOUTS_V1.probe,
  sessionPsqlMs: MUTATION_PSQL_TIMEOUTS_V1.session,
  boundedExternalMs: BOUNDED_EXTERNAL_MS,
  parentChildMs: MUTATION_PARENT_CHILD_TIMEOUT_MS,
  parentHeadroomMs: MUTATION_PARENT_CHILD_TIMEOUT_MS - BOUNDED_EXTERNAL_MS,
});
assert(MUTATION_REPLAY_TIMEOUTS_V1.parentHeadroomMs >= 180_000,
  'ACL_MUTATION_TIMEOUT_BUDGET_INVALID');

export function inspectMutationContainer(name) {
  assert(typeof name === 'string' && /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/.test(name),
    'ACL_MUTATION_CONTAINER_ARGUMENT_INVALID');
  const value = parseSingleton(runOk(undefined, 'docker', ['container', 'inspect', name],
    undefined, MUTATION_CONTAINER_INSPECT_TIMEOUT_MS, 2 * 1024 * 1024,
    'ACL_MUTATION_CONTAINER_INSPECT_FAILED'));
  const image = parseSingleton(runOk(undefined, 'docker', ['image', 'inspect', IMAGE_CONFIGURATION],
    undefined, MUTATION_CONTAINER_INSPECT_TIMEOUT_MS, 2 * 1024 * 1024,
    'ACL_MUTATION_IMAGE_INSPECT_FAILED'));
  return validateMutationContainerSnapshotV1(name, value, image);
}

export function validateMutationContainerSnapshotV1(name, value, image) {
  assert(typeof name === 'string' && /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/.test(name)
    && value !== null && typeof value === 'object' && !Array.isArray(value)
    && image !== null && typeof image === 'object' && !Array.isArray(image)
    && image.Config !== null && typeof image.Config === 'object'
    && Array.isArray(image.Config.Env) && image.Config.Env.every(isString),
  'ACL_MUTATION_CONTAINER_IDENTITY_INVALID');
  const mounts = value.Mounts;
  const configuredMounts = value.HostConfig?.Mounts;
  const expectedEnv = [...(image.Config?.Env ?? []),
    'POSTGRES_HOST_AUTH_METHOD=trust', 'POSTGRES_INITDB_ARGS=--locale=C --encoding=UTF8'];
  assert(image.Id === IMAGE_CONFIGURATION && image.Os === 'linux' && image.Architecture === 'amd64'
    && Array.isArray(image.RepoDigests) && image.RepoDigests.includes(IMAGE_REFERENCE)
    && /^[0-9a-f]{64}$/.test(value.Id ?? '')
    && value.Name === `/${name}` && value.Config?.Image === IMAGE_REFERENCE
    && value.Image === IMAGE_CONFIGURATION && value.State?.Running === true
    && value.HostConfig?.NetworkMode === 'none' && value.HostConfig?.PublishAllPorts === false
    && noPorts(value.HostConfig?.PortBindings) && noPorts(value.NetworkSettings?.Ports)
    && /^[0-9a-f]{32}$/.test(value.Config?.Labels?.[OWNER_LABEL] ?? '')
    && JSON.stringify(value.Config?.Entrypoint) === JSON.stringify(image.Config?.Entrypoint)
    && JSON.stringify(value.Config?.Cmd) === JSON.stringify(image.Config?.Cmd)
    && sameStringBag(value.Config?.Env, expectedEnv)
    && Array.isArray(mounts) && mounts.length === 1 && mounts[0]?.Type === 'volume'
    && mounts[0]?.Destination === PGDATA && mounts[0]?.RW === true
    && /^[0-9a-f]{64}$/.test(mounts[0]?.Name ?? '')
    && Array.isArray(configuredMounts) && configuredMounts.length === 1
    && configuredMounts[0]?.Type === 'volume' && configuredMounts[0]?.Target === PGDATA
    && (configuredMounts[0]?.Source === undefined || configuredMounts[0]?.Source === '')
    && (configuredMounts[0]?.ReadOnly === undefined || configuredMounts[0]?.ReadOnly === false)
    && noEntries(value.HostConfig?.Binds) && noEntries(value.HostConfig?.Tmpfs),
  'ACL_MUTATION_CONTAINER_IDENTITY_INVALID');
  return Object.freeze({ id: value.Id, volumeName: mounts[0].Name });
}

export function buildMutationPsqlInvocationV1(id) {
  assert(typeof id === 'string' && /^[0-9a-f]{64}$/.test(id),
    'ACL_MUTATION_PSQL_ARGUMENTS_INVALID');
  return Object.freeze([
    'exec', '-i', id, 'psql', '-U', 'postgres', '-d', DATABASE_NAME,
    '-X', '-q', '-A', '-t', '-v', 'ON_ERROR_STOP=1',
  ]);
}

export function runMutationPsql(root, id, source, maxBuffer, operation) {
  assert(typeof root === 'string' && isAbsolute(root)
    && Buffer.isBuffer(source) && source.byteLength > 0 && source.byteLength <= 1024 * 1024
    && Number.isSafeInteger(maxBuffer) && maxBuffer > 0 && maxBuffer <= 32 * 1024 * 1024
    && Object.hasOwn(MUTATION_PSQL_TIMEOUTS_V1, operation),
  'ACL_MUTATION_PSQL_ARGUMENTS_INVALID');
  const args = buildMutationPsqlInvocationV1(id);
  return runOk(root, 'docker', args, source, MUTATION_PSQL_TIMEOUTS_V1[operation], maxBuffer,
    'ACL_MUTATION_PSQL_FAILED');
}

export function readMutationSource(root, relativePath, limit) {
  assert(typeof root === 'string' && isAbsolute(root)
    && typeof relativePath === 'string' && relativePath.length > 0
    && !isAbsolute(relativePath) && !relativePath.includes('\0')
    && Number.isSafeInteger(limit) && limit > 0 && limit <= MAX_MUTATION_SOURCE_BYTES,
  'ACL_MUTATION_SOURCE_ARGUMENTS_INVALID');
  const rootPath = realpathSync(resolve(root));
  const relativeToRoot = relative(rootPath, resolve(rootPath, relativePath));
  assert(relativeToRoot.length > 0 && relativeToRoot !== '..'
    && !relativeToRoot.startsWith(`..${sep}`) && !isAbsolute(relativeToRoot),
  'ACL_MUTATION_SOURCE_ARGUMENTS_INVALID');
  const components = relativeToRoot.split(sep);
  assert(components.length > 0 && components.every((component) => component.length > 0
    && component !== '.' && component !== '..'), 'ACL_MUTATION_SOURCE_ARGUMENTS_INVALID');
  const descriptors = [];
  try {
    let rootDescriptor = openSync(rootPath, constants.O_RDONLY | constants.O_DIRECTORY
      | constants.O_NOFOLLOW | constants.O_NONBLOCK);
    descriptors.push(rootDescriptor);
    assert(realpathSync(`/proc/self/fd/${rootDescriptor}`) === rootPath,
      'ACL_MUTATION_SOURCE_FILE_INVALID');
    for (const component of components.slice(0, -1)) {
      const directoryDescriptor = openSync(`/proc/self/fd/${rootDescriptor}/${component}`,
        constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
      descriptors.push(directoryDescriptor);
      const stat = fstatSync(directoryDescriptor, { bigint: true });
      assert(stat.isDirectory()
        && containedByRoot(rootPath, realpathSync(`/proc/self/fd/${directoryDescriptor}`)),
      'ACL_MUTATION_SOURCE_FILE_INVALID');
      rootDescriptor = directoryDescriptor;
    }
    const fileDescriptor = openSync(
      `/proc/self/fd/${rootDescriptor}/${components.at(-1)}`,
      constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK,
    );
    descriptors.push(fileDescriptor);
    const before = fstatSync(fileDescriptor, { bigint: true });
    const descriptorPath = `/proc/self/fd/${fileDescriptor}`;
    const actualPath = realpathSync(descriptorPath);
    assert(before.isFile() && before.nlink === 1n && before.size > 0n
      && before.size <= BigInt(limit) && containedByRoot(rootPath, actualPath),
    'ACL_MUTATION_SOURCE_FILE_INVALID');
    const bytes = readFileSync(fileDescriptor);
    const after = fstatSync(fileDescriptor, { bigint: true });
    assert(bytes.byteLength === Number(before.size) && sameFile(before, after),
      'ACL_MUTATION_SOURCE_FILE_INVALID');
    return bytes;
  } catch (error) {
    if (error instanceof Error && /^ACL_MUTATION_SOURCE_[A-Z_]+$/.test(error.message)) throw error;
    throw new Error('ACL_MUTATION_SOURCE_FILE_INVALID', { cause: error });
  } finally {
    // No spawn can overlap this synchronous walk; every held descriptor closes here.
    [...descriptors].reverse().forEach((descriptor) => closeSync(descriptor));
  }
}

export function decodeMutationUtf8(value, code) {
  try { return new TextDecoder('utf-8', { fatal: true }).decode(value); }
  catch { throw new Error(code); }
}

export function classifyMutationCommandResultV1(result, code) {
  assert(typeof code === 'string' && /^ACL_MUTATION_[A-Z0-9_]+$/.test(code),
    'ACL_MUTATION_COMMAND_CLASSIFIER_ARGUMENTS_INVALID');
  if (result?.error !== undefined) {
    const suffix = result.error?.code === 'ETIMEDOUT' ? 'TIMEOUT'
      : result.error?.code === 'ENOBUFS' ? 'OUTPUT_LIMIT' : 'SPAWN';
    throw new Error(`${code}_${suffix}`, { cause: result.error });
  }
  if (!Buffer.isBuffer(result?.stdout) || !Buffer.isBuffer(result?.stderr))
    throw new Error(`${code}_RESULT`);
  if (result.signal !== null) throw new Error(`${code}_SIGNAL`);
  if (result.status !== 0) throw new Error(`${code}_STATUS`);
  if (result.stderr.byteLength !== 0) throw new Error(`${code}_STDERR`);
  return result.stdout;
}

function runOk(cwd, file, args, input, timeout, maxBuffer, code) {
  const result = spawnSync(file, args, {
    cwd, input, timeout, maxBuffer, shell: false, killSignal: 'SIGKILL',
  });
  return classifyMutationCommandResultV1(result, code);
}

function parseSingleton(source) {
  let value;
  try { value = JSON.parse(decodeMutationUtf8(source, 'ACL_MUTATION_INSPECT_JSON_INVALID')); }
  catch { throw new Error('ACL_MUTATION_INSPECT_JSON_INVALID'); }
  assert(Array.isArray(value) && value.length === 1, 'ACL_MUTATION_INSPECT_JSON_INVALID');
  return value[0];
}

function noPorts(value) {
  return value === null || value === undefined
    || (typeof value === 'object' && !Array.isArray(value)
      && Object.values(value).every((entry) => entry === null
        || (Array.isArray(entry) && entry.length === 0)));
}

function noEntries(value) {
  return value === null || value === undefined
    || (Array.isArray(value) && value.length === 0)
    || (typeof value === 'object' && !Array.isArray(value) && Object.keys(value).length === 0);
}

function containedByRoot(rootPath, actualPath) {
  const value = relative(rootPath, actualPath);
  return value.length > 0 && value !== '..' && !value.startsWith(`..${sep}`)
    && !isAbsolute(value);
}

function sameFile(left, right) {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.nlink === right.nlink && left.size === right.size
    && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}

function isString(value) { return typeof value === 'string'; }

function sameStringBag(actual, expected) {
  return Array.isArray(actual) && actual.every((item) => typeof item === 'string')
    && JSON.stringify([...actual].sort()) === JSON.stringify([...expected].sort());
}
