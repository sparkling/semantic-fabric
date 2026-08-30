// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';

export const DIGEST = Object.freeze({
  project: '9ba3490c3ff9becc86163de81360b6aa1ea64e9e2f7098ec72daefa5a66b77bf',
  configuration: '90ce861103ea0bcacebc37ee97f502fa5080ab5e9ab1b542c15393b96fac4c02',
  head: '80b825d44f308c1ef66d92129f2c34e5b05e88ee839ec7ca91bf2301f2013146',
  request: 'd836ed3af320f6840976fd070d994c82712b7c7920688049bd730d7b3abec14d',
  requestBytes: '9dc587e8b2a3b2210ec765a2bfb10f359a31ab5df69d1abda32737515636d97c',
  claimKey: '690ef8500613c6534704cf32aaf5bb43510e3343126975e043df739469c1649a',
  claim: '1e064130a00dd5595097fb049e6347200b7eba90ff58140cc8e9d7f21431fb61',
  rootedClaim: '9bbc363926435939074ca86c3fb372896ea1f9858624cc852d03a8685ea3e3d6',
  priorController: '641794e570acb0b64cdeab80dd592cf28735a686c95a8cdb82fbae615c689da1',
} as const);

export const CANONICAL_REQUEST = `{
  "schemaVersion": 2,
  "transactionKind": "programme-capture-v2",
  "requestKind": "supervisor-claim-registration-request-v2",
  "operationKind": "claim-registered-v2",
  "authority": "development-only-no-promotion",
  "authorityHead": {
    "configurationEpoch": "0",
    "configurationDigest": "${DIGEST.configuration}",
    "headDigest": "${DIGEST.head}"
  },
  "project": {
    "projectAuthorityDigest": "${DIGEST.project}",
    "principalId": "project_client_20260829"
  },
  "runId": "capture_run_20260829",
  "expectedRegistration": {
    "priorControllerStateHeadDigest": "${DIGEST.priorController}"
  },
  "claim": {
    "claimKeyDigest": "${DIGEST.claimKey}",
    "claimDigest": "${DIGEST.claim}",
    "rootedClaimValidationDigest": "${DIGEST.rootedClaim}"
  },
  "verificationScope": "canonical-registration-intent-only",
  "externalAdministrationVerified": false,
  "deploymentAttestationVerified": false,
  "authorityActivationVerified": false,
  "projectAuthenticationVerified": false,
  "serviceSignatureVerified": false,
  "priorGlobalEventVerified": false,
  "globalOrderVerified": false,
  "priorSemanticReceiptVerified": false,
  "controllerStateHeadVerified": false,
  "rootedClaimVerified": false,
  "runAdjacencyVerified": false,
  "stateTransitionAuthorized": false,
  "attemptStartAuthorized": false,
  "captureAuthorized": false,
  "importAuthorized": false,
  "promotionAuthorized": false,
  "releaseAuthorized": false,
  "semanticRequestDigest": "${DIGEST.request}"
}
`;

export const PROJECT = Object.freeze({
  projectAuthorityDigest: DIGEST.project,
  principalId: 'project_client_20260829',
  authenticationPolicyDigest: sha256Text('authentication-policy'),
});

export const ACTIVE_HEAD = Object.freeze({
  kind: 'active' as const,
  project: PROJECT,
  authorityHead: Object.freeze({
    configurationEpoch: '0',
    configurationDigest: DIGEST.configuration,
    headDigest: DIGEST.head,
  }),
  expectedNextGlobalSequence: '1',
  requiredPredecessor: Object.freeze({
    kind: 'authority-genesis' as const,
    eventDigest: null,
  }),
});

export const READY_RECEIPT = Object.freeze({
  kind: 'ready' as const,
  previousGlobal: Object.freeze({
    kind: 'authority-genesis' as const,
    eventDigest: null,
    semanticReceiptDigest: sha256Text('semantic-genesis'),
  }),
});

export const ACTIVE_SEMANTIC_HEAD = Object.freeze({
  ...ACTIVE_HEAD,
  expectedNextGlobalSequence: '2',
  requiredPredecessor: Object.freeze({
    kind: 'semantic-event' as const,
    eventDigest: sha256Text('original-event'),
  }),
});

export const READY_SEMANTIC_RECEIPT = Object.freeze({
  kind: 'ready' as const,
  previousGlobal: Object.freeze({
    ...ACTIVE_SEMANTIC_HEAD.requiredPredecessor,
    semanticReceiptDigest: sha256Text('current-global-semantic-receipt'),
  }),
});

export const REGISTERED_RUN = Object.freeze({
  kind: 'registered' as const,
  projectAuthorityDigest: DIGEST.project,
  runId: 'capture_run_20260829',
  originalRegistrationRequestDigest: sha256Text('original-request'),
  originalRegistrationRequestSha256: sha256Text('original-request-bytes'),
  registrationEventDigest: sha256Text('original-event'),
  lastRunEventDigest: sha256Text('original-event'),
  lastRunGlobalSequence: '1',
  currentControllerStateHeadDigest: sha256Text('post-registration-controller-state'),
  lastRunSequence: '0',
});

export function sha256Text(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}

export function canonical(value: unknown): string {
  if (value === null || typeof value !== 'object') return JSON.stringify(value);
  if (Array.isArray(value)) return `[${value.map(canonical).join(',')}]`;
  const record = value as Record<string, unknown>;
  return `{${Object.keys(record).sort().map(
    (key) => `${JSON.stringify(key)}:${canonical(record[key])}`,
  ).join(',')}}`;
}

export function canonicalPretty(value: unknown): string {
  return `${JSON.stringify(value, null, 2)}\n`;
}

const EVENT_NON_AUTHORITY = Object.freeze({
  externalAdministrationVerified: false,
  deploymentAttestationVerified: false,
  authorityActivationVerified: false,
  serviceSignatureVerified: false,
  priorGlobalEventVerified: false,
  priorSemanticReceiptVerified: false,
  controllerStateHeadVerified: false,
  rootedClaimVerified: false,
  runAdjacencyVerified: false,
  resourceHighWaterVerified: false,
  resourceFencingVerified: false,
  publicCommitmentVerified: false,
  checkpointWitnessQuorumVerified: false,
  semanticWitnessQuorumVerified: false,
  stateTransitionAuthorized: false,
  attemptStartAuthorized: false,
  captureAuthorized: false,
  importAuthorized: false,
  promotionAuthorized: false,
  releaseAuthorized: false,
});

export function registrationEnvelope(status: 201 | 409): string {
  const changed = status === 409;
  const previousEventDigest = REGISTERED_RUN.registrationEventDigest;
  const eventBody = {
    schemaVersion: 2,
    transactionKind: 'programme-capture-v2',
    recordKind: 'supervisor-run-event-v2',
    authority: 'development-only-no-promotion',
    eventKind: changed ? 'capture-run-terminal-v2' : 'claim-registered-v2',
    authorityHead: ACTIVE_HEAD.authorityHead,
    service: {
      principalId: 'supervisor_service_20260830',
      keyEpoch: '1',
      keyFingerprint: sha256Text('supervisor-service-key'),
    },
    project: {
      projectAuthorityDigest: PROJECT.projectAuthorityDigest,
      principalId: PROJECT.principalId,
    },
    runId: 'capture_run_20260829',
    semanticRequestDigest: DIGEST.request,
    globalSequence: changed ? '2' : '1',
    runSequence: changed ? '1' : '0',
    previousGlobal: changed ? {
      kind: 'semantic-event',
      eventDigest: previousEventDigest,
      semanticReceiptDigest: sha256Text('changed-replay-semantic-receipt'),
    } : READY_RECEIPT.previousGlobal,
    previousRun: changed ? {
      kind: 'run-event', eventDigest: previousEventDigest,
    } : { kind: 'run-genesis', eventDigest: null },
    priorControllerStateHeadDigest: changed
      ? REGISTERED_RUN.currentControllerStateHeadDigest : DIGEST.priorController,
    resourceTransition: null,
    body: changed ? {
      terminalStage: 'registration',
      outcomeCode: 'registration-changed-replay-v2',
      registrationEventDigest: previousEventDigest,
      outcomeEvidenceDigest: sha256Text(canonical({
        domain:
          'semantic-fabric/programme-capture/supervisor-registration-changed-replay-evidence-v2',
        originalRegistrationRequestDigest: REGISTERED_RUN.originalRegistrationRequestDigest,
        originalRegistrationEventDigest: previousEventDigest,
        changedRegistrationRequestDigest: DIGEST.request,
        project: {
          projectAuthorityDigest: PROJECT.projectAuthorityDigest,
          principalId: PROJECT.principalId,
        },
        authorityHead: ACTIVE_HEAD.authorityHead,
      })),
      leaseEventDigest: null,
      leaseId: null,
      fence: null,
      resourceDisposition: null,
      attemptId: null,
      captureRecordDigest: null,
      outputEnvelopeDigest: null,
      cleanupEvidenceDigest: null,
    } : {
      claimKeyDigest: DIGEST.claimKey,
      claimDigest: DIGEST.claim,
      rootedClaimValidationDigest: DIGEST.rootedClaim,
    },
    verificationScope: 'service-signed-structure-only',
    ...EVENT_NON_AUTHORITY,
  };
  const event = {
    ...eventBody,
    eventDigest: sha256Text(canonical({
      domain: 'semantic-fabric/programme-capture/supervisor-run-event-digest-v2',
      event: eventBody,
    })),
  };
  return canonicalPretty({
    schemaVersion: 2,
    transactionKind: 'programme-capture-v2',
    envelopeKind: 'supervisor-run-event-envelope-v2',
    event,
    signature: { algorithm: 'ed25519', valueBase64Url: 'A'.repeat(86) },
  });
}

export function coherentlyMutatedRegistrationEnvelope(
  status: 201 | 409,
  mutate: (event: Record<string, any>) => void,
): string {
  const envelope = JSON.parse(registrationEnvelope(status)) as Record<string, any>;
  const event = envelope.event as Record<string, any>;
  mutate(event);
  const { eventDigest: _ignored, ...eventBody } = event;
  event.eventDigest = sha256Text(canonical({
    domain: 'semantic-fabric/programme-capture/supervisor-run-event-digest-v2',
    event: eventBody,
  }));
  return canonicalPretty(envelope);
}

export function exactStoredResult(
  status: 201 | 409,
  overrides: Record<string, unknown> = {},
  rowOverrides: Record<string, unknown> = {},
) {
  const baselineEnvelope = registrationEnvelope(status);
  const serializedEventEnvelope = typeof overrides.serializedEventEnvelope === 'string'
    ? overrides.serializedEventEnvelope : baselineEnvelope;
  let event = JSON.parse(baselineEnvelope).event as Record<string, unknown>;
  try { event = JSON.parse(serializedEventEnvelope).event as Record<string, unknown>; }
  catch { /* malformed-envelope fixtures retain independent baseline provenance */ }
  const body = {
    schemaVersion: 2,
    transactionKind: 'programme-capture-v2',
    resultKind: 'supervisor-registration-result-v2',
    authority: 'development-only-no-promotion',
    semanticRequestDigest: DIGEST.request,
    serializedEventEnvelope,
    responseStatus: status,
    responseContentType: 'application/json; charset=utf-8',
    verificationScope: 'canonical-envelope-and-semantic-request-digest-binding-only',
    ...Object.fromEntries([
      'externalAdministrationVerified', 'deploymentAttestationVerified',
      'authorityActivationVerified', 'fullAuthorityHistoryVerified',
      'projectAuthenticationVerified', 'serviceSignatureVerified',
      'priorGlobalEventVerified', 'globalOrderVerified',
      'priorSemanticReceiptVerified', 'controllerStateHeadVerified',
      'rootedClaimVerified', 'runAdjacencyVerified', 'resourceAdjacencyVerified',
      'resourceHighWaterVerified', 'runnerAdmissionVerified', 'hostEvidenceVerified',
      'databaseCommitVerified', 'exactStoredResponseVerified',
      'publicCommitmentVerified', 'checkpointWitnessQuorumVerified',
      'semanticWitnessQuorumVerified', 'resourceFencingVerified',
      'stateTransitionAuthorized', 'attemptStartAuthorized', 'captureAuthorized',
      'importAuthorized', 'promotionAuthorized', 'releaseAuthorized',
    ].map((key) => [key, false])),
    ...overrides,
  };
  const result = {
    ...body,
    resultDigest: sha256Text(canonical({
      domain: 'semantic-fabric/programme-capture/supervisor-service-result-digest-v2',
      result: body,
    })),
  };
  const serializedResponse = canonicalPretty(result);
  return {
    kind: 'found' as const,
    row: {
      projectAuthorityDigest: DIGEST.project,
      semanticRequestDigest: DIGEST.request,
      originalRegistrationRequestDigest: status === 201
        ? DIGEST.request : REGISTERED_RUN.originalRegistrationRequestDigest,
      originalRegistrationEventDigest: status === 201
        ? String(event.eventDigest) : REGISTERED_RUN.registrationEventDigest,
      originalRegistrationGlobalSequence: '1',
      changedReplayPriorControllerStateHeadDigest: status === 201
        ? null : REGISTERED_RUN.currentControllerStateHeadDigest,
      serializedRequest: CANONICAL_REQUEST,
      serializedRequestSha256: DIGEST.requestBytes,
      responseStatus: status,
      responseContentType: 'application/json; charset=utf-8',
      serializedResponse,
      serializedResponseSha256: sha256Text(serializedResponse),
      ...rowOverrides,
    },
  };
}
