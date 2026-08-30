// SPDX-License-Identifier: MIT

import {
  asClosedRecord,
  asInteger,
  assertExactKeys,
  deepFreeze,
  normalizeWorkspacePath,
} from './contracts.js';
import { parseTaskOpaqueId } from './acceptance-task-v3.js';
import { normalizeAcceptanceTaskPath } from './manifest.js';
import {
  parseProgrammeCaptureDigestV1 as parseDigest,
  programmeCaptureRunClaimKeyDigestV1,
} from './programme-capture-claim-key-v1.js';
import {
  PROGRAMME_CAPTURE_MAX_JSON_BLOB_BYTES_V1,
  parseProgrammeCaptureGitObjectV1,
  parseProgrammeCaptureInputAttestationV1,
  type ProgrammeCaptureInputAttestationV1,
} from './programme-capture-input-attestation-record-v1.js';
import { PROGRAMME_CAPTURE_PROFILE_PATH } from './programme-capture-task-v1.js';
import { digestValue } from './receipts.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export const PROGRAMME_CAPTURE_RUN_CLAIM_MAX_BYTES_V1 = 100_000;
export { programmeCaptureRunClaimKeyDigestV1 } from './programme-capture-claim-key-v1.js';

interface ClaimBlobIdentityV1 {
  readonly path: string;
  readonly gitBlobId: string;
  readonly sha256: string;
  readonly byteLength: number;
}

interface ClaimTaskIdentityV1 extends ClaimBlobIdentityV1 {
  readonly valueDigest: string;
}

export interface ProgrammeCaptureRunClaimV1 {
  readonly schemaVersion: 1;
  readonly transactionKind: 'programme-capture-v1';
  readonly recordKind: 'run-claim-v1';
  readonly authority: Readonly<{
    projectAuthorityDigest: string;
    persistence: 'same-uid-create-new-v1';
    rollbackResistance: 'not-proven';
    externalAppendOnlyWitness: false;
  }>;
  readonly runId: string;
  readonly controller: Readonly<{ commit: string; tree: string }>;
  readonly task: ClaimTaskIdentityV1;
  readonly inputAttestationDigest: string;
  readonly runnerProfile: ClaimBlobIdentityV1 & Readonly<{
    path: typeof PROGRAMME_CAPTURE_PROFILE_PATH;
  }>;
  readonly expectedRunnerIdentityDigest: string;
  readonly hostAdmission: 'not-evaluated';
  readonly runnerLeaseAcquired: false;
  readonly attemptStartAuthorized: false;
  readonly captureAuthorized: false;
  readonly claimKeyDigest: string;
  readonly claimDigest: string;
}

const CLAIM_KEYS = [
  'schemaVersion', 'transactionKind', 'recordKind', 'authority', 'runId', 'controller',
  'task', 'inputAttestationDigest', 'runnerProfile', 'expectedRunnerIdentityDigest',
  'hostAdmission', 'runnerLeaseAcquired', 'attemptStartAuthorized', 'captureAuthorized',
  'claimKeyDigest', 'claimDigest',
] as const;

export function createProgrammeCaptureRunClaimV1(value: Readonly<{
  projectAuthorityDigest: string;
  runId: string;
  inputAttestation: ProgrammeCaptureInputAttestationV1;
  expectedRunnerIdentityDigest: string;
}>): ProgrammeCaptureRunClaimV1 {
  const input = asClosedRecord(value, 'programme capture run-claim creation input');
  assertExactKeys(input, [
    'projectAuthorityDigest', 'runId', 'inputAttestation', 'expectedRunnerIdentityDigest',
  ], 'programme capture run-claim creation input');
  const projectAuthorityDigest = parseDigest(
    input.projectAuthorityDigest, 'programme capture project authority digest',
  );
  const runId = parseTaskOpaqueId(input.runId, 'programme capture run-claim runId');
  const inputAttestation = parseProgrammeCaptureInputAttestationV1(input.inputAttestation);
  const expectedRunnerIdentityDigest = parseDigest(
    input.expectedRunnerIdentityDigest, 'programme capture expected runner identity digest',
  );
  const runnerProfile = inputAttestation.protectedInputs[0];
  if (runnerProfile?.path !== PROGRAMME_CAPTURE_PROFILE_PATH) {
    throw new Error('HARNESS_CAPTURE_CLAIM_RUNNER_PROFILE_MISSING');
  }
  const body = {
    schemaVersion: 1 as const,
    transactionKind: 'programme-capture-v1' as const,
    recordKind: 'run-claim-v1' as const,
    authority: {
      projectAuthorityDigest,
      persistence: 'same-uid-create-new-v1' as const,
      rollbackResistance: 'not-proven' as const,
      externalAppendOnlyWitness: false as const,
    },
    runId,
    controller: inputAttestation.controller,
    task: inputAttestation.task,
    inputAttestationDigest: inputAttestation.attestationDigest,
    runnerProfile,
    expectedRunnerIdentityDigest,
    hostAdmission: 'not-evaluated' as const,
    runnerLeaseAcquired: false as const,
    attemptStartAuthorized: false as const,
    captureAuthorized: false as const,
    claimKeyDigest: programmeCaptureRunClaimKeyDigestV1({ projectAuthorityDigest, runId }),
  };
  return parseProgrammeCaptureRunClaimV1({ ...body, claimDigest: digestValue(body) });
}

export function parseProgrammeCaptureRunClaimV1(value: unknown): ProgrammeCaptureRunClaimV1 {
  const input = asClosedRecord(value, 'programme capture run claim V1');
  assertExactKeys(input, CLAIM_KEYS, 'programme capture run claim V1');
  if (input.schemaVersion !== 1 || input.transactionKind !== 'programme-capture-v1'
    || input.recordKind !== 'run-claim-v1' || input.hostAdmission !== 'not-evaluated'
    || input.runnerLeaseAcquired !== false || input.attemptStartAuthorized !== false
    || input.captureAuthorized !== false) {
    throw new TypeError('HARNESS_CAPTURE_CLAIM_IDENTITY_INVALID');
  }
  const authorityInput = asClosedRecord(input.authority, 'programme capture claim.authority');
  assertExactKeys(authorityInput, [
    'projectAuthorityDigest', 'persistence', 'rollbackResistance',
    'externalAppendOnlyWitness',
  ], 'programme capture claim.authority');
  if (authorityInput.persistence !== 'same-uid-create-new-v1'
    || authorityInput.rollbackResistance !== 'not-proven'
    || authorityInput.externalAppendOnlyWitness !== false) {
    throw new TypeError('HARNESS_CAPTURE_CLAIM_AUTHORITY_CLASS_INVALID');
  }
  const authority = {
    projectAuthorityDigest: parseDigest(
      authorityInput.projectAuthorityDigest, 'programme capture claim project authority',
    ),
    persistence: 'same-uid-create-new-v1' as const,
    rollbackResistance: 'not-proven' as const,
    externalAppendOnlyWitness: false as const,
  };
  const runId = parseTaskOpaqueId(input.runId, 'programme capture claim.runId');
  const controllerInput = asClosedRecord(input.controller, 'programme capture claim.controller');
  assertExactKeys(controllerInput, ['commit', 'tree'], 'programme capture claim.controller');
  const controller = {
    commit: parseProgrammeCaptureGitObjectV1(
      controllerInput.commit, 'programme capture claim controller commit',
    ),
    tree: parseProgrammeCaptureGitObjectV1(
      controllerInput.tree, 'programme capture claim controller tree',
    ),
  };
  if (controller.commit.length !== controller.tree.length) {
    throw new TypeError('HARNESS_CAPTURE_CLAIM_GIT_FORMAT_MISMATCH');
  }
  const task = parseTaskIdentity(input.task, controller.commit.length);
  const runnerProfile = parseBlobIdentity(
    input.runnerProfile, 'programme capture claim.runnerProfile', controller.commit.length,
  );
  if (runnerProfile.path !== PROGRAMME_CAPTURE_PROFILE_PATH) {
    throw new TypeError('HARNESS_CAPTURE_CLAIM_RUNNER_PROFILE_INVALID');
  }
  const body = {
    schemaVersion: 1 as const,
    transactionKind: 'programme-capture-v1' as const,
    recordKind: 'run-claim-v1' as const,
    authority,
    runId,
    controller,
    task,
    inputAttestationDigest: parseDigest(
      input.inputAttestationDigest, 'programme capture claim input attestation digest',
    ),
    runnerProfile: { ...runnerProfile, path: PROGRAMME_CAPTURE_PROFILE_PATH },
    expectedRunnerIdentityDigest: parseDigest(
      input.expectedRunnerIdentityDigest, 'programme capture claim runner identity digest',
    ),
    hostAdmission: 'not-evaluated' as const,
    runnerLeaseAcquired: false as const,
    attemptStartAuthorized: false as const,
    captureAuthorized: false as const,
    claimKeyDigest: parseDigest(input.claimKeyDigest, 'programme capture claim key digest'),
  };
  const expectedKey = programmeCaptureRunClaimKeyDigestV1({
    projectAuthorityDigest: authority.projectAuthorityDigest, runId,
  });
  if (body.claimKeyDigest !== expectedKey) {
    throw new Error('HARNESS_CAPTURE_CLAIM_KEY_MISMATCH');
  }
  const claimDigest = parseDigest(input.claimDigest, 'programme capture claim digest');
  if (claimDigest !== digestValue(body)) {
    throw new Error('HARNESS_CAPTURE_CLAIM_DIGEST_MISMATCH');
  }
  return deepFreeze({ ...body, claimDigest });
}

export function serializeProgrammeCaptureRunClaimV1(value: unknown): string {
  const claim = parseProgrammeCaptureRunClaimV1(value);
  return `${JSON.stringify(claim, null, 2)}\n`;
}

export function parseProgrammeCaptureRunClaimBlobV1(
  serialized: string,
): ProgrammeCaptureRunClaimV1 {
  if (typeof serialized !== 'string'
    || Buffer.byteLength(serialized, 'utf8') > PROGRAMME_CAPTURE_RUN_CLAIM_MAX_BYTES_V1
    || decodeCanonicalUtf8(serialized) !== serialized) {
    throw new TypeError('programme capture run-claim blob must be bounded canonical UTF-8 JSON');
  }
  const claim = parseProgrammeCaptureRunClaimV1(
    parseJsonWithoutDuplicateKeys(serialized, 'programme capture run claim V1'),
  );
  if (serializeProgrammeCaptureRunClaimV1(claim) !== serialized) {
    throw new Error('HARNESS_CAPTURE_CLAIM_CANONICAL_SERIALIZATION_REQUIRED');
  }
  return claim;
}

export function verifyProgrammeCaptureRunClaimV1(value: Readonly<{
  claim: ProgrammeCaptureRunClaimV1;
  inputAttestation: ProgrammeCaptureInputAttestationV1;
  expectedProjectAuthorityDigest: string;
  expectedRunId: string;
  expectedRunnerIdentityDigest: string;
}>): ProgrammeCaptureRunClaimV1 {
  const input = asClosedRecord(value, 'programme capture run-claim verification input');
  assertExactKeys(input, [
    'claim', 'inputAttestation', 'expectedProjectAuthorityDigest',
    'expectedRunId', 'expectedRunnerIdentityDigest',
  ], 'programme capture run-claim verification input');
  const claim = parseProgrammeCaptureRunClaimV1(input.claim);
  const expected = createProgrammeCaptureRunClaimV1({
    projectAuthorityDigest: parseDigest(
      input.expectedProjectAuthorityDigest, 'expected project authority digest',
    ),
    runId: parseTaskOpaqueId(input.expectedRunId, 'expected programme capture runId'),
    inputAttestation: parseProgrammeCaptureInputAttestationV1(input.inputAttestation),
    expectedRunnerIdentityDigest: parseDigest(
      input.expectedRunnerIdentityDigest, 'expected runner identity digest',
    ),
  });
  if (JSON.stringify(claim) !== JSON.stringify(expected)) {
    throw new Error('HARNESS_CAPTURE_CLAIM_AUTHORITY_MISMATCH');
  }
  return claim;
}

function parseTaskIdentity(value: unknown, gitLength: number): ClaimTaskIdentityV1 {
  const input = asClosedRecord(value, 'programme capture claim.task');
  assertExactKeys(
    input, ['path', 'gitBlobId', 'sha256', 'byteLength', 'valueDigest'],
    'programme capture claim.task',
  );
  const blob = parseBlobIdentity(input, 'programme capture claim.task', gitLength, true);
  return {
    ...blob,
    path: normalizeAcceptanceTaskPath(blob.path),
    valueDigest: parseDigest(input.valueDigest, 'programme capture claim.task.valueDigest'),
  };
}

function parseBlobIdentity(
  value: unknown,
  label: string,
  gitLength: number,
  additionalKey = false,
): ClaimBlobIdentityV1 {
  const input = asClosedRecord(value, label);
  if (!additionalKey) {
    assertExactKeys(input, ['path', 'gitBlobId', 'sha256', 'byteLength'], label);
  }
  const path = normalizeWorkspacePath(input.path, `${label}.path`);
  const gitBlobId = parseProgrammeCaptureGitObjectV1(input.gitBlobId, `${label}.gitBlobId`);
  const byteLength = asInteger(input.byteLength, `${label}.byteLength`);
  if (gitBlobId.length !== gitLength || byteLength < 1
    || byteLength > PROGRAMME_CAPTURE_MAX_JSON_BLOB_BYTES_V1) {
    throw new TypeError(`${label} identity is invalid`);
  }
  return { path, gitBlobId, sha256: parseDigest(input.sha256, `${label}.sha256`), byteLength };
}

function decodeCanonicalUtf8(value: string): string {
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(Buffer.from(value, 'utf8'));
  } catch {
    return '';
  }
}
