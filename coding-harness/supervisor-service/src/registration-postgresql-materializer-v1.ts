// SPDX-License-Identifier: MIT

import {
  postgresTransparencyLogIdentityDigestV1,
} from './registration-postgresql-authority-configuration-v1.js';
import {
  canonicalDigestHexV1,
  canonicalJsonV1,
  deepFreezeV1,
  equalBytesV1,
  snapshotBytesV1,
  utf8BytesV1,
  verifyEd25519V1,
} from './registration-postgresql-canonical-v1.js';
import {
  parsePostgresMaterializationInputsV1,
  type NormalizedMaterializationInputsV1,
} from './registration-postgresql-materializer-contract-v1.js';
import {
  buildPostgresMaterializationRowsV1,
  type FinalizedPostgresRegistrationMaterializationV1,
} from './registration-postgresql-materializer-rows-v1.js';
import {
  REGISTRATION_CONTENT_TYPE_V2,
  validateStoredRegistrationResultV2,
} from './registration-protocol-v2.js';

const RUN_EVENT_DIGEST_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-run-event-digest-v2';
const RUN_EVENT_SIGNING_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-run-event-signing-v2';
const CONTROLLER_STATE_HEAD_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-controller-state-head-v2';
const SERVICE_RESULT_DIGEST_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-service-result-digest-v2';
const PUBLIC_COMMITMENT_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-public-event-commitment-v2';

const EVENT_NON_AUTHORITY = Object.freeze({
  externalAdministrationVerified: false as const,
  deploymentAttestationVerified: false as const,
  authorityActivationVerified: false as const,
  serviceSignatureVerified: false as const,
  priorGlobalEventVerified: false as const,
  priorSemanticReceiptVerified: false as const,
  controllerStateHeadVerified: false as const,
  rootedClaimVerified: false as const,
  runAdjacencyVerified: false as const,
  resourceHighWaterVerified: false as const,
  resourceFencingVerified: false as const,
  publicCommitmentVerified: false as const,
  checkpointWitnessQuorumVerified: false as const,
  semanticWitnessQuorumVerified: false as const,
  stateTransitionAuthorized: false as const,
  attemptStartAuthorized: false as const,
  captureAuthorized: false as const,
  importAuthorized: false as const,
  promotionAuthorized: false as const,
  releaseAuthorized: false as const,
});

const RESULT_NON_AUTHORITY = Object.freeze({
  externalAdministrationVerified: false as const,
  deploymentAttestationVerified: false as const,
  authorityActivationVerified: false as const,
  fullAuthorityHistoryVerified: false as const,
  projectAuthenticationVerified: false as const,
  serviceSignatureVerified: false as const,
  priorGlobalEventVerified: false as const,
  globalOrderVerified: false as const,
  priorSemanticReceiptVerified: false as const,
  controllerStateHeadVerified: false as const,
  rootedClaimVerified: false as const,
  runAdjacencyVerified: false as const,
  resourceAdjacencyVerified: false as const,
  resourceHighWaterVerified: false as const,
  runnerAdmissionVerified: false as const,
  hostEvidenceVerified: false as const,
  databaseCommitVerified: false as const,
  exactStoredResponseVerified: false as const,
  publicCommitmentVerified: false as const,
  checkpointWitnessQuorumVerified: false as const,
  semanticWitnessQuorumVerified: false as const,
  resourceFencingVerified: false as const,
  stateTransitionAuthorized: false as const,
  attemptStartAuthorized: false as const,
  captureAuthorized: false as const,
  importAuthorized: false as const,
  promotionAuthorized: false as const,
  releaseAuthorized: false as const,
});

declare const PREPARED_IDENTITY: unique symbol;

export interface PreparedPostgresRegistrationMaterializationV1 {
  readonly authority: 'none';
  readonly mutationAuthorized: false;
  readonly signingBytes: Uint8Array;
  readonly [PREPARED_IDENTITY]: true;
}

interface InternalPreparedV1 {
  readonly rawCandidate: Readonly<Record<string, unknown>>;
  readonly rawLockedSnapshots: Readonly<Record<string, unknown>>;
  readonly eventCanonical: string;
  readonly signingBytes: Uint8Array;
}

const PREPARED = new WeakMap<object, InternalPreparedV1>();

export async function preparePostgresRegistrationMaterializationV1(
  candidate: unknown,
  lockedSnapshots: unknown,
): Promise<PreparedPostgresRegistrationMaterializationV1> {
  const inputs = await parsePostgresMaterializationInputsV1(candidate, lockedSnapshots);
  const derived = derivePrepared(inputs);
  const outward = Object.freeze({
    authority: 'none' as const,
    mutationAuthorized: false as const,
    signingBytes: new Uint8Array(derived.signingBytes),
  }) as PreparedPostgresRegistrationMaterializationV1;
  PREPARED.set(outward, {
    rawCandidate: inputs.rawCandidate,
    rawLockedSnapshots: inputs.rawLockedSnapshots,
    eventCanonical: canonicalJsonV1(derived.event),
    signingBytes: new Uint8Array(derived.signingBytes),
  });
  return outward;
}

export async function finalizePostgresRegistrationMaterializationV1(
  prepared: PreparedPostgresRegistrationMaterializationV1,
  signatureValue: unknown,
): Promise<FinalizedPostgresRegistrationMaterializationV1> {
  const preparedObject = prepared !== null
    && (typeof prepared === 'object' || typeof prepared === 'function')
    ? prepared as object : null;
  const internal = preparedObject === null ? undefined : PREPARED.get(preparedObject);
  if (internal === undefined) throw new TypeError('prepared materialization identity is invalid');
  PREPARED.delete(preparedObject!);
  const signature = snapshotBytesV1(
    signatureValue, 'registration materializer signature', 64, 64,
  );
  return finalizeConsumed(internal, signature);
}

async function finalizeConsumed(
  internal: InternalPreparedV1,
  signature: Uint8Array,
): Promise<FinalizedPostgresRegistrationMaterializationV1> {
  const inputs = await parsePostgresMaterializationInputsV1(
    internal.rawCandidate, internal.rawLockedSnapshots,
  );
  const derived = derivePrepared(inputs);
  if (canonicalJsonV1(derived.event) !== internal.eventCanonical
    || !equalBytesV1(derived.signingBytes, internal.signingBytes)) {
    throw new Error('prepared registration materialization changed before finalization');
  }
  verifyEd25519V1(
    inputs.configuration.serviceSigningSpkiDer, internal.signingBytes, signature,
  );

  const signatureBase64Url = Buffer.from(signature).toString('base64url');
  const envelope = deepFreezeV1({
    schemaVersion: 2,
    transactionKind: 'programme-capture-v2',
    envelopeKind: 'supervisor-run-event-envelope-v2',
    event: derived.event,
    signature: { algorithm: 'ed25519', valueBase64Url: signatureBase64Url },
  });
  const serializedEnvelope = `${JSON.stringify(envelope, null, 2)}\n`;
  const result = buildServiceResult(inputs, serializedEnvelope);
  const serializedResponse = `${JSON.stringify(result, null, 2)}\n`;
  const resultingControllerStateHeadDigest = canonicalDigestHexV1({
    domain: CONTROLLER_STATE_HEAD_DOMAIN_V2,
    priorControllerStateHeadDigest: inputs.candidate.priorControllerStateHeadDigest,
    eventDigest: derived.eventDigest,
  });
  const commitment = deepFreezeV1({
    schemaVersion: 2,
    transactionKind: 'programme-capture-v2',
    leafKind: 'programme-capture-event-commitment-v2',
    logIdentityDigest: postgresTransparencyLogIdentityDigestV1(
      inputs.configuration.configuration,
    ),
    eventDigest: derived.eventDigest,
  });
  const publicCommitmentLeafBytes = utf8BytesV1(canonicalJsonV1({
    domain: PUBLIC_COMMITMENT_DOMAIN_V2,
    commitment,
  }), 'public commitment leaf', 1_024);

  await validateStoredRegistrationResultV2(
    serializedResponse, storedExpectation(inputs, derived.eventDigest),
    inputs.candidate.status,
  );
  return buildPostgresMaterializationRowsV1(inputs, {
    eventDigest: derived.eventDigest,
    serializedEnvelope,
    resultingControllerStateHeadDigest,
    serializedResponse,
    publicCommitmentLeafBytes,
  });
}

function derivePrepared(inputs: NormalizedMaterializationInputsV1) {
  const { candidate, configuration } = inputs;
  const body = deepFreezeV1({
    schemaVersion: 2,
    transactionKind: 'programme-capture-v2',
    recordKind: 'supervisor-run-event-v2',
    authority: 'development-only-no-promotion',
    eventKind: candidate.candidateKind,
    authorityHead: candidate.authorityHead,
    service: {
      principalId: configuration.servicePrincipalId,
      keyEpoch: configuration.serviceKeyEpoch,
      keyFingerprint: configuration.serviceKeyFingerprint,
    },
    project: {
      projectAuthorityDigest: candidate.project.projectAuthorityDigest,
      principalId: candidate.project.principalId,
    },
    runId: candidate.request.runId,
    semanticRequestDigest: candidate.request.semanticRequestDigest,
    globalSequence: candidate.globalSequence,
    runSequence: candidate.runSequence,
    previousGlobal: candidate.previousGlobal,
    previousRun: candidate.previousRun,
    priorControllerStateHeadDigest: candidate.priorControllerStateHeadDigest,
    resourceTransition: null,
    body: candidate.body,
    verificationScope: 'service-signed-structure-only',
    ...EVENT_NON_AUTHORITY,
  });
  const eventDigest = canonicalDigestHexV1({
    domain: RUN_EVENT_DIGEST_DOMAIN_V2,
    event: body,
  });
  const event = deepFreezeV1({ ...body, eventDigest });
  const signingBytes = utf8BytesV1(canonicalJsonV1({
    domain: RUN_EVENT_SIGNING_DOMAIN_V2,
    event,
  }), 'registration event signing payload', 131_072);
  return Object.freeze({ event, eventDigest, signingBytes });
}

function buildServiceResult(
  inputs: NormalizedMaterializationInputsV1,
  serializedEnvelope: string,
) {
  const body = deepFreezeV1({
    schemaVersion: 2,
    transactionKind: 'programme-capture-v2',
    resultKind: 'supervisor-registration-result-v2',
    authority: 'development-only-no-promotion',
    semanticRequestDigest: inputs.candidate.request.semanticRequestDigest,
    serializedEventEnvelope: serializedEnvelope,
    responseStatus: inputs.candidate.status,
    responseContentType: REGISTRATION_CONTENT_TYPE_V2,
    verificationScope: 'canonical-envelope-and-semantic-request-digest-binding-only',
    ...RESULT_NON_AUTHORITY,
  });
  return deepFreezeV1({
    ...body,
    resultDigest: canonicalDigestHexV1({
      domain: SERVICE_RESULT_DIGEST_DOMAIN_V2,
      result: body,
    }),
  });
}

function storedExpectation(
  inputs: NormalizedMaterializationInputsV1,
  eventDigest: string,
) {
  const { candidate, runState } = inputs;
  const original = candidate.status === 201 ? {
    requestDigest: candidate.request.semanticRequestDigest,
    eventDigest,
    globalSequence: candidate.globalSequence,
    changedReplayPriorState: null,
  } : registeredExpectation(runState, candidate.priorControllerStateHeadDigest);
  return deepFreezeV1({
    semanticRequestDigest: candidate.request.semanticRequestDigest,
    originalRegistrationRequestDigest: original.requestDigest,
    originalRegistrationEventDigest: original.eventDigest,
    originalRegistrationGlobalSequence: original.globalSequence,
    changedReplayPriorControllerStateHeadDigest: original.changedReplayPriorState,
    projectAuthorityDigest: candidate.project.projectAuthorityDigest,
    principalId: candidate.project.principalId,
    runId: candidate.request.runId,
    authorityHead: candidate.authorityHead,
    priorControllerStateHeadDigest: candidate.priorControllerStateHeadDigest,
    claim: candidate.request.claim,
  });
}

function registeredExpectation(
  run: NormalizedMaterializationInputsV1['runState'],
  priorControllerStateHeadDigest: string,
) {
  if (run.kind !== 'registered') throw new TypeError('registered run snapshot required');
  return Object.freeze({
    requestDigest: run.originalRegistrationRequestDigest,
    eventDigest: run.originalRegistrationEventDigest,
    globalSequence: run.lastRunGlobalSequence,
    changedReplayPriorState: priorControllerStateHeadDigest,
  });
}
