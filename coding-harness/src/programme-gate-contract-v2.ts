// SPDX-License-Identifier: MIT

import {
  DEVELOPMENT_AUTHORITY,
  asRecord,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import {
  parseProgrammeGateContractV1,
  type ProgrammeGateContractV1,
} from './programme-gate-contract-v1.js';
import { digestValue } from './receipts.js';

export const PROGRAMME_GATE_CONTRACT_V2_ID =
  'semantic-fabric-programme-gate-contract-v2' as const;

export interface ProgrammeGateContractV2 {
  readonly schemaVersion: 2;
  readonly contractId: typeof PROGRAMME_GATE_CONTRACT_V2_ID;
  readonly authority: typeof DEVELOPMENT_AUTHORITY;
  readonly authoritativeReplaySemantics: true;
  readonly baseGateContract: ProgrammeGateContractV1;
  readonly baseGateContractDigest: string;
  readonly attempts: Readonly<{
    maximumRepairs: number;
    evidenceSchemaVersion: 1;
    evidenceKind: 'candidate-transaction-repair-evidence';
    transitionCardinality: 'receipt.recovery.repairCount';
    sequenceRule: 'contiguous-zero-based-fromAttempt-toAttempt';
    patchHistoryBinding: 'transition-source-and-replacement-equal-adjacent-receipt-patchDigests';
    triggerDispositionRule: 'patch-admission-or-admission-validation-not-started-build-failed-otherwise-passed';
    dispositionRule: 'not-started-zero-builds-failed-prefix-passed-exact-successful-set';
    sourceCandidateRule: 'every-prior-build-command-binds-transition.sourceCandidate.tree';
    finalAttemptSemantics: 'programme-gate-contract-v1-unchanged';
  }>;
  readonly nativeEvidence: Readonly<{
    schemaVersion: 2;
    fullRuntimeEvidenceRequired: true;
    invocationSetRule: 'exact-nativeInvocationBindings';
    implementationPatchRule: 'equals-receipt.patchDigests[0]';
    repairPatchRule: 'equals-transition.replacementPatchDigest';
    receiptDigestRule: 'equals-receipt.coordination.nativeRuntimeEvidenceDigest';
  }>;
  readonly envelope: Readonly<{
    schemaVersion: 6;
    policyFingerprintBinding: 'externally-supplied-v2-anchor';
    candidateEvidenceDigestBinding:
      'envelope.candidateTransactionEvidenceDigest-equals-candidateTransactionEvidence.evidenceDigest';
    baseReceiptAndDiagnosticSemantics: 'programme-gate-contract-v1-unchanged';
  }>;
  readonly evaluation: Readonly<{
    baseDimensions: 'evaluateProgrammeGatesV5-over-frozen-base-policy';
    reliabilityRule: 'v1-reliability-base-and-transition-aware-prior-attempt-history';
    dimensionEvidenceRule:
      'schema-v2-binds-base-digest-v2-policy-candidate-evidence-and-result';
  }>;
}

const TOP_KEYS = [
  'schemaVersion', 'contractId', 'authority', 'authoritativeReplaySemantics',
  'baseGateContract', 'baseGateContractDigest', 'attempts', 'nativeEvidence', 'envelope',
  'evaluation',
] as const;

export function createProgrammeGateContractV2(
  baseGateContract: ProgrammeGateContractV1,
): ProgrammeGateContractV2 {
  const base = parseProgrammeGateContractV1(baseGateContract);
  return deepFreeze({
    schemaVersion: 2,
    contractId: PROGRAMME_GATE_CONTRACT_V2_ID,
    authority: DEVELOPMENT_AUTHORITY,
    authoritativeReplaySemantics: true,
    baseGateContract: base,
    baseGateContractDigest: digestValue(base),
    attempts: {
      maximumRepairs: base.attempts.maximumRepairs,
      evidenceSchemaVersion: 1,
      evidenceKind: 'candidate-transaction-repair-evidence',
      transitionCardinality: 'receipt.recovery.repairCount',
      sequenceRule: 'contiguous-zero-based-fromAttempt-toAttempt',
      patchHistoryBinding: 'transition-source-and-replacement-equal-adjacent-receipt-patchDigests',
      triggerDispositionRule:
        'patch-admission-or-admission-validation-not-started-build-failed-otherwise-passed',
      dispositionRule: 'not-started-zero-builds-failed-prefix-passed-exact-successful-set',
      sourceCandidateRule: 'every-prior-build-command-binds-transition.sourceCandidate.tree',
      finalAttemptSemantics: 'programme-gate-contract-v1-unchanged',
    },
    nativeEvidence: {
      schemaVersion: 2,
      fullRuntimeEvidenceRequired: true,
      invocationSetRule: 'exact-nativeInvocationBindings',
      implementationPatchRule: 'equals-receipt.patchDigests[0]',
      repairPatchRule: 'equals-transition.replacementPatchDigest',
      receiptDigestRule: 'equals-receipt.coordination.nativeRuntimeEvidenceDigest',
    },
    envelope: {
      schemaVersion: 6,
      policyFingerprintBinding: 'externally-supplied-v2-anchor',
      candidateEvidenceDigestBinding:
        'envelope.candidateTransactionEvidenceDigest-equals-candidateTransactionEvidence.evidenceDigest',
      baseReceiptAndDiagnosticSemantics: 'programme-gate-contract-v1-unchanged',
    },
    evaluation: {
      baseDimensions: 'evaluateProgrammeGatesV5-over-frozen-base-policy',
      reliabilityRule: 'v1-reliability-base-and-transition-aware-prior-attempt-history',
      dimensionEvidenceRule:
        'schema-v2-binds-base-digest-v2-policy-candidate-evidence-and-result',
    },
  });
}

export function parseProgrammeGateContractV2(value: unknown): ProgrammeGateContractV2 {
  const input = asRecord(value, 'programme gate contract V2');
  assertExactKeys(input, TOP_KEYS, 'programme gate contract V2');
  const base = parseProgrammeGateContractV1(input.baseGateContract);
  const expected = createProgrammeGateContractV2(base);
  assertExactShape(input, expected, 'programme gate contract V2');
  return expected;
}

function assertExactShape(actual: unknown, expected: unknown, label: string): void {
  if (Array.isArray(expected)) {
    if (!Array.isArray(actual) || actual.length !== expected.length) invalid();
    expected.forEach((entry, index) => assertExactShape(
      (actual as unknown[])[index], entry, `${label}[${index}]`,
    ));
    return;
  }
  if (expected !== null && typeof expected === 'object') {
    const input = asRecord(actual, label);
    const keys = Object.keys(expected as Record<string, unknown>);
    assertExactKeys(input, keys, label);
    for (const key of keys) {
      assertExactShape(input[key], (expected as Record<string, unknown>)[key], `${label}.${key}`);
    }
    return;
  }
  if (!Object.is(actual, expected)) invalid();

  function invalid(): never {
    throw new Error('HARNESS_PROGRAMME_GATE_CONTRACT_V2_INVALID');
  }
}
