// SPDX-License-Identifier: MIT

import {
  SHA256_PATTERN,
  asClosedRecord,
  asDenseArray,
  asInteger,
  assertExactKeys,
  deepFreeze,
  normalizeWorkspacePath,
} from './contracts.js';
import { SECURE_HARNESS_CONFIG } from './config.js';
import { assertGitMaterializationSafe } from './git-materialization.js';
import { runGitCommandBytes } from './git-process.js';
import {
  normalizeAcceptanceTaskPath,
  parseHarnessManifest,
  selectAcceptanceTaskPath,
  type HarnessManifest,
} from './manifest.js';
import { PROGRAMME_CAPTURE_HARNESS_CONFIG_V1 } from './programme-capture-config-v1.js';
import {
  assertCaptureControllerStoreStableV1,
  openCaptureControllerStoreV1,
  readCaptureCommitBlobsV1,
  readCaptureCommitTreeV1,
  type CaptureBlobIdentityV1,
} from './programme-capture-git-v1.js';
import {
  PROGRAMME_CAPTURE_OUTPUT_PATH,
  PROGRAMME_CAPTURE_PROFILE_PATH,
  PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS,
  PROGRAMME_CAPTURE_SCENARIOS_PATH,
  parseProgrammeCaptureTaskBlobV1,
  type ProgrammeCaptureTaskV1,
} from './programme-capture-task-v1.js';
import { digestValue, type GitIdentity } from './receipts.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

const MANIFEST_PATH = 'coding-harness/.harness/manifest.json' as const;
const GIT_OBJECT = /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/;
const MAX_JSON_BLOB_BYTES = 1_048_576;
const MAX_INPUT_BLOB_BYTES = 10_000_000, MAX_INPUT_TOTAL_BYTES = 100_000_000;
const PROTECTED_INPUT_PATHS = Object.freeze([
  PROGRAMME_CAPTURE_PROFILE_PATH, PROGRAMME_CAPTURE_SCENARIOS_PATH, 'Cargo.lock',
  ...PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS,
] as const);

export type { CaptureBlobIdentityV1 } from './programme-capture-git-v1.js';

export interface ProgrammeCaptureInputAttestationV1 {
  readonly schemaVersion: 1;
  readonly transactionKind: 'programme-capture-v1';
  readonly controller: GitIdentity;
  readonly manifest: CaptureBlobIdentityV1;
  readonly task: CaptureBlobIdentityV1 & Readonly<{ valueDigest: string }>;
  readonly protectedInputs: readonly CaptureBlobIdentityV1[];
  readonly output: Readonly<{
    path: typeof PROGRAMME_CAPTURE_OUTPUT_PATH;
    absentFromCommit: true;
  }>;
  readonly protectedInputsDigest: string;
  readonly attestationDigest: string;
}

export interface AttestedProgrammeCaptureInputsV1 {
  readonly manifest: HarnessManifest;
  readonly task: ProgrammeCaptureTaskV1;
  readonly taskBlob: string;
  readonly record: ProgrammeCaptureInputAttestationV1;
}

export async function attestProgrammeCaptureInputsV1(input: Readonly<{
  controllerStore: string;
  controllerCommit: string;
  taskPath: string;
  signal?: AbortSignal;
}>): Promise<AttestedProgrammeCaptureInputsV1> {
  const store = await openCaptureControllerStoreV1(input.controllerStore, input.signal);
  const controllerCommit = parseGitObject(input.controllerCommit, 'controller commit');
  const taskPath = normalizeAcceptanceTaskPath(input.taskPath);
  const commit = await resolveExactCommit(store.path, controllerCommit, input.signal);
  const tree = await gitValue(store.path, ['rev-parse', `${commit}^{tree}`], input.signal);
  if (!GIT_OBJECT.test(tree) || tree.length !== commit.length) {
    throw new Error('HARNESS_CAPTURE_CONTROLLER_TREE_INVALID');
  }
  await assertGitMaterializationSafe({
    repositoryRoot: store.path,
    commits: [commit],
    signal: input.signal,
  });
  const treeSnapshot = await readCaptureCommitTreeV1(
    store.path, commit, tree, store.objectFormat, input.signal,
  );
  const [manifestBlob] = await readCaptureCommitBlobsV1(
    store.path,
    treeSnapshot,
    [MANIFEST_PATH],
    MAX_JSON_BLOB_BYTES,
    MAX_JSON_BLOB_BYTES,
    store.objectFormat,
    input.signal,
  );
  const manifestText = decodeUtf8(manifestBlob.bytes, 'manifest');
  const manifest = parseHarnessManifest(
    parseJsonWithoutDuplicateKeys(manifestText, 'programme capture manifest'),
    SECURE_HARNESS_CONFIG,
  );
  const selectedTaskPath = selectAcceptanceTaskPath(manifest, taskPath);
  const [taskBlob] = await readCaptureCommitBlobsV1(
    store.path, treeSnapshot, [selectedTaskPath], MAX_JSON_BLOB_BYTES,
    MAX_JSON_BLOB_BYTES, store.objectFormat, input.signal,
  );
  const taskText = decodeUtf8(taskBlob.bytes, 'task');
  const task = parseProgrammeCaptureTaskBlobV1(
    taskText,
    PROGRAMME_CAPTURE_HARNESS_CONFIG_V1,
  );
  const bindings = taskBindings(task);
  const inputBlobs = await readCaptureCommitBlobsV1(
    store.path,
    treeSnapshot,
    bindings.map(({ path }) => path),
    MAX_INPUT_BLOB_BYTES,
    MAX_INPUT_TOTAL_BYTES,
    store.objectFormat,
    input.signal,
  );
  const protectedInputs = inputBlobs.map(({ identity }, index) => {
    if (identity.sha256 !== bindings[index].sha256) {
      throw new Error(
        `HARNESS_CAPTURE_PROTECTED_INPUT_DIGEST_MISMATCH:${bindings[index].path}`,
      );
    }
    return identity;
  });
  if (treeSnapshot.entries.has(PROGRAMME_CAPTURE_OUTPUT_PATH)) {
    throw new Error('HARNESS_CAPTURE_OUTPUT_PRESENT_AT_COMMIT');
  }
  await assertGitMaterializationSafe({
    repositoryRoot: store.path,
    commits: [commit],
    signal: input.signal,
  });
  await assertCaptureControllerStoreStableV1(store, input.signal);
  if (await resolveExactCommit(store.path, controllerCommit, input.signal) !== commit
    || await gitValue(store.path, ['rev-parse', `${commit}^{tree}`], input.signal) !== tree) {
    throw new Error('HARNESS_CAPTURE_CONTROLLER_IDENTITY_CHANGED');
  }
  const finalTreeSnapshot = await readCaptureCommitTreeV1(
    store.path, commit, tree, store.objectFormat, input.signal,
  );
  if (finalTreeSnapshot.listingDigest !== treeSnapshot.listingDigest
    || finalTreeSnapshot.entries.has(PROGRAMME_CAPTURE_OUTPUT_PATH)) {
    throw new Error('HARNESS_CAPTURE_CONTROLLER_TREE_CHANGED');
  }

  const protectedInputsDigest = digestValue(protectedInputs);
  const body = {
    schemaVersion: 1 as const,
    transactionKind: 'programme-capture-v1' as const,
    controller: { commit, tree },
    manifest: manifestBlob.identity,
    task: { ...taskBlob.identity, valueDigest: digestValue(task) },
    protectedInputs,
    output: { path: PROGRAMME_CAPTURE_OUTPUT_PATH, absentFromCommit: true as const },
    protectedInputsDigest,
  };
  const record = parseProgrammeCaptureInputAttestationV1({
    ...body,
    attestationDigest: digestValue(body),
  });
  return deepFreeze({ manifest, task, taskBlob: taskText, record });
}

export function parseProgrammeCaptureInputAttestationV1(
  value: unknown,
): ProgrammeCaptureInputAttestationV1 {
  const input = asClosedRecord(value, 'programme capture input attestation');
  assertExactKeys(input, [
    'schemaVersion', 'transactionKind', 'controller', 'manifest', 'task',
    'protectedInputs', 'output', 'protectedInputsDigest', 'attestationDigest',
  ], 'programme capture input attestation');
  if (input.schemaVersion !== 1 || input.transactionKind !== 'programme-capture-v1') {
    throw new TypeError('programme capture input attestation identity is invalid');
  }
  const controllerInput = asClosedRecord(
    input.controller, 'programme capture input attestation.controller',
  );
  assertExactKeys(
    controllerInput, ['commit', 'tree'], 'programme capture input attestation.controller',
  );
  const commit = parseGitObject(controllerInput.commit, 'attestation controller commit');
  const tree = parseGitObject(controllerInput.tree, 'attestation controller tree');
  if (tree.length !== commit.length) throw new TypeError('attestation Git object formats differ');
  const manifest = parseBlobIdentity(
    input.manifest, 'programme capture input attestation.manifest', commit.length,
  );
  if (manifest.path !== MANIFEST_PATH) {
    throw new TypeError('programme capture input attestation manifest path is invalid');
  }
  const taskInput = asClosedRecord(input.task, 'programme capture input attestation.task');
  assertExactKeys(
    taskInput, ['path', 'gitBlobId', 'sha256', 'byteLength', 'valueDigest'],
    'programme capture input attestation.task',
  );
  const task = {
    ...parseBlobIdentity(taskInput, 'programme capture input attestation.task', commit.length, true),
    valueDigest: parseDigest(
      taskInput.valueDigest, 'programme capture input attestation.task.valueDigest',
    ),
  };
  normalizeAcceptanceTaskPath(task.path);
  for (const [label, byteLength] of [
    ['manifest', manifest.byteLength], ['task', task.byteLength],
  ] as const) {
    if (byteLength === 0 || byteLength > MAX_JSON_BLOB_BYTES) {
      throw new TypeError(`capture ${label} blob byte length is invalid`);
    }
  }
  const protectedValues = asDenseArray(
    input.protectedInputs, 'programme capture input attestation.protectedInputs',
  );
  const protectedInputs = protectedValues.map((entry, index) => parseBlobIdentity(
    entry,
    `programme capture input attestation.protectedInputs[${index}]`,
    commit.length,
  ));
  if (protectedInputs.length !== PROTECTED_INPUT_PATHS.length
    || protectedInputs.some(({ path }, index) => path !== PROTECTED_INPUT_PATHS[index])) {
    throw new TypeError('programme capture input attestation protected input order is invalid');
  }
  let protectedInputBytes = 0;
  for (const { byteLength } of protectedInputs) {
    if (byteLength > MAX_INPUT_BLOB_BYTES) {
      throw new TypeError('programme capture input attestation protected input is too large');
    }
    protectedInputBytes += byteLength;
  }
  if (protectedInputBytes > MAX_INPUT_TOTAL_BYTES) {
    throw new TypeError('programme capture input attestation protected input total is too large');
  }
  const outputInput = asClosedRecord(
    input.output, 'programme capture input attestation.output',
  );
  assertExactKeys(
    outputInput, ['path', 'absentFromCommit'], 'programme capture input attestation.output',
  );
  if (outputInput.path !== PROGRAMME_CAPTURE_OUTPUT_PATH
    || outputInput.absentFromCommit !== true) {
    throw new TypeError('programme capture input attestation output absence is invalid');
  }
  const protectedInputsDigest = parseDigest(
    input.protectedInputsDigest,
    'programme capture input attestation.protectedInputsDigest',
  );
  if (protectedInputsDigest !== digestValue(protectedInputs)) {
    throw new Error('HARNESS_CAPTURE_PROTECTED_INPUTS_DIGEST_MISMATCH');
  }
  const body = {
    schemaVersion: 1 as const,
    transactionKind: 'programme-capture-v1' as const,
    controller: { commit, tree },
    manifest,
    task,
    protectedInputs,
    output: { path: PROGRAMME_CAPTURE_OUTPUT_PATH, absentFromCommit: true as const },
    protectedInputsDigest,
  };
  const attestationDigest = parseDigest(
    input.attestationDigest,
    'programme capture input attestation.attestationDigest',
  );
  if (attestationDigest !== digestValue(body)) {
    throw new Error('HARNESS_CAPTURE_INPUT_ATTESTATION_DIGEST_MISMATCH');
  }
  return deepFreeze({ ...body, attestationDigest });
}

export function parseProgrammeCaptureInputAttestationBlobV1(
  serialized: string,
): ProgrammeCaptureInputAttestationV1 {
  if (typeof serialized !== 'string' || Buffer.byteLength(serialized, 'utf8') > 1_048_576) {
    throw new TypeError('programme capture input attestation blob must be bounded UTF-8 JSON');
  }
  return parseProgrammeCaptureInputAttestationV1(
    parseJsonWithoutDuplicateKeys(serialized, 'programme capture input attestation'),
  );
}

function taskBindings(task: ProgrammeCaptureTaskV1): ReadonlyArray<{
  readonly path: string;
  readonly sha256: string;
}> {
  const bindings = [
    task.inputs.runnerProfile,
    task.inputs.scenarios,
    task.inputs.cargoLock,
    ...task.inputs.sources,
  ];
  if (bindings.length !== PROTECTED_INPUT_PATHS.length
    || bindings.some(({ path }, index) => path !== PROTECTED_INPUT_PATHS[index])) {
    throw new Error('HARNESS_CAPTURE_TASK_PROTECTED_INPUT_ORDER_INVALID');
  }
  return bindings;
}

function parseBlobIdentity(
  value: unknown,
  label: string,
  objectLength: number,
  allowAdditionalValueDigest = false,
): CaptureBlobIdentityV1 {
  const input = asClosedRecord(value, label);
  if (!allowAdditionalValueDigest) {
    assertExactKeys(input, ['path', 'gitBlobId', 'sha256', 'byteLength'], label);
  }
  const gitBlobId = parseGitObject(input.gitBlobId, `${label}.gitBlobId`);
  if (gitBlobId.length !== objectLength) throw new TypeError(`${label}.gitBlobId format differs`);
  return {
    path: normalizeWorkspacePath(input.path, `${label}.path`),
    gitBlobId,
    sha256: parseDigest(input.sha256, `${label}.sha256`),
    byteLength: asInteger(input.byteLength, `${label}.byteLength`),
  };
}

async function resolveExactCommit(
  root: string,
  requested: string,
  signal?: AbortSignal,
): Promise<string> {
  const resolved = await gitValue(root, ['rev-parse', '--verify', `${requested}^{commit}`], signal);
  if (resolved !== requested) throw new Error('HARNESS_CAPTURE_CONTROLLER_COMMIT_IDENTITY_MISMATCH');
  return resolved;
}

async function gitValue(root: string, args: readonly string[], signal?: AbortSignal): Promise<string> {
  return (await gitBytes(root, args, signal, 4096)).toString('utf8').trim();
}

async function gitBytes(
  root: string,
  args: readonly string[],
  signal: AbortSignal | undefined,
  maxOutputBytes: number,
): Promise<Buffer> {
  const result = await runGitCommandBytes(root, args, { signal, maxOutputBytes });
  if (result.exitCode !== 0) {
    throw new Error(`HARNESS_CAPTURE_GIT_COMMAND_FAILED:${args[0] ?? 'unknown'}`);
  }
  if (result.stderr !== '') {
    throw new Error(`HARNESS_CAPTURE_GIT_COMMAND_STDERR:${args[0] ?? 'unknown'}`);
  }
  return result.stdout;
}

function decodeUtf8(bytes: Buffer, label: string): string {
  let text: string;
  try {
    text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch {
    throw new Error(`HARNESS_CAPTURE_${label.toUpperCase()}_NOT_UTF-8`);
  }
  if (!Buffer.from(text, 'utf8').equals(bytes)) {
    throw new Error(`HARNESS_CAPTURE_${label.toUpperCase()}_NOT_CANONICAL_UTF-8`);
  }
  return text;
}

function parseGitObject(value: unknown, label: string): string {
  if (typeof value !== 'string' || !GIT_OBJECT.test(value)) {
    throw new TypeError(`${label} must be a full lowercase Git object ID`);
  }
  return value;
}

function parseDigest(value: unknown, label: string): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || /^0+$/.test(value)) {
    throw new TypeError(`${label} must be a non-zero lowercase SHA-256 digest`);
  }
  return value;
}
