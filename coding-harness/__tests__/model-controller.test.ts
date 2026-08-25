// SPDX-License-Identifier: MIT

import { VerifierRegistry, predicateVerifier } from '@metaharness/harness';
import { describe, expect, it } from 'vitest';
import { NativeRepositoryModelController } from '../src/model-controller.js';
import type { ModelContextProvider } from '../src/model-context.js';
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

const SEALED_EVALUATOR_FIXTURE = 'SEALED_EVALUATOR_FIXTURE_MUST_NOT_LEAK';
const DECLARED_SOURCE = 'fn checked_bind() { /* DECLARED_IMPLEMENTATION_SOURCE */ }\n';
const ADMITTED_SOURCE = 'fn checked_bind() { /* CURRENT_ADMITTED_SOURCE */ }\n';
const ADMITTED_DIFF = [
  'diff --git a/src/a.rs b/src/a.rs',
  '--- a/src/a.rs',
  '+++ b/src/a.rs',
  '@@ -1 +1 @@',
  '-fn checked_bind() {}',
  '+fn checked_bind() { /* CURRENT_ADMITTED_SOURCE */ }',
  '',
].join('\n');

const contextProvider: ModelContextProvider = {
  declaredSource: async () => ({
    schemaVersion: 1,
    kind: 'declared-implementation-source',
    headCommit: 'a'.repeat(40),
    indexTree: 'b'.repeat(40),
    files: [{ path: 'src/a.rs', digest: digestValue(DECLARED_SOURCE), content: DECLARED_SOURCE }],
    digest: digestValue(['declared', DECLARED_SOURCE]),
  }),
  admittedSource: async () => ({
    schemaVersion: 1,
    kind: 'admitted-implementation',
    headCommit: 'a'.repeat(40),
    indexTree: 'c'.repeat(40),
    files: [{ path: 'src/a.rs', digest: digestValue(ADMITTED_SOURCE), content: ADMITTED_SOURCE }],
    stagedPaths: ['src/a.rs'],
    stagedDiff: ADMITTED_DIFF,
    stagedDiffDigest: digestValue(ADMITTED_DIFF),
    digest: digestValue(['admitted', ADMITTED_SOURCE, ADMITTED_DIFF]),
  }),
};

function controller(
  events: string[],
  prompts: Array<{ operation: string; prompt: string }> = [],
  override?: (input: { candidate: NativeModelCandidate; operation: string }) => unknown,
): NativeRepositoryModelController {
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
  const invoke = async (input: {
    candidate: NativeModelCandidate;
    operation: string;
    prompt: string;
  }) => {
    events.push(`${input.operation}:${input.candidate.host}`);
    prompts.push({ operation: input.operation, prompt: input.prompt });
    let output: unknown;
    if (override !== undefined) {
      output = override(input);
    } else if (input.operation === 'architecture') {
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
    contextProvider,
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
      contextProvider,
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

  it('labels malformed architecture output without exposing response data', async () => {
    const target = controller([], [], () => ({ proposal: { secret: 'do-not-expose' } }));

    await expect(target.architecture()).rejects.toThrow(
      'HARNESS_NATIVE_ARCHITECTURE_RESPONSE_INVALID',
    );
  });

  it('injects declared source and admitted source plus exact staged diff without evaluator data', async () => {
    const events: string[] = [];
    const prompts: Array<{ operation: string; prompt: string }> = [];
    const target = controller(events, prompts);
    const architecture = await target.architecture();
    const patch = await target.implement(architecture);
    await target.repair(patch, ['independent verifier failed'], 1);
    await target.review('codex', {
      candidate: { commit: 'a'.repeat(40), tree: 'b'.repeat(40) },
      commands: [],
      artifactDigests: { 'build.out': digestValue('artifact') },
    });

    const declaredPrompts = prompts.filter(({ operation }) =>
      operation === 'architecture' || operation === 'implementation');
    expect(declaredPrompts.length).toBeGreaterThanOrEqual(3);
    for (const { prompt } of declaredPrompts) {
      expect(prompt).toContain(DECLARED_SOURCE.trim());
      expect(prompt).not.toContain(ADMITTED_DIFF);
      expect(prompt.indexOf(DECLARED_SOURCE.trim())).toBeLessThan(
        prompt.indexOf(operationInstruction(prompt)),
      );
    }
    for (const { prompt } of prompts.filter(({ operation }) =>
      operation === 'repair' || operation === 'review')) {
      expect(prompt).toContain(ADMITTED_SOURCE.trim());
      expect(prompt).toContain(ADMITTED_DIFF.trim());
    }
    for (const { prompt } of prompts) expect(prompt).not.toContain(SEALED_EVALUATOR_FIXTURE);
  });
});

function operationInstruction(prompt: string): string {
  if (prompt.includes('Return only a unified diff')) return 'Return only a unified diff';
  if (prompt.includes('Repair this architecture')) return 'Repair this architecture';
  return 'Propose a minimal architecture';
}
