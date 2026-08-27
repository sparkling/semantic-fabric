// SPDX-License-Identifier: MIT
import { randomBytes } from 'node:crypto';
import {
  acceptanceTaskPrompt,
  bindAcceptanceTaskToRustProfile,
  type AcceptanceTaskV3,
} from './acceptance-task.js';
import { AcceptanceRunner } from './acceptance-runner.js';
import { CandidateTransaction, type CandidateBuild } from './candidate.js';
import type { CandidateTransactionResult } from './candidate-types.js';
import { SECURE_HARNESS_CONFIG } from './config.js';
import { attestController, type ControllerAttestation } from './controller-attestation.js';
import { DEVELOPMENT_AUTHORITY, deepFreeze, type TaskContract } from './contracts.js';
import { materializeEvaluatorCommit, type EvaluatorIdentity } from './evaluator.js';
import {
  parseProgrammeV5RufloEvidence,
  type ProgrammeV5RufloEvidence,
} from './evidence.js';
import type { FrozenRegistryPackage } from './frozen-cargo-metadata.js';
import { GitProtectedInputBoundary } from './git-protected-boundary.js';
import { assertGitMaterializationSafe } from './git-materialization.js';
import { GitWorktreeSet, type PreparedWorktrees } from './git-worktrees.js';
import type { Issue8FrozenLockLease, Issue8NativeSession } from './issue-8-driver.js';
import { RepositoryModelContextProvider } from './model-context.js';
import { NativeRepositoryModelController } from './model-controller.js';
import { NativeInvocationRecovery } from './models/recovery.js';
import type { PersistentRoutedAgentPool } from './models/routing.js';
import {
  createFrozenProgrammePolicyV1,
  programmePolicyFingerprint,
  verifyFrozenProgrammePolicyV1,
  type FrozenProgrammePolicyV1,
} from './programme-policy-v5.js';
import {
  assembleProgrammeV5ToolVersions,
  assertProgrammeV5ControllerTask,
  assertProgrammeV5GitIdentity,
  assertProgrammeV5NativeExecutableBindings,
  assertProgrammeV5NativeSession,
  assertProgrammeV5RunBindings,
  assertProgrammeV5ToolRecheck,
  canonicalProgrammePolicyJson,
  canonicalProgrammeTimestamp,
  captureProgrammeV5Bootstrap,
  cleanupProgrammeV5Resources,
  programmeV5ArchitectureVerifiers,
  programmeV5Commands,
  programmeV5RoutedPool,
  programmeV5TaskContract,
  requireProgrammeV5Task,
  type ProgrammeV5BootstrapInputs,
  uniqueProgrammeV5Strings,
} from './programme-v5-driver-support.js';
import { bindProgrammeTaskRuntimeV1 } from './programme-task-runtime-v1.js';
import { digestValue, type GitIdentity } from './receipts.js';
import { candidateExpectationForTask, RepositoryCandidateOperations } from './repository-operations.js';
import type { RustOfflineProfile } from './rust-sandbox.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';
import { resolveTaskEvidencePlanV1 } from './task-evidence-plan.js';
import type { TaskQeCollectorFactory } from './task-qe.js';
const ROUTED_STEPS = Object.freeze(['architecture', 'implementation', 'repair'] as const);
export interface ProgrammeV5DriverOptions {
  readonly repositoryRoot: string;
  readonly controllerSourceRoot: string;
  readonly controllerRepositoryRoot: string;
  readonly runRoot: string;
  readonly evaluatorScratchRoot: string;
  readonly controllerCommit: string;
  readonly taskPath: string;
  readonly runId: string;
  readonly models: Readonly<{ codex: string; claude: string }>;
  readonly bootstrap: Pick<ProgrammeV5BootstrapInputs,
    'controllerStoreDigest' | 'nodeDigest' | 'gitDigest'>;
  readonly toolVersions: Readonly<Record<string, string>>;
  readonly recheckToolVersions: () => Readonly<Record<string, string>>;
  readonly createBootstrapRustProfile: (writableRoot: string) => RustOfflineProfile;
  readonly createLockedRustRuntime: (
    writableRoot: string,
    lockfile: Issue8FrozenLockLease['lockfile'],
    packages: readonly FrozenRegistryPackage[],
  ) => Readonly<{ profile: RustOfflineProfile; toolVersions: Readonly<Record<string, string>> }>;
  readonly prepareFrozenLockfile: (input: Readonly<{
    task: AcceptanceTaskV3;
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
    captureNonce: string;
    transactionStartedAt: string;
  }>) => Promise<unknown> | unknown;
  readonly createAgenticQeCollector: TaskQeCollectorFactory;
  readonly signal?: AbortSignal;
  readonly now?: () => string;
}
export interface ProgrammeV5DriverResult {
  readonly controller: GitIdentity;
  readonly evaluator: EvaluatorIdentity;
  readonly route: Readonly<{
    snapshot: unknown;
    snapshotDigest: string;
    frozenAt: string;
    routerVersion: '@metaharness/router@0.4.0';
  }>;
  readonly policy: FrozenProgrammePolicyV1;
  readonly policyFingerprint: string;
  readonly rufloEvidence: ProgrammeV5RufloEvidence;
  readonly transaction: CandidateTransactionResult;
}
export interface ProgrammeV5PreparedTransaction {
  readonly policyBlob: string;
  readonly policyFingerprint: string;
  execute(expectedPolicyBlob: string, expectedFingerprint: string): Promise<ProgrammeV5DriverResult>;
  abort(): Promise<void>;
}
export async function prepareProgrammeV5Transaction(
  options: ProgrammeV5DriverOptions,
): Promise<ProgrammeV5PreparedTransaction> {
  assertProgrammeV5RunBindings(options.runId, options.controllerCommit);
  if (options.toolVersions.bootstrapSource !== 'verified-packed-private-runtime') {
    throw new Error('HARNESS_PROGRAMME_V5_BOOTSTRAP_EVIDENCE_INVALID');
  }
  const bootstrap = captureProgrammeV5Bootstrap({
    schemaVersion: 3,
    source: options.toolVersions.bootstrapSource,
    controllerCommit: options.controllerCommit,
    taskPath: options.taskPath,
    controllerStoreDigest: options.bootstrap.controllerStoreDigest,
    buildManifestDigest: options.toolVersions.bootstrapBuildManifestDigest,
    runtimeTreeDigest: options.toolVersions.bootstrapRuntimeTreeDigest,
    nodeDigest: options.bootstrap.nodeDigest,
    gitDigest: options.bootstrap.gitDigest,
  });
  const baseToolVersions = deepFreeze({ ...options.toolVersions });
  const controller = await attestController({
    repositoryRoot: options.controllerSourceRoot,
    controllerRepositoryRoot: options.controllerRepositoryRoot,
    controllerCommit: options.controllerCommit,
    taskPath: options.taskPath,
    signal: options.signal,
  });
  assertProgrammeV5ControllerTask(controller, options, bootstrap);
  const unboundTask = controller.task;
  await assertProgrammeV5GitIdentity(options.repositoryRoot, unboundTask.baseline, 'BASELINE');
  await assertGitMaterializationSafe({
    repositoryRoot: options.repositoryRoot,
    commits: [options.controllerCommit, unboundTask.baseline.commit],
    signal: options.signal,
  });
  const evaluator = await materializeEvaluatorCommit({
    repositoryRoot: options.repositoryRoot,
    scratchRoot: options.evaluatorScratchRoot,
    baselineCommit: unboundTask.baseline.commit,
    source: { mode: 'verifier-only', controllerCommit: options.controllerCommit },
    evaluatorPaths: unboundTask.evaluatorPaths,
    implementationPaths: unboundTask.implementationPaths,
    taskId: unboundTask.taskId,
    signal: options.signal,
  });
  await assertGitMaterializationSafe({
    repositoryRoot: options.repositoryRoot,
    commits: [options.controllerCommit, unboundTask.baseline.commit, evaluator.commit],
    signal: options.signal,
  });

  const worktrees = new GitWorktreeSet({ repositoryRoot: options.repositoryRoot, runRoot: options.runRoot });
  let native: Issue8NativeSession | null = null;
  let frozen: Issue8FrozenLockLease | null = null;
  let operations: RepositoryCandidateOperations | null = null;
  let cleanupPromise: Promise<void> | null = null;
  const cleanup = () => cleanupPromise ??= operations === null
    ? cleanupProgrammeV5Resources([
      async () => await native?.cleanup(),
      async () => await frozen?.cleanup(),
      async () => await worktrees.dispose(),
    ])
    : operations.cleanup();
  try {
    const prepared = await worktrees.prepare(unboundTask.baseline.commit, evaluator.commit, options.signal);
    const bootstrapRust = options.createBootstrapRustProfile(worktrees.controlledRoot());
    const bootstrapTask = requireProgrammeV5Task(
      bindAcceptanceTaskToRustProfile(unboundTask, bootstrapRust),
    );
    frozen = await options.prepareFrozenLockfile({
      task: bootstrapTask, evaluator, prepared, rustProfile: bootstrapRust,
    });
    frozen.assertStable();
    if (frozen.lockfile.workspacePath !== 'Cargo.lock'
      || frozen.lockfile.digest !== unboundTask.rust.frozenLockSha256) {
      throw new Error('HARNESS_PROGRAMME_V5_FROZEN_LOCK_BINDING_MISMATCH');
    }
    const frozenLockfile = deepFreeze({ ...frozen.lockfile });
    const rust = options.createLockedRustRuntime(
      worktrees.controlledRoot(), frozenLockfile, frozen.registryPackages,
    );
    const task = requireProgrammeV5Task(bindAcceptanceTaskToRustProfile(unboundTask, rust.profile));
    if (digestValue(task) !== digestValue(bindProgrammeTaskRuntimeV1(unboundTask))) {
      throw new Error('HARNESS_PROGRAMME_V5_RUST_RUNTIME_BINDING_MISMATCH');
    }
    const evidencePlan = resolveTaskEvidencePlanV1({ task, taskPath: controller.taskPath });
    await worktrees.installFrozenOverlay(
      frozenLockfile.sourcePath, frozenLockfile.workspacePath, frozenLockfile.digest, options.signal,
    );
    native = await options.createNativeSession({
      prepared, evaluatorPaths: task.evaluatorPaths, taskPath: controller.taskPath, models: options.models,
    });
    assertProgrammeV5NativeSession(native, options.models);
    assertProgrammeV5NativeExecutableBindings(
      native.ledger.preflightExecutableIdentitySnapshot(),
      baseToolVersions,
    );
    const pool = programmeV5RoutedPool(options.runId, task, native.candidates);
    const routeSnapshot = pool.freeze(ROUTED_STEPS);
    const routeSnapshotDigest = digestValue(routeSnapshot);
    const frozenAt = canonicalProgrammeTimestamp(
      (options.now ?? (() => new Date().toISOString()))(),
    );
    const captureNonce = randomBytes(32).toString('hex');
    const ruflo = parseProgrammeV5RufloEvidence(await options.captureRufloEvidence({
      taskId: task.taskId, runId: options.runId, routeSnapshotDigest, captureNonce,
      transactionStartedAt: frozenAt,
    }));
    if (ruflo.taskId !== task.taskId || ruflo.runId !== options.runId
      || ruflo.routeSnapshotDigest !== routeSnapshotDigest
      || ruflo.captureNonce !== captureNonce || ruflo.transactionStartedAt !== frozenAt
      || baseToolVersions.rufloHive !== ruflo.swarmStatus.topology
      || baseToolVersions.rufloConsensus !== ruflo.swarmStatus.config.consensusMechanism) {
      throw new Error('HARNESS_PROGRAMME_V5_RUFLO_BINDING_MISMATCH');
    }
    const protectedPaths = uniqueProgrammeV5Strings([
      ...SECURE_HARNESS_CONFIG.requiredProtectedPaths,
      ...task.evaluatorPaths,
      frozenLockfile.workspacePath,
    ]);
    const boundary = new GitProtectedInputBoundary({
      repositoryRoot: options.controllerRepositoryRoot,
      controllerCommit: options.controllerCommit,
      evaluatorRoot: prepared.evaluatorRoot,
      evaluatorPaths: [...task.evaluatorPaths, frozenLockfile.workspacePath],
    });
    const taskContract = programmeV5TaskContract(
      task, options.runId, prepared.candidateRoot, protectedPaths,
    );
    const protectedInputs = await boundary.capture(taskContract, SECURE_HARNESS_CONFIG);
    if (protectedInputs[controller.taskPath] !== controller.taskBlobDigest) {
      throw new Error('HARNESS_CONTROLLER_TASK_PROTECTED_INPUT_MISMATCH');
    }
    const policy = createFrozenProgrammePolicyV1({
      bootstrap: {
        controllerStoreDigest: bootstrap.controllerStoreDigest,
        nodeDigest: bootstrap.nodeDigest,
        gitDigest: bootstrap.gitDigest,
      },
      controller,
      execution: { evaluator, protectedInputs, routeSnapshot },
      taskEvidencePlanDigest: evidencePlan.declarationDigest,
      maxRepairs: 1,
    });
    const policyFingerprint = programmePolicyFingerprint(policy);
    const verifiedPolicy = verifyFrozenProgrammePolicyV1(policy, policyFingerprint);
    const policyBlob = canonicalProgrammePolicyJson(verifiedPolicy.snapshot);
    verifyFrozenProgrammePolicyV1(
      parseJsonWithoutDuplicateKeys(policyBlob, 'programme v5 prepared policy'),
      policyFingerprint,
    );
    const toolVersions = assembleProgrammeV5ToolVersions({
      contract: verifiedPolicy.snapshot.gateContract,
      base: baseToolVersions,
      lockedRust: rust.toolVersions,
      bootstrap,
      controller,
      taskEvidencePlanDigest: evidencePlan.declarationDigest,
      boundTaskDigest: verifiedPolicy.snapshot.execution.boundTaskDigest,
      policyFingerprint,
      frozenCargoLockDigest: frozenLockfile.digest,
      hosts: native.hosts,
    });
    const qe = options.createAgenticQeCollector({
      taskId: task.taskId, runId: options.runId, qeBindings: evidencePlan.qeBindings,
    });
    const model = new NativeRepositoryModelController({
      pool,
      candidates: native.candidates,
      clients: native.clients,
      architectureVerifiers: programmeV5ArchitectureVerifiers(),
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
      task, worktrees, config: SECURE_HARNESS_CONFIG,
      offlineIsolator: rust.profile.isolator, sourceEnvironment: rust.profile.environment,
    });
    const commands = programmeV5Commands(task);
    operations = new RepositoryCandidateOperations({
      worktrees,
      config: SECURE_HARNESS_CONFIG,
      baselineCommit: task.baseline.commit,
      evaluatorCommit: evaluator.commit,
      candidateExpectation: candidateExpectationForTask(task),
      taskForWorkspace: (root) => programmeV5TaskContract(task, options.runId, root, protectedPaths),
      buildCommands: task.commands.build.map(({ command }) => command),
      verifierCommands: {
        public: task.commands.public.map(({ command }) => command),
        independent: task.commands.independent.map(({ command }) => command),
        regression: task.commands.regression.map(({ command }) => command),
      },
      verifierGeneratedOutputs: evidencePlan.verifierGeneratedOutputs,
      artifactPaths: task.artifactPaths,
      model,
      offlineIsolator: rust.profile.isolator,
      offlineEnvironment: rust.profile.environment,
      protectedInputBoundary: boundary,
      frozenLockfile,
      assertExternalState: () => frozen?.assertStable(),
      cleanupCallbacks: [async () => await frozen?.cleanup(), async () => await native?.cleanup()],
      agenticQeEvidence: async (build, signal) => await qe(build, {
        controlledRoot: worktrees.controlledRoot(),
        candidateRoot: prepared.verifierRoots.independent,
        outputRoot: worktrees.outputRoot('independent'),
        rustProfile: rust.profile,
      }, signal),
      nativeRuntime: { ledger: native.ledger, taskId: task.taskId, runId: options.runId, hosts: native.hosts },
      preflightEvidence: async (candidate, signal) => await acceptance.redBaseline(candidate, signal),
      mutationEvidence: async (build, signal) => await acceptance.mutations(build, signal),
    });

    let state: 'prepared' | 'executing' | 'executed' | 'aborted' = 'prepared';
    let abortPromise: Promise<void> | null = null;
    const route = deepFreeze({
      snapshot: routeSnapshot,
      snapshotDigest: routeSnapshotDigest,
      frozenAt,
      routerVersion: '@metaharness/router@0.4.0' as const,
    });
    const execute = async (expectedBlob: string, expectedFingerprint: string) => {
      if (state !== 'prepared') throw new Error('HARNESS_PROGRAMME_V5_TRANSACTION_ALREADY_USED');
      state = 'executing';
      try {
        if (expectedBlob !== policyBlob) throw new Error('HARNESS_PROGRAMME_V5_POLICY_BLOB_MISMATCH');
        if (expectedFingerprint !== policyFingerprint) {
          throw new Error('HARNESS_PROGRAMME_V5_POLICY_FINGERPRINT_MISMATCH');
        }
        const externallyAnchored = verifyFrozenProgrammePolicyV1(
          parseJsonWithoutDuplicateKeys(expectedBlob, 'programme v5 execution policy'),
          expectedFingerprint,
        );
        if (canonicalProgrammePolicyJson(externallyAnchored.snapshot) !== policyBlob) {
          throw new Error('HARNESS_PROGRAMME_V5_POLICY_BLOB_MISMATCH');
        }
        await recheckPreparedState({
          options, bootstrap, controller, task, evaluator, prepared, frozen: frozen!, worktrees,
          frozenLockfile, boundary, taskContract, protectedInputs, pool, routeSnapshotDigest,
        });
        assertProgrammeV5ToolRecheck(baseToolVersions, options.recheckToolVersions());
        assertProgrammeV5NativeExecutableBindings(
          native!.ledger.preflightExecutableIdentitySnapshot(),
          baseToolVersions,
        );
        assembleProgrammeV5ToolVersions({
          contract: externallyAnchored.snapshot.gateContract,
          base: baseToolVersions,
          lockedRust: rust.toolVersions,
          bootstrap,
          controller,
          taskEvidencePlanDigest: evidencePlan.declarationDigest,
          boundTaskDigest: externallyAnchored.snapshot.execution.boundTaskDigest,
          policyFingerprint,
          frozenCargoLockDigest: frozenLockfile.digest,
          hosts: native!.hosts,
        });
        const candidate = new CandidateTransaction({
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
            hosts: [...native!.hosts],
            toolVersions,
            requiredQeProfiles: [...evidencePlan.requiredQeProfiles],
            rufloEvidence: ruflo,
          },
          operations: operations!,
          maxRepairs: 1,
          signal: options.signal,
          now: options.now,
        });
        const transaction = await candidate.execute();
        return deepFreeze({
          controller: controller.identity,
          evaluator,
          route,
          policy: externallyAnchored.snapshot,
          policyFingerprint,
          rufloEvidence: ruflo,
          transaction,
        });
      } finally {
        state = 'executed';
        await cleanup();
      }
    };
    const abort = async () => {
      if (state !== 'prepared') return await (abortPromise ?? Promise.resolve());
      state = 'aborted';
      abortPromise ??= cleanup();
      await abortPromise;
    };
    return Object.freeze({ policyBlob, policyFingerprint, execute, abort });
  } catch (error) {
    try { await cleanup(); } catch (cleanupError) {
      throw new AggregateError([error, cleanupError], 'HARNESS_PROGRAMME_V5_PREPARE_AND_CLEANUP_FAILED');
    }
    throw error;
  }
}

async function recheckPreparedState(input: Readonly<{
  options: ProgrammeV5DriverOptions;
  bootstrap: ProgrammeV5BootstrapInputs;
  controller: ControllerAttestation;
  task: AcceptanceTaskV3;
  evaluator: EvaluatorIdentity;
  prepared: PreparedWorktrees;
  frozen: Issue8FrozenLockLease;
  frozenLockfile: Issue8FrozenLockLease['lockfile'];
  worktrees: GitWorktreeSet;
  boundary: GitProtectedInputBoundary;
  taskContract: TaskContract;
  protectedInputs: Readonly<Record<string, string>>;
  pool: PersistentRoutedAgentPool;
  routeSnapshotDigest: string;
}>): Promise<void> {
  input.frozen.assertStable();
  if (input.frozen.lockfile.sourcePath !== input.frozenLockfile.sourcePath
    || input.frozen.lockfile.workspacePath !== input.frozenLockfile.workspacePath
    || input.frozen.lockfile.digest !== input.frozenLockfile.digest) {
    throw new Error('HARNESS_PROGRAMME_V5_FROZEN_LOCK_LEASE_CHANGED');
  }
  input.worktrees.verifyFrozenOverlay(
    input.frozenLockfile.workspacePath, input.frozenLockfile.digest,
  );
  await assertGitMaterializationSafe({
    repositoryRoot: input.options.repositoryRoot,
    commits: [input.options.controllerCommit, input.task.baseline.commit, input.evaluator.commit],
    signal: input.options.signal,
  });
  await assertProgrammeV5GitIdentity(
    input.options.repositoryRoot, input.task.baseline, 'BASELINE',
  );
  const candidate = await input.worktrees.candidateIdentity(input.options.signal);
  if (candidate.commit !== input.prepared.candidate.commit || candidate.tree !== input.prepared.candidate.tree) {
    throw new Error('HARNESS_PROGRAMME_V5_PREPARED_IDENTITY_CHANGED');
  }
  await input.worktrees.assertCandidateSourceStable([], input.options.signal);
  for (const stage of ['public', 'independent', 'regression'] as const) {
    await input.worktrees.assertVerifierSourceStable(stage, input.options.signal);
  }
  const currentController = await attestController({
    repositoryRoot: input.options.controllerSourceRoot,
    controllerRepositoryRoot: input.options.controllerRepositoryRoot,
    controllerCommit: input.options.controllerCommit,
    taskPath: input.options.taskPath,
    signal: input.options.signal,
  });
  assertProgrammeV5ControllerTask(currentController, input.options, input.bootstrap);
  if (digestValue(currentController) !== digestValue(input.controller)) {
    throw new Error('HARNESS_PROGRAMME_V5_CONTROLLER_ATTESTATION_CHANGED');
  }
  const protectedDecision = await input.boundary.verify(
    input.taskContract, SECURE_HARNESS_CONFIG, input.protectedInputs,
  );
  if (!protectedDecision.allow) {
    throw new Error(`HARNESS_PROGRAMME_V5_PROTECTED_INPUT_CHANGED:${protectedDecision.reasons.join('; ')}`);
  }
  if (digestValue(input.pool.freeze(ROUTED_STEPS)) !== input.routeSnapshotDigest) {
    throw new Error('HARNESS_PROGRAMME_V5_ROUTE_SNAPSHOT_CHANGED');
  }
}
