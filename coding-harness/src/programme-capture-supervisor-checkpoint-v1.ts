// SPDX-License-Identifier: MIT

import { canonical } from '@metaharness/harness';
import { isProxy } from 'node:util/types';
import {
  DEVELOPMENT_AUTHORITY,
  SHA256_PATTERN,
  asClosedRecord,
  asInteger,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import { parseTaskOpaqueId } from './acceptance-task-v3.js';
import {
  parseProgrammeCaptureSupervisorEd25519SignatureV1,
  verifyProgrammeCaptureSupervisorEd25519SignatureV1,
} from './programme-capture-supervisor-crypto-v1.js';
import { digestValue } from './receipts.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export const PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_MAX_BYTES_V1 = 16_384;
export const PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_DIGEST_DOMAIN_V1 =
  'semantic-fabric/programme-capture/supervisor-log-checkpoint-digest-v1';
export const PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_SIGNING_DOMAIN_V1 =
  'semantic-fabric/programme-capture/supervisor-log-checkpoint-signing-v1';
const UINT64_DECIMAL_PATTERN = /^(?:0|[1-9][0-9]{0,19})$/;
const MAX_UINT64 = 18_446_744_073_709_551_615n;
const RFC9162_EMPTY_ROOT =
  'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855';

interface CheckpointNonAuthorityV1 {
  readonly externalAppendOnlyWitness: false;
  readonly appendOnlyPersistenceVerified: false;
  readonly rollbackResistance: 'not-proven';
  readonly forkResistance: 'not-proven';
  readonly globalOrderAuthority: 'not-proven';
  readonly supervisorAdministration: 'not-attested';
  readonly hostAdmission: 'not-evaluated';
  readonly runnerLeaseAcquired: false;
  readonly stateTransitionAuthorized: false;
  readonly attemptStartAuthorized: false;
  readonly captureAuthorized: false;
}

export interface ProgrammeCaptureSupervisorCheckpointV1 extends CheckpointNonAuthorityV1 {
  readonly schemaVersion: 1;
  readonly transactionKind: 'programme-capture-v1';
  readonly recordKind: 'supervisor-log-checkpoint-v1';
  readonly authority: typeof DEVELOPMENT_AUTHORITY;
  readonly supervisor: Readonly<{
    supervisorId: string;
    logId: string;
    keyEpoch: number;
    authorityKeyFingerprint: string;
  }>;
  readonly tree: Readonly<{ treeSize: string; rootDigest: string }>;
  readonly verificationScope: 'signed-log-checkpoint-only';
  readonly checkpointDigest: string;
}

export interface ProgrammeCaptureSupervisorCheckpointEnvelopeV1 {
  readonly schemaVersion: 1;
  readonly transactionKind: 'programme-capture-v1';
  readonly envelopeKind: 'supervisor-log-checkpoint-envelope-v1';
  readonly checkpoint: ProgrammeCaptureSupervisorCheckpointV1;
  readonly signature: Readonly<{ algorithm: 'ed25519'; valueBase64Url: string }>;
}

const NON_AUTHORITY = Object.freeze({
  externalAppendOnlyWitness: false as const,
  appendOnlyPersistenceVerified: false as const,
  rollbackResistance: 'not-proven' as const,
  forkResistance: 'not-proven' as const,
  globalOrderAuthority: 'not-proven' as const,
  supervisorAdministration: 'not-attested' as const,
  hostAdmission: 'not-evaluated' as const,
  runnerLeaseAcquired: false as const,
  stateTransitionAuthorized: false as const,
  attemptStartAuthorized: false as const,
  captureAuthorized: false as const,
});
const NON_AUTHORITY_KEYS = Object.keys(NON_AUTHORITY);

export function parseProgrammeCaptureSupervisorCheckpointV1(
  value: unknown,
): ProgrammeCaptureSupervisorCheckpointV1 {
  const input = closedRecord(value, 'programme capture supervisor checkpoint');
  assertExactKeys(input, [
    'schemaVersion', 'transactionKind', 'recordKind', 'authority', 'supervisor',
    'tree', 'verificationScope', ...NON_AUTHORITY_KEYS, 'checkpointDigest',
  ], 'programme capture supervisor checkpoint');
  assertIdentity(input, 'recordKind', 'supervisor-log-checkpoint-v1', true);
  if (input.verificationScope !== 'signed-log-checkpoint-only') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_CHECKPOINT_SCOPE_INVALID');
  }
  assertNonAuthority(input);
  const supervisor = parseSupervisor(input.supervisor);
  const treeInput = closedRecord(input.tree, 'supervisor checkpoint tree');
  assertExactKeys(treeInput, ['treeSize', 'rootDigest'], 'supervisor checkpoint tree');
  const treeSize = parseUint64(treeInput.treeSize, 'supervisor checkpoint tree size');
  const rootDigest = parseDigest(treeInput.rootDigest, 'supervisor checkpoint root');
  if (treeSize === '0' && rootDigest !== RFC9162_EMPTY_ROOT) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_CHECKPOINT_EMPTY_ROOT_INVALID');
  }
  const body = {
    schemaVersion: 1 as const,
    transactionKind: 'programme-capture-v1' as const,
    recordKind: 'supervisor-log-checkpoint-v1' as const,
    authority: DEVELOPMENT_AUTHORITY,
    supervisor,
    tree: { treeSize, rootDigest },
    verificationScope: 'signed-log-checkpoint-only' as const,
    ...NON_AUTHORITY,
  };
  const checkpointDigest = parseDigest(input.checkpointDigest, 'supervisor checkpoint digest');
  if (checkpointDigest !== digestValue({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_DIGEST_DOMAIN_V1,
    checkpoint: body,
  })) throw new Error('HARNESS_CAPTURE_SUPERVISOR_CHECKPOINT_DIGEST_MISMATCH');
  return deepFreeze({ ...body, checkpointDigest });
}

export function serializeProgrammeCaptureSupervisorCheckpointV1(value: unknown): string {
  return `${JSON.stringify(parseProgrammeCaptureSupervisorCheckpointV1(value), null, 2)}\n`;
}

export function parseProgrammeCaptureSupervisorCheckpointBlobV1(
  serialized: string,
): ProgrammeCaptureSupervisorCheckpointV1 {
  return parseCanonicalBlob(
    serialized, 'checkpoint', parseProgrammeCaptureSupervisorCheckpointV1,
    serializeProgrammeCaptureSupervisorCheckpointV1,
  );
}

export function parseProgrammeCaptureSupervisorCheckpointEnvelopeV1(
  value: unknown,
): ProgrammeCaptureSupervisorCheckpointEnvelopeV1 {
  const input = closedRecord(value, 'programme capture supervisor checkpoint envelope');
  assertExactKeys(input, [
    'schemaVersion', 'transactionKind', 'envelopeKind', 'checkpoint', 'signature',
  ], 'programme capture supervisor checkpoint envelope');
  assertIdentity(input, 'envelopeKind', 'supervisor-log-checkpoint-envelope-v1', false);
  const signatureInput = closedRecord(input.signature, 'supervisor checkpoint signature');
  assertExactKeys(signatureInput, ['algorithm', 'valueBase64Url'], 'supervisor checkpoint signature');
  if (signatureInput.algorithm !== 'ed25519') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_SIGNATURE_ALGORITHM_INVALID');
  }
  return deepFreeze({
    schemaVersion: 1,
    transactionKind: 'programme-capture-v1',
    envelopeKind: 'supervisor-log-checkpoint-envelope-v1',
    checkpoint: parseProgrammeCaptureSupervisorCheckpointV1(input.checkpoint),
    signature: {
      algorithm: 'ed25519',
      valueBase64Url: parseProgrammeCaptureSupervisorEd25519SignatureV1(
        signatureInput.valueBase64Url,
      ),
    },
  });
}

export function serializeProgrammeCaptureSupervisorCheckpointEnvelopeV1(value: unknown): string {
  return `${JSON.stringify(parseProgrammeCaptureSupervisorCheckpointEnvelopeV1(value), null, 2)}\n`;
}

export function parseProgrammeCaptureSupervisorCheckpointEnvelopeBlobV1(
  serialized: string,
): ProgrammeCaptureSupervisorCheckpointEnvelopeV1 {
  return parseCanonicalBlob(
    serialized, 'checkpoint envelope', parseProgrammeCaptureSupervisorCheckpointEnvelopeV1,
    serializeProgrammeCaptureSupervisorCheckpointEnvelopeV1,
  );
}

export function programmeCaptureSupervisorCheckpointSigningPayloadV1(value: unknown): Buffer {
  const checkpoint = parseProgrammeCaptureSupervisorCheckpointV1(value);
  return Buffer.from(canonical({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_SIGNING_DOMAIN_V1,
    checkpoint,
  }), 'utf8');
}

export function verifyProgrammeCaptureSupervisorCheckpointEnvelopeV1(value: unknown):
ProgrammeCaptureSupervisorCheckpointV1 {
  const input = closedRecord(value, 'programme capture supervisor checkpoint verification input');
  assertExactKeys(input, [
    'serializedEnvelope', 'trustedPublicKeySpkiDer', 'expectedAuthorityKeyFingerprint',
    'expectedSupervisorId', 'expectedLogId', 'expectedKeyEpoch',
  ], 'programme capture supervisor checkpoint verification input');
  const envelope = parseProgrammeCaptureSupervisorCheckpointEnvelopeBlobV1(
    input.serializedEnvelope as string,
  );
  const expected = parseExpectedSupervisor(input);
  assertSupervisorMatches(envelope.checkpoint.supervisor, expected);
  verifyProgrammeCaptureSupervisorEd25519SignatureV1({
    payload: programmeCaptureSupervisorCheckpointSigningPayloadV1(envelope.checkpoint),
    signatureBase64Url: envelope.signature.valueBase64Url,
    trustedPublicKeySpkiDer: input.trustedPublicKeySpkiDer,
    expectedAuthorityKeyFingerprint: expected.authorityKeyFingerprint,
  });
  return envelope.checkpoint;
}

function parseSupervisor(value: unknown): ProgrammeCaptureSupervisorCheckpointV1['supervisor'] {
  const input = closedRecord(value, 'supervisor checkpoint authority');
  assertExactKeys(input, [
    'supervisorId', 'logId', 'keyEpoch', 'authorityKeyFingerprint',
  ], 'supervisor checkpoint authority');
  return Object.freeze({
    supervisorId: parseTaskOpaqueId(input.supervisorId, 'supervisor checkpoint ID'),
    logId: parseTaskOpaqueId(input.logId, 'supervisor checkpoint log ID'),
    keyEpoch: asInteger(input.keyEpoch, 'supervisor checkpoint key epoch', 1),
    authorityKeyFingerprint: parseDigest(
      input.authorityKeyFingerprint, 'supervisor checkpoint key fingerprint',
    ),
  });
}

function parseExpectedSupervisor(input: Record<string, unknown>) {
  return Object.freeze({
    authorityKeyFingerprint: parseDigest(
      input.expectedAuthorityKeyFingerprint, 'expected checkpoint key fingerprint',
    ),
    supervisorId: parseTaskOpaqueId(input.expectedSupervisorId, 'expected checkpoint supervisor ID'),
    logId: parseTaskOpaqueId(input.expectedLogId, 'expected checkpoint log ID'),
    keyEpoch: asInteger(input.expectedKeyEpoch, 'expected checkpoint key epoch', 1),
  });
}

function assertSupervisorMatches(
  actual: ProgrammeCaptureSupervisorCheckpointV1['supervisor'],
  expected: ReturnType<typeof parseExpectedSupervisor>,
): void {
  if (actual.supervisorId !== expected.supervisorId || actual.logId !== expected.logId
    || actual.keyEpoch !== expected.keyEpoch
    || actual.authorityKeyFingerprint !== expected.authorityKeyFingerprint) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_CHECKPOINT_AUTHORITY_MISMATCH');
  }
}

function parseCanonicalBlob<T>(serialized: string, label: string,
  parse: (value: unknown) => T, serialize: (value: unknown) => string): T {
  if (typeof serialized !== 'string'
    || Buffer.byteLength(serialized, 'utf8') > PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_MAX_BYTES_V1
    || decodeCanonicalUtf8(serialized) !== serialized) {
    throw new TypeError(`supervisor ${label} must be bounded canonical UTF-8 JSON`);
  }
  const parsed = parse(parseJsonWithoutDuplicateKeys(serialized, `supervisor ${label}`));
  if (serialize(parsed) !== serialized) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_CHECKPOINT_CANONICAL_REQUIRED');
  }
  return parsed;
}

function parseUint64(value: unknown, label: string): string {
  if (typeof value !== 'string' || !UINT64_DECIMAL_PATTERN.test(value)
    || BigInt(value) > MAX_UINT64) {
    throw new TypeError(`${label} must be a canonical uint64 decimal string`);
  }
  return value;
}

function parseDigest(value: unknown, label: string): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || /^0+$/.test(value)) {
    throw new TypeError(`${label} must be a non-zero lowercase SHA-256 digest`);
  }
  return value;
}

function assertIdentity(input: Record<string, unknown>, kind: string, expected: string,
  requireAuthority: boolean): void {
  if (input.schemaVersion !== 1 || input.transactionKind !== 'programme-capture-v1'
    || input[kind] !== expected
    || (requireAuthority && input.authority !== DEVELOPMENT_AUTHORITY)) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_CHECKPOINT_IDENTITY_INVALID');
  }
}

function assertNonAuthority(input: Record<string, unknown>): void {
  if (NON_AUTHORITY_KEYS.some((key) => input[key] !== NON_AUTHORITY[key as keyof typeof NON_AUTHORITY])) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_CHECKPOINT_AUTHORITY_ESCALATION');
  }
}

function closedRecord(value: unknown, label: string): Record<string, unknown> {
  if (isProxy(value)) throw new TypeError(`${label} must not be a Proxy`);
  return asClosedRecord(value, label);
}

function decodeCanonicalUtf8(value: string): string {
  try { return new TextDecoder('utf-8', { fatal: true }).decode(Buffer.from(value, 'utf8')); }
  catch { return ''; }
}
