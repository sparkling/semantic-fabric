// SPDX-License-Identifier: MIT

import { isProxy } from 'node:util/types';
import { parseTaskOpaqueId } from './acceptance-task-v3.js';
import {
  SHA256_PATTERN,
  asClosedRecord,
  asDenseArray,
} from './contracts.js';
import type { DEVELOPMENT_AUTHORITY } from './contracts.js';

export const PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_MAX_BYTES_V2 = 65_536;
export const PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_DIGEST_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-run-event-digest-v2';
export const PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_SIGNING_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-run-event-signing-v2';
export const PROGRAMME_CAPTURE_SUPERVISOR_RESOURCE_CONFLICT_SET_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-resource-conflict-set-v2';
export const PROGRAMME_CAPTURE_SUPERVISOR_CONTROLLER_STATE_HEAD_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-controller-state-head-v2';
export const PROGRAMME_CAPTURE_SUPERVISOR_RUN_STATE_DIGEST_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-run-state-digest-v2';
export const PROGRAMME_CAPTURE_SUPERVISOR_RUN_HISTORY_DIGEST_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-run-history-digest-v2';
export const PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_VALIDATION_DIGEST_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-run-event-validation-digest-v2';
export const PROGRAMME_CAPTURE_SUPERVISOR_RUN_HISTORY_VALIDATION_DIGEST_DOMAIN_V2 =
  'semantic-fabric/programme-capture/supervisor-run-history-validation-digest-v2';
export const PROGRAMME_CAPTURE_SUPERVISOR_MAX_UINT64_V2 = 18_446_744_073_709_551_615n;

const UINT64_DECIMAL_PATTERN = /^(?:0|[1-9][0-9]{0,19})$/;

export const PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_KINDS_V2 = Object.freeze([
  'claim-registered-v2',
  'runner-lease-granted-v2',
  'capture-attempt-start-committed-v2',
  'capture-run-terminal-v2',
  'capture-attempt-terminal-v2',
  'capture-final-witness-v2',
] as const);

export type ProgrammeCaptureSupervisorRunEventKindV2 =
  typeof PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_KINDS_V2[number];

export interface ProgrammeCaptureSupervisorAuthorityHeadRefV2 {
  readonly configurationEpoch: string;
  readonly configurationDigest: string;
  readonly headDigest: string;
}

export type ProgrammeCaptureSupervisorPreviousGlobalV2 = Readonly<
  | {
    kind: 'authority-genesis';
    eventDigest: null;
    semanticReceiptDigest: string;
  }
  | {
    kind: 'semantic-event';
    eventDigest: string;
    semanticReceiptDigest: string;
  }
>;

export type ProgrammeCaptureSupervisorPreviousRunV2 = Readonly<
  | { kind: 'run-genesis'; eventDigest: null }
  | { kind: 'run-event'; eventDigest: string }
>;

export type ProgrammeCaptureSupervisorPriorResourceStateV2 = Readonly<
  | { kind: 'resource-genesis'; eventDigest: null; fence: null }
  | { kind: 'resource-event'; eventDigest: string; fence: string }
>;

export interface ProgrammeCaptureSupervisorResourceMemberV2 {
  readonly resourceId: string;
  readonly priorState: ProgrammeCaptureSupervisorPriorResourceStateV2;
}

export interface ProgrammeCaptureSupervisorResourceTransitionV2 {
  readonly runnerEnrollmentRecordDigest: string;
  readonly physicalParentId: string;
  readonly conflictSetDigest: string;
  readonly fence: string;
  readonly members: readonly ProgrammeCaptureSupervisorResourceMemberV2[];
}

export interface ProgrammeCaptureSupervisorClaimRegisteredBodyV2 {
  readonly claimKeyDigest: string;
  readonly claimDigest: string;
  readonly rootedClaimValidationDigest: string;
}

export interface ProgrammeCaptureSupervisorLeaseGrantedBodyV2 {
  readonly registrationEventDigest: string;
  readonly claimDigest: string;
  readonly admissionChallengeDigest: string;
  readonly admissionEvidenceDigest: string;
  readonly runner: Readonly<{
    runnerId: string;
    enrollmentRecordDigest: string;
    sessionId: string;
    bootId: string;
    keyEpoch: string;
    keyFingerprint: string;
    possessionProofDigest: string;
  }>;
  readonly hostEvidenceDigest: string;
  readonly runnerProfileDigest: string;
  readonly controlPolicyDigest: string;
  readonly preReview: Readonly<{
    codexReceiptDigest: string;
    claudeReceiptDigest: string;
  }>;
  readonly lease: Readonly<{
    leaseId: string;
    fence: string;
    serviceIssuedAt: string;
    notAfter: string;
    maxAttempts: 1;
    renew: false;
    releaseForReuse: false;
    reassign: false;
    reclaim: false;
    retry: false;
  }>;
}

export interface ProgrammeCaptureSupervisorAttemptStartBodyV2 {
  readonly leaseEventDigest: string;
  readonly leaseId: string;
  readonly fence: string;
  readonly runner: Readonly<{
    runnerId: string;
    sessionId: string;
    bootId: string;
  }>;
  readonly resourceConflictSetDigest: string;
  readonly quiescenceDigest: string;
  readonly freshHostPreflightDigest: string;
  readonly heldSourceDigest: string;
  readonly producerAgreementDigest: string;
  readonly producerArtifactDigest: string;
  readonly producerRuntimeClosureDigest: string;
  readonly commandDigest: string;
  readonly environmentDigest: string;
  readonly outputSlotDigest: string;
  readonly captureNonceDigest: string;
  readonly attemptId: string;
}

export type ProgrammeCaptureSupervisorRunTerminalStageV2 =
  | 'registration' | 'pre-lease' | 'leased-pre-start';

export type ProgrammeCaptureSupervisorRunTerminalOutcomeV2 =
  | 'registration-changed-replay-v2'
  | 'registration-authenticated-denial-v2'
  | 'pre-lease-admission-failed-v2'
  | 'pre-lease-pre-review-failed-v2'
  | 'pre-lease-runner-unavailable-v2'
  | 'pre-lease-policy-failed-v2'
  | 'pre-lease-internal-failure-v2'
  | 'leased-pre-start-expired-v2'
  | 'leased-pre-start-admission-revoked-v2'
  | 'leased-pre-start-preflight-failed-v2'
  | 'leased-pre-start-internal-failure-v2';

export interface ProgrammeCaptureSupervisorRunTerminalBodyV2 {
  readonly terminalStage: ProgrammeCaptureSupervisorRunTerminalStageV2;
  readonly outcomeCode: ProgrammeCaptureSupervisorRunTerminalOutcomeV2;
  readonly registrationEventDigest: string;
  readonly outcomeEvidenceDigest: string;
  readonly leaseEventDigest: string | null;
  readonly leaseId: string | null;
  readonly fence: string | null;
  readonly resourceDisposition: Readonly<{
    kind: 'released-unstarted' | 'quarantined';
    evidenceDigest: string;
  }> | null;
  readonly attemptId: null;
  readonly captureRecordDigest: null;
  readonly outputEnvelopeDigest: null;
  readonly cleanupEvidenceDigest: null;
}

export type ProgrammeCaptureSupervisorAttemptOutcomeV2 =
  | 'capture-candidate-complete-v2'
  | 'process-failed-v2'
  | 'attempt-timeout-v2'
  | 'runner-lost-v2'
  | 'output-missing-v2'
  | 'output-invalid-v2'
  | 'cleanup-failed-v2'
  | 'egress-violation-v2'
  | 'fence-invalidated-v2'
  | 'attempt-internal-failure-v2';

export interface ProgrammeCaptureSupervisorAttemptTerminalBodyV2 {
  readonly startEventDigest: string;
  readonly leaseEventDigest: string;
  readonly leaseId: string;
  readonly fence: string;
  readonly attemptId: string;
  readonly outcomeCode: ProgrammeCaptureSupervisorAttemptOutcomeV2;
  readonly outcomeEvidenceDigest: string;
  readonly processDisposition: Readonly<{
    kind: 'exited-zero' | 'exited-nonzero' | 'terminated' | 'runner-lost' | 'unknown';
    evidenceDigest: string;
  }>;
  readonly egressDisposition: Readonly<{
    kind: 'isolated-no-violation' | 'violation-detected' | 'unknown';
    evidenceDigest: string;
  }>;
  readonly leaseDisposition: Readonly<{
    kind: 'spent-never-reusable';
    evidenceDigest: string;
  }>;
  readonly resourceDisposition: Readonly<{
    kind: 'released-after-cleanup' | 'quarantined';
    evidenceDigest: string;
  }>;
  readonly cleanup: Readonly<{
    processCleanupDigest: string | null;
    egressCleanupDigest: string | null;
    resourceCleanupDigest: string | null;
  }>;
  readonly outputEnvelopeDigest: string | null;
  readonly captureRecordDigest: string | null;
  readonly finalStateDigest: string | null;
}

export interface ProgrammeCaptureSupervisorFinalWitnessBodyV2 {
  readonly attemptTerminalEventDigest: string;
  readonly leaseEventDigest: string;
  readonly leaseId: string;
  readonly fence: string;
  readonly attemptId: string;
  readonly frozenEnvelopeDigest: string;
  readonly captureRecordDigest: string;
  readonly finalStateDigest: string;
  readonly replayValidationDigest: string;
  readonly postReview: Readonly<{
    codexReceiptDigest: string;
    claudeReceiptDigest: string;
  }>;
}

export type ProgrammeCaptureSupervisorRunEventBodyV2 =
  | ProgrammeCaptureSupervisorClaimRegisteredBodyV2
  | ProgrammeCaptureSupervisorLeaseGrantedBodyV2
  | ProgrammeCaptureSupervisorAttemptStartBodyV2
  | ProgrammeCaptureSupervisorRunTerminalBodyV2
  | ProgrammeCaptureSupervisorAttemptTerminalBodyV2
  | ProgrammeCaptureSupervisorFinalWitnessBodyV2;

export interface ProgrammeCaptureSupervisorRunEventNonAuthorityV2 {
  readonly externalAdministrationVerified: false;
  readonly deploymentAttestationVerified: false;
  readonly authorityActivationVerified: false;
  readonly serviceSignatureVerified: false;
  readonly priorGlobalEventVerified: false;
  readonly priorSemanticReceiptVerified: false;
  readonly controllerStateHeadVerified: false;
  readonly rootedClaimVerified: false;
  readonly runAdjacencyVerified: false;
  readonly resourceHighWaterVerified: false;
  readonly resourceFencingVerified: false;
  readonly publicCommitmentVerified: false;
  readonly checkpointWitnessQuorumVerified: false;
  readonly semanticWitnessQuorumVerified: false;
  readonly stateTransitionAuthorized: false;
  readonly attemptStartAuthorized: false;
  readonly captureAuthorized: false;
  readonly importAuthorized: false;
  readonly promotionAuthorized: false;
  readonly releaseAuthorized: false;
}

export interface ProgrammeCaptureSupervisorRunEventV2
extends ProgrammeCaptureSupervisorRunEventNonAuthorityV2 {
  readonly schemaVersion: 2;
  readonly transactionKind: 'programme-capture-v2';
  readonly recordKind: 'supervisor-run-event-v2';
  readonly authority: typeof DEVELOPMENT_AUTHORITY;
  readonly eventKind: ProgrammeCaptureSupervisorRunEventKindV2;
  readonly authorityHead: ProgrammeCaptureSupervisorAuthorityHeadRefV2;
  readonly service: Readonly<{
    principalId: string;
    keyEpoch: string;
    keyFingerprint: string;
  }>;
  readonly project: Readonly<{
    projectAuthorityDigest: string;
    principalId: string;
  }>;
  readonly runId: string;
  readonly semanticRequestDigest: string;
  readonly globalSequence: string;
  readonly runSequence: string;
  readonly previousGlobal: ProgrammeCaptureSupervisorPreviousGlobalV2;
  readonly previousRun: ProgrammeCaptureSupervisorPreviousRunV2;
  readonly priorControllerStateHeadDigest: string;
  readonly resourceTransition: ProgrammeCaptureSupervisorResourceTransitionV2 | null;
  readonly body: ProgrammeCaptureSupervisorRunEventBodyV2;
  readonly verificationScope: 'service-signed-structure-only';
  readonly eventDigest: string;
}

export interface ProgrammeCaptureSupervisorRunEventEnvelopeV2 {
  readonly schemaVersion: 2;
  readonly transactionKind: 'programme-capture-v2';
  readonly envelopeKind: 'supervisor-run-event-envelope-v2';
  readonly event: ProgrammeCaptureSupervisorRunEventV2;
  readonly signature: Readonly<{
    algorithm: 'ed25519';
    valueBase64Url: string;
  }>;
}

export function parseRunEventDigestV2(value: unknown, label: string): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || /^0+$/.test(value)) {
    throw new TypeError(`${label} must be a non-zero lowercase SHA-256 digest`);
  }
  return value;
}

export function parseRunEventUint64V2(
  value: unknown,
  label: string,
  minimum: bigint,
): string {
  if (typeof value !== 'string' || !UINT64_DECIMAL_PATTERN.test(value)
    || BigInt(value) < minimum || BigInt(value) > PROGRAMME_CAPTURE_SUPERVISOR_MAX_UINT64_V2) {
    throw new TypeError(`${label} must be a canonical uint64 decimal string >= ${minimum}`);
  }
  return value;
}

export function parseRunEventOpaqueIdV2(value: unknown, label: string): string {
  return parseTaskOpaqueId(value, label);
}

export function parseRunEventTimestampV2(value: unknown, label: string): string {
  if (typeof value !== 'string') throw new TypeError(`${label} must be a canonical ISO timestamp`);
  const parsed = new Date(value);
  if (!Number.isFinite(parsed.valueOf()) || parsed.toISOString() !== value) {
    throw new TypeError(`${label} must be a canonical ISO timestamp`);
  }
  return value;
}

export function assertProgrammeCaptureSupervisorAttemptOutcomeDispositionsV2(
  value: Readonly<{
    outcomeCode: string;
    processKind: string;
    egressKind: string;
    resourceKind: string;
    cleanupComplete: boolean;
    outputsPresent: boolean;
  }>,
): void {
  const released = value.resourceKind === 'released-after-cleanup';
  const mismatch = () => {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_OUTCOME_DISPOSITION_MISMATCH');
  };
  if (released && (!value.cleanupComplete || value.processKind === 'runner-lost'
    || value.processKind === 'unknown' || value.egressKind !== 'isolated-no-violation')) mismatch();
  if (value.outcomeCode === 'capture-candidate-complete-v2'
    && (!value.outputsPresent || value.processKind !== 'exited-zero'
      || value.egressKind !== 'isolated-no-violation' || !released)) mismatch();
  if (value.outcomeCode === 'process-failed-v2'
    && value.processKind !== 'exited-nonzero' && value.processKind !== 'terminated') mismatch();
  if (value.outcomeCode === 'attempt-timeout-v2'
    && value.processKind !== 'terminated' && value.processKind !== 'unknown') mismatch();
  if (value.outcomeCode === 'runner-lost-v2'
    && (value.processKind !== 'runner-lost' || value.egressKind !== 'unknown'
      || value.resourceKind !== 'quarantined')) mismatch();
  if (value.outcomeCode === 'output-missing-v2' && value.outputsPresent) mismatch();
  if (value.outcomeCode === 'cleanup-failed-v2'
    && (value.cleanupComplete || value.resourceKind !== 'quarantined')) mismatch();
  if (value.outcomeCode === 'egress-violation-v2'
    && (value.egressKind !== 'violation-detected'
      || value.resourceKind !== 'quarantined')) mismatch();
  if (value.outcomeCode === 'fence-invalidated-v2'
    && value.resourceKind !== 'quarantined') mismatch();
}

export function closedRunEventRecordV2(value: unknown, label: string): Record<string, unknown> {
  if (isProxy(value)) throw new TypeError(`${label} must not be a Proxy`);
  return asClosedRecord(value, label);
}

export function denseRunEventArrayV2(value: unknown, label: string): unknown[] {
  if (isProxy(value)) throw new TypeError(`${label} must not be a Proxy`);
  return asDenseArray(value, label);
}
