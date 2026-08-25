// SPDX-License-Identifier: MIT

import { closeSync, existsSync, openSync } from 'node:fs';
import type { StructuredCommand, TaskContract, HarnessConfig } from './contracts.js';
import {
  DEVELOPMENT_AUTHORITY,
  normalizeWorkspacePath,
  pathsOverlap,
} from './contracts.js';
import {
  CandidateBuildFailure,
  type AcceptanceGateEvidence,
  type ArchitectureEvidence,
  type CandidateBuild,
  type CandidateOperations,
  type CandidateReview,
  type PatchAdmission,
  type PatchSubmission,
  type PreparedCandidate,
  type VerifierEvidence,
  type VerifierStage,
} from './candidate.js';
import { GitWorktreeSet } from './git-worktrees.js';
import type { OfflineProcessIsolator } from './network.js';
import {
  HarnessPolicy,
  auditMutableOutputs,
  captureProtectedInputs,
  listTrackedPaths,
  verifyProtectedInputs,
  type GateDecision,
} from './policy.js';
import { runStructuredProcess } from './process.js';
import { digestValue } from './receipts.js';
import { resolveWorkspacePath, sha256File } from './workspace.js';
import type { NativeHost } from './models/types.js';
import type { NativeInvocationExpectation } from './evidence.js';
import { NativeRuntimeLedger } from './native-runtime-ledger.js';
import type { HostEvidence } from './receipts.js';
import {
  commandEvidence,
  commandPassed,
  type UnboundCommandEvidence,
} from './repository-command-evidence.js';

export interface RepositoryModelController {
  architecture(signal?: AbortSignal): Promise<ArchitectureEvidence>;
  implement(architecture: ArchitectureEvidence, signal?: AbortSignal): Promise<PatchSubmission>;
  repair(
    patch: PatchSubmission,
    reasons: readonly string[],
    repairAttempt: number,
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
  taskForWorkspace: (candidateRoot: string) => TaskContract;
  buildCommands: readonly StructuredCommand[];
  verifierCommands: Readonly<Record<VerifierStage, readonly StructuredCommand[]>>;
  artifactPaths: readonly string[];
  model: RepositoryModelController;
  offlineIsolator: OfflineProcessIsolator;
  offlineEnvironment: Readonly<Record<string, string | undefined>>;
  frozenLockfile?: Readonly<{ sourcePath: string; workspacePath: string; digest: string }>;
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

export class RepositoryCandidateOperations implements CandidateOperations {
  readonly #options: RepositoryOperationsOptions;
  #task: TaskContract | null = null;
  #policy: HarnessPolicy | null = null;
  #protectedInputs: Readonly<Record<string, string>> | null = null;
  #trackedAtStart: readonly string[] | null = null;

  constructor(options: RepositoryOperationsOptions) {
    if (options.buildCommands.length === 0) throw new TypeError('HARNESS_BUILD_COMMANDS_REQUIRED');
    for (const stage of ['public', 'independent', 'regression'] as const) {
      if (options.verifierCommands[stage].length === 0) {
        throw new TypeError(`HARNESS_VERIFIER_COMMANDS_REQUIRED:${stage}`);
      }
    }
    if (options.artifactPaths.length === 0) throw new TypeError('HARNESS_ARTIFACT_PATHS_REQUIRED');
    const artifacts = options.artifactPaths.map((path, index) =>
      normalizeWorkspacePath(path, `artifactPaths[${index}]`));
    if (new Set(artifacts).size !== artifacts.length) {
      throw new TypeError('HARNESS_ARTIFACT_PATHS_DUPLICATE');
    }
    if (this.#hasRustCommands(options) && options.frozenLockfile === undefined) {
      throw new Error('HARNESS_FROZEN_LOCKFILE_REQUIRED');
    }
    if (options.nativeRuntime !== undefined
      && !(options.nativeRuntime.ledger instanceof NativeRuntimeLedger)) {
      throw new Error('HARNESS_NATIVE_TRUSTED_RUNTIME_UNAVAILABLE');
    }
    this.#options = options;
  }

  async prepare(signal?: AbortSignal): Promise<PreparedCandidate> {
    const prepared = await this.#options.worktrees.prepare(
      this.#options.baselineCommit,
      this.#options.evaluatorCommit,
      signal,
    );
    await this.#installFrozenLockfile(signal);
    const task = this.#options.taskForWorkspace(prepared.candidateRoot);
    if (task.workspaceRoot !== prepared.candidateRoot || task.authority !== DEVELOPMENT_AUTHORITY) {
      throw new Error('HARNESS_CANDIDATE_TASK_BINDING_INVALID');
    }
    this.#assertCommandsDeclared(task);
    this.#task = task;
    this.#policy = new HarnessPolicy(task, this.#options.config);
    this.#protectedInputs = await captureProtectedInputs(task, this.#options.config);
    this.#trackedAtStart = await listTrackedPaths(task, this.#options.config);
    for (const path of this.#options.artifactPaths) {
      if (!task.mutablePaths.includes(path)) throw new Error(`HARNESS_ARTIFACT_PATH_NOT_DECLARED:${path}`);
      if (task.protectedPaths.some((protectedPath) => pathsOverlap(path, protectedPath))) {
        throw new Error(`HARNESS_ARTIFACT_PATH_PROTECTED:${path}`);
      }
    }
    const protectedInputs = { ...this.#protectedInputs };
    if (this.#options.frozenLockfile !== undefined) {
      protectedInputs[this.#options.frozenLockfile.workspacePath] =
        this.#options.frozenLockfile.digest;
    }
    return Object.freeze({ ...prepared, protectedInputs });
  }

  async architecture(signal?: AbortSignal): Promise<ArchitectureEvidence> {
    return await this.#options.model.architecture(signal);
  }

  async preflightEvidence(
    prepared: PreparedCandidate,
    signal?: AbortSignal,
  ): Promise<AcceptanceGateEvidence> {
    const evidence = await this.#options.preflightEvidence(prepared, signal);
    this.#verifyFrozenLockfile();
    return evidence;
  }

  async implement(
    architecture: ArchitectureEvidence,
    signal?: AbortSignal,
  ): Promise<PatchSubmission> {
    return await this.#options.model.implement(architecture, signal);
  }

  async repair(
    patch: PatchSubmission,
    reasons: readonly string[],
    repairAttempt: number,
    signal?: AbortSignal,
  ): Promise<PatchSubmission> {
    return await this.#options.model.repair(patch, reasons, repairAttempt, signal);
  }

  async resetCandidate(signal?: AbortSignal): Promise<void> {
    await this.#options.worktrees.resetCandidate(signal);
    await this.#installFrozenLockfile(signal);
  }

  async admitAndApply(patch: PatchSubmission, signal?: AbortSignal): Promise<PatchAdmission> {
    const task = this.#requireTask();
    for (const path of task.mutablePaths) {
      const decision = this.#requirePolicy().evaluate({
        kind: 'write',
        tool: 'apply_patch',
        path,
        origin: null,
        authority: DEVELOPMENT_AUTHORITY,
      });
      if (!decision.allow) throw new Error(`HARNESS_PATCH_POLICY_GATE:${decision.reasons.join('; ')}`);
    }
    const admission = await this.#options.worktrees.admitAndApply(
      patch.payload,
      task.mutablePaths,
      signal,
    );
    const protectedInputs = await this.verifyProtectedInputs();
    if (!protectedInputs.allow) {
      throw new Error(`HARNESS_PROTECTED_INPUT_GATE:${protectedInputs.reasons.join('; ')}`);
    }
    const outputs = await this.auditMutableOutputs();
    if (!outputs.allow) throw new Error(`HARNESS_MUTABLE_OUTPUT_GATE:${outputs.reasons.join('; ')}`);
    return admission;
  }

  async build(
    admission: PatchAdmission,
    attempt: number,
    signal?: AbortSignal,
  ): Promise<CandidateBuild> {
    const artifactPaths = this.#prepareArtifactFiles();
    const outputRoot = this.#options.worktrees.outputRoot('candidate');
    const commands = await this.#runCommands(
      this.#options.buildCommands,
      this.#requireTask().workspaceRoot,
      [...artifactPaths, outputRoot],
      outputRoot,
      signal,
    );
    this.#verifyFrozenLockfile();
    await this.#options.worktrees.assertCandidateSourceStable(this.#options.artifactPaths, signal);
    const candidate = await this.#options.worktrees.candidateIdentity(signal);
    const artifactDigests: Record<string, string> = {};
    for (const path of this.#options.artifactPaths) {
      const absolute = resolveWorkspacePath(this.#requireTask().workspaceRoot, path, {
        requireRegularFile: true,
        rejectHardlinks: true,
      });
      artifactDigests[path] = sha256File(absolute);
    }
    const boundCommands = commands.map((command) => Object.freeze({
      ...command,
      stage: 'build' as const,
      attempt,
      candidateTree: candidate.tree,
    }));
    const build = Object.freeze({ candidate, commands: boundCommands, artifactDigests });
    const failures = boundCommands.filter((command) => !commandPassed(command))
      .map(({ tool, exitCode }) => `${tool} exited ${String(exitCode)}`);
    if (failures.length > 0) throw new CandidateBuildFailure(build, failures);
    return build;
  }

  async verify(
    stage: VerifierStage,
    build: CandidateBuild,
    signal?: AbortSignal,
  ): Promise<VerifierEvidence> {
    const verifierRoot = this.#options.worktrees.verifierRoot(stage);
    const outputRoot = this.#options.worktrees.outputRoot(stage);
    const before = await this.#options.worktrees.verifierIdentity(stage, signal);
    if (before.commit !== build.candidate.commit || before.tree !== build.candidate.tree) {
      throw new Error(`HARNESS_STALE_VERIFIER_IDENTITY:${stage}`);
    }
    const commands = await this.#runCommands(
      this.#options.verifierCommands[stage],
      verifierRoot,
      [outputRoot],
      outputRoot,
      signal,
    );
    this.#verifyFrozenLockfile();
    await this.#options.worktrees.assertVerifierSourceStable(stage, signal);
    const candidate = await this.#options.worktrees.verifierIdentity(stage, signal);
    const passed = commands.every(commandPassed);
    return Object.freeze({
      stage,
      candidate,
      passed,
      digest: digestValue({ stage, candidate, commands }),
      reasons: passed ? [] : commands.filter((command) => !commandPassed(command))
        .map(({ tool, exitCode }) => `${tool} exited ${String(exitCode)}`),
    });
  }

  async review(
    host: NativeHost,
    build: CandidateBuild,
    signal?: AbortSignal,
  ): Promise<CandidateReview> {
    this.#assertBuildArtifacts(build);
    const review = await this.#options.model.review(host, build, signal);
    this.#assertBuildArtifacts(build);
    this.#verifyFrozenLockfile();
    await this.#options.worktrees.assertCandidateSourceStable(this.#options.artifactPaths, signal);
    return review;
  }

  async agenticQeEvidence(
    build: CandidateBuild,
    signal?: AbortSignal,
  ): Promise<readonly unknown[]> {
    this.#assertBuildArtifacts(build);
    const evidence = await this.#options.agenticQeEvidence(build, signal);
    this.#assertBuildArtifacts(build);
    this.#verifyFrozenLockfile();
    await this.#options.worktrees.assertCandidateSourceStable(this.#options.artifactPaths, signal);
    return evidence;
  }

  async mutationEvidence(
    build: CandidateBuild,
    signal?: AbortSignal,
  ): Promise<AcceptanceGateEvidence> {
    this.#assertBuildArtifacts(build);
    const evidence = await this.#options.mutationEvidence(build, signal);
    this.#assertBuildArtifacts(build);
    this.#verifyFrozenLockfile();
    for (const stage of ['public', 'independent', 'regression'] as const) {
      await this.#options.worktrees.assertVerifierSourceStable(stage, signal);
    }
    return evidence;
  }

  async cleanup(): Promise<void> {
    await this.#options.worktrees.dispose();
  }

  runtimeEvidence(expectations: readonly NativeInvocationExpectation[]) {
    const recovery = this.#options.model.recoveryEvidence();
    if (this.#options.nativeRuntime === undefined) {
      throw new Error('HARNESS_NATIVE_TRUSTED_RUNTIME_UNAVAILABLE');
    }
    const nativeEvidence = this.#options.nativeRuntime.ledger.seal({
      taskId: this.#options.nativeRuntime.taskId,
      runId: this.#options.nativeRuntime.runId,
      hosts: this.#options.nativeRuntime.hosts,
      expectations,
    });
    return Object.freeze({
      retryCount: recovery.retryCount,
      breakerState: recovery.breakerState,
      nativeEvidence,
      recoveryEvents: Object.freeze([...recovery.events]),
    });
  }

  async verifyProtectedInputs(): Promise<GateDecision> {
    return await verifyProtectedInputs(
      this.#requireTask(),
      this.#options.config,
      this.#protectedInputs ?? {},
    );
  }

  async auditMutableOutputs(): Promise<GateDecision> {
    return auditMutableOutputs(
      this.#requireTask(),
      this.#options.config,
      this.#trackedAtStart ?? [],
    );
  }

  async #runCommands(
    commands: readonly StructuredCommand[],
    workspaceRoot: string,
    writablePaths: readonly string[],
    outputRoot: string,
    signal?: AbortSignal,
  ): Promise<UnboundCommandEvidence[]> {
    const evidence: UnboundCommandEvidence[] = [];
    for (const command of commands) {
      const decision = this.#requirePolicy().evaluate({
        kind: 'execute',
        tool: command.tool,
        path: null,
        origin: null,
        authority: DEVELOPMENT_AUTHORITY,
      });
      if (!decision.allow) throw new Error(`HARNESS_COMMAND_POLICY_GATE:${decision.reasons.join('; ')}`);
      const executed = command.tool === 'cargo' || command.tool === 'rustc'
        ? {
            ...command,
            env: {
              ...command.env,
              HOME: '/home/harness',
              CARGO_HOME: '/cargo-home',
              CARGO_NET_OFFLINE: 'true',
              CARGO_INCREMENTAL: '0',
              CARGO_TARGET_DIR: outputRoot,
            },
          }
        : command;
      const result = await runStructuredProcess(executed, {
        workspaceRoot,
        config: this.#options.config,
        declaredTools: this.#requireTask().tools,
        sourceEnvironment: this.#options.offlineEnvironment,
        signal,
        boundary: {
          kind: 'offline-candidate',
          isolator: this.#options.offlineIsolator,
          writablePaths,
        },
      });
      evidence.push(commandEvidence(executed, result));
    }
    return evidence;
  }

  #assertCommandsDeclared(task: TaskContract): void {
    const declared = new Set(task.commands.map(stableCommand));
    const configured = [
      ...this.#options.buildCommands,
      ...Object.values(this.#options.verifierCommands).flat(),
    ];
    if (configured.some((command) => !declared.has(stableCommand(command)))) {
      throw new Error('HARNESS_OPERATION_COMMAND_NOT_DECLARED');
    }
  }

  #prepareArtifactFiles(): string[] {
    return this.#options.artifactPaths.map((path) => {
      const absolute = resolveWorkspacePath(this.#requireTask().workspaceRoot, path, {
        allowMissingLeaf: true,
      });
      if (existsSync(absolute)) {
        resolveWorkspacePath(this.#requireTask().workspaceRoot, path, {
          requireRegularFile: true,
          rejectHardlinks: true,
        });
      } else {
        const descriptor = openSync(absolute, 'wx', 0o600);
        closeSync(descriptor);
      }
      return absolute;
    });
  }

  #assertBuildArtifacts(build: CandidateBuild): void {
    for (const path of this.#options.artifactPaths) {
      const absolute = resolveWorkspacePath(this.#requireTask().workspaceRoot, path, {
        requireRegularFile: true,
        rejectHardlinks: true,
      });
      if (sha256File(absolute) !== build.artifactDigests[path]) {
        throw new Error(`HARNESS_BUILD_ARTIFACT_CHANGED:${path}`);
      }
    }
  }

  async #installFrozenLockfile(signal?: AbortSignal): Promise<void> {
    const lockfile = this.#options.frozenLockfile;
    if (lockfile === undefined) return;
    await this.#options.worktrees.installFrozenOverlay(
      lockfile.sourcePath,
      lockfile.workspacePath,
      lockfile.digest,
      signal,
    );
  }

  #verifyFrozenLockfile(): void {
    const lockfile = this.#options.frozenLockfile;
    if (lockfile === undefined) return;
    this.#options.worktrees.verifyFrozenOverlay(lockfile.workspacePath, lockfile.digest);
  }

  #hasRustCommands(options: RepositoryOperationsOptions): boolean {
    return [
      ...options.buildCommands,
      ...Object.values(options.verifierCommands).flat(),
    ].some(({ tool }) => tool === 'cargo' || tool === 'rustc');
  }

  #requireTask(): TaskContract {
    if (this.#task === null) throw new Error('HARNESS_OPERATIONS_NOT_PREPARED');
    return this.#task;
  }

  #requirePolicy(): HarnessPolicy {
    if (this.#policy === null) throw new Error('HARNESS_OPERATIONS_NOT_PREPARED');
    return this.#policy;
  }
}

function stableCommand(command: StructuredCommand): string {
  return JSON.stringify(command);
}
