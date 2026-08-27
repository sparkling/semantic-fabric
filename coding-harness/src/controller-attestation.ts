// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  closeSync,
  constants,
  fstatSync,
  lstatSync,
  openSync,
  readSync,
  realpathSync,
} from 'node:fs';
import { isAbsolute, join, relative, resolve, sep } from 'node:path';
import { parseAcceptanceTask, type AcceptanceTask } from './acceptance-task.js';
import { SECURE_HARNESS_CONFIG } from './config.js';
import {
  CONTROLLER_BUILD_PATH,
  parseControllerBuildManifest,
  type ControllerBuildManifest,
} from './controller-build.js';
import { deepFreeze } from './contracts.js';
import { ISSUE_8_FROZEN_LOCK_FIXTURE_PATH } from './frozen-cargo-lock-fixture.js';
import { runGitCommand } from './git-process.js';
import {
  normalizeAcceptanceTaskPath,
  parseHarnessManifest,
  selectAcceptanceTaskPath,
  type HarnessManifest,
} from './manifest.js';
import { digestValue, type GitIdentity } from './receipts.js';

export const ISSUE_8_ACCEPTANCE_TASK_PATH =
  'coding-harness/config/issue-8-acceptance.json';
export const PROGRAMME_V5_ACCEPTANCE_TASK_PATH =
  'coding-harness/config/programme-v5-acceptance.json';
export const HARNESS_MANIFEST_PATH = 'coding-harness/.harness/manifest.json';
export const PROGRAMME_V5_CONTROLLER_REQUIRED_PATHS = Object.freeze([
  ISSUE_8_FROZEN_LOCK_FIXTURE_PATH,
  'coding-harness/scripts/launch-programme-v5.mjs',
  'coding-harness/scripts/programme-v5-operator-support.mjs',
  'coding-harness/scripts/run-programme-v5.mjs',
  'coding-harness/src/immutable-private-runtime.ts',
  'coding-harness/src/programme-v5-driver-support.ts',
  'coding-harness/src/programme-v5-driver.ts',
  'coding-harness/src/programme-v5-policy-anchor.ts',
  'coding-harness/src/programme-v5-program-runtime.ts',
  'coding-harness/src/programme-v5-program.ts',
  'coding-harness/src/programme-v5-qe.ts',
  'coding-harness/src/programme-v5-receipt-io.ts',
  'coding-harness/src/programme-v5-replay.ts',
  'coding-harness/src/programme-v5-ruflo-contract.ts',
  'coding-harness/src/programme-v5-ruflo-runtime.ts',
  'coding-harness/src/programme-v5-ruflo.ts',
  'coding-harness/src/programme-v5-system.ts',
] as const);
const LOCKFILE_PATH = 'coding-harness/package-lock.json';
const MAX_CONTROLLER_BLOB_BYTES = 10_000_000;
const MAX_RUNTIME_FILE_BYTES = 50_000_000;

export interface ControllerAttestation {
  readonly identity: GitIdentity;
  readonly manifestPath: typeof HARNESS_MANIFEST_PATH;
  readonly manifestBlob: string;
  readonly taskPath: string;
  readonly taskBlob: string;
  readonly buildManifestPath: typeof CONTROLLER_BUILD_PATH;
  readonly buildManifestBlob: string;
  readonly task: AcceptanceTask;
  readonly manifest: HarnessManifest;
  readonly build: ControllerBuildManifest;
  readonly taskBlobDigest: string;
  readonly manifestBlobDigest: string;
  readonly buildManifestBlobDigest: string;
  readonly executionDigest: string;
}

export async function attestController(input: Readonly<{
  repositoryRoot: string;
  controllerRepositoryRoot: string;
  controllerCommit: string;
  taskPath: string;
  signal?: AbortSignal;
}>): Promise<ControllerAttestation> {
  const repositoryRoot = canonicalDirectory(input.repositoryRoot);
  const controllerRepositoryRoot = canonicalDirectory(input.controllerRepositoryRoot);
  const requestedTaskPath = normalizeAcceptanceTaskPath(input.taskPath);
  const commit = await gitValue(
    controllerRepositoryRoot,
    ['rev-parse', '--verify', `${input.controllerCommit}^{commit}`],
    input.signal,
  );
  if (commit !== input.controllerCommit) {
    throw new Error('HARNESS_CONTROLLER_COMMIT_IDENTITY_MISMATCH');
  }
  const head = await gitValue(controllerRepositoryRoot, ['rev-parse', '--verify', 'HEAD'], input.signal);
  if (head !== commit) throw new Error('HARNESS_CONTROLLER_NOT_CURRENT_HEAD');
  const tree = await gitValue(controllerRepositoryRoot, ['rev-parse', `${commit}^{tree}`], input.signal);
  const manifestBlob = await readControllerBlob(
    controllerRepositoryRoot,
    commit,
    HARNESS_MANIFEST_PATH,
    input.signal,
  );
  const manifest = parseHarnessManifest(
    parseJson(manifestBlob.value, HARNESS_MANIFEST_PATH),
    SECURE_HARNESS_CONFIG,
  );
  const taskPath = selectAcceptanceTaskPath(manifest, requestedTaskPath);
  const taskBlob = await readControllerBlob(
    controllerRepositoryRoot,
    commit,
    taskPath,
    input.signal,
  );
  const buildBlob = await readControllerBlob(
    controllerRepositoryRoot,
    commit,
    CONTROLLER_BUILD_PATH,
    input.signal,
  );
  const lockfileBlob = await readControllerBlob(
    controllerRepositoryRoot,
    commit,
    LOCKFILE_PATH,
    input.signal,
  );
  const task = parseAcceptanceTask(parseJson(taskBlob.value, taskPath), SECURE_HARNESS_CONFIG);
  const build = parseControllerBuildManifest(
    parseJson(buildBlob.value, CONTROLLER_BUILD_PATH),
  );
  if (build.harnessManifestDigest !== manifestBlob.digest
    || build.lockfileDigest !== lockfileBlob.digest) {
    throw new Error('HARNESS_CONTROLLER_BUILD_INPUT_DIGEST_MISMATCH');
  }
  const executionPaths = controllerExecutionPaths(manifest.protectedPaths, taskPath);
  const sources: Record<string, string> = {};
  const outputs: Record<string, string> = {};
  for (const path of executionPaths) {
    const blob = await readControllerBlob(controllerRepositoryRoot, commit, path, input.signal);
    const absolute = trustedFile(repositoryRoot, path, 'CONTROLLER_SOURCE');
    const currentDigest = sha256FileBounded(absolute);
    if (currentDigest !== blob.digest) {
      throw new Error(`HARNESS_CONTROLLER_SOURCE_MISMATCH:${path}`);
    }
    sources[path] = blob.digest;
    if (path.startsWith('coding-harness/src/') && path.endsWith('.ts')) {
      const outputPath = path
        .replace('coding-harness/src/', 'coding-harness/dist/')
        .replace(/\.ts$/, '.js');
      outputs[outputPath] = build.outputs[outputPath] ?? '';
    }
  }
  const expectedOutputs = Object.keys(outputs).sort();
  if (JSON.stringify(expectedOutputs) !== JSON.stringify(Object.keys(build.outputs))) {
    throw new Error('HARNESS_CONTROLLER_BUILD_OUTPUT_SET_MISMATCH');
  }
  for (const [path, expected] of Object.entries(build.outputs)) {
    if (sha256FileBounded(trustedFile(repositoryRoot, path, 'BUILD_OUTPUT')) !== expected) {
      throw new Error(`HARNESS_CONTROLLER_BUILD_OUTPUT_MISMATCH:${path}`);
    }
  }
  const productionFiles: Record<string, string> = {};
  for (const [path, expected] of Object.entries(build.productionFiles)) {
    if (sha256FileBounded(trustedFile(repositoryRoot, path, 'PRODUCTION_FILE')) !== expected) {
      throw new Error(`HARNESS_CONTROLLER_PRODUCTION_FILE_MISMATCH:${path}`);
    }
    productionFiles[path] = expected;
  }
  return deepFreeze({
    identity: { commit, tree },
    manifestPath: HARNESS_MANIFEST_PATH,
    manifestBlob: manifestBlob.value,
    taskPath,
    taskBlob: taskBlob.value,
    buildManifestPath: CONTROLLER_BUILD_PATH,
    buildManifestBlob: buildBlob.value,
    task,
    manifest,
    build,
    taskBlobDigest: taskBlob.digest,
    manifestBlobDigest: manifestBlob.digest,
    buildManifestBlobDigest: buildBlob.digest,
    executionDigest: digestValue({
      controller: { commit, tree }, sources, outputs, productionFiles,
      runtimeTreeDigest: build.runtimeTreeDigest,
    }),
  });
}

export async function attestIssue8Controller(input: Readonly<{
  repositoryRoot: string;
  controllerRepositoryRoot: string;
  controllerCommit: string;
  signal?: AbortSignal;
}>): Promise<ControllerAttestation> {
  return await attestController({
    ...input,
    taskPath: ISSUE_8_ACCEPTANCE_TASK_PATH,
  });
}

export function controllerExecutionPaths(paths: readonly string[], taskPath: string): string[] {
  const selected = paths.filter((path) =>
    path.startsWith('coding-harness/src/')
    || path.startsWith('coding-harness/scripts/')
    || path === 'coding-harness/package.json'
    || path === 'coding-harness/package-lock.json'
    || path === 'coding-harness/tsconfig.json'
    || path === ISSUE_8_FROZEN_LOCK_FIXTURE_PATH
    || path === taskPath
    || path === HARNESS_MANIFEST_PATH
    || path === CONTROLLER_BUILD_PATH);
  const required = taskPath === PROGRAMME_V5_ACCEPTANCE_TASK_PATH
    ? PROGRAMME_V5_CONTROLLER_REQUIRED_PATHS
    : [
        'coding-harness/scripts/launch-issue-8.mjs',
        'coding-harness/src/issue-8-driver.ts',
      ];
  if (!selected.includes(taskPath) || !selected.includes(HARNESS_MANIFEST_PATH)
    || !selected.includes('coding-harness/src/controller-attestation.ts')
    || !selected.includes(CONTROLLER_BUILD_PATH)
    || required.some((path) => !selected.includes(path))) {
    throw new Error('HARNESS_CONTROLLER_EXECUTION_MANIFEST_INCOMPLETE');
  }
  return [...selected].sort();
}

async function readControllerBlob(
  root: string,
  commit: string,
  path: string,
  signal?: AbortSignal,
): Promise<Readonly<{ value: string; digest: string }>> {
  const object = `${commit}:${path}`;
  const type = await gitValue(root, ['cat-file', '-t', object], signal, 128);
  if (type !== 'blob') throw new Error(`HARNESS_CONTROLLER_BLOB_INVALID:${path}`);
  const size = Number(await gitValue(root, ['cat-file', '-s', object], signal, 128));
  if (!Number.isSafeInteger(size) || size < 1 || size > MAX_CONTROLLER_BLOB_BYTES) {
    throw new Error(`HARNESS_CONTROLLER_BLOB_SIZE_INVALID:${path}`);
  }
  const result = await runGitCommand(root, ['show', object], {
    signal,
    maxOutputBytes: MAX_CONTROLLER_BLOB_BYTES + 1,
  });
  if (result.exitCode !== 0 || Buffer.byteLength(result.stdout, 'utf8') !== size) {
    throw new Error(`HARNESS_CONTROLLER_BLOB_READ_FAILED:${path}`);
  }
  return Object.freeze({ value: result.stdout, digest: sha256(result.stdout) });
}

async function gitValue(
  root: string,
  args: readonly string[],
  signal?: AbortSignal,
  maxOutputBytes = 4096,
): Promise<string> {
  const result = await runGitCommand(root, args, { signal, maxOutputBytes });
  if (result.exitCode !== 0) throw new Error(`HARNESS_CONTROLLER_GIT_FAILED:${args[0]}`);
  return result.stdout.trim();
}

function trustedFile(root: string, path: string, label: string): string {
  const absolute = resolve(root, path);
  const delta = relative(root, absolute);
  if (delta === '' || delta === '..' || delta.startsWith(`..${sep}`) || isAbsolute(delta)) {
    throw new Error(`HARNESS_${label}_PATH_INVALID`);
  }
  const stat = lstatSync(absolute);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1
    || realpathSync(absolute) !== absolute) {
    throw new Error(`HARNESS_${label}_INVALID:${path}`);
  }
  return absolute;
}

function canonicalDirectory(value: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) {
    throw new TypeError('HARNESS_CONTROLLER_REPOSITORY_INVALID');
  }
  const stat = lstatSync(value);
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(value) !== value) {
    throw new Error('HARNESS_CONTROLLER_REPOSITORY_INVALID');
  }
  return value;
}

function parseJson(value: string, path: string): unknown {
  try {
    return JSON.parse(value);
  } catch {
    throw new Error(`HARNESS_CONTROLLER_JSON_INVALID:${path}`);
  }
}

function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}

function sha256FileBounded(path: string): string {
  const pathBefore = lstatSync(path, { bigint: true });
  if (!pathBefore.isFile() || pathBefore.isSymbolicLink() || pathBefore.nlink !== 1n
    || realpathSync(path) !== path) {
    throw new Error('HARNESS_CONTROLLER_RUNTIME_FILE_UNTRUSTED');
  }
  const descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const before = fstatSync(descriptor, { bigint: true });
    if (!sameRuntimeFile(pathBefore, before)) {
      throw new Error('HARNESS_CONTROLLER_RUNTIME_FILE_CHANGED');
    }
    if (!before.isFile() || before.size < 1n || before.size > BigInt(MAX_RUNTIME_FILE_BYTES)) {
      throw new Error('HARNESS_CONTROLLER_RUNTIME_FILE_SIZE_INVALID');
    }
    const hash = createHash('sha256');
    const buffer = Buffer.allocUnsafe(Math.min(Number(before.size), 1024 * 1024));
    let offset = 0n;
    while (offset < before.size) {
      const length = Math.min(buffer.length, Number(before.size - offset));
      const count = readSync(descriptor, buffer, 0, length, Number(offset));
      if (count === 0) break;
      hash.update(buffer.subarray(0, count));
      offset += BigInt(count);
    }
    const after = fstatSync(descriptor, { bigint: true });
    const pathAfter = lstatSync(path, { bigint: true });
    if (offset !== before.size || !sameRuntimeFile(before, after)
      || !sameRuntimeFile(after, pathAfter) || realpathSync(path) !== path) {
      throw new Error('HARNESS_CONTROLLER_RUNTIME_FILE_CHANGED');
    }
    return hash.digest('hex');
  } finally {
    closeSync(descriptor);
  }
}

function sameRuntimeFile(
  left: Readonly<{
    dev: bigint; ino: bigint; mode: bigint; nlink: bigint; size: bigint;
    mtimeNs: bigint; ctimeNs: bigint;
  }>,
  right: Readonly<{
    dev: bigint; ino: bigint; mode: bigint; nlink: bigint; size: bigint;
    mtimeNs: bigint; ctimeNs: bigint;
  }>,
): boolean {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.nlink === right.nlink && left.size === right.size
    && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}
