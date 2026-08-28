// SPDX-License-Identifier: MIT

import {
  DEVELOPMENT_AUTHORITY,
  SHA256_PATTERN,
  asInteger,
  asNonEmptyString,
  asRecord,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import type { CandidateRepairTransition } from './candidate-repair-transition.js';
import type { NativeInvocationExpectation, NativeModelOperation } from './evidence.js';
import {
  bindNativeRuntimeEvidenceV2,
  type NativeRuntimeEvidenceV2,
} from './native-runtime-evidence-v2.js';
import { digestValue, type GitIdentity, type Receipt } from './receipts.js';

export interface CandidateTransactionEvidenceV1 {
  readonly schemaVersion: 1;
  readonly authority: typeof DEVELOPMENT_AUTHORITY;
  readonly kind: 'candidate-transaction-repair-evidence';
  readonly receiptBinding: Readonly<{
    schemaVersion: 1;
    receiptSchemaVersion: 3;
    receiptDigest: string;
    runId: string;
    taskId: string;
    status: 'pass';
    repairCount: number;
    patchHistoryDigest: string;
    nativeRuntimeEvidenceDigest: string;
  }>;
  readonly nativeInvocationBindings: ReadonlyArray<Required<
    Pick<NativeInvocationExpectation, 'invocationId' | 'host' | 'operation'
      | 'candidateTree' | 'patchPayloadSha256'>
  >>;
  readonly nativeRuntimeEvidence: NativeRuntimeEvidenceV2;
  readonly repairTransitions: readonly CandidateRepairTransition[];
  readonly repairTransitionsDigest: string;
  readonly evidenceDigest: string;
}

const TOP_KEYS = [
  'schemaVersion', 'authority', 'kind', 'receiptBinding', 'nativeInvocationBindings',
  'nativeRuntimeEvidence', 'repairTransitions', 'repairTransitionsDigest', 'evidenceDigest',
] as const;
const RECEIPT_KEYS = [
  'schemaVersion', 'receiptSchemaVersion', 'receiptDigest', 'runId', 'taskId', 'status',
  'repairCount', 'patchHistoryDigest', 'nativeRuntimeEvidenceDigest',
] as const;
const INVOCATION_KEYS = [
  'invocationId', 'host', 'operation', 'candidateTree', 'patchPayloadSha256',
] as const;
const TRANSITION_KEYS = [
  'schemaVersion', 'fromAttempt', 'toAttempt', 'phase', 'trigger', 'buildDisposition',
  'sourcePatchDigest', 'replacementPatchDigest', 'sourceCandidate', 'repairResetIdentity',
  'resetIdentity', 'reasonDigests', 'nativeInvocation', 'digest',
] as const;

export function createCandidateTransactionEvidenceV1(input: Readonly<{
  receipt: Receipt;
  nativeRuntimeEvidence: NativeRuntimeEvidenceV2;
  repairTransitions: readonly CandidateRepairTransition[];
}>): CandidateTransactionEvidenceV1 {
  const bindings = input.nativeRuntimeEvidence.invocations.map((invocation) => ({
    invocationId: invocation.invocationId,
    host: invocation.host,
    operation: invocation.operation,
    candidateTree: invocation.candidateTree,
    patchPayloadSha256: invocation.patchPayloadSha256,
  }));
  const receiptBinding = {
    schemaVersion: 1 as const,
    receiptSchemaVersion: 3 as const,
    receiptDigest: input.receipt.digest,
    runId: input.receipt.runId,
    taskId: input.receipt.taskId,
    status: 'pass' as const,
    repairCount: input.receipt.recovery.repairCount,
    patchHistoryDigest: digestValue(input.receipt.patchDigests),
    nativeRuntimeEvidenceDigest: digestValue(input.nativeRuntimeEvidence),
  };
  const body = {
    schemaVersion: 1 as const,
    authority: DEVELOPMENT_AUTHORITY,
    kind: 'candidate-transaction-repair-evidence' as const,
    receiptBinding,
    nativeInvocationBindings: bindings,
    nativeRuntimeEvidence: input.nativeRuntimeEvidence,
    repairTransitions: input.repairTransitions,
    repairTransitionsDigest: digestValue(input.repairTransitions),
  };
  return parseCandidateTransactionEvidenceV1(
    { ...body, evidenceDigest: digestValue(body) },
    input.receipt,
  );
}

export function parseCandidateTransactionEvidenceV1(
  value: unknown,
  receipt: Receipt,
): CandidateTransactionEvidenceV1 {
  const input = asRecord(value, 'candidate transaction evidence');
  assertExactKeys(input, TOP_KEYS, 'candidate transaction evidence');
  if (input.schemaVersion !== 1
    || input.authority !== DEVELOPMENT_AUTHORITY
    || input.kind !== 'candidate-transaction-repair-evidence') {
    throw new TypeError('HARNESS_CANDIDATE_EVIDENCE_IDENTITY_INVALID');
  }
  const receiptBinding = parseReceiptBinding(input.receiptBinding, receipt);
  const bindings = parseInvocationBindings(input.nativeInvocationBindings);
  const nativeRuntimeEvidence = bindNativeRuntimeEvidenceV2({
    value: input.nativeRuntimeEvidence,
    taskId: receipt.taskId,
    runId: receipt.runId,
    hosts: receipt.hosts,
    expectations: bindings,
  });
  assertNativeBindings(receipt, bindings, nativeRuntimeEvidence);
  const repairTransitions = parseRepairTransitions(
    input.repairTransitions,
    receipt,
    nativeRuntimeEvidence,
  );
  const repairTransitionsDigest = parseDigest(
    input.repairTransitionsDigest,
    'candidate transaction evidence.repairTransitionsDigest',
  );
  if (repairTransitionsDigest !== digestValue(repairTransitions)) {
    throw new Error('HARNESS_CANDIDATE_EVIDENCE_TRANSITION_DIGEST_MISMATCH');
  }
  const body = {
    schemaVersion: 1 as const,
    authority: DEVELOPMENT_AUTHORITY,
    kind: 'candidate-transaction-repair-evidence' as const,
    receiptBinding,
    nativeInvocationBindings: bindings,
    nativeRuntimeEvidence,
    repairTransitions,
    repairTransitionsDigest,
  };
  const evidenceDigest = parseDigest(
    input.evidenceDigest,
    'candidate transaction evidence.evidenceDigest',
  );
  if (evidenceDigest !== digestValue(body)) {
    throw new Error('HARNESS_CANDIDATE_EVIDENCE_DIGEST_MISMATCH');
  }
  return deepFreeze({ ...body, evidenceDigest });
}

function parseReceiptBinding(value: unknown, receipt: Receipt) {
  const input = asRecord(value, 'candidate transaction evidence.receiptBinding');
  assertExactKeys(input, RECEIPT_KEYS, 'candidate transaction evidence.receiptBinding');
  const expected = {
    schemaVersion: 1 as const,
    receiptSchemaVersion: 3 as const,
    receiptDigest: receipt.digest,
    runId: receipt.runId,
    taskId: receipt.taskId,
    status: 'pass' as const,
    repairCount: receipt.recovery.repairCount,
    patchHistoryDigest: digestValue(receipt.patchDigests),
    nativeRuntimeEvidenceDigest: receipt.coordination.nativeRuntimeEvidenceDigest,
  };
  if (receipt.status !== 'pass' || expected.nativeRuntimeEvidenceDigest === null
    || Object.entries(expected).some(([key, item]) => input[key] !== item)) {
    throw new Error('HARNESS_CANDIDATE_EVIDENCE_RECEIPT_BINDING_MISMATCH');
  }
  return deepFreeze({
    ...expected,
    nativeRuntimeEvidenceDigest: parseDigest(
      expected.nativeRuntimeEvidenceDigest,
      'candidate transaction evidence receipt native runtime digest',
    ),
  });
}

function parseInvocationBindings(value: unknown) {
  if (!Array.isArray(value) || value.length === 0) {
    throw new TypeError('candidate transaction evidence native bindings must be non-empty');
  }
  const bindings = value.map((entry, index) => {
    const label = `candidate transaction evidence.nativeInvocationBindings[${index}]`;
    const input = asRecord(entry, label);
    assertExactKeys(input, INVOCATION_KEYS, label);
    const operation = parseOperation(input.operation, label);
    const patchPayloadSha256 = operation === 'implementation' || operation === 'repair'
      ? parseDigest(input.patchPayloadSha256, `${label}.patchPayloadSha256`)
      : parseNull(input.patchPayloadSha256, `${label}.patchPayloadSha256`);
    return deepFreeze({
      invocationId: asNonEmptyString(input.invocationId, `${label}.invocationId`),
      host: parseHost(input.host, `${label}.host`),
      operation,
      candidateTree: parseTree(input.candidateTree, `${label}.candidateTree`),
      patchPayloadSha256,
    });
  });
  if (new Set(bindings.map(({ invocationId }) => invocationId)).size !== bindings.length) {
    throw new Error('HARNESS_CANDIDATE_EVIDENCE_INVOCATION_DUPLICATE');
  }
  return deepFreeze(bindings);
}

function assertNativeBindings(
  receipt: Receipt,
  bindings: ReturnType<typeof parseInvocationBindings>,
  native: NativeRuntimeEvidenceV2,
): void {
  if (digestValue(native) !== receipt.coordination.nativeRuntimeEvidenceDigest
    || !sameStrings(receipt.coordination.nativeEvidenceDigests, [
      ...native.hosts.map(digestValue), ...native.invocations.map(digestValue),
    ])) {
    throw new Error('HARNESS_CANDIDATE_EVIDENCE_NATIVE_RECEIPT_MISMATCH');
  }
  if (!sameStrings(
    bindings.map(({ invocationId }) => invocationId),
    native.invocations.map(({ invocationId }) => invocationId),
  )) throw new Error('HARNESS_CANDIDATE_EVIDENCE_INVOCATION_ORDER_MISMATCH');
  const implementation = bindings.filter(({ operation }) => operation === 'implementation');
  const repairs = bindings.filter(({ operation }) => operation === 'repair');
  const architectures = bindings.filter(({ operation }) => operation === 'architecture');
  const finalReviews = bindings.filter(({ operation, candidateTree }) =>
    operation === 'review' && candidateTree === receipt.identities.candidate.tree);
  const trailingReviews = bindings.slice(-2);
  if (implementation.length !== 1
    || implementation[0].candidateTree !== receipt.identities.evaluator.tree
    || implementation[0].patchPayloadSha256 !== receipt.patchDigests[0]
    || repairs.length !== receipt.recovery.repairCount
    || architectures.some(({ candidateTree }) => candidateTree !== receipt.identities.evaluator.tree)
    || new Set(architectures.map(({ host }) => host)).size !== 2
    || finalReviews.length !== 2
    || new Set(finalReviews.map(({ host }) => host)).size !== 2
    || !sameStrings(
      trailingReviews.map(({ invocationId }) => invocationId),
      finalReviews.map(({ invocationId }) => invocationId),
    )) {
    throw new Error('HARNESS_CANDIDATE_EVIDENCE_NATIVE_ROLE_BINDING_MISMATCH');
  }
}

function parseRepairTransitions(
  value: unknown,
  receipt: Receipt,
  native: NativeRuntimeEvidenceV2,
): readonly CandidateRepairTransition[] {
  if (!Array.isArray(value) || value.length !== receipt.recovery.repairCount) {
    throw new Error('HARNESS_CANDIDATE_EVIDENCE_TRANSITION_COUNT_MISMATCH');
  }
  const repairs = new Map(native.invocations
    .filter(({ operation }) => operation === 'repair')
    .map((invocation) => [invocation.invocationId, invocation]));
  const usedRepairInvocationIds = new Set<string>();
  const transitions = value.map((entry, index) => {
    const label = `candidate transaction evidence.repairTransitions[${index}]`;
    const input = asRecord(entry, label);
    assertExactKeys(input, TRANSITION_KEYS, label);
    const trigger = parseTrigger(input.trigger, label);
    const phase: CandidateRepairTransition['phase'] = trigger === 'patch-admission'
      ? 'pre-admission' : 'post-admission';
    const buildDisposition: CandidateRepairTransition['buildDisposition'] =
      trigger === 'patch-admission' || trigger === 'admission-validation'
      ? 'not-started' : trigger === 'build' ? 'failed' : 'passed';
    const fromAttempt = asInteger(input.fromAttempt, `${label}.fromAttempt`);
    const toAttempt = asInteger(input.toAttempt, `${label}.toAttempt`);
    const sourceCandidate = parseIdentity(input.sourceCandidate, `${label}.sourceCandidate`);
    const repairResetIdentity = input.repairResetIdentity === null
      ? null : parseIdentity(input.repairResetIdentity, `${label}.repairResetIdentity`);
    const resetIdentity = parseIdentity(input.resetIdentity, `${label}.resetIdentity`);
    const nativeInput = asRecord(input.nativeInvocation, `${label}.nativeInvocation`);
    const invocationId = asNonEmptyString(
      nativeInput.invocationId,
      `${label}.nativeInvocation.invocationId`,
    );
    const nativeInvocation = repairs.get(invocationId);
    const reasonDigests = parseDigestArray(input.reasonDigests, `${label}.reasonDigests`);
    const sourcePatchDigest = parseDigest(input.sourcePatchDigest, `${label}.sourcePatchDigest`);
    const replacementPatchDigest = parseDigest(
      input.replacementPatchDigest,
      `${label}.replacementPatchDigest`,
    );
    if (input.schemaVersion !== 1 || fromAttempt !== index || toAttempt !== index + 1
      || input.phase !== phase || input.buildDisposition !== buildDisposition
      || sourcePatchDigest !== receipt.patchDigests[index]
      || replacementPatchDigest !== receipt.patchDigests[index + 1]
      || !sameIdentity(resetIdentity, receipt.identities.evaluator)
      || nativeInvocation === undefined || digestValue(nativeInput) !== digestValue(nativeInvocation)
      || usedRepairInvocationIds.has(invocationId)
      || nativeInvocation.candidateTree !== sourceCandidate.tree
      || nativeInvocation.patchPayloadSha256 !== replacementPatchDigest
      || sourcePatchDigest === replacementPatchDigest
      || (trigger === 'patch-admission'
        ? !sameIdentity(sourceCandidate, receipt.identities.evaluator)
        : sourceCandidate.tree === receipt.identities.evaluator.tree)
      || (trigger === 'patch-admission'
        ? repairResetIdentity === null || !sameIdentity(repairResetIdentity, sourceCandidate)
        : repairResetIdentity !== null)) {
      throw new Error('HARNESS_CANDIDATE_EVIDENCE_TRANSITION_BINDING_MISMATCH');
    }
    usedRepairInvocationIds.add(invocationId);
    const body = {
      schemaVersion: 1 as const, fromAttempt, toAttempt, phase, trigger, buildDisposition,
      sourcePatchDigest, replacementPatchDigest, sourceCandidate, repairResetIdentity,
      resetIdentity, reasonDigests, nativeInvocation,
    };
    const digest = parseDigest(input.digest, `${label}.digest`);
    if (digest !== digestValue(body)) {
      throw new Error('HARNESS_CANDIDATE_EVIDENCE_TRANSITION_DIGEST_MISMATCH');
    }
    return deepFreeze({ ...body, digest });
  });
  if (repairs.size !== transitions.length || usedRepairInvocationIds.size !== repairs.size
    || [...repairs.keys()].some((invocationId) => !usedRepairInvocationIds.has(invocationId))) {
    throw new Error('HARNESS_CANDIDATE_EVIDENCE_REPAIR_INVOCATION_UNBOUND');
  }
  return deepFreeze(transitions);
}

function parseOperation(value: unknown, label: string): NativeModelOperation {
  if (value !== 'architecture' && value !== 'implementation'
    && value !== 'repair' && value !== 'review') throw new TypeError(`${label}.operation is invalid`);
  return value;
}

function parseTrigger(value: unknown, label: string): CandidateRepairTransition['trigger'] {
  if (value !== 'patch-admission' && value !== 'admission-validation' && value !== 'build'
    && value !== 'verification' && value !== 'review' && value !== 'final-admission') {
    throw new TypeError(`${label}.trigger is invalid`);
  }
  return value;
}

function parseHost(value: unknown, label: string): 'codex' | 'claude-code' {
  if (value !== 'codex' && value !== 'claude-code') throw new TypeError(`${label} is invalid`);
  return value;
}

function parseIdentity(value: unknown, label: string): GitIdentity {
  const input = asRecord(value, label);
  assertExactKeys(input, ['commit', 'tree'], label);
  return deepFreeze({
    commit: parseTree(input.commit, `${label}.commit`),
    tree: parseTree(input.tree, `${label}.tree`),
  });
}

function parseTree(value: unknown, label: string): string {
  const text = asNonEmptyString(value, label);
  if (!/^[a-f0-9]{40,64}$/.test(text)) throw new TypeError(`${label} is invalid`);
  return text;
}

function parseDigestArray(value: unknown, label: string): readonly string[] {
  if (!Array.isArray(value) || value.length === 0) throw new TypeError(`${label} must be non-empty`);
  return deepFreeze(value.map((entry, index) => parseDigest(entry, `${label}[${index}]`)));
}

function parseDigest(value: unknown, label: string): string {
  const text = asNonEmptyString(value, label);
  if (!SHA256_PATTERN.test(text) || text === '0'.repeat(64)) {
    throw new TypeError(`${label} must be a non-genesis SHA-256 digest`);
  }
  return text;
}

function parseNull(value: unknown, label: string): null {
  if (value !== null) throw new TypeError(`${label} must be null`);
  return null;
}

function sameStrings(left: readonly string[], right: readonly string[]): boolean {
  return left.length === right.length && left.every((entry, index) => entry === right[index]);
}

function sameIdentity(left: GitIdentity, right: GitIdentity): boolean {
  return left.commit === right.commit && left.tree === right.tree;
}
