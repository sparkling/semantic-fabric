// SPDX-License-Identifier: MIT

import { DEVELOPMENT_AUTHORITY, SHA256_PATTERN, asClosedRecord, assertExactKeys,
  deepFreeze } from './contracts.js';
import type { ProgrammeCaptureRunClaimAuthorityInputV1 }
  from './programme-capture-claim-io-v1.js';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_VALIDATION_DIGEST_DOMAIN_V1,
  parseProgrammeCaptureSupervisorClaimAcknowledgementV1,
  parseProgrammeCaptureSupervisorClaimRequestV1,
  verifyProgrammeCaptureSupervisorClaimAcknowledgementV1,
  type ProgrammeCaptureSupervisorClaimRequestV1,
  type ProgrammeCaptureSupervisorClaimValidationV1,
} from './programme-capture-supervisor-claim-v1.js';
import { digestValue } from './receipts.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export const PROGRAMME_CAPTURE_SUPERVISOR_REQUEST_MAX_BYTES_V1 = 131_072;
export const PROGRAMME_CAPTURE_SUPERVISOR_VALIDATION_MAX_BYTES_V1 = 16_384;
const NON_AUTHORITY = Object.freeze({
  externalAppendOnlyWitness: false as const,
  appendOnlyPersistenceVerified: false as const,
  rollbackResistance: 'not-proven' as const,
  supervisorAdministration: 'not-attested' as const,
  hostAdmission: 'not-evaluated' as const,
  runnerLeaseAcquired: false as const,
  stateTransitionAuthorized: false as const,
  attemptStartAuthorized: false as const,
  captureAuthorized: false as const,
});
const NON_AUTHORITY_KEYS = Object.keys(NON_AUTHORITY);
const VERIFICATION_KEYS = [
  'claimAuthority', 'serializedEnvelope', 'trustedPublicKeySpkiDer',
  'expectedAuthorityKeyFingerprint', 'expectedSupervisorId', 'expectedLogId',
  'expectedKeyEpoch', 'expectedLogSequence', 'expectedPreviousCheckpointDigest',
] as const;
interface SupervisorVerificationInputV1 {
  readonly claimAuthority: ProgrammeCaptureRunClaimAuthorityInputV1;
  readonly serializedEnvelope: string; readonly trustedPublicKeySpkiDer: Uint8Array;
  readonly expectedAuthorityKeyFingerprint: string; readonly expectedSupervisorId: string;
  readonly expectedLogId: string; readonly expectedKeyEpoch: number;
  readonly expectedLogSequence: number; readonly expectedPreviousCheckpointDigest: string;
}

export function serializeProgrammeCaptureSupervisorClaimRequestV1(value: unknown): string {
  return `${JSON.stringify(parseProgrammeCaptureSupervisorClaimRequestV1(value), null, 2)}\n`;
}

export function parseProgrammeCaptureSupervisorClaimRequestBlobV1(
  serialized: string,
): ProgrammeCaptureSupervisorClaimRequestV1 {
  return parseCanonicalBlob(
    serialized, PROGRAMME_CAPTURE_SUPERVISOR_REQUEST_MAX_BYTES_V1, 'request',
    parseProgrammeCaptureSupervisorClaimRequestV1,
    serializeProgrammeCaptureSupervisorClaimRequestV1,
  );
}

function parseProgrammeCaptureSupervisorClaimValidationRecordV1(
  value: unknown,
): ProgrammeCaptureSupervisorClaimValidationV1 {
  const input = asClosedRecord(value, 'programme capture supervisor validation');
  assertExactKeys(input, [
    'schemaVersion', 'transactionKind', 'evidenceKind', 'authority', 'runId',
    'projectAuthorityDigest', 'claimKeyDigest', 'claimDigest', 'requestDigest',
    'acknowledgementDigest', 'serializedEnvelopeDigest', 'supervisor', 'event', 'verificationScope',
    'signatureVerified', 'suppliedCheckpointReferenceMatched', ...NON_AUTHORITY_KEYS,
    'validationDigest',
  ], 'programme capture supervisor validation');
  if (input.schemaVersion !== 1 || input.transactionKind !== 'programme-capture-v1'
    || input.evidenceKind !== 'non-authorizing-supervisor-claim-validation-v1'
    || input.authority !== DEVELOPMENT_AUTHORITY
    || input.verificationScope !== 'signature-and-rooted-claim-binding-only'
    || input.signatureVerified !== true || input.suppliedCheckpointReferenceMatched !== true) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_VALIDATION_IDENTITY_INVALID');
  }
  const acknowledgement = parseProgrammeCaptureSupervisorClaimAcknowledgementV1({
    schemaVersion: 1,
    transactionKind: 'programme-capture-v1',
    recordKind: 'supervisor-claim-registration-acknowledgement-v1',
    authority: DEVELOPMENT_AUTHORITY,
    runId: input.runId,
    projectAuthorityDigest: input.projectAuthorityDigest,
    claimKeyDigest: input.claimKeyDigest,
    claimDigest: input.claimDigest,
    requestDigest: input.requestDigest,
    supervisor: input.supervisor,
    event: input.event,
    verificationScope: 'signature-and-claim-binding-only',
    ...Object.fromEntries(NON_AUTHORITY_KEYS.map((key) => [key, input[key]])),
    acknowledgementDigest: input.acknowledgementDigest,
  });
  const body = {
    schemaVersion: 1 as const,
    transactionKind: 'programme-capture-v1' as const,
    evidenceKind: 'non-authorizing-supervisor-claim-validation-v1' as const,
    authority: DEVELOPMENT_AUTHORITY,
    runId: acknowledgement.runId,
    projectAuthorityDigest: acknowledgement.projectAuthorityDigest,
    claimKeyDigest: acknowledgement.claimKeyDigest,
    claimDigest: acknowledgement.claimDigest,
    requestDigest: acknowledgement.requestDigest,
    acknowledgementDigest: acknowledgement.acknowledgementDigest,
    serializedEnvelopeDigest: parseDigest(input.serializedEnvelopeDigest),
    supervisor: acknowledgement.supervisor,
    event: acknowledgement.event,
    verificationScope: 'signature-and-rooted-claim-binding-only' as const,
    signatureVerified: true as const,
    suppliedCheckpointReferenceMatched: true as const,
    ...NON_AUTHORITY,
  };
  const validationDigest = parseDigest(input.validationDigest);
  if (validationDigest !== digestValue({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_VALIDATION_DIGEST_DOMAIN_V1, validation: body,
  })) throw new Error('HARNESS_CAPTURE_SUPERVISOR_VALIDATION_DIGEST_MISMATCH');
  return deepFreeze({ ...body, validationDigest });
}

function serializeProgrammeCaptureSupervisorClaimValidationRecordV1(value: unknown): string {
  return `${JSON.stringify(parseProgrammeCaptureSupervisorClaimValidationRecordV1(value), null, 2)}\n`;
}

function parseProgrammeCaptureSupervisorClaimValidationBlobV1(
  serialized: string,
): ProgrammeCaptureSupervisorClaimValidationV1 {
  return parseCanonicalBlob(
    serialized, PROGRAMME_CAPTURE_SUPERVISOR_VALIDATION_MAX_BYTES_V1, 'validation',
    parseProgrammeCaptureSupervisorClaimValidationRecordV1,
    serializeProgrammeCaptureSupervisorClaimValidationRecordV1,
  );
}

export async function createProgrammeCaptureSupervisorClaimValidationBlobV1(
  value: SupervisorVerificationInputV1,
): Promise<string> {
  const input = asClosedRecord(value, 'programme capture supervisor validation creation input');
  assertExactKeys(input, VERIFICATION_KEYS, 'programme capture supervisor validation creation input');
  return serializeProgrammeCaptureSupervisorClaimValidationRecordV1(
    await verifyFromInput(input),
  );
}

export async function replayProgrammeCaptureSupervisorClaimValidationV1(
  value: SupervisorVerificationInputV1 & Readonly<{
    serializedValidation: string;
  }>,
): Promise<ProgrammeCaptureSupervisorClaimValidationV1> {
  const input = asClosedRecord(value, 'programme capture supervisor validation replay input');
  assertExactKeys(
    input, ['serializedValidation', ...VERIFICATION_KEYS],
    'programme capture supervisor validation replay input',
  );
  parseProgrammeCaptureSupervisorClaimValidationBlobV1(input.serializedValidation as string);
  const replayed = await verifyFromInput(input);
  if (serializeProgrammeCaptureSupervisorClaimValidationRecordV1(replayed)
    !== input.serializedValidation) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_VALIDATION_REPLAY_MISMATCH');
  }
  return replayed;
}

function verifyFromInput(input: Record<string, unknown>) {
  return verifyProgrammeCaptureSupervisorClaimAcknowledgementV1({
    claimAuthority: input.claimAuthority as ProgrammeCaptureRunClaimAuthorityInputV1,
    serializedEnvelope: input.serializedEnvelope as string,
    trustedPublicKeySpkiDer: input.trustedPublicKeySpkiDer as Uint8Array,
    expectedAuthorityKeyFingerprint: input.expectedAuthorityKeyFingerprint as string,
    expectedSupervisorId: input.expectedSupervisorId as string,
    expectedLogId: input.expectedLogId as string,
    expectedKeyEpoch: input.expectedKeyEpoch as number,
    expectedLogSequence: input.expectedLogSequence as number,
    expectedPreviousCheckpointDigest: input.expectedPreviousCheckpointDigest as string,
  });
}

function parseCanonicalBlob<T>(serialized: string, maximumBytes: number, label: string,
  parse: (value: unknown) => T, serialize: (value: unknown) => string): T {
  if (typeof serialized !== 'string' || Buffer.byteLength(serialized, 'utf8') > maximumBytes
    || decodeCanonicalUtf8(serialized) !== serialized) {
    throw new TypeError(`supervisor ${label} must be bounded canonical UTF-8 JSON`);
  }
  const record = parse(parseJsonWithoutDuplicateKeys(serialized, `supervisor ${label}`));
  if (serialize(record) !== serialized) {
    throw new Error(`HARNESS_CAPTURE_SUPERVISOR_${label.toUpperCase()}_CANONICAL_REQUIRED`);
  }
  return record;
}

function parseDigest(value: unknown): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || /^0+$/.test(value)) {
    throw new TypeError('supervisor validation digest must be a non-zero SHA-256 digest');
  }
  return value;
}

function decodeCanonicalUtf8(value: string): string {
  try { return new TextDecoder('utf-8', { fatal: true }).decode(Buffer.from(value, 'utf8')); }
  catch { return ''; }
}
