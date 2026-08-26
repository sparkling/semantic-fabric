// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { VerifierRegistry, predicateVerifier } from '@metaharness/harness';
import type { AcceptanceTask } from './acceptance-task.js';
import {
  acceptanceTaskPrompt,
  bindAcceptanceTaskToRustProfile,
  requiredQeProfiles,
  requireExactReferenceCandidate,
} from './acceptance-task.js';
import { AcceptanceRunner } from './acceptance-runner.js';
import { CandidateTransaction, type CandidateBuild } from './candidate.js';
import type { CandidateTransactionResult } from './candidate-types.js';
import { SECURE_HARNESS_CONFIG } from './config.js';
import { attestController, ISSUE_8_ACCEPTANCE_TASK_PATH, type ControllerAttestation } from './controller-attestation.js';
import { DEVELOPMENT_AUTHORITY, parseTaskContract } from './contracts.js';
import { materializeEvaluatorCommit, type EvaluatorIdentity } from './evaluator.js';
import type { FrozenRegistryPackage } from './frozen-cargo-metadata.js';
import { GitProtectedInputBoundary } from './git-protected-boundary.js';
import { runGitCommand } from './git-process.js';
import { assertGitMaterializationSafe } from './git-materialization.js';
import { GitWorktreeSet, type PreparedWorktrees } from './git-worktrees.js';
import { RepositoryModelContextProvider } from './model-context.js';
import { NativeRepositoryModelController, type NativeStructuredClient } from './model-controller.js';
import { NativeInvocationRecovery } from './models/recovery.js';
import { PersistentRoutedAgentPool, VerifiedRoutingHistory, type NativeModelCandidate } from './models/routing.js';
import type { NativeHost } from './models/types.js';
import type { NativeRuntimeLedger } from './native-runtime-ledger.js';
import { digestValue, type HostEvidence } from './receipts.js';
import { candidateExpectationForTask, RepositoryCandidateOperations } from './repository-operations.js';
import type { RustOfflineProfile } from './rust-sandbox.js';

export interface Issue8FrozenLockLease {
  readonly lockfile: Readonly<{
    readonly sourcePath: string;
    readonly workspacePath: 'Cargo.lock';
    readonly digest: string;
  }>;
  readonly registryPackages: readonly FrozenRegistryPackage[];
  assertStable(): void;
  cleanup(): Promise<void>;
}

export interface Issue8NativeSession {
  readonly candidates: readonly [NativeModelCandidate, NativeModelCandidate];
  readonly clients: Readonly<Record<NativeHost, NativeStructuredClient>>;
  readonly hosts: readonly [HostEvidence, HostEvidence];
  readonly ledger: NativeRuntimeLedger;
  cleanup(): Promise<void> | void;
}

export interface Issue8DriverOptions {
  readonly repositoryRoot: string;
  readonly controllerSourceRoot: string;
  readonly controllerRepositoryRoot: string;
  readonly runRoot: string;
  readonly evaluatorScratchRoot: string;
  readonly controllerCommit: string;
  readonly taskPath: string;
  readonly runId: string;
  readonly models: Readonly<{ codex: string; claude: string }>;
  readonly toolVersions: Readonly<Record<string, string>>;
  readonly createBootstrapRustProfile: (writableRoot: string) => RustOfflineProfile;
  readonly createLockedRustRuntime: (
    writableRoot: string,
    lockfile: Issue8FrozenLockLease['lockfile'],
    packages: readonly FrozenRegistryPackage[],
  ) => Readonly<{ profile: RustOfflineProfile; toolVersions: Readonly<Record<string, string>> }>;
  readonly prepareFrozenLockfile: (input: Readonly<{
    task: AcceptanceTask;
    evaluator: EvaluatorIdentity;
    prepared: PreparedWorktrees;
    rustProfile: RustOfflineProfile;
  }>) => Promise<Issue8FrozenLockLease>;
  readonly createNativeSession: (input: Readonly<{
    prepared: PreparedWorktrees;
    evaluatorPaths: readonly string[];
    taskPath: string;
    models: Readonly<{ codex: string; claude: string }>;
  }>) => Promise<Issue8NativeSession>;
  readonly captureRufloEvidence: (input: Readonly<{
    taskId: string;
    runId: string;
    routeSnapshotDigest: string;
  }>) => Promise<unknown> | unknown;
  readonly agenticQeEvidence: (
    build: CandidateBuild,
    context: Readonly<{
      controlledRoot: string;
      candidateRoot: string;
      outputRoot: string;
      rustProfile: RustOfflineProfile;
    }>,
    signal?: AbortSignal,
  ) => Promise<readonly unknown[]>;
  readonly signal?: AbortSignal;
  readonly now?: () => string;
}

export interface Issue8DriverResult {
  readonly controller: ControllerAttestation['identity'];
  readonly evaluator: EvaluatorIdentity;
  readonly routeSnapshotDigest: string;
  readonly transaction: CandidateTransactionResult;
}

const ROUTED_STEPS = Object.freeze([
  'architecture', 'implementation', 'repair',
] as const);
const CONFORMANCE_REPORT_PATHS = Object.freeze([
  'tests/w3c/rdb2rdf/earl-semantic-fabric-direct.ttl',
  'tests/w3c/rdb2rdf/earl-semantic-fabric-r2rml.ttl',
]);

export async function runIssue8Transaction(
  options: Issue8DriverOptions,
): Promise<Issue8DriverResult> {
  assertRunBindings(options);
  const controller = await attestController({
    repositoryRoot: options.controllerSourceRoot,
    controllerRepositoryRoot: options.controllerRepositoryRoot,
    controllerCommit: options.controllerCommit,
    taskPath: options.taskPath,
    signal: options.signal,
  });
  if (controller.taskPath !== options.taskPath
    || controller.taskPath !== ISSUE_8_ACCEPTANCE_TASK_PATH
    || controller.task.schemaVersion !== 2) {
    throw new Error('HARNESS_ISSUE_8_REQUIRES_TASK_SCHEMA_V2');
  }
  const unboundTask = controller.task;
  const referenceCandidate = requireExactReferenceCandidate(unboundTask);
  await assertDeclaredGitIdentities(options.repositoryRoot, unboundTask, referenceCandidate);
  await assertGitMaterializationSafe({
    repositoryRoot: options.repositoryRoot,
    commits: [
      options.controllerCommit,
      unboundTask.baseline.commit,
      referenceCandidate.commit,
    ],
    signal: options.signal,
  });
  const evaluator = await materializeEvaluatorCommit({
    repositoryRoot: options.repositoryRoot,
    scratchRoot: options.evaluatorScratchRoot,
    baselineCommit: unboundTask.baseline.commit,
    source: {
      mode: 'exact-reference',
      referenceCandidateCommit: referenceCandidate.commit,
    },
    evaluatorPaths: unboundTask.evaluatorPaths,
    implementationPaths: unboundTask.implementationPaths,
    taskId: unboundTask.taskId,
    signal: options.signal,
  });
  await assertGitMaterializationSafe({
    repositoryRoot: options.repositoryRoot,
    commits: [options.controllerCommit, evaluator.commit],
    signal: options.signal,
  });
  const worktrees = new GitWorktreeSet({
    repositoryRoot: options.repositoryRoot,
    runRoot: options.runRoot,
  });
  let native: Issue8NativeSession | null = null;
  let frozen: Issue8FrozenLockLease | null = null;
  let operations: RepositoryCandidateOperations | null = null;
  let transactionOwnsCleanup = false;
  try {
    const prepared = await worktrees.prepare(
      unboundTask.baseline.commit,
      evaluator.commit,
      options.signal,
    );
    const bootstrapRustProfile = options.createBootstrapRustProfile(worktrees.controlledRoot());
    const bootstrapTask = bindAcceptanceTaskToRustProfile(unboundTask, bootstrapRustProfile);
    frozen = await options.prepareFrozenLockfile({
      task: bootstrapTask,
      evaluator,
      prepared,
      rustProfile: bootstrapRustProfile,
    });
    frozen.assertStable();
    const frozenLockfile = frozen.lockfile;
    const rustRuntime = options.createLockedRustRuntime(
      worktrees.controlledRoot(), frozenLockfile, frozen.registryPackages,
    );
    const rustProfile = rustRuntime.profile;
    const task = bindAcceptanceTaskToRustProfile(unboundTask, rustProfile);
    await worktrees.installFrozenOverlay(
      frozenLockfile.sourcePath,
      frozenLockfile.workspacePath,
      frozenLockfile.digest,
      options.signal,
    );
    native = await options.createNativeSession({
      prepared,
      evaluatorPaths: task.evaluatorPaths,
      taskPath: controller.taskPath,
      models: options.models,
    });
    assertNativeSession(native, options.models);

    const pool = routedPool(options.runId, task, native.candidates);
    const routeSnapshot = pool.freeze(ROUTED_STEPS);
    const routeSnapshotDigest = digestValue(routeSnapshot);
    const frozenAt = (options.now ?? (() => new Date().toISOString()))();
    const rufloEvidence = await options.captureRufloEvidence({
      taskId: task.taskId,
      runId: options.runId,
      routeSnapshotDigest,
    });
    const model = new NativeRepositoryModelController({
      pool,
      candidates: native.candidates,
      clients: native.clients,
      architectureVerifiers: repositoryArchitectureVerifiers(),
      recovery: new NativeInvocationRecovery(),
      taskPrompt: acceptanceTaskPrompt(task),
      contextProvider: new RepositoryModelContextProvider({
        candidateRoot: prepared.candidateRoot,
        implementationPaths: task.implementationPaths,
        evaluatorPaths: task.evaluatorPaths,
        maxTotalBytes: 1_000_000,
      }),
    });
    const acceptance = new AcceptanceRunner({
      task,
      worktrees,
      config: SECURE_HARNESS_CONFIG,
      offlineIsolator: rustProfile.isolator,
      sourceEnvironment: rustProfile.environment,
    });
    const protectedPaths = unique([
      ...SECURE_HARNESS_CONFIG.requiredProtectedPaths,
      ...task.evaluatorPaths,
      frozenLockfile.workspacePath,
    ]);
    const protectedInputBoundary = new GitProtectedInputBoundary({
      repositoryRoot: options.controllerRepositoryRoot,
      controllerCommit: options.controllerCommit,
      evaluatorRoot: prepared.evaluatorRoot,
      evaluatorPaths: [...task.evaluatorPaths, frozenLockfile.workspacePath],
    });
    const commands = allCommands(task);
    operations = new RepositoryCandidateOperations({
      worktrees,
      config: SECURE_HARNESS_CONFIG,
      baselineCommit: task.baseline.commit,
      evaluatorCommit: evaluator.commit,
      candidateExpectation: candidateExpectationForTask(task),
      taskForWorkspace: (candidateRoot) => parseTaskContract({
        schemaVersion: 1,
        taskId: task.taskId,
        runId: options.runId,
        workspaceRoot: candidateRoot,
        readablePaths: task.implementationPaths,
        mutablePaths: task.implementationPaths,
        protectedPaths,
        tools: ['cargo', 'apply_patch'],
        commands,
        network: { mode: 'offline', allowedOrigins: [] },
        authority: DEVELOPMENT_AUTHORITY,
      }, SECURE_HARNESS_CONFIG),
      buildCommands: task.commands.build.map(({ command }) => command),
      verifierCommands: {
        public: task.commands.public.map(({ command }) => command),
        independent: task.commands.independent.map(({ command }) => command),
        regression: task.commands.regression.map(({ command }) => command),
      },
      verifierGeneratedOutputs: { regression: [
        {
          evidenceId: 'workspace-tests-earl',
          command: namedCommand(task, 'workspace-tests'),
          workspacePaths: CONFORMANCE_REPORT_PATHS,
        },
        {
          evidenceId: 'w3c-conformance-earl',
          command: namedCommand(task, 'w3c-conformance'),
          workspacePaths: CONFORMANCE_REPORT_PATHS,
        },
      ] },
      artifactPaths: task.artifactPaths,
      model,
      offlineIsolator: rustProfile.isolator,
      offlineEnvironment: rustProfile.environment,
      protectedInputBoundary,
      frozenLockfile,
      assertExternalState: () => frozen?.assertStable(),
      cleanupCallbacks: [
        async () => await frozen?.cleanup(),
        async () => await native?.cleanup(),
      ],
      agenticQeEvidence: async (build, signal) => await options.agenticQeEvidence(
        build,
        {
          controlledRoot: worktrees.controlledRoot(),
          candidateRoot: prepared.verifierRoots.independent,
          outputRoot: worktrees.outputRoot('independent'),
          rustProfile,
        },
        signal,
      ),
      nativeRuntime: {
        ledger: native.ledger,
        taskId: task.taskId,
        runId: options.runId,
        hosts: native.hosts,
      },
      preflightEvidence: async (candidate, signal) =>
        await acceptance.redBaseline(candidate, signal),
      mutationEvidence: async (build, signal) =>
        await acceptance.mutations(build, signal),
    });
    const protectedInputs = await protectedInputBoundary.capture(
      parseTaskContract({
        schemaVersion: 1,
        taskId: task.taskId,
        runId: options.runId,
        workspaceRoot: prepared.candidateRoot,
        readablePaths: task.implementationPaths,
        mutablePaths: task.implementationPaths,
        protectedPaths,
        tools: ['cargo', 'apply_patch'],
        commands,
        network: { mode: 'offline', allowedOrigins: [] },
        authority: DEVELOPMENT_AUTHORITY,
      }, SECURE_HARNESS_CONFIG),
      SECURE_HARNESS_CONFIG,
    );
    if (protectedInputs[controller.taskPath] !== controller.taskBlobDigest) {
      throw new Error('HARNESS_CONTROLLER_TASK_PROTECTED_INPUT_MISMATCH');
    }
    const candidateTransaction = new CandidateTransaction({
      context: {
        runId: options.runId,
        taskId: task.taskId,
        authority: DEVELOPMENT_AUTHORITY,
        identities: { controller: controller.identity, baseline: task.baseline, evaluator },
        protectedInputs,
        route: {
          snapshotDigest: routeSnapshotDigest,
          frozenAt,
          routerVersion: '@metaharness/router@0.4.0',
        },
        hosts: [...native.hosts],
        toolVersions: {
          ...options.toolVersions,
          ...rustRuntime.toolVersions,
          controllerExecutionDigest: controller.executionDigest,
          controllerBuildManifestDigest: controller.buildManifestBlobDigest,
          controllerRuntimeTreeDigest: controller.build.runtimeTreeDigest,
          controllerManifestDigest: controller.manifestBlobDigest,
          controllerTaskDigest: controller.taskBlobDigest,
          controllerTaskPath: controller.taskPath,
          controllerTaskPathDigest: digestValue(controller.taskPath),
          codex: native.hosts.find(({ host }) => host === 'codex')?.clientVersion ?? 'unknown',
          claude: native.hosts.find(({ host }) => host === 'claude-code')?.clientVersion ?? 'unknown',
        },
        requiredQeProfiles: requiredQeProfiles(task),
        rufloEvidence,
      },
      operations,
      maxRepairs: 1,
      signal: options.signal,
      now: options.now,
    });
    transactionOwnsCleanup = true;
    const transaction = await candidateTransaction.execute();
    return Object.freeze({ controller: controller.identity, evaluator, routeSnapshotDigest, transaction });
  } finally {
    if (transactionOwnsCleanup && operations !== null) {
      await operations.cleanup();
    } else {
      await cleanupResources([
        async () => await native?.cleanup(),
        async () => await frozen?.cleanup(),
        async () => await worktrees.dispose(),
      ]);
    }
  }
}

function allCommands(task: AcceptanceTask) {
  return [
    ...task.redBaseline.commands,
    ...task.commands.build,
    ...task.commands.public,
    ...task.commands.independent,
    ...task.commands.regression,
    ...task.commands.mutation,
  ].map(({ command }) => command);
}

function namedCommand(task: AcceptanceTask, commandId: string) {
  const matches = task.commands.regression.filter((entry) => entry.commandId === commandId);
  if (matches.length !== 1) throw new Error(`HARNESS_ISSUE_8_COMMAND_BINDING_INVALID:${commandId}`);
  return matches[0].command;
}

function routedPool(
  runId: string,
  task: AcceptanceTask,
  candidates: readonly NativeModelCandidate[],
): PersistentRoutedAgentPool {
  return new PersistentRoutedAgentPool({
    runId,
    task: {
      id: task.taskId,
      digest: digestValue(task),
      prompt: acceptanceTaskPrompt(task),
      tags: task.routing.tags,
      difficulty: task.routing.difficulty,
    },
    candidates,
    history: new VerifiedRoutingHistory(),
    embedder: {
      dimensions: 16,
      embed: (text) => [...createHash('sha256').update(text).digest().subarray(0, 16)]
        .map((value) => value / 255),
    },
  });
}

function repositoryArchitectureVerifiers(): VerifierRegistry {
  return new VerifierRegistry().register(predicateVerifier(
    'repository-change-architecture-shape',
    'architecture',
    (value) => value !== null
      && typeof value === 'object'
      && !Array.isArray(value)
      && Buffer.byteLength(JSON.stringify(value), 'utf8') <= 64_000,
  ));
}

async function assertDeclaredGitIdentities(
  repositoryRoot: string,
  task: AcceptanceTask,
  referenceCandidate: AcceptanceTask['baseline'],
): Promise<void> {
  for (const [label, identity] of [
    ['baseline', task.baseline],
    ['reference-candidate', referenceCandidate],
  ] as const) {
    const commit = await gitValue(repositoryRoot, ['rev-parse', '--verify', `${identity.commit}^{commit}`]);
    const tree = await gitValue(repositoryRoot, ['rev-parse', `${identity.commit}^{tree}`]);
    if (commit !== identity.commit || tree !== identity.tree) {
      throw new Error(`HARNESS_ISSUE_8_${label.toUpperCase().replace('-', '_')}_IDENTITY_MISMATCH`);
    }
  }
}

async function gitValue(repositoryRoot: string, args: readonly string[]): Promise<string> {
  const result = await runGitCommand(repositoryRoot, args, { maxOutputBytes: 1024 });
  if (result.exitCode !== 0) throw new Error(`HARNESS_ISSUE_8_GIT_FAILED:${args[0]}`);
  return result.stdout.trim();
}

function assertRunBindings(options: Issue8DriverOptions): void {
  if (!/^[A-Za-z0-9_-]{8,128}$/.test(options.runId)) {
    throw new Error('HARNESS_ISSUE_8_RUN_ID_INVALID');
  }
  if (!/^[a-f0-9]{40,64}$/.test(options.controllerCommit)) {
    throw new Error('HARNESS_ISSUE_8_CONTROLLER_COMMIT_INVALID');
  }
}

async function cleanupResources(cleanups: readonly (() => Promise<void>)[]): Promise<void> {
  const outcomes = await Promise.allSettled(cleanups.map(async (cleanup) => await cleanup()));
  const failures = outcomes.filter((outcome) => outcome.status === 'rejected');
  if (failures.length > 0) {
    throw new AggregateError(
      failures.map((outcome) => (outcome as PromiseRejectedResult).reason),
      'HARNESS_ISSUE_8_RESOURCE_CLEANUP_FAILED',
    );
  }
}

function assertNativeSession(
  session: Issue8NativeSession,
  models: Readonly<{ codex: string; claude: string }>,
): void {
  const candidateModels = new Map(session.candidates.map(({ host, model }) => [host, model]));
  const hostModels = new Map(session.hosts.map(({ host, model }) => [host, model]));
  if (session.candidates.length !== 2 || candidateModels.size !== 2
    || session.hosts.length !== 2 || hostModels.size !== 2
    || candidateModels.get('codex') !== models.codex
    || candidateModels.get('claude-code') !== models.claude
    || hostModels.get('codex') !== models.codex
    || hostModels.get('claude-code') !== models.claude) {
    throw new Error('HARNESS_ISSUE_8_NATIVE_SESSION_MISMATCH');
  }
}

function unique(values: readonly string[]): string[] { return [...new Set(values)]; }
