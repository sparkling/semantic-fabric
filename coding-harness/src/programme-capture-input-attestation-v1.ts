// SPDX-License-Identifier: MIT

import { deepFreeze } from './contracts.js';
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
} from './programme-capture-git-v1.js';
import {
  PROGRAMME_CAPTURE_MANIFEST_PATH_V1,
  PROGRAMME_CAPTURE_MAX_INPUT_BLOB_BYTES_V1,
  PROGRAMME_CAPTURE_MAX_INPUT_TOTAL_BYTES_V1,
  PROGRAMME_CAPTURE_MAX_JSON_BLOB_BYTES_V1,
  PROGRAMME_CAPTURE_PROTECTED_INPUT_PATHS_V1,
  parseProgrammeCaptureGitObjectV1,
  parseProgrammeCaptureInputAttestationV1,
  type ProgrammeCaptureInputAttestationV1,
} from './programme-capture-input-attestation-record-v1.js';
import {
  PROGRAMME_CAPTURE_OUTPUT_PATH,
  parseProgrammeCaptureTaskBlobV1,
  type ProgrammeCaptureTaskV1,
} from './programme-capture-task-v1.js';
import { digestValue } from './receipts.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export {
  parseProgrammeCaptureInputAttestationBlobV1,
  parseProgrammeCaptureInputAttestationV1,
  type CaptureBlobIdentityV1,
  type ProgrammeCaptureInputAttestationV1,
} from './programme-capture-input-attestation-record-v1.js';

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
  const controllerCommit = parseProgrammeCaptureGitObjectV1(
    input.controllerCommit, 'controller commit',
  );
  const taskPath = normalizeAcceptanceTaskPath(input.taskPath);
  const commit = await resolveExactCommit(store.path, controllerCommit, input.signal);
  const tree = await gitValue(store.path, ['rev-parse', `${commit}^{tree}`], input.signal);
  try {
    parseProgrammeCaptureGitObjectV1(tree, 'controller tree');
  } catch {
    throw new Error('HARNESS_CAPTURE_CONTROLLER_TREE_INVALID');
  }
  if (tree.length !== commit.length) {
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
    [PROGRAMME_CAPTURE_MANIFEST_PATH_V1],
    PROGRAMME_CAPTURE_MAX_JSON_BLOB_BYTES_V1,
    PROGRAMME_CAPTURE_MAX_JSON_BLOB_BYTES_V1,
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
    store.path, treeSnapshot, [selectedTaskPath], PROGRAMME_CAPTURE_MAX_JSON_BLOB_BYTES_V1,
    PROGRAMME_CAPTURE_MAX_JSON_BLOB_BYTES_V1, store.objectFormat, input.signal,
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
    PROGRAMME_CAPTURE_MAX_INPUT_BLOB_BYTES_V1,
    PROGRAMME_CAPTURE_MAX_INPUT_TOTAL_BYTES_V1,
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
  if (bindings.length !== PROGRAMME_CAPTURE_PROTECTED_INPUT_PATHS_V1.length
    || bindings.some(
      ({ path }, index) => path !== PROGRAMME_CAPTURE_PROTECTED_INPUT_PATHS_V1[index],
    )) {
    throw new Error('HARNESS_CAPTURE_TASK_PROTECTED_INPUT_ORDER_INVALID');
  }
  return bindings;
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
