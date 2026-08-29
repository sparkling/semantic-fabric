// SPDX-License-Identifier: MIT

import {
  createHash,
  createPrivateKey,
  createPublicKey,
  sign,
  verify,
} from 'node:crypto';
import { describe, expect, it } from 'vitest';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_DIGEST_DOMAIN_V1,
  PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_SIGNING_DOMAIN_V1,
  parseProgrammeCaptureSupervisorCheckpointBlobV1,
  parseProgrammeCaptureSupervisorCheckpointEnvelopeBlobV1,
  programmeCaptureSupervisorCheckpointSigningPayloadV1,
  serializeProgrammeCaptureSupervisorCheckpointEnvelopeV1,
  serializeProgrammeCaptureSupervisorCheckpointV1,
  verifyProgrammeCaptureSupervisorCheckpointEnvelopeV1,
} from '../src/programme-capture-supervisor-checkpoint-v1.js';

const RFC_SEED =
  '9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60';
const RFC_PUBLIC_KEY =
  'd75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a';
const PKCS8_PREFIX = '302e020100300506032b657004220420';
const SPKI_PREFIX = '302a300506032b6570032100';
const KEY_FINGERPRINT =
  '06e3fd8fda29bb60ab59557de61edb0aecdb231134be30e75b455f8e1b792fa9';
const DIGEST_DOMAIN =
  'semantic-fabric/programme-capture/supervisor-log-checkpoint-digest-v1';
const SIGNING_DOMAIN =
  'semantic-fabric/programme-capture/supervisor-log-checkpoint-signing-v1';
const CHECKPOINT_DIGEST =
  '838a22b920f9295dd7986e546e1b16cb400398d6d213fecfddcfb700835783ec';
const CHECKPOINT_BYTES_DIGEST =
  'f23140aab1c20e5de1896d22560875723fb707223e6687ff984fbdc033e00e69';
const SIGNING_PAYLOAD_DIGEST =
  'd2da533384d6a4a88f2dea2e57175daf6d4fcae7c945445e59ce7a4abd77be1b';
const PROTOCOL_SIGNATURE =
  'n6cHWZuk2tBayju5SYKR-BkdIjqbSB98MJVIbhUOCjPZd4OQ1CM49BVvAZsToPR4JiNdUALNRY6qnbRI_ZUYDw';
const ENVELOPE_BYTES_DIGEST =
  'b76c898e21f0bcdcfad21d45bd535712b1460bd76f7b66c5863db35f7657f3fb';

describe('programme capture V1 supervisor checkpoint wire known-answer vector', () => {
  it('pins independent canonical bytes, domains, digests, key, and signature', () => {
    expect(PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_DIGEST_DOMAIN_V1).toBe(DIGEST_DOMAIN);
    expect(PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_SIGNING_DOMAIN_V1).toBe(SIGNING_DOMAIN);
    const privateKey = createPrivateKey({
      key: Buffer.from(`${PKCS8_PREFIX}${RFC_SEED}`, 'hex'), format: 'der', type: 'pkcs8',
    });
    const publicKeySpki = createPublicKey(privateKey)
      .export({ format: 'der', type: 'spki' }) as Buffer;
    expect(publicKeySpki.toString('hex')).toBe(`${SPKI_PREFIX}${RFC_PUBLIC_KEY}`);
    expect(sha256(publicKeySpki)).toBe(KEY_FINGERPRINT);

    const body = fixedCheckpointBody();
    expect(sha256(Buffer.from(stableJson({ domain: DIGEST_DOMAIN, checkpoint: body }), 'utf8')))
      .toBe(CHECKPOINT_DIGEST);
    const checkpoint = { ...body, checkpointDigest: CHECKPOINT_DIGEST } as const;
    const checkpointBytes = `${JSON.stringify(checkpoint, null, 2)}\n`;
    expect(sha256(Buffer.from(checkpointBytes, 'utf8'))).toBe(CHECKPOINT_BYTES_DIGEST);
    expect(parseProgrammeCaptureSupervisorCheckpointBlobV1(checkpointBytes)).toEqual(checkpoint);
    expect(serializeProgrammeCaptureSupervisorCheckpointV1(checkpoint)).toBe(checkpointBytes);

    const independentPayload = Buffer.from(stableJson({
      domain: SIGNING_DOMAIN, checkpoint,
    }), 'utf8');
    expect(sha256(independentPayload)).toBe(SIGNING_PAYLOAD_DIGEST);
    expect(programmeCaptureSupervisorCheckpointSigningPayloadV1(checkpoint))
      .toEqual(independentPayload);
    const signature = sign(null, independentPayload, privateKey).toString('base64url');
    expect(signature).toBe(PROTOCOL_SIGNATURE);
    expect(verify(
      null, independentPayload, createPublicKey(privateKey), Buffer.from(signature, 'base64url'),
    )).toBe(true);

    const envelope = {
      schemaVersion: 1, transactionKind: 'programme-capture-v1',
      envelopeKind: 'supervisor-log-checkpoint-envelope-v1', checkpoint,
      signature: { algorithm: 'ed25519', valueBase64Url: PROTOCOL_SIGNATURE },
    } as const;
    const envelopeBytes = `${JSON.stringify(envelope, null, 2)}\n`;
    expect(sha256(Buffer.from(envelopeBytes, 'utf8'))).toBe(ENVELOPE_BYTES_DIGEST);
    expect(serializeProgrammeCaptureSupervisorCheckpointEnvelopeV1(envelope))
      .toBe(envelopeBytes);
    expect(parseProgrammeCaptureSupervisorCheckpointEnvelopeBlobV1(envelopeBytes))
      .toEqual(envelope);
    expect(verifyProgrammeCaptureSupervisorCheckpointEnvelopeV1({
      serializedEnvelope: envelopeBytes,
      trustedPublicKeySpkiDer: publicKeySpki,
      expectedAuthorityKeyFingerprint: KEY_FINGERPRINT,
      expectedSupervisorId: 'supervisor_checkpoint_kat_20260829',
      expectedLogId: 'checkpoint_log_kat_20260829', expectedKeyEpoch: 7,
    })).toEqual(checkpoint);
  });
});

function fixedCheckpointBody() {
  return {
    schemaVersion: 1, transactionKind: 'programme-capture-v1',
    recordKind: 'supervisor-log-checkpoint-v1', authority: 'development-only-no-promotion',
    supervisor: {
      supervisorId: 'supervisor_checkpoint_kat_20260829',
      logId: 'checkpoint_log_kat_20260829', keyEpoch: 7,
      authorityKeyFingerprint: KEY_FINGERPRINT,
    },
    tree: { treeSize: '41', rootDigest: '88'.repeat(32) },
    verificationScope: 'signed-log-checkpoint-only',
    externalAppendOnlyWitness: false, appendOnlyPersistenceVerified: false,
    rollbackResistance: 'not-proven', forkResistance: 'not-proven',
    globalOrderAuthority: 'not-proven', supervisorAdministration: 'not-attested',
    hostAdmission: 'not-evaluated', runnerLeaseAcquired: false,
    stateTransitionAuthorized: false, attemptStartAuthorized: false,
    captureAuthorized: false,
  } as const;
}

function stableJson(value: unknown): string {
  return JSON.stringify(sortKeys(value));
}

function sortKeys(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sortKeys);
  if (value !== null && typeof value === 'object') {
    const record = value as Record<string, unknown>;
    return Object.fromEntries(Object.keys(record).sort().map((key) => [key, sortKeys(record[key])]));
  }
  return value;
}

function sha256(value: Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}
