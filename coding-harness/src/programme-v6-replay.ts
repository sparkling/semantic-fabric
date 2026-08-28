// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { parseCandidateTransactionEvidenceV1 } from './candidate-transaction-evidence-v1.js';
import { attestController, type ControllerAttestation } from './controller-attestation.js';
import { DEVELOPMENT_AUTHORITY, deepFreeze } from './contracts.js';
import {
  finalizeProgrammeOutcomeV6,
  parseProgrammeEnvelopeV6,
  serializeProgrammeEnvelopeV6,
  type ProgrammeEnvelopeV6,
} from './programme-envelope-v6.js';
import {
  canonicalProgrammePolicyJson,
  assertProgrammeV5ControllerTask,
} from './programme-v5-driver-support.js';
import {
  parseProgrammeV5Bootstrap,
  parseProgrammeV5ReplayInvocation,
  type ProgrammeV5ReplayInvocation,
} from './programme-v5-program-runtime.js';
import { validProgrammeV5RufloBinding } from './programme-v5-ruflo-contract.js';
import {
  readProgrammeV6ExecutionClaim,
  readProgrammeV6PolicyReviewReceipt,
} from './programme-v6-policy-anchor.js';
import {
  PROGRAMME_V6_CLAIM_AUTHORITY_ROOT,
  assertProgrammeV6ArtifactAbsent,
  readProgrammeV6PrivateArtifact,
  requireProgrammeV6ArtifactPath,
  writeProgrammeV6PrivateArtifact,
} from './programme-v6-receipt-io.js';
import { verifyFrozenProgrammePolicyV2 } from './programme-policy-v6.js';

export interface TrustedProgrammeV6Replay {
  readonly status: 'pass' | 'fail' | 'gated' | 'cancelled';
  readonly reason: string | null;
  seal(): Promise<Readonly<{
    verificationStatus: 'verified';
    transactionStatus: 'pass' | 'fail' | 'gated' | 'cancelled';
    recordedStatus: 'pass' | 'fail' | 'gated' | 'cancelled';
    recordedReason: string | null;
    receiptPath: string;
    replayReceiptDigest: string;
    receiptDigest: string;
    envelopeDigest: string;
    policyFingerprint: string;
    basePolicyFingerprint: string;
    candidateTransactionEvidenceDigest: string | null;
    executionClaimDigest: string;
    launchReceiptDigest: string;
  }>>;
}

export async function replayTrustedProgrammeV6(
  argv: readonly string[],
  rawBootstrap: unknown,
  claimAuthorityRoot = PROGRAMME_V6_CLAIM_AUTHORITY_ROOT,
): Promise<TrustedProgrammeV6Replay> {
  const invocation = parseReplayInvocation(argv);
  const bootstrap = parseProgrammeV5Bootstrap(rawBootstrap);
  if (bootstrap.controllerCommit !== invocation.controllerCommit
    || bootstrap.taskPath !== invocation.taskPath) replayInvalid('BOOTSTRAP_BINDING');
  assertProgrammeV6ArtifactAbsent(
    invocation.replayReceipt, 'HARNESS_PROGRAMME_V6_REPLAY_RECEIPT_EXISTS',
  );
  const review = readProgrammeV6PolicyReviewReceipt(invocation, bootstrap);
  const claim = readProgrammeV6ExecutionClaim(invocation, review, claimAuthorityRoot);
  const serialized = readProgrammeV6PrivateArtifact(
    invocation.repositoryRoot, invocation.envelopeReceipt, 100_000_000,
  );
  const outerFingerprint = invocation.expectedPolicy.fingerprint;
  const envelope = parseProgrammeEnvelopeV6(serialized, outerFingerprint);
  if (serializeProgrammeEnvelopeV6(envelope, outerFingerprint) !== serialized) {
    replayInvalid('ENVELOPE_SERIALIZATION');
  }
  const policy = verifyFrozenProgrammePolicyV2(envelope.policy, outerFingerprint);
  if (review.policyFingerprint !== policy.fingerprint
    || envelope.policyFingerprint !== policy.fingerprint) replayInvalid('POLICY_BINDING');
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
  const transactionStatus = receipt.status;
  assertCandidateEvidenceBinding(envelope, transactionStatus);
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
  }) || receipt.runId !== invocation.runId || receipt.taskId !== policy.base.task.taskId
    || receipt.identities.controller.commit !== invocation.controllerCommit
    || receipt.identities.controller.tree !== controller.identity.tree
    || receipt.route.snapshotDigest !== policy.base.snapshot.execution.routeSnapshotDigest
    || receipt.coordination.swarmId !== invocation.swarmId
    || receipt.coordination.taskId !== invocation.coordinationTaskId
    || envelope.rufloEvidence.runId !== invocation.runId
    || envelope.rufloEvidence.taskId !== receipt.taskId
    || envelope.rufloEvidence.swarmId !== invocation.swarmId
    || envelope.rufloEvidence.coordinationTaskId !== invocation.coordinationTaskId
    || receipt.toolVersions.rufloHive !== invocation.hiveId
    || receipt.toolVersions.rufloConsensus !== invocation.consensusId
    || receipt.toolVersions.programmePolicyFingerprint !== policy.base.fingerprint
    || receipt.toolVersions.bootstrapControllerStoreDigest !== bootstrap.controllerStoreDigest
    || receipt.toolVersions.bootstrapBuildManifestDigest !== bootstrap.buildManifestDigest
    || receipt.toolVersions.bootstrapRuntimeTreeDigest !== bootstrap.runtimeTreeDigest
    || receipt.toolVersions.bootstrapNodeDigest !== bootstrap.nodeDigest
    || receipt.toolVersions.bootstrapGitDigest !== bootstrap.gitDigest) {
    replayInvalid('TRANSACTION_BINDING');
  }
  const transactionReason = transactionStatus === 'pass' ? null : receipt.failureCode;
  if (transactionStatus !== 'pass' && transactionReason === null) replayInvalid('FAILURE_REASON');
  const outcome = finalizeProgrammeOutcomeV6({
    expectedPolicyFingerprint: outerFingerprint,
    transactionStatus,
    transactionReason,
    envelope,
  });
  const launchReceiptDigest = sha256(canonicalProgrammePolicyJson({
    schemaVersion: 1,
    domain: 'semantic-fabric/programme-v6/replay-launch/v1',
    operation: 'programme-v6-replay-launch',
    controllerCommit: invocation.controllerCommit,
    taskPath: invocation.taskPath,
    outerPolicyFingerprint: outerFingerprint,
    basePolicyFingerprint: policy.base.fingerprint,
    envelopeDigest: envelope.envelopeDigest,
    transactionStatus,
    receiptDigest: receipt.digest,
    candidateTransactionEvidenceDigest: envelope.candidateTransactionEvidenceDigest,
    executionClaimDigest: claim.digest,
  }));
  const body = deepFreeze({
    schemaVersion: 1,
    envelopeSchemaVersion: 6,
    policySchemaVersion: 2,
    authority: DEVELOPMENT_AUTHORITY,
    operation: 'programme-v6-replay',
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
    transactionStatus,
    candidateTransactionEvidenceDigest: envelope.candidateTransactionEvidenceDigest,
    programmeAcceptanceDigest: envelope.programmeAcceptanceDigest,
    envelopeDigest: envelope.envelopeDigest,
    policyFingerprint: outerFingerprint,
    basePolicyFingerprint: policy.base.fingerprint,
    launchReceiptDigest,
    recordedStatus: outcome.status,
    recordedReason: outcome.reason,
  } as const);
  let sealed = false;
  return Object.freeze({
    status: outcome.status,
    reason: outcome.reason,
    async seal() {
      if (sealed) throw new Error('HARNESS_PROGRAMME_V6_REPLAY_ALREADY_SEALED');
      const replayReceiptDigest = sha256(canonicalProgrammePolicyJson(body));
      writeProgrammeV6PrivateArtifact(
        invocation.repositoryRoot,
        invocation.replayReceipt,
        `${JSON.stringify({ ...body, replayReceiptDigest })}\n`,
      );
      sealed = true;
      return Object.freeze({
        verificationStatus: 'verified' as const,
        transactionStatus,
        recordedStatus: outcome.status,
        recordedReason: outcome.reason,
        receiptPath: invocation.replayReceipt,
        replayReceiptDigest,
        receiptDigest: receipt.digest,
        envelopeDigest: envelope.envelopeDigest,
        policyFingerprint: outerFingerprint,
        basePolicyFingerprint: policy.base.fingerprint,
        candidateTransactionEvidenceDigest: envelope.candidateTransactionEvidenceDigest,
        executionClaimDigest: claim.digest,
        launchReceiptDigest,
      });
    },
  });
}

function parseReplayInvocation(argv: readonly string[]): ProgrammeV5ReplayInvocation {
  const invocation = parseProgrammeV5ReplayInvocation(argv);
  requireProgrammeV6ArtifactPath(
    invocation.repositoryRoot, invocation.runId, 'policy-review', invocation.policyReviewReceipt,
  );
  requireProgrammeV6ArtifactPath(
    invocation.repositoryRoot, invocation.runId, 'execution', invocation.envelopeReceipt,
  );
  requireProgrammeV6ArtifactPath(
    invocation.repositoryRoot, invocation.runId, 'replay', invocation.replayReceipt,
  );
  return invocation;
}

function assertCandidateEvidenceBinding(
  envelope: ProgrammeEnvelopeV6,
  transactionStatus: 'pass' | 'fail' | 'gated' | 'cancelled',
): void {
  const receipt = envelope.receiptChain.receipts[0];
  if (receipt === undefined) replayInvalid('RECEIPT_MISSING');
  if (receipt.status !== transactionStatus) replayInvalid('TRANSACTION_STATUS_BINDING');
  if (transactionStatus !== 'pass') {
    if (envelope.candidateTransactionEvidence !== null
      || envelope.candidateTransactionEvidenceDigest !== null) {
      replayInvalid('NONPASS_CANDIDATE_EVIDENCE');
    }
    return;
  }
  if (envelope.candidateTransactionEvidence === null
    || envelope.candidateTransactionEvidenceDigest === null) {
    replayInvalid('PASS_CANDIDATE_EVIDENCE');
  }
  const evidence = parseCandidateTransactionEvidenceV1(
    envelope.candidateTransactionEvidence,
    receipt,
  );
  if (evidence.evidenceDigest !== envelope.candidateTransactionEvidenceDigest
    || canonicalProgrammePolicyJson(evidence)
      !== canonicalProgrammePolicyJson(envelope.candidateTransactionEvidence)) {
    replayInvalid('CANDIDATE_EVIDENCE_BINDING');
  }
}

function assertControllerBindings(
  controller: ControllerAttestation,
  envelope: ProgrammeEnvelopeV6,
): void {
  const frozen = envelope.policy.basePolicy.controller;
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
  throw new Error(`HARNESS_PROGRAMME_V6_REPLAY_${reason}_INVALID`);
}
