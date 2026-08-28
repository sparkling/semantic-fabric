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
import { normalizeAcceptanceTaskPath } from './manifest.js';
import type { CaptureBlobIdentityV1 } from './programme-capture-git-v1.js';
import {
  PROGRAMME_CAPTURE_OUTPUT_PATH,
  PROGRAMME_CAPTURE_PROFILE_PATH,
  PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS,
  PROGRAMME_CAPTURE_SCENARIOS_PATH,
} from './programme-capture-task-v1.js';
import { digestValue, type GitIdentity } from './receipts.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export const PROGRAMME_CAPTURE_MANIFEST_PATH_V1 =
  'coding-harness/.harness/manifest.json' as const;
export const PROGRAMME_CAPTURE_MAX_JSON_BLOB_BYTES_V1 = 1_048_576;
export const PROGRAMME_CAPTURE_MAX_INPUT_BLOB_BYTES_V1 = 10_000_000;
export const PROGRAMME_CAPTURE_MAX_INPUT_TOTAL_BYTES_V1 = 100_000_000;
export const PROGRAMME_CAPTURE_PROTECTED_INPUT_PATHS_V1 = Object.freeze([
  PROGRAMME_CAPTURE_PROFILE_PATH,
  PROGRAMME_CAPTURE_SCENARIOS_PATH,
  'Cargo.lock',
  ...PROGRAMME_CAPTURE_REQUIRED_SOURCE_PATHS,
] as const);

const GIT_OBJECT = /^(?:[a-f0-9]{40}|[a-f0-9]{64})$/;

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
  const commit = parseProgrammeCaptureGitObjectV1(
    controllerInput.commit, 'attestation controller commit',
  );
  const tree = parseProgrammeCaptureGitObjectV1(
    controllerInput.tree, 'attestation controller tree',
  );
  if (tree.length !== commit.length) throw new TypeError('attestation Git object formats differ');
  const manifest = parseBlobIdentity(
    input.manifest, 'programme capture input attestation.manifest', commit.length,
  );
  if (manifest.path !== PROGRAMME_CAPTURE_MANIFEST_PATH_V1) {
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
    if (byteLength === 0 || byteLength > PROGRAMME_CAPTURE_MAX_JSON_BLOB_BYTES_V1) {
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
  if (protectedInputs.length !== PROGRAMME_CAPTURE_PROTECTED_INPUT_PATHS_V1.length
    || protectedInputs.some(
      ({ path }, index) => path !== PROGRAMME_CAPTURE_PROTECTED_INPUT_PATHS_V1[index],
    )) {
    throw new TypeError('programme capture input attestation protected input order is invalid');
  }
  let protectedInputBytes = 0;
  for (const { byteLength } of protectedInputs) {
    if (byteLength > PROGRAMME_CAPTURE_MAX_INPUT_BLOB_BYTES_V1) {
      throw new TypeError('programme capture input attestation protected input is too large');
    }
    protectedInputBytes += byteLength;
  }
  if (protectedInputBytes > PROGRAMME_CAPTURE_MAX_INPUT_TOTAL_BYTES_V1) {
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
  if (typeof serialized !== 'string'
    || Buffer.byteLength(serialized, 'utf8') > PROGRAMME_CAPTURE_MAX_JSON_BLOB_BYTES_V1) {
    throw new TypeError('programme capture input attestation blob must be bounded UTF-8 JSON');
  }
  return parseProgrammeCaptureInputAttestationV1(
    parseJsonWithoutDuplicateKeys(serialized, 'programme capture input attestation'),
  );
}

export function parseProgrammeCaptureGitObjectV1(value: unknown, label: string): string {
  if (typeof value !== 'string' || !GIT_OBJECT.test(value)) {
    throw new TypeError(`${label} must be a full lowercase Git object ID`);
  }
  return value;
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
  const gitBlobId = parseProgrammeCaptureGitObjectV1(input.gitBlobId, `${label}.gitBlobId`);
  if (gitBlobId.length !== objectLength) throw new TypeError(`${label}.gitBlobId format differs`);
  return {
    path: normalizeWorkspacePath(input.path, `${label}.path`),
    gitBlobId,
    sha256: parseDigest(input.sha256, `${label}.sha256`),
    byteLength: asInteger(input.byteLength, `${label}.byteLength`),
  };
}

function parseDigest(value: unknown, label: string): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || /^0+$/.test(value)) {
    throw new TypeError(`${label} must be a non-zero lowercase SHA-256 digest`);
  }
  return value;
}
