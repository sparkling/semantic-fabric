// SPDX-License-Identifier: MIT

import type { StructuredCommand, TaskContract, HarnessConfig } from './contracts.js';
import type {
  AcceptanceGateEvidence,
  ArchitectureEvidence,
  CandidateBuild,
  CandidateReview,
  CandidateRepairPhase,
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
  expectedCandidate: GitIdentity;
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
