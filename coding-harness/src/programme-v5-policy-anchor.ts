// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  DEVELOPMENT_AUTHORITY, SHA256_PATTERN, asRecord, assertExactKeys, deepFreeze,
} from './contracts.js';
import { canonicalProgrammePolicyJson } from './programme-v5-driver-support.js';
import {
  PROGRAMME_V5_CLAIM_AUTHORITY_ROOT, programmeV5AuthorityClaimPath,
  readProgrammeV5AuthorityClaim, readProgrammeV5PrivateArtifact,
  writeProgrammeV5AuthorityClaim,
} from './programme-v5-receipt-io.js';
import {
  verifyFrozenProgrammePolicyV1, type ParsedProgrammePolicyV1,
} from './programme-policy-v5.js';
import type {
  ProgrammeV5BootstrapEvidence, ProgrammeV5Invocation, ProgrammeV5ReplayInvocation,
} from './programme-v5-program-runtime.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

const REVIEW_KEYS = [
  'schemaVersion', 'authority', 'operation', 'controllerCommit', 'taskPath', 'runId',
  'swarmId', 'coordinationTaskId', 'hiveId', 'consensusId',
  'controllerStoreDigest', 'buildManifestDigest', 'runtimeTreeDigest', 'nodeDigest',
  'gitDigest', 'policyFingerprint', 'policyBlob', 'policyReviewReceiptDigest',
] as const;

export interface ProgrammeV5PolicyReviewReceipt {
  readonly schemaVersion: 1;
  readonly authority: typeof DEVELOPMENT_AUTHORITY;
  readonly operation: 'programme-v5-policy-review';
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

export function readProgrammeV5PolicyReviewReceipt(
  invocation: ProgrammeV5Invocation | ProgrammeV5ReplayInvocation,
  bootstrap: ProgrammeV5BootstrapEvidence,
): ProgrammeV5PolicyReviewReceipt {
  const serialized = readProgrammeV5PrivateArtifact(
    invocation.repositoryRoot, invocation.policyReviewReceipt, 6_000_000,
  );
  const input = asRecord(
    parseJsonWithoutDuplicateKeys(serialized, 'programme v5 policy review receipt'),
    'programme v5 policy review receipt',
  );
  assertExactKeys(input, REVIEW_KEYS, 'programme v5 policy review receipt');
  if (serialized !== `${JSON.stringify(input)}\n`) invalid();
  if (input.schemaVersion !== 1 || input.authority !== DEVELOPMENT_AUTHORITY
    || input.operation !== 'programme-v5-policy-review') invalid();
  const policyBlob = text(input.policyBlob);
  const policyFingerprint = digest(input.policyFingerprint);
  const policyValue = parseJsonWithoutDuplicateKeys(policyBlob, 'programme v5 reviewed policy');
  if (canonicalProgrammePolicyJson(policyValue) !== policyBlob
    || sha256(policyBlob) !== policyFingerprint) invalid();
  const policy = verifyFrozenProgrammePolicyV1(policyValue, policyFingerprint);
  const receipt = deepFreeze({
    schemaVersion: 1 as const,
    authority: DEVELOPMENT_AUTHORITY,
    operation: 'programme-v5-policy-review' as const,
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
  if (receipt.controllerCommit !== invocation.controllerCommit
    || receipt.taskPath !== invocation.taskPath || receipt.runId !== invocation.runId
    || receipt.swarmId !== invocation.swarmId
    || receipt.coordinationTaskId !== invocation.coordinationTaskId
    || receipt.hiveId !== invocation.hiveId || receipt.consensusId !== invocation.consensusId
    || receipt.policyFingerprint !== invocation.expectedPolicy.fingerprint
    || receipt.controllerStoreDigest !== bootstrap.controllerStoreDigest
    || receipt.buildManifestDigest !== bootstrap.buildManifestDigest
    || receipt.runtimeTreeDigest !== bootstrap.runtimeTreeDigest
    || receipt.nodeDigest !== bootstrap.nodeDigest || receipt.gitDigest !== bootstrap.gitDigest
    || receipt.policyReviewReceiptDigest !== sha256(canonicalBody(receipt))
    || !policyBindingsMatch(receipt, policy) || !routeRunIdMatches(policy, receipt.runId)) invalid();
  return receipt;
}

export function readProgrammeV5ExecutionClaim(
  invocation: ProgrammeV5ReplayInvocation,
  receipt: ProgrammeV5PolicyReviewReceipt,
  authorityRoot = PROGRAMME_V5_CLAIM_AUTHORITY_ROOT,
): Readonly<{ path: string; digest: string }> {
  const claimKeyDigest = executionClaimKeyDigest(invocation, receipt);
  const path = programmeV5AuthorityClaimPath(claimKeyDigest, authorityRoot);
  const serialized = readProgrammeV5AuthorityClaim(path, authorityRoot, 100_000);
  const input = asRecord(
    parseJsonWithoutDuplicateKeys(serialized, 'programme v5 execution claim'),
    'programme v5 execution claim',
  );
  assertExactKeys(input, [
    'schemaVersion', 'authority', 'operation', 'controllerCommit', 'taskPath', 'runId',
    'swarmId', 'coordinationTaskId', 'hiveId', 'consensusId',
    'policyFingerprint', 'policyReviewReceiptDigest', 'claimKeyDigest', 'claimDigest',
  ], 'programme v5 execution claim');
  if (serialized !== `${JSON.stringify(input)}\n` || input.schemaVersion !== 1
    || input.authority !== DEVELOPMENT_AUTHORITY
    || input.operation !== 'programme-v5-execution-claim'
    || input.controllerCommit !== invocation.controllerCommit
    || input.taskPath !== invocation.taskPath || input.runId !== invocation.runId
    || input.swarmId !== invocation.swarmId
    || input.coordinationTaskId !== invocation.coordinationTaskId
    || input.hiveId !== invocation.hiveId || input.consensusId !== invocation.consensusId
    || input.policyFingerprint !== invocation.expectedPolicy.fingerprint
    || input.policyReviewReceiptDigest !== receipt.policyReviewReceiptDigest
    || input.claimKeyDigest !== claimKeyDigest) invalidClaim();
  if (typeof input.claimDigest !== 'string' || !SHA256_PATTERN.test(input.claimDigest)
    || input.claimDigest === '0'.repeat(64)) invalidClaim();
  const claimDigest = input.claimDigest;
  const { claimDigest: _claimDigest, ...body } = input;
  if (claimDigest !== sha256(canonicalProgrammePolicyJson(body))) invalidClaim();
  return Object.freeze({ path, digest: claimDigest });
}

export function claimProgrammeV5Execution(
  invocation: ProgrammeV5Invocation,
  receipt: ProgrammeV5PolicyReviewReceipt,
  authorityRoot = PROGRAMME_V5_CLAIM_AUTHORITY_ROOT,
): Readonly<{ path: string; digest: string }> {
  const claimKeyDigest = executionClaimKeyDigest(invocation, receipt);
  const path = programmeV5AuthorityClaimPath(claimKeyDigest, authorityRoot);
  const body = {
    schemaVersion: 1,
    authority: DEVELOPMENT_AUTHORITY,
    operation: 'programme-v5-execution-claim',
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
  const claimDigest = sha256(canonicalProgrammePolicyJson(body));
  writeProgrammeV5AuthorityClaim(
    path, `${JSON.stringify({ ...body, claimDigest })}\n`, authorityRoot, 100_000,
  );
  return Object.freeze({ path, digest: claimDigest });
}

function executionClaimKeyDigest(
  invocation: ProgrammeV5Invocation | ProgrammeV5ReplayInvocation,
  receipt: ProgrammeV5PolicyReviewReceipt,
): string {
  return sha256(canonicalProgrammePolicyJson({
    schemaVersion: 1,
    authority: 'programme-v5-local-subscription-host',
    controllerCommit: invocation.controllerCommit,
    taskPath: invocation.taskPath,
    runId: invocation.runId,
    swarmId: invocation.swarmId,
    coordinationTaskId: invocation.coordinationTaskId,
    policyReviewReceiptDigest: receipt.policyReviewReceiptDigest,
  }));
}

function policyBindingsMatch(
  receipt: ProgrammeV5PolicyReviewReceipt,
  policy: ParsedProgrammePolicyV1,
): boolean {
  const snapshot = policy.snapshot;
  return snapshot.bootstrap.controllerStoreDigest === receipt.controllerStoreDigest
    && snapshot.bootstrap.nodeDigest === receipt.nodeDigest
    && snapshot.bootstrap.gitDigest === receipt.gitDigest
    && snapshot.controller.identity.commit === receipt.controllerCommit
    && snapshot.controller.taskPath === receipt.taskPath
    && snapshot.controller.buildManifestBlobDigest === receipt.buildManifestDigest
    && snapshot.controller.runtimeTreeDigest === receipt.runtimeTreeDigest;
}

function routeRunIdMatches(policy: ParsedProgrammePolicyV1, runId: string): boolean {
  const route = asRecord(policy.routeSnapshot, 'programme v5 route snapshot');
  assertExactKeys(route, ['historyEpoch', 'decisions'], 'programme v5 route snapshot');
  const decisions = asRecord(route.decisions, 'programme v5 route decisions');
  assertExactKeys(decisions, ['architecture', 'implementation', 'repair'],
    'programme v5 route decisions');
  return (['architecture', 'implementation', 'repair'] as const).every((step) => {
    const decision = asRecord(decisions[step], `programme v5 route ${step}`);
    return decision.runId === runId && decision.stepKind === step;
  });
}

function canonicalBody(receipt: ProgrammeV5PolicyReviewReceipt): string {
  const { policyReviewReceiptDigest: _digest, ...body } = receipt;
  return canonicalProgrammePolicyJson(body);
}

function digest(value: unknown): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || value === '0'.repeat(64)) invalid();
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
  throw new Error('HARNESS_PROGRAMME_V5_POLICY_REVIEW_RECEIPT_INVALID');
}

function invalidClaim(): never {
  throw new Error('HARNESS_PROGRAMME_V5_EXECUTION_CLAIM_INVALID');
}
