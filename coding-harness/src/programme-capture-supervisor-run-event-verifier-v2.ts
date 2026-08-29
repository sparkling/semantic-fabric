// SPDX-License-Identifier: MIT

import {
  DEVELOPMENT_AUTHORITY,
  assertExactKeys,
  deepFreeze,
  snapshotUint8Array,
} from './contracts.js';
import {
  parseProgrammeCaptureSupervisorAuthorityConfigurationBlobV2,
  parseProgrammeCaptureSupervisorAuthorityConfigurationV2,
  programmeCaptureSupervisorAuthorityGenesisHeadDigestV2,
  type ProgrammeCaptureSupervisorAuthorityConfigurationV2,
} from './programme-capture-supervisor-authority-config-v2.js';
import {
  verifyProgrammeCaptureSupervisorAuthorityTransitionV2,
} from './programme-capture-supervisor-authority-transition-v2.js';
import {
  parseProgrammeCaptureSupervisorPreviousGlobalV2,
  parseProgrammeCaptureSupervisorPreviousRunV2,
  parseProgrammeCaptureSupervisorPriorResourceStateV2,
} from './programme-capture-supervisor-run-event-body-v2.js';
import {
  parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2,
  programmeCaptureSupervisorRunEventSigningPayloadV2,
} from './programme-capture-supervisor-run-event-codec-v2.js';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_MAX_BYTES_V2,
  PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_VALIDATION_DIGEST_DOMAIN_V2,
  PROGRAMME_CAPTURE_SUPERVISOR_RUN_HISTORY_DIGEST_DOMAIN_V2,
  PROGRAMME_CAPTURE_SUPERVISOR_RUN_HISTORY_VALIDATION_DIGEST_DOMAIN_V2,
  closedRunEventRecordV2,
  denseRunEventArrayV2,
  parseRunEventDigestV2,
  parseRunEventOpaqueIdV2,
  parseRunEventUint64V2,
  type ProgrammeCaptureSupervisorPreviousGlobalV2,
  type ProgrammeCaptureSupervisorPriorResourceStateV2,
  type ProgrammeCaptureSupervisorRunEventV2,
} from './programme-capture-supervisor-run-event-contracts-v2.js';
import {
  programmeCaptureSupervisorUtf8Sha256V1,
  verifyProgrammeCaptureSupervisorEd25519SignatureV1,
} from './programme-capture-supervisor-crypto-v1.js';
import {
  deriveProgrammeCaptureSupervisorRunStateV2,
  programmeCaptureSupervisorControllerStateHeadDigestV2,
  type ProgrammeCaptureSupervisorRunStateV2,
} from './programme-capture-supervisor-run-event-transition-v2.js';
import { digestValue } from './receipts.js';

interface ParsedAuthorityContextV2 {
  readonly configuration: ProgrammeCaptureSupervisorAuthorityConfigurationV2;
  readonly activeHeadDigest: string;
  readonly transitionGlobalSequence: string | null;
}

export interface ProgrammeCaptureSupervisorRunEventValidationV2 {
  readonly schemaVersion: 2;
  readonly transactionKind: 'programme-capture-v2';
  readonly evidenceKind: 'non-authorizing-supervisor-run-event-validation-v2';
  readonly authority: typeof DEVELOPMENT_AUTHORITY;
  readonly runId: string;
  readonly eventKind: ProgrammeCaptureSupervisorRunEventV2['eventKind'];
  readonly eventDigest: string;
  readonly serializedEnvelopeDigest: string;
  readonly configurationEpoch: string;
  readonly configurationDigest: string;
  readonly activeAuthorityHeadDigest: string;
  readonly verificationScope: 'signature-and-supplied-reference-matching-only';
  readonly canonicalEnvelopeVerified: true;
  readonly eventDigestVerified: true;
  readonly serviceSignatureVerified: true;
  readonly suppliedAuthorityConfigurationMatched: true;
  readonly authorityAdjacencyVerified: true;
  readonly suppliedAuthorityHeadMatched: true;
  readonly suppliedGlobalReferencesMatched: true;
  readonly suppliedRunReferencesMatched: true;
  readonly externalAdministrationVerified: false;
  readonly deploymentAttestationVerified: false;
  readonly authorityActivationVerified: false;
  readonly fullAuthorityHistoryVerified: false;
  readonly priorGlobalEventVerified: false;
  readonly globalOrderVerified: false;
  readonly priorSemanticReceiptVerified: false;
  readonly controllerStateHeadVerified: false;
  readonly rootedClaimVerified: false;
  readonly runAdjacencyVerified: false;
  readonly resourceAdjacencyVerified: false;
  readonly resourceHighWaterVerified: false;
  readonly resourceFencingVerified: false;
  readonly runnerAdmissionVerified: false;
  readonly hostEvidenceVerified: false;
  readonly publicCommitmentVerified: false;
  readonly checkpointWitnessQuorumVerified: false;
  readonly semanticWitnessQuorumVerified: false;
  readonly stateTransitionAuthorized: false;
  readonly attemptStartAuthorized: false;
  readonly captureAuthorized: false;
  readonly importAuthorized: false;
  readonly promotionAuthorized: false;
  readonly releaseAuthorized: false;
  readonly validationDigest: string;
}

const VALIDATION_NON_AUTHORITY = Object.freeze({
  externalAdministrationVerified: false as const,
  deploymentAttestationVerified: false as const,
  authorityActivationVerified: false as const,
  fullAuthorityHistoryVerified: false as const,
  priorGlobalEventVerified: false as const,
  globalOrderVerified: false as const,
  priorSemanticReceiptVerified: false as const,
  controllerStateHeadVerified: false as const,
  rootedClaimVerified: false as const,
  runAdjacencyVerified: false as const,
  resourceAdjacencyVerified: false as const,
  resourceHighWaterVerified: false as const,
  resourceFencingVerified: false as const,
  runnerAdmissionVerified: false as const,
  hostEvidenceVerified: false as const,
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

export function verifyProgrammeCaptureSupervisorRunEventEnvelopeV2(
  value: unknown,
): ProgrammeCaptureSupervisorRunEventValidationV2 {
  const input = closedRunEventRecordV2(value, 'supervisor run-event verification input');
  assertExactKeys(input, [
    'serializedEnvelope', 'serializedAuthorityConfiguration', 'activeAuthorityHeadDigest',
    'activation', 'trustedServicePublicKeySpkiDer', 'expectedRunId',
    'expectedGlobalSequence', 'expectedPreviousGlobal', 'expectedRunSequence',
    'expectedPreviousRun', 'expectedPriorControllerStateHeadDigest',
  ], 'supervisor run-event verification input');
  if (typeof input.serializedEnvelope !== 'string'
    || typeof input.serializedAuthorityConfiguration !== 'string') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_SERIALIZED_INPUT_REQUIRED');
  }
  const envelope = parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2(
    input.serializedEnvelope,
  );
  const event = envelope.event;
  const authority = parseAuthorityContext(
    input.serializedAuthorityConfiguration, input.activeAuthorityHeadDigest,
    input.activation, event.globalSequence,
  );
  assertConfigurationBindings(event, authority);
  const expectedRunId = parseRunEventOpaqueIdV2(input.expectedRunId, 'expected run ID');
  const expectedGlobalSequence = parseRunEventUint64V2(
    input.expectedGlobalSequence, 'expected global sequence', 1n,
  );
  const expectedRunSequence = parseRunEventUint64V2(
    input.expectedRunSequence, 'expected run sequence', 0n,
  );
  const expectedPreviousGlobal = parseProgrammeCaptureSupervisorPreviousGlobalV2(
    input.expectedPreviousGlobal,
  );
  const expectedPreviousRun = parseProgrammeCaptureSupervisorPreviousRunV2(
    input.expectedPreviousRun,
  );
  const expectedStateHead = parseRunEventDigestV2(
    input.expectedPriorControllerStateHeadDigest,
    'expected prior controller state-head digest',
  );
  if (event.runId !== expectedRunId || event.globalSequence !== expectedGlobalSequence
    || event.runSequence !== expectedRunSequence
    || !samePreviousGlobal(event.previousGlobal, expectedPreviousGlobal)
    || !samePreviousRun(event.previousRun, expectedPreviousRun)
    || event.priorControllerStateHeadDigest !== expectedStateHead) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_SUPPLIED_REFERENCE_MISMATCH');
  }
  verifyProgrammeCaptureSupervisorEd25519SignatureV1({
    payload: programmeCaptureSupervisorRunEventSigningPayloadV2(event),
    signatureBase64Url: envelope.signature.valueBase64Url,
    trustedPublicKeySpkiDer: input.trustedServicePublicKeySpkiDer,
    expectedAuthorityKeyFingerprint: authority.configuration.service.principal.keyFingerprint,
  });
  const body = {
    schemaVersion: 2 as const,
    transactionKind: 'programme-capture-v2' as const,
    evidenceKind: 'non-authorizing-supervisor-run-event-validation-v2' as const,
    authority: DEVELOPMENT_AUTHORITY,
    runId: event.runId,
    eventKind: event.eventKind,
    eventDigest: event.eventDigest,
    serializedEnvelopeDigest: programmeCaptureSupervisorUtf8Sha256V1(
      input.serializedEnvelope, PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_MAX_BYTES_V2,
    ),
    configurationEpoch: authority.configuration.configurationEpoch,
    configurationDigest: authority.configuration.configurationDigest,
    activeAuthorityHeadDigest: authority.activeHeadDigest,
    verificationScope: 'signature-and-supplied-reference-matching-only' as const,
    canonicalEnvelopeVerified: true as const,
    eventDigestVerified: true as const,
    serviceSignatureVerified: true as const,
    suppliedAuthorityConfigurationMatched: true as const,
    authorityAdjacencyVerified: true as const,
    suppliedAuthorityHeadMatched: true as const,
    suppliedGlobalReferencesMatched: true as const,
    suppliedRunReferencesMatched: true as const,
    ...VALIDATION_NON_AUTHORITY,
  };
  return deepFreeze({
    ...body,
    validationDigest: digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_VALIDATION_DIGEST_DOMAIN_V2,
      validation: body,
    }),
  });
}

export function verifyProgrammeCaptureSupervisorRunHistoryV2(value: unknown) {
  const input = closedRunEventRecordV2(value, 'supervisor run-history verification input');
  assertExactKeys(input, [
    'serializedAuthorityConfiguration', 'activeAuthorityHeadDigest', 'activation',
    'trustedServicePublicKeySpkiDer', 'expectedRunId',
    'expectedInitialControllerStateHeadDigest', 'expectedLeaseResourcePredecessors',
    'entries',
  ], 'supervisor run-history verification input');
  if (typeof input.serializedAuthorityConfiguration !== 'string') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_SERIALIZED_CONFIG_REQUIRED');
  }
  const trustedKey = snapshotUint8Array(
    input.trustedServicePublicKeySpkiDer, 'trusted service public key', 1_024,
  );
  const expectedRunId = parseRunEventOpaqueIdV2(input.expectedRunId, 'expected history run ID');
  const entries = denseRunEventArrayV2(input.entries, 'supervisor signed run-history entries');
  if (entries.length === 0 || entries.length > 5) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RUN_HISTORY_LENGTH_INVALID');
  }
  let stateHead = parseRunEventDigestV2(
    input.expectedInitialControllerStateHeadDigest,
    'expected initial controller state-head digest',
  );
  let previous: ProgrammeCaptureSupervisorRunEventV2 | null = null;
  const events: ProgrammeCaptureSupervisorRunEventV2[] = [];
  const validations: ProgrammeCaptureSupervisorRunEventValidationV2[] = [];
  for (let index = 0; index < entries.length; index += 1) {
    const entry = closedRunEventRecordV2(entries[index], `signed run-history entry[${index}]`);
    assertExactKeys(entry, [
      'serializedEnvelope', 'expectedGlobalSequence', 'expectedPreviousGlobal',
    ], `signed run-history entry[${index}]`);
    if (typeof entry.serializedEnvelope !== 'string') {
      throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_SERIALIZED_ENVELOPE_REQUIRED');
    }
    const envelope = parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2(
      entry.serializedEnvelope,
    );
    const expectedPreviousRun = previous === null
      ? { kind: 'run-genesis' as const, eventDigest: null }
      : { kind: 'run-event' as const, eventDigest: previous.eventDigest };
    const validation = verifyProgrammeCaptureSupervisorRunEventEnvelopeV2({
      serializedEnvelope: entry.serializedEnvelope,
      serializedAuthorityConfiguration: input.serializedAuthorityConfiguration,
      activeAuthorityHeadDigest: input.activeAuthorityHeadDigest,
      activation: input.activation,
      trustedServicePublicKeySpkiDer: trustedKey,
      expectedRunId,
      expectedGlobalSequence: entry.expectedGlobalSequence,
      expectedPreviousGlobal: entry.expectedPreviousGlobal,
      expectedRunSequence: String(index),
      expectedPreviousRun,
      expectedPriorControllerStateHeadDigest: stateHead,
    });
    events.push(envelope.event);
    validations.push(validation);
    stateHead = programmeCaptureSupervisorControllerStateHeadDigestV2(
      stateHead, envelope.event.eventDigest,
    );
    previous = envelope.event;
  }
  const state = deriveProgrammeCaptureSupervisorRunStateV2(events);
  if (state.runId !== expectedRunId
    || state.controllerStateHeadDigest !== stateHead) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_RUN_HISTORY_STATE_MISMATCH');
  }
  assertExpectedLeaseResourcePredecessors(
    input.expectedLeaseResourcePredecessors, events,
  );
  const serializedEnvelopeDigests = validations.map(
    ({ serializedEnvelopeDigest }) => serializedEnvelopeDigest,
  );
  const historyDigest = digestValue({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_RUN_HISTORY_DIGEST_DOMAIN_V2,
    eventDigests: events.map(({ eventDigest }) => eventDigest),
    serializedEnvelopeDigests,
    stateDigest: state.stateDigest,
  });
  const body = {
    schemaVersion: 2 as const,
    transactionKind: 'programme-capture-v2' as const,
    evidenceKind: 'non-authorizing-supervisor-run-history-validation-v2' as const,
    authority: DEVELOPMENT_AUTHORITY,
    runId: state.runId,
    historyDigest,
    serializedEnvelopeDigests,
    state,
    verificationScope: 'signed-complete-run-history-structure-only' as const,
    canonicalEnvelopesVerified: true as const,
    serviceSignaturesVerified: true as const,
    suppliedAuthorityConfigurationMatched: true as const,
    authorityAdjacencyVerified: true as const,
    suppliedAuthorityHeadMatched: true as const,
    suppliedGlobalReferencesMatched: true as const,
    suppliedResourcePredecessorsMatched: true as const,
    runAdjacencyVerified: true as const,
    resourceAdjacencyVerified: true as const,
    externalAdministrationVerified: false as const,
    deploymentAttestationVerified: false as const,
    authorityActivationVerified: false as const,
    fullAuthorityHistoryVerified: false as const,
    priorGlobalEventVerified: false as const,
    globalOrderVerified: false as const,
    priorSemanticReceiptVerified: false as const,
    controllerStateHeadVerified: false as const,
    rootedClaimVerified: false as const,
    resourceHighWaterVerified: false as const,
    resourceFencingVerified: false as const,
    runnerAdmissionVerified: false as const,
    hostEvidenceVerified: false as const,
    publicCommitmentVerified: false as const,
    checkpointWitnessQuorumVerified: false as const,
    semanticWitnessQuorumVerified: false as const,
    stateTransitionAuthorized: false as const,
    attemptStartAuthorized: false as const,
    captureAuthorized: false as const,
    importAuthorized: false as const,
    promotionAuthorized: false as const,
    releaseAuthorized: false as const,
  };
  return deepFreeze({
    ...body,
    validationDigest: digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_RUN_HISTORY_VALIDATION_DIGEST_DOMAIN_V2,
      validation: body,
    }),
  });
}

function parseAuthorityContext(
  serializedConfiguration: string,
  activeHeadValue: unknown,
  activationValue: unknown,
  eventGlobalSequence: string,
): ParsedAuthorityContextV2 {
  const configuration = parseProgrammeCaptureSupervisorAuthorityConfigurationBlobV2(
    serializedConfiguration,
  );
  const activeHeadDigest = parseRunEventDigestV2(
    activeHeadValue, 'active authority head digest',
  );
  const activation = closedRunEventRecordV2(activationValue, 'authority activation context');
  if (activation.kind === 'genesis') {
    assertExactKeys(activation, ['kind'], 'genesis authority activation context');
    if (configuration.configurationEpoch !== '0'
      || activeHeadDigest !== programmeCaptureSupervisorAuthorityGenesisHeadDigestV2(
        configuration,
      )) throw new Error('HARNESS_CAPTURE_SUPERVISOR_ACTIVE_GENESIS_HEAD_MISMATCH');
    return Object.freeze({ configuration, activeHeadDigest, transitionGlobalSequence: null });
  }
  if (activation.kind !== 'transition-adjacency') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_ACTIVATION_CONTEXT_INVALID');
  }
  assertExactKeys(activation, [
    'kind', 'transition', 'transitionContext',
  ], 'transition authority activation context');
  const transition = verifyProgrammeCaptureSupervisorAuthorityTransitionV2(
    activation.transition, activation.transitionContext,
  );
  const transitionContext = closedRunEventRecordV2(
    activation.transitionContext, 'authority transition verification context',
  );
  assertExactKeys(transitionContext, [
    'predecessorConfiguration', 'expectedPredecessorHeadDigest',
    'expectedGlobalSequence', 'successorConfiguration',
  ], 'authority transition verification context');
  const successor = parseProgrammeCaptureSupervisorAuthorityConfigurationV2(
    transitionContext.successorConfiguration,
  );
  if (successor.configurationDigest !== configuration.configurationDigest
    || transition.successorConfiguration.configurationDigest !== configuration.configurationDigest
    || activeHeadDigest !== transition.transitionDigest
    || BigInt(eventGlobalSequence) <= BigInt(transition.globalSequence)) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_ACTIVE_TRANSITION_HEAD_MISMATCH');
  }
  return Object.freeze({
    configuration, activeHeadDigest, transitionGlobalSequence: transition.globalSequence,
  });
}

function assertConfigurationBindings(
  event: ProgrammeCaptureSupervisorRunEventV2,
  authority: ParsedAuthorityContextV2,
): void {
  const configuration = authority.configuration;
  if (event.authorityHead.configurationEpoch !== configuration.configurationEpoch
    || event.authorityHead.configurationDigest !== configuration.configurationDigest
    || event.authorityHead.headDigest !== authority.activeHeadDigest
    || event.service.principalId !== configuration.service.principal.principalId
    || event.service.keyEpoch !== configuration.service.principal.keyEpoch
    || event.service.keyFingerprint !== configuration.service.principal.keyFingerprint
    || event.project.projectAuthorityDigest !== configuration.project.projectAuthorityDigest
    || event.project.principalId !== configuration.project.principal.principalId) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_AUTHORITY_CONFIGURATION_MISMATCH');
  }
}

function assertExpectedLeaseResourcePredecessors(
  value: unknown,
  events: readonly ProgrammeCaptureSupervisorRunEventV2[],
): void {
  const lease = events.find(({ eventKind }) => eventKind === 'runner-lease-granted-v2');
  if (!lease) {
    if (value !== null) throw new Error('HARNESS_CAPTURE_SUPERVISOR_RESOURCE_SNAPSHOT_SURPLUS');
    return;
  }
  if (value === null) throw new Error('HARNESS_CAPTURE_SUPERVISOR_RESOURCE_SNAPSHOT_REQUIRED');
  const input = closedRunEventRecordV2(value, 'expected lease resource predecessors');
  assertExactKeys(input, [
    'runnerEnrollmentRecordDigest', 'physicalParentId', 'members',
  ], 'expected lease resource predecessors');
  const members = denseRunEventArrayV2(input.members, 'expected lease resource members');
  const parsedMembers = members.map((entry, index) => {
    const member = closedRunEventRecordV2(entry, `expected lease resource member[${index}]`);
    assertExactKeys(member, ['resourceId', 'priorState'], `expected lease resource member[${index}]`);
    return Object.freeze({
      resourceId: parseRunEventOpaqueIdV2(
        member.resourceId, `expected lease resource member[${index}] ID`,
      ),
      priorState: parseProgrammeCaptureSupervisorPriorResourceStateV2(
        member.priorState, index,
      ) as ProgrammeCaptureSupervisorPriorResourceStateV2,
    });
  });
  const resource = lease.resourceTransition!;
  if (parseRunEventDigestV2(
    input.runnerEnrollmentRecordDigest, 'expected runner enrollment record digest',
  ) !== resource.runnerEnrollmentRecordDigest
    || parseRunEventOpaqueIdV2(
      input.physicalParentId, 'expected physical parent ID',
    ) !== resource.physicalParentId
    || digestValue(parsedMembers) !== digestValue(resource.members)) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_RESOURCE_SNAPSHOT_MISMATCH');
  }
}

function samePreviousGlobal(
  left: ProgrammeCaptureSupervisorPreviousGlobalV2,
  right: ProgrammeCaptureSupervisorPreviousGlobalV2,
): boolean {
  return left.kind === right.kind && left.eventDigest === right.eventDigest
    && left.semanticReceiptDigest === right.semanticReceiptDigest;
}

function samePreviousRun(
  left: ProgrammeCaptureSupervisorRunEventV2['previousRun'],
  right: ProgrammeCaptureSupervisorRunEventV2['previousRun'],
): boolean {
  return left.kind === right.kind && left.eventDigest === right.eventDigest;
}
