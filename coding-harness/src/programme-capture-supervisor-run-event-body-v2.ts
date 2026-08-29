// SPDX-License-Identifier: MIT

import { assertExactKeys, deepFreeze } from './contracts.js';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_RESOURCE_CONFLICT_SET_DOMAIN_V2,
  assertProgrammeCaptureSupervisorAttemptOutcomeDispositionsV2,
  closedRunEventRecordV2,
  denseRunEventArrayV2,
  parseRunEventDigestV2,
  parseRunEventOpaqueIdV2,
  parseRunEventTimestampV2,
  parseRunEventUint64V2,
  type ProgrammeCaptureSupervisorAttemptStartBodyV2,
  type ProgrammeCaptureSupervisorAttemptTerminalBodyV2,
  type ProgrammeCaptureSupervisorAuthorityHeadRefV2,
  type ProgrammeCaptureSupervisorClaimRegisteredBodyV2,
  type ProgrammeCaptureSupervisorFinalWitnessBodyV2,
  type ProgrammeCaptureSupervisorLeaseGrantedBodyV2,
  type ProgrammeCaptureSupervisorPreviousGlobalV2,
  type ProgrammeCaptureSupervisorPreviousRunV2,
  type ProgrammeCaptureSupervisorPriorResourceStateV2,
  type ProgrammeCaptureSupervisorResourceTransitionV2,
  type ProgrammeCaptureSupervisorRunEventBodyV2,
  type ProgrammeCaptureSupervisorRunEventKindV2,
  type ProgrammeCaptureSupervisorRunTerminalBodyV2,
} from './programme-capture-supervisor-run-event-contracts-v2.js';
import { digestValue } from './receipts.js';

const MAXIMUM_RESOURCE_MEMBERS = 64;

const REGISTRATION_OUTCOMES = new Set([
  'registration-changed-replay-v2', 'registration-authenticated-denial-v2',
]);
const PRE_LEASE_OUTCOMES = new Set([
  'pre-lease-admission-failed-v2',
  'pre-lease-pre-review-failed-v2',
  'pre-lease-runner-unavailable-v2',
  'pre-lease-policy-failed-v2',
  'pre-lease-internal-failure-v2',
]);
const LEASED_PRE_START_OUTCOMES = new Set([
  'leased-pre-start-expired-v2', 'leased-pre-start-admission-revoked-v2',
  'leased-pre-start-preflight-failed-v2', 'leased-pre-start-internal-failure-v2',
]);
const ATTEMPT_OUTCOMES = new Set([
  'capture-candidate-complete-v2', 'process-failed-v2', 'attempt-timeout-v2',
  'runner-lost-v2', 'output-missing-v2', 'output-invalid-v2', 'cleanup-failed-v2',
  'egress-violation-v2', 'fence-invalidated-v2', 'attempt-internal-failure-v2',
]);

export function parseProgrammeCaptureSupervisorAuthorityHeadRefV2(
  value: unknown,
): ProgrammeCaptureSupervisorAuthorityHeadRefV2 {
  const input = closedRunEventRecordV2(value, 'supervisor run-event authority head');
  assertExactKeys(input, [
    'configurationEpoch', 'configurationDigest', 'headDigest',
  ], 'supervisor run-event authority head');
  return Object.freeze({
    configurationEpoch: parseRunEventUint64V2(
      input.configurationEpoch, 'run-event configuration epoch', 0n,
    ),
    configurationDigest: parseRunEventDigestV2(
      input.configurationDigest, 'run-event configuration digest',
    ),
    headDigest: parseRunEventDigestV2(input.headDigest, 'run-event authority head digest'),
  });
}

export function parseProgrammeCaptureSupervisorPreviousGlobalV2(
  value: unknown,
): ProgrammeCaptureSupervisorPreviousGlobalV2 {
  const input = closedRunEventRecordV2(value, 'supervisor prior global event');
  assertExactKeys(input, [
    'kind', 'eventDigest', 'semanticReceiptDigest',
  ], 'supervisor prior global event');
  const semanticReceiptDigest = parseRunEventDigestV2(
    input.semanticReceiptDigest, 'prior semantic receipt digest',
  );
  if (input.kind === 'authority-genesis' && input.eventDigest === null) {
    return Object.freeze({ kind: 'authority-genesis', eventDigest: null, semanticReceiptDigest });
  }
  if (input.kind === 'semantic-event') {
    return Object.freeze({
      kind: 'semantic-event',
      eventDigest: parseRunEventDigestV2(input.eventDigest, 'prior global event digest'),
      semanticReceiptDigest,
    });
  }
  throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_PREVIOUS_GLOBAL_INVALID');
}

export function parseProgrammeCaptureSupervisorPreviousRunV2(
  value: unknown,
): ProgrammeCaptureSupervisorPreviousRunV2 {
  const input = closedRunEventRecordV2(value, 'supervisor prior run event');
  assertExactKeys(input, ['kind', 'eventDigest'], 'supervisor prior run event');
  if (input.kind === 'run-genesis' && input.eventDigest === null) {
    return Object.freeze({ kind: 'run-genesis', eventDigest: null });
  }
  if (input.kind === 'run-event') {
    return Object.freeze({
      kind: 'run-event',
      eventDigest: parseRunEventDigestV2(input.eventDigest, 'prior run event digest'),
    });
  }
  throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_PREVIOUS_RUN_INVALID');
}

export function parseProgrammeCaptureSupervisorResourceTransitionV2(
  value: unknown,
): ProgrammeCaptureSupervisorResourceTransitionV2 {
  const input = closedRunEventRecordV2(value, 'supervisor resource transition');
  assertExactKeys(input, [
    'runnerEnrollmentRecordDigest', 'physicalParentId', 'conflictSetDigest',
    'fence', 'members',
  ], 'supervisor resource transition');
  const runnerEnrollmentRecordDigest = parseRunEventDigestV2(
    input.runnerEnrollmentRecordDigest, 'runner enrollment record digest',
  );
  const physicalParentId = parseRunEventOpaqueIdV2(
    input.physicalParentId, 'resource physical parent ID',
  );
  const fence = parseRunEventUint64V2(input.fence, 'resource fence', 1n);
  const entries = denseRunEventArrayV2(input.members, 'resource conflict members');
  if (entries.length === 0 || entries.length > MAXIMUM_RESOURCE_MEMBERS) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RESOURCE_MEMBER_COUNT_INVALID');
  }
  const members = entries.map((entry, index) => parseResourceMember(entry, index));
  const resourceIds = members.map(({ resourceId }) => resourceId);
  if (resourceIds.some((id, index) => index > 0 && resourceIds[index - 1] >= id)) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RESOURCE_MEMBER_ORDER_INVALID');
  }
  if (!resourceIds.includes(physicalParentId)) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_PHYSICAL_PARENT_MEMBER_REQUIRED');
  }
  const conflictSetDigest = parseRunEventDigestV2(
    input.conflictSetDigest, 'resource conflict-set digest',
  );
  const expectedConflictSetDigest = digestValue({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_RESOURCE_CONFLICT_SET_DOMAIN_V2,
    runnerEnrollmentRecordDigest,
    physicalParentId,
    resourceIds,
  });
  if (conflictSetDigest !== expectedConflictSetDigest) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_RESOURCE_CONFLICT_SET_DIGEST_MISMATCH');
  }
  return deepFreeze({
    runnerEnrollmentRecordDigest, physicalParentId, conflictSetDigest, fence, members,
  });
}

export function parseProgrammeCaptureSupervisorRunEventBodyV2(
  eventKind: ProgrammeCaptureSupervisorRunEventKindV2,
  value: unknown,
): ProgrammeCaptureSupervisorRunEventBodyV2 {
  if (eventKind === 'claim-registered-v2') return parseClaimRegistered(value);
  if (eventKind === 'runner-lease-granted-v2') return parseLeaseGranted(value);
  if (eventKind === 'capture-attempt-start-committed-v2') return parseAttemptStart(value);
  if (eventKind === 'capture-run-terminal-v2') return parseRunTerminal(value);
  if (eventKind === 'capture-attempt-terminal-v2') return parseAttemptTerminal(value);
  if (eventKind === 'capture-final-witness-v2') return parseFinalWitness(value);
  throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RUN_EVENT_KIND_INVALID');
}

function parseClaimRegistered(value: unknown): ProgrammeCaptureSupervisorClaimRegisteredBodyV2 {
  const input = exactBody(value, [
    'claimKeyDigest', 'claimDigest', 'rootedClaimValidationDigest',
  ], 'claim registration body');
  return Object.freeze({
    claimKeyDigest: parseRunEventDigestV2(input.claimKeyDigest, 'claim key digest'),
    claimDigest: parseRunEventDigestV2(input.claimDigest, 'claim digest'),
    rootedClaimValidationDigest: parseRunEventDigestV2(
      input.rootedClaimValidationDigest, 'rooted claim validation digest',
    ),
  });
}

function parseLeaseGranted(value: unknown): ProgrammeCaptureSupervisorLeaseGrantedBodyV2 {
  const input = exactBody(value, [
    'registrationEventDigest', 'claimDigest', 'admissionChallengeDigest',
    'admissionEvidenceDigest', 'runner', 'hostEvidenceDigest', 'runnerProfileDigest',
    'controlPolicyDigest', 'preReview', 'lease',
  ], 'runner lease body');
  const runnerInput = exactBody(input.runner, [
    'runnerId', 'enrollmentRecordDigest', 'sessionId', 'bootId', 'keyEpoch',
    'keyFingerprint', 'possessionProofDigest',
  ], 'lease runner');
  const preReviewInput = exactBody(
    input.preReview, ['codexReceiptDigest', 'claudeReceiptDigest'], 'lease pre-review',
  );
  const codexReceiptDigest = parseRunEventDigestV2(
    preReviewInput.codexReceiptDigest, 'Codex pre-review receipt digest',
  );
  const claudeReceiptDigest = parseRunEventDigestV2(
    preReviewInput.claudeReceiptDigest, 'Claude pre-review receipt digest',
  );
  if (codexReceiptDigest === claudeReceiptDigest) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_PRE_REVIEW_SEPARATION_REQUIRED');
  }
  const leaseInput = exactBody(input.lease, [
    'leaseId', 'fence', 'serviceIssuedAt', 'notAfter', 'maxAttempts', 'renew',
    'releaseForReuse', 'reassign', 'reclaim', 'retry',
  ], 'lease policy');
  if (leaseInput.maxAttempts !== 1 || leaseInput.renew !== false
    || leaseInput.releaseForReuse !== false || leaseInput.reassign !== false
    || leaseInput.reclaim !== false || leaseInput.retry !== false) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_SINGLE_USE_LEASE_REQUIRED');
  }
  const serviceIssuedAt = parseRunEventTimestampV2(
    leaseInput.serviceIssuedAt, 'lease service-issued time',
  );
  const notAfter = parseRunEventTimestampV2(leaseInput.notAfter, 'lease not-after time');
  if (new Date(serviceIssuedAt).valueOf() >= new Date(notAfter).valueOf()) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_LEASE_INTERVAL_INVALID');
  }
  return deepFreeze({
    registrationEventDigest: parseRunEventDigestV2(
      input.registrationEventDigest, 'registration event digest',
    ),
    claimDigest: parseRunEventDigestV2(input.claimDigest, 'lease claim digest'),
    admissionChallengeDigest: parseRunEventDigestV2(
      input.admissionChallengeDigest, 'admission challenge digest',
    ),
    admissionEvidenceDigest: parseRunEventDigestV2(
      input.admissionEvidenceDigest, 'admission evidence digest',
    ),
    runner: {
      runnerId: parseRunEventOpaqueIdV2(runnerInput.runnerId, 'runner ID'),
      enrollmentRecordDigest: parseRunEventDigestV2(
        runnerInput.enrollmentRecordDigest, 'runner enrollment record digest',
      ),
      sessionId: parseRunEventOpaqueIdV2(runnerInput.sessionId, 'runner session ID'),
      bootId: parseRunEventOpaqueIdV2(runnerInput.bootId, 'runner boot ID'),
      keyEpoch: parseRunEventUint64V2(runnerInput.keyEpoch, 'runner key epoch', 1n),
      keyFingerprint: parseRunEventDigestV2(
        runnerInput.keyFingerprint, 'runner key fingerprint',
      ),
      possessionProofDigest: parseRunEventDigestV2(
        runnerInput.possessionProofDigest, 'runner possession proof digest',
      ),
    },
    hostEvidenceDigest: parseRunEventDigestV2(input.hostEvidenceDigest, 'host evidence digest'),
    runnerProfileDigest: parseRunEventDigestV2(
      input.runnerProfileDigest, 'runner profile digest',
    ),
    controlPolicyDigest: parseRunEventDigestV2(
      input.controlPolicyDigest, 'runner control policy digest',
    ),
    preReview: { codexReceiptDigest, claudeReceiptDigest },
    lease: {
      leaseId: parseRunEventOpaqueIdV2(leaseInput.leaseId, 'lease ID'),
      fence: parseRunEventUint64V2(leaseInput.fence, 'lease fence', 1n),
      serviceIssuedAt, notAfter, maxAttempts: 1,
      renew: false, releaseForReuse: false, reassign: false, reclaim: false, retry: false,
    },
  });
}

function parseAttemptStart(value: unknown): ProgrammeCaptureSupervisorAttemptStartBodyV2 {
  const input = exactBody(value, [
    'leaseEventDigest', 'leaseId', 'fence', 'runner', 'resourceConflictSetDigest',
    'quiescenceDigest', 'freshHostPreflightDigest', 'heldSourceDigest',
    'producerAgreementDigest', 'producerArtifactDigest', 'producerRuntimeClosureDigest',
    'commandDigest', 'environmentDigest', 'outputSlotDigest', 'captureNonceDigest',
    'attemptId',
  ], 'attempt start body');
  const runner = exactBody(
    input.runner, ['runnerId', 'sessionId', 'bootId'], 'attempt-start runner',
  );
  return deepFreeze({
    leaseEventDigest: digestField(input, 'leaseEventDigest'),
    leaseId: idField(input, 'leaseId'),
    fence: uintField(input, 'fence', 1n),
    runner: {
      runnerId: idField(runner, 'runnerId'),
      sessionId: idField(runner, 'sessionId'),
      bootId: idField(runner, 'bootId'),
    },
    resourceConflictSetDigest: digestField(input, 'resourceConflictSetDigest'),
    quiescenceDigest: digestField(input, 'quiescenceDigest'),
    freshHostPreflightDigest: digestField(input, 'freshHostPreflightDigest'),
    heldSourceDigest: digestField(input, 'heldSourceDigest'),
    producerAgreementDigest: digestField(input, 'producerAgreementDigest'),
    producerArtifactDigest: digestField(input, 'producerArtifactDigest'),
    producerRuntimeClosureDigest: digestField(input, 'producerRuntimeClosureDigest'),
    commandDigest: digestField(input, 'commandDigest'),
    environmentDigest: digestField(input, 'environmentDigest'),
    outputSlotDigest: digestField(input, 'outputSlotDigest'),
    captureNonceDigest: digestField(input, 'captureNonceDigest'),
    attemptId: idField(input, 'attemptId'),
  });
}

function parseRunTerminal(value: unknown): ProgrammeCaptureSupervisorRunTerminalBodyV2 {
  const input = exactBody(value, [
    'terminalStage', 'outcomeCode', 'registrationEventDigest', 'outcomeEvidenceDigest',
    'leaseEventDigest', 'leaseId', 'fence', 'resourceDisposition', 'attemptId',
    'captureRecordDigest', 'outputEnvelopeDigest', 'cleanupEvidenceDigest',
  ], 'pre-start terminal body');
  if (input.attemptId !== null || input.captureRecordDigest !== null
    || input.outputEnvelopeDigest !== null || input.cleanupEvidenceDigest !== null) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_PRE_START_TERMINAL_NULLABILITY_INVALID');
  }
  const terminalStage = input.terminalStage;
  const outcomeCode = input.outcomeCode;
  const outcomeSet = terminalStage === 'registration' ? REGISTRATION_OUTCOMES
    : terminalStage === 'pre-lease' ? PRE_LEASE_OUTCOMES
      : terminalStage === 'leased-pre-start' ? LEASED_PRE_START_OUTCOMES : null;
  if (!outcomeSet || typeof outcomeCode !== 'string' || !outcomeSet.has(outcomeCode)) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RUN_TERMINAL_OUTCOME_INVALID');
  }
  const leased = terminalStage === 'leased-pre-start';
  const leaseReferences = [
    input.leaseEventDigest, input.leaseId, input.fence, input.resourceDisposition,
  ];
  const completeLeaseReferences = leaseReferences.every((entry) => entry !== null);
  const absentLeaseReferences = leaseReferences.every((entry) => entry === null);
  if ((leased && !completeLeaseReferences) || (!leased && !absentLeaseReferences)) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RUN_TERMINAL_LEASE_REFS_INVALID');
  }
  const resourceDisposition = leased
    ? parsePreStartResourceDisposition(input.resourceDisposition) : null;
  return deepFreeze({
    terminalStage,
    outcomeCode,
    registrationEventDigest: digestField(input, 'registrationEventDigest'),
    outcomeEvidenceDigest: digestField(input, 'outcomeEvidenceDigest'),
    leaseEventDigest: leased ? digestField(input, 'leaseEventDigest') : null,
    leaseId: leased ? idField(input, 'leaseId') : null,
    fence: leased ? uintField(input, 'fence', 1n) : null,
    resourceDisposition,
    attemptId: null, captureRecordDigest: null, outputEnvelopeDigest: null,
    cleanupEvidenceDigest: null,
  }) as ProgrammeCaptureSupervisorRunTerminalBodyV2;
}

function parseAttemptTerminal(value: unknown): ProgrammeCaptureSupervisorAttemptTerminalBodyV2 {
  const input = exactBody(value, [
    'startEventDigest', 'leaseEventDigest', 'leaseId', 'fence', 'attemptId',
    'outcomeCode', 'outcomeEvidenceDigest', 'processDisposition', 'egressDisposition',
    'leaseDisposition', 'resourceDisposition', 'cleanup', 'outputEnvelopeDigest',
    'captureRecordDigest', 'finalStateDigest',
  ], 'attempt terminal body');
  if (typeof input.outcomeCode !== 'string' || !ATTEMPT_OUTCOMES.has(input.outcomeCode)) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_ATTEMPT_OUTCOME_INVALID');
  }
  const processDisposition = disposition(input.processDisposition, [
    'exited-zero', 'exited-nonzero', 'terminated', 'runner-lost', 'unknown',
  ], 'process disposition');
  const egressDisposition = disposition(input.egressDisposition, [
    'isolated-no-violation', 'violation-detected', 'unknown',
  ], 'egress disposition');
  const leaseDisposition = disposition(
    input.leaseDisposition, ['spent-never-reusable'], 'lease disposition',
  );
  const resourceDisposition = disposition(input.resourceDisposition, [
    'released-after-cleanup', 'quarantined',
  ], 'resource disposition');
  const cleanupInput = exactBody(input.cleanup, [
    'processCleanupDigest', 'egressCleanupDigest', 'resourceCleanupDigest',
  ], 'attempt cleanup');
  const cleanup = {
    processCleanupDigest: nullableDigest(cleanupInput.processCleanupDigest, 'process cleanup'),
    egressCleanupDigest: nullableDigest(cleanupInput.egressCleanupDigest, 'egress cleanup'),
    resourceCleanupDigest: nullableDigest(cleanupInput.resourceCleanupDigest, 'resource cleanup'),
  };
  const outputEnvelopeDigest = nullableDigest(input.outputEnvelopeDigest, 'output envelope');
  const captureRecordDigest = nullableDigest(input.captureRecordDigest, 'capture record');
  const finalStateDigest = nullableDigest(input.finalStateDigest, 'final state');
  const outputs = [outputEnvelopeDigest, captureRecordDigest, finalStateDigest];
  if (outputs.some((entry) => entry === null) !== outputs.every((entry) => entry === null)) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_ATTEMPT_OUTPUT_SET_INVALID');
  }
  assertProgrammeCaptureSupervisorAttemptOutcomeDispositionsV2({
    outcomeCode: input.outcomeCode,
    processKind: processDisposition.kind,
    egressKind: egressDisposition.kind,
    resourceKind: resourceDisposition.kind,
    cleanupComplete: Object.values(cleanup).every((entry) => entry !== null),
    outputsPresent: outputs.every((entry) => entry !== null),
  });
  return deepFreeze({
    startEventDigest: digestField(input, 'startEventDigest'),
    leaseEventDigest: digestField(input, 'leaseEventDigest'),
    leaseId: idField(input, 'leaseId'),
    fence: uintField(input, 'fence', 1n),
    attemptId: idField(input, 'attemptId'),
    outcomeCode: input.outcomeCode,
    outcomeEvidenceDigest: digestField(input, 'outcomeEvidenceDigest'),
    processDisposition, egressDisposition, leaseDisposition, resourceDisposition,
    cleanup, outputEnvelopeDigest, captureRecordDigest, finalStateDigest,
  }) as ProgrammeCaptureSupervisorAttemptTerminalBodyV2;
}

function parseFinalWitness(value: unknown): ProgrammeCaptureSupervisorFinalWitnessBodyV2 {
  const input = exactBody(value, [
    'attemptTerminalEventDigest', 'leaseEventDigest', 'leaseId', 'fence', 'attemptId',
    'frozenEnvelopeDigest', 'captureRecordDigest', 'finalStateDigest',
    'replayValidationDigest', 'postReview',
  ], 'capture final witness body');
  const review = exactBody(
    input.postReview, ['codexReceiptDigest', 'claudeReceiptDigest'], 'post-run review',
  );
  const codexReceiptDigest = digestField(review, 'codexReceiptDigest');
  const claudeReceiptDigest = digestField(review, 'claudeReceiptDigest');
  if (codexReceiptDigest === claudeReceiptDigest) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_POST_REVIEW_SEPARATION_REQUIRED');
  }
  return deepFreeze({
    attemptTerminalEventDigest: digestField(input, 'attemptTerminalEventDigest'),
    leaseEventDigest: digestField(input, 'leaseEventDigest'),
    leaseId: idField(input, 'leaseId'),
    fence: uintField(input, 'fence', 1n),
    attemptId: idField(input, 'attemptId'),
    frozenEnvelopeDigest: digestField(input, 'frozenEnvelopeDigest'),
    captureRecordDigest: digestField(input, 'captureRecordDigest'),
    finalStateDigest: digestField(input, 'finalStateDigest'),
    replayValidationDigest: digestField(input, 'replayValidationDigest'),
    postReview: { codexReceiptDigest, claudeReceiptDigest },
  });
}

function parseResourceMember(value: unknown, index: number) {
  const input = exactBody(
    value, ['resourceId', 'priorState'], `resource conflict member[${index}]`,
  );
  return Object.freeze({
    resourceId: parseRunEventOpaqueIdV2(input.resourceId, `resource member[${index}] ID`),
    priorState: parseProgrammeCaptureSupervisorPriorResourceStateV2(input.priorState, index),
  });
}

export function parseProgrammeCaptureSupervisorPriorResourceStateV2(
  value: unknown,
  index: number,
): ProgrammeCaptureSupervisorPriorResourceStateV2 {
  const input = exactBody(
    value, ['kind', 'eventDigest', 'fence'], `resource member[${index}] prior state`,
  );
  if (input.kind === 'resource-genesis' && input.eventDigest === null && input.fence === null) {
    return Object.freeze({ kind: 'resource-genesis', eventDigest: null, fence: null });
  }
  if (input.kind === 'resource-event') {
    return Object.freeze({
      kind: 'resource-event',
      eventDigest: digestField(input, 'eventDigest'),
      fence: uintField(input, 'fence', 1n),
    });
  }
  throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RESOURCE_PRIOR_STATE_INVALID');
}

function parsePreStartResourceDisposition(value: unknown) {
  return disposition(value, ['released-unstarted', 'quarantined'], 'pre-start resource disposition');
}

function disposition(value: unknown, kinds: readonly string[], label: string) {
  const input = exactBody(value, ['kind', 'evidenceDigest'], label);
  if (typeof input.kind !== 'string' || !kinds.includes(input.kind)) {
    throw new TypeError(`HARNESS_CAPTURE_SUPERVISOR_${label.toUpperCase().replaceAll(' ', '_')}_INVALID`);
  }
  return Object.freeze({
    kind: input.kind,
    evidenceDigest: parseRunEventDigestV2(input.evidenceDigest, `${label} evidence digest`),
  });
}

function exactBody(value: unknown, keys: readonly string[], label: string) {
  const input = closedRunEventRecordV2(value, label);
  assertExactKeys(input, keys, label);
  return input;
}

function digestField(input: Record<string, unknown>, key: string): string {
  return parseRunEventDigestV2(input[key], key.replaceAll(/([A-Z])/g, ' $1').toLowerCase());
}

function idField(input: Record<string, unknown>, key: string): string {
  return parseRunEventOpaqueIdV2(input[key], key.replaceAll(/([A-Z])/g, ' $1').toLowerCase());
}

function uintField(input: Record<string, unknown>, key: string, minimum: bigint): string {
  return parseRunEventUint64V2(
    input[key], key.replaceAll(/([A-Z])/g, ' $1').toLowerCase(), minimum,
  );
}

function nullableDigest(value: unknown, label: string): string | null {
  return value === null ? null : parseRunEventDigestV2(value, `${label} digest`);
}
