// SPDX-License-Identifier: MIT

import { canonical } from '@metaharness/harness';
import { createHash } from 'node:crypto';
import { isProxy } from 'node:util/types';
import {
  DEVELOPMENT_AUTHORITY,
  SHA256_PATTERN,
  asClosedRecord,
  asDenseArray,
  asInteger,
  assertExactKeys,
  deepFreeze,
  snapshotUint8Array,
} from './contracts.js';
import { parseTaskOpaqueId } from './acceptance-task-v3.js';
import type { ProgrammeCaptureRunClaimAuthorityInputV1 }
  from './programme-capture-claim-io-v1.js';
import {
  parseProgrammeCaptureSupervisorClaimEnvelopeBlobV1,
  verifyProgrammeCaptureSupervisorClaimAcknowledgementV1,
  type ProgrammeCaptureSupervisorClaimAcknowledgementV1,
} from './programme-capture-supervisor-claim-v1.js';
import {
  parseProgrammeCaptureSupervisorCheckpointBlobV1,
  verifyProgrammeCaptureSupervisorCheckpointEnvelopeV1,
  type ProgrammeCaptureSupervisorCheckpointV1,
} from './programme-capture-supervisor-checkpoint-v1.js';
import {
  programmeCaptureSupervisorMerkleLeafHashV1,
  verifyProgrammeCaptureSupervisorMerkleConsistencyProofV1,
  verifyProgrammeCaptureSupervisorMerkleInclusionProofV1,
} from './programme-capture-supervisor-merkle-v1.js';
import { digestValue } from './receipts.js';

export const PROGRAMME_CAPTURE_SUPERVISOR_REGISTRATION_LOG_LEAF_DOMAIN_V1 =
  'semantic-fabric/programme-capture/supervisor-registration-log-leaf-v1';
export const PROGRAMME_CAPTURE_SUPERVISOR_LOG_PROOF_VALIDATION_DIGEST_DOMAIN_V1 =
  'semantic-fabric/programme-capture/supervisor-registration-log-proof-validation-digest-v1';
const PROOF_DIGEST_DOMAIN =
  'semantic-fabric/programme-capture/supervisor-registration-log-proof-path-digest-v1';
const MAX_PROOF_NODES = 65;

interface LogProofNonAuthorityV1 {
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

export interface ProgrammeCaptureSupervisorRegistrationLogProofInputV1 {
  readonly claimAuthority: ProgrammeCaptureRunClaimAuthorityInputV1;
  readonly serializedRegistrationEnvelope: string;
  readonly serializedPriorCheckpoint: string;
  readonly serializedNewCheckpointEnvelope: string;
  readonly trustedPublicKeySpkiDer: Uint8Array;
  readonly expectedAuthorityKeyFingerprint: string;
  readonly expectedSupervisorId: string;
  readonly expectedLogId: string;
  readonly expectedKeyEpoch: number;
  readonly expectedLogSequence: number;
  readonly trustedPriorCheckpointDigest: string;
  readonly inclusionProofDigests: readonly string[];
  readonly consistencyProofDigests: readonly string[];
}

export interface ProgrammeCaptureSupervisorRegistrationLogProofValidationV1
  extends LogProofNonAuthorityV1 {
  readonly schemaVersion: 1;
  readonly transactionKind: 'programme-capture-v1';
  readonly evidenceKind: 'non-authorizing-supervisor-registration-log-proof-validation-v1';
  readonly authority: typeof DEVELOPMENT_AUTHORITY;
  readonly runId: string;
  readonly projectAuthorityDigest: string;
  readonly claimKeyDigest: string;
  readonly claimDigest: string;
  readonly requestDigest: string;
  readonly acknowledgementDigest: string;
  readonly serializedRegistrationEnvelopeDigest: string;
  readonly supervisor: ProgrammeCaptureSupervisorClaimAcknowledgementV1['supervisor'];
  readonly event: ProgrammeCaptureSupervisorClaimAcknowledgementV1['event'];
  readonly registrationLeaf: Readonly<{
    logSequence: string;
    leafIndex: string;
    payloadDigest: string;
    merkleLeafDigest: string;
  }>;
  readonly priorCheckpoint: Readonly<{
    checkpointDigest: string;
    serializedCheckpointDigest: string;
    treeSize: string;
    rootDigest: string;
  }>;
  readonly newCheckpoint: Readonly<{
    checkpointDigest: string;
    serializedEnvelopeDigest: string;
    treeSize: string;
    rootDigest: string;
  }>;
  readonly proofs: Readonly<{
    inclusionProofDigest: string;
    consistencyProofDigest: string;
  }>;
  readonly verificationScope:
    'registration-signature-new-checkpoint-signature-inclusion-and-consistency-only';
  readonly registrationSignatureVerified: true;
  readonly priorCheckpointSignatureVerified: false;
  readonly newCheckpointSignatureVerified: true;
  readonly registrationInclusionProofVerified: true;
  readonly checkpointConsistencyProofVerified: true;
  readonly trustedPriorCheckpointDigestMatched: true;
  readonly validationDigest: string;
}

export const PROGRAMME_CAPTURE_SUPERVISOR_LOG_PROOF_INPUT_KEYS_V1 = Object.freeze([
  'claimAuthority', 'serializedRegistrationEnvelope', 'serializedPriorCheckpoint',
  'serializedNewCheckpointEnvelope', 'trustedPublicKeySpkiDer',
  'expectedAuthorityKeyFingerprint', 'expectedSupervisorId', 'expectedLogId',
  'expectedKeyEpoch', 'expectedLogSequence', 'trustedPriorCheckpointDigest',
  'inclusionProofDigests', 'consistencyProofDigests',
] as const);

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

export function programmeCaptureSupervisorRegistrationLogLeafBytesV1(
  serializedRegistrationEnvelope: unknown,
): Buffer {
  const envelope = parseProgrammeCaptureSupervisorClaimEnvelopeBlobV1(
    serializedRegistrationEnvelope as string,
  );
  const acknowledgement = envelope.acknowledgement;
  const leaf = {
    schemaVersion: 1 as const,
    transactionKind: 'programme-capture-v1' as const,
    leafKind: 'supervisor-claim-registration-log-leaf-v1' as const,
    acknowledgementDigest: acknowledgement.acknowledgementDigest,
    serializedRegistrationEnvelopeDigest: sha256String(serializedRegistrationEnvelope as string),
    logId: acknowledgement.supervisor.logId,
    logSequence: String(acknowledgement.event.logSequence),
  };
  return Buffer.from(canonical({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_REGISTRATION_LOG_LEAF_DOMAIN_V1, leaf,
  }), 'utf8');
}

export async function verifyProgrammeCaptureSupervisorRegistrationLogProofV1(
  value: unknown,
): Promise<ProgrammeCaptureSupervisorRegistrationLogProofValidationV1> {
  const input = closedRecord(value, 'supervisor registration log proof input');
  assertExactKeys(
    input, PROGRAMME_CAPTURE_SUPERVISOR_LOG_PROOF_INPUT_KEYS_V1,
    'supervisor registration log proof input',
  );
  if (isProxy(input.claimAuthority)) {
    throw new TypeError('supervisor registration log proof claim authority must not be a Proxy');
  }
  const expected = parseExpectedReferences(input);
  const trustedPublicKeySpkiDer = snapshotUint8Array(
    input.trustedPublicKeySpkiDer, 'trusted supervisor log proof Ed25519 SPKI', 1_024,
  );
  const inclusionProofDigests = snapshotProof(
    input.inclusionProofDigests, 'supervisor registration inclusion proof',
  );
  const consistencyProofDigests = snapshotProof(
    input.consistencyProofDigests, 'supervisor checkpoint consistency proof',
  );
  const serializedRegistrationEnvelope = asString(
    input.serializedRegistrationEnvelope, 'serialized supervisor registration envelope',
  );
  const serializedPriorCheckpoint = asString(
    input.serializedPriorCheckpoint, 'serialized trusted prior checkpoint',
  );
  const serializedNewCheckpointEnvelope = asString(
    input.serializedNewCheckpointEnvelope, 'serialized new checkpoint envelope',
  );
  const priorCheckpoint = parseProgrammeCaptureSupervisorCheckpointBlobV1(
    serializedPriorCheckpoint,
  );
  assertCheckpointReferences(priorCheckpoint, expected);
  if (priorCheckpoint.checkpointDigest !== expected.priorCheckpointDigest) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_TRUSTED_PRIOR_CHECKPOINT_MISMATCH');
  }

  const registration = await verifyProgrammeCaptureSupervisorClaimAcknowledgementV1({
    claimAuthority: input.claimAuthority as ProgrammeCaptureRunClaimAuthorityInputV1,
    serializedEnvelope: serializedRegistrationEnvelope,
    trustedPublicKeySpkiDer,
    expectedAuthorityKeyFingerprint: expected.authorityKeyFingerprint,
    expectedSupervisorId: expected.supervisorId,
    expectedLogId: expected.logId,
    expectedKeyEpoch: expected.keyEpoch,
    expectedLogSequence: expected.logSequence,
    expectedPreviousCheckpointDigest: expected.priorCheckpointDigest,
  });
  const newCheckpoint = verifyProgrammeCaptureSupervisorCheckpointEnvelopeV1({
    serializedEnvelope: serializedNewCheckpointEnvelope,
    trustedPublicKeySpkiDer,
    expectedAuthorityKeyFingerprint: expected.authorityKeyFingerprint,
    expectedSupervisorId: expected.supervisorId,
    expectedLogId: expected.logId,
    expectedKeyEpoch: expected.keyEpoch,
  });

  const leafIndex = BigInt(registration.event.logSequence) - 1n;
  const priorTreeSize = BigInt(priorCheckpoint.tree.treeSize);
  const newTreeSize = BigInt(newCheckpoint.tree.treeSize);
  if (leafIndex < priorTreeSize) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_LOG_PROOF_SEQUENCE_PRECEDES_CHECKPOINT');
  }
  if (newTreeSize <= priorTreeSize || leafIndex >= newTreeSize) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_LOG_PROOF_TREE_BOUNDS_INVALID');
  }
  const leafBytes = programmeCaptureSupervisorRegistrationLogLeafBytesV1(
    serializedRegistrationEnvelope,
  );
  verifyProgrammeCaptureSupervisorMerkleInclusionProofV1({
    leafBytes, leafIndex: String(leafIndex), treeSize: String(newTreeSize),
    rootDigest: newCheckpoint.tree.rootDigest, proofDigests: inclusionProofDigests,
  });
  verifyProgrammeCaptureSupervisorMerkleConsistencyProofV1({
    oldTreeSize: String(priorTreeSize), newTreeSize: String(newTreeSize),
    oldRootDigest: priorCheckpoint.tree.rootDigest,
    newRootDigest: newCheckpoint.tree.rootDigest,
    proofDigests: consistencyProofDigests,
  });
  return validationFromVerified({
    registration, priorCheckpoint, newCheckpoint, leafBytes,
    serializedPriorCheckpoint, serializedNewCheckpointEnvelope,
    inclusionProofDigests, consistencyProofDigests,
  });
}

function validationFromVerified(value: Readonly<{
  registration: Awaited<ReturnType<typeof verifyProgrammeCaptureSupervisorClaimAcknowledgementV1>>;
  priorCheckpoint: ProgrammeCaptureSupervisorCheckpointV1;
  newCheckpoint: ProgrammeCaptureSupervisorCheckpointV1;
  leafBytes: Buffer;
  serializedPriorCheckpoint: string;
  serializedNewCheckpointEnvelope: string;
  inclusionProofDigests: string[];
  consistencyProofDigests: string[];
}>): ProgrammeCaptureSupervisorRegistrationLogProofValidationV1 {
  const registration = value.registration;
  const leafIndex = BigInt(registration.event.logSequence) - 1n;
  const body = {
    schemaVersion: 1 as const,
    transactionKind: 'programme-capture-v1' as const,
    evidenceKind: 'non-authorizing-supervisor-registration-log-proof-validation-v1' as const,
    authority: DEVELOPMENT_AUTHORITY,
    runId: registration.runId,
    projectAuthorityDigest: registration.projectAuthorityDigest,
    claimKeyDigest: registration.claimKeyDigest,
    claimDigest: registration.claimDigest,
    requestDigest: registration.requestDigest,
    acknowledgementDigest: registration.acknowledgementDigest,
    serializedRegistrationEnvelopeDigest: registration.serializedEnvelopeDigest,
    supervisor: registration.supervisor,
    event: registration.event,
    registrationLeaf: {
      logSequence: String(registration.event.logSequence),
      leafIndex: String(leafIndex),
      payloadDigest: sha256Bytes(value.leafBytes),
      merkleLeafDigest: programmeCaptureSupervisorMerkleLeafHashV1(value.leafBytes),
    },
    priorCheckpoint: {
      checkpointDigest: value.priorCheckpoint.checkpointDigest,
      serializedCheckpointDigest: sha256String(value.serializedPriorCheckpoint),
      treeSize: value.priorCheckpoint.tree.treeSize,
      rootDigest: value.priorCheckpoint.tree.rootDigest,
    },
    newCheckpoint: {
      checkpointDigest: value.newCheckpoint.checkpointDigest,
      serializedEnvelopeDigest: sha256String(value.serializedNewCheckpointEnvelope),
      treeSize: value.newCheckpoint.tree.treeSize,
      rootDigest: value.newCheckpoint.tree.rootDigest,
    },
    proofs: {
      inclusionProofDigest: proofDigest('registration-inclusion-v1', value.inclusionProofDigests),
      consistencyProofDigest: proofDigest('checkpoint-consistency-v1', value.consistencyProofDigests),
    },
    verificationScope:
      'registration-signature-new-checkpoint-signature-inclusion-and-consistency-only' as const,
    registrationSignatureVerified: true as const,
    priorCheckpointSignatureVerified: false as const,
    newCheckpointSignatureVerified: true as const,
    registrationInclusionProofVerified: true as const,
    checkpointConsistencyProofVerified: true as const,
    trustedPriorCheckpointDigestMatched: true as const,
    ...NON_AUTHORITY,
  };
  return deepFreeze({
    ...body,
    validationDigest: digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_LOG_PROOF_VALIDATION_DIGEST_DOMAIN_V1,
      validation: body,
    }),
  });
}

function parseExpectedReferences(input: Record<string, unknown>) {
  return Object.freeze({
    authorityKeyFingerprint: parseDigest(
      input.expectedAuthorityKeyFingerprint, 'expected supervisor log proof key fingerprint',
    ),
    supervisorId: parseTaskOpaqueId(input.expectedSupervisorId, 'expected supervisor log proof ID'),
    logId: parseTaskOpaqueId(input.expectedLogId, 'expected supervisor log proof log ID'),
    keyEpoch: asInteger(input.expectedKeyEpoch, 'expected supervisor log proof key epoch', 1),
    logSequence: asInteger(input.expectedLogSequence, 'expected registration log sequence', 1),
    priorCheckpointDigest: parseDigest(
      input.trustedPriorCheckpointDigest, 'trusted prior checkpoint digest',
    ),
  });
}

function assertCheckpointReferences(value: ProgrammeCaptureSupervisorCheckpointV1,
  expected: ReturnType<typeof parseExpectedReferences>): void {
  if (value.supervisor.authorityKeyFingerprint !== expected.authorityKeyFingerprint
    || value.supervisor.supervisorId !== expected.supervisorId
    || value.supervisor.logId !== expected.logId
    || value.supervisor.keyEpoch !== expected.keyEpoch) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_PRIOR_CHECKPOINT_AUTHORITY_MISMATCH');
  }
}

function snapshotProof(value: unknown, label: string): string[] {
  if (isProxy(value)) throw new TypeError(`${label} must not be a Proxy`);
  if (Array.isArray(value) && value.length > MAX_PROOF_NODES) {
    throw new RangeError(`${label} exceeds its node bound`);
  }
  const entries = asDenseArray(value, label);
  const result: string[] = [];
  for (let index = 0; index < entries.length; index += 1) {
    const descriptor = Object.getOwnPropertyDescriptor(entries, index);
    result.push(parseDigest(descriptor?.value, `${label}[${index}]`));
  }
  return result;
}

function proofDigest(kind: string, proofDigests: readonly string[]): string {
  return digestValue({ domain: PROOF_DIGEST_DOMAIN, kind, proofDigests });
}

function parseDigest(value: unknown, label: string): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || /^0+$/.test(value)) {
    throw new TypeError(`${label} must be a non-zero lowercase SHA-256 digest`);
  }
  return value;
}

function asString(value: unknown, label: string): string {
  if (typeof value !== 'string') throw new TypeError(`${label} must be a string`);
  return value;
}

function closedRecord(value: unknown, label: string): Record<string, unknown> {
  if (isProxy(value)) throw new TypeError(`${label} must not be a Proxy`);
  return asClosedRecord(value, label);
}

function sha256String(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}

function sha256Bytes(value: Uint8Array): string {
  return createHash('sha256').update(value).digest('hex');
}
