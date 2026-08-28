// SPDX-License-Identifier: MIT

import {
  parseCandidateTransactionEvidenceV1,
  type CandidateTransactionEvidenceV1,
} from './candidate-transaction-evidence-v1.js';
import { DEVELOPMENT_AUTHORITY, SHA256_PATTERN, deepFreeze } from './contracts.js';
import type { MetaHarnessDiagnosticSnapshot } from './metaharness-diagnostics.js';
import type { ProgrammeV5RufloEvidence } from './programme-v5-ruflo-contract.js';
import {
  evaluateProgrammeGatesV5, type ProgrammeGateEvaluationV5,
} from './programme-gates-v5.js';
import type { ProgrammeGateContractV1 } from './programme-gate-contract-v1.js';
import type { ParsedProgrammePolicyV2 } from './programme-policy-v6.js';
import {
  programmeCommandReceiptProjectionsV1,
  type ProgrammeCommandReceiptProjectionV1,
} from './programme-task-runtime-v1.js';
import { digestValue, type CommandEvidence, type Receipt } from './receipts.js';

const GENESIS_DIGEST = '0'.repeat(64);

export interface ProgrammeGateEvaluationV6 extends ProgrammeGateEvaluationV5 {
  readonly candidateTransactionEvidence: CandidateTransactionEvidenceV1 | null;
}

export function evaluateProgrammeGatesV6(input: Readonly<{
  policy: ParsedProgrammePolicyV2;
  receipt: Receipt;
  diagnostics: MetaHarnessDiagnosticSnapshot;
  rufloEvidence: ProgrammeV5RufloEvidence;
  candidateTransactionEvidence: unknown;
}>): ProgrammeGateEvaluationV6 {
  if (!nonnegativeInteger(input.receipt.recovery.repairCount)
    || input.receipt.recovery.repairCount > input.policy.snapshot.gateContract.attempts.maximumRepairs) {
    throw new Error('HARNESS_PROGRAMME_V6_REPAIR_COUNT_EXCEEDS_POLICY');
  }
  const evidence = candidateEvidenceForReceipt(
    input.candidateTransactionEvidence,
    input.receipt,
  );
  const base = evaluateProgrammeGatesV5({
    policy: input.policy.base,
    receipt: input.receipt,
    diagnostics: input.diagnostics,
    rufloEvidence: input.rufloEvidence,
  });
  const reliability = evidence !== null
    && validReliabilityBase(input.policy, input.receipt)
    && validTransitionAttemptHistory(input.policy, input.receipt, evidence);
  const dimensions = base.dimensions.map((dimension) => {
    const passed = dimension.id === 'reliability-and-receipts'
      ? reliability
      : dimension.passed;
    return deepFreeze({
      id: dimension.id,
      passed,
      evidenceDigest: digestValue({
        schemaVersion: 2,
        authority: DEVELOPMENT_AUTHORITY,
        id: dimension.id,
        baseEvidenceDigest: dimension.evidenceDigest,
        programmePolicyFingerprint: input.policy.fingerprint,
        candidateTransactionEvidenceDigest: evidence?.evidenceDigest ?? null,
        passed,
      }),
    });
  });
  return deepFreeze({
    dimensions,
    diagnostics: base.diagnostics,
    candidateTransactionEvidence: evidence,
  });
}

function candidateEvidenceForReceipt(
  value: unknown,
  receipt: Receipt,
): CandidateTransactionEvidenceV1 | null {
  if (receipt.status === 'pass') return parseCandidateTransactionEvidenceV1(value, receipt);
  if (value !== null) throw new Error('HARNESS_PROGRAMME_V6_NONPASS_EVIDENCE_FORBIDDEN');
  return null;
}

function validTransitionAttemptHistory(
  policy: ParsedProgrammePolicyV2,
  receipt: Receipt,
  evidence: CandidateTransactionEvidenceV1,
): boolean {
  const expected = programmeCommandReceiptProjectionsV1(policy.base.task)
    .filter(({ stage }) => stage === 'build');
  if (evidence.repairTransitions.length !== receipt.recovery.repairCount) return false;
  for (const transition of evidence.repairTransitions) {
    const actual = receipt.commands.filter((command) =>
      command.stage === 'build' && command.attempt === transition.fromAttempt);
    if (actual.some((command) => command.candidateTree !== transition.sourceCandidate.tree
      || !normalProgrammeCommandCompletionV1(command))) return false;
    if (transition.buildDisposition === 'not-started') {
      if (actual.length !== 0) return false;
      continue;
    }
    if (actual.length === 0 || actual.length > expected.length
      || actual.some((command, index) =>
        !matchesProgrammeCommandProjectionV1(command, expected[index]))) return false;
    if (transition.buildDisposition === 'failed') {
      if (!actual.slice(0, -1).every(({ exitCode }) => exitCode === 0)
        || !Number.isSafeInteger(actual.at(-1)?.exitCode)
        || actual.at(-1)?.exitCode === 0) return false;
      continue;
    }
    if (actual.length !== expected.length || actual.some(({ exitCode }) => exitCode !== 0)) {
      return false;
    }
  }
  return true;
}

function validReliabilityBase(policy: ParsedProgrammePolicyV2, receipt: Receipt): boolean {
  const gate = policy.base.snapshot.gateContract;
  return validProgrammeReceiptIntegrityV1(receipt, gate)
    && receipt.recovery.cancelled === gate.reliability.cancelled
    && receipt.recovery.breakerState === gate.reliability.breakerState
    && nonnegativeInteger(receipt.recovery.retryCount)
    && receipt.commands.every(normalProgrammeCommandCompletionV1);
}

function validProgrammeReceiptIntegrityV1(
  receipt: Receipt,
  gate: ProgrammeGateContractV1,
): boolean {
  const { digest, ...body } = receipt;
  return receipt.schemaVersion === gate.receiptSchemaVersion
    && receipt.sequence === gate.receipt.sequence
    && receipt.previousDigest === gate.receipt.previousDigest
    && validDigest(digest) && digestValue(body) === digest
    && receipt.step === gate.receipt.step
    && receipt.status === gate.receipt.status
    && receipt.failureCode === gate.receipt.failureCode
    && receipt.authority === gate.receipt.authority
    && canonicalTimestamp(receipt.issuedAt)
    && nonnegativeInteger(receipt.recovery.repairCount)
    && receipt.recovery.repairCount <= gate.attempts.maximumRepairs
    && receipt.patchDigests.length === receipt.recovery.repairCount + 1
    && receipt.patchDigests.every(validDigest)
    && receipt.patchDigest !== null && validDigest(receipt.patchDigest)
    && receipt.patchDigests.at(-1) === receipt.patchDigest;
}

function matchesProgrammeCommandProjectionV1(
  command: CommandEvidence,
  expected: ProgrammeCommandReceiptProjectionV1 | undefined,
): boolean {
  return expected !== undefined
    && command.stage === expected.stage && command.tool === expected.tool
    && command.executable === expected.executable && command.cwd === expected.cwd
    && sameStrings(command.argv, expected.argv);
}

function normalProgrammeCommandCompletionV1(command: CommandEvidence): boolean {
  return command.signal === null && !command.timedOut && !command.cancelled
    && !command.outputLimitExceeded && command.spawnErrorDigest === null;
}

function validDigest(value: unknown): value is string {
  return typeof value === 'string' && SHA256_PATTERN.test(value) && value !== GENESIS_DIGEST;
}

function canonicalTimestamp(value: string): boolean {
  const date = new Date(value);
  return Number.isFinite(date.valueOf()) && date.toISOString() === value;
}

function sameStrings(actual: readonly string[], expected: readonly string[]): boolean {
  return actual.length === expected.length && actual.every((value, index) => value === expected[index]);
}

function nonnegativeInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) >= 0;
}
