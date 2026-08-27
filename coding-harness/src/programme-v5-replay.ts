// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { attestController, type ControllerAttestation } from './controller-attestation.js';
import { DEVELOPMENT_AUTHORITY, deepFreeze } from './contracts.js';
import {
  finalizeProgrammeOutcomeV5, parseProgrammeEnvelopeV5, serializeProgrammeEnvelopeV5,
  type ProgrammeEnvelopeV5,
} from './programme-envelope-v5.js';
import { verifyFrozenProgrammePolicyV1 } from './programme-policy-v5.js';
import {
  canonicalProgrammePolicyJson,
  assertProgrammeV5ControllerTask,
} from './programme-v5-driver-support.js';
import {
  readProgrammeV5ExecutionClaim,
  readProgrammeV5PolicyReviewReceipt,
} from './programme-v5-policy-anchor.js';
import {
  assertProgrammeV5ArtifactAbsent,
  readProgrammeV5PrivateArtifact,
  writeProgrammeV5PrivateArtifact,
} from './programme-v5-receipt-io.js';
import {
  parseProgrammeV5Bootstrap, parseProgrammeV5ReplayInvocation,
} from './programme-v5-program-runtime.js';
import { validProgrammeV5RufloBinding } from './programme-v5-ruflo-contract.js';

export interface TrustedProgrammeV5Replay {
  readonly status: 'pass' | 'fail' | 'gated' | 'cancelled';
  readonly reason: string | null;
  seal(): Promise<Readonly<{
    verificationStatus: 'verified';
    recordedStatus: 'pass' | 'fail' | 'gated' | 'cancelled';
    recordedReason: string | null;
    receiptPath: string;
    replayReceiptDigest: string;
    envelopeDigest: string;
    policyFingerprint: string;
    launchReceiptDigest: string;
  }>>;
}

export async function replayTrustedProgrammeV5(
  argv: readonly string[],
  rawBootstrap: unknown,
): Promise<TrustedProgrammeV5Replay> {
  const invocation = parseProgrammeV5ReplayInvocation(argv);
  const bootstrap = parseProgrammeV5Bootstrap(rawBootstrap);
  if (bootstrap.controllerCommit !== invocation.controllerCommit
    || bootstrap.taskPath !== invocation.taskPath) replayInvalid('BOOTSTRAP_BINDING');
  assertProgrammeV5ArtifactAbsent(
    invocation.replayReceipt, 'HARNESS_PROGRAMME_V5_REPLAY_RECEIPT_EXISTS',
  );
  const review = readProgrammeV5PolicyReviewReceipt(invocation, bootstrap);
  const claim = readProgrammeV5ExecutionClaim(invocation, review);
  const serialized = readProgrammeV5PrivateArtifact(
    invocation.repositoryRoot, invocation.envelopeReceipt, 100_000_000,
  );
  const fingerprint = invocation.expectedPolicy.fingerprint;
  const envelope = parseProgrammeEnvelopeV5(serialized, fingerprint);
  if (serializeProgrammeEnvelopeV5(envelope, fingerprint) !== serialized) {
    replayInvalid('ENVELOPE_SERIALIZATION');
  }
  const policy = verifyFrozenProgrammePolicyV1(envelope.policy, fingerprint);
  const controller = await attestController({
    repositoryRoot: invocation.repositoryRoot,
    controllerRepositoryRoot: invocation.controllerStore,
    controllerCommit: invocation.controllerCommit,
    taskPath: invocation.taskPath,
  });
  assertProgrammeV5ControllerTask(controller, invocation, bootstrap);
  assertControllerBindings(controller, envelope);
  const receipt = envelope.receiptChain.receipts[0];
  if (receipt === undefined) replayInvalid('RECEIPT_MISSING');
  if (!validProgrammeV5RufloBinding(envelope.rufloEvidence, {
    taskId: receipt.taskId,
    runId: receipt.runId,
    routeSnapshotDigest: receipt.route.snapshotDigest,
    swarmId: receipt.coordination.swarmId,
    coordinationTaskId: receipt.coordination.taskId,
    hookIds: receipt.coordination.hookIds,
    traceIds: receipt.coordination.traceIds,
    transactionStartedAt: receipt.route.frozenAt,
    receiptIssuedAt: receipt.issuedAt,
  }) || receipt.runId !== invocation.runId || receipt.taskId !== policy.task.taskId
    || receipt.identities.controller.commit !== invocation.controllerCommit
    || receipt.identities.controller.tree !== controller.identity.tree
    || receipt.coordination.swarmId !== invocation.swarmId
    || receipt.coordination.taskId !== invocation.coordinationTaskId
    || envelope.rufloEvidence.runId !== invocation.runId
    || envelope.rufloEvidence.swarmId !== invocation.swarmId
    || envelope.rufloEvidence.coordinationTaskId !== invocation.coordinationTaskId
    || receipt.toolVersions.rufloHive !== invocation.hiveId
    || receipt.toolVersions.rufloConsensus !== invocation.consensusId
    || receipt.toolVersions.programmePolicyFingerprint !== fingerprint
    || receipt.toolVersions.bootstrapControllerStoreDigest !== bootstrap.controllerStoreDigest
    || receipt.toolVersions.bootstrapBuildManifestDigest !== bootstrap.buildManifestDigest
    || receipt.toolVersions.bootstrapRuntimeTreeDigest !== bootstrap.runtimeTreeDigest
    || receipt.toolVersions.bootstrapNodeDigest !== bootstrap.nodeDigest
    || receipt.toolVersions.bootstrapGitDigest !== bootstrap.gitDigest) {
    replayInvalid('TRANSACTION_BINDING');
  }
  const transactionReason = receipt.status === 'pass' ? null : receipt.failureCode;
  if (receipt.status !== 'pass' && transactionReason === null) replayInvalid('FAILURE_REASON');
  const outcome = finalizeProgrammeOutcomeV5({
    expectedPolicyFingerprint: fingerprint,
    transactionStatus: receipt.status,
    transactionReason,
    envelope,
  });
  const launchReceiptDigest = sha256(canonicalProgrammePolicyJson({
    controllerCommit: invocation.controllerCommit,
    taskPath: invocation.taskPath,
    policyFingerprint: fingerprint,
    envelopeDigest: envelope.envelopeDigest,
    executionClaimDigest: claim.digest,
  }));
  const body = deepFreeze({
    schemaVersion: 1,
    authority: DEVELOPMENT_AUTHORITY,
    operation: 'programme-v5-replay',
    verificationStatus: 'verified',
    controllerCommit: invocation.controllerCommit,
    controllerTree: controller.identity.tree,
    taskPath: invocation.taskPath,
    runId: invocation.runId,
    swarmId: invocation.swarmId,
    coordinationTaskId: invocation.coordinationTaskId,
    hiveId: invocation.hiveId,
    consensusId: invocation.consensusId,
    controllerStoreDigest: bootstrap.controllerStoreDigest,
    buildManifestDigest: bootstrap.buildManifestDigest,
    runtimeTreeDigest: bootstrap.runtimeTreeDigest,
    nodeDigest: bootstrap.nodeDigest,
    gitDigest: bootstrap.gitDigest,
    policyReviewReceiptDigest: review.policyReviewReceiptDigest,
    executionClaimDigest: claim.digest,
    envelopeFileDigest: sha256(serialized),
    receiptDigest: receipt.digest,
    programmeAcceptanceDigest: envelope.programmeAcceptanceDigest,
    envelopeDigest: envelope.envelopeDigest,
    policyFingerprint: fingerprint,
    launchReceiptDigest,
    recordedStatus: outcome.status,
    recordedReason: outcome.reason,
  } as const);
  let sealed = false;
  return Object.freeze({
    status: outcome.status,
    reason: outcome.reason,
    async seal() {
      if (sealed) throw new Error('HARNESS_PROGRAMME_V5_REPLAY_ALREADY_SEALED');
      const replayReceiptDigest = sha256(canonicalProgrammePolicyJson(body));
      writeProgrammeV5PrivateArtifact(
        invocation.repositoryRoot,
        invocation.replayReceipt,
        `${JSON.stringify({ ...body, replayReceiptDigest })}\n`,
      );
      sealed = true;
      return Object.freeze({
        verificationStatus: 'verified' as const,
        recordedStatus: outcome.status,
        recordedReason: outcome.reason,
        receiptPath: invocation.replayReceipt,
        replayReceiptDigest,
        envelopeDigest: envelope.envelopeDigest,
        policyFingerprint: fingerprint,
        launchReceiptDigest,
      });
    },
  });
}

function assertControllerBindings(
  controller: ControllerAttestation,
  envelope: ProgrammeEnvelopeV5,
): void {
  const frozen = envelope.policy.controller;
  if (frozen.identity.commit !== controller.identity.commit
    || frozen.identity.tree !== controller.identity.tree
    || frozen.manifestPath !== controller.manifestPath
    || frozen.manifestBlob !== controller.manifestBlob
    || frozen.manifestBlobDigest !== controller.manifestBlobDigest
    || frozen.taskPath !== controller.taskPath || frozen.taskBlob !== controller.taskBlob
    || frozen.taskBlobDigest !== controller.taskBlobDigest
    || frozen.buildManifestPath !== controller.buildManifestPath
    || frozen.buildManifestBlob !== controller.buildManifestBlob
    || frozen.buildManifestBlobDigest !== controller.buildManifestBlobDigest
    || frozen.runtimeTreeDigest !== controller.build.runtimeTreeDigest
    || frozen.lockfileDigest !== controller.build.lockfileDigest
    || frozen.executionDigest !== controller.executionDigest) {
    replayInvalid('CONTROLLER_BINDING');
  }
}

function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}

function replayInvalid(reason: string): never {
  throw new Error(`HARNESS_PROGRAMME_V5_REPLAY_${reason}_INVALID`);
}
