// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  DEVELOPMENT_AUTHORITY,
  SHA256_PATTERN,
  asNonEmptyString,
  asRecord,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import {
  parseProgrammeV5RufloEvidence, type ProgrammeV5RufloEvidence,
} from './evidence.js';
import { failureCodeForReason } from './failure-code.js';
import {
  parseMetaHarnessDiagnosticSnapshot,
  type MetaHarnessDiagnosticSnapshot,
} from './metaharness-diagnostics.js';
import { evaluateProgrammeGatesV5 } from './programme-gates-v5.js';
import {
  verifyFrozenProgrammePolicyV1,
  type FrozenProgrammePolicyV1,
  type ParsedProgrammePolicyV1,
} from './programme-policy-v5.js';
import { scoreProgrammeReceiptV5 } from './programme-score-v5.js';
import {
  ReceiptChain,
  digestValue,
  type Receipt,
  type ReceiptStatus,
} from './receipts.js';
import type { ProgrammeAcceptanceResult } from './programme-acceptance.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

const ENVELOPE_KEYS = [
  'schemaVersion', 'authority', 'policy', 'policyFingerprint',
  'rufloEvidence', 'rufloEvidenceDigest', 'receiptChain', 'diagnosticBlob',
  'diagnosticBlobDigest', 'programmeAcceptance', 'programmeAcceptanceDigest',
  'envelopeDigest',
] as const;
const CREATE_KEYS = ['policy', 'rufloEvidence', 'receipt', 'diagnosticBlob'] as const;
const FINALIZE_KEYS = [
  'expectedPolicyFingerprint', 'transactionStatus', 'transactionReason', 'envelope',
] as const;

export interface ProgrammeReceiptChainV5 {
  readonly schemaVersion: 3;
  readonly receipts: readonly Receipt[];
}

export interface ProgrammeEnvelopeV5 {
  readonly schemaVersion: 5;
  readonly authority: typeof DEVELOPMENT_AUTHORITY;
  readonly policy: FrozenProgrammePolicyV1;
  readonly policyFingerprint: string;
  readonly rufloEvidence: ProgrammeV5RufloEvidence;
  readonly rufloEvidenceDigest: string;
  readonly receiptChain: ProgrammeReceiptChainV5;
  readonly diagnosticBlob: string;
  readonly diagnosticBlobDigest: string;
  readonly programmeAcceptance: ProgrammeAcceptanceResult;
  readonly programmeAcceptanceDigest: string;
  readonly envelopeDigest: string;
}

export interface ProgrammeEnvelopeInputV5 {
  readonly policy: FrozenProgrammePolicyV1;
  readonly rufloEvidence: ProgrammeV5RufloEvidence;
  readonly receipt: Receipt;
  readonly diagnosticBlob: string;
}

export function createProgrammeEnvelopeV5(
  value: ProgrammeEnvelopeInputV5,
  expectedPolicyFingerprint: string,
): ProgrammeEnvelopeV5 {
  const anchor = parseExternalAnchor(expectedPolicyFingerprint);
  const input = asRecord(value, 'programme envelope v5 creation input');
  assertExactKeys(input, CREATE_KEYS, 'programme envelope v5 creation input');
  return assembleEnvelope({
    policy: input.policy,
    rufloEvidence: input.rufloEvidence,
    receiptChain: { schemaVersion: 3, receipts: [input.receipt] },
    diagnosticBlob: input.diagnosticBlob,
  }, anchor);
}

export function parseProgrammeEnvelopeV5(
  serialized: string,
  expectedPolicyFingerprint: string,
): ProgrammeEnvelopeV5 {
  const anchor = parseExternalAnchor(expectedPolicyFingerprint);
  const text = asNonEmptyString(serialized, 'programme envelope v5 serialization');
  const input = asRecord(
    parseJsonWithoutDuplicateKeys(text, 'programme envelope v5'),
    'programme envelope v5',
  );
  assertExactKeys(input, ENVELOPE_KEYS, 'programme envelope v5');
  if (input.schemaVersion !== 5 || input.authority !== DEVELOPMENT_AUTHORITY) {
    throw new TypeError('HARNESS_PROGRAMME_ENVELOPE_V5_IDENTITY_INVALID');
  }
  if (input.policyFingerprint !== anchor) {
    throw new Error('HARNESS_PROGRAMME_POLICY_FINGERPRINT_MISMATCH');
  }
  const expected = assembleEnvelope({
    policy: input.policy,
    rufloEvidence: input.rufloEvidence,
    receiptChain: input.receiptChain,
    diagnosticBlob: input.diagnosticBlob,
  }, anchor);
  if (input.rufloEvidenceDigest !== expected.rufloEvidenceDigest
    || input.diagnosticBlobDigest !== expected.diagnosticBlobDigest
    || digestValue(input.programmeAcceptance) !== expected.programmeAcceptanceDigest
    || input.programmeAcceptanceDigest !== expected.programmeAcceptanceDigest
    || input.envelopeDigest !== expected.envelopeDigest) {
    throw new Error('HARNESS_PROGRAMME_ENVELOPE_V5_DIGEST_INVALID');
  }
  return expected;
}

export function serializeProgrammeEnvelopeV5(
  envelope: ProgrammeEnvelopeV5,
  expectedPolicyFingerprint: string,
): string {
  const anchor = parseExternalAnchor(expectedPolicyFingerprint);
  const verified = parseProgrammeEnvelopeV5(JSON.stringify(envelope), anchor);
  return `${JSON.stringify(verified, null, 2)}\n`;
}

export function finalizeProgrammeOutcomeV5(value: Readonly<{
  expectedPolicyFingerprint: string;
  transactionStatus: ReceiptStatus;
  transactionReason: string | null;
  envelope: ProgrammeEnvelopeV5;
}>): Readonly<{ status: ReceiptStatus; reason: string | null }> {
  const input = asRecord(value, 'programme envelope v5 finalization input');
  assertExactKeys(input, FINALIZE_KEYS, 'programme envelope v5 finalization input');
  const anchor = parseExternalAnchor(input.expectedPolicyFingerprint);
  const envelope = parseProgrammeEnvelopeV5(JSON.stringify(input.envelope), anchor);
  const status = parseReceiptStatus(input.transactionStatus);
  const reason = input.transactionReason;
  if (reason !== null && typeof reason !== 'string') {
    throw new TypeError('programme envelope v5 transactionReason is invalid');
  }
  const receipt = envelope.receiptChain.receipts[0];
  if (receipt === undefined || receipt.status !== status) {
    throw new Error('HARNESS_PROGRAMME_ENVELOPE_V5_TRANSACTION_STATUS_MISMATCH');
  }
  if (status === 'pass') {
    if (reason !== null) throw new Error('HARNESS_PROGRAMME_ENVELOPE_V5_PASS_REASON_INVALID');
    return envelope.programmeAcceptance.status === 'ACCEPTED'
      ? deepFreeze({ status: 'pass', reason: null })
      : deepFreeze({ status: 'gated', reason: 'HARNESS_PROGRAMME_ACCEPTANCE_REJECTED' });
  }
  const failureCode = failureCodeForReason(reason);
  if (receipt.failureCode !== failureCode) {
    throw new Error('HARNESS_PROGRAMME_ENVELOPE_V5_FAILURE_CODE_MISMATCH');
  }
  return deepFreeze({ status, reason: failureCode });
}

function assembleEnvelope(input: Readonly<{
  policy: unknown;
  rufloEvidence: unknown;
  receiptChain: unknown;
  diagnosticBlob: unknown;
}>, anchor: string): ProgrammeEnvelopeV5 {
  const policy = verifyFrozenProgrammePolicyV1(input.policy, anchor);
  const receiptChain = verifyOneReceipt(input.receiptChain);
  const receipt = receiptChain.receipts[0];
  if (receipt === undefined) throw new Error('HARNESS_PROGRAMME_ENVELOPE_V5_RECEIPT_MISSING');
  const rufloEvidence = parseProgrammeV5RufloEvidence(input.rufloEvidence);
  const rufloEvidenceDigest = digestValue(rufloEvidence);
  const diagnostics = parseDiagnosticBlob(input.diagnosticBlob, policy);
  if (receipt.protectedInputs[policy.snapshot.gateContract.envelope.diagnosticBlobPath]
    !== diagnostics.digest) {
    throw new Error('HARNESS_PROGRAMME_ENVELOPE_V5_DIAGNOSTIC_BLOB_MISMATCH');
  }
  const gateEvaluation = evaluateProgrammeGatesV5({
    policy,
    receipt,
    diagnostics: diagnostics.snapshot,
    rufloEvidence,
  });
  const programmeAcceptance = scoreProgrammeReceiptV5({
    gateContract: policy.snapshot.gateContract,
    receipt,
    gateEvaluation,
  });
  const programmeAcceptanceDigest = digestValue(programmeAcceptance);
  const body = {
    schemaVersion: 5 as const,
    authority: DEVELOPMENT_AUTHORITY,
    policy: policy.snapshot,
    policyFingerprint: policy.fingerprint,
    rufloEvidence,
    rufloEvidenceDigest,
    receiptChain,
    diagnosticBlob: diagnostics.blob,
    diagnosticBlobDigest: diagnostics.digest,
    programmeAcceptance,
    programmeAcceptanceDigest,
  };
  return deepFreeze({ ...body, envelopeDigest: digestValue(body) });
}

function verifyOneReceipt(value: unknown): ProgrammeReceiptChainV5 {
  const chain = ReceiptChain.import(JSON.stringify(value));
  if (chain.length !== 1) {
    throw new Error('HARNESS_PROGRAMME_ENVELOPE_V5_RECEIPT_COUNT_INVALID');
  }
  return deepFreeze(JSON.parse(chain.export()) as ProgrammeReceiptChainV5);
}

function parseDiagnosticBlob(
  value: unknown,
  policy: ParsedProgrammePolicyV1,
): Readonly<{ blob: string; digest: string; snapshot: MetaHarnessDiagnosticSnapshot }> {
  const blob = asNonEmptyString(value, 'programme envelope v5 diagnosticBlob');
  if (Buffer.byteLength(blob, 'utf8')
    > policy.snapshot.gateContract.envelope.diagnosticBlobMaximumBytes) {
    throw new TypeError('programme envelope v5 diagnosticBlob is too large');
  }
  const snapshot = parseMetaHarnessDiagnosticSnapshot(
    parseJsonWithoutDuplicateKeys(blob, 'programme envelope v5 diagnosticBlob'),
  );
  return deepFreeze({ blob, digest: sha256(blob), snapshot });
}

function parseExternalAnchor(value: unknown): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || value === '0'.repeat(64)) {
    throw new TypeError('HARNESS_PROGRAMME_ENVELOPE_V5_POLICY_ANCHOR_INVALID');
  }
  return value;
}

function parseReceiptStatus(value: unknown): ReceiptStatus {
  if (value !== 'pass' && value !== 'fail' && value !== 'gated' && value !== 'cancelled') {
    throw new TypeError('programme envelope v5 transactionStatus is invalid');
  }
  return value;
}

function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}
