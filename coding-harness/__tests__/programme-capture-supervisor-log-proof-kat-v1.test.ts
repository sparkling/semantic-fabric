// SPDX-License-Identifier: MIT

import { createHash, createPrivateKey, createPublicKey, sign } from 'node:crypto';
import { describe, expect, it, vi } from 'vitest';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_ACK_SIGNING_DOMAIN_V1,
  createProgrammeCaptureSupervisorClaimAcknowledgementV1,
  parseProgrammeCaptureSupervisorClaimRequestV1,
  programmeCaptureSupervisorClaimSigningPayloadV1,
  serializeProgrammeCaptureSupervisorClaimEnvelopeV1,
} from '../src/programme-capture-supervisor-claim-v1.js';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_DIGEST_DOMAIN_V1,
  PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_SIGNING_DOMAIN_V1,
  programmeCaptureSupervisorCheckpointSigningPayloadV1,
  serializeProgrammeCaptureSupervisorCheckpointEnvelopeV1,
  serializeProgrammeCaptureSupervisorCheckpointV1,
} from '../src/programme-capture-supervisor-checkpoint-v1.js';
import {
  createProgrammeCaptureSupervisorRegistrationLogProofValidationBlobV1,
  replayProgrammeCaptureSupervisorRegistrationLogProofValidationV1,
} from '../src/programme-capture-supervisor-log-proof-codec-v1.js';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_LOG_PROOF_VALIDATION_DIGEST_DOMAIN_V1,
  PROGRAMME_CAPTURE_SUPERVISOR_REGISTRATION_LOG_LEAF_DOMAIN_V1,
  programmeCaptureSupervisorRegistrationLogLeafBytesV1,
  verifyProgrammeCaptureSupervisorRegistrationLogProofV1,
} from '../src/programme-capture-supervisor-log-proof-v1.js';

vi.mock('../src/programme-capture-claim-io-v1.js', () => ({
  readProgrammeCaptureRunClaimV1: async () => ({ record: fixedRequest().claim }),
}));

// RFC 8032 section 7.1, TEST 1. This public test seed must never become a runtime key.
// https://www.rfc-editor.org/rfc/rfc8032.html#section-7.1
const RFC_SEED =
  '9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60';
const RFC_PUBLIC_KEY =
  'd75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a';
const PKCS8_PREFIX = '302e020100300506032b657004220420';
const SPKI_PREFIX = '302a300506032b6570032100';
const KEY_FINGERPRINT =
  '06e3fd8fda29bb60ab59557de61edb0aecdb231134be30e75b455f8e1b792fa9';
const SUPERVISOR_ID = 'supervisor_log_proof_kat_20260829';
const LOG_ID = 'registration_log_proof_kat_20260829';
const CLAIM_KEY_DIGEST =
  '6ae0f3b6b43d2a03610045c827d104ecba2db3f2f9d1b21ff914d01d046b6f3d';
const CLAIM_DIGEST =
  '5aef5ddc2398c7d2efc735ee10400908b5fed1ca528591722d8074e90ebdc2e1';
const REQUEST_DIGEST =
  '4fcc54b6dcbb9ffc46a2c73a18db24619a03a750b4bfc4c5d444255d21202ac0';
const PATH_DIGEST_DOMAIN =
  'semantic-fabric/programme-capture/supervisor-registration-log-proof-path-digest-v1';

const EXPECTED_VECTOR = Object.freeze({
  acknowledgementDigest:
    '3a2eddf0b8520581f5b951d94bbab9497f49cb968447a4c2775415a2653b2db1',
  registrationSignature:
    'QpN-aDWqtynJ39r38QlP0TKkGv0kuEdnEop6jLtxLCT50e4IdzZrrgjf5r1vzb9nDqlixgGXJZIrC4c_doyZCA',
  registrationEnvelopeDigest:
    '675503a2b6cb015b506bb4f1d3807b1517bb2bc5533287cd421258ecc711052b',
  registrationEnvelopeByteLength: 1912,
  leafPayloadBase64:
    'eyJkb21haW4iOiJzZW1hbnRpYy1mYWJyaWMvcHJvZ3JhbW1lLWNhcHR1cmUvc3VwZXJ2aXNvci1yZWdpc3RyYXRpb24tbG9nLWxlYWYtdjEiLCJsZWFmIjp7ImFja25vd2xlZGdlbWVudERpZ2VzdCI6IjNhMmVkZGYwYjg1MjA1ODFmNWI5NTFkOTRiYmFiOTQ5N2Y0OWNiOTY4NDQ3YTRjMjc3NTQxNWEyNjUzYjJkYjEiLCJsZWFmS2luZCI6InN1cGVydmlzb3ItY2xhaW0tcmVnaXN0cmF0aW9uLWxvZy1sZWFmLXYxIiwibG9nSWQiOiJyZWdpc3RyYXRpb25fbG9nX3Byb29mX2thdF8yMDI2MDgyOSIsImxvZ1NlcXVlbmNlIjoiMyIsInNjaGVtYVZlcnNpb24iOjEsInNlcmlhbGl6ZWRSZWdpc3RyYXRpb25FbnZlbG9wZURpZ2VzdCI6IjY3NTUwM2EyYjZjYjAxNWI1MDZiYjRmMWQzODA3YjE1MTdiYjJiYzU1MzMyODdjZDQyMTI1OGVjYzcxMTA1MmIiLCJ0cmFuc2FjdGlvbktpbmQiOiJwcm9ncmFtbWUtY2FwdHVyZS12MSJ9fQ==',
  leafPayloadDigest:
    'a151b0415f4fa6a42569ebea4ca1a4bdef6896605599bcda303ef49697a8fd2b',
  merkleLeafDigest:
    '8143b13b1ee15f7ae0c6f3e9c13fe1bd86bbbeb8957d796b33ac104c75c5d119',
  priorCheckpointDigest:
    '0ccb553c152941d8272fd80cdfd2834c1bf844c41ef8da9948a8a689e8da1ca3',
  priorRootDigest:
    'ee8527191bc104c39d1111af551f7141f446da9395d843cc2dfc1573105fb1e5',
  priorCheckpointBytesDigest:
    '3014c97189e8b88f25288c73bf5accb41bd9da423d3209abdda175151753f4e2',
  priorCheckpointByteLength: 1090,
  newCheckpointDigest:
    '17e95a8c400b43e9d5a7aab956913616a2fa892864945e91d44cddbd068de6a3',
  newRootDigest:
    'b29c377ffcf99d6523ae924827006a103df1c44c095cfe62b5580002a728d667',
  checkpointSignature:
    'TKQJ9QI03kBE60haZzBBdBpWzKtpnKOdf5KEhXwHYVB-LGX7D8qxA1NrGUPVduS_d_wlXi8xlpLpwTkUOA2JCQ',
  newCheckpointEnvelopeBytesDigest:
    '42945937c2123b0a4478bebc32066cbce13acc8ea31285848020eef8bc1e238d',
  newCheckpointEnvelopeByteLength: 1453,
  inclusionProofDigests: [
    'ee8527191bc104c39d1111af551f7141f446da9395d843cc2dfc1573105fb1e5',
  ],
  consistencyProofDigests: [
    '8143b13b1ee15f7ae0c6f3e9c13fe1bd86bbbeb8957d796b33ac104c75c5d119',
  ],
  inclusionProofListDigest:
    '852722df0dbe8a2e9fe89d15658876424587ed4a2a9bef2be562672b3b7b4137',
  consistencyProofListDigest:
    'acf7eb8c3297f4025634fd8bfdc46bd980da70377f8e3ab19da8a9667e2b6307',
  validationDigest:
    'eca1b7e1aa7a66b4a544dd2d0aaaf6f4b830a73113229283ac48797789011ee6',
  validationBytesDigest:
    'a57967f31b8a1cf2b35a0df91e890f603915a9ce786f0cebac32336930eeafb7',
  validationByteLength: 3250,
});

describe('programme capture V1 supervisor registration log proof known-answer vector', () => {
  it('pins independent leaf, tree, proof, checkpoint, and validation bytes', async () => {
    const privateKey = createPrivateKey({
      key: Buffer.from(`${PKCS8_PREFIX}${RFC_SEED}`, 'hex'), format: 'der', type: 'pkcs8',
    });
    const publicKeySpki = createPublicKey(privateKey)
      .export({ format: 'der', type: 'spki' }) as Buffer;
    expect(publicKeySpki.toString('hex')).toBe(`${SPKI_PREFIX}${RFC_PUBLIC_KEY}`);
    expect(sha256(publicKeySpki)).toBe(KEY_FINGERPRINT);

    const oldLeaves = [Buffer.from('kat-old-0'), Buffer.from('kat-old-1')];
    const priorCheckpoint = checkpointRecord(oldLeaves);
    const serializedPriorCheckpoint = `${JSON.stringify(priorCheckpoint, null, 2)}\n`;
    expect(serializeProgrammeCaptureSupervisorCheckpointV1(priorCheckpoint))
      .toBe(serializedPriorCheckpoint);

    const acknowledgement = createProgrammeCaptureSupervisorClaimAcknowledgementV1({
      request: fixedRequest(), supervisorId: SUPERVISOR_ID, logId: LOG_ID,
      keyEpoch: 7, authorityKeyFingerprint: KEY_FINGERPRINT, logSequence: 3,
      previousCheckpointDigest: priorCheckpoint.checkpointDigest,
    });
    const acknowledgementBody = withoutKey(acknowledgement, 'acknowledgementDigest');
    expect(acknowledgement.acknowledgementDigest).toBe(domainDigest(
      'semantic-fabric/programme-capture/supervisor-claim-acknowledgement-digest-v1',
      'acknowledgement', acknowledgementBody,
    ));
    const registrationSigningPayload = Buffer.from(stableJson({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_ACK_SIGNING_DOMAIN_V1, acknowledgement,
    }), 'utf8');
    expect(programmeCaptureSupervisorClaimSigningPayloadV1(acknowledgement))
      .toEqual(registrationSigningPayload);
    const registrationSignature = sign(
      null, registrationSigningPayload, privateKey,
    ).toString('base64url');
    const registrationEnvelope = {
      schemaVersion: 1, transactionKind: 'programme-capture-v1',
      envelopeKind: 'supervisor-claim-acknowledgement-envelope-v1', acknowledgement,
      signature: { algorithm: 'ed25519', valueBase64Url: registrationSignature },
    } as const;
    const serializedRegistrationEnvelope = `${JSON.stringify(registrationEnvelope, null, 2)}\n`;
    expect(serializeProgrammeCaptureSupervisorClaimEnvelopeV1(registrationEnvelope))
      .toBe(serializedRegistrationEnvelope);

    const independentLeafBytes = Buffer.from(stableJson({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_REGISTRATION_LOG_LEAF_DOMAIN_V1,
      leaf: {
        schemaVersion: 1, transactionKind: 'programme-capture-v1',
        leafKind: 'supervisor-claim-registration-log-leaf-v1',
        acknowledgementDigest: acknowledgement.acknowledgementDigest,
        serializedRegistrationEnvelopeDigest: sha256String(serializedRegistrationEnvelope),
        logId: LOG_ID, logSequence: '3',
      },
    }), 'utf8');
    expect(programmeCaptureSupervisorRegistrationLogLeafBytesV1(
      serializedRegistrationEnvelope,
    )).toEqual(independentLeafBytes);

    const newLeaves = [...oldLeaves, independentLeafBytes];
    const newCheckpoint = checkpointRecord(newLeaves);
    const checkpointSigningPayload = Buffer.from(stableJson({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_SIGNING_DOMAIN_V1,
      checkpoint: newCheckpoint,
    }), 'utf8');
    expect(programmeCaptureSupervisorCheckpointSigningPayloadV1(newCheckpoint))
      .toEqual(checkpointSigningPayload);
    const checkpointSignature = sign(
      null, checkpointSigningPayload, privateKey,
    ).toString('base64url');
    const newCheckpointEnvelope = {
      schemaVersion: 1, transactionKind: 'programme-capture-v1',
      envelopeKind: 'supervisor-log-checkpoint-envelope-v1', checkpoint: newCheckpoint,
      signature: { algorithm: 'ed25519', valueBase64Url: checkpointSignature },
    } as const;
    const serializedNewCheckpointEnvelope =
      `${JSON.stringify(newCheckpointEnvelope, null, 2)}\n`;
    expect(serializeProgrammeCaptureSupervisorCheckpointEnvelopeV1(newCheckpointEnvelope))
      .toBe(serializedNewCheckpointEnvelope);

    const inclusionProofDigests = inclusionProof(2, newLeaves).map(hex);
    const consistencyProofDigests = consistencyProof(2, newLeaves).map(hex);
    const input = {
      claimAuthority: fixedClaimAuthority(), serializedRegistrationEnvelope,
      serializedPriorCheckpoint, serializedNewCheckpointEnvelope,
      trustedPublicKeySpkiDer: publicKeySpki,
      expectedAuthorityKeyFingerprint: KEY_FINGERPRINT,
      expectedSupervisorId: SUPERVISOR_ID, expectedLogId: LOG_ID,
      expectedKeyEpoch: 7, expectedLogSequence: 3,
      trustedPriorCheckpointDigest: priorCheckpoint.checkpointDigest,
      inclusionProofDigests, consistencyProofDigests,
    } as const;
    const validation = await verifyProgrammeCaptureSupervisorRegistrationLogProofV1(input);
    const validationBody = withoutKey(validation, 'validationDigest');
    expect(validation.validationDigest).toBe(domainDigest(
      PROGRAMME_CAPTURE_SUPERVISOR_LOG_PROOF_VALIDATION_DIGEST_DOMAIN_V1,
      'validation', validationBody,
    ));
    const serializedValidation =
      await createProgrammeCaptureSupervisorRegistrationLogProofValidationBlobV1(input);
    expect(await replayProgrammeCaptureSupervisorRegistrationLogProofValidationV1({
      ...input, serializedValidation,
    })).toEqual(validation);

    const vector = {
      acknowledgementDigest: acknowledgement.acknowledgementDigest,
      registrationSignature,
      registrationEnvelopeDigest: sha256String(serializedRegistrationEnvelope),
      registrationEnvelopeByteLength: Buffer.byteLength(serializedRegistrationEnvelope),
      leafPayloadBase64: independentLeafBytes.toString('base64'),
      leafPayloadDigest: sha256(independentLeafBytes),
      merkleLeafDigest: hex(leafHash(independentLeafBytes)),
      priorCheckpointDigest: priorCheckpoint.checkpointDigest,
      priorRootDigest: priorCheckpoint.tree.rootDigest,
      priorCheckpointBytesDigest: sha256String(serializedPriorCheckpoint),
      priorCheckpointByteLength: Buffer.byteLength(serializedPriorCheckpoint),
      newCheckpointDigest: newCheckpoint.checkpointDigest,
      newRootDigest: newCheckpoint.tree.rootDigest,
      checkpointSignature,
      newCheckpointEnvelopeBytesDigest: sha256String(serializedNewCheckpointEnvelope),
      newCheckpointEnvelopeByteLength: Buffer.byteLength(serializedNewCheckpointEnvelope),
      inclusionProofDigests,
      consistencyProofDigests,
      inclusionProofListDigest: proofDigest('registration-inclusion-v1', inclusionProofDigests),
      consistencyProofListDigest: proofDigest('checkpoint-consistency-v1', consistencyProofDigests),
      validationDigest: validation.validationDigest,
      validationBytesDigest: sha256String(serializedValidation),
      validationByteLength: Buffer.byteLength(serializedValidation),
    };
    expect(vector).toEqual(EXPECTED_VECTOR);
  });
});

function checkpointRecord(leaves: readonly Uint8Array[]) {
  const body = {
    schemaVersion: 1, transactionKind: 'programme-capture-v1',
    recordKind: 'supervisor-log-checkpoint-v1', authority: 'development-only-no-promotion',
    supervisor: {
      supervisorId: SUPERVISOR_ID, logId: LOG_ID, keyEpoch: 7,
      authorityKeyFingerprint: KEY_FINGERPRINT,
    },
    tree: { treeSize: String(leaves.length), rootDigest: hex(treeHash(leaves)) },
    verificationScope: 'signed-log-checkpoint-only',
    externalAppendOnlyWitness: false, appendOnlyPersistenceVerified: false,
    rollbackResistance: 'not-proven', forkResistance: 'not-proven',
    globalOrderAuthority: 'not-proven', supervisorAdministration: 'not-attested',
    hostAdmission: 'not-evaluated', runnerLeaseAcquired: false,
    stateTransitionAuthorized: false, attemptStartAuthorized: false,
    captureAuthorized: false,
  } as const;
  return { ...body, checkpointDigest: domainDigest(
    PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_DIGEST_DOMAIN_V1, 'checkpoint', body,
  ) } as const;
}

function fixedClaimAuthority() {
  return {
    authorityRoot: '/test-only/supervisor-log-proof-kat-authority',
    projectAuthorityDigest: '11'.repeat(32),
    runId: 'capture_supervisor_kat_20260829_0001',
    controllerStore: '/test-only/supervisor-log-proof-kat-controller',
    controllerCommit: 'a'.repeat(40),
    taskPath: 'coding-harness/config/programme-v5-acceptance.json',
    expectedRunnerIdentityDigest: '66'.repeat(32),
  };
}

function fixedRequest() {
  return parseProgrammeCaptureSupervisorClaimRequestV1({
    schemaVersion: 1, transactionKind: 'programme-capture-v1',
    requestKind: 'supervisor-claim-registration-request-v1',
    authority: 'development-only-no-promotion',
    claim: {
      schemaVersion: 1, transactionKind: 'programme-capture-v1', recordKind: 'run-claim-v1',
      authority: { projectAuthorityDigest: '11'.repeat(32),
        persistence: 'same-uid-create-new-v1', rollbackResistance: 'not-proven',
        externalAppendOnlyWitness: false },
      runId: 'capture_supervisor_kat_20260829_0001',
      controller: { commit: 'a'.repeat(40), tree: 'b'.repeat(40) },
      task: { path: 'coding-harness/config/programme-v5-acceptance.json',
        gitBlobId: 'c'.repeat(40), sha256: '22'.repeat(32), byteLength: 4096,
        valueDigest: '33'.repeat(32) },
      inputAttestationDigest: '44'.repeat(32),
      runnerProfile: { path: 'crates/sf-bench/config/performance-runner-profile-v1.tsv',
        gitBlobId: 'd'.repeat(40), sha256: '55'.repeat(32), byteLength: 2048 },
      expectedRunnerIdentityDigest: '66'.repeat(32), hostAdmission: 'not-evaluated',
      runnerLeaseAcquired: false, attemptStartAuthorized: false, captureAuthorized: false,
      claimKeyDigest: CLAIM_KEY_DIGEST, claimDigest: CLAIM_DIGEST,
    },
    externalAppendOnlyWitness: false, appendOnlyPersistenceVerified: false,
    rollbackResistance: 'not-proven', supervisorAdministration: 'not-attested',
    hostAdmission: 'not-evaluated', runnerLeaseAcquired: false,
    stateTransitionAuthorized: false, attemptStartAuthorized: false,
    captureAuthorized: false, requestDigest: REQUEST_DIGEST,
  });
}

function domainDigest(domain: string, key: string, value: unknown): string {
  return sha256(Buffer.from(stableJson({ domain, [key]: value }), 'utf8'));
}
function proofDigest(kind: string, proofDigests: readonly string[]): string {
  return sha256(Buffer.from(stableJson({ domain: PATH_DIGEST_DOMAIN, kind, proofDigests }), 'utf8'));
}
function withoutKey(value: unknown, key: string): Record<string, unknown> {
  return Object.fromEntries(Object.entries(value as Record<string, unknown>)
    .filter(([name]) => name !== key));
}
function stableJson(value: unknown): string { return JSON.stringify(sortKeys(value)); }
function sortKeys(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(sortKeys);
  if (value !== null && typeof value === 'object') {
    const record = value as Record<string, unknown>;
    return Object.fromEntries(Object.keys(record).sort().map((key) => [key, sortKeys(record[key])]));
  }
  return value;
}
function hash(bytes: Uint8Array): Buffer { return createHash('sha256').update(bytes).digest(); }
function leafHash(bytes: Uint8Array): Buffer {
  return hash(Buffer.concat([Buffer.from([0]), Buffer.from(bytes)]));
}
function nodeHash(left: Uint8Array, right: Uint8Array): Buffer {
  return hash(Buffer.concat([Buffer.from([1]), Buffer.from(left), Buffer.from(right)]));
}
function treeHash(tree: readonly Uint8Array[]): Buffer {
  if (tree.length === 0) return hash(Buffer.alloc(0));
  if (tree.length === 1) return leafHash(tree[0]);
  const split = largestPowerOfTwoLessThan(tree.length);
  return nodeHash(treeHash(tree.slice(0, split)), treeHash(tree.slice(split)));
}
function inclusionProof(index: number, tree: readonly Uint8Array[]): Buffer[] {
  if (tree.length === 1) return [];
  const split = largestPowerOfTwoLessThan(tree.length);
  return index < split
    ? [...inclusionProof(index, tree.slice(0, split)), treeHash(tree.slice(split))]
    : [...inclusionProof(index - split, tree.slice(split)), treeHash(tree.slice(0, split))];
}
function consistencyProof(oldSize: number, tree: readonly Uint8Array[]): Buffer[] {
  return consistencySubproof(oldSize, tree, true);
}
function consistencySubproof(oldSize: number, tree: readonly Uint8Array[], complete: boolean): Buffer[] {
  if (oldSize === tree.length) return complete ? [] : [treeHash(tree)];
  const split = largestPowerOfTwoLessThan(tree.length);
  return oldSize <= split
    ? [...consistencySubproof(oldSize, tree.slice(0, split), complete), treeHash(tree.slice(split))]
    : [...consistencySubproof(oldSize - split, tree.slice(split), false), treeHash(tree.slice(0, split))];
}
function largestPowerOfTwoLessThan(value: number): number {
  let result = 1; while (result * 2 < value) result *= 2; return result;
}
function sha256(value: Uint8Array): string { return createHash('sha256').update(value).digest('hex'); }
function sha256String(value: string): string { return sha256(Buffer.from(value, 'utf8')); }
function hex(value: Uint8Array): string { return Buffer.from(value).toString('hex'); }
