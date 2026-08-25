// SPDX-License-Identifier: MIT

import { describe, expect, it, vi } from 'vitest';
import type { WorkerOutput } from '@metaharness/harness';
import {
  PersistentRoutedAgentPool,
  VerifiedRoutingHistory,
  type NativeModelCandidate,
  type RoutingObservation,
} from '../../src/models/routing.js';

const output: WorkerOutput = {
  output: null,
  quality: 1,
  confidence: 1,
  risk: 0,
  costUsd: 0,
  latencyMs: 1,
};

const candidates: NativeModelCandidate[] = [
  {
    id: 'codex-primary',
    host: 'codex',
    model: 'gpt-5.6',
    handles: ['architecture', 'implementation', 'repair', 'review'],
    run: vi.fn(async () => output),
  },
  {
    id: 'claude-primary',
    host: 'claude-code',
    model: 'claude-opus-4-1',
    handles: ['architecture', 'implementation', 'repair', 'review'],
    run: vi.fn(async () => output),
  },
];

const task = {
  id: 'task-1',
  digest: 'sha256:task',
  prompt: 'implement a deterministic parser',
  tags: ['parser'],
  difficulty: 0.8,
};

const embedder = {
  dimensions: 2,
  embed: () => [1, 0] as const,
};

const observation = (
  candidateId: string,
  quality: number,
  accepted: boolean,
  latencyMs: number,
): RoutingObservation => ({
  runId: 'seed-run',
  taskDigest: 'sha256:seed',
  stepKind: 'architecture',
  candidateId,
  embedding: [1, 0],
  predictedQuality: quality,
  realizedQuality: quality,
  accepted,
  latencyMs,
  verifiedAt: '2026-08-25T00:00:00.000Z',
});

describe('persistent quality-first routed pool', () => {
  it('cold-starts on the least-observed capable host', () => {
    const history = new VerifiedRoutingHistory([
      observation('codex-primary', 0.9, true, 20),
    ]);
    const pool = new PersistentRoutedAgentPool({
      runId: 'run-1',
      task,
      candidates,
      history,
      embedder,
    });

    expect(pool.select('architecture').id).toBe('claude-primary');
    expect(pool.routeSnapshot().decisions.architecture?.mode).toBe('cold-start');
  });

  it('uses real quality predictions, then reliability and elapsed time as ties', () => {
    const history = new VerifiedRoutingHistory([
      observation('codex-primary', 0.8, false, 10),
      observation('claude-primary', 0.8, true, 40),
    ]);
    const pool = new PersistentRoutedAgentPool({
      runId: 'run-2',
      task,
      candidates,
      history,
      embedder,
    });

    expect(pool.select('architecture').id).toBe('claude-primary');
    expect(pool.routeSnapshot().decisions.architecture).toMatchObject({
      mode: 'learned-quality-first',
      predictedQuality: 0.8,
      subscriptionCostUsd: 0,
    });
  });

  it('records only deterministic verifier outcomes and keeps the run snapshot frozen', () => {
    const history = new VerifiedRoutingHistory([
      observation('codex-primary', 0.9, true, 10),
      observation('claude-primary', 0.7, true, 10),
    ]);
    const pool = new PersistentRoutedAgentPool({
      runId: 'run-3',
      task,
      candidates,
      history,
      embedder,
    });
    expect(pool.select('architecture').id).toBe('codex-primary');
    const frozenEpoch = pool.routeSnapshot().historyEpoch;

    expect(() =>
      pool.recordVerified('architecture', {
        source: 'deterministic-verifier',
        quality: 0,
        accepted: false,
        infrastructureFailure: true,
        latencyMs: 30,
      }),
    ).toThrow('HARNESS_ROUTING_INFRASTRUCTURE_OUTCOME');
    expect(history.epoch).toBe(frozenEpoch);

    pool.recordVerified('architecture', {
      source: 'deterministic-verifier',
      quality: 1,
      accepted: true,
      infrastructureFailure: false,
      latencyMs: 30,
    });

    expect(history.epoch).toBe(frozenEpoch + 1);
    expect(pool.routeSnapshot().historyEpoch).toBe(frozenEpoch);
    expect(pool.select('architecture').id).toBe('codex-primary');
    expect(pool.poolSnapshot().agents['codex-primary']).toMatchObject({ pulls: 2 });
  });

  it('preselects and seals every transaction route before model execution', () => {
    const pool = new PersistentRoutedAgentPool({
      runId: 'run-frozen', task, candidates, history: new VerifiedRoutingHistory(), embedder,
    });

    const frozen = pool.freeze(['architecture', 'implementation', 'repair']);

    expect(Object.keys(frozen.decisions).sort()).toEqual([
      'architecture', 'implementation', 'repair',
    ]);
    expect(pool.freeze(['repair', 'architecture', 'implementation'])).toEqual(frozen);
    expect(() => pool.select('review')).toThrow('HARNESS_ROUTING_SNAPSHOT_FROZEN:review');
    expect(() => pool.freeze(['architecture'])).toThrow('HARNESS_ROUTING_SNAPSHOT_ALREADY_FROZEN');
  });
});
