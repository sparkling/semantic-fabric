// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { describe, expect, it, beforeEach, vi } from 'vitest';

const state = vi.hoisted(() => ({
  controller: null as unknown,
  attest: vi.fn(),
  materialize: vi.fn(),
  materializationSafety: vi.fn(),
  worktreePrepare: vi.fn(),
  worktreeDispose: vi.fn(),
  frozenStable: vi.fn(),
  frozenCleanup: vi.fn(),
  nativeCleanup: vi.fn(),
  clientInvoke: vi.fn(),
  candidateRun: vi.fn(),
  transactionConstructed: vi.fn(),
  transactionExecute: vi.fn(),
  operationCleanup: vi.fn(),
  cleanupEvents: [] as string[],
}));

vi.mock('../src/controller-attestation.js', () => ({
  HARNESS_MANIFEST_PATH: 'coding-harness/.harness/manifest.json',
  attestController: state.attest,
}));

vi.mock('../src/evaluator.js', () => ({ materializeEvaluatorCommit: state.materialize }));
vi.mock('../src/git-materialization.js', () => ({
  assertGitMaterializationSafe: state.materializationSafety,
}));
vi.mock('../src/git-process.js', () => ({
  runGitCommand: vi.fn(async (_root: string, args: readonly string[]) => {
    const task = (state.controller as { task: { baseline: { commit: string; tree: string } } }).task;
    return { exitCode: 0, stdout: args[1]?.endsWith('^{tree}') ? `${task.baseline.tree}\n` : `${task.baseline.commit}\n` };
  }),
}));

vi.mock('../src/git-worktrees.js', () => ({
  GitWorktreeSet: class {
    async prepare(baseline: string, evaluator: string) {
      const task = (state.controller as { task: { baseline: { tree: string } } }).task;
      const value = {
        baseline: { commit: baseline, tree: task.baseline.tree },
        evaluator: { commit: evaluator, tree: 'e'.repeat(40), ref: 'refs/evaluator' },
        candidate: { commit: evaluator, tree: 'e'.repeat(40) },
        candidateRoot: '/run/candidate',
        evaluatorRoot: '/run/evaluator',
        verifierRoots: {
          public: '/run/public', independent: '/run/independent', regression: '/run/regression',
        },
      };
      state.worktreePrepare(value);
      return value;
    }
    controlledRoot() { return '/run'; }
    outputRoot(stage: string) { return `/run/${stage}-output`; }
    async installFrozenOverlay() {}
    verifyFrozenOverlay() {}
    async candidateIdentity() {
      return { commit: 'd'.repeat(40), tree: 'e'.repeat(40) };
    }
    async assertCandidateSourceStable() {}
    async assertVerifierSourceStable() {}
    async dispose() { state.cleanupEvents.push('worktrees'); state.worktreeDispose(); }
  },
}));

vi.mock('../src/git-protected-boundary.js', () => ({
  GitProtectedInputBoundary: class {
    async capture() {
      const controller = state.controller as { taskPath: string; taskBlobDigest: string };
      return Object.freeze({ [controller.taskPath]: controller.taskBlobDigest });
    }
    async verify() { return Object.freeze({ allow: true, reasons: ['stable'] }); }
  },
}));

vi.mock('../src/model-context.js', () => ({
  RepositoryModelContextProvider: class {
    async declaredSource() { return {}; }
    async admittedSource() { return {}; }
  },
}));

vi.mock('../src/acceptance-runner.js', () => ({
  AcceptanceRunner: class {
    async redBaseline() { return {}; }
    async mutations() { return {}; }
  },
}));

vi.mock('../src/repository-operations.js', () => ({
  candidateExpectationForTask: vi.fn(() => Object.freeze({ mode: 'verifier-only' })),
  RepositoryCandidateOperations: class {
    readonly options: {
      worktrees: { dispose(): Promise<void> };
      worktreeChildCleanupCallbacks: (() => Promise<void>)[];
      cleanupCallbacks: (() => Promise<void>)[];
    };
    cleaned = false;
    constructor(options: typeof this.options) { this.options = options; }
    async cleanup() {
      if (this.cleaned) return;
      this.cleaned = true;
      state.operationCleanup();
      await Promise.all(this.options.worktreeChildCleanupCallbacks.map(async (cleanup) => await cleanup()));
      await Promise.all([
        this.options.worktrees.dispose(),
        ...this.options.cleanupCallbacks.map(async (cleanup) => await cleanup()),
      ]);
    }
  },
}));

vi.mock('../src/candidate.js', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../src/candidate.js')>();
  return {
    ...actual,
    CandidateTransaction: class {
      constructor() { state.transactionConstructed(); }
      async execute() {
        state.transactionExecute();
        return Object.freeze({
          status: 'pass', reason: null, receipt: { digest: '9'.repeat(64) }, finalPatch: 'patch',
        });
      }
    },
  };
});

vi.mock('../src/programme-policy-v5.js', async () => {
  const gate = await import('../src/programme-gate-contract-v1.js');
  const fingerprint = 'f'.repeat(64);
  return {
    createFrozenProgrammePolicyV1: vi.fn((input: Record<string, any>) => Object.freeze({
      schemaVersion: 1,
      policyId: 'semantic-fabric-programme-v5-policy-v1',
      authority: 'development-only-no-promotion',
      gateContract: gate.createProgrammeGateContractV1(1),
      bootstrap: input.bootstrap,
      controller: { identity: input.controller.identity },
      execution: {
        evaluator: input.execution.evaluator,
        protectedInputs: input.execution.protectedInputs,
        boundTaskDigest: '8'.repeat(64),
        routeSnapshotDigest: '7'.repeat(64),
      },
      taskEvidencePlanDigest: input.taskEvidencePlanDigest,
    })),
    programmePolicyFingerprint: vi.fn(() => fingerprint),
    verifyFrozenProgrammePolicyV1: vi.fn((value: unknown, anchor: string) => {
      if (anchor !== fingerprint) throw new Error('HARNESS_PROGRAMME_POLICY_FINGERPRINT_MISMATCH');
      return Object.freeze({ snapshot: value, fingerprint });
    }),
  };
});

import { parseAcceptanceTask, type AcceptanceTask } from '../src/acceptance-task.js';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import { createProgrammeGateContractV1 } from '../src/programme-gate-contract-v1.js';
import {
  prepareProgrammeV5Transaction,
  type ProgrammeV5DriverOptions,
} from '../src/programme-v5-driver.js';
import { programmeV5RufloFixture } from './candidate-fixtures.js';

const TASK_PATH = 'coding-harness/config/programme-v5-acceptance.json';
const CONTROLLER_COMMIT = 'c'.repeat(40);
const BUILD_DIGEST = '2'.repeat(64);
const RUNTIME_DIGEST = '3'.repeat(64);
const STORE_DIGEST = '4'.repeat(64);
const FIXED_NODE = '53fb205ae78805130177e24bcb459a69a1518c8d98f8965f31d85aae7ea840fc';
const FIXED_GIT = '2a8c18fbf43da9f692d75474c72bea9dfd796c260b0f3dfe456376abc3bbd668';
const validTask = parseAcceptanceTask(JSON.parse(readFileSync(
  new URL('../config/programme-v5-acceptance.json', import.meta.url), 'utf8',
)), SECURE_HARNESS_CONFIG);
const legacyTask = parseAcceptanceTask(JSON.parse(readFileSync(
  new URL('../config/issue-8-acceptance.json', import.meta.url), 'utf8',
)), SECURE_HARNESS_CONFIG);

describe('programme v5 driver', () => {
  beforeEach(() => {
    for (const spy of Object.values(state)) if (typeof spy === 'function' && 'mockClear' in spy) spy.mockClear();
    state.cleanupEvents.length = 0;
    state.controller = controller(validTask);
    state.attest.mockImplementation(async () => state.controller);
    state.materialize.mockResolvedValue({
      commit: 'd'.repeat(40), tree: 'e'.repeat(40), ref: 'refs/metaharness/evaluators/test',
    });
  });

  it('rejects schema v2 before native or model preparation', async () => {
    state.controller = controller(legacyTask);
    const native = vi.fn();
    await expect(prepareProgrammeV5Transaction(options({ createNativeSession: native })))
      .rejects.toThrow('HARNESS_PROGRAMME_V5_VERIFIER_ONLY_TASK_REQUIRED');
    expect(native).not.toHaveBeenCalled();
    expect(state.clientInvoke).not.toHaveBeenCalled();
    expect(state.transactionConstructed).not.toHaveBeenCalled();
  });

  it('prepares a frozen canonical policy without a model call', async () => {
    const prepared = await prepareProgrammeV5Transaction(options());
    expect(Object.isFrozen(prepared)).toBe(true);
    expect(prepared.policyBlob.endsWith('\n')).toBe(false);
    expect(prepared.policyBlob).toBe(canonicalJson(JSON.parse(prepared.policyBlob)));
    expect(state.clientInvoke).not.toHaveBeenCalled();
    expect(state.candidateRun).not.toHaveBeenCalled();
    expect(state.transactionConstructed).not.toHaveBeenCalled();
    await prepared.abort();
  });

  it('requires the byte-exact policy and external anchor', async () => {
    const wrongBlob = await prepareProgrammeV5Transaction(options());
    await expect(wrongBlob.execute(`${wrongBlob.policyBlob} `, wrongBlob.policyFingerprint))
      .rejects.toThrow('HARNESS_PROGRAMME_V5_POLICY_BLOB_MISMATCH');
    const wrongAnchor = await prepareProgrammeV5Transaction(options());
    await expect(wrongAnchor.execute(wrongAnchor.policyBlob, '0'.repeat(64)))
      .rejects.toThrow('HARNESS_PROGRAMME_V5_POLICY_FINGERPRINT_MISMATCH');
    expect(state.transactionConstructed).not.toHaveBeenCalled();
    expect(state.operationCleanup).toHaveBeenCalledTimes(2);
  });

  it('is single-use and cleans every leased resource after execute', async () => {
    const prepared = await prepareProgrammeV5Transaction(options());
    const result = await prepared.execute(prepared.policyBlob, prepared.policyFingerprint);
    expect(result.policyFingerprint).toBe(prepared.policyFingerprint);
    expect(result.rufloEvidence.taskId).toBe(validTask.taskId);
    await expect(prepared.execute(prepared.policyBlob, prepared.policyFingerprint))
      .rejects.toThrow('HARNESS_PROGRAMME_V5_TRANSACTION_ALREADY_USED');
    expect(state.transactionConstructed).toHaveBeenCalledTimes(1);
    expect(state.transactionExecute).toHaveBeenCalledTimes(1);
    expect(state.operationCleanup).toHaveBeenCalledTimes(1);
    expect(state.frozenCleanup).toHaveBeenCalledTimes(1);
    expect(state.nativeCleanup).toHaveBeenCalledTimes(1);
    expect(state.worktreeDispose).toHaveBeenCalledTimes(1);
    expect(state.cleanupEvents.indexOf('frozen')).toBeLessThan(state.cleanupEvents.indexOf('worktrees'));
  });

  it('rejects otherwise valid evidence captured for a different transaction nonce', async () => {
    const capture = vi.fn((input: Parameters<ProgrammeV5DriverOptions['captureRufloEvidence']>[0]) => {
      const replayNonce = `${input.captureNonce[0] === 'a' ? 'b' : 'a'}${input.captureNonce.slice(1)}`;
      return programmeV5RufloFixture({
        ...input, captureNonce: replayNonce, swarmId: 'swarm_0001',
        coordinationTaskId: 'coordination_0001', hookIds: [], traceIds: [],
        capturedAt: input.transactionStartedAt,
      });
    });
    await expect(prepareProgrammeV5Transaction(options({ captureRufloEvidence: capture })))
      .rejects.toThrow('HARNESS_PROGRAMME_V5_RUFLO_BINDING_MISMATCH');
    expect(capture).toHaveBeenCalledOnce();
    expect(capture.mock.calls[0]![0].captureNonce).toMatch(/^[a-f0-9]{64}$/);
  });

  it('aborts idempotently and prevents later execution', async () => {
    const prepared = await prepareProgrammeV5Transaction(options());
    await Promise.all([prepared.abort(), prepared.abort()]);
    await expect(prepared.execute(prepared.policyBlob, prepared.policyFingerprint))
      .rejects.toThrow('HARNESS_PROGRAMME_V5_TRANSACTION_ALREADY_USED');
    expect(state.operationCleanup).toHaveBeenCalledTimes(1);
    expect(state.nativeCleanup).toHaveBeenCalledTimes(1);
  });

  it.each([
    ['missing', (tools: Record<string, string>) => { delete tools.caBundle; }],
    ['extra', (tools: Record<string, string>) => { tools.legacyCodex = 'forbidden'; }],
    ['mismatched', (tools: Record<string, string>) => { tools.node = '/wrong/node#sha256:bad'; }],
  ])('fails closed on a %s base tool set before model execution', async (_name, mutate) => {
    const base = baseTools();
    mutate(base);
    await expect(prepareProgrammeV5Transaction(options({ toolVersions: base }))).rejects.toThrow();
    expect(state.transactionConstructed).not.toHaveBeenCalled();
    expect(state.clientInvoke).not.toHaveBeenCalled();
  });

  it('rechecks system and native executable identities before constructing the candidate', async () => {
    const changed = recheckedTools();
    changed.node = '/changed/node#sha256:bad';
    const prepared = await prepareProgrammeV5Transaction(options({
      recheckToolVersions: () => changed,
    }));
    await expect(prepared.execute(prepared.policyBlob, prepared.policyFingerprint))
      .rejects.toThrow('HARNESS_PROGRAMME_V5_SYSTEM_TOOL_CHANGED:node');
    expect(state.transactionConstructed).not.toHaveBeenCalled();

    const native = nativeSession() as any;
    native.ledger.preflightExecutableIdentitySnapshot = () => [
      executableIdentity('codex', baseTools().codexExecutable),
      { ...executableIdentity('claude-code', baseTools().claudeExecutable), digest: 'f'.repeat(64) },
    ];
    state.cleanupEvents.length = 0;
    await expect(prepareProgrammeV5Transaction(options({
      createNativeSession: async () => native,
    }))).rejects.toThrow('HARNESS_PROGRAMME_V5_NATIVE_EXECUTABLE_BINDING_MISMATCH:claude-code');
    expect(state.cleanupEvents.indexOf('frozen')).toBeLessThan(state.cleanupEvents.indexOf('worktrees'));
  });
});

function controller(task: AcceptanceTask) {
  return Object.freeze({
    identity: { commit: CONTROLLER_COMMIT, tree: 'a'.repeat(40) },
    manifestPath: 'coding-harness/.harness/manifest.json',
    manifestBlob: '{}\n',
    manifestBlobDigest: '5'.repeat(64),
    taskPath: TASK_PATH,
    taskBlob: '{}\n',
    taskBlobDigest: '6'.repeat(64),
    buildManifestPath: 'coding-harness/.harness/controller-build.json',
    buildManifestBlob: '{}\n',
    buildManifestBlobDigest: BUILD_DIGEST,
    task,
    manifest: {},
    build: { runtimeTreeDigest: RUNTIME_DIGEST, lockfileDigest: '7'.repeat(64) },
    executionDigest: '8'.repeat(64),
  });
}

function options(overrides: Partial<ProgrammeV5DriverOptions> = {}): ProgrammeV5DriverOptions {
  const profile = {
    cargoExecutable: '/toolchain/bin/cargo' as const,
    environment: {
      PATH: '/cargo-home/bin:/toolchain/bin:/usr/bin', HOME: '/home/harness',
      CARGO_HOME: '/cargo-home', CARGO_NET_OFFLINE: 'true', CARGO_INCREMENTAL: '0',
    },
    readOnlyMounts: [],
    isolator: {},
  } as ProgrammeV5DriverOptions['createBootstrapRustProfile'] extends (...args: any[]) => infer R ? R : never;
  const frozen = {
    lockfile: {
      sourcePath: '/frozen/Cargo.lock', workspacePath: 'Cargo.lock' as const,
      digest: validTask.schemaVersion === 3 ? validTask.rust.frozenLockSha256 : '',
    },
    registryPackages: [],
    assertStable: state.frozenStable,
    cleanup: async () => { state.cleanupEvents.push('frozen'); state.frozenCleanup(); },
  };
  const native = nativeSession();
  return {
    repositoryRoot: '/repo',
    controllerSourceRoot: '/source',
    controllerRepositoryRoot: '/controller',
    runRoot: '/run',
    evaluatorScratchRoot: '/scratch/evaluator',
    controllerCommit: CONTROLLER_COMMIT,
    taskPath: TASK_PATH,
    runId: 'programme_run_0001',
    models: { codex: 'gpt-5.6-sol', claude: 'claude-sonnet-4-6' },
    bootstrap: { controllerStoreDigest: STORE_DIGEST, nodeDigest: FIXED_NODE, gitDigest: FIXED_GIT },
    toolVersions: baseTools(),
    recheckToolVersions: recheckedTools,
    createBootstrapRustProfile: () => profile,
    createLockedRustRuntime: () => ({ profile, toolVersions: lockedRust() }),
    prepareFrozenLockfile: async () => frozen,
    createNativeSession: async () => native,
    captureRufloEvidence: ({
      taskId, runId, routeSnapshotDigest, captureNonce, transactionStartedAt,
    }) =>
      programmeV5RufloFixture({
      taskId, runId,
      swarmId: 'swarm_0001',
      coordinationTaskId: 'coordination_0001',
      hookIds: [],
      traceIds: [],
      routeSnapshotDigest,
      captureNonce,
      transactionStartedAt,
      capturedAt: '2026-08-27T12:00:00.000Z',
    }),
    createAgenticQeCollector: () => async () => [],
    now: () => '2026-08-27T12:00:00.000Z',
    ...overrides,
  };
}

function nativeSession() {
  const output = { output: {}, quality: 1, confidence: 1, risk: 0, costUsd: 0, latencyMs: 1 };
  const candidates = [
    {
      id: 'codex-native', host: 'codex' as const, model: 'gpt-5.6-sol',
      handles: ['architecture', 'implementation', 'repair', 'review'],
      run: async () => { state.candidateRun(); return output; },
    },
    {
      id: 'claude-native', host: 'claude-code' as const, model: 'claude-sonnet-4-6',
      handles: ['architecture', 'implementation', 'repair', 'review'],
      run: async () => { state.candidateRun(); return output; },
    },
  ] as const;
  const host = (name: 'codex' | 'claude-code') => ({
    host: name,
    model: name === 'codex' ? 'gpt-5.6-sol' : 'claude-sonnet-4-6',
    role: name === 'codex' ? 'implementation-review' as const : 'architecture-review' as const,
    clientVersion: name === 'codex' ? 'codex-cli 0.149.1' : '2.1.234 (Claude Code)',
    authClass: name === 'codex'
      ? 'native-openai-subscription' as const : 'native-anthropic-subscription' as const,
    subscriptionCostUsd: 0 as const,
  });
  return {
    candidates,
    clients: {
      codex: { invoke: state.clientInvoke },
      'claude-code': { invoke: state.clientInvoke },
    },
    hosts: [host('codex'), host('claude-code')] as const,
    ledger: {
      preflightExecutableIdentitySnapshot: () => {
        const tools = baseTools();
        return [
          executableIdentity('codex', tools.codexExecutable),
          executableIdentity('claude-code', tools.claudeExecutable),
        ];
      },
    },
    cleanup: async () => { state.cleanupEvents.push('native'); state.nativeCleanup(); },
  } as unknown as Awaited<ReturnType<ProgrammeV5DriverOptions['createNativeSession']>>;
}

function baseTools(): Record<string, string> {
  const contract = createProgrammeGateContractV1(1);
  const dynamic = new Set([
    'controllerExecutionDigest', 'controllerBuildManifestDigest', 'controllerRuntimeTreeDigest',
    'controllerManifestDigest', 'controllerTaskPath', 'controllerTaskPathDigest',
    'controllerTaskDigest', 'taskEvidencePlanDigest', 'boundTaskDigest',
    'programmePolicyFingerprint', 'frozenCargoLockDigest', 'codex', 'claude',
    'rustRegistryClosure', 'rustRegistryLock', 'rustRegistrySelection', 'rustRegistryMetadata',
  ]);
  const tools = Object.fromEntries(contract.tools.requiredKeys
    .filter((key) => !dynamic.has(key)).map((key) => [key, 'evidence'])) as Record<string, string>;
  for (const [key, value] of Object.entries(contract.tools.exactValues)) {
    if (key in tools) tools[key] = value;
  }
  Object.assign(tools, {
    bootstrapSource: 'verified-packed-private-runtime',
    bootstrapControllerStoreDigest: STORE_DIGEST,
    bootstrapBuildManifestDigest: BUILD_DIGEST,
    bootstrapRuntimeTreeDigest: RUNTIME_DIGEST,
    bootstrapNodeDigest: FIXED_NODE,
    bootstrapGitDigest: FIXED_GIT,
    rustToolchainClosure: `81cc515ef94bae07d2451ff3701ce6e6eee7878327dc8088ebac773f1570f7c4:1:1`,
    rustRegistryBootstrapSnapshot: `${'a'.repeat(64)}:1:1`,
    rustBootstrapClosureMetadata: 'a'.repeat(64),
    rufloHive: 'hierarchical',
    rufloConsensus: 'raft',
  });
  return tools;
}

function lockedRust(): Record<string, string> {
  const lock = validTask.schemaVersion === 3 ? validTask.rust.frozenLockSha256 : '';
  return {
    rustRegistryClosure: `1bb717af28554b8cbb83ff1a219bbbd294ccee98691191bc9f65dc431106e908:1:1`,
    rustRegistryLock: `${lock}:1:1`,
    rustRegistrySelection: `x86_64-unknown-linux-gnu:${'b'.repeat(64)}`,
    rustRegistryMetadata: 'c'.repeat(64),
  };
}

function recheckedTools(): Record<string, string> {
  const tools = baseTools();
  const keys = [
    'cargo', 'cargoLlvmCov', 'node', 'codexExecutable', 'claudeExecutable',
    'bwrap', 'systemdRun', 'systemctl', 'caBundle', 'agenticQeMcp', 'agenticQe',
    'agenticQePackageTreeDigest',
  ];
  return Object.fromEntries(keys.map((key) => [key, tools[key]]));
}

function executableIdentity(host: 'codex' | 'claude-code', claim: string) {
  const [path, digest] = claim.split('#sha256:');
  return { host, path, digest };
}

function canonicalJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`;
  if (value !== null && typeof value === 'object') {
    const record = value as Record<string, unknown>;
    return `{${Object.keys(record).sort().map((key) =>
      `${JSON.stringify(key)}:${canonicalJson(record[key])}`).join(',')}}`;
  }
  return JSON.stringify(value);
}
