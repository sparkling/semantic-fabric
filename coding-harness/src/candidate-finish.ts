// SPDX-License-Identifier: MIT

import {
  sealCandidateRepairTransitions,
  type CandidateRepairTransition,
} from './candidate-repair-transition.js';
import { runtimeTrustUnavailable } from './candidate-gates.js';
import type {
  CandidateEvidenceState,
  CandidateOperations,
  CandidateTransactionContext,
  CandidateTransactionResult,
} from './candidate-types.js';
import { deepFreeze, DEVELOPMENT_AUTHORITY } from './contracts.js';
import { bindNativeRuntimeEvidenceV2 } from './native-runtime-evidence-v2.js';
import type { ReceiptFailureCode } from './failure-code.js';
import { digestValue, type ReceiptChain } from './receipts.js';

export interface FinishCandidateTransactionInput {
  readonly receipts: ReceiptChain;
  readonly context: CandidateTransactionContext;
  readonly operations: CandidateOperations;
  readonly now: () => string;
  readonly requestedStatus: CandidateTransactionResult['status'];
  readonly requestedReason: string | null;
  readonly requestedFailureCode: ReceiptFailureCode | null;
  readonly evidence: CandidateEvidenceState;
  readonly finalPatch: string | null;
}

export async function finishCandidateTransaction(
  input: FinishCandidateTransactionInput,
): Promise<CandidateTransactionResult> {
  let status = input.requestedStatus;
  let reason = input.requestedReason;
  let failureCode = input.requestedFailureCode;
  let repairTransitions: readonly CandidateRepairTransition[] = [];
  try {
    const recovery = input.operations.recoveryEvidence();
    input.evidence.runtime = {
      retryCount: recovery.retryCount,
      breakerState: recovery.breakerState,
    };
    const runtime = input.operations.runtimeEvidence(input.evidence.nativeInvocations);
    if (input.requestedStatus === 'pass') {
      const native = bindNativeRuntimeEvidenceV2({
        value: runtime.nativeEvidence,
        taskId: input.context.taskId,
        runId: input.context.runId,
        hosts: input.context.hosts,
        expectations: input.evidence.nativeInvocations,
      });
      input.evidence.coordination.nativeEvidenceDigests = [
        ...native.hosts.map(digestValue),
        ...native.invocations.map(digestValue),
      ];
      input.evidence.coordination.nativeRuntimeEvidenceDigest = digestValue(native);
      repairTransitions = sealCandidateRepairTransitions(input.evidence.repairTransitions, native);
    }
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    const runtimeReason = `HARNESS_RUNTIME_EVIDENCE_FAILED:${detail}`;
    status = runtimeTrustUnavailable(error) ? 'gated' : 'fail';
    reason = reason === null ? runtimeReason : `${reason}; ${runtimeReason}`;
    failureCode ??= 'HARNESS_RUNTIME_EVIDENCE_FAILED';
  }
  try {
    await input.operations.cleanup();
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    const cleanupReason = `HARNESS_CLEANUP_FAILED:${detail}`;
    status = 'fail';
    reason = reason === null ? cleanupReason : `${reason}; ${cleanupReason}`;
    failureCode ??= 'HARNESS_CLEANUP_FAILED';
  }
  return finalizeCandidateTransaction({
    ...input,
    status,
    reason,
    failureCode,
    repairTransitions,
  });
}

function finalizeCandidateTransaction(input: FinishCandidateTransactionInput & Readonly<{
  status: CandidateTransactionResult['status'];
  reason: string | null;
  failureCode: ReceiptFailureCode | null;
  repairTransitions: readonly CandidateRepairTransition[];
}>): CandidateTransactionResult {
  const cancelled = input.status === 'cancelled';
  const receipt = input.receipts.append({
    schemaVersion: 3,
    runId: input.context.runId,
    taskId: input.context.taskId,
    step: 'candidate-transaction',
    status: input.status,
    failureCode: input.failureCode,
    authority: DEVELOPMENT_AUTHORITY,
    issuedAt: input.now(),
    identities: {
      controller: input.context.identities.controller,
      baseline: input.evidence.prepared.baseline,
      evaluator: input.evidence.prepared.evaluator,
      candidate: input.evidence.admission?.candidate ?? input.evidence.prepared.candidate,
    },
    protectedInputs: input.evidence.prepared.protectedInputs,
    route: input.context.route,
    hosts: input.context.hosts,
    admittedPaths: input.evidence.admission?.admittedPaths ?? [],
    patchDigest: input.evidence.admission?.patchDigest ?? null,
    patchDigests: input.evidence.patchDigests,
    toolVersions: input.context.toolVersions,
    commands: input.evidence.commands,
    artifactDigests: input.evidence.artifacts,
    verifierDigests: input.evidence.verifiers,
    critiqueDigests: input.evidence.critiques,
    reviewDigests: input.evidence.reviews,
    recovery: {
      retryCount: input.evidence.runtime.retryCount,
      breakerState: input.evidence.runtime.breakerState,
      cancelled,
      repairCount: input.evidence.repairCount,
    },
    coordination: input.evidence.coordination,
  });
  return deepFreeze({
    status: input.status,
    reason: input.reason,
    repairCount: input.evidence.repairCount,
    finalPatch: input.status === 'pass' ? input.finalPatch : null,
    receipt,
    repairTransitions: input.repairTransitions,
  });
}
