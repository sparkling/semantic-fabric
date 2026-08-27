// SPDX-License-Identifier: MIT

import { deepFreeze, SHA256_PATTERN } from './contracts.js';
import type { NativeRuntimeEvidence } from './evidence.js';
import { digestValue, type GitIdentity } from './receipts.js';

export type CandidateRepairPhase = 'pre-admission' | 'post-admission';
export type CandidateRepairTrigger =
  | 'patch-admission'
  | 'admission-validation'
  | 'build'
  | 'verification'
  | 'review'
  | 'final-admission';
export type CandidateBuildDisposition = 'not-started' | 'failed' | 'passed';

export interface CandidateRepairTransitionDraft {
  readonly schemaVersion: 1;
  readonly fromAttempt: number;
  readonly toAttempt: number;
  readonly phase: CandidateRepairPhase;
  readonly trigger: CandidateRepairTrigger;
  readonly buildDisposition: CandidateBuildDisposition;
  readonly sourcePatchDigest: string;
  readonly replacementPatchDigest: string;
  readonly sourceCandidate: GitIdentity;
  readonly repairResetIdentity: GitIdentity | null;
  readonly resetIdentity: GitIdentity | null;
  readonly reasonDigests: readonly string[];
  readonly repairInvocationId: string;
}

export interface CandidateRepairTransition extends Omit<
  CandidateRepairTransitionDraft,
  'resetIdentity' | 'repairInvocationId'
> {
  readonly resetIdentity: GitIdentity;
  readonly nativeInvocation: NativeRuntimeEvidence['invocations'][number];
  readonly digest: string;
}

export function createCandidateRepairTransitionDraft(input: Readonly<{
  fromAttempt: number;
  phase: CandidateRepairPhase;
  trigger: CandidateRepairTrigger;
  sourcePatchDigest: string;
  replacementPatchDigest: string;
  sourceCandidate: GitIdentity;
  repairResetIdentity?: GitIdentity | null;
  reasons: readonly string[];
  repairInvocationId: string;
}>): CandidateRepairTransitionDraft {
  const toAttempt = input.fromAttempt + 1;
  const buildDisposition = dispositionFor(input.trigger);
  assertAttempt(input.fromAttempt, 'repair transition.fromAttempt');
  assertAttempt(toAttempt, 'repair transition.toAttempt');
  assertPhase(input.phase, input.trigger);
  assertDigest(input.sourcePatchDigest, 'repair transition.sourcePatchDigest');
  assertDigest(input.replacementPatchDigest, 'repair transition.replacementPatchDigest');
  if (input.sourcePatchDigest === input.replacementPatchDigest) {
    throw new Error('HARNESS_REPAIR_TRANSITION_PATCH_UNCHANGED');
  }
  assertIdentity(input.sourceCandidate, 'repair transition.sourceCandidate');
  const repairResetIdentity = input.repairResetIdentity ?? null;
  if (input.trigger === 'patch-admission') {
    if (repairResetIdentity === null) {
      throw new Error('HARNESS_REPAIR_TRANSITION_REPAIR_RESET_REQUIRED');
    }
    assertIdentity(repairResetIdentity, 'repair transition.repairResetIdentity');
  } else if (repairResetIdentity !== null) {
    throw new Error('HARNESS_REPAIR_TRANSITION_REPAIR_RESET_UNEXPECTED');
  }
  if (input.reasons.length === 0) throw new Error('HARNESS_REPAIR_TRANSITION_REASONS_REQUIRED');
  if (typeof input.repairInvocationId !== 'string' || input.repairInvocationId.length === 0) {
    throw new Error('HARNESS_REPAIR_TRANSITION_INVOCATION_REQUIRED');
  }
  return deepFreeze({
    schemaVersion: 1,
    fromAttempt: input.fromAttempt,
    toAttempt,
    phase: input.phase,
    trigger: input.trigger,
    buildDisposition,
    sourcePatchDigest: input.sourcePatchDigest,
    replacementPatchDigest: input.replacementPatchDigest,
    sourceCandidate: { ...input.sourceCandidate },
    repairResetIdentity: repairResetIdentity === null ? null : { ...repairResetIdentity },
    resetIdentity: null,
    reasonDigests: input.reasons.map((reason) => digestValue(reason)),
    repairInvocationId: input.repairInvocationId,
  });
}

export function completeCandidateRepairTransitionReset(
  drafts: CandidateRepairTransitionDraft[],
  attempt: number,
  resetIdentity: GitIdentity,
  replacementPatchDigest: string,
): void {
  assertIdentity(resetIdentity, 'repair transition.resetIdentity');
  assertDigest(replacementPatchDigest, 'repair transition replacement patch');
  const matches = drafts
    .map((draft, index) => ({ draft, index }))
    .filter(({ draft }) => draft.toAttempt === attempt && draft.resetIdentity === null);
  if (matches.length === 0) {
    if (attempt === 0 && drafts.length === 0) return;
    throw new Error('HARNESS_REPAIR_TRANSITION_PENDING_MISSING');
  }
  if (matches.length !== 1) throw new Error('HARNESS_REPAIR_TRANSITION_PENDING_DUPLICATE');
  const [{ draft, index }] = matches;
  if (draft.replacementPatchDigest !== replacementPatchDigest) {
    throw new Error('HARNESS_REPAIR_TRANSITION_REPLACEMENT_MISMATCH');
  }
  drafts[index] = deepFreeze({ ...draft, resetIdentity: { ...resetIdentity } });
}

export function sealCandidateRepairTransitions(
  drafts: readonly CandidateRepairTransitionDraft[],
  nativeEvidence: NativeRuntimeEvidence,
): readonly CandidateRepairTransition[] {
  const repairInvocations = new Map(nativeEvidence.invocations
    .filter(({ operation }) => operation === 'repair')
    .map((invocation) => [invocation.invocationId, invocation]));
  const used = new Set<string>();
  const transitions = drafts.map((draft, index) => {
    if (draft.fromAttempt !== index || draft.toAttempt !== index + 1) {
      throw new Error('HARNESS_REPAIR_TRANSITION_SEQUENCE_INVALID');
    }
    if (draft.resetIdentity === null) {
      throw new Error('HARNESS_REPAIR_TRANSITION_RESET_MISSING');
    }
    const nativeInvocation = repairInvocations.get(draft.repairInvocationId);
    if (nativeInvocation === undefined || used.has(nativeInvocation.invocationId)) {
      throw new Error('HARNESS_REPAIR_TRANSITION_NATIVE_INVOCATION_INVALID');
    }
    if (nativeInvocation.candidateTree !== draft.sourceCandidate.tree) {
      throw new Error('HARNESS_REPAIR_TRANSITION_CANDIDATE_MISMATCH');
    }
    used.add(nativeInvocation.invocationId);
    const body = {
      schemaVersion: draft.schemaVersion,
      fromAttempt: draft.fromAttempt,
      toAttempt: draft.toAttempt,
      phase: draft.phase,
      trigger: draft.trigger,
      buildDisposition: draft.buildDisposition,
      sourcePatchDigest: draft.sourcePatchDigest,
      replacementPatchDigest: draft.replacementPatchDigest,
      sourceCandidate: draft.sourceCandidate,
      repairResetIdentity: draft.repairResetIdentity,
      resetIdentity: draft.resetIdentity,
      reasonDigests: draft.reasonDigests,
      nativeInvocation,
    };
    return deepFreeze({ ...body, digest: digestValue(body) });
  });
  if (used.size !== repairInvocations.size) {
    throw new Error('HARNESS_REPAIR_TRANSITION_NATIVE_INVOCATION_UNBOUND');
  }
  return deepFreeze(transitions);
}

function dispositionFor(trigger: CandidateRepairTrigger): CandidateBuildDisposition {
  if (trigger === 'patch-admission' || trigger === 'admission-validation') return 'not-started';
  return trigger === 'build' ? 'failed' : 'passed';
}

function assertPhase(phase: CandidateRepairPhase, trigger: CandidateRepairTrigger): void {
  const expected = trigger === 'patch-admission' ? 'pre-admission' : 'post-admission';
  if (phase !== expected) throw new Error('HARNESS_REPAIR_TRANSITION_PHASE_INVALID');
}

function assertAttempt(value: number, label: string): void {
  if (!Number.isSafeInteger(value) || value < 0 || value > 10) {
    throw new TypeError(`${label} is invalid`);
  }
}

function assertDigest(value: string, label: string): void {
  if (!SHA256_PATTERN.test(value) || value === '0'.repeat(64)) {
    throw new TypeError(`${label} is invalid`);
  }
}

function assertIdentity(value: GitIdentity, label: string): void {
  if (!/^[a-f0-9]{40,64}$/.test(value.commit) || !/^[a-f0-9]{40,64}$/.test(value.tree)) {
    throw new TypeError(`${label} is invalid`);
  }
}
