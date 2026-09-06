// SPDX-License-Identifier: MIT

import { createHash, randomBytes } from 'node:crypto';
import { lstatSync, realpathSync } from 'node:fs';
import { performance } from 'node:perf_hooks';
import { isAbsolute, relative, resolve } from 'node:path';
import type { PostgresObservationQualificationExecutionInput } from
  './postgres-observation-qualification-runner.js';
import type { QualificationImageIdentity } from
  './postgres-observation-qualification-provenance.js';
import {
  CONTAINER_INSPECT_FORMAT,
  DOCKER_ENVIRONMENT,
  DOCKER_EXECUTABLE,
  IMAGE_INSPECT_FORMAT,
  type QualificationContainerExpectation,
  parseQualificationImageInspection,
  validateDockerHost,
  verifyQualificationContainerInspection,
  verifyQualificationVolumeInspection,
} from './postgres-observation-qualification-docker-inspect.js';
import type {
  PostgresQualificationImage,
  PostgresQualificationProtocol,
} from './postgres-observation-qualification-protocol.js';
import {
  nativeQualificationProcessExecutor,
  type QualificationProcessExecutor,
  type QualificationProcessResult,
} from './postgres-observation-qualification-process.js';

const COMMAND_OUTPUT_LIMIT = 1_048_576;
const EMPTY_SHA256 = 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855';
const TOKEN = /^[a-f0-9]{32}$/;
const CONTAINER_ID = /^[a-f0-9]{64}$/;

export interface PostgresQualificationDockerRun {
  readonly probeStdout: Buffer;
  readonly image: QualificationImageIdentity;
  readonly execution: PostgresObservationQualificationExecutionInput;
}

export interface PostgresQualificationDockerDependencies {
  readonly executor: QualificationProcessExecutor;
  readonly createToken: () => string;
  readonly now: () => number;
  readonly delay: (milliseconds: number) => Promise<void>;
  readonly validateHost: () => void;
}

const NATIVE_DEPENDENCIES: PostgresQualificationDockerDependencies = Object.freeze({
  executor: nativeQualificationProcessExecutor,
  createToken: () => randomBytes(16).toString('hex'),
  now: () => performance.now(),
  delay: (milliseconds: number) => new Promise<void>(
    (resolveDelay) => setTimeout(resolveDelay, milliseconds),
  ),
  validateHost: validateDockerHost,
});

export async function runPostgresQualificationContainer(input: Readonly<{
  repositoryRoot: string;
  protocol: PostgresQualificationProtocol;
  image: PostgresQualificationImage;
  slot: 1 | 2;
  dependencies?: PostgresQualificationDockerDependencies;
}>): Promise<PostgresQualificationDockerRun> {
  const dependencies = input.dependencies ?? NATIVE_DEPENDENCIES;
  dependencies.validateHost();
  const root = canonicalRoot(input.repositoryRoot);
  const fixture = qualificationFile(root,
    'tests/postgresql/observation-qualification-fixture.sql', false);
  const artifact = qualificationFile(root, input.protocol.probe.artifactPath, true);
  const token = dependencies.createToken();
  if (!TOKEN.test(token)) throw new Error('POSTGRES_QUALIFICATION_RESOURCE_TOKEN_INVALID');
  const containerName = `sf-pgq-${token}`;
  const volumeName = `sf-pgq-volume-${token}`;
  const runner = new DockerRunner(root, dependencies.executor);
  const imageBefore = await inspectImage(runner, input.image);
  let volumeCreationAttempted = false;
  let containerCreationAttempted = false;
  let containerId = '';
  let successful: Omit<PostgresQualificationDockerRun['execution'], 'cleanup'> | undefined;
  let probeStdout: Buffer | undefined;
  let failure: unknown;
  try {
    volumeCreationAttempted = true;
    const volume = await runner.checked([
      'volume', 'create', '--label', `${input.protocol.container.ownerLabelKey}=${token}`,
      volumeName,
    ], input.protocol.replay.commandTimeoutMs);
    requireExactLine(volume.stdout, volumeName, 'VOLUME_CREATE');
    const volumeInspection = await runner.checked([
      'volume', 'inspect', '--format', '{"Name":{{json .Name}},"Labels":{{json .Labels}}}',
      volumeName,
    ], input.protocol.replay.commandTimeoutMs);
    verifyQualificationVolumeInspection(
      volumeInspection.stdout, input.protocol, volumeName, token,
    );

    containerCreationAttempted = true;
    const created = await runner.checked(createArguments(
      input.protocol, input.image, containerName, volumeName, fixture, artifact, token,
    ), input.protocol.replay.commandTimeoutMs);
    containerId = exactContainerId(created.stdout);
    await inspectContainer(runner, input.protocol, input.image, {
      name: containerName, id: containerId, token, volumeName,
      fixturePath: fixture, artifactPath: artifact, running: false,
    });
    const started = await runner.checked(
      ['start', containerName], input.protocol.replay.commandTimeoutMs,
    );
    requireExactLine(started.stdout, containerName, 'CONTAINER_START');
    await waitUntilReady(
      runner, input.protocol, containerName, dependencies.now, dependencies.delay,
    );
    await inspectContainer(runner, input.protocol, input.image, {
      name: containerName, id: containerId, token, volumeName,
      fixturePath: fixture, artifactPath: artifact, running: true,
    });
    const probe = await runner.raw([
      'exec', '--user', 'postgres', containerName, input.protocol.container.probePath,
    ], input.protocol.replay.commandTimeoutMs, input.protocol.probe.maxOutputBytes);
    assertSuccessfulProbe(probe);
    probeStdout = Buffer.from(probe.stdout);
    const imageAfter = await inspectImage(runner, input.image);
    if (JSON.stringify(imageAfter) !== JSON.stringify(imageBefore)) {
      throw new Error('POSTGRES_QUALIFICATION_IMAGE_DRIFT');
    }
    successful = {
      slot: input.slot,
      containerIdentitySha256: sha256(Buffer.from(containerId, 'utf8')),
      volumeIdentitySha256: sha256(Buffer.from(volumeName, 'utf8')),
      imageConfigDigestBefore: imageBefore.configDigest,
      imageConfigDigestAfter: imageAfter.configDigest,
      stdout: outputEvidence(probe.stdout),
      stderr: outputEvidence(probe.stderr),
      exitCode: 0,
      timedOut: false,
      outputLimitExceeded: false,
    };
  } catch (error) {
    failure = error;
  }
  let cleanup: PostgresObservationQualificationExecutionInput['cleanup'] | undefined;
  try {
    cleanup = await cleanupOwnedResources({
      runner, protocol: input.protocol, token, containerName, volumeName,
      containerCreationAttempted, volumeCreationAttempted,
      successfulExecution: successful !== undefined,
    });
  } catch (cleanupError) {
    if (failure !== undefined) throw new AggregateError(
      [failure, cleanupError], 'POSTGRES_QUALIFICATION_EXECUTION_AND_CLEANUP_FAILED',
    );
    throw cleanupError;
  }
  if (failure !== undefined) throw failure;
  if (successful === undefined || probeStdout === undefined || cleanup === undefined) {
    throw new Error('POSTGRES_QUALIFICATION_EXECUTION_INCOMPLETE');
  }
  return Object.freeze({
    probeStdout,
    image: imageBefore,
    execution: Object.freeze({ ...successful, cleanup: Object.freeze(cleanup) }),
  });
}

class DockerRunner {
  constructor(
    private readonly root: string,
    private readonly executor: QualificationProcessExecutor,
  ) {}

  raw(args: readonly string[], timeoutMs: number, maxOutputBytes = COMMAND_OUTPUT_LIMIT) {
    return this.executor.run({
      file: DOCKER_EXECUTABLE,
      args,
      cwd: this.root,
      environment: DOCKER_ENVIRONMENT,
      timeoutMs,
      maxOutputBytes,
    });
  }

  async checked(args: readonly string[], timeoutMs: number): Promise<QualificationProcessResult> {
    const result = await this.raw(args, timeoutMs);
    if (result.status !== 0 || result.signal !== null || result.stderr.length !== 0
      || result.timedOut || result.outputLimitExceeded || result.spawnError !== null) {
      throw new Error(`POSTGRES_QUALIFICATION_DOCKER_COMMAND_FAILED:${args[0] ?? 'unknown'}`);
    }
    return result;
  }
}

function createArguments(
  protocol: PostgresQualificationProtocol,
  image: PostgresQualificationImage,
  containerName: string,
  volumeName: string,
  fixture: string,
  artifact: string,
  token: string,
): readonly string[] {
  return [
    'create', '--name', containerName,
    '--label', `${protocol.container.ownerLabelKey}=${token}`,
    '--network', 'none', '--pull', 'never', '--platform', protocol.platform,
    '--read-only',
    '--tmpfs', '/tmp:rw,noexec,nosuid,nodev,size=64m',
    '--tmpfs', `${protocol.container.socketPath}:rw,nosuid,nodev,size=16m`,
    '--env', 'POSTGRES_HOST_AUTH_METHOD=trust',
    '--env', 'POSTGRES_INITDB_ARGS=--locale-provider=libc --locale=C --encoding=UTF8',
    '--mount', `type=volume,src=${volumeName},dst=${protocol.container.pgdataPath}`,
    '--mount', `type=bind,src=${fixture},dst=${protocol.container.fixturePath},readonly`,
    '--mount', `type=bind,src=${artifact},dst=${protocol.container.probePath},readonly`,
    image.reference,
  ];
}

async function inspectImage(
  runner: DockerRunner,
  image: PostgresQualificationImage,
): Promise<QualificationImageIdentity> {
  const result = await runner.checked(
    ['image', 'inspect', '--format', IMAGE_INSPECT_FORMAT, image.reference], 30_000,
  );
  return parseQualificationImageInspection(result.stdout, image);
}

async function inspectContainer(
  runner: DockerRunner,
  protocol: PostgresQualificationProtocol,
  image: PostgresQualificationImage,
  expectation: Omit<QualificationContainerExpectation, 'imageConfigDigest'>,
): Promise<void> {
  const result = await runner.checked([
    'container', 'inspect', '--format', CONTAINER_INSPECT_FORMAT, expectation.name,
  ], protocol.replay.commandTimeoutMs);
  verifyQualificationContainerInspection(result.stdout, protocol, {
    ...expectation,
    imageConfigDigest: image.configDigest,
  });
}

async function waitUntilReady(
  runner: DockerRunner,
  protocol: PostgresQualificationProtocol,
  containerName: string,
  now: () => number,
  delay: (milliseconds: number) => Promise<void>,
): Promise<void> {
  const deadline = now() + protocol.replay.readinessTimeoutMs;
  while (now() <= deadline) {
    const pidTimeout = Math.max(1, Math.min(5_000, deadline - now()));
    const pid = await runner.raw([
      'exec', '--user', 'postgres', containerName, '/bin/cat', '/proc/1/comm',
    ], pidTimeout, 4_096);
    if (isCleanSuccess(pid) && pid.stdout.equals(Buffer.from('postgres\n'))) {
      const readyTimeout = Math.max(1, Math.min(5_000, deadline - now()));
      const ready = await runner.raw([
        'exec', '--user', 'postgres', containerName, '/usr/bin/pg_isready', '--quiet',
        '--host', protocol.container.socketPath, '--port', '5432',
        '--dbname', protocol.database.name, '--username', protocol.roles.observer,
      ], readyTimeout, 4_096);
      if (isCleanSuccess(ready) && ready.stdout.length === 0) return;
    }
    if (now() < deadline) await delay(Math.min(250, deadline - now()));
  }
  throw new Error('POSTGRES_QUALIFICATION_DATABASE_NOT_READY');
}

async function cleanupOwnedResources(input: Readonly<{
  runner: DockerRunner;
  protocol: PostgresQualificationProtocol;
  token: string;
  containerName: string;
  volumeName: string;
  containerCreationAttempted: boolean;
  volumeCreationAttempted: boolean;
  successfulExecution: boolean;
}>): Promise<PostgresObservationQualificationExecutionInput['cleanup']> {
  const errors: unknown[] = [];
  let containerRemoved = false;
  let volumeRemoved = false;
  if (input.containerCreationAttempted) {
    try { containerRemoved = await removeContainerIfOwned(input); }
    catch (error) { errors.push(error); }
  }
  if (input.volumeCreationAttempted) {
    try { volumeRemoved = await removeVolumeIfOwned(input); }
    catch (error) { errors.push(error); }
  }
  for (const args of [
    [
      'ps', '--all', '--filter',
      `label=${input.protocol.container.ownerLabelKey}=${input.token}`, '--format', '{{.ID}}',
    ],
    [
      'volume', 'ls', '--filter',
      `label=${input.protocol.container.ownerLabelKey}=${input.token}`, '--format', '{{.Name}}',
    ],
  ] as const) {
    try {
      const inventory = await input.runner.checked(
        args, input.protocol.replay.cleanupTimeoutMs,
      );
      if (inventory.stdout.length !== 0) {
        errors.push(new Error('POSTGRES_QUALIFICATION_CLEANUP_INCOMPLETE'));
      }
    } catch (error) {
      errors.push(error);
    }
  }
  if (input.successfulExecution && (!containerRemoved || !volumeRemoved)) {
    errors.push(new Error('POSTGRES_QUALIFICATION_CLEANUP_REMOVAL_UNPROVEN'));
  }
  if (errors.length > 0) {
    throw new AggregateError(errors, 'POSTGRES_QUALIFICATION_CLEANUP_FAILED');
  }
  const evidence = Buffer.from(JSON.stringify({
    containerCreationAttempted: input.containerCreationAttempted,
    containerRemoved,
    volumeCreationAttempted: input.volumeCreationAttempted,
    volumeRemoved,
    labelledContainersRemaining: 0,
    labelledVolumesRemaining: 0,
  }), 'utf8');
  return {
    containerRemoved: true,
    volumeRemoved: true,
    labelledContainersRemaining: 0,
    labelledVolumesRemaining: 0,
    verificationSha256: sha256(evidence),
  };
}

async function removeContainerIfOwned(input: Readonly<{
  runner: DockerRunner;
  protocol: PostgresQualificationProtocol;
  token: string;
  containerName: string;
}>): Promise<boolean> {
  const format = `{{index .Config.Labels ${JSON.stringify(input.protocol.container.ownerLabelKey)}}}`;
  const ownership = await input.runner.raw(
    ['container', 'inspect', '--format', format, input.containerName],
    input.protocol.replay.cleanupTimeoutMs, 4_096,
  );
  if (!isCleanSuccess(ownership)) return false;
  requireExactLine(ownership.stdout, input.token, 'CONTAINER_OWNERSHIP');
  const removed = await input.runner.checked(
    ['rm', '--force', input.containerName], input.protocol.replay.cleanupTimeoutMs,
  );
  requireExactLine(removed.stdout, input.containerName, 'CONTAINER_REMOVE');
  return true;
}

async function removeVolumeIfOwned(input: Readonly<{
  runner: DockerRunner;
  protocol: PostgresQualificationProtocol;
  token: string;
  volumeName: string;
}>): Promise<boolean> {
  const format = `{{index .Labels ${JSON.stringify(input.protocol.container.ownerLabelKey)}}}`;
  const ownership = await input.runner.raw(
    ['volume', 'inspect', '--format', format, input.volumeName],
    input.protocol.replay.cleanupTimeoutMs, 4_096,
  );
  if (!isCleanSuccess(ownership)) return false;
  requireExactLine(ownership.stdout, input.token, 'VOLUME_OWNERSHIP');
  const removed = await input.runner.checked(
    ['volume', 'rm', input.volumeName], input.protocol.replay.cleanupTimeoutMs,
  );
  requireExactLine(removed.stdout, input.volumeName, 'VOLUME_REMOVE');
  return true;
}

function assertSuccessfulProbe(result: QualificationProcessResult): void {
  if (!isCleanSuccess(result) || result.stdout.length === 0 || result.stderr.length !== 0) {
    throw new Error('POSTGRES_QUALIFICATION_PROBE_FAILED');
  }
}

function isCleanSuccess(result: QualificationProcessResult): boolean {
  return result.status === 0 && result.signal === null && !result.timedOut
    && !result.outputLimitExceeded && result.spawnError === null && result.stderr.length === 0;
}

function outputEvidence(value: Buffer) {
  const digest = sha256(value);
  if (value.length === 0 && digest !== EMPTY_SHA256) {
    throw new Error('POSTGRES_QUALIFICATION_EMPTY_OUTPUT_DIGEST_INVALID');
  }
  return Object.freeze({ bytes: value.length, sha256: digest, truncated: false as const });
}

function exactContainerId(value: Buffer): string {
  const text = exactLine(value, 'CONTAINER_CREATE');
  if (!CONTAINER_ID.test(text)) throw new Error('POSTGRES_QUALIFICATION_CONTAINER_ID_INVALID');
  return text;
}

function requireExactLine(value: Buffer, expected: string, label: string): void {
  if (exactLine(value, label) !== expected) {
    throw new Error(`POSTGRES_QUALIFICATION_${label}_OUTPUT_INVALID`);
  }
}

function exactLine(value: Buffer, label: string): string {
  const text = value.toString('utf8');
  if (!value.equals(Buffer.from(text, 'utf8')) || text.includes('\r')
    || !text.endsWith('\n') || text.slice(0, -1).includes('\n')) {
    throw new Error(`POSTGRES_QUALIFICATION_${label}_OUTPUT_INVALID`);
  }
  return text.slice(0, -1);
}

function qualificationFile(root: string, path: string, executable: boolean): string {
  const absolute = resolve(root, path);
  const rel = relative(root, absolute);
  if (rel.startsWith('..') || isAbsolute(rel) || realpathSync(absolute) !== absolute) {
    throw new Error('POSTGRES_QUALIFICATION_BIND_PATH_INVALID');
  }
  const stat = lstatSync(absolute);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1 || stat.size < 1
    || (executable && (stat.mode & 0o111) === 0)) {
    throw new Error('POSTGRES_QUALIFICATION_BIND_SOURCE_INVALID');
  }
  return absolute;
}

function canonicalRoot(root: string): string {
  if (!isAbsolute(root) || resolve(root) !== root || realpathSync(root) !== root
    || !lstatSync(root).isDirectory()) {
    throw new Error('POSTGRES_QUALIFICATION_REPOSITORY_ROOT_INVALID');
  }
  return root;
}

function sha256(value: Buffer): string {
  return createHash('sha256').update(value).digest('hex');
}
