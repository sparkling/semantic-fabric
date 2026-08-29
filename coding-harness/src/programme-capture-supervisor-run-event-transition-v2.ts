// SPDX-License-Identifier: MIT

import { DEVELOPMENT_AUTHORITY, deepFreeze } from './contracts.js';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_CONTROLLER_STATE_HEAD_DOMAIN_V2,
  PROGRAMME_CAPTURE_SUPERVISOR_RUN_STATE_DIGEST_DOMAIN_V2,
  denseRunEventArrayV2,
  parseRunEventDigestV2,
  type ProgrammeCaptureSupervisorAttemptStartBodyV2,
  type ProgrammeCaptureSupervisorAttemptTerminalBodyV2,
  type ProgrammeCaptureSupervisorFinalWitnessBodyV2,
  type ProgrammeCaptureSupervisorLeaseGrantedBodyV2,
  type ProgrammeCaptureSupervisorResourceTransitionV2,
  type ProgrammeCaptureSupervisorRunEventV2,
  type ProgrammeCaptureSupervisorRunTerminalBodyV2,
} from './programme-capture-supervisor-run-event-contracts-v2.js';
import { parseProgrammeCaptureSupervisorRunEventV2 } from './programme-capture-supervisor-run-event-codec-v2.js';
import { digestValue } from './receipts.js';

export type ProgrammeCaptureSupervisorRunPhaseV2 =
  | 'registered'
  | 'leased'
  | 'attempt-started'
  | 'pre-start-terminal'
  | 'candidate-success-awaiting-final'
  | 'failed-final-optional'
  | 'final-witnessed';

export interface ProgrammeCaptureSupervisorRunStateV2 {
  readonly schemaVersion: 2;
  readonly transactionKind: 'programme-capture-v2';
  readonly evidenceKind: 'non-authorizing-supervisor-run-state-v2';
  readonly authority: typeof DEVELOPMENT_AUTHORITY;
  readonly runId: string;
  readonly projectAuthorityDigest: string;
  readonly authorityHead: ProgrammeCaptureSupervisorRunEventV2['authorityHead'];
  readonly service: ProgrammeCaptureSupervisorRunEventV2['service'];
  readonly phase: ProgrammeCaptureSupervisorRunPhaseV2;
  readonly attempts: '0' | '1';
  readonly runSpent: boolean;
  readonly finalWitnessRequired: boolean;
  readonly finalWitnessAllowed: boolean;
  readonly lastEventDigest: string;
  readonly lastGlobalSequence: string;
  readonly lastRunSequence: string;
  readonly controllerStateHeadDigest: string;
  readonly registrationEventDigest: string;
  readonly leaseEventDigest: string | null;
  readonly attemptStartEventDigest: string | null;
  readonly attemptTerminalEventDigest: string | null;
  readonly runTerminalEventDigest: string | null;
  readonly finalWitnessEventDigest: string | null;
  readonly resourceConflictSetDigest: string | null;
  readonly resourceFence: string | null;
  readonly verificationScope: 'complete-run-history-structure-only';
  readonly runAdjacencyVerified: true;
  readonly resourceAdjacencyVerified: true;
  readonly serviceSignatureVerified: false;
  readonly globalOrderVerified: false;
  readonly priorSemanticReceiptVerified: false;
  readonly controllerStateHeadVerified: false;
  readonly resourceHighWaterVerified: false;
  readonly resourceFencingVerified: false;
  readonly stateTransitionAuthorized: false;
  readonly attemptStartAuthorized: false;
  readonly captureAuthorized: false;
  readonly importAuthorized: false;
  readonly promotionAuthorized: false;
  readonly releaseAuthorized: false;
  readonly stateDigest: string;
}

const STATE_NON_AUTHORITY = Object.freeze({
  serviceSignatureVerified: false as const,
  globalOrderVerified: false as const,
  priorSemanticReceiptVerified: false as const,
  controllerStateHeadVerified: false as const,
  resourceHighWaterVerified: false as const,
  resourceFencingVerified: false as const,
  stateTransitionAuthorized: false as const,
  attemptStartAuthorized: false as const,
  captureAuthorized: false as const,
  importAuthorized: false as const,
  promotionAuthorized: false as const,
  releaseAuthorized: false as const,
});

export function programmeCaptureSupervisorControllerStateHeadDigestV2(
  priorControllerStateHeadDigest: unknown,
  eventDigest: unknown,
): string {
  return digestValue({
    domain: PROGRAMME_CAPTURE_SUPERVISOR_CONTROLLER_STATE_HEAD_DOMAIN_V2,
    priorControllerStateHeadDigest: parseRunEventDigestV2(
      priorControllerStateHeadDigest, 'prior controller state-head digest',
    ),
    eventDigest: parseRunEventDigestV2(eventDigest, 'controller state-head event digest'),
  });
}

export function deriveProgrammeCaptureSupervisorRunStateV2(
  value: unknown,
): ProgrammeCaptureSupervisorRunStateV2 {
  const entries = denseRunEventArrayV2(value, 'supervisor run-event history');
  if (entries.length === 0 || entries.length > 5) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_RUN_HISTORY_LENGTH_INVALID');
  }
  const events = entries.map(parseProgrammeCaptureSupervisorRunEventV2);
  const first = events[0];
  if (first.eventKind !== 'claim-registered-v2') {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_RUN_REGISTRATION_REQUIRED');
  }
  const requestDigests = new Set<string>();
  let phase: 'absent' | ProgrammeCaptureSupervisorRunPhaseV2 = 'absent';
  let expectedStateHead = first.priorControllerStateHeadDigest;
  let previous: ProgrammeCaptureSupervisorRunEventV2 | null = null;
  let lease: ProgrammeCaptureSupervisorRunEventV2 | null = null;
  let start: ProgrammeCaptureSupervisorRunEventV2 | null = null;
  let terminal: ProgrammeCaptureSupervisorRunEventV2 | null = null;
  let runTerminal: ProgrammeCaptureSupervisorRunEventV2 | null = null;
  let finalWitness: ProgrammeCaptureSupervisorRunEventV2 | null = null;

  for (let index = 0; index < events.length; index += 1) {
    const event = events[index];
    assertStableIdentity(first, event);
    if (event.runSequence !== String(index)) {
      throw new Error('HARNESS_CAPTURE_SUPERVISOR_RUN_SEQUENCE_INVALID');
    }
    if (previous === null) {
      if (event.previousRun.kind !== 'run-genesis') {
        throw new Error('HARNESS_CAPTURE_SUPERVISOR_RUN_PREDECESSOR_INVALID');
      }
    } else {
      if (event.previousRun.kind !== 'run-event'
        || event.previousRun.eventDigest !== previous.eventDigest) {
        throw new Error('HARNESS_CAPTURE_SUPERVISOR_RUN_PREDECESSOR_INVALID');
      }
      if (BigInt(event.globalSequence) <= BigInt(previous.globalSequence)) {
        throw new Error('HARNESS_CAPTURE_SUPERVISOR_GLOBAL_SEQUENCE_NOT_INCREASING');
      }
    }
    if (event.priorControllerStateHeadDigest !== expectedStateHead) {
      throw new Error('HARNESS_CAPTURE_SUPERVISOR_CONTROLLER_STATE_HEAD_MISMATCH');
    }
    if (requestDigests.has(event.semanticRequestDigest)) {
      throw new Error('HARNESS_CAPTURE_SUPERVISOR_SEMANTIC_REQUEST_REUSED');
    }
    requestDigests.add(event.semanticRequestDigest);
    phase = advancePhase(phase, event);
    if (event.eventKind === 'runner-lease-granted-v2') {
      lease = event;
      assertLeaseBindings(first, event);
      assertLeaseResource(event.resourceTransition!);
    } else if (event.eventKind === 'capture-attempt-start-committed-v2') {
      if (!lease) throw new Error('HARNESS_CAPTURE_SUPERVISOR_LEASE_REQUIRED');
      start = event;
      assertStartBindings(lease, event);
      assertContinuationResource(lease, event, lease.eventDigest);
    } else if (event.eventKind === 'capture-run-terminal-v2') {
      runTerminal = event;
      assertRunTerminalBindings(first, lease, event);
      if ((event.body as ProgrammeCaptureSupervisorRunTerminalBodyV2)
        .terminalStage === 'leased-pre-start') {
        if (!lease) throw new Error('HARNESS_CAPTURE_SUPERVISOR_LEASE_REQUIRED');
        assertContinuationResource(lease, event, lease.eventDigest);
      }
    } else if (event.eventKind === 'capture-attempt-terminal-v2') {
      if (!lease || !start) throw new Error('HARNESS_CAPTURE_SUPERVISOR_START_REQUIRED');
      terminal = event;
      assertAttemptTerminalBindings(lease, start, event);
      assertContinuationResource(lease, event, start.eventDigest);
    } else if (event.eventKind === 'capture-final-witness-v2') {
      if (!lease || !terminal) throw new Error('HARNESS_CAPTURE_SUPERVISOR_TERMINAL_REQUIRED');
      finalWitness = event;
      assertFinalBindings(lease, terminal, event);
    }
    expectedStateHead = programmeCaptureSupervisorControllerStateHeadDigestV2(
      expectedStateHead, event.eventDigest,
    );
    previous = event;
  }
  const last = events.at(-1)!;
  const body = {
    schemaVersion: 2 as const,
    transactionKind: 'programme-capture-v2' as const,
    evidenceKind: 'non-authorizing-supervisor-run-state-v2' as const,
    authority: DEVELOPMENT_AUTHORITY,
    runId: first.runId,
    projectAuthorityDigest: first.project.projectAuthorityDigest,
    authorityHead: first.authorityHead,
    service: first.service,
    phase: phase as ProgrammeCaptureSupervisorRunPhaseV2,
    attempts: start ? '1' as const : '0' as const,
    runSpent: phase === 'pre-start-terminal' || phase === 'candidate-success-awaiting-final'
      || phase === 'failed-final-optional' || phase === 'final-witnessed',
    finalWitnessRequired: phase === 'candidate-success-awaiting-final',
    finalWitnessAllowed: phase === 'candidate-success-awaiting-final'
      || phase === 'failed-final-optional',
    lastEventDigest: last.eventDigest,
    lastGlobalSequence: last.globalSequence,
    lastRunSequence: last.runSequence,
    controllerStateHeadDigest: expectedStateHead,
    registrationEventDigest: first.eventDigest,
    leaseEventDigest: lease?.eventDigest ?? null,
    attemptStartEventDigest: start?.eventDigest ?? null,
    attemptTerminalEventDigest: terminal?.eventDigest ?? null,
    runTerminalEventDigest: runTerminal?.eventDigest ?? null,
    finalWitnessEventDigest: finalWitness?.eventDigest ?? null,
    resourceConflictSetDigest: lease?.resourceTransition?.conflictSetDigest ?? null,
    resourceFence: lease?.resourceTransition?.fence ?? null,
    verificationScope: 'complete-run-history-structure-only' as const,
    runAdjacencyVerified: true as const,
    resourceAdjacencyVerified: true as const,
    ...STATE_NON_AUTHORITY,
  };
  return deepFreeze({
    ...body,
    stateDigest: digestValue({
      domain: PROGRAMME_CAPTURE_SUPERVISOR_RUN_STATE_DIGEST_DOMAIN_V2,
      state: body,
    }),
  });
}

function advancePhase(
  phase: 'absent' | ProgrammeCaptureSupervisorRunPhaseV2,
  event: ProgrammeCaptureSupervisorRunEventV2,
): ProgrammeCaptureSupervisorRunPhaseV2 {
  if (phase === 'absent' && event.eventKind === 'claim-registered-v2') return 'registered';
  if (phase === 'registered' && event.eventKind === 'runner-lease-granted-v2') return 'leased';
  if (phase === 'registered' && event.eventKind === 'capture-run-terminal-v2') {
    const stage = (event.body as ProgrammeCaptureSupervisorRunTerminalBodyV2).terminalStage;
    if (stage === 'registration' || stage === 'pre-lease') return 'pre-start-terminal';
  }
  if (phase === 'leased' && event.eventKind === 'capture-attempt-start-committed-v2') {
    return 'attempt-started';
  }
  if (phase === 'leased' && event.eventKind === 'capture-run-terminal-v2'
    && (event.body as ProgrammeCaptureSupervisorRunTerminalBodyV2)
      .terminalStage === 'leased-pre-start') return 'pre-start-terminal';
  if (phase === 'attempt-started' && event.eventKind === 'capture-attempt-terminal-v2') {
    return (event.body as ProgrammeCaptureSupervisorAttemptTerminalBodyV2).outcomeCode
      === 'capture-candidate-complete-v2'
      ? 'candidate-success-awaiting-final' : 'failed-final-optional';
  }
  if ((phase === 'candidate-success-awaiting-final' || phase === 'failed-final-optional')
    && event.eventKind === 'capture-final-witness-v2') return 'final-witnessed';
  throw new Error('HARNESS_CAPTURE_SUPERVISOR_RUN_TRANSITION_INVALID');
}

function assertStableIdentity(
  first: ProgrammeCaptureSupervisorRunEventV2,
  event: ProgrammeCaptureSupervisorRunEventV2,
): void {
  if (event.runId !== first.runId
    || event.project.projectAuthorityDigest !== first.project.projectAuthorityDigest
    || event.project.principalId !== first.project.principalId
    || event.service.principalId !== first.service.principalId
    || event.service.keyEpoch !== first.service.keyEpoch
    || event.service.keyFingerprint !== first.service.keyFingerprint
    || event.authorityHead.configurationEpoch !== first.authorityHead.configurationEpoch
    || event.authorityHead.configurationDigest !== first.authorityHead.configurationDigest
    || event.authorityHead.headDigest !== first.authorityHead.headDigest) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_RUN_IDENTITY_CHANGED');
  }
}

function assertLeaseBindings(
  registration: ProgrammeCaptureSupervisorRunEventV2,
  lease: ProgrammeCaptureSupervisorRunEventV2,
): void {
  const registrationBody = registration.body as { claimDigest: string };
  const body = lease.body as ProgrammeCaptureSupervisorLeaseGrantedBodyV2;
  if (body.registrationEventDigest !== registration.eventDigest
    || body.claimDigest !== registrationBody.claimDigest) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_LEASE_REGISTRATION_MISMATCH');
  }
}

function assertStartBindings(
  lease: ProgrammeCaptureSupervisorRunEventV2,
  start: ProgrammeCaptureSupervisorRunEventV2,
): void {
  const leaseBody = lease.body as ProgrammeCaptureSupervisorLeaseGrantedBodyV2;
  const body = start.body as ProgrammeCaptureSupervisorAttemptStartBodyV2;
  if (body.leaseEventDigest !== lease.eventDigest || body.leaseId !== leaseBody.lease.leaseId
    || body.fence !== leaseBody.lease.fence || body.runner.runnerId !== leaseBody.runner.runnerId
    || body.runner.sessionId !== leaseBody.runner.sessionId
    || body.runner.bootId !== leaseBody.runner.bootId) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_START_LEASE_MISMATCH');
  }
}

function assertRunTerminalBindings(
  registration: ProgrammeCaptureSupervisorRunEventV2,
  lease: ProgrammeCaptureSupervisorRunEventV2 | null,
  terminal: ProgrammeCaptureSupervisorRunEventV2,
): void {
  const body = terminal.body as ProgrammeCaptureSupervisorRunTerminalBodyV2;
  if (body.registrationEventDigest !== registration.eventDigest) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_RUN_TERMINAL_REGISTRATION_MISMATCH');
  }
  if (body.terminalStage === 'leased-pre-start') {
    const leaseBody = lease?.body as ProgrammeCaptureSupervisorLeaseGrantedBodyV2 | undefined;
    if (!lease || !leaseBody || body.leaseEventDigest !== lease.eventDigest
      || body.leaseId !== leaseBody.lease.leaseId || body.fence !== leaseBody.lease.fence) {
      throw new Error('HARNESS_CAPTURE_SUPERVISOR_RUN_TERMINAL_LEASE_MISMATCH');
    }
  }
}

function assertAttemptTerminalBindings(
  lease: ProgrammeCaptureSupervisorRunEventV2,
  start: ProgrammeCaptureSupervisorRunEventV2,
  terminal: ProgrammeCaptureSupervisorRunEventV2,
): void {
  const leaseBody = lease.body as ProgrammeCaptureSupervisorLeaseGrantedBodyV2;
  const startBody = start.body as ProgrammeCaptureSupervisorAttemptStartBodyV2;
  const body = terminal.body as ProgrammeCaptureSupervisorAttemptTerminalBodyV2;
  if (body.startEventDigest !== start.eventDigest || body.leaseEventDigest !== lease.eventDigest
    || body.leaseId !== leaseBody.lease.leaseId || body.fence !== leaseBody.lease.fence
    || body.attemptId !== startBody.attemptId) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_ATTEMPT_TERMINAL_BINDING_MISMATCH');
  }
}

function assertFinalBindings(
  lease: ProgrammeCaptureSupervisorRunEventV2,
  terminal: ProgrammeCaptureSupervisorRunEventV2,
  finalWitness: ProgrammeCaptureSupervisorRunEventV2,
): void {
  const leaseBody = lease.body as ProgrammeCaptureSupervisorLeaseGrantedBodyV2;
  const terminalBody = terminal.body as ProgrammeCaptureSupervisorAttemptTerminalBodyV2;
  const body = finalWitness.body as ProgrammeCaptureSupervisorFinalWitnessBodyV2;
  if (terminalBody.outputEnvelopeDigest === null || terminalBody.captureRecordDigest === null
    || terminalBody.finalStateDigest === null
    || body.attemptTerminalEventDigest !== terminal.eventDigest
    || body.leaseEventDigest !== lease.eventDigest || body.leaseId !== terminalBody.leaseId
    || body.fence !== terminalBody.fence || body.attemptId !== terminalBody.attemptId
    || body.frozenEnvelopeDigest !== terminalBody.outputEnvelopeDigest
    || body.captureRecordDigest !== terminalBody.captureRecordDigest
    || body.finalStateDigest !== terminalBody.finalStateDigest
    || body.postReview.codexReceiptDigest === leaseBody.preReview.codexReceiptDigest
    || body.postReview.codexReceiptDigest === leaseBody.preReview.claudeReceiptDigest
    || body.postReview.claudeReceiptDigest === leaseBody.preReview.codexReceiptDigest
    || body.postReview.claudeReceiptDigest === leaseBody.preReview.claudeReceiptDigest) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_FINAL_WITNESS_BINDING_MISMATCH');
  }
}

function assertLeaseResource(resource: ProgrammeCaptureSupervisorResourceTransitionV2): void {
  for (const member of resource.members) {
    if (member.priorState.kind === 'resource-event'
      && BigInt(resource.fence) <= BigInt(member.priorState.fence)) {
      throw new Error('HARNESS_CAPTURE_SUPERVISOR_RESOURCE_FENCE_NOT_MONOTONIC');
    }
  }
}

function assertContinuationResource(
  lease: ProgrammeCaptureSupervisorRunEventV2,
  event: ProgrammeCaptureSupervisorRunEventV2,
  expectedPriorEventDigest: string,
): void {
  const baseline = lease.resourceTransition!;
  const resource = event.resourceTransition!;
  if (resource.runnerEnrollmentRecordDigest !== baseline.runnerEnrollmentRecordDigest
    || resource.physicalParentId !== baseline.physicalParentId
    || resource.conflictSetDigest !== baseline.conflictSetDigest
    || resource.fence !== baseline.fence
    || resource.members.length !== baseline.members.length) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_RESOURCE_SET_CHANGED');
  }
  for (let index = 0; index < resource.members.length; index += 1) {
    const member = resource.members[index];
    if (member.resourceId !== baseline.members[index].resourceId
      || member.priorState.kind !== 'resource-event'
      || member.priorState.eventDigest !== expectedPriorEventDigest
      || member.priorState.fence !== baseline.fence) {
      throw new Error('HARNESS_CAPTURE_SUPERVISOR_RESOURCE_PREDECESSOR_MISMATCH');
    }
  }
}
