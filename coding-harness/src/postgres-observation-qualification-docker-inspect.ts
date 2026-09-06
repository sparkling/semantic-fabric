// SPDX-License-Identifier: MIT

import { lstatSync, readFileSync, realpathSync } from 'node:fs';
import { asClosedRecord, asDenseArray, assertExactKeys } from './contracts.js';
import type {
  QualificationBuilderImageIdentity,
  QualificationImageIdentity,
} from './postgres-observation-qualification-provenance.js';
import type {
  PostgresQualificationBuilderImage,
  PostgresQualificationImage,
  PostgresQualificationProtocol,
} from './postgres-observation-qualification-protocol.js';
import {
  expectedQualificationBuilderImageIdentity,
  expectedQualificationImageIdentity,
} from
  './postgres-observation-qualification-protocol.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export const DOCKER_EXECUTABLE = '/usr/bin/docker' as const;
export const DOCKER_ENVIRONMENT = Object.freeze({
  PATH: '/usr/bin:/bin',
  HOME: '/nonexistent',
  LANG: 'C',
  LC_ALL: 'C',
  DOCKER_CONFIG: '/nonexistent',
  DOCKER_HOST: 'unix:///var/run/docker.sock',
} as const);

export const IMAGE_INSPECT_FORMAT =
  '{"Id":{{json .Id}},"Os":{{json .Os}},"Architecture":{{json .Architecture}},"RepoDigests":{{json .RepoDigests}}}' as const;
export const CONTAINER_INSPECT_FORMAT =
  '{"Id":{{json .Id}},"Image":{{json .Image}},"Name":{{json .Name}},"Running":{{json .State.Running}},"NetworkMode":{{json .HostConfig.NetworkMode}},"PortBindings":{{json .HostConfig.PortBindings}},"ReadonlyRootfs":{{json .HostConfig.ReadonlyRootfs}},"Tmpfs":{{json .HostConfig.Tmpfs}},"Mounts":{{json .Mounts}},"Labels":{{json .Config.Labels}},"Env":{{json .Config.Env}}}' as const;

const SHA256_PREFIXED = /^sha256:[a-f0-9]{64}$/;
const CONTAINER_ID = /^[a-f0-9]{64}$/;

export interface QualificationContainerExpectation {
  readonly name: string;
  readonly id: string;
  readonly token: string;
  readonly volumeName: string;
  readonly fixturePath: string;
  readonly artifactPath: string;
  readonly imageConfigDigest: string;
  readonly running: boolean;
}

export function validateDockerHost(): void {
  const executable = lstatSync(DOCKER_EXECUTABLE);
  const socket = lstatSync('/var/run/docker.sock');
  const socketPath = realpathSync('/var/run/docker.sock');
  if (!executable.isFile() || executable.isSymbolicLink()
    || realpathSync(DOCKER_EXECUTABLE) !== DOCKER_EXECUTABLE
    || executable.uid !== 0 || executable.nlink !== 1
    || (executable.mode & 0o111) === 0 || (executable.mode & 0o022) !== 0
    || !socket.isSocket() || socket.isSymbolicLink() || socket.uid !== 0 || socket.nlink !== 1
    || (socketPath !== '/var/run/docker.sock' && socketPath !== '/run/docker.sock')) {
    throw new Error('POSTGRES_QUALIFICATION_DOCKER_HOST_UNTRUSTED');
  }
  // Reading the fixed executable also detects an inaccessible or unstable host binary.
  const bytes = readFileSync(DOCKER_EXECUTABLE);
  const after = lstatSync(DOCKER_EXECUTABLE);
  if (bytes.length !== executable.size || after.dev !== executable.dev
    || after.ino !== executable.ino || after.size !== executable.size
    || after.mtimeMs !== executable.mtimeMs || after.ctimeMs !== executable.ctimeMs
    || after.mode !== executable.mode || after.uid !== executable.uid) {
    throw new Error('POSTGRES_QUALIFICATION_DOCKER_EXECUTABLE_CHANGED');
  }
}

export function parseQualificationImageInspection(
  serialized: Buffer,
  expected: PostgresQualificationImage,
): QualificationImageIdentity {
  verifyImageInspection(serialized, expected.configDigest, expected.reference);
  return expectedQualificationImageIdentity(expected);
}

export function parseQualificationBuilderImageInspection(
  serialized: Buffer,
  expected: PostgresQualificationBuilderImage,
): QualificationBuilderImageIdentity {
  verifyImageInspection(serialized, expected.configDigest, expected.reference);
  return expectedQualificationBuilderImageIdentity(expected);
}

function verifyImageInspection(
  serialized: Buffer,
  expectedConfigDigest: string,
  expectedReference: string,
): void {
  const root = parseDockerJson(serialized, [
    'Id', 'Os', 'Architecture', 'RepoDigests',
  ], 'image inspection');
  const configDigest = prefixedDigest(root.Id, 'image configuration');
  if (configDigest !== expectedConfigDigest || root.Os !== 'linux'
    || root.Architecture !== 'amd64') {
    throw new Error('POSTGRES_QUALIFICATION_IMAGE_IDENTITY_MISMATCH');
  }
  const repoDigests = asDenseArray(root.RepoDigests, 'image repository digests');
  if (repoDigests.length < 1 || repoDigests.length > 16
    || repoDigests.some((entry) => typeof entry !== 'string')
    || new Set(repoDigests).size !== repoDigests.length
    || !repoDigests.includes(expectedReference)) {
    throw new Error('POSTGRES_QUALIFICATION_IMAGE_MANIFEST_MISSING');
  }
}

export function verifyQualificationContainerInspection(
  serialized: Buffer,
  protocol: PostgresQualificationProtocol,
  expected: QualificationContainerExpectation,
): void {
  const root = parseDockerJson(serialized, [
    'Id', 'Image', 'Name', 'Running', 'NetworkMode', 'PortBindings',
    'ReadonlyRootfs', 'Tmpfs', 'Mounts', 'Labels', 'Env',
  ], 'container inspection');
  if (root.Id !== expected.id || !CONTAINER_ID.test(expected.id)
    || root.Image !== expected.imageConfigDigest
    || root.Name !== `/${expected.name}` || root.Running !== expected.running
    || root.NetworkMode !== protocol.container.networkMode
    || root.ReadonlyRootfs !== true) {
    throw new Error('POSTGRES_QUALIFICATION_CONTAINER_IDENTITY_MISMATCH');
  }
  const ports = asClosedRecord(root.PortBindings, 'container port bindings');
  if (Object.keys(ports).length !== 0) {
    throw new Error('POSTGRES_QUALIFICATION_CONTAINER_PORT_EXPOSURE');
  }
  const tmpfs = asClosedRecord(root.Tmpfs, 'container tmpfs');
  if (Object.keys(tmpfs).sort().join('\n') !== '/tmp\n/var/run/postgresql'
    || tmpfs['/tmp'] !== 'rw,noexec,nosuid,nodev,size=64m'
    || tmpfs[protocol.container.socketPath] !== 'rw,nosuid,nodev,size=16m') {
    throw new Error('POSTGRES_QUALIFICATION_CONTAINER_TMPFS_MISMATCH');
  }
  const labels = asClosedRecord(root.Labels, 'container labels');
  if (labels[protocol.container.ownerLabelKey] !== expected.token) {
    throw new Error('POSTGRES_QUALIFICATION_CONTAINER_OWNERSHIP_MISMATCH');
  }
  verifyEnvironment(root.Env, protocol);
  verifyMounts(root.Mounts, protocol, expected);
}

export function verifyQualificationVolumeInspection(
  serialized: Buffer,
  protocol: PostgresQualificationProtocol,
  volumeName: string,
  token: string,
): void {
  const root = parseDockerJson(serialized, ['Name', 'Labels'], 'volume inspection');
  const labels = asClosedRecord(root.Labels, 'volume labels');
  if (root.Name !== volumeName || labels[protocol.container.ownerLabelKey] !== token) {
    throw new Error('POSTGRES_QUALIFICATION_VOLUME_OWNERSHIP_MISMATCH');
  }
}

function verifyEnvironment(value: unknown, protocol: PostgresQualificationProtocol): void {
  const environment = asDenseArray(value, 'container environment');
  if (environment.some((entry) => typeof entry !== 'string' || entry.includes('\0'))) {
    throw new TypeError('POSTGRES_QUALIFICATION_CONTAINER_ENVIRONMENT_INVALID');
  }
  const required = [
    `PGDATA=${protocol.container.pgdataPath}`,
    'POSTGRES_HOST_AUTH_METHOD=trust',
    'POSTGRES_INITDB_ARGS=--locale-provider=libc --locale=C --encoding=UTF8',
  ];
  if (required.some((entry) => environment.filter((actual) => actual === entry).length !== 1)) {
    throw new Error('POSTGRES_QUALIFICATION_CONTAINER_ENVIRONMENT_MISMATCH');
  }
  const names = environment.map((entry) => (entry as string).split('=', 1)[0]);
  if (new Set(names).size !== names.length) {
    throw new Error('POSTGRES_QUALIFICATION_CONTAINER_ENVIRONMENT_DUPLICATE');
  }
}

function verifyMounts(
  value: unknown,
  protocol: PostgresQualificationProtocol,
  expected: QualificationContainerExpectation,
): void {
  const mounts = asDenseArray(value, 'container mounts');
  if (mounts.length !== 3) throw new Error('POSTGRES_QUALIFICATION_CONTAINER_MOUNT_COUNT');
  const parsed = mounts.map((value) => {
    const row = asClosedRecord(value, 'container mount');
    return {
      type: row.Type,
      name: row.Name,
      source: row.Source,
      destination: row.Destination,
      rw: row.RW,
    };
  });
  const volume = parsed.find((entry) => entry.destination === protocol.container.pgdataPath);
  const fixture = parsed.find((entry) => entry.destination === protocol.container.fixturePath);
  const artifact = parsed.find((entry) => entry.destination === protocol.container.probePath);
  if (volume?.type !== 'volume' || volume.name !== expected.volumeName || volume.rw !== true
    || fixture?.type !== 'bind' || fixture.source !== expected.fixturePath || fixture.rw !== false
    || artifact?.type !== 'bind' || artifact.source !== expected.artifactPath
    || artifact.rw !== false) {
    throw new Error('POSTGRES_QUALIFICATION_CONTAINER_MOUNT_MISMATCH');
  }
}

function parseDockerJson(
  value: Buffer,
  keys: readonly string[],
  label: string,
): Record<string, unknown> {
  if (value.length < 3 || value.length > 1_048_576 || value.at(-1) !== 0x0a) {
    throw new TypeError(`POSTGRES_QUALIFICATION_DOCKER_${labelCode(label)}_ENCODING`);
  }
  const text = value.subarray(0, value.length - 1).toString('utf8');
  if (!value.subarray(0, value.length - 1).equals(Buffer.from(text, 'utf8'))
    || text.includes('\r') || text.includes('\0') || text.includes('\n')) {
    throw new TypeError(`POSTGRES_QUALIFICATION_DOCKER_${labelCode(label)}_ENCODING`);
  }
  const root = asClosedRecord(parseJsonWithoutDuplicateKeys(text, label), label);
  assertExactKeys(root, keys, label);
  return root;
}

function prefixedDigest(value: unknown, label: string): string {
  if (typeof value !== 'string' || !SHA256_PREFIXED.test(value)) {
    throw new TypeError(`POSTGRES_QUALIFICATION_${labelCode(label)}_INVALID`);
  }
  return value;
}

function labelCode(label: string): string {
  return label.toUpperCase().replaceAll(/[^A-Z0-9]+/g, '_');
}
