// SPDX-License-Identifier: MIT

import { randomBytes } from 'node:crypto';
import {
  closeSync,
  constants,
  existsSync,
  fsyncSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import {
  DOCKER_ENVIRONMENT,
  DOCKER_EXECUTABLE,
  IMAGE_INSPECT_FORMAT,
  parseQualificationBuilderImageInspection,
  validateDockerHost,
} from './postgres-observation-qualification-docker-inspect.js';
import type { QualificationBuilderImageIdentity } from
  './postgres-observation-qualification-provenance.js';
import type { PostgresQualificationProtocol } from
  './postgres-observation-qualification-protocol.js';
import {
  nativeQualificationProcessExecutor,
  type QualificationProcessExecutor,
  type QualificationProcessResult,
} from './postgres-observation-qualification-process.js';

const TOOLCHAIN_BIN = '/usr/local/rustup/toolchains/1.96.0-x86_64-unknown-linux-gnu/bin';
const CARGO = `${TOOLCHAIN_BIN}/cargo`;
const RUSTC = `${TOOLCHAIN_BIN}/rustc`;
const TARGET = 'x86_64-unknown-linux-gnu';
const TOKEN = /^[a-f0-9]{32}$/;
const CONTAINER_ID = /^[a-f0-9]{64}$/;
const COMMAND_OUTPUT_LIMIT = 32 * 1024 * 1024;
const BUILD_TIMEOUT_MS = 15 * 60_000;

export interface PostgresQualificationToolchainEvidence {
  readonly rustcVv: Buffer;
  readonly cargoVersion: Buffer;
  readonly builderImage: QualificationBuilderImageIdentity;
}

export interface PostgresQualificationBuildDependencies {
  readonly executor: QualificationProcessExecutor;
  readonly createToken: () => string;
  readonly createTemporaryDirectory: () => string;
  readonly validateHost: () => void;
  readonly uid: number;
  readonly gid: number;
}

const NATIVE_DEPENDENCIES: PostgresQualificationBuildDependencies = Object.freeze({
  executor: nativeQualificationProcessExecutor,
  createToken: () => randomBytes(16).toString('hex'),
  createTemporaryDirectory: () => mkdtempSync(join(tmpdir(), 'sf-pgq-build-')),
  validateHost: validateDockerHost,
  uid: typeof process.getuid === 'function' ? process.getuid() : -1,
  gid: typeof process.getgid === 'function' ? process.getgid() : -1,
});

export async function buildPostgresQualificationProbe(input: Readonly<{
  repositoryRoot: string;
  protocol: PostgresQualificationProtocol;
  dependencies?: PostgresQualificationBuildDependencies;
}>): Promise<PostgresQualificationToolchainEvidence> {
  const dependencies = input.dependencies ?? NATIVE_DEPENDENCIES;
  dependencies.validateHost();
  const root = canonicalRoot(input.repositoryRoot);
  validateIdentity(dependencies);
  validateMountSource(root);
  const token = dependencies.createToken();
  if (!TOKEN.test(token)) throw new Error('POSTGRES_QUALIFICATION_BUILDER_TOKEN_INVALID');
  const sandbox = canonicalTemporaryDirectory(
    dependencies.createTemporaryDirectory(), dependencies.uid, dependencies.gid,
  );
  const cargoHome = join(sandbox, 'cargo');
  const targetDirectory = join(sandbox, 'target');
  const runner = new DockerRunner(root, dependencies.executor);
  let failure: unknown;
  try {
    mkdirSync(cargoHome, { mode: 0o700 });
    mkdirSync(targetDirectory, { mode: 0o700 });
    const imageBefore = await inspectBuilderImage(runner, input.protocol);
    const context = Object.freeze({
      root, cargoHome, targetDirectory, token, protocol: input.protocol, dependencies, runner,
    });
    const rustcVv = await runBuilderContainer(context, 'rustc', 'none', RUSTC, ['-Vv'], 30_000);
    const cargoVersion = await runBuilderContainer(
      context, 'cargo', 'none', CARGO, ['--version'], 30_000,
    );
    validateEvidence(rustcVv, 'RUSTC');
    validateEvidence(cargoVersion, 'CARGO');
    await runBuilderContainer(context, 'fetch', 'bridge', CARGO, [
      'fetch', '--locked', '--manifest-path', '/workspace/Cargo.toml',
      '--target', TARGET, '--quiet',
    ], BUILD_TIMEOUT_MS, true);
    await runBuilderContainer(context, 'build', 'none', CARGO, [
      'build', '--locked', '--offline', '--release', '--target', TARGET,
      '--manifest-path', '/workspace/Cargo.toml', '-p', input.protocol.probe.package,
      '--features', 'evidence-receipts', '--bin', input.protocol.probe.bin, '--quiet',
    ], BUILD_TIMEOUT_MS, true);
    const imageAfter = await inspectBuilderImage(runner, input.protocol);
    if (JSON.stringify(imageAfter) !== JSON.stringify(imageBefore)) {
      throw new Error('POSTGRES_QUALIFICATION_BUILDER_IMAGE_DRIFT');
    }
    installArtifact(root, targetDirectory, input.protocol.probe.artifactPath);
    return Object.freeze({ rustcVv, cargoVersion, builderImage: imageBefore });
  } catch (error) {
    failure = error;
    throw error;
  } finally {
    try {
      rmSync(sandbox, { recursive: true, force: true });
    } catch (cleanupError) {
      if (failure !== undefined) throw new AggregateError(
        [failure, cleanupError], 'POSTGRES_QUALIFICATION_BUILD_AND_SANDBOX_CLEANUP_FAILED',
      );
      throw cleanupError;
    }
  }
}

interface BuilderContext {
  readonly root: string;
  readonly cargoHome: string;
  readonly targetDirectory: string;
  readonly token: string;
  readonly protocol: PostgresQualificationProtocol;
  readonly dependencies: PostgresQualificationBuildDependencies;
  readonly runner: DockerRunner;
}

async function runBuilderContainer(
  context: BuilderContext,
  stage: 'rustc' | 'cargo' | 'fetch' | 'build',
  network: 'none' | 'bridge',
  executable: string,
  args: readonly string[],
  timeoutMs: number,
  requireEmptyOutput = false,
): Promise<Buffer> {
  const name = `sf-pgq-builder-${stage}-${context.token}`;
  let creationAttempted = false;
  let successful = false;
  let output: Buffer | undefined;
  let failure: unknown;
  try {
    creationAttempted = true;
    const created = await context.runner.checked(createArguments(
      context, name, network, executable, args,
    ), context.protocol.replay.commandTimeoutMs);
    exactContainerId(created.stdout);
    const started = await context.runner.checked(
      ['start', name], context.protocol.replay.commandTimeoutMs,
    );
    requireExactLine(started.stdout, name, 'BUILDER_START');
    const waited = await context.runner.checked(['wait', name], timeoutMs);
    requireExactLine(waited.stdout, '0', 'BUILDER_WAIT');
    const logs = await context.runner.raw(['logs', name], 30_000, COMMAND_OUTPUT_LIMIT);
    if (!isProcessSuccess(logs) || logs.stderr.length !== 0
      || (requireEmptyOutput && logs.stdout.length !== 0)) {
      throw new Error(`POSTGRES_QUALIFICATION_BUILDER_${stage.toUpperCase()}_FAILED`);
    }
    output = Buffer.from(logs.stdout);
    successful = true;
  } catch (error) {
    failure = error;
  }
  try {
    await cleanupBuilderContainer(context, name, creationAttempted, successful);
  } catch (cleanupError) {
    if (failure !== undefined) throw new AggregateError(
      [failure, cleanupError], 'POSTGRES_QUALIFICATION_BUILDER_AND_CLEANUP_FAILED',
    );
    throw cleanupError;
  }
  if (failure !== undefined) throw failure;
  if (output === undefined) throw new Error('POSTGRES_QUALIFICATION_BUILDER_OUTPUT_MISSING');
  return output;
}

function createArguments(
  context: BuilderContext,
  name: string,
  network: 'none' | 'bridge',
  executable: string,
  args: readonly string[],
): readonly string[] {
  const environment = builderEnvironment(network);
  return [
    'create', '--name', name,
    '--label', `${context.protocol.container.ownerLabelKey}=${context.token}`,
    '--network', network, '--pull', 'never', '--platform', context.protocol.builder.platform,
    '--read-only', '--user', `${context.dependencies.uid}:${context.dependencies.gid}`,
    '--workdir', '/workspace', '--cap-drop', 'ALL',
    '--security-opt', 'no-new-privileges', '--pids-limit', '512',
    '--tmpfs', '/tmp:rw,nosuid,nodev,size=256m',
    ...Object.entries(environment).flatMap(([key, value]) => ['--env', `${key}=${value}`]),
    '--mount', `type=bind,src=${context.root},dst=/workspace,readonly`,
    '--mount', `type=bind,src=${context.cargoHome},dst=/cargo`,
    '--mount', `type=bind,src=${context.targetDirectory},dst=/target`,
    context.protocol.builder.reference, executable, ...args,
  ];
}

function builderEnvironment(network: 'none' | 'bridge'): Readonly<Record<string, string>> {
  return Object.freeze({
    PATH: `${TOOLCHAIN_BIN}:/usr/bin:/bin`,
    HOME: '/nonexistent',
    CARGO_HOME: '/cargo',
    CARGO_TARGET_DIR: '/target',
    RUSTUP_HOME: '/usr/local/rustup',
    RUSTC,
    CARGO_ENCODED_RUSTFLAGS: [
      '--remap-path-prefix=/workspace=/workspace',
      '--remap-path-prefix=/cargo=/cargo',
      '--remap-path-prefix=/usr/local/rustup=/rustup',
    ].join('\u001f'),
    CARGO_INCREMENTAL: '0',
    CARGO_NET_OFFLINE: network === 'none' ? 'true' : 'false',
    CARGO_NET_GIT_FETCH_WITH_CLI: 'false',
    CARGO_TERM_COLOR: 'never',
    GIT_CONFIG_GLOBAL: '/dev/null',
    GIT_CONFIG_NOSYSTEM: '1',
    SOURCE_DATE_EPOCH: '0',
    LANG: 'C',
    LC_ALL: 'C',
    TZ: 'UTC',
  });
}

async function cleanupBuilderContainer(
  context: BuilderContext,
  name: string,
  creationAttempted: boolean,
  successful: boolean,
): Promise<void> {
  let removed = false;
  const errors: unknown[] = [];
  if (creationAttempted) {
    try {
      const format = `{{index .Config.Labels ${JSON.stringify(
        context.protocol.container.ownerLabelKey,
      )}}}`;
      const ownership = await context.runner.raw(
        ['container', 'inspect', '--format', format, name],
        context.protocol.replay.cleanupTimeoutMs, 4_096,
      );
      if (isProcessSuccess(ownership) && ownership.stderr.length === 0) {
        requireExactLine(ownership.stdout, context.token, 'BUILDER_OWNERSHIP');
        const result = await context.runner.checked(
          ['rm', '--force', name], context.protocol.replay.cleanupTimeoutMs,
        );
        requireExactLine(result.stdout, name, 'BUILDER_REMOVE');
        removed = true;
      }
    } catch (error) { errors.push(error); }
  }
  try {
    const inventory = await context.runner.checked([
      'ps', '--all', '--filter',
      `label=${context.protocol.container.ownerLabelKey}=${context.token}`,
      '--format', '{{.ID}}',
    ], context.protocol.replay.cleanupTimeoutMs);
    if (inventory.stdout.length !== 0) {
      errors.push(new Error('POSTGRES_QUALIFICATION_BUILDER_CLEANUP_INCOMPLETE'));
    }
  } catch (error) { errors.push(error); }
  if (successful && !removed) {
    errors.push(new Error('POSTGRES_QUALIFICATION_BUILDER_REMOVAL_UNPROVEN'));
  }
  if (errors.length > 0) {
    throw new AggregateError(errors, 'POSTGRES_QUALIFICATION_BUILDER_CLEANUP_FAILED');
  }
}

async function inspectBuilderImage(
  runner: DockerRunner,
  protocol: PostgresQualificationProtocol,
): Promise<QualificationBuilderImageIdentity> {
  const result = await runner.checked([
    'image', 'inspect', '--format', IMAGE_INSPECT_FORMAT, protocol.builder.reference,
  ], 30_000);
  return parseQualificationBuilderImageInspection(result.stdout, protocol.builder);
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
    if (!isProcessSuccess(result) || result.stderr.length !== 0) {
      throw new Error(`POSTGRES_QUALIFICATION_DOCKER_COMMAND_FAILED:${args[0] ?? 'unknown'}`);
    }
    return result;
  }
}

function installArtifact(root: string, targetDirectory: string, artifactPath: string): void {
  const source = resolve(targetDirectory, TARGET, 'release', 'postgres-observation-qualification');
  const sourceRelative = relative(targetDirectory, source);
  const sourceStat = lstatSync(source);
  if (sourceRelative.startsWith('..') || isAbsolute(sourceRelative)
    || !sourceStat.isFile() || sourceStat.isSymbolicLink()
    || realpathSync(source) !== source || sourceStat.size < 1
    || sourceStat.size > 128 * 1024 * 1024 || (sourceStat.mode & 0o111) === 0) {
    throw new Error('POSTGRES_QUALIFICATION_BUILD_ARTIFACT_INVALID');
  }
  const bytes = readFileSync(source);
  const sourceAfter = lstatSync(source);
  if (bytes.length !== sourceStat.size || sourceAfter.dev !== sourceStat.dev
    || sourceAfter.ino !== sourceStat.ino || sourceAfter.size !== sourceStat.size
    || sourceAfter.mtimeMs !== sourceStat.mtimeMs || sourceAfter.ctimeMs !== sourceStat.ctimeMs
    || sourceAfter.mode !== sourceStat.mode) {
    throw new Error('POSTGRES_QUALIFICATION_BUILD_ARTIFACT_CHANGED');
  }
  const destination = resolve(root, artifactPath);
  const destinationRelative = relative(root, destination);
  if (!destinationRelative || destinationRelative.startsWith('..')
    || isAbsolute(destinationRelative)) {
    throw new Error('POSTGRES_QUALIFICATION_BUILD_ARTIFACT_PATH_INVALID');
  }
  const parent = ensureDirectoryTree(root, dirname(destinationRelative));
  const temporary = join(parent, `.sf-pgq-artifact-${randomBytes(16).toString('hex')}.tmp`);
  let descriptor: number | undefined;
  try {
    descriptor = openSync(
      temporary,
      constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW,
      0o755,
    );
    writeFileSync(descriptor, bytes);
    fsyncSync(descriptor);
    closeSync(descriptor);
    descriptor = undefined;
    renameSync(temporary, destination);
    const installed = lstatSync(destination);
    if (!installed.isFile() || installed.isSymbolicLink() || installed.nlink !== 1
      || (installed.mode & 0o111) === 0 || installed.size !== bytes.length) {
      throw new Error('POSTGRES_QUALIFICATION_INSTALLED_ARTIFACT_INVALID');
    }
    const parentDescriptor = openSync(parent, constants.O_RDONLY | constants.O_DIRECTORY);
    try { fsyncSync(parentDescriptor); } finally { closeSync(parentDescriptor); }
  } finally {
    if (descriptor !== undefined) closeSync(descriptor);
    rmSync(temporary, { force: true });
  }
}

function ensureDirectoryTree(root: string, relativeDirectory: string): string {
  let current = root;
  for (const segment of relativeDirectory.split(/[\\/]/u).filter(Boolean)) {
    current = join(current, segment);
    if (!existsSync(current)) mkdirSync(current, { mode: 0o755 });
    const stat = lstatSync(current);
    if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(current) !== current) {
      throw new Error('POSTGRES_QUALIFICATION_BUILD_ARTIFACT_DIRECTORY_INVALID');
    }
  }
  return current;
}

function validateEvidence(value: Buffer, label: string): void {
  const text = value.toString('utf8');
  if (value.length < 2 || value.length > 64 * 1024 || value.at(-1) !== 0x0a
    || !value.equals(Buffer.from(text, 'utf8')) || text.includes('\r')
    || text.includes('\0')) {
    throw new Error(`POSTGRES_QUALIFICATION_${label}_OUTPUT_INVALID`);
  }
}

function exactContainerId(value: Buffer): string {
  const id = exactLine(value, 'BUILDER_CREATE');
  if (!CONTAINER_ID.test(id)) throw new Error('POSTGRES_QUALIFICATION_BUILDER_ID_INVALID');
  return id;
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

function isProcessSuccess(result: QualificationProcessResult): boolean {
  return result.status === 0 && result.signal === null && !result.timedOut
    && !result.outputLimitExceeded && result.spawnError === null;
}

function validateIdentity(dependencies: PostgresQualificationBuildDependencies): void {
  if (!Number.isSafeInteger(dependencies.uid) || dependencies.uid < 1
    || !Number.isSafeInteger(dependencies.gid) || dependencies.gid < 1) {
    throw new Error('POSTGRES_QUALIFICATION_BUILDER_USER_INVALID');
  }
}

function validateMountSource(path: string): void {
  if (path.includes(',') || path.includes('\n') || path.includes('\0')) {
    throw new Error('POSTGRES_QUALIFICATION_BUILDER_MOUNT_PATH_INVALID');
  }
}

function canonicalTemporaryDirectory(path: string, uid: number, gid: number): string {
  const temporaryRoot = realpathSync(tmpdir());
  const relativePath = relative(temporaryRoot, path);
  const stat = lstatSync(path);
  if (!isAbsolute(path) || resolve(path) !== path || realpathSync(path) !== path
    || !stat.isDirectory() || stat.isSymbolicLink() || stat.uid !== uid || stat.gid !== gid
    || (stat.mode & 0o077) !== 0 || relativePath.includes(sep)
    || !/^sf-pgq-build-[A-Za-z0-9_-]{6,}$/u.test(relativePath)) {
    throw new Error('POSTGRES_QUALIFICATION_BUILD_SANDBOX_INVALID');
  }
  validateMountSource(path);
  return path;
}

function canonicalRoot(root: string): string {
  if (!isAbsolute(root) || resolve(root) !== root || realpathSync(root) !== root
    || !lstatSync(root).isDirectory()) {
    throw new Error('POSTGRES_QUALIFICATION_REPOSITORY_ROOT_INVALID');
  }
  return root;
}
