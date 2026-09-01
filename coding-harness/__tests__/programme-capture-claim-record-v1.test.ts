// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  createProgrammeCaptureRunClaimV1,
  parseProgrammeCaptureRunClaimBlobV1,
  parseProgrammeCaptureRunClaimV1,
  programmeCaptureRunClaimKeyDigestV1,
  serializeProgrammeCaptureRunClaimV1,
  verifyProgrammeCaptureRunClaimV1,
} from '../src/programme-capture-claim-record-v1.js';
import {
  PROGRAMME_CAPTURE_PROTECTED_INPUT_PATHS_V1,
  parseProgrammeCaptureInputAttestationV1,
  type ProgrammeCaptureInputAttestationV1,
} from '../src/programme-capture-input-attestation-record-v1.js';
import { PROGRAMME_CAPTURE_OUTPUT_PATH } from '../src/programme-capture-task-v1.js';
import { parseProgrammeEnvelopeV6 } from '../src/programme-envelope-v6.js';
import { digestValue } from '../src/receipts.js';

const RUN_ID = 'capture_claim_20260828_0001';
const PROJECT_AUTHORITY = '1'.repeat(64);
const RUNNER_IDENTITY = '2'.repeat(64);
const digest = (character: string): string => character.repeat(64);

describe('programme capture V1 run-claim record', () => {
  it('derives one closed non-authorizing canonical record', () => {
    const inputAttestation = attestation();
    const claim = createProgrammeCaptureRunClaimV1({
      projectAuthorityDigest: PROJECT_AUTHORITY,
      runId: RUN_ID,
      inputAttestation,
      expectedRunnerIdentityDigest: RUNNER_IDENTITY,
    });
    const serialized = serializeProgrammeCaptureRunClaimV1(claim);
    const parsed = parseProgrammeCaptureRunClaimBlobV1(serialized);

    expect(parsed).toEqual(claim);
    expect(serialized).toBe(`${JSON.stringify(claim, null, 2)}\n`);
    expect(claim).toMatchObject({
      schemaVersion: 1,
      transactionKind: 'programme-capture-v1',
      recordKind: 'run-claim-v1',
      authority: {
        projectAuthorityDigest: PROJECT_AUTHORITY,
        persistence: 'same-uid-create-new-v1',
        rollbackResistance: 'not-proven',
        externalAppendOnlyWitness: false,
      },
      runId: RUN_ID,
      inputAttestationDigest: inputAttestation.attestationDigest,
      expectedRunnerIdentityDigest: RUNNER_IDENTITY,
      hostAdmission: 'not-evaluated',
      runnerLeaseAcquired: false,
      attemptStartAuthorized: false,
      captureAuthorized: false,
    });
    expect(claim.claimKeyDigest)
      .toBe('4132efd8b7b1efe5890c7cd10b0bf675744305e6d17bb789a75c87c563487489');
    expect(claim.claimDigest)
      .toBe('0f7c91bff74e9a9ef6710f03b0f3e73b085cfecfffd6f5c956bac14b2da4b3df');
    for (const value of [claim, claim.authority, claim.task, claim.runnerProfile]) {
      expect(Object.isFrozen(value)).toBe(true);
    }
  });

  it('limits claim-slot selection to project authority and run ID', () => {
    const original = createClaim();
    const changedController = createClaim({ inputAttestation: attestation({ controller: 'e' }) });
    const changedTask = createClaim({ inputAttestation: attestation({ taskValue: digest('f') }) });
    const changedRunner = createClaim({ expectedRunnerIdentityDigest: digest('3') });

    expect(changedController.claimKeyDigest).toBe(original.claimKeyDigest);
    expect(changedTask.claimKeyDigest).toBe(original.claimKeyDigest);
    expect(changedRunner.claimKeyDigest).toBe(original.claimKeyDigest);
    expect(new Set([
      original.claimDigest,
      changedController.claimDigest,
      changedTask.claimDigest,
      changedRunner.claimDigest,
    ])).toHaveLength(4);

    expect(programmeCaptureRunClaimKeyDigestV1({
      projectAuthorityDigest: digest('4'), runId: RUN_ID,
    })).not.toBe(original.claimKeyDigest);
    expect(programmeCaptureRunClaimKeyDigestV1({
      projectAuthorityDigest: PROJECT_AUTHORITY, runId: 'capture_claim_20260828_0002',
    })).not.toBe(original.claimKeyDigest);
  });

  it('requires independent authority, runner, run, and attestation comparisons', () => {
    const claim = createClaim();
    const inputAttestation = attestation();
    expect(verifyProgrammeCaptureRunClaimV1({
      claim,
      inputAttestation,
      expectedProjectAuthorityDigest: PROJECT_AUTHORITY,
      expectedRunId: RUN_ID,
      expectedRunnerIdentityDigest: RUNNER_IDENTITY,
    })).toEqual(claim);

    for (const overrides of [
      { expectedProjectAuthorityDigest: digest('4') },
      { expectedRunId: 'capture_claim_20260828_0002' },
      { expectedRunnerIdentityDigest: digest('5') },
      { inputAttestation: attestation({ controller: 'e' }) },
    ]) {
      expect(() => verifyProgrammeCaptureRunClaimV1({
        claim,
        inputAttestation,
        expectedProjectAuthorityDigest: PROJECT_AUTHORITY,
        expectedRunId: RUN_ID,
        expectedRunnerIdentityDigest: RUNNER_IDENTITY,
        ...overrides,
      })).toThrow(/AUTHORITY_MISMATCH/);
    }
  });

  it('rejects loose, duplicate, noncanonical, cross-version, and recomputed mutations', () => {
    const claim = createClaim();
    const serialized = serializeProgrammeCaptureRunClaimV1(claim);
    const unknown = { ...structuredClone(claim), extra: true };
    const duplicate = serialized.replace(
      '"schemaVersion": 1,', '"schemaVersion": 1,\n  "schemaVersion": 1,',
    );
    const bodyMutation = structuredClone(claim) as any;
    bodyMutation.captureAuthorized = true;
    bodyMutation.claimDigest = digestValue(withoutClaimDigest(bodyMutation));
    const keyMutation = structuredClone(claim) as any;
    keyMutation.claimKeyDigest = digest('a');
    keyMutation.claimDigest = digestValue(withoutClaimDigest(keyMutation));

    expect(() => parseProgrammeCaptureRunClaimV1(unknown)).toThrow(/invalid keys/);
    expect(() => parseProgrammeCaptureRunClaimBlobV1(duplicate)).toThrow(/duplicate/);
    expect(() => parseProgrammeCaptureRunClaimBlobV1(JSON.stringify(claim))).toThrow(/CANONICAL/);
    expect(() => parseProgrammeCaptureRunClaimBlobV1(`${JSON.stringify(claim, null, 4)}\n`))
      .toThrow(/CANONICAL/);
    expect(() => parseProgrammeCaptureRunClaimV1(bodyMutation)).toThrow(/IDENTITY_INVALID/);
    expect(() => parseProgrammeCaptureRunClaimV1(keyMutation)).toThrow(/KEY_MISMATCH/);
    expect(() => parseProgrammeCaptureRunClaimBlobV1('x'.repeat(100_001))).toThrow(/bounded/);
    expect(() => parseProgrammeCaptureRunClaimBlobV1('{"schemaVersion":6}\n')).toThrow();
    expect(() => parseProgrammeEnvelopeV6(serialized, digest('f'))).toThrow();
  });
});

function createClaim(overrides: Readonly<Record<string, unknown>> = {}) {
  return createProgrammeCaptureRunClaimV1({
    projectAuthorityDigest: PROJECT_AUTHORITY,
    runId: RUN_ID,
    inputAttestation: attestation(),
    expectedRunnerIdentityDigest: RUNNER_IDENTITY,
    ...overrides,
  } as any);
}

function attestation(overrides: Readonly<{
  controller?: string;
  taskValue?: string;
}> = {}): ProgrammeCaptureInputAttestationV1 {
  const controller = {
    commit: (overrides.controller ?? 'c').repeat(40),
    tree: 'd'.repeat(40),
  };
  const protectedInputs = PROGRAMME_CAPTURE_PROTECTED_INPUT_PATHS_V1.map((path, index) => ({
    path,
    gitBlobId: index === 0 ? '1'.repeat(40) : '2'.repeat(40),
    sha256: index === 0 ? digest('3') : digest('4'),
    byteLength: index === 0 ? 256 : 1,
  }));
  const body = {
    schemaVersion: 1 as const,
    transactionKind: 'programme-capture-v1' as const,
    controller,
    manifest: {
      path: 'coding-harness/.harness/manifest.json',
      gitBlobId: '5'.repeat(40), sha256: digest('6'), byteLength: 1,
    },
    task: {
      path: 'coding-harness/config/m0-performance-baseline-acceptance.json',
      gitBlobId: '7'.repeat(40), sha256: digest('8'), byteLength: 1,
      valueDigest: overrides.taskValue ?? digest('9'),
    },
    protectedInputs,
    output: { path: PROGRAMME_CAPTURE_OUTPUT_PATH, absentFromCommit: true as const },
    protectedInputsDigest: digestValue(protectedInputs),
  };
  return parseProgrammeCaptureInputAttestationV1({
    ...body, attestationDigest: digestValue(body),
  });
}

function withoutClaimDigest(value: Record<string, unknown>): Record<string, unknown> {
  const { claimDigest: _claimDigest, ...body } = value;
  return body;
}
