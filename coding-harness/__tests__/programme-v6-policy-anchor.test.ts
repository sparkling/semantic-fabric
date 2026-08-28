// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  chmodSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, rmSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { canonicalProgrammePolicyJson } from '../src/programme-v5-driver-support.js';
import { programmeV5AuthorityClaimPath } from '../src/programme-v5-receipt-io.js';
import {
  programmePolicyFingerprint, type FrozenProgrammePolicyV1,
} from '../src/programme-policy-v5.js';
import {
  createFrozenProgrammePolicyV2, programmePolicyV2Fingerprint,
  verifyFrozenProgrammePolicyV2, type ParsedProgrammePolicyV2,
} from '../src/programme-policy-v6.js';
import {
  claimProgrammeV6Execution, readProgrammeV6ExecutionClaim,
  readProgrammeV6PolicyReviewReceipt, type ProgrammeV6BootstrapEvidence,
  type ProgrammeV6PolicyInvocation, type ProgrammeV6PolicyReviewReceipt,
} from '../src/programme-v6-policy-anchor.js';
import {
  programmeV6ArtifactPath, programmeV6AuthorityClaimPath,
  writeProgrammeV6AuthorityClaim, writeProgrammeV6PrivateArtifact,
} from '../src/programme-v6-receipt-io.js';
import { digestValue } from '../src/receipts.js';
import { programmeV6Fixture } from './programme-v6-fixtures.js';

const roots: string[] = [];

interface AnchorFixture {
  readonly repository: string;
  readonly policy: ParsedProgrammePolicyV2;
  readonly invocation: ProgrammeV6PolicyInvocation;
  readonly bootstrap: ProgrammeV6BootstrapEvidence;
}

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('programme-v6 policy anchor', () => {
  it('binds the canonical full Policy V2 blob and independently anchored fingerprint', () => {
    const fixture = anchorFixture('programme_v6_anchor_valid');
    const expected = writeReview(fixture);
    const receipt = readProgrammeV6PolicyReviewReceipt(
      fixture.invocation, fixture.bootstrap,
    );

    expect(receipt).toEqual(expected);
    expect(receipt.operation).toBe('programme-v6-policy-review');
    expect(receipt.policyFingerprint).toBe(fixture.policy.fingerprint);
    expect(receipt.policyFingerprint).toBe(sha256(receipt.policyBlob));
    expect(JSON.parse(receipt.policyBlob)).toEqual(fixture.policy.snapshot);
    expect(JSON.parse(receipt.policyBlob)).toMatchObject({
      schemaVersion: 2,
      basePolicyFingerprint: fixture.policy.base.fingerprint,
      basePolicy: { schemaVersion: 1 },
    });
    expect(Object.isFrozen(receipt)).toBe(true);
  });

  it('rejects V1 policy substitution and invalid embedded base policy after reminting', () => {
    const v1Fixture = anchorFixture('programme_v6_anchor_v1_blob');
    const v1Blob = canonicalProgrammePolicyJson(v1Fixture.policy.base.snapshot);
    const v1Fingerprint = sha256(v1Blob);
    writeReview(v1Fixture, { policyBlob: v1Blob, policyFingerprint: v1Fingerprint });
    expect(() => readProgrammeV6PolicyReviewReceipt(
      withFingerprint(v1Fixture.invocation, v1Fingerprint), v1Fixture.bootstrap,
    )).toThrow();

    const baseFixture = anchorFixture('programme_v6_anchor_base_tamper');
    const outer = structuredClone(baseFixture.policy.snapshot) as any;
    outer.basePolicyFingerprint = 'e'.repeat(64);
    const outerBlob = canonicalProgrammePolicyJson(outer);
    const attackerFingerprint = sha256(outerBlob);
    writeReview(baseFixture, {
      policyBlob: outerBlob,
      policyFingerprint: attackerFingerprint,
    });
    expect(() => readProgrammeV6PolicyReviewReceipt(
      withFingerprint(baseFixture.invocation, attackerFingerprint), baseFixture.bootstrap,
    )).toThrow();
  });

  it('rejects a reminted V5 policy-review receipt and V5 receipt object substitution', () => {
    const fixture = anchorFixture('programme_v6_anchor_v5_review');
    writeReview(fixture, { operation: 'programme-v5-policy-review' });
    expect(() => readProgrammeV6PolicyReviewReceipt(
      fixture.invocation, fixture.bootstrap,
    )).toThrow('HARNESS_PROGRAMME_V6_POLICY_REVIEW_RECEIPT_INVALID');

    const validFixture = anchorFixture('programme_v6_anchor_receipt_object');
    writeReview(validFixture);
    const receipt = readProgrammeV6PolicyReviewReceipt(
      validFixture.invocation, validFixture.bootstrap,
    );
    const v5Receipt = {
      ...receipt,
      operation: 'programme-v5-policy-review',
    } as unknown as ProgrammeV6PolicyReviewReceipt;
    const authority = temporary('programme-v6-anchor-v5-receipt-');
    expect(() => claimProgrammeV6Execution(
      validFixture.invocation, v5Receipt, authority,
    )).toThrow('HARNESS_PROGRAMME_V6_POLICY_REVIEW_RECEIPT_INVALID');
  });

  it('writes and replays a claim only in the V6 namespace', () => {
    const fixture = anchorFixture('programme_v6_anchor_claim');
    writeReview(fixture);
    const receipt = readProgrammeV6PolicyReviewReceipt(
      fixture.invocation, fixture.bootstrap,
    );
    const authority = temporary('programme-v6-anchor-claim-');
    const claim = claimProgrammeV6Execution(fixture.invocation, receipt, authority);

    expect(dirname(claim.path)).toBe(join(authority, 'programme-v6-claims'));
    expect(lstatSync(dirname(claim.path)).mode & 0o777).toBe(0o700);
    expect(lstatSync(claim.path).mode & 0o777).toBe(0o600);
    expect(JSON.parse(readFileSync(claim.path, 'utf8'))).toMatchObject({
      operation: 'programme-v6-execution-claim',
      policyFingerprint: fixture.policy.fingerprint,
      policyReviewReceiptDigest: receipt.policyReviewReceiptDigest,
      claimDigest: claim.digest,
    });
    expect(readProgrammeV6ExecutionClaim(
      fixture.invocation, receipt, authority,
    )).toEqual(claim);
    expect(() => claimProgrammeV6Execution(fixture.invocation, receipt, authority))
      .toThrow('HARNESS_PROGRAMME_V6_RECEIPT_EXISTS');
  });

  it('rejects a V5 execution claim even when copied and reminted into the V6 path', () => {
    const fixture = anchorFixture('programme_v6_anchor_v5_claim');
    writeReview(fixture);
    const receipt = readProgrammeV6PolicyReviewReceipt(
      fixture.invocation, fixture.bootstrap,
    );
    const authority = temporary('programme-v6-anchor-v5-claim-');
    const claimKeyDigest = executionClaimKeyDigest(fixture.invocation, receipt);
    const path = programmeV6AuthorityClaimPath(claimKeyDigest, authority);
    const body = claimBody(
      fixture.invocation, receipt, claimKeyDigest, 'programme-v5-execution-claim',
    );
    const claimDigest = sha256(canonicalProgrammePolicyJson(body));
    writeProgrammeV6AuthorityClaim(
      path, `${JSON.stringify({ ...body, claimDigest })}\n`, authority,
    );

    expect(path).not.toBe(programmeV5AuthorityClaimPath(claimKeyDigest, authority));
    expect(() => readProgrammeV6ExecutionClaim(fixture.invocation, receipt, authority))
      .toThrow('HARNESS_PROGRAMME_V6_EXECUTION_CLAIM_INVALID');
  });
});

function anchorFixture(runId: string): AnchorFixture {
  const base = structuredClone(
    programmeV6Fixture().policy.base.snapshot,
  ) as unknown as FrozenProgrammePolicyV1;
  const routeSnapshot = {
    historyEpoch: 0,
    decisions: Object.fromEntries(['architecture', 'implementation', 'repair'].map(
      (step) => [step, { runId, stepKind: step }],
    )),
  };
  const routeSnapshotBlob = canonicalProgrammePolicyJson(routeSnapshot);
  Object.assign((base as any).execution, {
    routeSnapshotBlob,
    routeSnapshotBlobDigest: sha256(routeSnapshotBlob),
    routeSnapshotDigest: digestValue(routeSnapshot),
  });
  const baseFingerprint = programmePolicyFingerprint(base);
  const snapshot = createFrozenProgrammePolicyV2(base, baseFingerprint);
  const policy = verifyFrozenProgrammePolicyV2(
    snapshot, programmePolicyV2Fingerprint(snapshot),
  );
  const repository = temporary('programme-v6-anchor-repository-');
  mkdirSync(join(repository, 'coding-harness'), { mode: 0o755 });
  const taskPath = policy.base.snapshot.controller.taskPath;
  const invocation: ProgrammeV6PolicyInvocation = {
    repositoryRoot: repository,
    controllerCommit: policy.base.snapshot.controller.identity.commit,
    taskPath,
    runId,
    swarmId: 'programme_v6_swarm',
    coordinationTaskId: 'programme_v6_task',
    hiveId: 'hierarchical',
    consensusId: 'raft',
    policyReviewReceipt: programmeV6ArtifactPath(repository, runId, 'policy-review'),
    expectedPolicy: {
      controllerCommit: policy.base.snapshot.controller.identity.commit,
      taskPath,
      fingerprint: policy.fingerprint,
    },
  };
  const bootstrap: ProgrammeV6BootstrapEvidence = {
    schemaVersion: 3,
    source: 'verified-packed-private-runtime',
    controllerCommit: invocation.controllerCommit,
    taskPath,
    controllerStoreDigest: policy.base.snapshot.bootstrap.controllerStoreDigest,
    buildManifestDigest: policy.base.snapshot.controller.buildManifestBlobDigest,
    runtimeTreeDigest: policy.base.snapshot.controller.runtimeTreeDigest,
    nodeDigest: policy.base.snapshot.bootstrap.nodeDigest,
    gitDigest: policy.base.snapshot.bootstrap.gitDigest,
  };
  return { repository, policy, invocation, bootstrap };
}

function writeReview(
  fixture: AnchorFixture,
  override: Readonly<{
    policyBlob?: string;
    policyFingerprint?: string;
    operation?: 'programme-v6-policy-review' | 'programme-v5-policy-review';
  }> = {},
): Record<string, unknown> {
  const policyBlob = override.policyBlob
    ?? canonicalProgrammePolicyJson(fixture.policy.snapshot);
  const policyFingerprint = override.policyFingerprint ?? sha256(policyBlob);
  const body = {
    schemaVersion: 1,
    authority: 'development-only-no-promotion',
    operation: override.operation ?? 'programme-v6-policy-review',
    controllerCommit: fixture.invocation.controllerCommit,
    taskPath: fixture.invocation.taskPath,
    runId: fixture.invocation.runId,
    swarmId: fixture.invocation.swarmId,
    coordinationTaskId: fixture.invocation.coordinationTaskId,
    hiveId: fixture.invocation.hiveId,
    consensusId: fixture.invocation.consensusId,
    controllerStoreDigest: fixture.bootstrap.controllerStoreDigest,
    buildManifestDigest: fixture.bootstrap.buildManifestDigest,
    runtimeTreeDigest: fixture.bootstrap.runtimeTreeDigest,
    nodeDigest: fixture.bootstrap.nodeDigest,
    gitDigest: fixture.bootstrap.gitDigest,
    policyFingerprint,
    policyBlob,
  } as const;
  const receipt = {
    ...body,
    policyReviewReceiptDigest: sha256(canonicalProgrammePolicyJson(body)),
  };
  writeProgrammeV6PrivateArtifact(
    fixture.repository,
    fixture.invocation.policyReviewReceipt,
    `${JSON.stringify(receipt)}\n`,
  );
  return receipt;
}

function withFingerprint(
  invocation: ProgrammeV6PolicyInvocation,
  fingerprint: string,
): ProgrammeV6PolicyInvocation {
  return {
    ...invocation,
    expectedPolicy: { ...invocation.expectedPolicy, fingerprint },
  };
}

function executionClaimKeyDigest(
  invocation: ProgrammeV6PolicyInvocation,
  receipt: ProgrammeV6PolicyReviewReceipt,
): string {
  return sha256(canonicalProgrammePolicyJson({
    schemaVersion: 1,
    authority: 'programme-v6-local-subscription-host',
    controllerCommit: invocation.controllerCommit,
    taskPath: invocation.taskPath,
    runId: invocation.runId,
    swarmId: invocation.swarmId,
    coordinationTaskId: invocation.coordinationTaskId,
    policyReviewReceiptDigest: receipt.policyReviewReceiptDigest,
  }));
}

function claimBody(
  invocation: ProgrammeV6PolicyInvocation,
  receipt: ProgrammeV6PolicyReviewReceipt,
  claimKeyDigest: string,
  operation: 'programme-v5-execution-claim',
) {
  return {
    schemaVersion: 1,
    authority: 'development-only-no-promotion',
    operation,
    controllerCommit: invocation.controllerCommit,
    taskPath: invocation.taskPath,
    runId: invocation.runId,
    swarmId: invocation.swarmId,
    coordinationTaskId: invocation.coordinationTaskId,
    hiveId: invocation.hiveId,
    consensusId: invocation.consensusId,
    policyFingerprint: invocation.expectedPolicy.fingerprint,
    policyReviewReceiptDigest: receipt.policyReviewReceiptDigest,
    claimKeyDigest,
  } as const;
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
