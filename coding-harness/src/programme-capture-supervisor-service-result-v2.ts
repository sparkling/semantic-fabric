// SPDX-License-Identifier: MIT

import { isProxy } from 'node:util/types';
import {
  DEVELOPMENT_AUTHORITY,
  asClosedRecord,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import {
  parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2,
} from './programme-capture-supervisor-run-event-codec-v2.js';
import {
  parseRunEventDigestV2,
  type ProgrammeCaptureSupervisorRunTerminalBodyV2,
} from
  './programme-capture-supervisor-run-event-contracts-v2.js';
import { digestValue } from './receipts.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export const PROGRAMME_CAPTURE_SUPERVISOR_SERVICE_RESULT_MAX_BYTES_V2 = 196_608;
export const PROGRAMME_CAPTURE_SUPERVISOR_SERVICE_RESULT_DIGEST_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-service-result-digest-v2';
export const PROGRAMME_CAPTURE_SUPERVISOR_SERVICE_RESULT_CONTENT_TYPE_V2 =
  'application/json; charset=utf-8' as const;

const NON_AUTHORITY = Object.freeze({
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
const NON_AUTHORITY_KEYS = Object.freeze(Object.keys(NON_AUTHORITY));

export interface ProgrammeCaptureSupervisorServiceResultV2 {
  readonly schemaVersion: 2;
  readonly transactionKind: 'programme-capture-v2';
  readonly resultKind: 'supervisor-registration-result-v2';
  readonly authority: typeof DEVELOPMENT_AUTHORITY;
  readonly semanticRequestDigest: string;
  readonly serializedEventEnvelope: string;
  readonly responseStatus: 201 | 409;
  readonly responseContentType: typeof PROGRAMME_CAPTURE_SUPERVISOR_SERVICE_RESULT_CONTENT_TYPE_V2;
  readonly verificationScope: 'canonical-envelope-and-semantic-request-digest-binding-only';
  readonly externalAdministrationVerified: false;
  readonly deploymentAttestationVerified: false;
  readonly authorityActivationVerified: false;
  readonly fullAuthorityHistoryVerified: false;
  readonly projectAuthenticationVerified: false;
  readonly serviceSignatureVerified: false;
  readonly priorGlobalEventVerified: false;
  readonly globalOrderVerified: false;
  readonly priorSemanticReceiptVerified: false;
  readonly controllerStateHeadVerified: false;
  readonly rootedClaimVerified: false;
  readonly runAdjacencyVerified: false;
  readonly resourceAdjacencyVerified: false;
  readonly resourceHighWaterVerified: false;
  readonly runnerAdmissionVerified: false;
  readonly hostEvidenceVerified: false;
  readonly databaseCommitVerified: false;
  readonly exactStoredResponseVerified: false;
  readonly publicCommitmentVerified: false;
  readonly checkpointWitnessQuorumVerified: false;
  readonly semanticWitnessQuorumVerified: false;
  readonly resourceFencingVerified: false;
  readonly stateTransitionAuthorized: false;
  readonly attemptStartAuthorized: false;
  readonly captureAuthorized: false;
  readonly importAuthorized: false;
  readonly promotionAuthorized: false;
  readonly releaseAuthorized: false;
  readonly resultDigest: string;
}

export function buildProgrammeCaptureSupervisorServiceResultV2(
  value: unknown,
): ProgrammeCaptureSupervisorServiceResultV2 {
  const input = closed(value, 'supervisor service-result construction input');
  assertExactKeys(
    input, ['semanticRequestDigest', 'serializedEventEnvelope'],
    'supervisor service-result construction input',
  );
  const body = normalizedResultBody(input);
  return parseProgrammeCaptureSupervisorServiceResultV2({
    ...body,
    resultDigest: digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_SERVICE_RESULT_DIGEST_DOMAIN_V2,
      result: body,
    }),
  });
}

export function parseProgrammeCaptureSupervisorServiceResultV2(
  value: unknown,
): ProgrammeCaptureSupervisorServiceResultV2 {
  const input = closed(value, 'supervisor service result');
  assertExactKeys(input, [
    'schemaVersion', 'transactionKind', 'resultKind', 'authority',
    'semanticRequestDigest', 'serializedEventEnvelope', 'responseStatus',
    'responseContentType', 'verificationScope',
    ...NON_AUTHORITY_KEYS, 'resultDigest',
  ], 'supervisor service result');
  assertIdentity(input);
  const body = normalizedResultBody(input);
  const resultDigest = parseRunEventDigestV2(input.resultDigest, 'supervisor result digest');
  if (resultDigest !== digestValue({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_SERVICE_RESULT_DIGEST_DOMAIN_V2,
    result: body,
  })) throw new Error('HARNESS_CAPTURE_SUPERVISOR_RESULT_DIGEST_MISMATCH');
  return deepFreeze({ ...body, resultDigest });
}

export function serializeProgrammeCaptureSupervisorServiceResultV2(value: unknown): string {
  return `${JSON.stringify(parseProgrammeCaptureSupervisorServiceResultV2(value), null, 2)}\n`;
}

export function parseProgrammeCaptureSupervisorServiceResultBlobV2(
  serialized: string,
): ProgrammeCaptureSupervisorServiceResultV2 {
  assertCanonicalBlob(serialized);
  const result = parseProgrammeCaptureSupervisorServiceResultV2(
    parseJsonWithoutDuplicateKeys(serialized, 'supervisor service result'),
  );
  if (serializeProgrammeCaptureSupervisorServiceResultV2(result) !== serialized) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_RESULT_CANONICAL_REQUIRED');
  }
  return result;
}

function normalizedResultBody(input: Record<string, unknown>) {
  const semanticRequestDigest = parseRunEventDigestV2(
    input.semanticRequestDigest, 'supervisor result semantic request digest',
  );
  if (typeof input.serializedEventEnvelope !== 'string') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RESULT_ENVELOPE_REQUIRED');
  }
  const envelope = parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2(
    input.serializedEventEnvelope,
  );
  if (envelope.event.semanticRequestDigest !== semanticRequestDigest) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_RESULT_REQUEST_BINDING_MISMATCH');
  }
  const responseStatus = registrationResponseStatus(envelope.event);
  if (input.responseStatus !== undefined && input.responseStatus !== responseStatus) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_RESULT_STATUS_MISMATCH');
  }
  if (input.responseContentType !== undefined
    && input.responseContentType
      !== PROGRAMME_CAPTURE_SUPERVISOR_SERVICE_RESULT_CONTENT_TYPE_V2) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_RESULT_CONTENT_TYPE_MISMATCH');
  }
  return {
    schemaVersion: 2 as const,
    transactionKind: 'programme-capture-v2' as const,
    resultKind: 'supervisor-registration-result-v2' as const,
    authority: DEVELOPMENT_AUTHORITY,
    semanticRequestDigest,
    serializedEventEnvelope: input.serializedEventEnvelope,
    responseStatus,
    responseContentType: PROGRAMME_CAPTURE_SUPERVISOR_SERVICE_RESULT_CONTENT_TYPE_V2,
    verificationScope:
      'canonical-envelope-and-semantic-request-digest-binding-only' as const,
    ...NON_AUTHORITY,
  };
}

function registrationResponseStatus(
  event: ReturnType<typeof parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2>['event'],
): 201 | 409 {
  if (event.eventKind === 'claim-registered-v2') return 201;
  if (event.eventKind === 'capture-run-terminal-v2') {
    const body = event.body as ProgrammeCaptureSupervisorRunTerminalBodyV2;
    if (body.terminalStage === 'registration'
      && body.outcomeCode === 'registration-changed-replay-v2'
      && event.resourceTransition === null) return 409;
  }
  throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_REGISTRATION_RESULT_EVENT_INVALID');
}

function assertIdentity(input: Record<string, unknown>): void {
  if (input.schemaVersion !== 2 || input.transactionKind !== 'programme-capture-v2'
    || input.resultKind !== 'supervisor-registration-result-v2'
    || input.authority !== DEVELOPMENT_AUTHORITY
    || input.verificationScope
      !== 'canonical-envelope-and-semantic-request-digest-binding-only') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RESULT_IDENTITY_INVALID');
  }
  if (NON_AUTHORITY_KEYS.some(
    (key) => input[key] !== NON_AUTHORITY[key as keyof typeof NON_AUTHORITY],
  )) throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RESULT_AUTHORITY_ESCALATION');
}

function closed(value: unknown, label: string): Record<string, unknown> {
  if (isProxy(value)) throw new TypeError(`${label} must not be a Proxy`);
  return asClosedRecord(value, label);
}

function assertCanonicalBlob(serialized: string): void {
  if (typeof serialized !== 'string'
    || Buffer.byteLength(serialized, 'utf8') > PROGRAMME_CAPTURE_SUPERVISOR_SERVICE_RESULT_MAX_BYTES_V2
    || decodeCanonicalUtf8(serialized) !== serialized) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RESULT_CANONICAL_INVALID');
  }
}

function decodeCanonicalUtf8(value: string): string {
  try { return new TextDecoder('utf-8', { fatal: true }).decode(Buffer.from(value, 'utf8')); }
  catch { return ''; }
}
