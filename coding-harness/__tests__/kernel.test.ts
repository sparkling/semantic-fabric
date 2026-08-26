// SPDX-License-Identifier: MIT

import {
  VerifierRegistry,
  predicateVerifier,
  type WorkerOutput,
} from '@metaharness/harness';
import { describe, expect, it, vi } from 'vitest';
import {
  critiqueAndChooseArchitecture,
  createEngineeringKernel,
} from '../src/kernel.js';
import {
  PersistentRoutedAgentPool,
  VerifiedRoutingHistory,
  type NativeModelCandidate,
} from '../src/models/routing.js';

const output = (value: unknown): WorkerOutput => ({
  output: value,
  quality: 1,
  confidence: 1,
  risk: 0,
  costUsd: 0,
  latencyMs: 1,
});

const candidate = (
  id: string,
  host: NativeModelCandidate['host'],
): NativeModelCandidate => ({
  id,
  host,
  model: `${id}-model`,
  handles: ['architecture', 'implementation', 'repair', 'review', 'evolution-reflection'],
  run: vi.fn(async ({ step }) => output({ kind: step.kind, valid: true })),
});

const embedder = {
  dimensions: 2,
  embed: () => [0.25, 0.75],
};

describe('engineering HarnessKernel composition', () => {
  it('runs the real algorithm router, persistent pool, policy, verifiers, and memory hooks', async () => {
    const candidates = [candidate('codex-primary', 'codex'), candidate('claude-primary', 'claude-code')];
    const pool = new PersistentRoutedAgentPool({
      runId: 'run-kernel-0001',
      task: {
        id: 'task-kernel-0001',
        digest: 'a'.repeat(64),
        prompt: 'repair a repository issue',
        tags: ['rust', 'semantic'],
        difficulty: 0.8,
      },
      candidates,
      history: new VerifiedRoutingHistory(),
      embedder,
    });
    const verifiers = new VerifierRegistry().register(
      predicateVerifier(
        'structured-output',
        'schema',
        (value) => typeof value === 'object' && value !== null && (value as { valid?: boolean }).valid === true,
      ),
    );
    const retrieveMemory = vi.fn(async () => ({ recalled: 'verified-only' }));
    const updateMemory = vi.fn(async () => undefined);
    const kernel = createEngineeringKernel({ pool, verifiers, retrieveMemory, updateMemory });

    const result = await kernel.run({
      text: 'repair a repository issue',
      intent: 'repository-change',
    }, 'run-kernel-0001');

    expect(result.success).toBe(true);
    expect(result.steps.map(({ step }) => step.kind)).toEqual([
      'architecture',
      'implementation',
      'review',
    ]);
    expect(result.receiptsValid).toBe(true);
    expect(retrieveMemory).toHaveBeenCalledOnce();
    expect(updateMemory).toHaveBeenCalledOnce();
    expect(pool.routeSnapshot().historyEpoch).toBe(0);
  });

  it('does not impose an artificial USD ceiling on native subscription work', async () => {
    const paidTelemetryCandidate = (
      id: string,
      host: NativeModelCandidate['host'],
    ): NativeModelCandidate => ({
      ...candidate(id, host),
      run: vi.fn(async ({ step }) => ({
        ...output({ kind: step.kind, valid: true }),
        costUsd: 1_000_000,
      })),
    });
    const pool = new PersistentRoutedAgentPool({
      runId: 'run-kernel-no-ceiling',
      task: {
        id: 'task-kernel-no-ceiling',
        digest: 'b'.repeat(64),
        prompt: 'verify native subscription accounting does not gate execution',
        tags: ['native-subscription'],
        difficulty: 0.5,
      },
      candidates: [
        paidTelemetryCandidate('codex-no-ceiling', 'codex'),
        paidTelemetryCandidate('claude-no-ceiling', 'claude-code'),
      ],
      history: new VerifiedRoutingHistory(),
      embedder,
    });
    const verifiers = new VerifierRegistry().register(predicateVerifier(
      'structured-output',
      'schema',
      (value) => typeof value === 'object' && value !== null
        && (value as { valid?: boolean }).valid === true,
    ));

    const result = await createEngineeringKernel({ pool, verifiers }).run({
      text: 'verify native subscription accounting',
      intent: 'repository-change',
    }, 'run-kernel-no-ceiling');

    expect(result.success).toBe(true);
    expect(result.steps.every(({ gate }) => gate.costOk)).toBe(true);
  });

  it('critiques distinct-host proposals in parallel and chooses by verified weighted consensus', async () => {
    const verifier = new VerifierRegistry().register({
      id: 'architecture-law',
      kind: 'architecture',
      check: async (value) => ({
        pass: (value as { repaired?: boolean }).repaired === true,
        score: (value as { repaired?: boolean }).repaired === true ? 1 : 0,
        reasons: ['missing invariant'],
      }),
    });
    const repair = vi.fn(async (host: string, value: unknown) => ({
      ...(value as object),
      host,
      repaired: true,
    }));

    const result = await critiqueAndChooseArchitecture({
      proposals: [
        { host: 'codex', value: { design: 'a' }, confidence: 0.9 },
        { host: 'claude-code', value: { design: 'b' }, confidence: 0.7 },
      ],
      verifiers: verifier,
      repair,
      maxAttempts: 1,
    });

    expect(repair).toHaveBeenCalledTimes(2);
    expect(result.winner).toMatchObject({ design: 'a', repaired: true });
    expect(result.entries.map(({ host }) => host).sort()).toEqual(['claude-code', 'codex']);
  });
});
