// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import type { CandidateTransactionEvidenceV1 } from './candidate-transaction-evidence-v1.js';
import {
  DEVELOPMENT_AUTHORITY,
  SHA256_PATTERN,
  asNonEmptyString,
  asRecord,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import {
  parseProgrammeV5RufloEvidence,
  type ProgrammeV5RufloEvidence,
} from './evidence.js';
import { failureCodeForReason } from './failure-code.js';
import {
  parseMetaHarnessDiagnosticSnapshot,
  type MetaHarnessDiagnosticSnapshot,
} from './metaharness-diagnostics.js';
import { evaluateProgrammeGatesV6 } from './programme-gates-v6.js';
import {
  verifyFrozenProgrammePolicyV2,
  type FrozenProgrammePolicyV2,
  type ParsedProgrammePolicyV2,
} from './programme-policy-v6.js';
import type { ProgrammeAcceptanceResult } from './programme-acceptance.js';
import { scoreProgrammeReceiptV5 } from './programme-score-v5.js';
import {
  ReceiptChain,
  digestValue,
  type Receipt,
  type ReceiptStatus,
} from './receipts.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

const ENVELOPE_KEYS = [
  'schemaVersion', 'authority', 'policy', 'policyFingerprint', 'rufloEvidence',
  'rufloEvidenceDigest', 'receiptChain', 'candidateTransactionEvidence',
  'candidateTransactionEvidenceDigest', 'diagnosticBlob', 'diagnosticBlobDigest', 'programmeAcceptance',
  'programmeAcceptanceDigest', 'envelopeDigest',
] as const;
const CREATE_KEYS = [
  'policy', 'rufloEvidence', 'candidateTransactionEvidence', 'receipt', 'diagnosticBlob',
] as const;
const FINALIZE_KEYS = [
  'expectedPolicyFingerprint', 'transactionStatus', 'transactionReason', 'envelope',
] as const;

export interface ProgrammeReceiptChainV6 {
  readonly schemaVersion: 3;
  readonly receipts: readonly Receipt[];
}

export interface ProgrammeEnvelopeV6 {
  readonly schemaVersion: 6;
  readonly authority: typeof DEVELOPMENT_AUTHORITY;
  readonly policy: FrozenProgrammePolicyV2;
  readonly policyFingerprint: string;
  readonly rufloEvidence: ProgrammeV5RufloEvidence;
  readonly rufloEvidenceDigest: string;
  readonly receiptChain: ProgrammeReceiptChainV6;
  readonly candidateTransactionEvidence: CandidateTransactionEvidenceV1 | null;
  readonly candidateTransactionEvidenceDigest: string | null;
  readonly diagnosticBlob: string;
  readonly diagnosticBlobDigest: string;
  readonly programmeAcceptance: ProgrammeAcceptanceResult;
  readonly programmeAcceptanceDigest: string;
  readonly envelopeDigest: string;
}

export interface ProgrammeEnvelopeInputV6 {
  readonly policy: FrozenProgrammePolicyV2;
  readonly rufloEvidence: ProgrammeV5RufloEvidence;
  readonly candidateTransactionEvidence: CandidateTransactionEvidenceV1 | null;
  readonly receipt: Receipt;
  readonly diagnosticBlob: string;
}

export function createProgrammeEnvelopeV6(
  value: ProgrammeEnvelopeInputV6,
  expectedPolicyFingerprint: string,
): ProgrammeEnvelopeV6 {
  const anchor = parseExternalAnchor(expectedPolicyFingerprint);
  const input = asRecord(value, 'programme envelope V6 creation input');
  assertExactKeys(input, CREATE_KEYS, 'programme envelope V6 creation input');
  return assembleEnvelope({
    policy: input.policy,
    rufloEvidence: input.rufloEvidence,
    candidateTransactionEvidence: input.candidateTransactionEvidence,
    receiptChain: { schemaVersion: 3, receipts: [input.receipt] },
    diagnosticBlob: input.diagnosticBlob,
  }, anchor);
}

export function parseProgrammeEnvelopeV6(
  serialized: string,
  expectedPolicyFingerprint: string,
): ProgrammeEnvelopeV6 {
  const anchor = parseExternalAnchor(expectedPolicyFingerprint);
  const text = asNonEmptyString(serialized, 'programme envelope V6 serialization');
  const input = asRecord(
    parseJsonWithoutDuplicateKeys(text, 'programme envelope V6'),
    'programme envelope V6',
  );
  assertExactKeys(input, ENVELOPE_KEYS, 'programme envelope V6');
  if (input.schemaVersion !== 6 || input.authority !== DEVELOPMENT_AUTHORITY) {
    throw new TypeError('HARNESS_PROGRAMME_ENVELOPE_V6_IDENTITY_INVALID');
  }
  if (input.policyFingerprint !== anchor) {
    throw new Error('HARNESS_PROGRAMME_POLICY_V2_FINGERPRINT_MISMATCH');
  }
  const expected = assembleEnvelope({
    policy: input.policy,
    rufloEvidence: input.rufloEvidence,
    candidateTransactionEvidence: input.candidateTransactionEvidence,
    receiptChain: input.receiptChain,
    diagnosticBlob: input.diagnosticBlob,
  }, anchor);
  if (input.rufloEvidenceDigest !== expected.rufloEvidenceDigest
    || input.candidateTransactionEvidenceDigest !== expected.candidateTransactionEvidenceDigest
    || input.diagnosticBlobDigest !== expected.diagnosticBlobDigest
    || digestValue(input.programmeAcceptance) !== expected.programmeAcceptanceDigest
    || input.programmeAcceptanceDigest !== expected.programmeAcceptanceDigest
    || input.envelopeDigest !== expected.envelopeDigest) {
    throw new Error('HARNESS_PROGRAMME_ENVELOPE_V6_DIGEST_INVALID');
  }
  return expected;
}

export function serializeProgrammeEnvelopeV6(
  envelope: ProgrammeEnvelopeV6,
  expectedPolicyFingerprint: string,
): string {
  const anchor = parseExternalAnchor(expectedPolicyFingerprint);
  const verified = parseProgrammeEnvelopeV6(JSON.stringify(envelope), anchor);
  return `${JSON.stringify(verified, null, 2)}\n`;
}

export function finalizeProgrammeOutcomeV6(value: Readonly<{
  expectedPolicyFingerprint: string;
  transactionStatus: ReceiptStatus;
  transactionReason: string | null;
  envelope: ProgrammeEnvelopeV6;
}>): Readonly<{ status: ReceiptStatus; reason: string | null }> {
  const input = asRecord(value, 'programme envelope V6 finalization input');
  assertExactKeys(input, FINALIZE_KEYS, 'programme envelope V6 finalization input');
  const anchor = parseExternalAnchor(input.expectedPolicyFingerprint);
  const envelope = parseProgrammeEnvelopeV6(JSON.stringify(input.envelope), anchor);
  const status = parseReceiptStatus(input.transactionStatus);
  const reason = input.transactionReason;
  if (reason !== null && typeof reason !== 'string') {
    throw new TypeError('programme envelope V6 transactionReason is invalid');
  }
  const receipt = envelope.receiptChain.receipts[0];
  if (receipt === undefined || receipt.status !== status) {
    throw new Error('HARNESS_PROGRAMME_ENVELOPE_V6_TRANSACTION_STATUS_MISMATCH');
  }
  if (status === 'pass') {
    if (reason !== null) throw new Error('HARNESS_PROGRAMME_ENVELOPE_V6_PASS_REASON_INVALID');
    return envelope.programmeAcceptance.status === 'ACCEPTED'
      ? deepFreeze({ status: 'pass', reason: null })
      : deepFreeze({ status: 'gated', reason: 'HARNESS_PROGRAMME_ACCEPTANCE_REJECTED' });
  }
  const failureCode = failureCodeForReason(reason);
  if (receipt.failureCode !== failureCode) {
    throw new Error('HARNESS_PROGRAMME_ENVELOPE_V6_FAILURE_CODE_MISMATCH');
  }
  return deepFreeze({ status, reason: failureCode });
}

function assembleEnvelope(input: Readonly<{
  policy: unknown;
  rufloEvidence: unknown;
  candidateTransactionEvidence: unknown;
  receiptChain: unknown;
  diagnosticBlob: unknown;
}>, anchor: string): ProgrammeEnvelopeV6 {
  const policy = verifyFrozenProgrammePolicyV2(input.policy, anchor);
  const receiptChain = verifyOneReceipt(input.receiptChain);
  const receipt = receiptChain.receipts[0];
  if (receipt === undefined) throw new Error('HARNESS_PROGRAMME_ENVELOPE_V6_RECEIPT_MISSING');
  const rufloEvidence = parseProgrammeV5RufloEvidence(input.rufloEvidence);
  const rufloEvidenceDigest = digestValue(rufloEvidence);
  const diagnostics = parseDiagnosticBlob(input.diagnosticBlob, policy);
  const baseEnvelope = policy.base.snapshot.gateContract.envelope;
  if (receipt.protectedInputs[baseEnvelope.diagnosticBlobPath] !== diagnostics.digest) {
    throw new Error('HARNESS_PROGRAMME_ENVELOPE_V6_DIAGNOSTIC_BLOB_MISMATCH');
  }
  const gateEvaluation = evaluateProgrammeGatesV6({
    policy,
    receipt,
    diagnostics: diagnostics.snapshot,
    rufloEvidence,
    candidateTransactionEvidence: input.candidateTransactionEvidence,
  });
  const candidateTransactionEvidence = gateEvaluation.candidateTransactionEvidence;
  const candidateTransactionEvidenceDigest = candidateTransactionEvidence?.evidenceDigest ?? null;
  const programmeAcceptance = scoreProgrammeReceiptV5({
    gateContract: policy.base.snapshot.gateContract,
    receipt,
    gateEvaluation: {
      dimensions: gateEvaluation.dimensions,
      diagnostics: gateEvaluation.diagnostics,
    },
  });
  const programmeAcceptanceDigest = digestValue(programmeAcceptance);
  const body = {
    schemaVersion: 6 as const,
    authority: DEVELOPMENT_AUTHORITY,
    policy: policy.snapshot,
    policyFingerprint: policy.fingerprint,
    rufloEvidence,
    rufloEvidenceDigest,
    receiptChain,
    candidateTransactionEvidence,
    candidateTransactionEvidenceDigest,
    diagnosticBlob: diagnostics.blob,
    diagnosticBlobDigest: diagnostics.digest,
    programmeAcceptance,
    programmeAcceptanceDigest,
  };
  return deepFreeze({ ...body, envelopeDigest: digestValue(body) });
}

function verifyOneReceipt(value: unknown): ProgrammeReceiptChainV6 {
  const chain = ReceiptChain.import(JSON.stringify(value));
  if (chain.length !== 1) {
    throw new Error('HARNESS_PROGRAMME_ENVELOPE_V6_RECEIPT_COUNT_INVALID');
  }
  return deepFreeze(JSON.parse(chain.export()) as ProgrammeReceiptChainV6);
}

function parseDiagnosticBlob(
  value: unknown,
  policy: ParsedProgrammePolicyV2,
): Readonly<{ blob: string; digest: string; snapshot: MetaHarnessDiagnosticSnapshot }> {
  const blob = asNonEmptyString(value, 'programme envelope V6 diagnosticBlob');
  if (Buffer.byteLength(blob, 'utf8')
    > policy.base.snapshot.gateContract.envelope.diagnosticBlobMaximumBytes) {
    throw new TypeError('programme envelope V6 diagnosticBlob is too large');
  }
  const snapshot = parseMetaHarnessDiagnosticSnapshot(
    parseJsonWithoutDuplicateKeys(blob, 'programme envelope V6 diagnosticBlob'),
  );
  return deepFreeze({ blob, digest: sha256(blob), snapshot });
}

function parseExternalAnchor(value: unknown): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || value === '0'.repeat(64)) {
    throw new TypeError('HARNESS_PROGRAMME_ENVELOPE_V6_POLICY_ANCHOR_INVALID');
  }
  return value;
}

function parseReceiptStatus(value: unknown): ReceiptStatus {
  if (value !== 'pass' && value !== 'fail' && value !== 'gated' && value !== 'cancelled') {
    throw new TypeError('programme envelope V6 transactionStatus is invalid');
  }
  return value;
}

function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}
