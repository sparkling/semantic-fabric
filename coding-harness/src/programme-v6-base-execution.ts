// SPDX-License-Identifier: MIT

import { chmodSync, lstatSync, realpathSync } from 'node:fs';
import { mkdir, mkdtemp, rm } from 'node:fs/promises';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { SECURE_HARNESS_CONFIG } from './config.js';
import {
  createStructuredFrozenCargoLockExecutor,
  prepareFrozenCargoLock,
} from './frozen-cargo-lock.js';
import { readTaskFrozenCargoLock } from './frozen-cargo-lock-fixture.js';
import { createIssue8NativeSession } from './issue-8-native-session.js';
import { prepareIssue8RustRuntimeFactory } from './issue-8-rust-runtime.js';
import {
  ISSUE_8_NATIVE_LIMITS,
  ISSUE_8_SYSTEM_PATHS,
  ISSUE_8_TARGET_TRIPLE,
  prepareCargoExtension,
} from './issue-8-system.js';
import {
  prepareProgrammeV5Transaction,
  type ProgrammeV5PreparedTransaction,
} from './programme-v5-driver.js';
import { createProgrammeV5TaskQeCollector } from './programme-v5-qe.js';
import { collectProgrammeV5RufloEvidence } from './programme-v5-ruflo.js';
import {
  assertProgrammeV5ControlledRoot,
  cloneProgrammeV5ControllerRepository,
  prepareProgrammeV5ScratchLayout,
  type ProgrammeV5BaseInvocation,
  type ProgrammeV5BootstrapEvidence,
} from './programme-v5-program-runtime.js';
import {
  PROGRAMME_V5_AGENTIC_QE_VERSION,
  attestProgrammeV5SystemTools,
} from './programme-v5-system.js';

const MODELS = Object.freeze({ codex: 'gpt-5.6-sol', claude: 'claude-sonnet-4-6' });
const SCRATCH_PARENT = '/home/claude/.cache/semantic-fabric-harness';

export async function createProgrammeV6ScratchRoot(): Promise<string> {
  await mkdir(SCRATCH_PARENT, { recursive: true, mode: 0o700 });
  privateDirectory(SCRATCH_PARENT, 'SCRATCH_PARENT');
  let root: string | undefined;
  try {
    root = await mkdtemp(join(SCRATCH_PARENT, 'v6-'));
    chmodSync(root, 0o700);
    return privateDirectory(root, 'SCRATCH_ROOT');
  } catch (error) {
    if (root === undefined) throw error;
    let cleanupError: unknown;
    try {
      const stat = lstatSync(root);
      if (dirname(root) !== SCRATCH_PARENT || !basename(root).startsWith('v6-')
        || !stat.isDirectory() || stat.isSymbolicLink() || realpathSync(root) !== root) {
        throw new Error('HARNESS_PROGRAMME_V6_SCRATCH_ROOT_CHANGED');
      }
      await rm(root, { recursive: true, force: true });
    } catch (caught) { cleanupError = caught; }
    if (cleanupError !== undefined) {
      throw new AggregateError(
        [error, cleanupError], 'HARNESS_PROGRAMME_V6_SCRATCH_ALLOCATION_AND_CLEANUP_FAILED',
      );
    }
    throw error;
  }
}

export async function prepareProgrammeV6BaseExecution(
  invocation: ProgrammeV5BaseInvocation,
  bootstrap: ProgrammeV5BootstrapEvidence,
  scratch: string,
): Promise<ProgrammeV5PreparedTransaction> {
  const paths = await prepareProgrammeV5ScratchLayout(scratch);
  const transactionRepository = await cloneProgrammeV5ControllerRepository(
    invocation.controllerStore,
    paths.repository,
    paths.gitTemplate,
    invocation.controllerCommit,
  );
  const cargoExtensionRoot = prepareCargoExtension(scratch);
  const rust = prepareIssue8RustRuntimeFactory({ scratchRoot: scratch, cargoExtensionRoot });
  const proxyLauncher = join(dirname(fileURLToPath(import.meta.url)), 'native-proxy-launcher.js');
  return await prepareProgrammeV5Transaction({
    repositoryRoot: transactionRepository,
    controllerSourceRoot: invocation.repositoryRoot,
    controllerRepositoryRoot: invocation.controllerStore,
    runRoot: paths.worktrees,
    evaluatorScratchRoot: paths.evaluator,
    controllerCommit: invocation.controllerCommit,
    taskPath: invocation.taskPath,
    runId: invocation.runId,
    models: MODELS,
    bootstrap: {
      controllerStoreDigest: bootstrap.controllerStoreDigest,
      nodeDigest: bootstrap.nodeDigest,
      gitDigest: bootstrap.gitDigest,
    },
    toolVersions: {
      ...attestProgrammeV5SystemTools(),
      ...rust.bootstrapEvidence,
      bootstrapSource: bootstrap.source,
      bootstrapControllerStoreDigest: bootstrap.controllerStoreDigest,
      bootstrapBuildManifestDigest: bootstrap.buildManifestDigest,
      bootstrapRuntimeTreeDigest: bootstrap.runtimeTreeDigest,
      bootstrapNodeDigest: bootstrap.nodeDigest,
      bootstrapGitDigest: bootstrap.gitDigest,
      rufloHive: invocation.hiveId,
      rufloConsensus: invocation.consensusId,
      agenticQe: PROGRAMME_V5_AGENTIC_QE_VERSION,
    },
    recheckToolVersions: attestProgrammeV5SystemTools,
    createBootstrapRustProfile: (controlledRoot) =>
      rust.createBootstrapProfile(assertProgrammeV5ControlledRoot(scratch, controlledRoot)),
    createLockedRustRuntime: (controlledRoot, lockfile, packages) =>
      rust.createLockedRuntime(
        assertProgrammeV5ControlledRoot(scratch, controlledRoot), lockfile, packages,
      ),
    prepareFrozenLockfile: async ({ task, evaluator, rustProfile }) =>
      await prepareFrozenCargoLock({
        repositoryRoot: transactionRepository,
        scratchRoot: paths.frozen,
        baseline: task.baseline,
        source: evaluator,
        cargoExecutable: rustProfile.cargoExecutable,
        cargoEnvironment: rustProfile.environment,
        config: SECURE_HARNESS_CONFIG,
        expectedDigest: task.rust.frozenLockSha256,
        pinnedLockfileContents: await readTaskFrozenCargoLock({
          controllerRepositoryRoot: transactionRepository,
          controllerCommit: invocation.controllerCommit,
          taskPath: invocation.taskPath,
          baselineCommit: task.baseline.commit,
          expectedDigest: task.rust.frozenLockSha256,
        }),
        targetTriple: ISSUE_8_TARGET_TRIPLE,
        executor: createStructuredFrozenCargoLockExecutor({
          config: SECURE_HARNESS_CONFIG,
          offlineIsolator: rustProfile.isolator,
          sourceEnvironment: {},
        }),
      }),
    createNativeSession: async ({ prepared, evaluatorPaths, taskPath, models }) =>
      await createIssue8NativeSession({
        config: SECURE_HARNESS_CONFIG,
        controllerRoot: invocation.repositoryRoot,
        runtimeParent: paths.native,
        prepared,
        evaluatorPaths,
        taskPath,
        models,
        executables: {
          codex: ISSUE_8_SYSTEM_PATHS.codex,
          claude: ISSUE_8_SYSTEM_PATHS.claude,
          node: ISSUE_8_SYSTEM_PATHS.node,
          bwrap: ISSUE_8_SYSTEM_PATHS.bwrap,
          systemdRun: ISSUE_8_SYSTEM_PATHS.systemdRun,
          systemctl: ISSUE_8_SYSTEM_PATHS.systemctl,
          proxyLauncher,
        },
        credentials: {
          codex: ISSUE_8_SYSTEM_PATHS.codexCredential,
          'claude-code': ISSUE_8_SYSTEM_PATHS.claudeCredential,
        },
        resourceLimits: ISSUE_8_NATIVE_LIMITS,
        controllerEnvironment: process.env,
      }),
    captureRufloEvidence: async ({
      taskId, runId, routeSnapshotDigest, captureNonce, transactionStartedAt,
    }) =>
      await collectProgrammeV5RufloEvidence({
        repositoryRoot: invocation.repositoryRoot,
        taskId,
        runId,
        routeSnapshotDigest,
        captureNonce,
        transactionStartedAt,
        swarmId: invocation.swarmId,
        coordinationTaskId: invocation.coordinationTaskId,
        hookIds: [],
        traceIds: [],
      }),
    createAgenticQeCollector: ({ taskId, runId, qeBindings }) =>
      createProgrammeV5TaskQeCollector({
        taskId,
        runId,
        qeBindings,
        snapshotParent: paths.sast,
        nodeExecutable: ISSUE_8_SYSTEM_PATHS.node,
        bwrapExecutable: ISSUE_8_SYSTEM_PATHS.bwrap,
        packageRoot: ISSUE_8_SYSTEM_PATHS.agenticQeRoot,
        mcpExecutable: ISSUE_8_SYSTEM_PATHS.agenticQeMcp,
      }),
  });
}

function privateDirectory(value: string, label: string): string {
  if (resolve(value) !== value || value.includes('\0')) {
    throw new Error(`HARNESS_PROGRAMME_V6_${label}_INVALID`);
  }
  const stat = lstatSync(value);
  const uid = process.getuid?.() ?? stat.uid;
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(value) !== value
    || stat.uid !== uid || (stat.mode & 0o077) !== 0) {
    throw new Error(`HARNESS_PROGRAMME_V6_${label}_INVALID`);
  }
  return value;
}
