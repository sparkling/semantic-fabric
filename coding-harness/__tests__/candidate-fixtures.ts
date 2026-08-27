// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { vi } from 'vitest';
import type {
  CandidateOperations,
  CandidateTransactionContext,
} from '../src/candidate.js';
import type { NativeInvocationExpectation } from '../src/candidate-types.js';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import { METAHARNESS_DIAGNOSTICS_PATH } from '../src/metaharness-diagnostics.js';
import {
  PROGRAMME_V5_RUFLO_CLI_IDENTITY,
  PROGRAMME_V5_RUFLO_MCP_IDENTITY,
  programmeV5RufloCaptureBindingDigest,
  programmeV5RufloRequests,
  programmeV5RufloSnapshotDigest,
  type ProgrammeV5RufloEvidence,
} from '../src/programme-v5-ruflo-contract.js';
import { digestValue } from '../src/receipts.js';

export const digest = (character: string) => character.repeat(64);

export const identity = (character: string) => ({
  commit: character.repeat(40),
  tree: character.repeat(40),
});

export function programmeV5RufloFixture(input: Readonly<{
  taskId: string;
  runId: string;
  swarmId: string;
  coordinationTaskId: string;
  routeSnapshotDigest: string;
  hookIds: readonly string[];
  traceIds: readonly string[];
  capturedAt: string;
  captureNonce?: string;
  transactionStartedAt?: string;
}>): ProgrammeV5RufloEvidence {
  const captureNonce = input.captureNonce ?? digest('9');
  const transactionStartedAt = input.transactionStartedAt ?? input.capturedAt;
  const taskStatus = {
    taskId: input.coordinationTaskId, type: 'feature', description: 'Schema-v5 programme fixture',
    status: 'in_progress' as const, progress: 50, priority: 'critical' as const,
    assignedTo: ['codex-root'], tags: ['schema-v5'],
    createdAt: '2026-08-25T11:58:00.000Z', startedAt: '2026-08-25T11:59:00.000Z',
    completedAt: null, result: null,
  };
  const swarmStatus = {
    swarmId: input.swarmId, status: 'running' as const, topology: 'hierarchical' as const,
    maxAgents: 4, agentCount: 0, taskCount: 0,
    config: {
      topology: 'hierarchical' as const, maxAgents: 4, strategy: 'specialized' as const,
      communicationProtocol: 'message-bus' as const, autoScaling: false,
      consensusMechanism: 'raft' as const,
    },
    createdAt: '2026-08-25T11:57:00.000Z', updatedAt: '2026-08-25T12:00:00.000Z',
  };
  const captureBindingDigest = programmeV5RufloCaptureBindingDigest({
    ...input, captureNonce, transactionStartedAt,
  });
  return {
    schemaVersion: 2, source: 'ruflo-coordination-ledger', ...input,
    hookIds: [...input.hookIds], traceIds: [...input.traceIds],
    captureNonce, transactionStartedAt, captureBindingDigest,
    mcp: PROGRAMME_V5_RUFLO_MCP_IDENTITY, cli: PROGRAMME_V5_RUFLO_CLI_IDENTITY,
    ...programmeV5RufloRequests(input.coordinationTaskId, input.swarmId),
    taskStatus, taskStatusDigest: programmeV5RufloSnapshotDigest(taskStatus),
    swarmStatus, swarmStatusDigest: programmeV5RufloSnapshotDigest(swarmStatus),
    providerVariablesStripped: true, authoritative: false,
  };
}

const diagnosticBody = {
  schemaVersion: 1,
  authority: 'development-only-no-promotion',
  source: 'ruflo-metaharness-score-mcp',
  capturedAt: '2026-08-25T15:44:09.009Z',
  implementation: {
    ruflo: '3.38.20',
    claudeFlowCli: '3.34.0',
    metaharnessRange: '~0.3.0',
    metaharness: '0.3.2',
    wrapperDigest: 'e14b64f1bcd51c61d3a33c0f1c6712c248e71726f104cbab1d0e0ffc26d775d5',
    bridgeDigest: '9bd511d2ed8b52d40a911113169f8cb9075a124e0a49b966c3975e6959cd0302',
    scorecardDigest: '89af201762b2e1284ca0715bc77bbb5c76a9703113ac7e139f2be282db423a31',
    analyzerDigest: 'da6050f1db03a5f8a267074b6f8f06f05d7c8c5ea158b391edd9443cff9a55f9',
  },
  targets: [diagnosticTarget('repository', '.', 71), diagnosticTarget(
    'coding-harness', 'coding-harness', 67,
  )],
};

export const diagnosticSnapshot = {
  ...diagnosticBody,
  digest: digestValue(diagnosticBody),
};
export const diagnosticBlob = `${JSON.stringify(diagnosticSnapshot, null, 2)}\n`;
export const diagnosticBlobDigest = createHash('sha256').update(diagnosticBlob).digest('hex');

export const context: CandidateTransactionContext = {
  runId: 'run-candidate-0001',
  taskId: 'bprune_8_20260825',
  authority: 'development-only-no-promotion',
  identities: { controller: identity('0'), baseline: identity('1'), evaluator: identity('2') },
  protectedInputs: Object.fromEntries([
    ...SECURE_HARNESS_CONFIG.requiredProtectedPaths,
    'crates/sf-conformance/tests/issue_8_binding_pruning.rs',
  ].map((path) => [
    path,
    path === METAHARNESS_DIAGNOSTICS_PATH ? diagnosticBlobDigest : digest('a'),
  ])),
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
  toolVersions: {
    bootstrapSource: 'verified-packed-private-runtime',
    bootstrapControllerStoreDigest: digest('1'),
    bootstrapBuildManifestDigest: digest('2'),
    bootstrapRuntimeTreeDigest: digest('3'),
    bootstrapNodeDigest: digest('4'),
    bootstrapGitDigest: digest('5'),
    controllerExecutionDigest: digest('6'),
    controllerBuildManifestDigest: digest('7'),
    controllerRuntimeTreeDigest: digest('8'),
    controllerManifestDigest: digest('a'),
    controllerTaskDigest: digest('a'),
    cargo: 'cargo#sha256:test',
    cargoLlvmCov: 'cargo-llvm-cov#sha256:test',
    node: 'node#sha256:test',
    codex: 'codex 1',
    claude: 'claude 1',
    bwrap: 'bwrap#sha256:test',
    systemdRun: 'systemd-run#sha256:test',
    systemctl: 'systemctl#sha256:test',
    agenticQeMcp: 'agentic-qe#sha256:test',
    agenticQe: '3.13.10#sast-only-flat-v1+lcov-gap',
    rufloHive: 'hive-test-0001',
    rufloConsensus: 'consensus-test-0001',
  },
  requiredQeProfiles: ['lcov-gap', 'sast'],
  rufloEvidence: {
    schemaVersion: 1,
    source: 'ruflo-coordination-ledger',
    taskId: 'bprune_8_20260825',
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
      return identity('2');
    }),
    admitAndApply: vi.fn(async () => {
      events.push('admit');
      const payload = cycle === 1 ? 'patch-one' : 'patch-two';
      return {
        candidate: identity(cycle === 1 ? '3' : '4'),
        patchDigest: createHash('sha256').update(payload, 'utf8').digest('hex'),
        admittedPaths: ['crates/sf-sparql/src/unfold.rs'],
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
    recoveryEvidence: vi.fn(() => ({
      retryCount: 0,
      breakerState: 'closed',
      recoveryEvents: [],
    })),
    runtimeEvidence: vi.fn((expectations: readonly NativeInvocationExpectation[]) => ({
      nativeEvidence: nativeProof(expectations),
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
    patchPayloadSha256: string | null,
  ) => ({
    invocationId, host: name, model, operation, candidateTree,
    environmentDigest: digest('5'), outputDigest: digest('6'), patchPayloadSha256, exitCode: 0,
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
    schemaVersion: 2, source: 'trusted-native-runtime',
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
        expected.operation === 'implementation' || expected.operation === 'repair'
          ? expected.patchPayloadSha256 ?? null
          : null,
      );
    }),
  };
}

function diagnosticTarget(
  target: 'repository' | 'coding-harness',
  repositoryPath: '.' | 'coding-harness',
  harnessFit: number,
) {
  return {
    target, repositoryPath, success: true, degraded: false, exitCode: 0, schema: 1,
    harnessFit, compileConfidence: target === 'repository' ? 100 : 90,
    taskCoverage: 79, toolSafety: 100, memoryUsefulness: target === 'repository' ? 46 : 36,
    scaffoldReady: true, hardConstraintsPassed: 6, hardConstraintsTotal: 6,
    archetype: target === 'repository' ? 'rust-crate-harness' : 'typescript-sdk-harness',
    template: 'vertical:coding', recommendedMode: 'CLI + MCP',
    generatedAt: target === 'repository'
      ? '2026-08-25T15:44:08.681Z' : '2026-08-25T15:44:09.009Z',
    durationMs: target === 'repository' ? 88 : 82,
  };
}
