// SPDX-License-Identifier: MIT

import {
  createHash,
  createPublicKey,
  verify as verifyDetachedSignature,
  type KeyObject,
} from 'node:crypto';
import { isProxy } from 'node:util/types';
import {
  SHA256_PATTERN,
  asClosedRecord,
  assertExactKeys,
  snapshotUint8Array,
} from './contracts.js';

const ED25519_SPKI_PREFIX = Buffer.from('302a300506032b6570032100', 'hex');
const ED25519_SPKI_BYTES = 44;
const ED25519_SIGNATURE_BYTES = 64;
const MAX_SIGNING_PAYLOAD_BYTES = 131_072;

export function parseProgrammeCaptureSupervisorEd25519SignatureV1(value: unknown): string {
  if (typeof value !== 'string' || !/^[A-Za-z0-9_-]{86}$/.test(value)) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_SIGNATURE_INVALID');
  }
  const bytes = Buffer.from(value, 'base64url');
  if (bytes.length !== ED25519_SIGNATURE_BYTES || bytes.toString('base64url') !== value) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_SIGNATURE_INVALID');
  }
  return value;
}

export function verifyProgrammeCaptureSupervisorEd25519SignatureV1(value: unknown): void {
  if (isProxy(value)) {
    throw new TypeError('supervisor signature verification input must not be a Proxy');
  }
  const input = asClosedRecord(value, 'supervisor signature verification input');
  assertExactKeys(input, [
    'payload', 'signatureBase64Url', 'trustedPublicKeySpkiDer',
    'expectedAuthorityKeyFingerprint',
  ], 'supervisor signature verification input');
  const payload = snapshotUint8Array(
    input.payload, 'supervisor signature payload', MAX_SIGNING_PAYLOAD_BYTES,
  );
  const signature = Buffer.from(
    parseProgrammeCaptureSupervisorEd25519SignatureV1(input.signatureBase64Url), 'base64url',
  );
  const expectedFingerprint = parseFingerprint(input.expectedAuthorityKeyFingerprint);
  const trustedKey = trustedEd25519Key(input.trustedPublicKeySpkiDer, expectedFingerprint);
  if (!verifyDetachedSignature(null, payload, trustedKey, signature)) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_SIGNATURE_INVALID');
  }
}

function trustedEd25519Key(value: unknown, expectedFingerprint: string): KeyObject {
  const bytes = Buffer.from(snapshotUint8Array(value, 'trusted Ed25519 SPKI', 1_024));
  if (bytes.length !== ED25519_SPKI_BYTES
    || !bytes.subarray(0, ED25519_SPKI_PREFIX.length).equals(ED25519_SPKI_PREFIX)) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_PUBLIC_KEY_INVALID');
  }
  let key: KeyObject;
  try { key = createPublicKey({ key: bytes, format: 'der', type: 'spki' }); }
  catch (error) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_PUBLIC_KEY_INVALID', { cause: error });
  }
  const canonicalBytes = key.export({ format: 'der', type: 'spki' }) as Buffer;
  if (key.asymmetricKeyType !== 'ed25519' || !canonicalBytes.equals(bytes)
    || createHash('sha256').update(bytes).digest('hex') !== expectedFingerprint) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_PUBLIC_KEY_INVALID');
  }
  return key;
}

function parseFingerprint(value: unknown): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || /^0+$/.test(value)) {
    throw new TypeError('supervisor key fingerprint must be a non-zero lowercase SHA-256 digest');
  }
  return value;
}
