// SPDX-License-Identifier: MIT
import { createHash } from 'node:crypto';
import { describe, expect, it, vi } from 'vitest';
import {
  CandidateBuildFailure,
  CandidateTransaction,
  type CandidateOperations,
  type CandidateTransactionContext,
} from '../src/candidate.js';
import { NativeCancellationError } from '../src/models/recovery.js';
const digest = (character: string) => character.repeat(64);
const identity = (character: string) => ({
  commit: character.repeat(40),
  tree: character.repeat(40),
});
const context: CandidateTransactionContext = {
  runId: 'run-candidate-0001',
  taskId: 'task-candidate-0001',
  authority: 'development-only-no-promotion',
  identities: { baseline: identity('1'), evaluator: identity('2') },
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

function commandEvidence(
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
function operations(events: string[]): CandidateOperations {
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
        invocationId: `review-${host}`,
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
      return [{
        schemaVersion: 1,
        source: 'agentic-qe-local-profile',
        profile: 'quality-contract',
        taskId: context.taskId,
        runId: context.runId,
        candidateTree: build.candidate.tree,
        commandDigest: digest('a'),
        outputDigest: digest('b'),
        providerVariablesStripped: true,
        authoritative: false,
        capturedAt: '2026-08-25T12:01:00.000Z',
      }];
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
    runtimeEvidence: vi.fn(() => ({
      retryCount: 0,
      breakerState: 'closed',
      nativeEvidence: nativeProof(),
      recoveryEvents: [],
    })),
    cleanup: vi.fn(async () => {
      events.push('cleanup');
    }),
  };
}
function nativeProof() {
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
    invocations: [
      invocation('architecture-codex', 'codex', 'gpt-5', 'architecture', identity('2').tree),
      invocation('architecture-claude', 'claude-code', 'claude-sonnet', 'architecture', identity('2').tree),
      invocation('author-0001', 'codex', 'gpt-5', 'implementation', identity('2').tree),
      invocation('repair-0001', 'claude-code', 'claude-sonnet', 'repair', identity('3').tree),
      invocation('review-codex', 'codex', 'gpt-5', 'review', identity('4').tree),
      invocation('review-claude-code', 'claude-code', 'claude-sonnet', 'review', identity('4').tree),
    ],
  };
}
describe('patched candidate transaction', () => {
  it('re-admits, rebuilds, and reruns all parallel verifiers after repair', async () => {
    const events: string[] = [];
    const transaction = new CandidateTransaction({
      context,
      operations: operations(events),
      maxRepairs: 1,
      now: () => '2026-08-25T12:02:00.000Z',
    });

    const result = await transaction.execute();

    expect(result.status).toBe('pass');
    expect(result.repairCount).toBe(1);
    expect(result.finalPatch).toBe('patch-two');
    expect(events).toEqual([
      'prepare', 'architecture', 'implement',
      'reset', 'admit', 'build',
      'verify:public', 'verify:independent', 'verify:regression', 'repair',
      'reset', 'admit', 'build',
      'verify:public', 'verify:independent', 'verify:regression',
      'review:codex', 'review:claude-code', 'protected', 'audit', 'qe', 'cleanup',
    ]);
    expect(result.receipt.status).toBe('pass');
    expect(result.receipt.recovery.repairCount).toBe(1);
    expect(transaction.receipts.verify()).toEqual({ ok: true });
  });

  it('rejects stale build identities and emits a failure receipt', async () => {
    const events: string[] = [];
    const target = operations(events);
    target.build = vi.fn(async () => ({
      candidate: identity('1'),
      commands: [commandEvidence(0, identity('1').tree)],
      artifactDigests: { binary: digest('3') },
    }));
    const result = await new CandidateTransaction({
      context,
      operations: target,
      maxRepairs: 0,
      now: () => '2026-08-25T12:02:00.000Z',
    }).execute();

    expect(result.status).toBe('fail');
    expect(result.finalPatch).toBeNull();
    expect(result.reason).toMatch(/STALE_BUILD_IDENTITY/);
    expect(result.receipt.status).toBe('fail');
  });

  it('records cancellation even when it occurs before a patch exists', async () => {
    const events: string[] = [];
    const target = operations(events);
    target.implement = vi.fn(async () => {
      throw new NativeCancellationError();
    });
    const result = await new CandidateTransaction({
      context,
      operations: target,
      maxRepairs: 1,
      now: () => '2026-08-25T12:02:00.000Z',
    }).execute();

    expect(result.status).toBe('cancelled');
    expect(result.finalPatch).toBeNull();
    expect(result.receipt.status).toBe('cancelled');
    expect(result.receipt.recovery.cancelled).toBe(true);
  });

  it('emits a cancellation receipt before worktree preparation starts', async () => {
    const events: string[] = [];
    const controller = new AbortController();
    controller.abort('cancel before prepare');
    const target = operations(events);
    const result = await new CandidateTransaction({
      context,
      operations: target,
      maxRepairs: 1,
      signal: controller.signal,
      now: () => '2026-08-25T12:02:00.000Z',
    }).execute();

    expect(result.status).toBe('cancelled');
    expect(result.receipt.identities.candidate).toEqual(context.identities.evaluator);
    expect(target.prepare).not.toHaveBeenCalled();
  });

  it('cannot pass an explicit verifier rejection with an empty reason list', async () => {
    const events: string[] = [];
    const target = operations(events);
    target.verify = vi.fn(async (stage, build) => ({
      stage,
      candidate: build.candidate,
      passed: stage !== 'independent',
      digest: digest(stage === 'public' ? '5' : stage === 'independent' ? '6' : '7'),
      reasons: [],
    }));

    const result = await new CandidateTransaction({
      context,
      operations: target,
      maxRepairs: 0,
      now: () => '2026-08-25T12:02:00.000Z',
    }).execute();

    expect(result.status).toBe('fail');
    expect(result.reason).toContain('HARNESS_VERIFIER_REJECTED_WITHOUT_REASON');
    expect(target.review).not.toHaveBeenCalled();
  });

  it('requires distinct native hosts for architecture evidence', async () => {
    const target = operations([]);
    target.architecture = vi.fn(async () => ({
      value: {},
      critiqueDigests: [digest('f')],
      invocations: [
        { invocationId: 'architecture-one', host: 'codex' },
        { invocationId: 'architecture-two', host: 'codex' },
      ],
    }));
    const result = await new CandidateTransaction({ context, operations: target, maxRepairs: 0 }).execute();
    expect(result.status).toBe('fail');
    expect(result.reason).toContain('HARNESS_NATIVE_DUAL_HOST_ARCHITECTURE_REQUIRED');
  });

  it('requires the prepared protected-input set to match controller context', async () => {
    for (const protectedInputs of [{}, { 'protected.txt': digest('b') }]) {
      const target = operations([]);
      target.prepare = vi.fn(async () => ({
        baseline: identity('1'), evaluator: identity('2'), candidate: identity('2'), protectedInputs,
      }));
      const result = await new CandidateTransaction({ context, operations: target, maxRepairs: 0 }).execute();
      expect(result.status).toBe('fail');
      expect(result.reason).toContain('HARNESS_PROTECTED_INPUT');
    }
  });

  it('rejects duplicate native host declarations', () => {
    expect(() => new CandidateTransaction({
      context: { ...context, hosts: [...context.hosts, context.hosts[0]] },
      operations: operations([]), maxRepairs: 0,
    })).toThrow('HARNESS_TRANSACTION_DUAL_HOST_EVIDENCE_REQUIRED');
  });

  it('cannot pass an explicit review rejection with an empty reason list', async () => {
    const events: string[] = [];
    const target = operations(events);
    target.verify = vi.fn(async (stage, build) => ({
      stage,
      candidate: build.candidate,
      passed: true,
      digest: digest(stage === 'public' ? '5' : stage === 'independent' ? '6' : '7'),
      reasons: [],
    }));
    target.review = vi.fn(async (host, build) => ({
      host,
      invocationId: `review-${host}`,
      candidate: build.candidate,
      accepted: host !== 'codex',
      digest: digest(host === 'codex' ? '8' : '9'),
      reasons: [],
    }));

    const result = await new CandidateTransaction({
      context,
      operations: target,
      maxRepairs: 0,
      now: () => '2026-08-25T12:02:00.000Z',
    }).execute();

    expect(result.status).toBe('fail');
    expect(result.reason).toContain('HARNESS_REVIEW_REJECTED_WITHOUT_REASON');
    expect(target.verifyProtectedInputs).not.toHaveBeenCalled();
  });

  it('rejects empty or stale acceptance evidence', async () => {
    const missing = operations([]);
    missing.preflightEvidence = vi.fn(async () => ({
      passed: true, reasons: [], commands: [], digests: {},
    }));
    const missingResult = await new CandidateTransaction({
      context, operations: missing, maxRepairs: 1,
    }).execute();
    expect(missingResult.status).toBe('fail');
    expect(missingResult.reason).toContain('HARNESS_ACCEPTANCE_EVIDENCE_INCOMPLETE');

    const stale = operations([]);
    stale.mutationEvidence = vi.fn(async (build) => ({
      passed: true,
      reasons: [],
      commands: [{ ...commandEvidence(0, build.candidate.tree, 'mutation'), exitCode: 101 }],
      digests: { mutation: digest('c') },
    }));
    const staleResult = await new CandidateTransaction({
      context, operations: stale, maxRepairs: 1,
    }).execute();
    expect(staleResult.status).toBe('fail');
    expect(staleResult.reason).toContain('HARNESS_ACCEPTANCE_COMMAND_BINDING_MISMATCH');
  });

  it('does not count recovery events as native execution evidence', async () => {
    const target = operations([]);
    target.runtimeEvidence = vi.fn(() => ({
      retryCount: 2,
      breakerState: 'closed',
      nativeEvidence: {},
      recoveryEvents: [{ outcome: 'transient-retry' }, { outcome: 'success' }],
    }));
    const result = await new CandidateTransaction({
      context, operations: target, maxRepairs: 1,
    }).execute();
    expect(result.status).toBe('fail');
    expect(result.reason).toContain('HARNESS_RUNTIME_EVIDENCE_FAILED');
  });

  it('preserves failed build evidence and repairs from the compiler failure', async () => {
    const events: string[] = [];
    const target = operations(events);
    const successfulBuild = target.build;
    target.build = vi.fn(async (admission, attempt, signal) => {
      if (attempt > 0) return await successfulBuild(admission, attempt, signal);
      throw new CandidateBuildFailure({
        candidate: admission.candidate,
        commands: [{ ...commandEvidence(attempt, admission.candidate.tree), exitCode: 101 }],
        artifactDigests: { binary: digest('3') },
      }, ['cargo exited 101']);
    });

    const result = await new CandidateTransaction({
      context,
      operations: target,
      maxRepairs: 1,
      now: () => '2026-08-25T12:02:00.000Z',
    }).execute();

    expect(result.status).toBe('pass');
    expect(result.receipt.commands.filter(({ stage }) => stage === 'build')
      .map(({ exitCode }) => exitCode)).toEqual([101, 0]);
    expect(result.receipt.patchDigests).toHaveLength(2);
    expect(result.receipt.artifactDigests).toHaveProperty('attempt-0:binary');
    expect(result.receipt.artifactDigests).toHaveProperty('attempt-1:binary');
  });
});
