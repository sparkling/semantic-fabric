// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  DEVELOPMENT_AUTHORITY, SHA256_PATTERN, asRecord, assertExactKeys, deepFreeze,
} from './contracts.js';
import { canonicalProgrammePolicyJson } from './programme-v5-driver-support.js';
import {
  verifyFrozenProgrammePolicyV2, type ParsedProgrammePolicyV2,
} from './programme-policy-v6.js';
import {
  PROGRAMME_V6_CLAIM_AUTHORITY_ROOT, programmeV6AuthorityClaimPath,
  readProgrammeV6AuthorityClaim, readProgrammeV6PrivateArtifact,
  writeProgrammeV6AuthorityClaim,
} from './programme-v6-receipt-io.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

const REVIEW_KEYS = [
  'schemaVersion', 'authority', 'operation', 'controllerCommit', 'taskPath', 'runId',
  'swarmId', 'coordinationTaskId', 'hiveId', 'consensusId',
  'controllerStoreDigest', 'buildManifestDigest', 'runtimeTreeDigest', 'nodeDigest',
  'gitDigest', 'policyFingerprint', 'policyBlob', 'policyReviewReceiptDigest',
] as const;

const CLAIM_KEYS = [
  'schemaVersion', 'authority', 'operation', 'controllerCommit', 'taskPath', 'runId',
  'swarmId', 'coordinationTaskId', 'hiveId', 'consensusId',
  'policyFingerprint', 'policyReviewReceiptDigest', 'claimKeyDigest', 'claimDigest',
] as const;

export interface ProgrammeV6PolicyInvocation {
  readonly repositoryRoot: string;
  readonly controllerCommit: string;
  readonly taskPath: string;
  readonly runId: string;
  readonly swarmId: string;
  readonly coordinationTaskId: string;
  readonly hiveId: string;
  readonly consensusId: string;
  readonly policyReviewReceipt: string;
  readonly expectedPolicy: Readonly<{
    readonly controllerCommit: string;
    readonly taskPath: string;
    readonly fingerprint: string;
  }>;
}

export interface ProgrammeV6PolicyReplayInvocation extends ProgrammeV6PolicyInvocation {}

export interface ProgrammeV6BootstrapEvidence {
  readonly schemaVersion: 3;
  readonly source: 'verified-packed-private-runtime';
  readonly controllerCommit: string;
  readonly taskPath: string;
  readonly controllerStoreDigest: string;
  readonly buildManifestDigest: string;
  readonly runtimeTreeDigest: string;
  readonly nodeDigest: string;
  readonly gitDigest: string;
}

export interface ProgrammeV6PolicyReviewReceipt {
  readonly schemaVersion: 1;
  readonly authority: typeof DEVELOPMENT_AUTHORITY;
  readonly operation: 'programme-v6-policy-review';
  readonly controllerCommit: string;
  readonly taskPath: string;
  readonly runId: string;
  readonly swarmId: string;
  readonly coordinationTaskId: string;
  readonly hiveId: string;
  readonly consensusId: string;
  readonly controllerStoreDigest: string;
  readonly buildManifestDigest: string;
  readonly runtimeTreeDigest: string;
  readonly nodeDigest: string;
  readonly gitDigest: string;
  readonly policyFingerprint: string;
  readonly policyBlob: string;
  readonly policyReviewReceiptDigest: string;
}

export function readProgrammeV6PolicyReviewReceipt(
  invocation: ProgrammeV6PolicyInvocation | ProgrammeV6PolicyReplayInvocation,
  bootstrap: ProgrammeV6BootstrapEvidence,
): ProgrammeV6PolicyReviewReceipt {
  const serialized = readProgrammeV6PrivateArtifact(
    invocation.repositoryRoot, invocation.policyReviewReceipt, 6_000_000,
  );
  const input = asRecord(
    parseJsonWithoutDuplicateKeys(serialized, 'programme v6 policy review receipt'),
    'programme v6 policy review receipt',
  );
  assertExactKeys(input, REVIEW_KEYS, 'programme v6 policy review receipt');
  if (serialized !== `${JSON.stringify(input)}\n`) invalid();
  const receipt = parseBoundReviewReceipt(input, invocation);
  if (bootstrap.schemaVersion !== 3
    || bootstrap.source !== 'verified-packed-private-runtime'
    || bootstrap.controllerCommit !== receipt.controllerCommit
    || bootstrap.taskPath !== receipt.taskPath
    || bootstrap.controllerStoreDigest !== receipt.controllerStoreDigest
    || bootstrap.buildManifestDigest !== receipt.buildManifestDigest
    || bootstrap.runtimeTreeDigest !== receipt.runtimeTreeDigest
    || bootstrap.nodeDigest !== receipt.nodeDigest || bootstrap.gitDigest !== receipt.gitDigest) {
    invalid();
  }
  return receipt;
}

export function readProgrammeV6ExecutionClaim(
  invocation: ProgrammeV6PolicyReplayInvocation,
  receipt: ProgrammeV6PolicyReviewReceipt,
  authorityRoot = PROGRAMME_V6_CLAIM_AUTHORITY_ROOT,
): Readonly<{ path: string; digest: string }> {
  const boundReceipt = parseBoundReviewReceipt(receipt, invocation);
  const claimKeyDigest = executionClaimKeyDigest(invocation, boundReceipt);
  const path = programmeV6AuthorityClaimPath(claimKeyDigest, authorityRoot);
  const serialized = readProgrammeV6AuthorityClaim(path, authorityRoot, 100_000);
  const input = asRecord(
    parseJsonWithoutDuplicateKeys(serialized, 'programme v6 execution claim'),
    'programme v6 execution claim',
  );
  assertExactKeys(input, CLAIM_KEYS, 'programme v6 execution claim');
  if (serialized !== `${JSON.stringify(input)}\n` || input.schemaVersion !== 1
    || input.authority !== DEVELOPMENT_AUTHORITY
    || input.operation !== 'programme-v6-execution-claim'
    || input.controllerCommit !== invocation.controllerCommit
    || input.taskPath !== invocation.taskPath || input.runId !== invocation.runId
    || input.swarmId !== invocation.swarmId
    || input.coordinationTaskId !== invocation.coordinationTaskId
    || input.hiveId !== invocation.hiveId || input.consensusId !== invocation.consensusId
    || input.policyFingerprint !== invocation.expectedPolicy.fingerprint
    || input.policyFingerprint !== boundReceipt.policyFingerprint
    || input.policyReviewReceiptDigest !== boundReceipt.policyReviewReceiptDigest
    || input.claimKeyDigest !== claimKeyDigest) invalidClaim();
  const claimDigest = claimDigestValue(input.claimDigest);
  const { claimDigest: _claimDigest, ...body } = input;
  if (claimDigest !== sha256(canonicalProgrammePolicyJson(body))) invalidClaim();
  return Object.freeze({ path, digest: claimDigest });
}

export function claimProgrammeV6Execution(
  invocation: ProgrammeV6PolicyInvocation,
  receipt: ProgrammeV6PolicyReviewReceipt,
  authorityRoot = PROGRAMME_V6_CLAIM_AUTHORITY_ROOT,
): Readonly<{ path: string; digest: string }> {
  const boundReceipt = parseBoundReviewReceipt(receipt, invocation);
  const claimKeyDigest = executionClaimKeyDigest(invocation, boundReceipt);
  const path = programmeV6AuthorityClaimPath(claimKeyDigest, authorityRoot);
  const body = {
    schemaVersion: 1,
    authority: DEVELOPMENT_AUTHORITY,
    operation: 'programme-v6-execution-claim',
    controllerCommit: invocation.controllerCommit,
    taskPath: invocation.taskPath,
    runId: invocation.runId,
    swarmId: invocation.swarmId,
    coordinationTaskId: invocation.coordinationTaskId,
    hiveId: invocation.hiveId,
    consensusId: invocation.consensusId,
    policyFingerprint: boundReceipt.policyFingerprint,
    policyReviewReceiptDigest: boundReceipt.policyReviewReceiptDigest,
    claimKeyDigest,
  } as const;
  const claimDigest = sha256(canonicalProgrammePolicyJson(body));
  writeProgrammeV6AuthorityClaim(
    path, `${JSON.stringify({ ...body, claimDigest })}\n`, authorityRoot, 100_000,
  );
  return Object.freeze({ path, digest: claimDigest });
}

function parseBoundReviewReceipt(
  value: unknown,
  invocation: ProgrammeV6PolicyInvocation | ProgrammeV6PolicyReplayInvocation,
): ProgrammeV6PolicyReviewReceipt {
  const input = asRecord(value, 'programme v6 policy review receipt');
  assertExactKeys(input, REVIEW_KEYS, 'programme v6 policy review receipt');
  if (input.schemaVersion !== 1 || input.authority !== DEVELOPMENT_AUTHORITY
    || input.operation !== 'programme-v6-policy-review') invalid();
  const policyBlob = text(input.policyBlob);
  const policyFingerprint = digest(input.policyFingerprint);
  const policyValue = parseJsonWithoutDuplicateKeys(policyBlob, 'programme v6 reviewed policy');
  if (canonicalProgrammePolicyJson(policyValue) !== policyBlob
    || sha256(policyBlob) !== policyFingerprint) invalid();
  const policy = verifyFrozenProgrammePolicyV2(policyValue, policyFingerprint);
  if (canonicalProgrammePolicyJson(policy.snapshot) !== policyBlob) invalid();
  const receipt = deepFreeze({
    schemaVersion: 1 as const,
    authority: DEVELOPMENT_AUTHORITY,
    operation: 'programme-v6-policy-review' as const,
    controllerCommit: text(input.controllerCommit),
    taskPath: text(input.taskPath),
    runId: text(input.runId),
    swarmId: text(input.swarmId),
    coordinationTaskId: text(input.coordinationTaskId),
    hiveId: text(input.hiveId),
    consensusId: text(input.consensusId),
    controllerStoreDigest: digest(input.controllerStoreDigest),
    buildManifestDigest: digest(input.buildManifestDigest),
    runtimeTreeDigest: digest(input.runtimeTreeDigest),
    nodeDigest: digest(input.nodeDigest),
    gitDigest: digest(input.gitDigest),
    policyFingerprint,
    policyBlob,
    policyReviewReceiptDigest: digest(input.policyReviewReceiptDigest),
  });
  if (invocation.expectedPolicy.controllerCommit !== invocation.controllerCommit
    || invocation.expectedPolicy.taskPath !== invocation.taskPath
    || receipt.controllerCommit !== invocation.controllerCommit
    || receipt.taskPath !== invocation.taskPath || receipt.runId !== invocation.runId
    || receipt.swarmId !== invocation.swarmId
    || receipt.coordinationTaskId !== invocation.coordinationTaskId
    || receipt.hiveId !== invocation.hiveId || receipt.consensusId !== invocation.consensusId
    || receipt.policyFingerprint !== invocation.expectedPolicy.fingerprint
    || receipt.policyReviewReceiptDigest !== sha256(canonicalBody(receipt))
    || !policyBindingsMatch(receipt, policy) || !routeRunIdMatches(policy, receipt.runId)) {
    invalid();
  }
  return receipt;
}

function executionClaimKeyDigest(
  invocation: ProgrammeV6PolicyInvocation | ProgrammeV6PolicyReplayInvocation,
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

function policyBindingsMatch(
  receipt: ProgrammeV6PolicyReviewReceipt,
  policy: ParsedProgrammePolicyV2,
): boolean {
  const snapshot = policy.base.snapshot;
  return snapshot.bootstrap.controllerStoreDigest === receipt.controllerStoreDigest
    && snapshot.bootstrap.nodeDigest === receipt.nodeDigest
    && snapshot.bootstrap.gitDigest === receipt.gitDigest
    && snapshot.controller.identity.commit === receipt.controllerCommit
    && snapshot.controller.taskPath === receipt.taskPath
    && snapshot.controller.buildManifestBlobDigest === receipt.buildManifestDigest
    && snapshot.controller.runtimeTreeDigest === receipt.runtimeTreeDigest;
}

function routeRunIdMatches(policy: ParsedProgrammePolicyV2, runId: string): boolean {
  const route = asRecord(policy.base.routeSnapshot, 'programme v6 route snapshot');
  assertExactKeys(route, ['historyEpoch', 'decisions'], 'programme v6 route snapshot');
  const decisions = asRecord(route.decisions, 'programme v6 route decisions');
  assertExactKeys(decisions, ['architecture', 'implementation', 'repair'],
    'programme v6 route decisions');
  return (['architecture', 'implementation', 'repair'] as const).every((step) => {
    const decision = asRecord(decisions[step], `programme v6 route ${step}`);
    return decision.runId === runId && decision.stepKind === step;
  });
}

function canonicalBody(receipt: ProgrammeV6PolicyReviewReceipt): string {
  const { policyReviewReceiptDigest: _digest, ...body } = receipt;
  return canonicalProgrammePolicyJson(body);
}

function digest(value: unknown): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value)
    || value === '0'.repeat(64)) invalid();
  return value;
}

function claimDigestValue(value: unknown): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value)
    || value === '0'.repeat(64)) invalidClaim();
  return value;
}

function text(value: unknown): string {
  if (typeof value !== 'string' || value.length === 0 || value.includes('\0')) invalid();
  return value;
}

function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}

function invalid(): never {
  throw new Error('HARNESS_PROGRAMME_V6_POLICY_REVIEW_RECEIPT_INVALID');
}

function invalidClaim(): never {
  throw new Error('HARNESS_PROGRAMME_V6_EXECUTION_CLAIM_INVALID');
}
