// SPDX-License-Identifier: MIT

import { createHash, generateKeyPairSync, sign } from 'node:crypto';
import { describe, expect, it } from 'vitest';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_DIGEST_DOMAIN_V1,
  PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_MAX_BYTES_V1,
  parseProgrammeCaptureSupervisorCheckpointEnvelopeBlobV1,
  parseProgrammeCaptureSupervisorCheckpointV1,
  programmeCaptureSupervisorCheckpointSigningPayloadV1,
  serializeProgrammeCaptureSupervisorCheckpointEnvelopeV1,
  serializeProgrammeCaptureSupervisorCheckpointV1,
  verifyProgrammeCaptureSupervisorCheckpointEnvelopeV1,
} from '../src/programme-capture-supervisor-checkpoint-v1.js';
import { digestValue } from '../src/receipts.js';

const SUPERVISOR_ID = 'external_supervisor_checkpoint_20260829';
const LOG_ID = 'capture_checkpoint_log_20260829';
const KEY_EPOCH = 7;
const keyPair = generateKeyPairSync('ed25519');
const publicKeySpki = keyPair.publicKey.export({ format: 'der', type: 'spki' }) as Buffer;
const keyFingerprint = sha256(publicKeySpki);

describe('programme capture V1 non-authorizing signed supervisor checkpoint', () => {
  it('parses, canonicalizes, and verifies one independently anchored checkpoint', () => {
    const checkpoint = checkpointRecord();
    const serializedCheckpoint = serializeProgrammeCaptureSupervisorCheckpointV1(checkpoint);
    const serializedEnvelope = signedCheckpointEnvelope();
    expect(JSON.parse(serializedCheckpoint)).toEqual(checkpoint);
    expect(parseProgrammeCaptureSupervisorCheckpointEnvelopeBlobV1(serializedEnvelope)
      .checkpoint).toEqual(checkpoint);
    const verified = verifyProgrammeCaptureSupervisorCheckpointEnvelopeV1({
      serializedEnvelope, trustedPublicKeySpkiDer: publicKeySpki,
      expectedAuthorityKeyFingerprint: keyFingerprint,
      expectedSupervisorId: SUPERVISOR_ID, expectedLogId: LOG_ID,
      expectedKeyEpoch: KEY_EPOCH,
    });
    expect(verified).toEqual(checkpoint);
    expect(verified).toMatchObject({
      verificationScope: 'signed-log-checkpoint-only',
      externalAppendOnlyWitness: false,
      appendOnlyPersistenceVerified: false,
      rollbackResistance: 'not-proven',
      forkResistance: 'not-proven',
      globalOrderAuthority: 'not-proven',
      runnerLeaseAcquired: false,
      stateTransitionAuthorized: false,
      attemptStartAuthorized: false,
      captureAuthorized: false,
    });
    expect(Object.isFrozen(verified)).toBe(true);
  });

  it('rejects re-signed reference, tree, scope, and authority escalation mutations', () => {
    for (const override of [
      { supervisorId: 'different_checkpoint_supervisor_20260829' },
      { logId: 'different_checkpoint_log_20260829' },
      { keyEpoch: KEY_EPOCH + 1 },
      { authorityKeyFingerprint: '1'.repeat(64) },
      { treeSize: '03' }, { treeSize: '18446744073709551616' },
      { treeSize: '0', rootDigest: '2'.repeat(64) },
      { rootDigest: '0'.repeat(64) },
    ]) {
      expect(() => verifyProgrammeCaptureSupervisorCheckpointEnvelopeV1({
        ...verificationInput(), serializedEnvelope: signedCheckpointEnvelope(override),
      })).toThrow();
    }
    const invalid = checkpointBody() as any;
    invalid.captureAuthorized = true;
    invalid.checkpointDigest = digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_DIGEST_DOMAIN_V1,
      checkpoint: withoutDigest(invalid),
    });
    expect(() => parseProgrammeCaptureSupervisorCheckpointV1(invalid)).toThrow(/ESCALATION/);
    expect(() => checkpointRecord({
      treeSize: '0',
      rootDigest: 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
    })).not.toThrow();
    const corrupted = structuredClone(
      parseProgrammeCaptureSupervisorCheckpointEnvelopeBlobV1(signedCheckpointEnvelope()),
    ) as any;
    corrupted.signature.valueBase64Url = `${
      corrupted.signature.valueBase64Url.startsWith('A') ? 'B' : 'A'
    }${corrupted.signature.valueBase64Url.slice(1)}`;
    expect(() => verifyProgrammeCaptureSupervisorCheckpointEnvelopeV1({
      ...verificationInput(),
      serializedEnvelope: serializeProgrammeCaptureSupervisorCheckpointEnvelopeV1(corrupted),
    })).toThrow(/SIGNATURE/);
  });

  it('admits conflicting signed roots without claiming fork or rollback resistance', () => {
    const left = verifyProgrammeCaptureSupervisorCheckpointEnvelopeV1({
      ...verificationInput(),
      serializedEnvelope: signedCheckpointEnvelope({ rootDigest: 'a'.repeat(64) }),
    });
    const right = verifyProgrammeCaptureSupervisorCheckpointEnvelopeV1({
      ...verificationInput(),
      serializedEnvelope: signedCheckpointEnvelope({ rootDigest: 'b'.repeat(64) }),
    });
    expect(left.tree.treeSize).toBe(right.tree.treeSize);
    expect(left.tree.rootDigest).not.toBe(right.tree.rootDigest);
    for (const checkpoint of [left, right]) {
      expect(checkpoint.forkResistance).toBe('not-proven');
      expect(checkpoint.rollbackResistance).toBe('not-proven');
      expect(checkpoint.externalAppendOnlyWitness).toBe(false);
      expect(checkpoint.appendOnlyPersistenceVerified).toBe(false);
    }
  });

  it('rejects noncanonical, duplicate, oversized, loose, and Proxy inputs', () => {
    const canonical = signedCheckpointEnvelope();
    for (const invalid of [
      JSON.stringify(JSON.parse(canonical)), `${canonical} `, `\ufeff${canonical}`,
      canonical.replace('"schemaVersion": 1,', '"schemaVersion": 1,\n  "schemaVersion": 1,'),
      ' '.repeat(PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_MAX_BYTES_V1 + 1),
    ]) expect(() => parseProgrammeCaptureSupervisorCheckpointEnvelopeBlobV1(invalid)).toThrow();
    expect(() => parseProgrammeCaptureSupervisorCheckpointV1({
      ...checkpointRecord(), extra: true,
    })).toThrow(/invalid keys/);
    let trapCalls = 0;
    const proxy = new Proxy(checkpointRecord(), {
      getPrototypeOf(value) { trapCalls += 1; return Reflect.getPrototypeOf(value); },
      ownKeys(value) { trapCalls += 1; return Reflect.ownKeys(value); },
      getOwnPropertyDescriptor(value, property) {
        trapCalls += 1; return Reflect.getOwnPropertyDescriptor(value, property);
      },
    });
    expect(() => parseProgrammeCaptureSupervisorCheckpointV1(proxy)).toThrow(/Proxy/);
    expect(trapCalls).toBe(0);
  });
});

function verificationInput() {
  return {
    serializedEnvelope: signedCheckpointEnvelope(),
    trustedPublicKeySpkiDer: publicKeySpki,
    expectedAuthorityKeyFingerprint: keyFingerprint,
    expectedSupervisorId: SUPERVISOR_ID,
    expectedLogId: LOG_ID,
    expectedKeyEpoch: KEY_EPOCH,
  } as const;
}

function signedCheckpointEnvelope(overrides: Record<string, unknown> = {}): string {
  const checkpoint = checkpointRecord(overrides);
  const signature = sign(
    null, programmeCaptureSupervisorCheckpointSigningPayloadV1(checkpoint), keyPair.privateKey,
  ).toString('base64url');
  return serializeProgrammeCaptureSupervisorCheckpointEnvelopeV1({
    schemaVersion: 1,
    transactionKind: 'programme-capture-v1',
    envelopeKind: 'supervisor-log-checkpoint-envelope-v1',
    checkpoint,
    signature: { algorithm: 'ed25519', valueBase64Url: signature },
  });
}

function checkpointRecord(overrides: Record<string, unknown> = {}) {
  const body = checkpointBody(overrides);
  return parseProgrammeCaptureSupervisorCheckpointV1({
    ...body,
    checkpointDigest: digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_DIGEST_DOMAIN_V1,
      checkpoint: body,
    }),
  });
}

function checkpointBody(overrides: Record<string, unknown> = {}) {
  const {
    supervisorId = SUPERVISOR_ID, logId = LOG_ID, keyEpoch = KEY_EPOCH,
    authorityKeyFingerprint = keyFingerprint, treeSize = '41',
    rootDigest = '2'.repeat(64), ...rest
  } = overrides;
  return {
    schemaVersion: 1, transactionKind: 'programme-capture-v1',
    recordKind: 'supervisor-log-checkpoint-v1', authority: 'development-only-no-promotion',
    supervisor: { supervisorId, logId, keyEpoch, authorityKeyFingerprint },
    tree: { treeSize, rootDigest }, verificationScope: 'signed-log-checkpoint-only',
    externalAppendOnlyWitness: false, appendOnlyPersistenceVerified: false,
    rollbackResistance: 'not-proven', forkResistance: 'not-proven',
    globalOrderAuthority: 'not-proven', supervisorAdministration: 'not-attested',
    hostAdmission: 'not-evaluated', runnerLeaseAcquired: false,
    stateTransitionAuthorized: false, attemptStartAuthorized: false,
    captureAuthorized: false, ...rest,
  };
}

function withoutDigest(value: Record<string, unknown>) {
  const { checkpointDigest: _discarded, ...body } = value;
  return body;
}

function sha256(value: Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}
