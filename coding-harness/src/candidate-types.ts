// SPDX-License-Identifier: MIT

import type { DEVELOPMENT_AUTHORITY } from './contracts.js';
import type {
  CandidateRepairTransition,
  CandidateRepairTransitionDraft,
  CandidateRepairPhase,
} from './candidate-repair-transition.js';
export type {
  CandidateBuildDisposition,
  CandidateRepairPhase,
  CandidateRepairTransition,
  CandidateRepairTransitionDraft,
  CandidateRepairTrigger,
} from './candidate-repair-transition.js';
import type { AgenticQeProfile, NativeInvocationExpectation } from './evidence.js';
import type { NativeHost } from './models/types.js';
import type { GateDecision } from './policy.js';
import type {
  CommandEvidence,
  GitIdentity,
  HostEvidence,
  Receipt,
  ReceiptDraft,
} from './receipts.js';

export type VerifierStage = 'public' | 'independent' | 'regression';

export interface CandidateTransactionContext {
  runId: string;
  taskId: string;
  authority: typeof DEVELOPMENT_AUTHORITY;
  identities: Pick<ReceiptDraft['identities'], 'controller' | 'baseline' | 'evaluator'>;
  protectedInputs: Record<string, string>;
  route: ReceiptDraft['route'];
  hosts: HostEvidence[];
  toolVersions: Record<string, string>;
  requiredQeProfiles: AgenticQeProfile[];
  rufloEvidence: unknown;
}

export interface PreparedCandidate {
  baseline: GitIdentity;
  evaluator: GitIdentity;
  candidate: GitIdentity;
  protectedInputs: Record<string, string>;
}

export interface ArchitectureEvidence {
  value: unknown;
  critiqueDigests: string[];
  invocations: ReadonlyArray<{ invocationId: string; host: NativeHost }>;
}

export interface PatchSubmission {
  payload: string;
  authorInvocationId: string;
}

export interface PatchAdmission {
  candidate: GitIdentity;
  patchDigest: string;
  admittedPaths: string[];
}

export interface CandidateBuild {
  candidate: GitIdentity;
  commands: CommandEvidence[];
  artifactDigests: Record<string, string>;
}

export interface VerifierEvidence {
  stage: VerifierStage;
  candidate: GitIdentity;
  passed: boolean;
  digest: string;
  reasons: string[];
  generatedOutputDigests?: Readonly<Record<string, string>>;
}

export interface CandidateReview {
  host: NativeHost;
  invocationId: string;
  candidate: GitIdentity;
  accepted: boolean;
  digest: string;
  reasons: string[];
}

export interface CandidateOperations {
  prepare(signal?: AbortSignal): Promise<PreparedCandidate>;
  preflightEvidence(prepared: PreparedCandidate, signal?: AbortSignal): Promise<AcceptanceGateEvidence>;
  architecture(signal?: AbortSignal): Promise<ArchitectureEvidence>;
  implement(architecture: ArchitectureEvidence, signal?: AbortSignal): Promise<PatchSubmission>;
  repair(
    patch: PatchSubmission,
    reasons: readonly string[],
    repairAttempt: number,
    phase: CandidateRepairPhase,
    signal?: AbortSignal,
  ): Promise<PatchSubmission>;
  resetCandidate(signal?: AbortSignal): Promise<GitIdentity>;
  admitAndApply(patch: PatchSubmission, signal?: AbortSignal): Promise<PatchAdmission>;
  validateAdmission(admission: PatchAdmission, signal?: AbortSignal): Promise<readonly string[]>;
  build(admission: PatchAdmission, attempt: number, signal?: AbortSignal): Promise<CandidateBuild>;
  verify(stage: VerifierStage, build: CandidateBuild, signal?: AbortSignal): Promise<VerifierEvidence>;
  review(host: NativeHost, build: CandidateBuild, signal?: AbortSignal): Promise<CandidateReview>;
  verifyProtectedInputs(signal?: AbortSignal): Promise<GateDecision>;
  auditMutableOutputs(signal?: AbortSignal): Promise<GateDecision>;
  agenticQeEvidence(build: CandidateBuild, signal?: AbortSignal): Promise<readonly unknown[]>;
  mutationEvidence(build: CandidateBuild, signal?: AbortSignal): Promise<AcceptanceGateEvidence>;
  recoveryEvidence(): CandidateRecoveryEvidence;
  runtimeEvidence(expectations: readonly NativeInvocationExpectation[]): CandidateRuntimeEvidence;
  cleanup(): Promise<void>;
}

export interface CandidateRuntimeEvidence {
  readonly nativeEvidence: unknown;
}

export interface CandidateRecoveryEvidence {
  readonly retryCount: number;
  readonly breakerState: 'closed' | 'open' | 'half-open';
  readonly recoveryEvents: readonly unknown[];
}

export interface AcceptanceGateEvidence {
  readonly passed: boolean;
  readonly reasons: readonly string[];
  readonly commands: readonly CommandEvidence[];
  readonly digests: Readonly<Record<string, string>>;
}

export interface CandidateTransactionResult {
  status: Receipt['status'];
  reason: string | null;
  repairCount: number;
  finalPatch: string | null;
  receipt: Receipt;
  repairTransitions: readonly CandidateRepairTransition[];
}

export interface CandidateEvidenceState {
  prepared: PreparedCandidate;
  critiques: string[];
  commands: CommandEvidence[];
  artifacts: Record<string, string>;
  verifiers: Record<string, string>;
  reviews: string[];
  admission: PatchAdmission | null;
  patchDigests: string[];
  repairTransitions: CandidateRepairTransitionDraft[];
  coordination: ReceiptDraft['coordination'];
  repairCount: number;
  runtime: Pick<CandidateRecoveryEvidence, 'retryCount' | 'breakerState'>;
  nativeInvocations: NativeInvocationExpectation[];
}
