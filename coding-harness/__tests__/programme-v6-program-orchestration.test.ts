// SPDX-License-Identifier: MIT

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { diagnosticBlob } from './candidate-fixtures.js';
import { programmeV6Fixture } from './programme-v6-fixtures.js';
import { canonicalProgrammePolicyJson } from '../src/programme-v5-driver-support.js';

const mocks = vi.hoisted(() => ({
  prepareExecution: vi.fn(),
  parseInvocation: vi.fn(),
  parsePolicyReviewInvocation: vi.fn(),
  parseReplayInvocation: vi.fn(),
  parseBootstrap: vi.fn(),
  createScratch: vi.fn(),
  removeScratch: vi.fn(),
  readDiagnostics: vi.fn(),
  assertAbsent: vi.fn(),
  readReview: vi.fn(),
  claimExecution: vi.fn(),
  readExecutionClaim: vi.fn(),
  artifactPath: vi.fn(),
  requireArtifactPath: vi.fn(),
  assertArtifactAbsent: vi.fn(),
  readArtifact: vi.fn(),
  writeArtifact: vi.fn(),
}));

vi.mock('../src/programme-v6-base-execution.js', () => ({
  createProgrammeV6ScratchRoot: mocks.createScratch,
  prepareProgrammeV6BaseExecution: mocks.prepareExecution,
}));

vi.mock('../src/programme-v5-program-runtime.js', () => ({
  assertAbsent: mocks.assertAbsent,
  parseProgrammeV5Bootstrap: mocks.parseBootstrap,
  parseProgrammeV5Invocation: mocks.parseInvocation,
  parseProgrammeV5PolicyReviewInvocation: mocks.parsePolicyReviewInvocation,
  parseProgrammeV5ReplayInvocation: mocks.parseReplayInvocation,
  readProgrammeV5Diagnostics: mocks.readDiagnostics,
  removeProgrammeV5Scratch: mocks.removeScratch,
}));

vi.mock('../src/programme-v6-policy-anchor.js', () => ({
  claimProgrammeV6Execution: mocks.claimExecution,
  readProgrammeV6ExecutionClaim: mocks.readExecutionClaim,
  readProgrammeV6PolicyReviewReceipt: mocks.readReview,
}));

vi.mock('../src/programme-v6-receipt-io.js', () => ({
  PROGRAMME_V6_CLAIM_AUTHORITY_ROOT: '/authority',
  assertProgrammeV6ArtifactAbsent: mocks.assertArtifactAbsent,
  programmeV6ArtifactPath: mocks.artifactPath,
  readProgrammeV6PrivateArtifact: mocks.readArtifact,
  requireProgrammeV6ArtifactPath: mocks.requireArtifactPath,
  writeProgrammeV6PrivateArtifact: mocks.writeArtifact,
}));

import {
  createReviewableProgrammeV6Policy,
  prepareTrustedProgrammeV6,
} from '../src/programme-v6-program.js';

describe('programme V6 trusted orchestration', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.createScratch.mockResolvedValue('/scratch');
    mocks.removeScratch.mockResolvedValue(true);
    mocks.readDiagnostics.mockResolvedValue(diagnosticBlob);
    mocks.artifactPath.mockReturnValue('/repository/execution.json');
    mocks.requireArtifactPath.mockImplementation((...args: unknown[]) => args.at(-1));
    mocks.claimExecution.mockReturnValue({ path: '/claim', digest: 'c'.repeat(64) });
  });

  it('executes the exact embedded base policy and seals V6 transaction evidence', async () => {
    const setup = transactionSetup();
    bindPreparation(setup);

    const preparation = await prepareTrustedProgrammeV6([], setup.bootstrap);
    expect(preparation.policyBlob).toBe(setup.review.policyBlob);
    const outcome = await preparation.execute(setup.review.policyBlob);
    const sealed = await outcome.seal();

    expect(setup.execute).toHaveBeenCalledWith(
      setup.basePolicyBlob, setup.fixture.policy.base.fingerprint,
    );
    expect(setup.abort).not.toHaveBeenCalled();
    expect(mocks.removeScratch).toHaveBeenCalledWith('/scratch', undefined);
    expect(sealed.status).toBe('pass');
    expect(sealed.transactionStatus).toBe('pass');
    expect(sealed.policyFingerprint).toBe(setup.review.policyFingerprint);
    expect(sealed.candidateTransactionEvidenceDigest)
      .toBe(setup.fixture.candidateTransactionEvidence.evidenceDigest);
    expect(mocks.writeArtifact).toHaveBeenCalledOnce();
    await expect(preparation.execute(setup.review.policyBlob))
      .rejects.toThrow('HARNESS_PROGRAMME_V6_EXECUTION_REUSED');
  });

  it('aborts the base preparation when outer policy bytes are substituted', async () => {
    const setup = transactionSetup();
    bindPreparation(setup);
    const preparation = await prepareTrustedProgrammeV6([], setup.bootstrap);

    await expect(preparation.execute(`${setup.review.policyBlob}\n`))
      .rejects.toThrow('HARNESS_PROGRAMME_V6_POLICY_SERIALIZATION_INVALID');
    expect(setup.execute).not.toHaveBeenCalled();
    expect(setup.abort).toHaveBeenCalledOnce();
    expect(mocks.removeScratch).toHaveBeenCalledWith('/scratch', expect.any(Error));
  });
});

function transactionSetup() {
  const fixture = programmeV6Fixture();
  const basePolicyBlob = canonicalProgrammePolicyJson(fixture.policy.base.snapshot);
  const review = createReviewableProgrammeV6Policy(
    basePolicyBlob, fixture.policy.base.fingerprint,
  );
  const invocation = {
    repositoryRoot: '/repository',
    controllerStore: '/controller-store',
    controllerCommit: fixture.policy.base.snapshot.controller.identity.commit,
    taskPath: fixture.policy.base.snapshot.controller.taskPath,
    runId: fixture.receipt.runId,
    swarmId: fixture.receipt.coordination.swarmId,
    coordinationTaskId: fixture.receipt.coordination.taskId,
    hiveId: fixture.receipt.toolVersions.rufloHive,
    consensusId: fixture.receipt.toolVersions.rufloConsensus,
    policyReviewReceipt: '/repository/policy-review.json',
    expectedPolicy: {
      controllerCommit: fixture.policy.base.snapshot.controller.identity.commit,
      taskPath: fixture.policy.base.snapshot.controller.taskPath,
      fingerprint: review.policyFingerprint,
    },
  };
  const bootstrap = {
    schemaVersion: 3 as const,
    source: 'verified-packed-private-runtime' as const,
    controllerCommit: invocation.controllerCommit,
    taskPath: invocation.taskPath,
    controllerStoreDigest: fixture.policy.base.snapshot.bootstrap.controllerStoreDigest,
    buildManifestDigest: fixture.policy.base.snapshot.controller.buildManifestBlobDigest,
    runtimeTreeDigest: fixture.policy.base.snapshot.controller.runtimeTreeDigest,
    nodeDigest: fixture.policy.base.snapshot.bootstrap.nodeDigest,
    gitDigest: fixture.policy.base.snapshot.bootstrap.gitDigest,
  };
  const execute = vi.fn().mockResolvedValue({
    controller: fixture.policy.base.snapshot.controller.identity,
    evaluator: fixture.policy.base.snapshot.execution.evaluator,
    route: {
      snapshot: fixture.policy.base.routeSnapshot,
      snapshotDigest: fixture.policy.base.snapshot.execution.routeSnapshotDigest,
      frozenAt: fixture.receipt.route.frozenAt,
      routerVersion: '@metaharness/router@0.4.0' as const,
    },
    policy: fixture.policy.base.snapshot,
    policyFingerprint: fixture.policy.base.fingerprint,
    rufloEvidence: fixture.rufloEvidence,
    transaction: {
      status: fixture.receipt.status,
      reason: null,
      repairCount: 0,
      finalPatch: 'patch',
      receipt: fixture.receipt,
      repairTransitions: fixture.repairTransitions,
      transactionEvidence: fixture.candidateTransactionEvidence,
    },
  });
  const abort = vi.fn().mockResolvedValue(undefined);
  return { fixture, basePolicyBlob, review, invocation, bootstrap, execute, abort };
}

function bindPreparation(setup: ReturnType<typeof transactionSetup>): void {
  mocks.parseInvocation.mockReturnValue(setup.invocation);
  mocks.parseBootstrap.mockReturnValue(setup.bootstrap);
  mocks.readReview.mockReturnValue({
    policyFingerprint: setup.review.policyFingerprint,
    policyReviewReceiptDigest: 'b'.repeat(64),
  });
  mocks.prepareExecution.mockResolvedValue({
    policyBlob: setup.basePolicyBlob,
    policyFingerprint: setup.fixture.policy.base.fingerprint,
    execute: setup.execute,
    abort: setup.abort,
  });
}
