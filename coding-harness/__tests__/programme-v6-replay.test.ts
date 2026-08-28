// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createCandidateTransactionEvidenceV1 } from '../src/candidate-transaction-evidence-v1.js';
import type { ControllerAttestation } from '../src/controller-attestation.js';
import {
  createProgrammeEnvelopeV6, serializeProgrammeEnvelopeV6,
} from '../src/programme-envelope-v6.js';
import { canonicalProgrammePolicyJson } from '../src/programme-v5-driver-support.js';
import type { ProgrammeV5BootstrapEvidence } from '../src/programme-v5-program-runtime.js';
import {
  claimProgrammeV6Execution,
  readProgrammeV6PolicyReviewReceipt,
  type ProgrammeV6PolicyInvocation,
} from '../src/programme-v6-policy-anchor.js';
import {
  createFrozenProgrammePolicyV2,
  programmePolicyV2Fingerprint,
  verifyFrozenProgrammePolicyV2,
  type ParsedProgrammePolicyV2,
} from '../src/programme-policy-v6.js';
import {
  programmePolicyFingerprint, type FrozenProgrammePolicyV1,
} from '../src/programme-policy-v5.js';
import {
  programmeV6ArtifactPath, writeProgrammeV6PrivateArtifact,
} from '../src/programme-v6-receipt-io.js';
import { replayTrustedProgrammeV6 } from '../src/programme-v6-replay.js';
import { digestValue, type Receipt } from '../src/receipts.js';
import { diagnosticBlob, programmeV5RufloFixture } from './candidate-fixtures.js';
import { programmeV6Fixture, rehashReceipt } from './programme-v6-fixtures.js';

const mocks = vi.hoisted(() => ({ attestController: vi.fn() }));
vi.mock('../src/controller-attestation.js', async (importOriginal) => ({
  ...await importOriginal<typeof import('../src/controller-attestation.js')>(),
  attestController: mocks.attestController,
}));

const roots: string[] = [];

afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('programme-v6 deterministic replay', () => {
  it('recomputes provider-free and seals a canonical V6-domain receipt', async () => {
    const fixture = replayFixture();
    const fetch = vi.fn();
    vi.stubGlobal('fetch', fetch);

    const replay = await replayTrustedProgrammeV6(
      fixture.argv, fixture.bootstrap, fixture.authority,
    );
    expect(replay).toMatchObject({ status: 'pass', reason: null });
    const sealed = await replay.seal();

    expect(fetch).not.toHaveBeenCalled();
    expect(mocks.attestController).toHaveBeenCalledTimes(1);
    expect(sealed).toMatchObject({
      verificationStatus: 'verified',
      transactionStatus: 'pass',
      recordedStatus: 'pass',
      policyFingerprint: fixture.policy.fingerprint,
      candidateTransactionEvidenceDigest:
        fixture.envelope.candidateTransactionEvidenceDigest,
    });
    const bytes = readFileSync(sealed.receiptPath, 'utf8');
    const parsed = JSON.parse(bytes) as Record<string, unknown>;
    const { replayReceiptDigest, ...body } = parsed;
    expect(bytes).toBe(`${JSON.stringify(parsed)}\n`);
    expect(replayReceiptDigest).toBe(sha256(canonicalProgrammePolicyJson(body)));
    expect(parsed.launchReceiptDigest).toBe(expectedLaunchReceiptDigest(parsed));
    expect(parsed).toMatchObject({
      schemaVersion: 1,
      envelopeSchemaVersion: 6,
      policySchemaVersion: 2,
      operation: 'programme-v6-replay',
      policyFingerprint: fixture.policy.fingerprint,
      basePolicyFingerprint: fixture.policy.base.fingerprint,
      receiptDigest: fixture.receipt.digest,
      transactionStatus: 'pass',
      recordedStatus: 'pass',
      candidateTransactionEvidenceDigest:
        fixture.envelope.candidateTransactionEvidenceDigest,
    });
    await expect(replay.seal()).rejects.toThrow('HARNESS_PROGRAMME_V6_REPLAY_ALREADY_SEALED');
  });

  it('allows exact null evidence for a non-pass receipt', async () => {
    const fixture = replayFixture({ status: 'fail' });
    const replay = await replayTrustedProgrammeV6(
      fixture.argv, fixture.bootstrap, fixture.authority,
    );
    const sealed = await replay.seal();

    expect(replay).toEqual(expect.objectContaining({
      status: 'fail', reason: 'HARNESS_TRANSACTION_FAILED',
    }));
    expect(sealed).toMatchObject({ transactionStatus: 'fail', recordedStatus: 'fail' });
    expect(sealed.candidateTransactionEvidenceDigest).toBeNull();
    expect(JSON.parse(readFileSync(sealed.receiptPath, 'utf8')))
      .toMatchObject({
        transactionStatus: 'fail', recordedStatus: 'fail',
        candidateTransactionEvidenceDigest: null,
      });
  });

  it('keeps pass transaction evidence when acceptance records a gated outcome', async () => {
    const fixture = replayFixture({ acceptanceRejected: true });
    expect(fixture.receipt.status).toBe('pass');
    expect(fixture.envelope.programmeAcceptance.status).toBe('REJECTED');
    expect(fixture.envelope.candidateTransactionEvidenceDigest).not.toBeNull();

    const replay = await replayTrustedProgrammeV6(
      fixture.argv, fixture.bootstrap, fixture.authority,
    );
    const sealed = await replay.seal();

    expect(replay).toEqual(expect.objectContaining({
      status: 'gated', reason: 'HARNESS_PROGRAMME_ACCEPTANCE_REJECTED',
    }));
    expect(sealed).toMatchObject({
      transactionStatus: 'pass',
      recordedStatus: 'gated',
      recordedReason: 'HARNESS_PROGRAMME_ACCEPTANCE_REJECTED',
      candidateTransactionEvidenceDigest:
        fixture.envelope.candidateTransactionEvidenceDigest,
    });
    expect(JSON.parse(readFileSync(sealed.receiptPath, 'utf8'))).toMatchObject({
      transactionStatus: 'pass',
      recordedStatus: 'gated',
      candidateTransactionEvidenceDigest:
        fixture.envelope.candidateTransactionEvidenceDigest,
    });
  });

  it('keeps genuine gated transaction status distinct with null evidence', async () => {
    const fixture = replayFixture({ status: 'gated' });
    expect(fixture.envelope.candidateTransactionEvidence).toBeNull();

    const replay = await replayTrustedProgrammeV6(
      fixture.argv, fixture.bootstrap, fixture.authority,
    );
    const sealed = await replay.seal();

    expect(replay).toEqual(expect.objectContaining({
      status: 'gated', reason: 'HARNESS_ACCEPTANCE_GATE_FAILED',
    }));
    expect(sealed).toMatchObject({
      transactionStatus: 'gated',
      recordedStatus: 'gated',
      recordedReason: 'HARNESS_ACCEPTANCE_GATE_FAILED',
      candidateTransactionEvidenceDigest: null,
    });
    expect(JSON.parse(readFileSync(sealed.receiptPath, 'utf8'))).toMatchObject({
      transactionStatus: 'gated',
      recordedStatus: 'gated',
      candidateTransactionEvidenceDigest: null,
    });
  });

  it('rejects pass evidence absence and a substituted evidence digest', async () => {
    for (const mutation of ['missing', 'digest'] as const) {
      const fixture = replayFixture();
      const document = JSON.parse(fixture.serialized) as Record<string, unknown>;
      if (mutation === 'missing') {
        document.candidateTransactionEvidence = null;
        document.candidateTransactionEvidenceDigest = null;
      } else {
        document.candidateTransactionEvidenceDigest = 'e'.repeat(64);
      }
      overwriteEnvelope(fixture, `${JSON.stringify(document, null, 2)}\n`);

      await expect(replayTrustedProgrammeV6(
        fixture.argv, fixture.bootstrap, fixture.authority,
      )).rejects.toThrow();
    }
  });

  it('rejects non-canonical envelope bytes and a schema-V5 downgrade', async () => {
    const loose = replayFixture();
    overwriteEnvelope(loose, loose.serialized.trimEnd());
    await expect(replayTrustedProgrammeV6(
      loose.argv, loose.bootstrap, loose.authority,
    )).rejects.toThrow('HARNESS_PROGRAMME_V6_REPLAY_ENVELOPE_SERIALIZATION_INVALID');

    const downgraded = replayFixture();
    const document = JSON.parse(downgraded.serialized) as Record<string, unknown>;
    document.schemaVersion = 5;
    overwriteEnvelope(downgraded, `${JSON.stringify(document, null, 2)}\n`);
    await expect(replayTrustedProgrammeV6(
      downgraded.argv, downgraded.bootstrap, downgraded.authority,
    )).rejects.toThrow('HARNESS_PROGRAMME_ENVELOPE_V6_IDENTITY_INVALID');
  });

  it('rejects the inner V1 fingerprint as the expected outer anchor', async () => {
    const fixture = replayFixture();
    const argv = replaceFlag(
      fixture.argv, '--expected-policy-fingerprint', fixture.policy.base.fingerprint,
    );
    await expect(replayTrustedProgrammeV6(
      argv, fixture.bootstrap, fixture.authority,
    )).rejects.toThrow('HARNESS_PROGRAMME_V6_POLICY_REVIEW_RECEIPT_INVALID');
  });

  it('rejects V5 review and claim identity substitution', async () => {
    const reviewFixture = replayFixture();
    const review = JSON.parse(readFileSync(reviewFixture.reviewPath, 'utf8')) as any;
    review.operation = 'programme-v5-policy-review';
    writeFileSync(reviewFixture.reviewPath, `${JSON.stringify(review)}\n`);
    await expect(replayTrustedProgrammeV6(
      reviewFixture.argv, reviewFixture.bootstrap, reviewFixture.authority,
    )).rejects.toThrow('HARNESS_PROGRAMME_V6_POLICY_REVIEW_RECEIPT_INVALID');

    const claimFixture = replayFixture();
    const claim = JSON.parse(readFileSync(claimFixture.claimPath, 'utf8')) as any;
    claim.operation = 'programme-v5-execution-claim';
    writeFileSync(claimFixture.claimPath, `${JSON.stringify(claim)}\n`);
    await expect(replayTrustedProgrammeV6(
      claimFixture.argv, claimFixture.bootstrap, claimFixture.authority,
    )).rejects.toThrow('HARNESS_PROGRAMME_V6_EXECUTION_CLAIM_INVALID');
  });

  it('rejects a review-authorized Ruflo identity not present in the receipt', async () => {
    const fixture = replayFixture({ invocationSwarmId: 'programme_v6_other_swarm' });
    await expect(replayTrustedProgrammeV6(
      fixture.argv, fixture.bootstrap, fixture.authority,
    )).rejects.toThrow('HARNESS_PROGRAMME_V6_REPLAY_TRANSACTION_BINDING_INVALID');
  });
});

interface ReplayFixture {
  readonly repository: string;
  readonly authority: string;
  readonly argv: readonly string[];
  readonly bootstrap: ProgrammeV5BootstrapEvidence;
  readonly policy: ParsedProgrammePolicyV2;
  readonly receipt: Receipt;
  readonly envelope: ReturnType<typeof createProgrammeEnvelopeV6>;
  readonly serialized: string;
  readonly envelopePath: string;
  readonly reviewPath: string;
  readonly claimPath: string;
}

function replayFixture(options: Readonly<{
  status?: 'pass' | 'fail' | 'gated';
  acceptanceRejected?: boolean;
  invocationSwarmId?: string;
}> = {}): ReplayFixture {
  const source = programmeV6Fixture();
  const runId = source.receipt.runId;
  const routeSnapshot = {
    historyEpoch: 0,
    decisions: Object.fromEntries(['architecture', 'implementation', 'repair'].map(
      (step) => [step, { runId, stepKind: step }],
    )),
  };
  const base = structuredClone(source.policy.base.snapshot) as FrozenProgrammePolicyV1;
  const routeSnapshotBlob = canonicalProgrammePolicyJson(routeSnapshot);
  Object.assign((base as any).execution, {
    routeSnapshotBlob,
    routeSnapshotBlobDigest: sha256(routeSnapshotBlob),
    routeSnapshotDigest: digestValue(routeSnapshot),
  });
  const baseFingerprint = programmePolicyFingerprint(base);
  const outer = createFrozenProgrammePolicyV2(base, baseFingerprint);
  const policy = verifyFrozenProgrammePolicyV2(
    outer, programmePolicyV2Fingerprint(outer),
  );
  const mutableReceipt = structuredClone(source.receipt) as Receipt;
  mutableReceipt.route.snapshotDigest = policy.base.snapshot.execution.routeSnapshotDigest;
  mutableReceipt.toolVersions.programmePolicyFingerprint = policy.base.fingerprint;
  if (options.acceptanceRejected) {
    mutableReceipt.admittedPaths = ['crates/sf-sparql/src/not-declared.rs'];
  }
  if (options.status !== undefined && options.status !== 'pass') {
    mutableReceipt.status = options.status;
    mutableReceipt.failureCode = options.status === 'fail'
      ? 'HARNESS_TRANSACTION_FAILED'
      : 'HARNESS_ACCEPTANCE_GATE_FAILED';
  }
  const receipt = rehashReceipt(mutableReceipt);
  const rufloEvidence = programmeV5RufloFixture({
    taskId: receipt.taskId,
    runId: receipt.runId,
    swarmId: receipt.coordination.swarmId!,
    coordinationTaskId: receipt.coordination.taskId!,
    routeSnapshotDigest: receipt.route.snapshotDigest,
    hookIds: receipt.coordination.hookIds,
    traceIds: receipt.coordination.traceIds,
    transactionStartedAt: receipt.route.frozenAt,
    capturedAt: receipt.issuedAt,
  });
  const candidateTransactionEvidence = receipt.status === 'pass'
    ? createCandidateTransactionEvidenceV1({
        receipt,
        nativeRuntimeEvidence: source.nativeRuntimeEvidence,
        repairTransitions: source.repairTransitions,
      })
    : null;
  const envelope = createProgrammeEnvelopeV6({
    policy: policy.snapshot,
    rufloEvidence,
    candidateTransactionEvidence,
    receipt,
    diagnosticBlob,
  }, policy.fingerprint);
  const serialized = serializeProgrammeEnvelopeV6(envelope, policy.fingerprint);
  const repository = temporary('programme-v6-replay-repository-');
  mkdirSync(join(repository, 'coding-harness'), { mode: 0o755 });
  const controllerStore = temporary('programme-v6-replay-controller-');
  const authority = temporary('programme-v6-replay-authority-');
  const reviewPath = programmeV6ArtifactPath(repository, runId, 'policy-review');
  const envelopePath = programmeV6ArtifactPath(repository, runId, 'execution');
  const replayPath = programmeV6ArtifactPath(repository, runId, 'replay');
  const invocation: ProgrammeV6PolicyInvocation = {
    repositoryRoot: repository,
    controllerCommit: policy.base.snapshot.controller.identity.commit,
    taskPath: policy.base.snapshot.controller.taskPath,
    runId,
    swarmId: options.invocationSwarmId ?? receipt.coordination.swarmId!,
    coordinationTaskId: receipt.coordination.taskId!,
    hiveId: 'hierarchical',
    consensusId: 'raft',
    policyReviewReceipt: reviewPath,
    expectedPolicy: {
      controllerCommit: policy.base.snapshot.controller.identity.commit,
      taskPath: policy.base.snapshot.controller.taskPath,
      fingerprint: policy.fingerprint,
    },
  };
  const bootstrap: ProgrammeV5BootstrapEvidence = {
    schemaVersion: 3,
    source: 'verified-packed-private-runtime',
    controllerCommit: invocation.controllerCommit,
    taskPath: invocation.taskPath,
    controllerStoreDigest: policy.base.snapshot.bootstrap.controllerStoreDigest,
    buildManifestDigest: policy.base.snapshot.controller.buildManifestBlobDigest,
    runtimeTreeDigest: policy.base.snapshot.controller.runtimeTreeDigest,
    nodeDigest: policy.base.snapshot.bootstrap.nodeDigest,
    gitDigest: policy.base.snapshot.bootstrap.gitDigest,
  };
  writeReview(invocation, bootstrap, policy);
  const review = readProgrammeV6PolicyReviewReceipt(invocation, bootstrap);
  const claim = claimProgrammeV6Execution(invocation, review, authority);
  writeProgrammeV6PrivateArtifact(repository, envelopePath, serialized);
  mocks.attestController.mockResolvedValue(controllerFor(policy));
  const argv = [
    '--repository', repository,
    '--controller-store', controllerStore,
    '--controller-commit', invocation.controllerCommit,
    '--run-id', runId,
    '--swarm-id', invocation.swarmId,
    '--coordination-task-id', invocation.coordinationTaskId,
    '--hive-id', invocation.hiveId,
    '--consensus-id', invocation.consensusId,
    '--task-path', invocation.taskPath,
    '--expected-policy-fingerprint', policy.fingerprint,
    '--policy-review-receipt', reviewPath,
    '--replay', 'verify-only',
    '--envelope-receipt', envelopePath,
    '--receipt-path', replayPath,
  ];
  return {
    repository, authority, argv, bootstrap, policy, receipt, envelope, serialized,
    envelopePath, reviewPath, claimPath: claim.path,
  };
}

function writeReview(
  invocation: ProgrammeV6PolicyInvocation,
  bootstrap: ProgrammeV5BootstrapEvidence,
  policy: ParsedProgrammePolicyV2,
): void {
  const policyBlob = canonicalProgrammePolicyJson(policy.snapshot);
  const body = {
    schemaVersion: 1,
    authority: 'development-only-no-promotion',
    operation: 'programme-v6-policy-review',
    controllerCommit: invocation.controllerCommit,
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
    policyFingerprint: policy.fingerprint,
    policyBlob,
  } as const;
  const receipt = {
    ...body,
    policyReviewReceiptDigest: sha256(canonicalProgrammePolicyJson(body)),
  };
  writeProgrammeV6PrivateArtifact(
    invocation.repositoryRoot,
    invocation.policyReviewReceipt,
    `${JSON.stringify(receipt)}\n`,
  );
}

function controllerFor(policy: ParsedProgrammePolicyV2): ControllerAttestation {
  const frozen = policy.base.snapshot.controller;
  return {
    identity: frozen.identity,
    manifestPath: frozen.manifestPath,
    manifestBlob: frozen.manifestBlob,
    taskPath: frozen.taskPath,
    taskBlob: frozen.taskBlob,
    buildManifestPath: frozen.buildManifestPath,
    buildManifestBlob: frozen.buildManifestBlob,
    task: policy.base.task,
    manifest: policy.base.manifest,
    build: policy.base.build,
    taskBlobDigest: frozen.taskBlobDigest,
    manifestBlobDigest: frozen.manifestBlobDigest,
    buildManifestBlobDigest: frozen.buildManifestBlobDigest,
    executionDigest: frozen.executionDigest,
  };
}

function overwriteEnvelope(fixture: ReplayFixture, value: string): void {
  writeFileSync(fixture.envelopePath, value, 'utf8');
}

function replaceFlag(argv: readonly string[], flag: string, value: string): string[] {
  const copy = [...argv];
  copy[copy.indexOf(flag) + 1] = value;
  return copy;
}

function expectedLaunchReceiptDigest(receipt: Record<string, unknown>): string {
  return sha256(canonicalProgrammePolicyJson({
    schemaVersion: 1,
    domain: 'semantic-fabric/programme-v6/replay-launch/v1',
    operation: 'programme-v6-replay-launch',
    controllerCommit: receipt.controllerCommit,
    taskPath: receipt.taskPath,
    outerPolicyFingerprint: receipt.policyFingerprint,
    basePolicyFingerprint: receipt.basePolicyFingerprint,
    envelopeDigest: receipt.envelopeDigest,
    transactionStatus: receipt.transactionStatus,
    receiptDigest: receipt.receiptDigest,
    candidateTransactionEvidenceDigest: receipt.candidateTransactionEvidenceDigest,
    executionClaimDigest: receipt.executionClaimDigest,
  }));
}

function temporary(prefix: string): string {
  const root = mkdtempSync(join(tmpdir(), prefix));
  chmodSync(root, 0o700);
  roots.push(root);
  return root;
}

function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}
