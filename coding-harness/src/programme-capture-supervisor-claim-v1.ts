// SPDX-License-Identifier: MIT
import { canonical } from '@metaharness/harness';
import { createHash } from 'node:crypto';
import {
  DEVELOPMENT_AUTHORITY, SHA256_PATTERN, asClosedRecord, asInteger,
  assertExactKeys, deepFreeze,
} from './contracts.js';
import { parseTaskOpaqueId } from './acceptance-task-v3.js';
import { readProgrammeCaptureRunClaimV1,
  type ProgrammeCaptureRunClaimAuthorityInputV1 } from './programme-capture-claim-io-v1.js';
import {
  parseProgrammeCaptureRunClaimV1, programmeCaptureRunClaimKeyDigestV1,
  serializeProgrammeCaptureRunClaimV1,
  type ProgrammeCaptureRunClaimV1,
} from './programme-capture-claim-record-v1.js';
import { digestValue } from './receipts.js';
import {
  parseProgrammeCaptureSupervisorEd25519SignatureV1,
  verifyProgrammeCaptureSupervisorEd25519SignatureV1,
} from './programme-capture-supervisor-crypto-v1.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';
export const PROGRAMME_CAPTURE_SUPERVISOR_CLAIM_MAX_BYTES_V1 = 16_384;
export const PROGRAMME_CAPTURE_SUPERVISOR_ACK_DIGEST_DOMAIN_V1 =
  'semantic-fabric/programme-capture/supervisor-claim-acknowledgement-digest-v1';
export const PROGRAMME_CAPTURE_SUPERVISOR_ACK_SIGNING_DOMAIN_V1 =
  'semantic-fabric/programme-capture/supervisor-claim-acknowledgement-signing-v1';
const REQUEST_DIGEST_DOMAIN = 'semantic-fabric/programme-capture/'
  + 'supervisor-claim-registration-request-digest-v1';
export const PROGRAMME_CAPTURE_SUPERVISOR_VALIDATION_DIGEST_DOMAIN_V1 =
  'semantic-fabric/programme-capture/'
  + 'supervisor-claim-validation-digest-v1';
interface NonAuthorityFieldsV1 {
  readonly externalAppendOnlyWitness: false; readonly appendOnlyPersistenceVerified: false;
  readonly rollbackResistance: 'not-proven'; readonly supervisorAdministration: 'not-attested';
  readonly hostAdmission: 'not-evaluated'; readonly runnerLeaseAcquired: false;
  readonly stateTransitionAuthorized: false; readonly attemptStartAuthorized: false;
  readonly captureAuthorized: false;
}
export interface ProgrammeCaptureSupervisorClaimRequestV1 extends NonAuthorityFieldsV1 {
  readonly schemaVersion: 1; readonly transactionKind: 'programme-capture-v1';
  readonly requestKind: 'supervisor-claim-registration-request-v1';
  readonly authority: typeof DEVELOPMENT_AUTHORITY; readonly claim: ProgrammeCaptureRunClaimV1;
  readonly requestDigest: string;
}
export interface ProgrammeCaptureSupervisorClaimAcknowledgementV1
  extends NonAuthorityFieldsV1 {
  readonly schemaVersion: 1; readonly transactionKind: 'programme-capture-v1';
  readonly recordKind: 'supervisor-claim-registration-acknowledgement-v1';
  readonly authority: typeof DEVELOPMENT_AUTHORITY; readonly runId: string;
  readonly projectAuthorityDigest: string; readonly claimKeyDigest: string;
  readonly claimDigest: string; readonly requestDigest: string;
  readonly supervisor: Readonly<{
    supervisorId: string; logId: string; keyEpoch: number; authorityKeyFingerprint: string;
  }>;
  readonly event: Readonly<{
    kind: 'claim-registered-v1'; runSequence: 0; logSequence: number;
    previousCheckpointDigest: string;
  }>;
  readonly verificationScope: 'signature-and-claim-binding-only'; readonly acknowledgementDigest: string;
}
export interface ProgrammeCaptureSupervisorClaimEnvelopeV1 {
  readonly schemaVersion: 1; readonly transactionKind: 'programme-capture-v1';
  readonly envelopeKind: 'supervisor-claim-acknowledgement-envelope-v1';
  readonly acknowledgement: ProgrammeCaptureSupervisorClaimAcknowledgementV1;
  readonly signature: Readonly<{ algorithm: 'ed25519'; valueBase64Url: string }>;
}
export interface ProgrammeCaptureSupervisorClaimValidationV1 extends NonAuthorityFieldsV1 {
  readonly schemaVersion: 1; readonly transactionKind: 'programme-capture-v1';
  readonly evidenceKind: 'non-authorizing-supervisor-claim-validation-v1';
  readonly authority: typeof DEVELOPMENT_AUTHORITY; readonly runId: string;
  readonly projectAuthorityDigest: string; readonly claimKeyDigest: string;
  readonly claimDigest: string;
  readonly requestDigest: string; readonly acknowledgementDigest: string;
  readonly serializedEnvelopeDigest: string;
  readonly supervisor: ProgrammeCaptureSupervisorClaimAcknowledgementV1['supervisor'];
  readonly event: ProgrammeCaptureSupervisorClaimAcknowledgementV1['event'];
  readonly verificationScope: 'signature-and-rooted-claim-binding-only';
  readonly signatureVerified: true; readonly suppliedCheckpointReferenceMatched: true;
  readonly validationDigest: string;
}
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
export async function deriveProgrammeCaptureSupervisorClaimRequestV1(value: Readonly<{
  claimAuthority: ProgrammeCaptureRunClaimAuthorityInputV1;
}>): Promise<ProgrammeCaptureSupervisorClaimRequestV1> {
  const input = asClosedRecord(value, 'programme capture supervisor request input');
  assertExactKeys(input, ['claimAuthority'], 'programme capture supervisor request input');
  const authority = snapshotClaimAuthority(input.claimAuthority);
  const before = await readRootedClaim(authority);
  const request = requestFromClaim(before);
  const after = await readRootedClaim(authority);
  assertSameClaim(before, after);
  return request;
}
export function createProgrammeCaptureSupervisorClaimAcknowledgementV1(value: Readonly<{
  request: ProgrammeCaptureSupervisorClaimRequestV1;
  supervisorId: string;
  logId: string;
  keyEpoch: number;
  authorityKeyFingerprint: string;
  logSequence: number;
  previousCheckpointDigest: string;
}>): ProgrammeCaptureSupervisorClaimAcknowledgementV1 {
  const input = asClosedRecord(value, 'programme capture supervisor acknowledgement input');
  assertExactKeys(input, [
    'request', 'supervisorId', 'logId', 'keyEpoch', 'authorityKeyFingerprint',
    'logSequence', 'previousCheckpointDigest',
  ], 'programme capture supervisor acknowledgement input');
  const request = parseProgrammeCaptureSupervisorClaimRequestV1(input.request);
  const body = {
    schemaVersion: 1 as const,
    transactionKind: 'programme-capture-v1' as const,
    recordKind: 'supervisor-claim-registration-acknowledgement-v1' as const,
    authority: DEVELOPMENT_AUTHORITY,
    runId: request.claim.runId,
    projectAuthorityDigest: request.claim.authority.projectAuthorityDigest,
    claimKeyDigest: request.claim.claimKeyDigest,
    claimDigest: request.claim.claimDigest,
    requestDigest: request.requestDigest,
    supervisor: {
      supervisorId: parseTaskOpaqueId(input.supervisorId, 'supervisor acknowledgement ID'),
      logId: parseTaskOpaqueId(input.logId, 'supervisor acknowledgement log ID'),
      keyEpoch: asInteger(input.keyEpoch, 'supervisor acknowledgement key epoch', 1),
      authorityKeyFingerprint: parseDigest(
        input.authorityKeyFingerprint, 'supervisor authority key fingerprint',
      ),
    },
    event: {
      kind: 'claim-registered-v1' as const,
      runSequence: 0 as const,
      logSequence: asInteger(input.logSequence, 'supervisor log sequence', 1),
      previousCheckpointDigest: parseDigest(
        input.previousCheckpointDigest, 'supervisor previous checkpoint',
      ),
    },
    verificationScope: 'signature-and-claim-binding-only' as const,
    ...NON_AUTHORITY,
  };
  return parseProgrammeCaptureSupervisorClaimAcknowledgementV1({
    ...body,
    acknowledgementDigest: acknowledgementDigest(body),
  });
}
export function parseProgrammeCaptureSupervisorClaimRequestV1(
  value: unknown,
): ProgrammeCaptureSupervisorClaimRequestV1 {
  const input = asClosedRecord(value, 'programme capture supervisor claim request');
  assertExactKeys(input, [
    'schemaVersion', 'transactionKind', 'requestKind', 'authority', 'claim',
    ...NON_AUTHORITY_KEYS, 'requestDigest',
  ], 'programme capture supervisor claim request');
  assertIdentity(input, 'requestKind', 'supervisor-claim-registration-request-v1');
  assertNonAuthority(input);
  const body = {
    schemaVersion: 1 as const,
    transactionKind: 'programme-capture-v1' as const,
    requestKind: 'supervisor-claim-registration-request-v1' as const,
    authority: DEVELOPMENT_AUTHORITY,
    claim: parseProgrammeCaptureRunClaimV1(input.claim),
    ...NON_AUTHORITY,
  };
  const requestDigest = parseDigest(input.requestDigest, 'supervisor request digest');
  if (requestDigest !== digestValue({ domain: REQUEST_DIGEST_DOMAIN, request: body })) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_REQUEST_DIGEST_MISMATCH');
  }
  return deepFreeze({ ...body, requestDigest });
}
export function parseProgrammeCaptureSupervisorClaimAcknowledgementV1(
  value: unknown,
): ProgrammeCaptureSupervisorClaimAcknowledgementV1 {
  const input = asClosedRecord(value, 'programme capture supervisor acknowledgement');
  assertExactKeys(input, [
    'schemaVersion', 'transactionKind', 'recordKind', 'authority', 'runId',
    'projectAuthorityDigest', 'claimKeyDigest', 'claimDigest', 'requestDigest',
    'supervisor', 'event', 'verificationScope', ...NON_AUTHORITY_KEYS,
    'acknowledgementDigest',
  ], 'programme capture supervisor acknowledgement');
  assertIdentity(
    input, 'recordKind', 'supervisor-claim-registration-acknowledgement-v1',
  );
  if (input.verificationScope !== 'signature-and-claim-binding-only') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_ACK_SCOPE_INVALID');
  }
  assertNonAuthority(input);
  const supervisorInput = asClosedRecord(input.supervisor, 'supervisor acknowledgement authority');
  assertExactKeys(supervisorInput, [
    'supervisorId', 'logId', 'keyEpoch', 'authorityKeyFingerprint',
  ], 'supervisor acknowledgement authority');
  const eventInput = asClosedRecord(input.event, 'supervisor acknowledgement event');
  assertExactKeys(eventInput, [
    'kind', 'runSequence', 'logSequence', 'previousCheckpointDigest',
  ], 'supervisor acknowledgement event');
  if (eventInput.kind !== 'claim-registered-v1' || eventInput.runSequence !== 0) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_ACK_EVENT_INVALID');
  }
  const body = {
    schemaVersion: 1 as const,
    transactionKind: 'programme-capture-v1' as const,
    recordKind: 'supervisor-claim-registration-acknowledgement-v1' as const,
    authority: DEVELOPMENT_AUTHORITY,
    runId: parseTaskOpaqueId(input.runId, 'supervisor acknowledgement run ID'),
    projectAuthorityDigest: parseDigest(input.projectAuthorityDigest, 'project authority'),
    claimKeyDigest: parseDigest(input.claimKeyDigest, 'supervisor acknowledgement claim key'),
    claimDigest: parseDigest(input.claimDigest, 'supervisor acknowledgement claim'),
    requestDigest: parseDigest(input.requestDigest, 'supervisor acknowledgement request'),
    supervisor: {
      supervisorId: parseTaskOpaqueId(supervisorInput.supervisorId, 'supervisor ID'),
      logId: parseTaskOpaqueId(supervisorInput.logId, 'supervisor log ID'),
      keyEpoch: asInteger(supervisorInput.keyEpoch, 'supervisor key epoch', 1),
      authorityKeyFingerprint: parseDigest(
        supervisorInput.authorityKeyFingerprint, 'supervisor key fingerprint',
      ),
    },
    event: {
      kind: 'claim-registered-v1' as const,
      runSequence: 0 as const,
      logSequence: asInteger(eventInput.logSequence, 'supervisor log sequence', 1),
      previousCheckpointDigest: parseDigest(
        eventInput.previousCheckpointDigest, 'supervisor previous checkpoint',
      ),
    },
    verificationScope: 'signature-and-claim-binding-only' as const,
    ...NON_AUTHORITY,
  };
  const acknowledgementDigestValue = parseDigest(
    input.acknowledgementDigest, 'supervisor acknowledgement digest',
  );
  if (acknowledgementDigestValue !== acknowledgementDigest(body)) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_ACK_DIGEST_MISMATCH');
  }
  if (body.claimKeyDigest !== programmeCaptureRunClaimKeyDigestV1({
    projectAuthorityDigest: body.projectAuthorityDigest, runId: body.runId,
  })) throw new Error('HARNESS_CAPTURE_SUPERVISOR_ACK_CLAIM_KEY_MISMATCH');
  return deepFreeze({ ...body, acknowledgementDigest: acknowledgementDigestValue });
}
export function parseProgrammeCaptureSupervisorClaimEnvelopeV1(
  value: unknown,
): ProgrammeCaptureSupervisorClaimEnvelopeV1 {
  const input = asClosedRecord(value, 'programme capture supervisor claim envelope');
  assertExactKeys(input, [
    'schemaVersion', 'transactionKind', 'envelopeKind', 'acknowledgement', 'signature',
  ], 'programme capture supervisor claim envelope');
  assertIdentity(input, 'envelopeKind', 'supervisor-claim-acknowledgement-envelope-v1', false);
  const signatureInput = asClosedRecord(input.signature, 'supervisor acknowledgement signature');
  assertExactKeys(
    signatureInput, ['algorithm', 'valueBase64Url'], 'supervisor acknowledgement signature',
  );
  if (signatureInput.algorithm !== 'ed25519') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_SIGNATURE_ALGORITHM_INVALID');
  }
  const valueBase64Url = parseProgrammeCaptureSupervisorEd25519SignatureV1(
    signatureInput.valueBase64Url,
  );
  return deepFreeze({
    schemaVersion: 1,
    transactionKind: 'programme-capture-v1',
    envelopeKind: 'supervisor-claim-acknowledgement-envelope-v1',
    acknowledgement: parseProgrammeCaptureSupervisorClaimAcknowledgementV1(
      input.acknowledgement,
    ),
    signature: { algorithm: 'ed25519', valueBase64Url },
  });
}
export function serializeProgrammeCaptureSupervisorClaimEnvelopeV1(value: unknown): string {
  return `${JSON.stringify(parseProgrammeCaptureSupervisorClaimEnvelopeV1(value), null, 2)}\n`;
}
export function parseProgrammeCaptureSupervisorClaimEnvelopeBlobV1(
  serialized: string,
): ProgrammeCaptureSupervisorClaimEnvelopeV1 {
  if (typeof serialized !== 'string'
    || Buffer.byteLength(serialized, 'utf8') > PROGRAMME_CAPTURE_SUPERVISOR_CLAIM_MAX_BYTES_V1
    || decodeCanonicalUtf8(serialized) !== serialized) {
    throw new TypeError('supervisor claim envelope must be bounded canonical UTF-8 JSON');
  }
  const envelope = parseProgrammeCaptureSupervisorClaimEnvelopeV1(
    parseJsonWithoutDuplicateKeys(serialized, 'programme capture supervisor claim envelope'),
  );
  if (serializeProgrammeCaptureSupervisorClaimEnvelopeV1(envelope) !== serialized) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_ENVELOPE_CANONICAL_REQUIRED');
  }
  return envelope;
}
export function programmeCaptureSupervisorClaimSigningPayloadV1(value: unknown): Buffer {
  const acknowledgement = parseProgrammeCaptureSupervisorClaimAcknowledgementV1(value);
  return Buffer.from(canonical({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_ACK_SIGNING_DOMAIN_V1,
    acknowledgement,
  }), 'utf8');
}
export async function verifyProgrammeCaptureSupervisorClaimAcknowledgementV1(
  value: Readonly<{
    claimAuthority: ProgrammeCaptureRunClaimAuthorityInputV1;
    serializedEnvelope: string;
    trustedPublicKeySpkiDer: Uint8Array;
    expectedAuthorityKeyFingerprint: string;
    expectedSupervisorId: string;
    expectedLogId: string;
    expectedKeyEpoch: number;
    expectedLogSequence: number;
    expectedPreviousCheckpointDigest: string;
  }>,
): Promise<ProgrammeCaptureSupervisorClaimValidationV1> {
  const input = asClosedRecord(value, 'programme capture supervisor verification input');
  assertExactKeys(input, [
    'claimAuthority', 'serializedEnvelope', 'trustedPublicKeySpkiDer',
    'expectedAuthorityKeyFingerprint', 'expectedSupervisorId', 'expectedLogId',
    'expectedKeyEpoch', 'expectedLogSequence', 'expectedPreviousCheckpointDigest',
  ], 'programme capture supervisor verification input');
  const authority = snapshotClaimAuthority(input.claimAuthority);
  const envelope = parseProgrammeCaptureSupervisorClaimEnvelopeBlobV1(
    input.serializedEnvelope as string,
  );
  const acknowledgement = envelope.acknowledgement;
  const expectations = parseExpectations(input);
  assertAcknowledgementReferences(acknowledgement, expectations);
  verifyProgrammeCaptureSupervisorEd25519SignatureV1({
    payload: programmeCaptureSupervisorClaimSigningPayloadV1(acknowledgement),
    signatureBase64Url: envelope.signature.valueBase64Url,
    trustedPublicKeySpkiDer: input.trustedPublicKeySpkiDer,
    expectedAuthorityKeyFingerprint: expectations.keyFingerprint,
  });
  const before = await readRootedClaim(authority);
  const request = requestFromClaim(before);
  assertAcknowledgementBindings(request, acknowledgement);
  const after = await readRootedClaim(authority);
  assertSameClaim(before, after);
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
    serializedEnvelopeDigest: createHash('sha256').update(
      input.serializedEnvelope as string, 'utf8',
    ).digest('hex'),
    supervisor: acknowledgement.supervisor,
    event: acknowledgement.event,
    verificationScope: 'signature-and-rooted-claim-binding-only' as const,
    signatureVerified: true as const,
    suppliedCheckpointReferenceMatched: true as const,
    ...NON_AUTHORITY,
  };
  return deepFreeze({
    ...body,
    validationDigest: digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_VALIDATION_DIGEST_DOMAIN_V1, validation: body,
    }),
  });
}
function requestFromClaim(value: ProgrammeCaptureRunClaimV1): ProgrammeCaptureSupervisorClaimRequestV1 {
  const body = {
    schemaVersion: 1 as const,
    transactionKind: 'programme-capture-v1' as const,
    requestKind: 'supervisor-claim-registration-request-v1' as const,
    authority: DEVELOPMENT_AUTHORITY,
    claim: parseProgrammeCaptureRunClaimV1(value),
    ...NON_AUTHORITY,
  };
  return parseProgrammeCaptureSupervisorClaimRequestV1({
    ...body, requestDigest: digestValue({ domain: REQUEST_DIGEST_DOMAIN, request: body }),
  });
}
async function readRootedClaim(
  authority: ProgrammeCaptureRunClaimAuthorityInputV1,
): Promise<ProgrammeCaptureRunClaimV1> {
  return (await readProgrammeCaptureRunClaimV1(authority)).record;
}
function snapshotClaimAuthority(value: unknown): ProgrammeCaptureRunClaimAuthorityInputV1 {
  const input = asClosedRecord(value, 'programme capture supervisor claim authority');
  assertExactKeys(input, [
    'authorityRoot', 'projectAuthorityDigest', 'runId', 'controllerStore',
    'controllerCommit', 'taskPath', 'expectedRunnerIdentityDigest',
  ], 'programme capture supervisor claim authority');
  return Object.freeze({
    authorityRoot: input.authorityRoot as string,
    projectAuthorityDigest: input.projectAuthorityDigest as string,
    runId: input.runId as string, controllerStore: input.controllerStore as string,
    controllerCommit: input.controllerCommit as string, taskPath: input.taskPath as string,
    expectedRunnerIdentityDigest: input.expectedRunnerIdentityDigest as string,
  });
}
function assertSameClaim(left: ProgrammeCaptureRunClaimV1, right: ProgrammeCaptureRunClaimV1): void {
  if (left.claimDigest !== right.claimDigest
    || serializeProgrammeCaptureRunClaimV1(left) !== serializeProgrammeCaptureRunClaimV1(right)) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_ROOTED_CLAIM_CHANGED');
  }
}
function assertAcknowledgementBindings(
  request: ProgrammeCaptureSupervisorClaimRequestV1,
  value: ProgrammeCaptureSupervisorClaimAcknowledgementV1,
): void {
  const claim = request.claim;
  if (value.requestDigest !== request.requestDigest || value.runId !== claim.runId
    || value.projectAuthorityDigest !== claim.authority.projectAuthorityDigest
    || value.claimKeyDigest !== claim.claimKeyDigest || value.claimDigest !== claim.claimDigest) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_ACK_AUTHORITY_MISMATCH');
  }
}
function assertAcknowledgementReferences(value: ProgrammeCaptureSupervisorClaimAcknowledgementV1,
  expected: ReturnType<typeof parseExpectations>): void {
  if (value.supervisor.authorityKeyFingerprint !== expected.keyFingerprint
    || value.supervisor.supervisorId !== expected.supervisorId
    || value.supervisor.logId !== expected.logId || value.supervisor.keyEpoch !== expected.keyEpoch
    || value.event.logSequence !== expected.logSequence
    || value.event.previousCheckpointDigest !== expected.previousCheckpointDigest) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_ACK_AUTHORITY_MISMATCH');
  }
}
function parseExpectations(input: Record<string, unknown>) {
  return Object.freeze({
    keyFingerprint: parseDigest(
      input.expectedAuthorityKeyFingerprint, 'expected supervisor key fingerprint',
    ),
    supervisorId: parseTaskOpaqueId(input.expectedSupervisorId, 'expected supervisor ID'),
    logId: parseTaskOpaqueId(input.expectedLogId, 'expected supervisor log ID'),
    keyEpoch: asInteger(input.expectedKeyEpoch, 'expected supervisor key epoch', 1),
    logSequence: asInteger(input.expectedLogSequence, 'expected supervisor log sequence', 1),
    previousCheckpointDigest: parseDigest(
      input.expectedPreviousCheckpointDigest, 'expected supervisor previous checkpoint',
    ),
  });
}
function assertIdentity(input: Record<string, unknown>, kindName: string, expectedKind: string,
  requireAuthority = true): void {
  if (input.schemaVersion !== 1 || input.transactionKind !== 'programme-capture-v1'
    || input[kindName] !== expectedKind
    || (requireAuthority && input.authority !== DEVELOPMENT_AUTHORITY)) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_IDENTITY_INVALID');
  }
}
function assertNonAuthority(input: Record<string, unknown>): void {
  if (NON_AUTHORITY_KEYS.some((key) => input[key] !== NON_AUTHORITY[key as keyof typeof NON_AUTHORITY])) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_ESCALATION');
  }
}
function acknowledgementDigest(value: unknown): string {
  return digestValue({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_ACK_DIGEST_DOMAIN_V1,
    acknowledgement: value,
  });
}
function parseDigest(value: unknown, label: string): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || /^0+$/.test(value)) {
    throw new TypeError(`${label} must be a non-zero lowercase SHA-256 digest`);
  }
  return value;
}

function decodeCanonicalUtf8(value: string): string {
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(Buffer.from(value, 'utf8'));
  } catch {
    return '';
  }
}
