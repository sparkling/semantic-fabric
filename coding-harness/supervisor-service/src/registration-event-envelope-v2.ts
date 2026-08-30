// SPDX-License-Identifier: MIT

import {
  asRecord,
  assertExactKeys,
  parseCanonicalPrettyJson,
  parseDigest,
  parseOpaqueId,
  parseUint64,
  sha256CanonicalValue,
} from './closed-json.js';

const RUN_EVENT_MAX_BYTES_V2 = 65_536;
const RUN_EVENT_DIGEST_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-run-event-digest-v2';
const CHANGED_REPLAY_EVIDENCE_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-registration-changed-replay-evidence-v2';
const EVENT_KEYS = Object.freeze([
  'schemaVersion', 'transactionKind', 'recordKind', 'authority', 'eventKind',
  'authorityHead', 'service', 'project', 'runId', 'semanticRequestDigest',
  'globalSequence', 'runSequence', 'previousGlobal', 'previousRun',
  'priorControllerStateHeadDigest', 'resourceTransition', 'body', 'verificationScope',
  'externalAdministrationVerified', 'deploymentAttestationVerified',
  'authorityActivationVerified', 'serviceSignatureVerified', 'priorGlobalEventVerified',
  'priorSemanticReceiptVerified', 'controllerStateHeadVerified', 'rootedClaimVerified',
  'runAdjacencyVerified', 'resourceHighWaterVerified', 'resourceFencingVerified',
  'publicCommitmentVerified', 'checkpointWitnessQuorumVerified',
  'semanticWitnessQuorumVerified', 'stateTransitionAuthorized',
  'attemptStartAuthorized', 'captureAuthorized', 'importAuthorized',
  'promotionAuthorized', 'releaseAuthorized', 'eventDigest',
]);
const EVENT_NON_AUTHORITY_KEYS = Object.freeze([
  'externalAdministrationVerified', 'deploymentAttestationVerified',
  'authorityActivationVerified', 'serviceSignatureVerified', 'priorGlobalEventVerified',
  'priorSemanticReceiptVerified', 'controllerStateHeadVerified', 'rootedClaimVerified',
  'runAdjacencyVerified', 'resourceHighWaterVerified', 'resourceFencingVerified',
  'publicCommitmentVerified', 'checkpointWitnessQuorumVerified',
  'semanticWitnessQuorumVerified', 'stateTransitionAuthorized',
  'attemptStartAuthorized', 'captureAuthorized', 'importAuthorized',
  'promotionAuthorized', 'releaseAuthorized',
]);

export interface StoredRegistrationEventExpectationV2 {
  readonly semanticRequestDigest: string;
  readonly originalRegistrationRequestDigest: string;
  readonly originalRegistrationEventDigest: string;
  readonly originalRegistrationGlobalSequence: string;
  readonly changedReplayPriorControllerStateHeadDigest: string | null;
  readonly projectAuthorityDigest: string;
  readonly principalId: string;
  readonly runId: string;
  readonly authorityHead: Readonly<{
    configurationEpoch: string;
    configurationDigest: string;
    headDigest: string;
  }>;
  readonly priorControllerStateHeadDigest: string;
  readonly claim: Readonly<{
    claimKeyDigest: string;
    claimDigest: string;
    rootedClaimValidationDigest: string;
  }>;
}

export async function validateStoredRegistrationEventEnvelopeV2(
  serialized: string,
  expected: StoredRegistrationEventExpectationV2,
  status: 201 | 409,
): Promise<void> {
  const envelope = parseCanonicalPrettyJson(
    serialized, RUN_EVENT_MAX_BYTES_V2, 'stored registration event envelope',
  );
  const originalRequestDigest = parseDigest(
    expected.originalRegistrationRequestDigest, 'stored original request digest',
  );
  const originalEventDigest = parseDigest(
    expected.originalRegistrationEventDigest, 'stored original event digest',
  );
  const originalGlobalSequence = parseUint64(
    expected.originalRegistrationGlobalSequence, 'stored original global sequence',
  );
  if (originalGlobalSequence === '0') throw new TypeError('invalid original global sequence');
  const changedReplayPriorState = expected.changedReplayPriorControllerStateHeadDigest === null
    ? null : parseDigest(
      expected.changedReplayPriorControllerStateHeadDigest, 'changed-replay prior state',
    );
  if ((status === 201) !== (changedReplayPriorState === null)) {
    throw new TypeError('stored changed-replay prior-state provenance is invalid');
  }
  if ((status === 201) !== (originalRequestDigest === expected.semanticRequestDigest)) {
    throw new TypeError('stored registration original request binding is invalid');
  }
  assertOrderedKeys(envelope, [
    'schemaVersion', 'transactionKind', 'envelopeKind', 'event', 'signature',
  ], 'stored registration event envelope');
  if (envelope.schemaVersion !== 2 || envelope.transactionKind !== 'programme-capture-v2'
    || envelope.envelopeKind !== 'supervisor-run-event-envelope-v2') {
    throw new TypeError('stored registration event envelope identity is invalid');
  }
  const signature = asRecord(envelope.signature, 'stored registration event signature');
  assertOrderedKeys(
    signature, ['algorithm', 'valueBase64Url'], 'stored registration event signature',
  );
  if (signature.algorithm !== 'ed25519'
    || typeof signature.valueBase64Url !== 'string'
    || !/^[A-Za-z0-9_-]{85}[AQgw]$/.test(signature.valueBase64Url)) {
    throw new TypeError('stored registration event signature is invalid');
  }
  const event = asRecord(envelope.event, 'stored registration event');
  assertOrderedKeys(event, EVENT_KEYS, 'stored registration event');
  if (event.schemaVersion !== 2 || event.transactionKind !== 'programme-capture-v2'
    || event.recordKind !== 'supervisor-run-event-v2'
    || event.authority !== 'development-only-no-promotion'
    || event.verificationScope !== 'service-signed-structure-only'
    || EVENT_NON_AUTHORITY_KEYS.some((key) => event[key] !== false)) {
    throw new TypeError('stored registration event identity or authority is invalid');
  }
  validateAuthorityHead(event.authorityHead, expected.authorityHead);
  validateService(event.service);
  validateProject(event.project, expected.projectAuthorityDigest, expected.principalId);
  if (parseOpaqueId(event.runId, 'stored registration event run ID') !== expected.runId
    || parseDigest(event.semanticRequestDigest, 'stored event request digest')
      !== expected.semanticRequestDigest) {
    throw new TypeError('stored registration event request binding is invalid');
  }
  const globalSequence = parseUint64(event.globalSequence, 'stored event global sequence');
  if (globalSequence === '0') throw new TypeError('stored event global sequence is invalid');
  const runSequence = parseUint64(event.runSequence, 'stored event run sequence');
  const previousGlobal = validatePreviousGlobal(event.previousGlobal);
  const previousRun = validatePreviousRun(event.previousRun);
  if ((globalSequence === '1') !== (previousGlobal.kind === 'authority-genesis')) {
    throw new TypeError('stored event global predecessor is invalid');
  }
  const globalOrdinal = BigInt(globalSequence);
  const runOrdinal = BigInt(runSequence);
  const originalGlobalOrdinal = BigInt(originalGlobalSequence);
  if ((status === 201 && globalOrdinal !== originalGlobalOrdinal)
    || (status === 409 && (globalOrdinal <= originalGlobalOrdinal
      || (globalOrdinal === originalGlobalOrdinal + 1n
        && previousGlobal.eventDigest !== originalEventDigest)))) {
    throw new TypeError('stored original/current global order is inconsistent');
  }
  if (globalOrdinal < runOrdinal + 1n
    || (globalOrdinal === runOrdinal + 1n && runOrdinal > 0n
      && previousGlobal.eventDigest !== previousRun.eventDigest)) {
    throw new TypeError('stored event global/run order is inconsistent');
  }
  const priorControllerStateHeadDigest = parseDigest(
    event.priorControllerStateHeadDigest, 'stored event prior state digest',
  );
  if (event.resourceTransition !== null) {
    throw new TypeError('registration result must not transition a resource');
  }
  if (status === 201) validateRegistrationEvent(
    event, runSequence, previousRun, priorControllerStateHeadDigest, expected,
  );
  else await validateChangedReplayEvent(
    event, globalSequence, runSequence, previousRun,
    priorControllerStateHeadDigest, changedReplayPriorState!, expected,
  );
  const eventDigest = parseDigest(event.eventDigest, 'stored registration event digest');
  const { eventDigest: _ignored, ...eventBody } = event;
  if (eventDigest !== await sha256CanonicalValue({
    domain: RUN_EVENT_DIGEST_DOMAIN_V2, event: eventBody,
  })) throw new TypeError('stored registration event digest mismatch');
  if (status === 201 && eventDigest !== originalEventDigest) {
    throw new TypeError('stored original registration event mismatch');
  }
}

function validateService(value: unknown): void {
  const service = asRecord(value, 'stored registration event service');
  assertOrderedKeys(
    service, ['principalId', 'keyEpoch', 'keyFingerprint'],
    'stored registration event service',
  );
  parseOpaqueId(service.principalId, 'stored registration service principal ID');
  if (parseUint64(service.keyEpoch, 'stored registration service key epoch') === '0') {
    throw new TypeError('stored registration service key epoch is invalid');
  }
  parseDigest(service.keyFingerprint, 'stored registration service key fingerprint');
}

function validateAuthorityHead(
  value: unknown,
  expected: StoredRegistrationEventExpectationV2['authorityHead'],
): void {
  const head = asRecord(value, 'stored registration event authority head');
  assertOrderedKeys(head, [
    'configurationEpoch', 'configurationDigest', 'headDigest',
  ], 'stored registration event authority head');
  if (parseUint64(head.configurationEpoch, 'stored registration event authority epoch')
      !== expected.configurationEpoch
    || parseDigest(head.configurationDigest, 'stored registration event configuration digest')
      !== expected.configurationDigest
    || parseDigest(head.headDigest, 'stored registration event authority head digest')
      !== expected.headDigest) {
    throw new TypeError('stored registration event authority head mismatch');
  }
}

function validateProject(
  value: unknown, expectedDigest: string, expectedPrincipalId: string,
): void {
  const project = asRecord(value, 'stored registration event project');
  assertOrderedKeys(
    project, ['projectAuthorityDigest', 'principalId'], 'stored registration event project',
  );
  if (parseDigest(project.projectAuthorityDigest, 'stored event project digest')
    !== expectedDigest) throw new TypeError('stored registration event project mismatch');
  if (parseOpaqueId(project.principalId, 'stored registration event project principal ID')
    !== expectedPrincipalId) throw new TypeError('stored registration event principal mismatch');
}

function validatePreviousGlobal(
  value: unknown,
): Readonly<{ kind: string; eventDigest: unknown }> {
  const previous = asRecord(value, 'stored registration global predecessor');
  assertOrderedKeys(
    previous, ['kind', 'eventDigest', 'semanticReceiptDigest'],
    'stored registration global predecessor',
  );
  parseDigest(previous.semanticReceiptDigest, 'stored predecessor receipt digest');
  if (previous.kind === 'authority-genesis' && previous.eventDigest === null) {
    return previous as Readonly<{ kind: string; eventDigest: unknown }>;
  }
  if (previous.kind === 'semantic-event') {
    parseDigest(previous.eventDigest, 'stored global predecessor event digest');
    return previous as Readonly<{ kind: string; eventDigest: unknown }>;
  }
  throw new TypeError('stored registration global predecessor is invalid');
}

function validatePreviousRun(value: unknown): Readonly<{ kind: string; eventDigest: unknown }> {
  const previous = asRecord(value, 'stored registration run predecessor');
  assertOrderedKeys(
    previous, ['kind', 'eventDigest'], 'stored registration run predecessor',
  );
  if (previous.kind === 'run-genesis' && previous.eventDigest === null) {
    return previous as Readonly<{ kind: string; eventDigest: unknown }>;
  }
  if (previous.kind === 'run-event') {
    parseDigest(previous.eventDigest, 'stored run predecessor event digest');
    return previous as Readonly<{ kind: string; eventDigest: unknown }>;
  }
  throw new TypeError('stored registration run predecessor is invalid');
}

function validateRegistrationEvent(
  event: Record<string, unknown>,
  runSequence: string,
  previousRun: Readonly<{ kind: string }>,
  priorControllerStateHeadDigest: string,
  expected: StoredRegistrationEventExpectationV2,
): void {
  if (event.eventKind !== 'claim-registered-v2' || runSequence !== '0'
    || previousRun.kind !== 'run-genesis'
    || priorControllerStateHeadDigest !== expected.priorControllerStateHeadDigest) {
    throw new TypeError('stored 201 registration event is invalid');
  }
  const body = asRecord(event.body, 'stored registration event body');
  assertOrderedKeys(
    body, ['claimKeyDigest', 'claimDigest', 'rootedClaimValidationDigest'],
    'stored registration event body',
  );
  if (parseDigest(body.claimKeyDigest, 'stored claim-key digest')
      !== expected.claim.claimKeyDigest
    || parseDigest(body.claimDigest, 'stored claim digest') !== expected.claim.claimDigest
    || parseDigest(body.rootedClaimValidationDigest, 'stored rooted-claim digest')
      !== expected.claim.rootedClaimValidationDigest) {
    throw new TypeError('stored registration event claim mismatch');
  }
}

function validateChangedReplayEvent(
  event: Record<string, unknown>,
  globalSequence: string,
  runSequence: string,
  previousRun: Readonly<{ kind: string; eventDigest: unknown }>,
  priorControllerStateHeadDigest: string,
  expectedPriorControllerStateHeadDigest: string,
  expected: StoredRegistrationEventExpectationV2,
): Promise<void> {
  if (event.eventKind !== 'capture-run-terminal-v2' || globalSequence === '1'
    || runSequence !== '1'
    || previousRun.kind !== 'run-event'
    || priorControllerStateHeadDigest !== expectedPriorControllerStateHeadDigest) {
    throw new TypeError('stored 409 changed-replay event is invalid');
  }
  const body = asRecord(event.body, 'stored changed-replay event body');
  assertOrderedKeys(body, [
    'terminalStage', 'outcomeCode', 'registrationEventDigest',
    'outcomeEvidenceDigest', 'leaseEventDigest', 'leaseId', 'fence',
    'resourceDisposition', 'attemptId', 'captureRecordDigest',
    'outputEnvelopeDigest', 'cleanupEvidenceDigest',
  ], 'stored changed-replay event body');
  if (body.terminalStage !== 'registration'
    || body.outcomeCode !== 'registration-changed-replay-v2'
    || parseDigest(body.registrationEventDigest, 'stored registration event reference')
      !== previousRun.eventDigest
    || body.registrationEventDigest !== expected.originalRegistrationEventDigest
    || [
      'leaseEventDigest', 'leaseId', 'fence', 'resourceDisposition', 'attemptId',
      'captureRecordDigest', 'outputEnvelopeDigest', 'cleanupEvidenceDigest',
    ].some((key) => body[key] !== null)) {
    throw new TypeError('stored changed-replay outcome is invalid');
  }
  const outcomeEvidenceDigest = parseDigest(
    body.outcomeEvidenceDigest, 'stored changed-replay evidence digest',
  );
  return sha256CanonicalValue({
    domain: CHANGED_REPLAY_EVIDENCE_DOMAIN_V2,
    originalRegistrationRequestDigest: expected.originalRegistrationRequestDigest,
    originalRegistrationEventDigest: expected.originalRegistrationEventDigest,
    changedRegistrationRequestDigest: expected.semanticRequestDigest,
    project: {
      projectAuthorityDigest: expected.projectAuthorityDigest,
      principalId: expected.principalId,
    },
    authorityHead: expected.authorityHead,
  }).then((derived) => {
    if (derived !== outcomeEvidenceDigest) {
      throw new TypeError('stored changed-replay evidence digest mismatch');
    }
  });
}

function assertOrderedKeys(
  value: Record<string, unknown>, expected: readonly string[], label: string,
): void {
  assertExactKeys(value, expected, label);
  const actual = Object.keys(value);
  if (actual.some((key, index) => key !== expected[index])) {
    throw new TypeError(`${label} member order is invalid`);
  }
}
