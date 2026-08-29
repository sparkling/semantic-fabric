// SPDX-License-Identifier: MIT

import { createHash, createPrivateKey, createPublicKey, sign } from 'node:crypto';
import { describe, expect, it, vi } from 'vitest';
import {
  createProgrammeCaptureSupervisorClaimAcknowledgementV1,
  parseProgrammeCaptureSupervisorClaimRequestV1,
  programmeCaptureSupervisorClaimSigningPayloadV1,
  serializeProgrammeCaptureSupervisorClaimEnvelopeV1,
} from '../src/programme-capture-supervisor-claim-v1.js';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_DIGEST_DOMAIN_V1,
  parseProgrammeCaptureSupervisorCheckpointV1,
  programmeCaptureSupervisorCheckpointSigningPayloadV1,
  serializeProgrammeCaptureSupervisorCheckpointEnvelopeV1,
  serializeProgrammeCaptureSupervisorCheckpointV1,
} from '../src/programme-capture-supervisor-checkpoint-v1.js';
import {
  programmeCaptureSupervisorRegistrationLogLeafBytesV1,
  verifyProgrammeCaptureSupervisorRegistrationLogProofV1,
} from '../src/programme-capture-supervisor-log-proof-v1.js';
import {
  createProgrammeCaptureSupervisorRegistrationLogProofValidationBlobV1,
  replayProgrammeCaptureSupervisorRegistrationLogProofValidationV1,
} from '../src/programme-capture-supervisor-log-proof-codec-v1.js';
import { digestValue } from '../src/receipts.js';

const claimIoHooks = vi.hoisted(() => ({
  onRead: undefined as undefined | (() => void),
}));
vi.mock('../src/programme-capture-claim-io-v1.js', () => ({
  readProgrammeCaptureRunClaimV1: async () => {
    claimIoHooks.onRead?.();
    return { record: fixedRequest().claim };
  },
}));

const RFC_SEED =
  '9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60';
const RFC_PUBLIC_KEY =
  'd75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a';
const PRIVATE_KEY = createPrivateKey({
  key: Buffer.from(`302e020100300506032b657004220420${RFC_SEED}`, 'hex'),
  format: 'der', type: 'pkcs8',
});
const PUBLIC_KEY_SPKI = createPublicKey(PRIVATE_KEY)
  .export({ format: 'der', type: 'spki' }) as Buffer;
const KEY_FINGERPRINT = sha256(PUBLIC_KEY_SPKI);
const SUPERVISOR_ID = 'supervisor_log_proof_20260829';
const LOG_ID = 'registration_log_proof_20260829';

describe('programme capture V1 non-authorizing supervisor registration log proof', () => {
  it('reverifies registration, checkpoint, inclusion, consistency, and canonical replay', async () => {
    const fixture = logProofFixture();
    const validation = await verifyProgrammeCaptureSupervisorRegistrationLogProofV1(fixture.input);
    expect(validation).toMatchObject({
      evidenceKind: 'non-authorizing-supervisor-registration-log-proof-validation-v1',
      registrationLeaf: { logSequence: '3', leafIndex: '2' },
      priorCheckpoint: { treeSize: '2' }, newCheckpoint: { treeSize: '3' },
      registrationSignatureVerified: true, priorCheckpointSignatureVerified: false,
      newCheckpointSignatureVerified: true,
      registrationInclusionProofVerified: true,
      checkpointConsistencyProofVerified: true,
      trustedPriorCheckpointDigestMatched: true,
      externalAppendOnlyWitness: false, appendOnlyPersistenceVerified: false,
      rollbackResistance: 'not-proven', forkResistance: 'not-proven',
      globalOrderAuthority: 'not-proven', runnerLeaseAcquired: false,
      stateTransitionAuthorized: false, attemptStartAuthorized: false,
      captureAuthorized: false,
    });
    expect(Object.isFrozen(validation)).toBe(true);
    const serializedValidation =
      await createProgrammeCaptureSupervisorRegistrationLogProofValidationBlobV1(fixture.input);
    expect(await replayProgrammeCaptureSupervisorRegistrationLogProofValidationV1({
      ...fixture.input, serializedValidation,
    })).toEqual(validation);
    await expect(replayProgrammeCaptureSupervisorRegistrationLogProofValidationV1({
      ...fixture.input, serializedValidation: JSON.stringify(validation),
    })).rejects.toThrow();
  });

  it('rejects false anchors, proof mutations, signatures, and sequence/tree contradictions', async () => {
    const fixture = logProofFixture();
    const bitFlip = [...fixture.input.inclusionProofDigests];
    bitFlip[0] = `${bitFlip[0][0] === '0' ? '1' : '0'}${bitFlip[0].slice(1)}`;
    const corruptedCheckpoint = JSON.parse(fixture.input.serializedNewCheckpointEnvelope);
    corruptedCheckpoint.signature.valueBase64Url = `${
      corruptedCheckpoint.signature.valueBase64Url.startsWith('A') ? 'B' : 'A'
    }${corruptedCheckpoint.signature.valueBase64Url.slice(1)}`;
    for (const override of [
      { trustedPriorCheckpointDigest: '9'.repeat(64) },
      { expectedAuthorityKeyFingerprint: '9'.repeat(64) },
      { expectedSupervisorId: 'different_supervisor_20260829' },
      { expectedLogId: 'different_registration_log_20260829' },
      { expectedKeyEpoch: 8 },
      { inclusionProofDigests: fixture.input.inclusionProofDigests.slice(1) },
      { inclusionProofDigests: bitFlip },
      { consistencyProofDigests: [] },
      { serializedNewCheckpointEnvelope: `${JSON.stringify(corruptedCheckpoint, null, 2)}\n` },
    ]) {
      await expect(verifyProgrammeCaptureSupervisorRegistrationLogProofV1({
        ...fixture.input, ...override,
      })).rejects.toThrow();
    }
    await expect(verifyProgrammeCaptureSupervisorRegistrationLogProofV1(
      logProofFixture({ logSequence: 2 }).input,
    )).rejects.toThrow(/SEQUENCE/);
    await expect(verifyProgrammeCaptureSupervisorRegistrationLogProofV1(
      logProofFixture({ omitRegistrationFromNewTree: true }).input,
    )).rejects.toThrow(/TREE/);
  });

  it('snapshots key and proof inputs before rooted async reads and rejects proxies', async () => {
    const fixture = logProofFixture();
    const baseline = await verifyProgrammeCaptureSupervisorRegistrationLogProofV1(fixture.input);
    const inclusionProofDigests = [...fixture.input.inclusionProofDigests];
    const consistencyProofDigests = [...fixture.input.consistencyProofDigests];
    const trustedPublicKeySpkiDer = Buffer.from(fixture.input.trustedPublicKeySpkiDer);
    let mutated = false;
    claimIoHooks.onRead = () => {
      if (mutated) return;
      mutated = true;
      inclusionProofDigests[0] = '9'.repeat(64);
      consistencyProofDigests[0] = '8'.repeat(64);
      trustedPublicKeySpkiDer[0] ^= 1;
    };
    try {
      expect(await verifyProgrammeCaptureSupervisorRegistrationLogProofV1({
        ...fixture.input, inclusionProofDigests, consistencyProofDigests,
        trustedPublicKeySpkiDer,
      })).toEqual(baseline);
    } finally {
      claimIoHooks.onRead = undefined;
    }
    let trapCalls = 0;
    const traps: ProxyHandler<object> = {
      get() { trapCalls += 1; throw new Error('proxy get trap executed'); },
      ownKeys() { trapCalls += 1; throw new Error('proxy ownKeys trap executed'); },
      getOwnPropertyDescriptor() {
        trapCalls += 1; throw new Error('proxy descriptor trap executed');
      },
    };
    await expect(verifyProgrammeCaptureSupervisorRegistrationLogProofV1({
      ...fixture.input,
      inclusionProofDigests: new Proxy([...fixture.input.inclusionProofDigests], traps),
    })).rejects.toThrow(/Proxy/);
    await expect(verifyProgrammeCaptureSupervisorRegistrationLogProofV1({
      ...fixture.input,
      claimAuthority: new Proxy({ ...fixture.input.claimAuthority }, traps),
    })).rejects.toThrow(/Proxy/);
    expect(trapCalls).toBe(0);
  });

  it('admits two valid extensions while explicitly leaving split-view resistance unproved', async () => {
    const left = logProofFixture({ suffix: Buffer.from('left') });
    const right = logProofFixture({ suffix: Buffer.from('right') });
    const [leftView, rightView] = await Promise.all([
      verifyProgrammeCaptureSupervisorRegistrationLogProofV1(left.input),
      verifyProgrammeCaptureSupervisorRegistrationLogProofV1(right.input),
    ]);
    expect(leftView.newCheckpoint.treeSize).toBe('4');
    expect(rightView.newCheckpoint.treeSize).toBe('4');
    expect(leftView.newCheckpoint.rootDigest).not.toBe(rightView.newCheckpoint.rootDigest);
    for (const view of [leftView, rightView]) {
      expect(view.forkResistance).toBe('not-proven');
      expect(view.globalOrderAuthority).toBe('not-proven');
      expect(view.appendOnlyPersistenceVerified).toBe(false);
    }
  });
});

function logProofFixture(options: Readonly<{
  logSequence?: number; suffix?: Buffer; omitRegistrationFromNewTree?: boolean;
}> = {}) {
  const oldLeaves = [Buffer.from('old-0'), Buffer.from('old-1')];
  const priorCheckpoint = checkpointRecord(oldLeaves);
  const serializedPriorCheckpoint =
    serializeProgrammeCaptureSupervisorCheckpointV1(priorCheckpoint);
  const request = fixedRequest();
  const acknowledgement = createProgrammeCaptureSupervisorClaimAcknowledgementV1({
    request, supervisorId: SUPERVISOR_ID, logId: LOG_ID, keyEpoch: 7,
    authorityKeyFingerprint: KEY_FINGERPRINT,
    logSequence: options.logSequence ?? 3,
    previousCheckpointDigest: priorCheckpoint.checkpointDigest,
  });
  const registrationEnvelope = {
    schemaVersion: 1, transactionKind: 'programme-capture-v1',
    envelopeKind: 'supervisor-claim-acknowledgement-envelope-v1', acknowledgement,
    signature: {
      algorithm: 'ed25519',
      valueBase64Url: sign(
        null, programmeCaptureSupervisorClaimSigningPayloadV1(acknowledgement), PRIVATE_KEY,
      ).toString('base64url'),
    },
  } as const;
  const serializedRegistrationEnvelope =
    serializeProgrammeCaptureSupervisorClaimEnvelopeV1(registrationEnvelope);
  const registrationLeaf = programmeCaptureSupervisorRegistrationLogLeafBytesV1(
    serializedRegistrationEnvelope,
  );
  const newLeaves = options.omitRegistrationFromNewTree
    ? oldLeaves : [...oldLeaves, registrationLeaf];
  if (options.suffix) newLeaves.push(options.suffix);
  const newCheckpoint = checkpointRecord(newLeaves);
  const serializedNewCheckpointEnvelope = signedCheckpointEnvelope(newCheckpoint);
  const physicalLeafIndex = oldLeaves.length;
  return {
    input: {
      claimAuthority: fixedClaimAuthority(), serializedRegistrationEnvelope,
      serializedPriorCheckpoint, serializedNewCheckpointEnvelope,
      trustedPublicKeySpkiDer: PUBLIC_KEY_SPKI,
      expectedAuthorityKeyFingerprint: KEY_FINGERPRINT,
      expectedSupervisorId: SUPERVISOR_ID, expectedLogId: LOG_ID,
      expectedKeyEpoch: 7, expectedLogSequence: options.logSequence ?? 3,
      trustedPriorCheckpointDigest: priorCheckpoint.checkpointDigest,
      inclusionProofDigests: options.omitRegistrationFromNewTree ? []
        : inclusionProof(physicalLeafIndex, newLeaves).map(hex),
      consistencyProofDigests: options.omitRegistrationFromNewTree ? []
        : consistencyProof(oldLeaves.length, newLeaves).map(hex),
    },
  };
}

function checkpointRecord(leaves: readonly Uint8Array[]) {
  const body = {
    schemaVersion: 1, transactionKind: 'programme-capture-v1',
    recordKind: 'supervisor-log-checkpoint-v1', authority: 'development-only-no-promotion',
    supervisor: {
      supervisorId: SUPERVISOR_ID, logId: LOG_ID, keyEpoch: 7,
      authorityKeyFingerprint: KEY_FINGERPRINT,
    },
    tree: { treeSize: String(leaves.length), rootDigest: treeHash(leaves).toString('hex') },
    verificationScope: 'signed-log-checkpoint-only',
    externalAppendOnlyWitness: false, appendOnlyPersistenceVerified: false,
    rollbackResistance: 'not-proven', forkResistance: 'not-proven',
    globalOrderAuthority: 'not-proven', supervisorAdministration: 'not-attested',
    hostAdmission: 'not-evaluated', runnerLeaseAcquired: false,
    stateTransitionAuthorized: false, attemptStartAuthorized: false,
    captureAuthorized: false,
  } as const;
  return parseProgrammeCaptureSupervisorCheckpointV1({
    ...body, checkpointDigest: digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_CHECKPOINT_DIGEST_DOMAIN_V1, checkpoint: body,
    }),
  });
}

function signedCheckpointEnvelope(checkpoint: ReturnType<typeof checkpointRecord>): string {
  return serializeProgrammeCaptureSupervisorCheckpointEnvelopeV1({
    schemaVersion: 1, transactionKind: 'programme-capture-v1',
    envelopeKind: 'supervisor-log-checkpoint-envelope-v1', checkpoint,
    signature: { algorithm: 'ed25519', valueBase64Url: sign(
      null, programmeCaptureSupervisorCheckpointSigningPayloadV1(checkpoint), PRIVATE_KEY,
    ).toString('base64url') },
  });
}

function fixedClaimAuthority() {
  return {
    authorityRoot: '/test-only/supervisor-log-proof-authority',
    projectAuthorityDigest: '11'.repeat(32),
    runId: 'capture_supervisor_kat_20260829_0001',
    controllerStore: '/test-only/supervisor-log-proof-controller',
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
      claimKeyDigest: '6ae0f3b6b43d2a03610045c827d104ecba2db3f2f9d1b21ff914d01d046b6f3d',
      claimDigest: '5aef5ddc2398c7d2efc735ee10400908b5fed1ca528591722d8074e90ebdc2e1',
    },
    externalAppendOnlyWitness: false, appendOnlyPersistenceVerified: false,
    rollbackResistance: 'not-proven', supervisorAdministration: 'not-attested',
    hostAdmission: 'not-evaluated', runnerLeaseAcquired: false,
    stateTransitionAuthorized: false, attemptStartAuthorized: false,
    captureAuthorized: false,
    requestDigest: '4fcc54b6dcbb9ffc46a2c73a18db24619a03a750b4bfc4c5d444255d21202ac0',
  });
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
function hex(value: Uint8Array): string { return Buffer.from(value).toString('hex'); }

expect(PUBLIC_KEY_SPKI.toString('hex'))
  .toBe(`302a300506032b6570032100${RFC_PUBLIC_KEY}`);
