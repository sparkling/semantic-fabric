// SPDX-License-Identifier: MIT

import {
  AlgorithmRouter,
  HarnessKernel,
  PolicyGate,
  allowTools,
  critiqueLoop,
  denyTools,
  weightedMajority,
  type RunResult,
  type Verdict,
  type VerifierRegistry,
} from '@metaharness/harness';
import { digestValue } from './receipts.js';
import type { PersistentRoutedAgentPool } from './models/routing.js';
import type { NativeHost } from './models/types.js';

export interface EngineeringKernelOptions {
  pool: PersistentRoutedAgentPool;
  verifiers: VerifierRegistry;
  retrieveMemory?: (goal: { text: string; intent?: string; context?: Record<string, unknown> }) =>
    Record<string, unknown> | Promise<Record<string, unknown>>;
  updateMemory?: (run: RunResult) => void | Promise<void>;
}

export interface ArchitectureProposal {
  host: NativeHost;
  value: unknown;
  confidence: number;
}

export interface CritiquedArchitecture {
  host: NativeHost;
  value: unknown;
  confidence: number;
  verdict: Verdict;
  attempts: number;
  digest: string;
}

const ENGINEERING_STEPS = [
  'architecture', 'implementation', 'repair', 'review', 'evolution-reflection',
];

export function createEngineeringKernel(options: EngineeringKernelOptions): HarnessKernel {
  const router = new AlgorithmRouter({
    'repository-change': {
      intent: 'repository-change',
      steps: [
        { kind: 'architecture' },
        { kind: 'implementation', deps: ['architecture'] },
        { kind: 'review', deps: ['implementation'] },
      ],
    },
  });
  const policy = new PolicyGate([
    allowTools(ENGINEERING_STEPS, 0.05),
    denyTools(['commit', 'merge', 'push', 'publish', 'deploy', 'release'], 1),
  ], 0.2);
  return new HarnessKernel({
    router,
    pool: options.pool,
    verifiers: options.verifiers,
    policy,
    budget: {
      costUsd: Number.POSITIVE_INFINITY,
      risk: 0.2,
      retries: 1,
      confidence: 0.8,
    },
    breakerThreshold: 2,
    actionFor: (step, agentId) => ({
      tool: step.kind,
      args: { agentId, authority: 'development-only-no-promotion' },
    }),
    retrieveMemory: options.retrieveMemory,
    updateMemory: options.updateMemory,
  });
}

export async function critiqueAndChooseArchitecture(input: {
  proposals: readonly [ArchitectureProposal, ArchitectureProposal];
  verifiers: VerifierRegistry;
  repair: (host: NativeHost, value: unknown, verdict: Verdict) => Promise<unknown> | unknown;
  maxAttempts?: number;
}): Promise<Readonly<{
  winner: unknown;
  winnerDigest: string;
  entries: readonly CritiquedArchitecture[];
}>> {
  const hosts = input.proposals.map(({ host }) => host);
  if (new Set(hosts).size !== 2 || !hosts.includes('codex') || !hosts.includes('claude-code')) {
    throw new Error('HARNESS_ARCHITECTURE_HOSTS_NOT_DISTINCT');
  }
  for (const proposal of input.proposals) assertConfidence(proposal.confidence);
  const entries = await Promise.all(input.proposals.map(async (proposal) => {
    const result = await critiqueLoop(
      input.verifiers,
      proposal.value,
      (value, verdict) => input.repair(proposal.host, value, verdict),
      { maxAttempts: input.maxAttempts ?? 2, kinds: ['architecture'] },
    );
    if (!result.verdict.pass) {
      throw new Error(`HARNESS_ARCHITECTURE_CRITIQUE_FAILED:${proposal.host}`);
    }
    return Object.freeze({
      host: proposal.host,
      value: result.output,
      confidence: proposal.confidence,
      verdict: result.verdict,
      attempts: result.attempts,
      digest: digestValue(result.output),
    });
  }));
  const consensus = weightedMajority(entries.map((entry) => ({
    value: entry.digest,
    weight: entry.confidence * entry.verdict.score,
  })));
  const winner = entries.find(({ digest }) => digest === consensus.winner);
  if (winner === undefined) throw new Error('HARNESS_ARCHITECTURE_CONSENSUS_INVALID');
  return Object.freeze({
    winner: winner.value,
    winnerDigest: winner.digest,
    entries: Object.freeze(entries),
  });
}

function assertConfidence(value: number): void {
  if (!Number.isFinite(value) || value < 0 || value > 1) {
    throw new TypeError('architecture confidence must be between 0 and 1');
  }
}
