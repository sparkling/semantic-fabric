// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { vi } from 'vitest';
import type {
  CandidateOperations,
  CandidateTransactionContext,
} from '../src/candidate.js';
import type { NativeInvocationExpectation } from '../src/candidate-types.js';

export const digest = (character: string) => character.repeat(64);

export const identity = (character: string) => ({
  commit: character.repeat(40),
  tree: character.repeat(40),
});

export const context: CandidateTransactionContext = {
  runId: 'run-candidate-0001',
  taskId: 'task-candidate-0001',
  authority: 'development-only-no-promotion',
  identities: { controller: identity('0'), baseline: identity('1'), evaluator: identity('2') },
  protectedInputs: { 'protected.txt': digest('a') },
  route: {
    snapshotDigest: digest('b'),
    frozenAt: '2026-08-25T12:00:00.000Z',
    routerVersion: '@metaharness/router@0.4.0',
  },
  hosts: [
    {
      host: 'codex', model: 'gpt-5', role: 'implementation-review', clientVersion: 'codex 1',
      authClass: 'native-openai-subscription', subscriptionCostUsd: 0,
    },
    {
      host: 'claude-code', model: 'claude-sonnet', role: 'architecture-review', clientVersion: 'claude 1',
      authClass: 'native-anthropic-subscription', subscriptionCostUsd: 0,
    },
  ],
  toolVersions: { git: '2.51.0', cargo: '1.90.0' },
  requiredQeProfiles: ['lcov-gap', 'sast'],
  rufloEvidence: {
    schemaVersion: 1,
    source: 'ruflo-coordination-ledger',
    taskId: 'task-candidate-0001',
    runId: 'run-candidate-0001',
    swarmId: 'swarm-0001',
    coordinationTaskId: 'ruflo-task-0001',
    hookIds: ['hook-route-0001'],
    traceIds: ['trace-route-0001'],
    routeSnapshotDigest: digest('b'),
    authoritative: false,
    capturedAt: '2026-08-25T12:00:00.000Z',
  },
};

export function commandEvidence(
  attempt: number,
  candidateTree: string,
  stage: 'red-baseline' | 'build' | 'mutation' = 'build',
) {
  return {
    stage,
    attempt,
    candidateTree,
    tool: 'cargo',
    executable: 'cargo',
    argv: ['test'],
    cwd: '.',
    exitCode: 0,
    signal: null,
    durationMs: 10,
    stdoutDigest: digest('d'),
    stderrDigest: digest('e'),
    timedOut: false,
    cancelled: false,
    outputLimitExceeded: false,
    spawnErrorDigest: null,
  };
}

export function operations(events: string[]): CandidateOperations {
  let cycle = 0;
  return {
    prepare: vi.fn(async () => {
      events.push('prepare');
      return {
        baseline: identity('1'), evaluator: identity('2'), candidate: identity('2'),
        protectedInputs: { ...context.protectedInputs },
      };
    }),
    preflightEvidence: vi.fn(async (prepared) => ({
      passed: true,
      reasons: [],
      commands: [{ ...commandEvidence(0, prepared.evaluator.tree, 'red-baseline'), exitCode: 101 }],
      digests: { 'red-baseline': digest('b') },
    })),
    architecture: vi.fn(async () => {
      events.push('architecture');
      return {
        value: { invariant: true },
        critiqueDigests: [digest('f')],
        invocations: [
          { invocationId: 'architecture-codex', host: 'codex' as const },
          { invocationId: 'architecture-claude', host: 'claude-code' as const },
        ],
      };
    }),
    implement: vi.fn(async () => {
      events.push('implement');
      return { payload: 'patch-one', authorInvocationId: 'author-0001' };
    }),
    repair: vi.fn(async () => {
      events.push('repair');
      return { payload: 'patch-two', authorInvocationId: 'repair-0001' };
    }),
    resetCandidate: vi.fn(async () => {
      events.push('reset');
      cycle += 1;
    }),
    admitAndApply: vi.fn(async () => {
      events.push('admit');
      const payload = cycle === 1 ? 'patch-one' : 'patch-two';
      return {
        candidate: identity(cycle === 1 ? '3' : '4'),
        patchDigest: createHash('sha256').update(payload, 'utf8').digest('hex'),
        admittedPaths: ['src/file.ts'],
      };
    }),
    validateAdmission: vi.fn(async () => {
      events.push('validate');
      return [];
    }),
    build: vi.fn(async (admission, attempt) => {
      events.push('build');
      return {
        candidate: admission.candidate,
        commands: [commandEvidence(attempt, admission.candidate.tree)],
        artifactDigests: { binary: digest(cycle === 1 ? '3' : '4') },
      };
    }),
    verify: vi.fn(async (stage, build) => {
      events.push(`verify:${stage}`);
      return {
        stage,
        candidate: build.candidate,
        passed: cycle > 1 || stage !== 'independent',
        digest: digest(stage === 'public' ? '5' : stage === 'independent' ? '6' : '7'),
        reasons: cycle === 1 && stage === 'independent' ? ['red oracle'] : [],
      };
    }),
    review: vi.fn(async (host, build) => {
      events.push(`review:${host}`);
      return {
        host,
        invocationId: `review-${host}-${build.candidate.tree.slice(0, 8)}`,
        candidate: build.candidate,
        accepted: true,
        digest: digest(host === 'codex' ? '8' : '9'),
        reasons: [],
      };
    }),
    verifyProtectedInputs: vi.fn(async () => {
      events.push('protected');
      return { allow: true, reasons: ['digests match'] };
    }),
    auditMutableOutputs: vi.fn(async () => {
      events.push('audit');
      return { allow: true, reasons: ['within policy'] };
    }),
    agenticQeEvidence: vi.fn(async (build) => {
      events.push('qe');
      const common = {
        schemaVersion: 1,
        source: 'agentic-qe-local-profile',
        taskId: context.taskId,
        runId: context.runId,
        candidateTree: build.candidate.tree,
        providerVariablesStripped: true,
        authoritative: false,
        capturedAt: '2026-08-25T12:01:00.000Z',
      } as const;
      return [
        { ...common, profile: 'lcov-gap' as const, commandDigest: digest('a'), outputDigest: digest('b') },
        { ...common, profile: 'sast' as const, commandDigest: digest('c'), outputDigest: digest('d') },
      ];
    }),
    mutationEvidence: vi.fn(async (build) => ({
      passed: true,
      reasons: [],
      commands: [{
        ...commandEvidence(build.commands[0].attempt, build.candidate.tree, 'mutation'),
        exitCode: 101,
      }],
      digests: { mutation: digest('c') },
    })),
    runtimeEvidence: vi.fn((expectations: readonly NativeInvocationExpectation[]) => ({
      retryCount: 0,
      breakerState: 'closed',
      nativeEvidence: nativeProof(expectations),
      recoveryEvents: [],
    })),
    cleanup: vi.fn(async () => {
      events.push('cleanup');
    }),
  };
}

function nativeProof(expectations: readonly NativeInvocationExpectation[]) {
  const host = (
    name: 'codex' | 'claude-code',
    model: string,
    authentication: 'chatgpt-subscription' | 'claude-subscription',
    clientVersion: string,
  ) => ({
    host: name, model, authentication, clientVersion,
    executablePath: `/tools/${name}`,
    executableDigest: digest(name === 'codex' ? '1' : '2'),
    preflightDigest: digest(name === 'codex' ? '3' : '4'),
    credentialCapability: 'invocation-private-copy',
    hostCredentialPathMounted: false,
  });
  const invocation = (
    invocationId: string,
    name: 'codex' | 'claude-code',
    model: string,
    operation: 'architecture' | 'implementation' | 'repair' | 'review',
    candidateTree: string,
  ) => ({
    invocationId, host: name, model, operation, candidateTree,
    environmentDigest: digest('5'), outputDigest: digest('6'), exitCode: 0,
    network: {
      enforcement: 'origin-pinned-process-boundary', mechanism: 'test-firewall',
      pinnedOrigins: name === 'codex'
        ? ['https://api.openai.com', 'https://chatgpt.com']
        : ['https://api.anthropic.com', 'https://claude.ai'],
      allowedConnections: 1, deniedConnections: 0, connectDigest: digest('a'),
    },
    filesystem: {
      enforcement: 'os-filesystem-namespace', mechanism: 'test-namespace',
      workspaceRootDigest: digest('7'), mountManifestDigest: digest('8'),
      configurationMaskDigest: digest('a'),
      outputChannelDigest: digest('9'), hostFileConfidentiality: true,
      emptyPrivateHome: true, privateEphemeralHome: true, hostRootMounted: false,
      hostCredentialPathMounted: false, gitMetadataMasked: true,
    },
    resources: {
      enforcement: 'systemd-cgroup-v2', mechanism: 'systemd-transient-service',
      limitsDigest: digest('b'),
    },
  });
  return {
    schemaVersion: 1, source: 'trusted-native-runtime',
    taskId: context.taskId, runId: context.runId,
    hosts: [
      host('codex', 'gpt-5', 'chatgpt-subscription', 'codex 1'),
      host('claude-code', 'claude-sonnet', 'claude-subscription', 'claude 1'),
    ],
    invocations: expectations.map((expected) => {
      const name = expected.host
        ?? (expected.invocationId === 'author-0001' ? 'codex' : 'claude-code');
      return invocation(
        expected.invocationId,
        name,
        name === 'codex' ? 'gpt-5' : 'claude-sonnet',
        expected.operation,
        expected.candidateTree,
      );
    }),
  };
}
