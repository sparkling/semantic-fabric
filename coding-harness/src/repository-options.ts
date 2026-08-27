// SPDX-License-Identifier: MIT

import {
  normalizeWorkspacePath,
  type StructuredCommand,
  type TaskContract,
  type HarnessConfig,
} from './contracts.js';
import type {
  AcceptanceGateEvidence,
  ArchitectureEvidence,
  CandidateBuild,
  CandidateReview,
  CandidateRepairPhase,
  PatchAdmission,
  PatchSubmission,
  PreparedCandidate,
  VerifierStage,
} from './candidate.js';
import type { GitWorktreeSet } from './git-worktrees.js';
import type { OfflineProcessIsolator } from './network.js';
import type { ProtectedInputBoundary } from './policy.js';
import type { NativeHost } from './models/types.js';
import type { NativeRuntimeLedger } from './native-runtime-ledger.js';
import type { GitIdentity, HostEvidence } from './receipts.js';
import type { AcceptanceTask } from './acceptance-task.js';

const CANDIDATE_EXPECTATION_BRAND = Symbol('candidate-expectation');
const ISSUED_CANDIDATE_EXPECTATIONS = new WeakSet<object>();
const GIT_ID = /^[a-f0-9]{40}$/;
const TASK_ID = /^[A-Za-z0-9_-]{8,128}$/;

type CandidateExpectationBinding = Readonly<{
  taskId: string;
  [CANDIDATE_EXPECTATION_BRAND]: true;
}>;

type CandidateExpectationValue =
  | Readonly<{ mode: 'exact-reference'; candidate: GitIdentity }>
  | Readonly<{ mode: 'verifier-only'; requiredAdmittedPaths: readonly string[] }>;

export type CandidateExpectation = CandidateExpectationBinding & CandidateExpectationValue;

export function candidateExpectationForTask(task: AcceptanceTask): CandidateExpectation {
  let expectation: Readonly<{ taskId: string }> & CandidateExpectationValue;
  if (task.candidateOracle.mode === 'exact-reference') {
    expectation = {
      taskId: task.taskId,
      mode: 'exact-reference',
      candidate: Object.freeze({ ...task.candidateOracle.candidate }),
    };
  } else {
    if (task.schemaVersion !== 3) throw new TypeError('HARNESS_CANDIDATE_EXPECTATION_INVALID');
    expectation = {
      taskId: task.taskId,
      mode: 'verifier-only',
      requiredAdmittedPaths: Object.freeze([...task.evidence.requiredAdmittedPaths]),
    };
  }
  Object.defineProperty(expectation, CANDIDATE_EXPECTATION_BRAND, {
    value: true,
    enumerable: false,
    configurable: false,
    writable: false,
  });
  ISSUED_CANDIDATE_EXPECTATIONS.add(expectation);
  assertCandidateExpectation(expectation);
  return Object.freeze(expectation) as CandidateExpectation;
}

export function assertCandidateExpectation(value: unknown): asserts value is CandidateExpectation {
  if (value === null || typeof value !== 'object') {
    throw new TypeError('HARNESS_CANDIDATE_EXPECTATION_INVALID');
  }
  const input = value as Partial<CandidateExpectation> & Record<PropertyKey, unknown>;
  if (!ISSUED_CANDIDATE_EXPECTATIONS.has(value)
    || input[CANDIDATE_EXPECTATION_BRAND] !== true
    || !TASK_ID.test(String(input.taskId ?? ''))) {
    throw new TypeError('HARNESS_CANDIDATE_EXPECTATION_INVALID');
  }
  if (input.mode === 'verifier-only') {
    if (!Array.isArray(input.requiredAdmittedPaths) || input.requiredAdmittedPaths.length === 0) {
      throw new TypeError('HARNESS_CANDIDATE_EXPECTATION_INVALID');
    }
    const paths = input.requiredAdmittedPaths.map((path, index) =>
      normalizeWorkspacePath(path, `candidateExpectation.requiredAdmittedPaths[${index}]`));
    if (new Set(paths).size !== paths.length) {
      throw new TypeError('HARNESS_CANDIDATE_EXPECTATION_INVALID');
    }
    return;
  }
  if (input.mode !== 'exact-reference' || input.candidate === null
    || typeof input.candidate !== 'object'
    || !GIT_ID.test(String(input.candidate.commit ?? ''))
    || !GIT_ID.test(String(input.candidate.tree ?? ''))) {
    throw new TypeError('HARNESS_CANDIDATE_EXPECTATION_INVALID');
  }
}

export function candidateAdmissionReasons(input: Readonly<{
  expectation: CandidateExpectation;
  current: GitIdentity;
  admission: PatchAdmission;
}>): readonly string[] {
  const reasons: string[] = [];
  if (input.current.commit !== input.admission.candidate.commit
    || input.current.tree !== input.admission.candidate.tree) {
    reasons.push('HARNESS_ADMISSION_WORKTREE_IDENTITY_MISMATCH');
  }
  if (input.expectation.mode === 'exact-reference'
    && input.admission.candidate.tree !== input.expectation.candidate.tree) {
    reasons.push('HARNESS_CANDIDATE_SOURCE_FIX_MISMATCH');
  }
  if (input.expectation.mode === 'verifier-only'
    && JSON.stringify([...input.admission.admittedPaths].sort()) !== JSON.stringify(
      [...input.expectation.requiredAdmittedPaths].sort(),
    )) {
    reasons.push('HARNESS_CANDIDATE_ADMITTED_PATHS_MISMATCH');
  }
  return Object.freeze(reasons);
}

export interface RepositoryModelController {
  architecture(signal?: AbortSignal): Promise<ArchitectureEvidence>;
  implement(architecture: ArchitectureEvidence, signal?: AbortSignal): Promise<PatchSubmission>;
  repair(
    patch: PatchSubmission,
    reasons: readonly string[],
    repairAttempt: number,
    phase: CandidateRepairPhase,
    signal?: AbortSignal,
  ): Promise<PatchSubmission>;
  review(host: NativeHost, build: CandidateBuild, signal?: AbortSignal): Promise<CandidateReview>;
  recoveryEvidence(): Readonly<{
    retryCount: number;
    breakerState: 'closed' | 'open' | 'half-open';
    events: readonly unknown[];
  }>;
}

export interface RepositoryOperationsOptions {
  worktrees: GitWorktreeSet;
  config: HarnessConfig;
  baselineCommit: string;
  evaluatorCommit: string;
  candidateExpectation: CandidateExpectation;
  taskForWorkspace: (candidateRoot: string) => TaskContract;
  buildCommands: readonly StructuredCommand[];
  verifierCommands: Readonly<Record<VerifierStage, readonly StructuredCommand[]>>;
  verifierGeneratedOutputs?: Readonly<Partial<Record<
    VerifierStage,
    readonly VerifierGeneratedOutputSpec[]
  >>>;
  artifactPaths: readonly string[];
  model: RepositoryModelController;
  offlineIsolator: OfflineProcessIsolator;
  offlineEnvironment: Readonly<Record<string, string | undefined>>;
  protectedInputBoundary?: ProtectedInputBoundary;
  frozenLockfile?: Readonly<{ sourcePath: string; workspacePath: string; digest: string }>;
  assertExternalState?: () => void;
  worktreeChildCleanupCallbacks?: readonly (() => Promise<void> | void)[];
  cleanupCallbacks?: readonly (() => Promise<void> | void)[];
  agenticQeEvidence: (
    build: CandidateBuild,
    signal?: AbortSignal,
  ) => Promise<readonly unknown[]>;
  nativeRuntime?: Readonly<{
    ledger: NativeRuntimeLedger;
    taskId: string;
    runId: string;
    hosts: readonly HostEvidence[];
  }>;
  preflightEvidence: (
    prepared: PreparedCandidate,
    signal?: AbortSignal,
  ) => Promise<AcceptanceGateEvidence>;
  mutationEvidence: (
    build: CandidateBuild,
    signal?: AbortSignal,
  ) => Promise<AcceptanceGateEvidence>;
}

export interface VerifierGeneratedOutputSpec {
  readonly evidenceId: string;
  readonly command: StructuredCommand;
  readonly workspacePaths: readonly string[];
}
