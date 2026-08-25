// SPDX-License-Identifier: MIT

import { VerifierRegistry, predicateVerifier } from '@metaharness/harness';
import { describe, expect, it } from 'vitest';
import { NativeRepositoryModelController } from '../src/model-controller.js';
import { NativeInvocationRecovery } from '../src/models/recovery.js';
import {
  PersistentRoutedAgentPool,
  VerifiedRoutingHistory,
  type NativeModelCandidate,
} from '../src/models/routing.js';
import { digestValue } from '../src/receipts.js';

const candidates: readonly NativeModelCandidate[] = [
  {
    id: 'codex-native',
    host: 'codex',
    model: 'gpt-5.6',
    handles: ['architecture', 'implementation', 'repair', 'review'],
    run: async () => ({ output: {}, quality: 1, confidence: 1, risk: 0, costUsd: 0, latencyMs: 1 }),
  },
  {
    id: 'claude-native',
    host: 'claude-code',
    model: 'claude-sonnet-5',
    handles: ['architecture', 'implementation', 'repair', 'review'],
    run: async () => ({ output: {}, quality: 1, confidence: 1, risk: 0, costUsd: 0, latencyMs: 1 }),
  },
];

function controller(events: string[]): NativeRepositoryModelController {
  const pool = new PersistentRoutedAgentPool({
    runId: 'run-model-controller',
    task: {
      id: 'task-model-controller',
      digest: digestValue('task'),
      prompt: 'implement the checked-bind invariant',
      tags: ['sparql'],
      difficulty: 0.8,
    },
    candidates,
    history: new VerifiedRoutingHistory(),
    embedder: { dimensions: 2, embed: () => [0.5, 0.5] },
  });
  const invoke = async (input: { candidate: NativeModelCandidate; operation: string }) => {
    events.push(`${input.operation}:${input.candidate.host}`);
    let output: unknown;
    if (input.operation === 'architecture') {
      output = {
        proposal: { host: input.candidate.host, invariant: 'checked-bind' },
        confidence: input.candidate.host === 'codex' ? 0.9 : 0.8,
      };
    } else if (input.operation === 'review') {
      output = { accepted: true, reasons: [] };
    } else {
      output = {
        patch: 'diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n',
      };
    }
    return {
      invocationId: `${input.operation}-${input.candidate.host}`,
      output,
      outputDigest: digestValue(output),
    };
  };
  const verifiers = new VerifierRegistry().register(
    predicateVerifier('architecture-shape', 'architecture', (value) =>
      typeof value === 'object' && value !== null),
  );
  return new NativeRepositoryModelController({
    pool,
    candidates,
    clients: { codex: { invoke }, 'claude-code': { invoke } },
    architectureVerifiers: verifiers,
    recovery: new NativeInvocationRecovery(),
    taskPrompt: 'Issue #8',
  });
}

describe('native repository model controller', () => {
  it('uses distinct hosts for architecture and returns internally bound reviews', async () => {
    const events: string[] = [];
    const target = controller(events);
    const architecture = await target.architecture();
    const patch = await target.implement(architecture);
    const build = {
      candidate: { commit: 'a'.repeat(40), tree: 'b'.repeat(40) },
      commands: [],
      artifactDigests: { 'build.out': digestValue('artifact') },
    };
    const reviews = await Promise.all([
      target.review('codex', build),
      target.review('claude-code', build),
    ]);

    expect(events.filter((event) => event.startsWith('architecture:')).sort()).toEqual([
      'architecture:claude-code', 'architecture:codex',
    ]);
    expect(patch.payload).toMatch(/^diff --git /);
    expect(reviews.map(({ host }) => host)).toEqual(['codex', 'claude-code']);
    expect(reviews.every(({ candidate }) => candidate.tree === build.candidate.tree)).toBe(true);
  });

  it('rejects a negative native review that omits its reason', async () => {
    const rejectingClient = {
      invoke: async (input: { candidate: NativeModelCandidate; operation: string }) => {
        const output = input.operation === 'review'
          ? { accepted: false, reasons: [] }
          : { proposal: {}, confidence: 1 };
        return {
          invocationId: `internal-${input.operation}-${input.candidate.host}`,
          output,
          outputDigest: digestValue(output),
        };
      },
    };
    const pool = new PersistentRoutedAgentPool({
      runId: 'run-rejection',
      task: {
        id: 'task-rejection', digest: digestValue('task'), prompt: 'review', tags: [], difficulty: 0.1,
      },
      candidates,
      history: new VerifiedRoutingHistory(),
      embedder: { dimensions: 2, embed: () => [0.5, 0.5] },
    });
    const rejecting = new NativeRepositoryModelController({
      pool,
      candidates,
      clients: { codex: rejectingClient, 'claude-code': rejectingClient },
      architectureVerifiers: new VerifierRegistry().register(
        predicateVerifier('shape', 'architecture', () => true),
      ),
      recovery: new NativeInvocationRecovery(),
      taskPrompt: 'Issue #8',
    });
    const build = {
      candidate: { commit: 'a'.repeat(40), tree: 'b'.repeat(40) },
      commands: [],
      artifactDigests: { 'build.out': digestValue('artifact') },
    };

    await expect(rejecting.review('codex', build)).rejects.toThrow(
      'HARNESS_NATIVE_REVIEW_REASON_REQUIRED',
    );
  });
});
