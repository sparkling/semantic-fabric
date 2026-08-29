// SPDX-License-Identifier: MIT

import {
  createHash,
  createPrivateKey,
  createPublicKey,
  sign,
  verify,
} from 'node:crypto';
import { describe, expect, it, vi } from 'vitest';
import {
  createProgrammeCaptureSupervisorClaimAcknowledgementV1,
  parseProgrammeCaptureSupervisorClaimEnvelopeBlobV1,
  parseProgrammeCaptureSupervisorClaimRequestV1,
  programmeCaptureSupervisorClaimSigningPayloadV1,
  serializeProgrammeCaptureSupervisorClaimEnvelopeV1,
  verifyProgrammeCaptureSupervisorClaimAcknowledgementV1,
} from '../src/programme-capture-supervisor-claim-v1.js';

vi.mock('../src/programme-capture-claim-io-v1.js', () => ({
  readProgrammeCaptureRunClaimV1: async () => ({ record: fixedRequest().claim }),
}));

// RFC 8032 section 7.1, TEST 1. This public test seed must never become a runtime key.
// https://www.rfc-editor.org/rfc/rfc8032.html#section-7.1
const RFC_SEED =
  '9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60';
const RFC_PUBLIC_KEY =
  'd75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a';
const RFC_EMPTY_SIGNATURE =
  'e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155'
  + '5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b';
const PKCS8_PREFIX = '302e020100300506032b657004220420';
const SPKI_PREFIX = '302a300506032b6570032100';
const KEY_FINGERPRINT =
  '06e3fd8fda29bb60ab59557de61edb0aecdb231134be30e75b455f8e1b792fa9';
const CLAIM_KEY_DIGEST =
  '6ae0f3b6b43d2a03610045c827d104ecba2db3f2f9d1b21ff914d01d046b6f3d';
const CLAIM_DIGEST =
  '5aef5ddc2398c7d2efc735ee10400908b5fed1ca528591722d8074e90ebdc2e1';
const REQUEST_DIGEST =
  '4fcc54b6dcbb9ffc46a2c73a18db24619a03a750b4bfc4c5d444255d21202ac0';
const ACKNOWLEDGEMENT_DIGEST =
  'b83e60e32c6adad8c69587060b97271ccfbd35f6621df1ef54eae61cbe4cdccc';
const SIGNING_PAYLOAD_DIGEST =
  '9ba426e3254e589e4483d55c4e102752d64d09fc4b9d4e03285b8fb9eb40ebe0';
const PROTOCOL_SIGNATURE =
  '02n-_Bt92ooXEbnXwAU-wBqGqgUP166HbFh5AZ6zEGs1waMiOQm0jwz5UeVGUyLg'
  + 'rKawPaKmjfSdvk3hwCFqAQ';
const ENVELOPE_DIGEST =
  'e35577e7b7daeae084fd0126501f69c70109cb9ef65b5db390d1517e498d4b51';
const SIGNING_PAYLOAD_BASE64 = [
  'eyJhY2tub3dsZWRnZW1lbnQiOnsiYWNrbm93bGVkZ2VtZW50RGlnZXN0IjoiYjgzZTYwZTMyYzZhZGFkOGM2OTU4NzA2MGI5',
  'NzI3MWNjZmJkMzVmNjYyMWRmMWVmNTRlYWU2MWNiZTRjZGNjYyIsImFwcGVuZE9ubHlQZXJzaXN0ZW5jZVZlcmlmaWVkIjpm',
  'YWxzZSwiYXR0ZW1wdFN0YXJ0QXV0aG9yaXplZCI6ZmFsc2UsImF1dGhvcml0eSI6ImRldmVsb3BtZW50LW9ubHktbm8tcHJv',
  'bW90aW9uIiwiY2FwdHVyZUF1dGhvcml6ZWQiOmZhbHNlLCJjbGFpbURpZ2VzdCI6IjVhZWY1ZGRjMjM5OGM3ZDJlZmM3MzVl',
  'ZTEwNDAwOTA4YjVmZWQxY2E1Mjg1OTE3MjJkODA3NGU5MGViZGMyZTEiLCJjbGFpbUtleURpZ2VzdCI6IjZhZTBmM2I2YjQz',
  'ZDJhMDM2MTAwNDVjODI3ZDEwNGVjYmEyZGIzZjJmOWQxYjIxZmY5MTRkMDFkMDQ2YjZmM2QiLCJldmVudCI6eyJraW5kIjoi',
  'Y2xhaW0tcmVnaXN0ZXJlZC12MSIsImxvZ1NlcXVlbmNlIjo0MSwicHJldmlvdXNDaGVja3BvaW50RGlnZXN0IjoiNzc3Nzc3',
  'Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3NyIsInJ1blNlcXVlbmNlIjow',
  'fSwiZXh0ZXJuYWxBcHBlbmRPbmx5V2l0bmVzcyI6ZmFsc2UsImhvc3RBZG1pc3Npb24iOiJub3QtZXZhbHVhdGVkIiwicHJv',
  'amVjdEF1dGhvcml0eURpZ2VzdCI6IjExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTEx',
  'MTExMTExMTExMTExMTEiLCJyZWNvcmRLaW5kIjoic3VwZXJ2aXNvci1jbGFpbS1yZWdpc3RyYXRpb24tYWNrbm93bGVkZ2Vt',
  'ZW50LXYxIiwicmVxdWVzdERpZ2VzdCI6IjRmY2M1NGI2ZGNiYjlmZmM0NmEyYzczYTE4ZGIyNDYxOWEwM2E3NTBiNGJmYzRj',
  'NWQ0NDQyNTVkMjEyMDJhYzAiLCJyb2xsYmFja1Jlc2lzdGFuY2UiOiJub3QtcHJvdmVuIiwicnVuSWQiOiJjYXB0dXJlX3N1',
  'cGVydmlzb3Jfa2F0XzIwMjYwODI5XzAwMDEiLCJydW5uZXJMZWFzZUFjcXVpcmVkIjpmYWxzZSwic2NoZW1hVmVyc2lvbiI6',
  'MSwic3RhdGVUcmFuc2l0aW9uQXV0aG9yaXplZCI6ZmFsc2UsInN1cGVydmlzb3IiOnsiYXV0aG9yaXR5S2V5RmluZ2VycHJp',
  'bnQiOiIwNmUzZmQ4ZmRhMjliYjYwYWI1OTU1N2RlNjFlZGIwYWVjZGIyMzExMzRiZTMwZTc1YjQ1NWY4ZTFiNzkyZmE5Iiwi',
  'a2V5RXBvY2giOjcsImxvZ0lkIjoiY2xhaW1fbG9nX2thdF8yMDI2MDgyOSIsInN1cGVydmlzb3JJZCI6InN1cGVydmlzb3Jf',
  'a2F0XzIwMjYwODI5In0sInN1cGVydmlzb3JBZG1pbmlzdHJhdGlvbiI6Im5vdC1hdHRlc3RlZCIsInRyYW5zYWN0aW9uS2lu',
  'ZCI6InByb2dyYW1tZS1jYXB0dXJlLXYxIiwidmVyaWZpY2F0aW9uU2NvcGUiOiJzaWduYXR1cmUtYW5kLWNsYWltLWJpbmRp',
  'bmctb25seSJ9LCJkb21haW4iOiJzZW1hbnRpYy1mYWJyaWMvcHJvZ3JhbW1lLWNhcHR1cmUvc3VwZXJ2aXNvci1jbGFpbS1h',
  'Y2tub3dsZWRnZW1lbnQtc2lnbmluZy12MSJ9',
].join('');

const ENVELOPE_BASE64 = [
  'ewogICJzY2hlbWFWZXJzaW9uIjogMSwKICAidHJhbnNhY3Rpb25LaW5kIjogInByb2dyYW1tZS1jYXB0dXJlLXYxIiwKICAi',
  'ZW52ZWxvcGVLaW5kIjogInN1cGVydmlzb3ItY2xhaW0tYWNrbm93bGVkZ2VtZW50LWVudmVsb3BlLXYxIiwKICAiYWNrbm93',
  'bGVkZ2VtZW50IjogewogICAgInNjaGVtYVZlcnNpb24iOiAxLAogICAgInRyYW5zYWN0aW9uS2luZCI6ICJwcm9ncmFtbWUt',
  'Y2FwdHVyZS12MSIsCiAgICAicmVjb3JkS2luZCI6ICJzdXBlcnZpc29yLWNsYWltLXJlZ2lzdHJhdGlvbi1hY2tub3dsZWRn',
  'ZW1lbnQtdjEiLAogICAgImF1dGhvcml0eSI6ICJkZXZlbG9wbWVudC1vbmx5LW5vLXByb21vdGlvbiIsCiAgICAicnVuSWQi',
  'OiAiY2FwdHVyZV9zdXBlcnZpc29yX2thdF8yMDI2MDgyOV8wMDAxIiwKICAgICJwcm9qZWN0QXV0aG9yaXR5RGlnZXN0Ijog',
  'IjExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTEiLAogICAg',
  'ImNsYWltS2V5RGlnZXN0IjogIjZhZTBmM2I2YjQzZDJhMDM2MTAwNDVjODI3ZDEwNGVjYmEyZGIzZjJmOWQxYjIxZmY5MTRk',
  'MDFkMDQ2YjZmM2QiLAogICAgImNsYWltRGlnZXN0IjogIjVhZWY1ZGRjMjM5OGM3ZDJlZmM3MzVlZTEwNDAwOTA4YjVmZWQx',
  'Y2E1Mjg1OTE3MjJkODA3NGU5MGViZGMyZTEiLAogICAgInJlcXVlc3REaWdlc3QiOiAiNGZjYzU0YjZkY2JiOWZmYzQ2YTJj',
  'NzNhMThkYjI0NjE5YTAzYTc1MGI0YmZjNGM1ZDQ0NDI1NWQyMTIwMmFjMCIsCiAgICAic3VwZXJ2aXNvciI6IHsKICAgICAg',
  'InN1cGVydmlzb3JJZCI6ICJzdXBlcnZpc29yX2thdF8yMDI2MDgyOSIsCiAgICAgICJsb2dJZCI6ICJjbGFpbV9sb2dfa2F0',
  'XzIwMjYwODI5IiwKICAgICAgImtleUVwb2NoIjogNywKICAgICAgImF1dGhvcml0eUtleUZpbmdlcnByaW50IjogIjA2ZTNm',
  'ZDhmZGEyOWJiNjBhYjU5NTU3ZGU2MWVkYjBhZWNkYjIzMTEzNGJlMzBlNzViNDU1ZjhlMWI3OTJmYTkiCiAgICB9LAogICAg',
  'ImV2ZW50IjogewogICAgICAia2luZCI6ICJjbGFpbS1yZWdpc3RlcmVkLXYxIiwKICAgICAgInJ1blNlcXVlbmNlIjogMCwK',
  'ICAgICAgImxvZ1NlcXVlbmNlIjogNDEsCiAgICAgICJwcmV2aW91c0NoZWNrcG9pbnREaWdlc3QiOiAiNzc3Nzc3Nzc3Nzc3',
  'Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3Nzc3NyIKICAgIH0sCiAgICAidmVyaWZp',
  'Y2F0aW9uU2NvcGUiOiAic2lnbmF0dXJlLWFuZC1jbGFpbS1iaW5kaW5nLW9ubHkiLAogICAgImV4dGVybmFsQXBwZW5kT25s',
  'eVdpdG5lc3MiOiBmYWxzZSwKICAgICJhcHBlbmRPbmx5UGVyc2lzdGVuY2VWZXJpZmllZCI6IGZhbHNlLAogICAgInJvbGxi',
  'YWNrUmVzaXN0YW5jZSI6ICJub3QtcHJvdmVuIiwKICAgICJzdXBlcnZpc29yQWRtaW5pc3RyYXRpb24iOiAibm90LWF0dGVz',
  'dGVkIiwKICAgICJob3N0QWRtaXNzaW9uIjogIm5vdC1ldmFsdWF0ZWQiLAogICAgInJ1bm5lckxlYXNlQWNxdWlyZWQiOiBm',
  'YWxzZSwKICAgICJzdGF0ZVRyYW5zaXRpb25BdXRob3JpemVkIjogZmFsc2UsCiAgICAiYXR0ZW1wdFN0YXJ0QXV0aG9yaXpl',
  'ZCI6IGZhbHNlLAogICAgImNhcHR1cmVBdXRob3JpemVkIjogZmFsc2UsCiAgICAiYWNrbm93bGVkZ2VtZW50RGlnZXN0Ijog',
  'ImI4M2U2MGUzMmM2YWRhZDhjNjk1ODcwNjBiOTcyNzFjY2ZiZDM1ZjY2MjFkZjFlZjU0ZWFlNjFjYmU0Y2RjY2MiCiAgfSwK',
  'ICAic2lnbmF0dXJlIjogewogICAgImFsZ29yaXRobSI6ICJlZDI1NTE5IiwKICAgICJ2YWx1ZUJhc2U2NFVybCI6ICIwMm4t',
  'X0J0OTJvb1hFYm5Yd0FVLXdCcUdxZ1VQMTY2SGJGaDVBWjZ6RUdzMXdhTWlPUW0wand6NVVlVkdVeUxnckthd1BhS21qZlNk',
  'dmszaHdDRnFBUSIKICB9Cn0K',
].join('');

describe('programme capture V1 supervisor wire known-answer vector', () => {
  it('pins the Ed25519 key, digests, signing bytes, signature, and envelope bytes', async () => {
    const privateKey = createPrivateKey({
      key: Buffer.from(`${PKCS8_PREFIX}${RFC_SEED}`, 'hex'),
      format: 'der',
      type: 'pkcs8',
    });
    const derivedPublicKey = createPublicKey(privateKey);
    const publicKeySpki = derivedPublicKey.export({ format: 'der', type: 'spki' }) as Buffer;
    const trustedPublicKey = createPublicKey({
      key: Buffer.from(`${SPKI_PREFIX}${RFC_PUBLIC_KEY}`, 'hex'),
      format: 'der',
      type: 'spki',
    });
    expect(publicKeySpki.toString('hex')).toBe(`${SPKI_PREFIX}${RFC_PUBLIC_KEY}`);
    expect(sha256(publicKeySpki)).toBe(KEY_FINGERPRINT);
    expect(sign(null, Buffer.alloc(0), privateKey).toString('hex')).toBe(RFC_EMPTY_SIGNATURE);
    expect(verify(
      null, Buffer.alloc(0), trustedPublicKey, Buffer.from(RFC_EMPTY_SIGNATURE, 'hex'),
    )).toBe(true);

    const request = fixedRequest();
    expect(request.requestDigest).toBe(REQUEST_DIGEST);
    const acknowledgement = createProgrammeCaptureSupervisorClaimAcknowledgementV1({
      request,
      supervisorId: 'supervisor_kat_20260829',
      logId: 'claim_log_kat_20260829',
      keyEpoch: 7,
      authorityKeyFingerprint: KEY_FINGERPRINT,
      logSequence: 41,
      previousCheckpointDigest: '77'.repeat(32),
    });
    expect(acknowledgement.acknowledgementDigest).toBe(ACKNOWLEDGEMENT_DIGEST);
    const wrongRequestDomain = structuredClone(request) as any;
    wrongRequestDomain.requestDigest = ACKNOWLEDGEMENT_DIGEST;
    expect(() => parseProgrammeCaptureSupervisorClaimRequestV1(wrongRequestDomain))
      .toThrow('HARNESS_CAPTURE_SUPERVISOR_REQUEST_DIGEST_MISMATCH');
    const wrongAcknowledgementDomain = structuredClone(acknowledgement) as any;
    wrongAcknowledgementDomain.acknowledgementDigest = REQUEST_DIGEST;
    expect(() => programmeCaptureSupervisorClaimSigningPayloadV1(wrongAcknowledgementDomain))
      .toThrow('HARNESS_CAPTURE_SUPERVISOR_ACK_DIGEST_MISMATCH');

    const signingPayload = programmeCaptureSupervisorClaimSigningPayloadV1(acknowledgement);
    expect(signingPayload).toEqual(Buffer.from(SIGNING_PAYLOAD_BASE64, 'base64'));
    expect(sha256(signingPayload)).toBe(SIGNING_PAYLOAD_DIGEST);
    const signature = sign(null, signingPayload, privateKey).toString('base64url');
    expect(signature).toBe(PROTOCOL_SIGNATURE);
    const protocolSignature = Buffer.from(signature, 'base64url');
    expect(verify(
      null, signingPayload, trustedPublicKey, protocolSignature,
    )).toBe(true);
    expect(verify(
      null, signingPayload, trustedPublicKey, Buffer.from(RFC_EMPTY_SIGNATURE, 'hex'),
    )).toBe(false);
    expect(verify(null, Buffer.alloc(0), trustedPublicKey, protocolSignature)).toBe(false);
    const changedPayload = Buffer.from(signingPayload);
    changedPayload[changedPayload.length - 1] ^= 1;
    expect(verify(null, changedPayload, trustedPublicKey, protocolSignature)).toBe(false);

    const envelope = {
      schemaVersion: 1,
      transactionKind: 'programme-capture-v1',
      envelopeKind: 'supervisor-claim-acknowledgement-envelope-v1',
      acknowledgement,
      signature: { algorithm: 'ed25519', valueBase64Url: signature },
    } as const;
    const serialized = serializeProgrammeCaptureSupervisorClaimEnvelopeV1(envelope);
    const pinnedEnvelope = Buffer.from(ENVELOPE_BASE64, 'base64').toString('utf8');
    expect(Buffer.from(serialized, 'utf8')).toEqual(Buffer.from(pinnedEnvelope, 'utf8'));
    expect(Buffer.from(pinnedEnvelope, 'utf8').toString('base64')).toBe(ENVELOPE_BASE64);
    expect(sha256(Buffer.from(serialized, 'utf8'))).toBe(ENVELOPE_DIGEST);
    expect(parseProgrammeCaptureSupervisorClaimEnvelopeBlobV1(pinnedEnvelope)).toEqual(envelope);
    const productionVerification = {
      claimAuthority: fixedClaimAuthority(),
      serializedEnvelope: pinnedEnvelope,
      trustedPublicKeySpkiDer: Buffer.from(`${SPKI_PREFIX}${RFC_PUBLIC_KEY}`, 'hex'),
      expectedAuthorityKeyFingerprint: KEY_FINGERPRINT,
      expectedSupervisorId: 'supervisor_kat_20260829',
      expectedLogId: 'claim_log_kat_20260829',
      expectedKeyEpoch: 7,
      expectedLogSequence: 41,
      expectedPreviousCheckpointDigest: '77'.repeat(32),
    } as const;
    const validation = await verifyProgrammeCaptureSupervisorClaimAcknowledgementV1(
      productionVerification,
    );
    expect(validation).toMatchObject({
      requestDigest: REQUEST_DIGEST,
      acknowledgementDigest: ACKNOWLEDGEMENT_DIGEST,
      serializedEnvelopeDigest: ENVELOPE_DIGEST,
      signatureVerified: true,
      externalAppendOnlyWitness: false,
      stateTransitionAuthorized: false,
      captureAuthorized: false,
    });
    expect(new Set([
      REQUEST_DIGEST,
      ACKNOWLEDGEMENT_DIGEST,
      SIGNING_PAYLOAD_DIGEST,
      ENVELOPE_DIGEST,
      validation.validationDigest,
    ]).size).toBe(5);
    const changedSignature = Buffer.from(protocolSignature);
    changedSignature[0] ^= 1;
    expect(verify(null, signingPayload, trustedPublicKey, changedSignature)).toBe(false);
    await expect(verifyProgrammeCaptureSupervisorClaimAcknowledgementV1({
      ...productionVerification,
      serializedEnvelope: pinnedEnvelope.replace(
        PROTOCOL_SIGNATURE, changedSignature.toString('base64url'),
      ),
    })).rejects.toThrow('HARNESS_CAPTURE_SUPERVISOR_SIGNATURE_INVALID');
  });
});

function fixedClaimAuthority() {
  return {
    authorityRoot: '/test-only/supervisor-kat-authority',
    projectAuthorityDigest: '11'.repeat(32),
    runId: 'capture_supervisor_kat_20260829_0001',
    controllerStore: '/test-only/supervisor-kat-controller',
    controllerCommit: 'a'.repeat(40),
    taskPath: 'coding-harness/config/programme-v5-acceptance.json',
    expectedRunnerIdentityDigest: '66'.repeat(32),
  };
}

function fixedRequest() {
  return parseProgrammeCaptureSupervisorClaimRequestV1({
    schemaVersion: 1,
    transactionKind: 'programme-capture-v1',
    requestKind: 'supervisor-claim-registration-request-v1',
    authority: 'development-only-no-promotion',
    claim: {
      schemaVersion: 1,
      transactionKind: 'programme-capture-v1',
      recordKind: 'run-claim-v1',
      authority: {
        projectAuthorityDigest: '11'.repeat(32),
        persistence: 'same-uid-create-new-v1',
        rollbackResistance: 'not-proven',
        externalAppendOnlyWitness: false,
      },
      runId: 'capture_supervisor_kat_20260829_0001',
      controller: { commit: 'a'.repeat(40), tree: 'b'.repeat(40) },
      task: {
        path: 'coding-harness/config/programme-v5-acceptance.json',
        gitBlobId: 'c'.repeat(40),
        sha256: '22'.repeat(32),
        byteLength: 4096,
        valueDigest: '33'.repeat(32),
      },
      inputAttestationDigest: '44'.repeat(32),
      runnerProfile: {
        path: 'crates/sf-bench/config/performance-runner-profile-v1.tsv',
        gitBlobId: 'd'.repeat(40),
        sha256: '55'.repeat(32),
        byteLength: 2048,
      },
      expectedRunnerIdentityDigest: '66'.repeat(32),
      hostAdmission: 'not-evaluated',
      runnerLeaseAcquired: false,
      attemptStartAuthorized: false,
      captureAuthorized: false,
      claimKeyDigest: CLAIM_KEY_DIGEST,
      claimDigest: CLAIM_DIGEST,
    },
    externalAppendOnlyWitness: false,
    appendOnlyPersistenceVerified: false,
    rollbackResistance: 'not-proven',
    supervisorAdministration: 'not-attested',
    hostAdmission: 'not-evaluated',
    runnerLeaseAcquired: false,
    stateTransitionAuthorized: false,
    attemptStartAuthorized: false,
    captureAuthorized: false,
    requestDigest: REQUEST_DIGEST,
  });
}

function sha256(value: Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}
