// SPDX-License-Identifier: MIT

import { describe, expect, it, vi } from 'vitest';
import type { WorkerOutput } from '@metaharness/harness';
import {
  NativeCancellationError,
  NativeInvocationRecovery,
  TransientNativeHostError,
} from '../../src/models/recovery.js';
import {
  assertIndependentReviewEvidence,
  requireCrossVendorReviewers,
  requireDistinctHostProposal,
} from '../../src/models/review.js';
import type { NativeModelCandidate } from '../../src/models/routing.js';

const workerOutput: WorkerOutput = {
  output: null,
  quality: 1,
  confidence: 1,
  risk: 0,
  costUsd: 0,
  latencyMs: 1,
};

const candidate = (
  id: string,
  host: NativeModelCandidate['host'],
): NativeModelCandidate => ({
  id,
  host,
  model: `${id}-model`,
  handles: ['architecture', 'review'],
  run: vi.fn(async () => workerOutput),
});

describe('native invocation recovery', () => {
  it('allows exactly one same-host retry for a classified transient failure', async () => {
    const recovery = new NativeInvocationRecovery();
    const invoke = vi
      .fn<() => Promise<string>>()
      .mockRejectedValueOnce(new TransientNativeHostError('temporary'))
      .mockResolvedValueOnce('ok');

    await expect(
      recovery.invoke({
        candidate: candidate('codex-primary', 'codex'),
        operation: 'implementation',
        invoke,
      }),
    ).resolves.toBe('ok');

    expect(invoke).toHaveBeenCalledTimes(2);
    expect(recovery.snapshot().events.map(({ host }) => host)).toEqual([
      'codex',
      'codex',
    ]);
    expect(recovery.snapshot().events.map(({ outcome }) => outcome)).toEqual([
      'transient-retry',
      'success',
    ]);
  });

  it('does not retry permanent failures and opens a host breaker', async () => {
    const recovery = new NativeInvocationRecovery({
      breakerThreshold: 2,
      breakerCooldownMs: 60_000,
    });
    const target = candidate('claude-primary', 'claude-code');
    const fail = vi.fn(async () => {
      throw new Error('permanent');
    });

    await expect(
      recovery.invoke({ candidate: target, operation: 'review', invoke: fail }),
    ).rejects.toThrow('permanent');
    await expect(
      recovery.invoke({ candidate: target, operation: 'review', invoke: fail }),
    ).rejects.toThrow('permanent');
    await expect(
      recovery.invoke({ candidate: target, operation: 'review', invoke: fail }),
    ).rejects.toThrow('HARNESS_NATIVE_CIRCUIT_OPEN:claude-code');

    expect(fail).toHaveBeenCalledTimes(2);
  });

  it('propagates cancellation without consuming a breaker failure', async () => {
    const recovery = new NativeInvocationRecovery();
    const controller = new AbortController();
    controller.abort('stop');
    const invoke = vi.fn(async () => 'not reached');

    await expect(
      recovery.invoke({
        candidate: candidate('codex-primary', 'codex'),
        operation: 'repair',
        signal: controller.signal,
        invoke,
      }),
    ).rejects.toBeInstanceOf(NativeCancellationError);
    expect(invoke).not.toHaveBeenCalled();
    expect(recovery.snapshot().breakers.codex).toBe('closed');
  });
});

describe('independent host constraints', () => {
  const codex = candidate('codex-primary', 'codex');
  const claude = candidate('claude-primary', 'claude-code');

  it('requires an opposite-host proposal and reviewers from both vendors', () => {
    expect(requireDistinctHostProposal(codex, [codex, claude]).id).toBe(
      'claude-primary',
    );
    expect(requireCrossVendorReviewers([codex, claude]).map(({ host }) => host)).toEqual([
      'codex',
      'claude-code',
    ]);
  });

  it('fails closed when either host is unavailable', () => {
    expect(() => requireCrossVendorReviewers([codex])).toThrow(
      'HARNESS_REVIEW_HOST_UNAVAILABLE:claude-code',
    );
    expect(() => requireDistinctHostProposal(codex, [codex])).toThrow(
      'HARNESS_DISTINCT_HOST_PROPOSAL_UNAVAILABLE',
    );
  });

  it('requires fresh, unique invocations for both independent reviews', () => {
    expect(() =>
      assertIndependentReviewEvidence('author-1', [
        { host: 'codex', invocationId: 'review-codex', accepted: true },
        { host: 'claude-code', invocationId: 'review-claude', accepted: true },
      ]),
    ).not.toThrow();

    expect(() =>
      assertIndependentReviewEvidence('author-1', [
        { host: 'codex', invocationId: 'author-1', accepted: true },
        { host: 'claude-code', invocationId: 'review-claude', accepted: true },
      ]),
    ).toThrow('HARNESS_REVIEW_NOT_INDEPENDENT');
  });
});
