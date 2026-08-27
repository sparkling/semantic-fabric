// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { SECURE_HARNESS_CONFIG } from './config.js';
import {
  createStructuredFrozenCargoLockExecutor,
  prepareFrozenCargoLock,
} from './frozen-cargo-lock.js';
import { readIssue8FrozenCargoLock } from './frozen-cargo-lock-fixture.js';
import { createIssue8NativeSession } from './issue-8-native-session.js';
import { prepareIssue8RustRuntimeFactory } from './issue-8-rust-runtime.js';
import {
  ISSUE_8_FROZEN_LOCK_DIGEST,
  ISSUE_8_NATIVE_LIMITS,
  ISSUE_8_SYSTEM_PATHS,
  ISSUE_8_TARGET_TRIPLE,
  prepareCargoExtension,
} from './issue-8-system.js';
import {
  createProgrammeEnvelopeV5,
  finalizeProgrammeOutcomeV5,
  parseProgrammeEnvelopeV5,
  serializeProgrammeEnvelopeV5,
} from './programme-envelope-v5.js';
import {
  prepareProgrammeV5Transaction,
  type ProgrammeV5DriverResult,
  type ProgrammeV5PreparedTransaction,
} from './programme-v5-driver.js';
import {
  PROGRAMME_V5_AGENTIC_QE_VERSION,
  attestProgrammeV5SystemTools,
} from './programme-v5-system.js';
import { createProgrammeV5TaskQeCollector } from './programme-v5-qe.js';
import { collectProgrammeV5RufloEvidence } from './programme-v5-ruflo.js';
import {
  claimProgrammeV5Execution,
  readProgrammeV5PolicyReviewReceipt,
} from './programme-v5-policy-anchor.js';
import {
  assertAbsent,
  assertProgrammeV5ControlledRoot,
  cloneProgrammeV5ControllerRepository,
  createProgrammeV5ScratchRoot,
  parseProgrammeV5Bootstrap,
  parseProgrammeV5Invocation,
  prepareProgrammeV5ScratchLayout,
  parseProgrammeV5PolicyReviewInvocation,
  readProgrammeV5Diagnostics,
  removeProgrammeV5Scratch,
  verifyProgrammeV5ExpectedPolicyFingerprint,
  type ProgrammeV5BaseInvocation,
  type ProgrammeV5BootstrapEvidence,
  type ProgrammeV5Invocation,
} from './programme-v5-program-runtime.js';
import {
  PROGRAMME_V5_CLAIM_AUTHORITY_ROOT,
  programmeV5ArtifactPath,
  writeProgrammeV5PrivateArtifact,
} from './programme-v5-receipt-io.js';

const MODELS = Object.freeze({ codex: 'gpt-5.6-sol', claude: 'claude-sonnet-4-6' });

export { replayTrustedProgrammeV5 } from './programme-v5-replay.js';

export interface TrustedProgrammeV5Preparation {
  readonly policyBlob: string;
  execute(policyBlob: string): Promise<TrustedProgrammeV5Outcome>;
  abort(): Promise<void>;
}

export interface TrustedProgrammeV5PolicyReview {
  readonly policyBlob: string;
  readonly policyFingerprint: string;
}

export interface TrustedProgrammeV5Outcome {
  readonly status: 'pass' | 'fail' | 'gated' | 'cancelled';
  readonly reason: string | null;
  seal(): Promise<Readonly<{
    status: string;
    receiptPath: string;
    receiptDigest: string;
    programmeAcceptanceDigest: string;
    envelopeDigest: string;
    policyFingerprint: string;
    executionClaimDigest: string;
  }>>;
}

export async function prepareReviewableProgrammeV5Policy(
  argv: readonly string[],
  rawBootstrap: unknown,
): Promise<TrustedProgrammeV5PolicyReview> {
  const invocation = parseProgrammeV5PolicyReviewInvocation(argv);
  const bootstrap = parseProgrammeV5Bootstrap(rawBootstrap);
  assertBootstrapBinding(invocation, bootstrap);
  const receiptPath = programmeV5ArtifactPath(
    invocation.repositoryRoot, invocation.runId, 'execution',
  );
  assertAbsent(receiptPath, 'HARNESS_PROGRAMME_V5_RECEIPT_EXISTS');
  const scratch = await createProgrammeV5ScratchRoot();
  let prepared: ProgrammeV5PreparedTransaction | undefined;
  try {
    prepared = await prepareExecution(invocation, bootstrap, scratch);
  } catch (error) {
    let failure: unknown = error;
    try {
      await removeProgrammeV5Scratch(scratch, failure);
    } catch (cleanupError) {
      failure = new AggregateError(
        [failure, cleanupError],
        'HARNESS_PROGRAMME_V5_POLICY_REVIEW_AND_SCRATCH_CLEANUP_FAILED',
      );
    }
    throw failure;
  }
  const review = Object.freeze({
    policyBlob: prepared.policyBlob,
    policyFingerprint: prepared.policyFingerprint,
  });
  let failure: unknown = createHash('sha256').update(review.policyBlob, 'utf8').digest('hex')
    === review.policyFingerprint
    ? undefined
    : new Error('HARNESS_PROGRAMME_V5_POLICY_REVIEW_FINGERPRINT_MISMATCH');
  try {
    await prepared.abort();
  } catch (error) {
    failure = failure === undefined
      ? error
      : new AggregateError(
          [failure, error],
          'HARNESS_PROGRAMME_V5_POLICY_REVIEW_AND_TRANSACTION_CLEANUP_FAILED',
        );
  }
  try {
    await removeProgrammeV5Scratch(scratch, failure);
  } catch (cleanupError) {
    failure = failure === undefined
      ? cleanupError
      : new AggregateError(
          [failure, cleanupError],
          'HARNESS_PROGRAMME_V5_POLICY_REVIEW_AND_SCRATCH_CLEANUP_FAILED',
        );
  }
  if (failure !== undefined) throw failure;
  return review;
}

export async function prepareTrustedProgrammeV5(
  argv: readonly string[],
  rawBootstrap: unknown,
  claimAuthorityRoot = PROGRAMME_V5_CLAIM_AUTHORITY_ROOT,
): Promise<TrustedProgrammeV5Preparation> {
  const invocation = parseProgrammeV5Invocation(argv);
  const bootstrap = parseProgrammeV5Bootstrap(rawBootstrap);
  assertBootstrapBinding(invocation, bootstrap);
  const receiptPath = programmeV5ArtifactPath(
    invocation.repositoryRoot, invocation.runId, 'execution',
  );
  assertAbsent(receiptPath, 'HARNESS_PROGRAMME_V5_RECEIPT_EXISTS');
  const policyReviewReceipt = readProgrammeV5PolicyReviewReceipt(invocation, bootstrap);
  const executionClaim = claimProgrammeV5Execution(
    invocation, policyReviewReceipt, claimAuthorityRoot,
  );
  const scratch = await createProgrammeV5ScratchRoot();
  let prepared: ProgrammeV5PreparedTransaction | undefined;
  try {
    prepared = await prepareExecution(invocation, bootstrap, scratch);
    verifyProgrammeV5ExpectedPolicyFingerprint(invocation, prepared.policyFingerprint);
  } catch (error) {
    let failure: unknown = error;
    if (prepared !== undefined) {
      try { await prepared.abort(); } catch (cleanupError) {
        failure = new AggregateError(
          [failure, cleanupError],
          'HARNESS_PROGRAMME_V5_PREPARE_AND_TRANSACTION_CLEANUP_FAILED',
        );
      }
    }
    try {
      await removeProgrammeV5Scratch(scratch, failure);
    } catch (cleanupError) {
      failure = new AggregateError(
        [failure, cleanupError],
        'HARNESS_PROGRAMME_V5_PREPARE_AND_SCRATCH_CLEANUP_FAILED',
      );
    }
    throw failure;
  }
  let state: 'prepared' | 'executing' | 'closed' = 'prepared';
  let scratchPresent = true;
  const removeScratch = async (priorFailure?: unknown) => {
    if (!scratchPresent) return;
    if (await removeProgrammeV5Scratch(scratch, priorFailure)) scratchPresent = false;
  };
  return Object.freeze({
    policyBlob: prepared.policyBlob,
    async execute(policyBlob: string) {
      if (state !== 'prepared') throw new Error('HARNESS_PROGRAMME_V5_EXECUTION_REUSED');
      state = 'executing';
      let result: ProgrammeV5DriverResult | undefined;
      let failure: unknown;
      try {
        result = await prepared.execute(policyBlob, invocation.expectedPolicy.fingerprint);
      } catch (error) {
        failure = error;
      }
      try {
        await removeScratch(failure);
      } catch (cleanupError) {
        failure = failure === undefined
          ? cleanupError
          : new AggregateError(
              [failure, cleanupError],
              'HARNESS_PROGRAMME_V5_EXECUTION_AND_SCRATCH_CLEANUP_FAILED',
            );
      }
      state = 'closed';
      if (failure !== undefined) throw failure;
      if (result === undefined) throw new Error('HARNESS_PROGRAMME_V5_RESULT_MISSING');
      return await createOutcome(
        invocation, result, invocation.expectedPolicy.fingerprint, receiptPath,
        executionClaim.digest,
      );
    },
    async abort() {
      if (state !== 'prepared') return;
      state = 'closed';
      let failure: unknown;
      try {
        await prepared.abort();
      } catch (error) {
        failure = error;
      }
      try {
        await removeScratch(failure);
      } catch (cleanupError) {
        failure = failure === undefined
          ? cleanupError
          : new AggregateError(
              [failure, cleanupError],
              'HARNESS_PROGRAMME_V5_ABORT_AND_SCRATCH_CLEANUP_FAILED',
            );
      }
      if (failure !== undefined) throw failure;
    },
  });
}

async function prepareExecution(
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
        expectedDigest: ISSUE_8_FROZEN_LOCK_DIGEST,
        pinnedLockfileContents: await readIssue8FrozenCargoLock({
          controllerRepositoryRoot: transactionRepository,
          controllerCommit: invocation.controllerCommit,
          expectedDigest: task.rust.frozenLockSha256,
        }),
        targetTriple: ISSUE_8_TARGET_TRIPLE,
        executor: createStructuredFrozenCargoLockExecutor({
          config: SECURE_HARNESS_CONFIG,
          offlineIsolator: rustProfile.isolator,
          sourceEnvironment: {},
        }),
      }),
    createNativeSession: async ({ prepared: worktrees, evaluatorPaths, taskPath, models }) =>
      await createIssue8NativeSession({
        config: SECURE_HARNESS_CONFIG,
        controllerRoot: invocation.repositoryRoot,
        runtimeParent: paths.native,
        prepared: worktrees,
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

function assertBootstrapBinding(
  invocation: ProgrammeV5BaseInvocation,
  bootstrap: ProgrammeV5BootstrapEvidence,
): void {
  if (bootstrap.controllerCommit !== invocation.controllerCommit
    || bootstrap.taskPath !== invocation.taskPath) {
    throw new Error('HARNESS_PROGRAMME_V5_BOOTSTRAP_BINDING_MISMATCH');
  }
}

async function createOutcome(
  invocation: ProgrammeV5Invocation,
  result: ProgrammeV5DriverResult,
  policyFingerprint: string,
  receiptPath: string,
  executionClaimDigest: string,
): Promise<TrustedProgrammeV5Outcome> {
  if (result.policyFingerprint !== policyFingerprint) {
    throw new Error('HARNESS_PROGRAMME_V5_DRIVER_POLICY_ANCHOR_MISMATCH');
  }
  const diagnosticBlob = await readProgrammeV5Diagnostics(
    invocation.controllerStore,
    invocation.controllerCommit,
  );
  const envelope = createProgrammeEnvelopeV5({
    policy: result.policy,
    rufloEvidence: result.rufloEvidence,
    receipt: result.transaction.receipt,
    diagnosticBlob,
  }, policyFingerprint);
  const serialized = serializeProgrammeEnvelopeV5(envelope, policyFingerprint);
  const verified = parseProgrammeEnvelopeV5(serialized, policyFingerprint);
  if (verified.receiptChain.receipts.length !== 1
    || verified.receiptChain.receipts[0]?.digest !== result.transaction.receipt.digest) {
    throw new Error('HARNESS_PROGRAMME_V5_RECEIPT_CHAIN_INVALID');
  }
  const finalized = finalizeProgrammeOutcomeV5({
    expectedPolicyFingerprint: policyFingerprint,
    transactionStatus: result.transaction.status,
    transactionReason: result.transaction.reason,
    envelope: verified,
  });
  let sealed = false;
  return Object.freeze({
    ...finalized,
    async seal() {
      if (sealed) throw new Error('HARNESS_PROGRAMME_V5_OUTCOME_ALREADY_SEALED');
      assertAbsent(receiptPath, 'HARNESS_PROGRAMME_V5_RECEIPT_EXISTS');
      writeProgrammeV5PrivateArtifact(invocation.repositoryRoot, receiptPath, serialized);
      sealed = true;
      return Object.freeze({
        status: finalized.status,
        receiptPath,
        receiptDigest: result.transaction.receipt.digest,
        programmeAcceptanceDigest: verified.programmeAcceptanceDigest,
        envelopeDigest: verified.envelopeDigest,
        policyFingerprint,
        executionClaimDigest,
      });
    },
  });
}
