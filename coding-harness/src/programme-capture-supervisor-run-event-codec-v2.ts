// SPDX-License-Identifier: MIT

import { canonical } from '@metaharness/harness';
import { assertExactKeys, deepFreeze, DEVELOPMENT_AUTHORITY } from './contracts.js';
import {
  parseProgrammeCaptureSupervisorAuthorityHeadRefV2,
  parseProgrammeCaptureSupervisorPreviousGlobalV2,
  parseProgrammeCaptureSupervisorPreviousRunV2,
  parseProgrammeCaptureSupervisorResourceTransitionV2,
  parseProgrammeCaptureSupervisorRunEventBodyV2,
} from './programme-capture-supervisor-run-event-body-v2.js';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_DIGEST_DOMAIN_V2,
  PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_KINDS_V2,
  PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_MAX_BYTES_V2,
  PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_SIGNING_DOMAIN_V2,
  closedRunEventRecordV2,
  parseRunEventDigestV2,
  parseRunEventOpaqueIdV2,
  parseRunEventUint64V2,
  type ProgrammeCaptureSupervisorAttemptStartBodyV2,
  type ProgrammeCaptureSupervisorAttemptTerminalBodyV2,
  type ProgrammeCaptureSupervisorLeaseGrantedBodyV2,
  type ProgrammeCaptureSupervisorResourceTransitionV2,
  type ProgrammeCaptureSupervisorRunEventBodyV2,
  type ProgrammeCaptureSupervisorRunEventEnvelopeV2,
  type ProgrammeCaptureSupervisorRunEventKindV2,
  type ProgrammeCaptureSupervisorRunEventNonAuthorityV2,
  type ProgrammeCaptureSupervisorRunEventV2,
  type ProgrammeCaptureSupervisorRunTerminalBodyV2,
} from './programme-capture-supervisor-run-event-contracts-v2.js';
import {
  parseProgrammeCaptureSupervisorEd25519SignatureV1,
} from './programme-capture-supervisor-crypto-v1.js';
import { digestValue } from './receipts.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export const PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_NON_AUTHORITY_V2 = Object.freeze({
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
}) satisfies ProgrammeCaptureSupervisorRunEventNonAuthorityV2;

const NON_AUTHORITY_KEYS = Object.freeze(
  Object.keys(PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_NON_AUTHORITY_V2),
);

export function parseProgrammeCaptureSupervisorRunEventV2(
  value: unknown,
): ProgrammeCaptureSupervisorRunEventV2 {
  const input = closedRunEventRecordV2(value, 'programme capture supervisor run event');
  assertExactKeys(input, [
    'schemaVersion', 'transactionKind', 'recordKind', 'authority', 'eventKind',
    'authorityHead', 'service', 'project', 'runId', 'semanticRequestDigest',
    'globalSequence', 'runSequence', 'previousGlobal', 'previousRun',
    'priorControllerStateHeadDigest', 'resourceTransition', 'body', 'verificationScope',
    ...NON_AUTHORITY_KEYS, 'eventDigest',
  ], 'programme capture supervisor run event');
  assertIdentityAndScope(input);
  assertNonAuthority(input);
  const eventKind = parseEventKind(input.eventKind);
  const serviceInput = exactRecord(
    input.service, ['principalId', 'keyEpoch', 'keyFingerprint'], 'run-event service',
  );
  const projectInput = exactRecord(
    input.project, ['projectAuthorityDigest', 'principalId'], 'run-event project',
  );
  const globalSequence = parseRunEventUint64V2(
    input.globalSequence, 'run-event global sequence', 1n,
  );
  const runSequence = parseRunEventUint64V2(
    input.runSequence, 'run-event run sequence', 0n,
  );
  const previousGlobal = parseProgrammeCaptureSupervisorPreviousGlobalV2(input.previousGlobal);
  const previousRun = parseProgrammeCaptureSupervisorPreviousRunV2(input.previousRun);
  const resourceTransition = input.resourceTransition === null ? null
    : parseProgrammeCaptureSupervisorResourceTransitionV2(input.resourceTransition);
  const body = parseProgrammeCaptureSupervisorRunEventBodyV2(eventKind, input.body);
  assertStructuralReferences({
    eventKind, globalSequence, runSequence, previousGlobal, previousRun,
    resourceTransition, body,
  });
  const eventBody = {
    schemaVersion: 2 as const,
    transactionKind: 'programme-capture-v2' as const,
    recordKind: 'supervisor-run-event-v2' as const,
    authority: DEVELOPMENT_AUTHORITY,
    eventKind,
    authorityHead: parseProgrammeCaptureSupervisorAuthorityHeadRefV2(input.authorityHead),
    service: {
      principalId: parseRunEventOpaqueIdV2(serviceInput.principalId, 'service principal ID'),
      keyEpoch: parseRunEventUint64V2(serviceInput.keyEpoch, 'service key epoch', 1n),
      keyFingerprint: parseRunEventDigestV2(
        serviceInput.keyFingerprint, 'service key fingerprint',
      ),
    },
    project: {
      projectAuthorityDigest: parseRunEventDigestV2(
        projectInput.projectAuthorityDigest, 'project authority digest',
      ),
      principalId: parseRunEventOpaqueIdV2(
        projectInput.principalId, 'project principal ID',
      ),
    },
    runId: parseRunEventOpaqueIdV2(input.runId, 'run-event run ID'),
    semanticRequestDigest: parseRunEventDigestV2(
      input.semanticRequestDigest, 'semantic request digest',
    ),
    globalSequence,
    runSequence,
    previousGlobal,
    previousRun,
    priorControllerStateHeadDigest: parseRunEventDigestV2(
      input.priorControllerStateHeadDigest, 'prior controller state-head digest',
    ),
    resourceTransition,
    body,
    verificationScope: 'service-signed-structure-only' as const,
    ...PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_NON_AUTHORITY_V2,
  };
  const eventDigest = parseRunEventDigestV2(input.eventDigest, 'supervisor run-event digest');
  if (eventDigest !== digestValue({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_DIGEST_DOMAIN_V2,
    event: eventBody,
  })) throw new Error('HARNESS_CAPTURE_SUPERVISOR_RUN_EVENT_DIGEST_MISMATCH');
  return deepFreeze({ ...eventBody, eventDigest });
}

export function serializeProgrammeCaptureSupervisorRunEventV2(value: unknown): string {
  return `${JSON.stringify(parseProgrammeCaptureSupervisorRunEventV2(value), null, 2)}\n`;
}

export function parseProgrammeCaptureSupervisorRunEventBlobV2(
  serialized: string,
): ProgrammeCaptureSupervisorRunEventV2 {
  assertCanonicalBlob(serialized, 'supervisor run event');
  const event = parseProgrammeCaptureSupervisorRunEventV2(
    parseJsonWithoutDuplicateKeys(serialized, 'programme capture supervisor run event'),
  );
  if (serializeProgrammeCaptureSupervisorRunEventV2(event) !== serialized) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_RUN_EVENT_CANONICAL_REQUIRED');
  }
  return event;
}

export function parseProgrammeCaptureSupervisorRunEventEnvelopeV2(
  value: unknown,
): ProgrammeCaptureSupervisorRunEventEnvelopeV2 {
  const input = closedRunEventRecordV2(value, 'programme capture supervisor run-event envelope');
  assertExactKeys(input, [
    'schemaVersion', 'transactionKind', 'envelopeKind', 'event', 'signature',
  ], 'programme capture supervisor run-event envelope');
  if (input.schemaVersion !== 2 || input.transactionKind !== 'programme-capture-v2'
    || input.envelopeKind !== 'supervisor-run-event-envelope-v2') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RUN_EVENT_ENVELOPE_IDENTITY_INVALID');
  }
  const signature = exactRecord(
    input.signature, ['algorithm', 'valueBase64Url'], 'run-event envelope signature',
  );
  if (signature.algorithm !== 'ed25519') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_SIGNATURE_ALGORITHM_INVALID');
  }
  return deepFreeze({
    schemaVersion: 2,
    transactionKind: 'programme-capture-v2',
    envelopeKind: 'supervisor-run-event-envelope-v2',
    event: parseProgrammeCaptureSupervisorRunEventV2(input.event),
    signature: {
      algorithm: 'ed25519',
      valueBase64Url: parseProgrammeCaptureSupervisorEd25519SignatureV1(
        signature.valueBase64Url,
      ),
    },
  });
}

export function serializeProgrammeCaptureSupervisorRunEventEnvelopeV2(value: unknown): string {
  return `${JSON.stringify(parseProgrammeCaptureSupervisorRunEventEnvelopeV2(value), null, 2)}\n`;
}

export function parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2(
  serialized: string,
): ProgrammeCaptureSupervisorRunEventEnvelopeV2 {
  assertCanonicalBlob(serialized, 'supervisor run-event envelope');
  const envelope = parseProgrammeCaptureSupervisorRunEventEnvelopeV2(
    parseJsonWithoutDuplicateKeys(serialized, 'programme capture supervisor run-event envelope'),
  );
  if (serializeProgrammeCaptureSupervisorRunEventEnvelopeV2(envelope) !== serialized) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_RUN_EVENT_ENVELOPE_CANONICAL_REQUIRED');
  }
  return envelope;
}

export function programmeCaptureSupervisorRunEventSigningPayloadV2(value: unknown): Buffer {
  const event = parseProgrammeCaptureSupervisorRunEventV2(value);
  return Buffer.from(canonical({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_SIGNING_DOMAIN_V2,
    event,
  }), 'utf8');
}

function assertStructuralReferences(value: Readonly<{
  eventKind: ProgrammeCaptureSupervisorRunEventKindV2;
  globalSequence: string;
  runSequence: string;
  previousGlobal: ProgrammeCaptureSupervisorRunEventV2['previousGlobal'];
  previousRun: ProgrammeCaptureSupervisorRunEventV2['previousRun'];
  resourceTransition: ProgrammeCaptureSupervisorResourceTransitionV2 | null;
  body: ProgrammeCaptureSupervisorRunEventBodyV2;
}>): void {
  const registration = value.eventKind === 'claim-registered-v2';
  if (registration !== (value.runSequence === '0'
    && value.previousRun.kind === 'run-genesis')) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RUN_GENESIS_INVALID');
  }
  if (!registration && (value.runSequence === '0' || value.previousRun.kind !== 'run-event')) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RUN_PREDECESSOR_INVALID');
  }
  const globalGenesis = value.previousGlobal.kind === 'authority-genesis';
  if (globalGenesis !== (value.globalSequence === '1')) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_GLOBAL_GENESIS_INVALID');
  }
  const resourceRequired = value.eventKind === 'runner-lease-granted-v2'
    || value.eventKind === 'capture-attempt-start-committed-v2'
    || value.eventKind === 'capture-attempt-terminal-v2'
    || (value.eventKind === 'capture-run-terminal-v2'
      && (value.body as ProgrammeCaptureSupervisorRunTerminalBodyV2)
        .terminalStage === 'leased-pre-start');
  if (resourceRequired !== (value.resourceTransition !== null)) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RESOURCE_TRANSITION_PRESENCE_INVALID');
  }
  if (value.eventKind === 'runner-lease-granted-v2') {
    const body = value.body as ProgrammeCaptureSupervisorLeaseGrantedBodyV2;
    if (body.lease.fence !== value.resourceTransition?.fence
      || body.runner.enrollmentRecordDigest
        !== value.resourceTransition?.runnerEnrollmentRecordDigest) {
      throw new Error('HARNESS_CAPTURE_SUPERVISOR_LEASE_RESOURCE_BINDING_MISMATCH');
    }
  } else if (value.eventKind === 'capture-attempt-start-committed-v2') {
    const body = value.body as ProgrammeCaptureSupervisorAttemptStartBodyV2;
    if (body.fence !== value.resourceTransition?.fence
      || body.resourceConflictSetDigest !== value.resourceTransition?.conflictSetDigest) {
      throw new Error('HARNESS_CAPTURE_SUPERVISOR_START_RESOURCE_BINDING_MISMATCH');
    }
  } else if (value.eventKind === 'capture-attempt-terminal-v2') {
    const body = value.body as ProgrammeCaptureSupervisorAttemptTerminalBodyV2;
    if (body.fence !== value.resourceTransition?.fence) {
      throw new Error('HARNESS_CAPTURE_SUPERVISOR_TERMINAL_RESOURCE_BINDING_MISMATCH');
    }
  } else if (value.eventKind === 'capture-run-terminal-v2'
    && value.resourceTransition !== null
    && (value.body as ProgrammeCaptureSupervisorRunTerminalBodyV2).fence
      !== value.resourceTransition.fence) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_TERMINAL_RESOURCE_BINDING_MISMATCH');
  }
}

function parseEventKind(value: unknown): ProgrammeCaptureSupervisorRunEventKindV2 {
  if (typeof value !== 'string'
    || !PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_KINDS_V2.includes(
      value as ProgrammeCaptureSupervisorRunEventKindV2,
    )) throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RUN_EVENT_KIND_INVALID');
  return value as ProgrammeCaptureSupervisorRunEventKindV2;
}

function assertIdentityAndScope(input: Record<string, unknown>): void {
  if (input.schemaVersion !== 2 || input.transactionKind !== 'programme-capture-v2'
    || input.recordKind !== 'supervisor-run-event-v2'
    || input.authority !== DEVELOPMENT_AUTHORITY) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RUN_EVENT_IDENTITY_INVALID');
  }
  if (input.verificationScope !== 'service-signed-structure-only') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RUN_EVENT_SCOPE_INVALID');
  }
}

function assertNonAuthority(input: Record<string, unknown>): void {
  if (NON_AUTHORITY_KEYS.some((key) => input[key]
    !== PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_NON_AUTHORITY_V2[
      key as keyof typeof PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_NON_AUTHORITY_V2
    ])) throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RUN_EVENT_AUTHORITY_ESCALATION');
}

function exactRecord(value: unknown, keys: readonly string[], label: string) {
  const input = closedRunEventRecordV2(value, label);
  assertExactKeys(input, keys, label);
  return input;
}

function assertCanonicalBlob(serialized: string, label: string): void {
  if (typeof serialized !== 'string'
    || Buffer.byteLength(serialized, 'utf8')
      > PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_MAX_BYTES_V2
    || decodeCanonicalUtf8(serialized) !== serialized) {
    throw new TypeError(`${label} must be bounded canonical UTF-8 JSON`);
  }
}

function decodeCanonicalUtf8(value: string): string {
  try { return new TextDecoder('utf-8', { fatal: true }).decode(Buffer.from(value, 'utf8')); }
  catch { return ''; }
}
