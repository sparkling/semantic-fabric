// SPDX-License-Identifier: MIT

import { deepFreeze, DEVELOPMENT_AUTHORITY } from './contracts.js';
import {
  digestValue,
  ReceiptChain,
  type CommandEvidence,
  type GitIdentity,
} from './receipts.js';
import { NativeCancellationError } from './models/recovery.js';
import { assertIndependentReviewEvidence } from './models/review.js';
import {
  bindExternalEvidence,
  bindNativeRuntimeEvidence,
  parseRufloEvidence,
  type RufloEvidence,
} from './evidence.js';
import type {
  AcceptanceGateEvidence,
  ArchitectureEvidence,
  CandidateBuild,
  CandidateEvidenceState,
  CandidateOperations,
  CandidateTransactionContext,
  CandidateTransactionResult,
  PatchAdmission,
  PatchSubmission,
  VerifierStage,
} from './candidate-types.js';
export type * from './candidate-types.js';

export class CandidateBuildFailure extends Error {
  readonly build: CandidateBuild;
  readonly reasons: readonly string[];

  constructor(build: CandidateBuild, reasons: readonly string[]) {
    super(`HARNESS_BUILD_COMMAND_FAILED:${reasons.join('; ')}`);
    this.name = 'CandidateBuildFailure';
    this.build = deepFreeze(build);
    this.reasons = Object.freeze([...reasons]);
  }
}

const VERIFIER_STAGES = Object.freeze([
  'public', 'independent', 'regression',
] as const satisfies readonly VerifierStage[]);

export class CandidateTransaction {
  readonly receipts = new ReceiptChain();
  readonly #context: CandidateTransactionContext;
  readonly #operations: CandidateOperations;
  readonly #maxRepairs: number;
  readonly #signal?: AbortSignal;
  readonly #now: () => string;
  readonly #ruflo: RufloEvidence;
  #executed = false;

  constructor(input: {
    context: CandidateTransactionContext;
    operations: CandidateOperations;
    maxRepairs: number;
    signal?: AbortSignal;
    now?: () => string;
  }) {
    if (!Number.isSafeInteger(input.maxRepairs) || input.maxRepairs < 0 || input.maxRepairs > 10) {
      throw new TypeError('maxRepairs must be a safe integer between 0 and 10');
    }
    if (input.context.authority !== DEVELOPMENT_AUTHORITY) {
      throw new Error('HARNESS_TRANSACTION_AUTHORITY_INVALID');
    }
    const hosts = new Set(input.context.hosts.map(({ host }) => host));
    if (input.context.hosts.length !== 2
      || hosts.size !== 2 || !hosts.has('codex') || !hosts.has('claude-code')) {
      throw new Error('HARNESS_TRANSACTION_DUAL_HOST_EVIDENCE_REQUIRED');
    }
    const ruflo = parseRufloEvidence(input.context.rufloEvidence);
    if (ruflo.taskId !== input.context.taskId || ruflo.runId !== input.context.runId) {
      throw new Error('HARNESS_RUFLO_TASK_BINDING_MISMATCH');
    }
    if (ruflo.routeSnapshotDigest !== input.context.route.snapshotDigest) {
      throw new Error('HARNESS_RUFLO_ROUTE_BINDING_MISMATCH');
    }
    assertNonEmptyRecord(input.context.protectedInputs, 'HARNESS_PROTECTED_INPUTS_REQUIRED');
    assertNonEmptyRecord(input.context.toolVersions, 'HARNESS_TOOL_VERSIONS_REQUIRED');
    this.#context = input.context;
    this.#operations = input.operations;
    this.#maxRepairs = input.maxRepairs;
    this.#signal = input.signal;
    this.#now = input.now ?? (() => new Date().toISOString());
    this.#ruflo = ruflo;
  }

  async execute(): Promise<CandidateTransactionResult> {
    if (this.#executed) throw new Error('HARNESS_TRANSACTION_ALREADY_EXECUTED');
    this.#executed = true;
    const evidence: CandidateEvidenceState = {
      prepared: {
        baseline: this.#context.identities.baseline,
        evaluator: this.#context.identities.evaluator,
        candidate: this.#context.identities.evaluator,
        protectedInputs: { ...this.#context.protectedInputs },
      },
      critiques: [],
      commands: [],
      artifacts: {},
      verifiers: {},
      reviews: [],
      admission: null,
      patchDigests: [],
      coordination: {
        swarmId: this.#ruflo.swarmId,
        taskId: this.#ruflo.coordinationTaskId,
        hookIds: [...this.#ruflo.hookIds],
        traceIds: [...this.#ruflo.traceIds],
        agenticQeEvidenceDigests: [],
        nativeEvidenceDigests: [],
        nativeRuntimeEvidenceDigest: null,
      },
      repairCount: 0,
      runtime: { retryCount: 0, breakerState: 'closed' },
      nativeInvocations: [],
    };

    try {
      this.#assertActive();
      const prepared = await this.#operations.prepare(this.#signal);
      assertSameIdentity(
        prepared.baseline,
        this.#context.identities.baseline,
        'HARNESS_BASELINE_IDENTITY_MISMATCH',
      );
      assertSameIdentity(
        prepared.evaluator,
        this.#context.identities.evaluator,
        'HARNESS_EVALUATOR_IDENTITY_MISMATCH',
      );
      assertSameIdentity(prepared.candidate, prepared.evaluator, 'HARNESS_CANDIDATE_NOT_FROZEN_EVALUATOR');
      assertProtectedInputSet(prepared.protectedInputs, this.#context.protectedInputs);
      evidence.prepared = prepared;
      this.#recordGateEvidence(
        await this.#operations.preflightEvidence(prepared, this.#signal),
        prepared.evaluator.tree,
        'red-baseline',
        0,
        evidence,
      );
      const architecture = await this.#operations.architecture(this.#signal);
      assertDualHostArchitecture(architecture);
      evidence.critiques = [...architecture.critiqueDigests];
      evidence.nativeInvocations.push(...architecture.invocations.map(({ invocationId, host }) => ({
        invocationId,
        host,
        operation: 'architecture' as const,
        candidateTree: prepared.evaluator.tree,
      })));
      this.#assertActive();
      let patch = await this.#operations.implement(architecture, this.#signal);
      evidence.nativeInvocations.push({
        invocationId: patch.authorInvocationId,
        operation: 'implementation',
        candidateTree: prepared.evaluator.tree,
      });

      while (true) {
        this.#assertActive();
        await this.#operations.resetCandidate(this.#signal);
        evidence.admission = await this.#operations.admitAndApply(patch, this.#signal);
        evidence.patchDigests.push(evidence.admission.patchDigest);
        if (evidence.admission.candidate.tree === evidence.prepared.evaluator.tree) {
          throw new Error('HARNESS_CANDIDATE_TREE_UNCHANGED');
        }
        let build: CandidateBuild;
        try {
          build = await this.#operations.build(
            evidence.admission,
            evidence.repairCount,
            this.#signal,
          );
        } catch (error) {
          if (!(error instanceof CandidateBuildFailure)) throw error;
          this.#recordBuild(error.build, evidence.admission, evidence);
          patch = await this.#repairOrFail(patch, error.reasons, evidence);
          continue;
        }
        this.#recordBuild(build, evidence.admission, evidence);
        const attempt = evidence.repairCount;
        const artifactPrefix = `attempt-${attempt}:`;

        const verifiers = await Promise.all(VERIFIER_STAGES.map(
          (stage) => this.#operations.verify(stage, build, this.#signal),
        ));
        for (const [index, verifier] of verifiers.entries()) {
          const requestedStage = VERIFIER_STAGES[index];
          if (verifier.stage !== requestedStage) throw new Error('HARNESS_VERIFIER_STAGE_MISMATCH');
          assertSameIdentity(verifier.candidate, build.candidate, 'HARNESS_STALE_VERIFIER_IDENTITY');
          evidence.verifiers[`${artifactPrefix}${verifier.stage}`] = verifier.digest;
        }
        const verifierFailures = verifiers.filter(({ passed }) => !passed)
          .flatMap(({ stage, reasons }) => reasons.length === 0
            ? [`${stage}: HARNESS_VERIFIER_REJECTED_WITHOUT_REASON`]
            : reasons.map((reason) => `${stage}: ${reason}`));
        if (verifierFailures.length > 0) {
          patch = await this.#repairOrFail(patch, verifierFailures, evidence);
          continue;
        }

        this.#recordGateEvidence(
          await this.#operations.mutationEvidence(build, this.#signal),
          build.candidate.tree,
          'mutation',
          evidence.repairCount,
          evidence,
        );

        const reviews = await Promise.all((['codex', 'claude-code'] as const).map(
          (host) => this.#operations.review(host, build, this.#signal),
        ));
        assertIndependentReviewEvidence(patch.authorInvocationId, reviews);
        for (const review of reviews) {
          assertSameIdentity(review.candidate, build.candidate, 'HARNESS_STALE_REVIEW_IDENTITY');
          evidence.nativeInvocations.push({
            invocationId: review.invocationId,
            host: review.host,
            operation: 'review',
            candidateTree: build.candidate.tree,
          });
        }
        evidence.reviews = reviews.map(({ digest }) => digest);
        const reviewFailures = reviews.filter(({ accepted }) => !accepted)
          .flatMap(({ host, reasons }) => reasons.length === 0
            ? [`${host}: HARNESS_REVIEW_REJECTED_WITHOUT_REASON`]
            : reasons.map((reason) => `${host}: ${reason}`));
        if (reviewFailures.length > 0) {
          patch = await this.#repairOrFail(patch, reviewFailures, evidence);
          continue;
        }

        const [protectedInputs, mutableOutputs] = await Promise.all([
          this.#operations.verifyProtectedInputs(this.#signal),
          this.#operations.auditMutableOutputs(this.#signal),
        ]);
        if (!protectedInputs.allow) {
          throw new Error(`HARNESS_PROTECTED_INPUT_GATE:${protectedInputs.reasons.join('; ')}`);
        }
        if (!mutableOutputs.allow) {
          throw new Error(`HARNESS_MUTABLE_OUTPUT_GATE:${mutableOutputs.reasons.join('; ')}`);
        }
        const external = bindExternalEvidence({
          taskId: this.#context.taskId,
          runId: this.#context.runId,
          candidateTree: build.candidate.tree,
          ruflo: this.#ruflo,
          qe: await this.#operations.agenticQeEvidence(build, this.#signal),
        });
        if (external.qe.length === 0) throw new Error('HARNESS_AGENTIC_QE_EVIDENCE_REQUIRED');
        evidence.coordination.agenticQeEvidenceDigests = [...external.qeDigests];
        return await this.#finish('pass', null, evidence);
      }
    } catch (error) {
      const cancelled = this.#isCancellation(error);
      return await this.#finish(
        cancelled ? 'cancelled' : 'fail',
        error instanceof Error ? error.message : String(error),
        evidence,
      );
    }
  }

  async #finish(
    requestedStatus: CandidateTransactionResult['status'],
    requestedReason: string | null,
    evidence: CandidateEvidenceState,
  ): Promise<CandidateTransactionResult> {
    let status = requestedStatus;
    let reason = requestedReason;
    try {
      const runtime = this.#operations.runtimeEvidence();
      evidence.runtime = {
        retryCount: runtime.retryCount,
        breakerState: runtime.breakerState,
      };
      if (requestedStatus === 'pass') {
        const native = bindNativeRuntimeEvidence({
          value: runtime.nativeEvidence,
          taskId: this.#context.taskId,
          runId: this.#context.runId,
          hosts: this.#context.hosts,
          expectations: evidence.nativeInvocations,
        });
        evidence.coordination.nativeEvidenceDigests = [
          ...native.hosts.map(digestValue),
          ...native.invocations.map(digestValue),
        ];
        evidence.coordination.nativeRuntimeEvidenceDigest = digestValue(native);
      }
    } catch (error) {
      const runtimeReason = `HARNESS_RUNTIME_EVIDENCE_FAILED:${error instanceof Error ? error.message : String(error)}`;
      status = 'fail';
      reason = reason === null ? runtimeReason : `${reason}; ${runtimeReason}`;
    }
    try {
      await this.#operations.cleanup();
    } catch (error) {
      const cleanupReason = `HARNESS_CLEANUP_FAILED:${error instanceof Error ? error.message : String(error)}`;
      status = 'fail';
      reason = reason === null ? cleanupReason : `${reason}; ${cleanupReason}`;
    }
    return this.#finalize(status, reason, evidence);
  }

  async #repairOrFail(
    patch: PatchSubmission,
    reasons: readonly string[],
    evidence: CandidateEvidenceState,
  ): Promise<PatchSubmission> {
    if (evidence.repairCount >= this.#maxRepairs) {
      throw new Error(`HARNESS_REPAIR_BUDGET_EXHAUSTED:${reasons.join('; ')}`);
    }
    evidence.repairCount += 1;
    this.#assertActive();
    const repaired = await this.#operations.repair(
      patch,
      reasons,
      evidence.repairCount,
      this.#signal,
    );
    evidence.nativeInvocations.push({
      invocationId: repaired.authorInvocationId,
      operation: 'repair',
      candidateTree: evidence.admission?.candidate.tree ?? evidence.prepared.evaluator.tree,
    });
    return repaired;
  }

  #recordBuild(
    build: CandidateBuild,
    admission: PatchAdmission,
    evidence: CandidateEvidenceState,
  ): void {
    assertSameIdentity(build.candidate, admission.candidate, 'HARNESS_STALE_BUILD_IDENTITY');
    if (build.commands.length === 0 || Object.keys(build.artifactDigests).length === 0) {
      throw new Error('HARNESS_BUILD_EVIDENCE_INCOMPLETE');
    }
    for (const command of build.commands) {
      if (command.attempt !== evidence.repairCount || command.candidateTree !== build.candidate.tree) {
        throw new Error('HARNESS_COMMAND_CANDIDATE_BINDING_MISMATCH');
      }
    }
    evidence.commands.push(...build.commands);
    Object.assign(evidence.artifacts, prefixArtifacts(build.artifactDigests, evidence.repairCount));
  }

  #recordGateEvidence(
    gate: AcceptanceGateEvidence,
    tree: string,
    stage: CommandEvidence['stage'],
    attempt: number,
    evidence: CandidateEvidenceState,
  ): void {
    if (gate.commands.length === 0 || Object.keys(gate.digests).length === 0) {
      throw new Error('HARNESS_ACCEPTANCE_EVIDENCE_INCOMPLETE');
    }
    for (const command of gate.commands) {
      if (command.stage !== stage || command.attempt !== attempt || command.candidateTree !== tree) {
        throw new Error('HARNESS_ACCEPTANCE_COMMAND_BINDING_MISMATCH');
      }
    }
    for (const [name, digest] of Object.entries(gate.digests)) {
      if (name in evidence.verifiers) throw new Error('HARNESS_ACCEPTANCE_DIGEST_COLLISION');
      evidence.verifiers[name] = digest;
    }
    evidence.commands.push(...gate.commands);
    if (!gate.passed) {
      const reasons = gate.reasons.length === 0
        ? ['HARNESS_ACCEPTANCE_GATE_REJECTED_WITHOUT_REASON']
        : gate.reasons;
      throw new Error(`HARNESS_ACCEPTANCE_GATE_FAILED:${reasons.join('; ')}`);
    }
  }

  #finalize(
    status: CandidateTransactionResult['status'],
    reason: string | null,
    evidence: CandidateEvidenceState,
  ): CandidateTransactionResult {
    const cancelled = status === 'cancelled';
    const receipt = this.receipts.append({
      schemaVersion: 1,
      runId: this.#context.runId,
      taskId: this.#context.taskId,
      step: 'candidate-transaction',
      status,
      authority: DEVELOPMENT_AUTHORITY,
      issuedAt: this.#now(),
      identities: {
        baseline: evidence.prepared.baseline,
        evaluator: evidence.prepared.evaluator,
        candidate: evidence.admission?.candidate ?? evidence.prepared.candidate,
      },
      protectedInputs: evidence.prepared.protectedInputs,
      route: this.#context.route,
      hosts: this.#context.hosts,
      admittedPaths: evidence.admission?.admittedPaths ?? [],
      patchDigest: evidence.admission?.patchDigest ?? null,
      patchDigests: evidence.patchDigests,
      toolVersions: this.#context.toolVersions,
      commands: evidence.commands,
      artifactDigests: evidence.artifacts,
      verifierDigests: evidence.verifiers,
      critiqueDigests: evidence.critiques,
      reviewDigests: evidence.reviews,
      recovery: {
        retryCount: evidence.runtime.retryCount,
        breakerState: evidence.runtime.breakerState,
        cancelled,
        repairCount: evidence.repairCount,
      },
      coordination: evidence.coordination,
    });
    return deepFreeze({ status, reason, repairCount: evidence.repairCount, receipt });
  }

  #assertActive(): void {
    if (this.#signal?.aborted === true) throw new NativeCancellationError();
  }

  #isCancellation(error: unknown): boolean {
    return this.#signal?.aborted === true
      || error instanceof NativeCancellationError
      || (error instanceof Error && error.name === 'AbortError');
  }
}

function prefixArtifacts(
  artifacts: Readonly<Record<string, string>>,
  repairCount: number,
): Record<string, string> {
  return Object.fromEntries(Object.entries(artifacts).map(([name, digest]) => [
    `attempt-${repairCount}:${name}`,
    digest,
  ]));
}

function assertSameIdentity(left: GitIdentity, right: GitIdentity, message: string): void {
  if (left.commit !== right.commit || left.tree !== right.tree) throw new Error(message);
}

function assertNonEmptyRecord(value: Readonly<Record<string, string>>, error: string): void {
  if (Object.keys(value).length === 0) throw new Error(error);
}

function assertProtectedInputSet(
  prepared: Readonly<Record<string, string>>,
  expected: Readonly<Record<string, string>>,
): void {
  assertNonEmptyRecord(prepared, 'HARNESS_PROTECTED_INPUTS_REQUIRED');
  const actualPaths = Object.keys(prepared).sort();
  const expectedPaths = Object.keys(expected).sort();
  if (JSON.stringify(actualPaths) !== JSON.stringify(expectedPaths)
    || actualPaths.some((path) => prepared[path] !== expected[path])) {
    throw new Error('HARNESS_PROTECTED_INPUT_SET_MISMATCH');
  }
}

function assertDualHostArchitecture(architecture: ArchitectureEvidence): void {
  const hosts = new Set(architecture.invocations.map(({ host }) => host));
  const ids = architecture.invocations.map(({ invocationId }) => invocationId);
  if (architecture.invocations.length < 2
    || hosts.size !== 2 || !hosts.has('codex') || !hosts.has('claude-code')
    || new Set(ids).size !== ids.length) {
    throw new Error('HARNESS_NATIVE_DUAL_HOST_ARCHITECTURE_REQUIRED');
  }
}
