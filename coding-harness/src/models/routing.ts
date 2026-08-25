// SPDX-License-Identifier: MIT

import {
  AgentPool,
  type AgentSpec,
} from '@metaharness/harness';
import { Router } from '@metaharness/router';
import type { NativeHost } from './types.js';

export type ModelStepKind =
  | 'architecture'
  | 'implementation'
  | 'repair'
  | 'review'
  | 'evolution-reflection';

const MODEL_STEP_KINDS = new Set<string>([
  'architecture',
  'implementation',
  'repair',
  'review',
  'evolution-reflection',
]);

export interface NativeModelCandidate extends AgentSpec {
  readonly host: NativeHost;
}

export interface RoutingTask {
  readonly id: string;
  readonly digest: string;
  readonly prompt: string;
  readonly tags: readonly string[];
  readonly difficulty: number;
}

export interface TaskEmbedder {
  readonly dimensions: number;
  embed(text: string): readonly number[];
}

export interface RoutingObservation {
  readonly runId: string;
  readonly taskDigest: string;
  readonly stepKind: ModelStepKind;
  readonly candidateId: string;
  readonly embedding: readonly number[];
  readonly predictedQuality: number;
  readonly realizedQuality: number;
  readonly accepted: boolean;
  readonly latencyMs: number;
  readonly verifiedAt: string;
}

export interface DeterministicRoutingOutcome {
  readonly source: 'deterministic-verifier';
  readonly quality: number;
  readonly accepted: boolean;
  readonly infrastructureFailure: boolean;
  readonly latencyMs: number;
}

export interface RoutingDecision {
  readonly runId: string;
  readonly taskDigest: string;
  readonly stepKind: ModelStepKind;
  readonly candidateId: string;
  readonly predictedQuality: number;
  readonly mode: 'cold-start' | 'learned-quality-first';
  readonly embedding: readonly number[];
  readonly historyEpoch: number;
  readonly subscriptionCostUsd: 0;
}

export interface RoutingHistorySnapshot {
  readonly epoch: number;
  readonly observations: readonly RoutingObservation[];
}

export class VerifiedRoutingHistory {
  #observations: RoutingObservation[];
  #epoch: number;

  constructor(seed: readonly RoutingObservation[] = []) {
    this.#observations = seed.map(captureObservation);
    this.#epoch = this.#observations.length;
  }

  get epoch(): number {
    return this.#epoch;
  }

  snapshot(): RoutingHistorySnapshot {
    return Object.freeze({
      epoch: this.#epoch,
      observations: Object.freeze(this.#observations.map(copyObservation)),
    });
  }

  recordVerified(
    decision: RoutingDecision,
    outcome: DeterministicRoutingOutcome,
    verifiedAt = new Date().toISOString(),
  ): RoutingObservation {
    captureOutcome(outcome);
    const observation = captureObservation({
      runId: decision.runId,
      taskDigest: decision.taskDigest,
      stepKind: decision.stepKind,
      candidateId: decision.candidateId,
      embedding: decision.embedding,
      predictedQuality: decision.predictedQuality,
      realizedQuality: outcome.quality,
      accepted: outcome.accepted,
      latencyMs: outcome.latencyMs,
      verifiedAt,
    });
    this.#observations.push(observation);
    this.#epoch += 1;
    return copyObservation(observation);
  }
}

class FrozenQualityFirstRouter {
  readonly #candidates: readonly NativeModelCandidate[];
  readonly #history: RoutingHistorySnapshot;
  readonly #embedder: TaskEmbedder;

  constructor(input: {
    readonly candidates: readonly NativeModelCandidate[];
    readonly history: RoutingHistorySnapshot;
    readonly embedder: TaskEmbedder;
  }) {
    this.#candidates = input.candidates;
    this.#history = input.history;
    this.#embedder = input.embedder;
  }

  route(input: {
    readonly runId: string;
    readonly task: RoutingTask;
    readonly stepKind: ModelStepKind;
    readonly excludeCandidateIds?: readonly string[];
  }): RoutingDecision {
    const excluded = new Set(input.excludeCandidateIds ?? []);
    const candidates = this.#candidates.filter(
      ({ id, handles }) => !excluded.has(id) && handles.includes(input.stepKind),
    );
    if (candidates.length === 0) {
      throw new Error(`HARNESS_ROUTER_NO_CANDIDATE:${input.stepKind}`);
    }
    const embedding = captureEmbedding(
      this.#embedder.embed(taskRoutingText(input.task, input.stepKind)),
      this.#embedder.dimensions,
    );
    const rows = candidates.map((candidate) => ({
      candidate,
      observations: this.#history.observations.filter(
        ({ candidateId, stepKind }) =>
          candidateId === candidate.id && stepKind === input.stepKind,
      ),
    }));
    const cold = rows.filter(({ observations }) => observations.length === 0);
    if (cold.length > 0) {
      const hostCounts = observationsByHost(rows);
      cold.sort(
        (left, right) =>
          hostCounts[left.candidate.host] - hostCounts[right.candidate.host] ||
          left.observations.length - right.observations.length ||
          left.candidate.host.localeCompare(right.candidate.host) ||
          left.candidate.id.localeCompare(right.candidate.id),
      );
      return decision(input, cold[0]!.candidate.id, embedding, 0, 'cold-start', this.#history.epoch);
    }

    const routerCandidates = rows.map(({ candidate, observations }) => ({
      id: candidate.id,
      costPerMTok: 0,
      examples: observations.map(({ embedding: seen, realizedQuality }) => ({
        embedding: [...seen],
        quality: realizedQuality,
      })),
    }));
    const router = new Router({ candidates: routerCandidates });
    const routed = router.route([...embedding]);
    const statistics = rows.map(({ candidate, observations }, index) => ({
      candidate,
      observations,
      predictedQuality: clamp01(
        router.predict(routerCandidates[index]!, [...embedding]),
      ),
      reliability:
        observations.filter(({ accepted }) => accepted).length / observations.length,
      elapsedMean:
        observations.reduce((sum, { latencyMs }) => sum + latencyMs, 0) /
        observations.length,
    }));
    const tied = statistics
      .filter(
        ({ predictedQuality }) =>
          Math.abs(predictedQuality - routed.predictedQuality) <= Number.EPSILON,
      )
      .sort(
        (left, right) =>
          right.reliability - left.reliability ||
          left.elapsedMean - right.elapsedMean ||
          left.candidate.id.localeCompare(right.candidate.id),
      );
    const selected = tied[0] ?? statistics.find(({ candidate }) => candidate.id === routed.id);
    if (selected === undefined) throw new Error('HARNESS_ROUTER_RESULT_INVALID');
    return decision(
      input,
      selected.candidate.id,
      embedding,
      selected.predictedQuality,
      'learned-quality-first',
      this.#history.epoch,
    );
  }
}

export class PersistentRoutedAgentPool extends AgentPool {
  readonly #runId: string;
  readonly #task: RoutingTask;
  readonly #history: VerifiedRoutingHistory;
  readonly #frozenHistory: RoutingHistorySnapshot;
  readonly #router: FrozenQualityFirstRouter;
  readonly #decisions = new Map<ModelStepKind, RoutingDecision>();

  constructor(input: {
    readonly runId: string;
    readonly task: RoutingTask;
    readonly candidates: readonly NativeModelCandidate[];
    readonly history: VerifiedRoutingHistory;
    readonly embedder: TaskEmbedder;
  }) {
    validatePoolInput(input);
    super([...input.candidates], { rng: () => 0.5 });
    this.#runId = input.runId;
    this.#task = Object.freeze({ ...input.task, tags: Object.freeze([...input.task.tags]) });
    this.#history = input.history;
    this.#frozenHistory = input.history.snapshot();
    this.#router = new FrozenQualityFirstRouter({
      candidates: input.candidates,
      history: this.#frozenHistory,
      embedder: input.embedder,
    });
    for (const observation of this.#frozenHistory.observations) {
      if (this.get(observation.candidateId) !== undefined) {
        super.update(observation.candidateId, observation.realizedQuality);
      }
    }
  }

  override select(kind: string): AgentSpec {
    if (!isModelStepKind(kind)) return super.select(kind);
    const existing = this.#decisions.get(kind);
    const selected = existing ?? this.route(kind);
    if (existing === undefined) this.#decisions.set(kind, selected);
    const agent = this.get(selected.candidateId);
    if (agent === undefined) throw new Error('HARNESS_ROUTED_AGENT_UNAVAILABLE');
    return agent;
  }

  route(
    stepKind: ModelStepKind,
    excludeCandidateIds: readonly string[] = [],
  ): RoutingDecision {
    return this.#router.route({
      runId: this.#runId,
      task: this.#task,
      stepKind,
      excludeCandidateIds,
    });
  }

  recordVerified(
    stepKind: ModelStepKind,
    outcome: DeterministicRoutingOutcome,
  ): RoutingObservation {
    const selected = this.#decisions.get(stepKind);
    if (selected === undefined) {
      throw new Error(`HARNESS_ROUTING_DECISION_ABSENT:${stepKind}`);
    }
    const observation = this.#history.recordVerified(selected, outcome);
    super.update(selected.candidateId, outcome.quality);
    return observation;
  }

  routeSnapshot(): Readonly<{
    historyEpoch: number;
    decisions: Readonly<Partial<Record<ModelStepKind, RoutingDecision>>>;
  }> {
    return Object.freeze({
      historyEpoch: this.#frozenHistory.epoch,
      decisions: Object.freeze(Object.fromEntries(this.#decisions)),
    });
  }

  poolSnapshot(): Readonly<{
    agents: ReturnType<AgentPool['snapshot']>;
    routing: ReturnType<PersistentRoutedAgentPool['routeSnapshot']>;
  }> {
    return Object.freeze({
      agents: Object.freeze(super.snapshot()),
      routing: this.routeSnapshot(),
    });
  }
}

function observationsByHost(
  rows: readonly {
    readonly candidate: NativeModelCandidate;
    readonly observations: readonly RoutingObservation[];
  }[],
): Record<NativeHost, number> {
  const counts: Record<NativeHost, number> = { codex: 0, 'claude-code': 0 };
  for (const row of rows) counts[row.candidate.host] += row.observations.length;
  return counts;
}

function decision(
  input: { readonly runId: string; readonly task: RoutingTask; readonly stepKind: ModelStepKind },
  candidateId: string,
  embedding: readonly number[],
  predictedQuality: number,
  mode: RoutingDecision['mode'],
  historyEpoch: number,
): RoutingDecision {
  return Object.freeze({
    runId: input.runId,
    taskDigest: input.task.digest,
    stepKind: input.stepKind,
    candidateId,
    predictedQuality: clamp01(predictedQuality),
    mode,
    embedding,
    historyEpoch,
    subscriptionCostUsd: 0,
  });
}

function taskRoutingText(task: RoutingTask, stepKind: ModelStepKind): string {
  return [
    stepKind,
    `difficulty:${task.difficulty}`,
    `tags:${[...task.tags].sort().join(',')}`,
    task.prompt,
  ].join('\n');
}

function captureEmbedding(value: readonly number[], dimensions: number): readonly number[] {
  if (
    !Number.isSafeInteger(dimensions) ||
    dimensions < 1 ||
    value.length !== dimensions ||
    value.some((entry) => !Number.isFinite(entry))
  ) {
    throw new Error('HARNESS_ROUTING_EMBEDDING_INVALID');
  }
  return Object.freeze([...value]);
}

function captureObservation(value: RoutingObservation): RoutingObservation {
  if (
    value.runId.length === 0 ||
    value.taskDigest.length === 0 ||
    value.candidateId.length === 0 ||
    !isUnit(value.predictedQuality) ||
    !isUnit(value.realizedQuality) ||
    !Number.isSafeInteger(value.latencyMs) ||
    value.latencyMs < 0 ||
    value.embedding.length === 0 ||
    value.embedding.some((entry) => !Number.isFinite(entry)) ||
    Number.isNaN(Date.parse(value.verifiedAt))
  ) {
    throw new Error('HARNESS_ROUTING_OBSERVATION_INVALID');
  }
  return copyObservation(value);
}

function copyObservation(value: RoutingObservation): RoutingObservation {
  return Object.freeze({ ...value, embedding: Object.freeze([...value.embedding]) });
}

function captureOutcome(value: DeterministicRoutingOutcome): void {
  if (value.source !== 'deterministic-verifier') {
    throw new Error('HARNESS_ROUTING_OUTCOME_SOURCE_INVALID');
  }
  if (value.infrastructureFailure) {
    throw new Error('HARNESS_ROUTING_INFRASTRUCTURE_OUTCOME');
  }
  if (!isUnit(value.quality) || !Number.isSafeInteger(value.latencyMs) || value.latencyMs < 0) {
    throw new Error('HARNESS_ROUTING_OUTCOME_INVALID');
  }
}

function validatePoolInput(input: {
  readonly runId: string;
  readonly task: RoutingTask;
  readonly candidates: readonly NativeModelCandidate[];
  readonly embedder: TaskEmbedder;
}): void {
  if (
    input.runId.length === 0 ||
    input.task.id.length === 0 ||
    input.task.digest.length === 0 ||
    input.task.prompt.length === 0 ||
    !isUnit(input.task.difficulty) ||
    !Number.isSafeInteger(input.embedder.dimensions) ||
    input.embedder.dimensions < 1 ||
    input.candidates.length < 1
  ) {
    throw new Error('HARNESS_ROUTED_POOL_INPUT_INVALID');
  }
  const ids = new Set<string>();
  for (const candidate of input.candidates) {
    if (candidate.id.length === 0 || ids.has(candidate.id) || candidate.handles.length === 0) {
      throw new Error('HARNESS_MODEL_CANDIDATE_INVALID');
    }
    ids.add(candidate.id);
  }
}

function isModelStepKind(value: string): value is ModelStepKind {
  return MODEL_STEP_KINDS.has(value);
}

function isUnit(value: number): boolean {
  return Number.isFinite(value) && value >= 0 && value <= 1;
}

function clamp01(value: number): number {
  return Math.max(0, Math.min(1, value));
}
