// SPDX-License-Identifier: MIT

import {
  parseCanonicalRegistrationRequestV2,
  registrationChangedReplayEvidenceDigestV2,
  type AuthorityHeadRefV2,
  type CanonicalRegistrationRequestV2,
} from './registration-protocol-v2.js';
import {
  canonicalJsonV1, closedRecordV1, deepFreezeV1, exactKeysV1,
  parseDigestV1, parseOpaqueIdV1, parseUint64V1, snapshotClosedGraphV1,
  successorUint64V1,
} from './registration-postgresql-canonical-v1.js';
import {
  parseAuthorityStateV1, parseLockedConfigurationV1,
  parsePredecessorReceiptV1, parseRunStateV1,
  type NormalizedAuthorityStateV1, type NormalizedLockedConfigurationV1,
  type NormalizedPredecessorReceiptV1, type NormalizedRunStateV1,
} from './registration-postgresql-locked-snapshots-v1.js';

const CANDIDATE_KEYS = Object.freeze([
  'transactionScope', 'project', 'authorityHead', 'expectedNextGlobalSequence',
  'previousGlobal', 'request', 'candidateKind', 'expectedRunState', 'runSequence',
  'previousRun', 'priorControllerStateHeadDigest', 'resourceTransition', 'body',
]);
const LOCKED_SNAPSHOT_KEYS = Object.freeze([
  'lockedConfiguration', 'lockedAuthorityState',
  'lockedPredecessorReceipt', 'lockedRunState',
]);

export interface NormalizedCandidateV1 {
  readonly status: 201 | 409;
  readonly candidateKind: 'claim-registered-v2' | 'capture-run-terminal-v2';
  readonly project: Readonly<{
    projectAuthorityDigest: string; principalId: string; authenticationPolicyDigest: string;
  }>;
  readonly authorityHead: AuthorityHeadRefV2;
  readonly globalSequence: string;
  readonly previousGlobal: Readonly<{
    kind: 'authority-genesis' | 'semantic-event';
    eventDigest: string | null; semanticReceiptDigest: string;
  }>;
  readonly request: CanonicalRegistrationRequestV2;
  readonly expectedRunState: Readonly<Record<string, unknown>>;
  readonly runSequence: '0' | '1';
  readonly previousRun: Readonly<{
    kind: 'run-genesis' | 'run-event'; eventDigest: string | null;
  }>;
  readonly priorControllerStateHeadDigest: string;
  readonly body: Readonly<Record<string, unknown>>;
}

export interface NormalizedMaterializationInputsV1 {
  readonly rawCandidate: Readonly<Record<string, unknown>>;
  readonly rawLockedSnapshots: Readonly<Record<string, unknown>>;
  readonly candidate: NormalizedCandidateV1;
  readonly configuration: NormalizedLockedConfigurationV1;
  readonly authorityState: NormalizedAuthorityStateV1;
  readonly predecessorReceipt: NormalizedPredecessorReceiptV1;
  readonly runState: NormalizedRunStateV1;
}

export async function parsePostgresMaterializationInputsV1(
  candidateValue: unknown,
  snapshotsValue: unknown,
): Promise<NormalizedMaterializationInputsV1> {
  const candidateIngress = closedRecordV1(
    candidateValue, 'registration materializer candidate', 64,
  );
  exactKeysV1(candidateIngress, CANDIDATE_KEYS, 'registration materializer candidate');
  const snapshotsIngress = closedRecordV1(
    snapshotsValue, 'registration materializer snapshots', 16,
  );
  exactKeysV1(
    snapshotsIngress, LOCKED_SNAPSHOT_KEYS, 'registration materializer snapshots',
  );
  const rawCandidate = closedRecordV1(
    snapshotClosedGraphV1(candidateIngress, 'registration materializer candidate'),
    'registration materializer candidate',
  );
  const rawLockedSnapshots = closedRecordV1(
    snapshotClosedGraphV1(snapshotsIngress, 'registration materializer snapshots'),
    'registration materializer snapshots',
  );
  exactKeysV1(rawLockedSnapshots, LOCKED_SNAPSHOT_KEYS, 'registration materializer snapshots');
  const candidate = await parseCandidate(rawCandidate);
  const configuration = parseLockedConfigurationV1(rawLockedSnapshots.lockedConfiguration);
  const authorityState = parseAuthorityStateV1(rawLockedSnapshots.lockedAuthorityState);
  const predecessorReceipt = parsePredecessorReceiptV1(
    rawLockedSnapshots.lockedPredecessorReceipt,
  );
  const runState = parseRunStateV1(rawLockedSnapshots.lockedRunState);
  bindConfiguration(candidate, configuration, authorityState);
  bindGlobalPredecessor(candidate, configuration, authorityState, predecessorReceipt);
  bindRun(candidate, configuration, runState);
  return deepFreezeV1({
    rawCandidate, rawLockedSnapshots, candidate, configuration,
    authorityState, predecessorReceipt, runState,
  });
}

async function parseCandidate(input: Record<string, unknown>): Promise<NormalizedCandidateV1> {
  exactKeysV1(input, CANDIDATE_KEYS, 'registration materializer candidate');
  if (input.transactionScope !== 'same-serializable-transaction-required'
    || input.resourceTransition !== null) throw new TypeError('candidate scope is invalid');
  const status = input.candidateKind === 'claim-registered-v2' ? 201
    : input.candidateKind === 'capture-run-terminal-v2' ? 409 : null;
  if (status === null) throw new TypeError('candidate kind is invalid');
  const projectInput = closedRecordV1(input.project, 'candidate project');
  exactKeysV1(projectInput, [
    'projectAuthorityDigest', 'principalId', 'authenticationPolicyDigest',
  ], 'candidate project');
  const project = deepFreezeV1({
    projectAuthorityDigest: parseDigestV1(
      projectInput.projectAuthorityDigest, 'candidate project digest',
    ),
    principalId: parseOpaqueIdV1(projectInput.principalId, 'candidate project principal'),
    authenticationPolicyDigest: parseDigestV1(
      projectInput.authenticationPolicyDigest, 'candidate authentication policy digest',
    ),
  });
  const authorityHead = parseHead(input.authorityHead, 'candidate authority head');
  const globalSequence = parseUint64V1(
    input.expectedNextGlobalSequence, 'candidate global sequence', 1n,
  );
  const previousGlobal = parsePreviousGlobal(input.previousGlobal);
  const requestInput = closedRecordV1(input.request, 'candidate request snapshot');
  exactKeysV1(requestInput, [
    'serialized', 'serializedSha256', 'semanticRequestDigest', 'authorityHead',
    'assertedProject', 'runId', 'priorControllerStateHeadDigest', 'claim',
  ], 'candidate request snapshot');
  if (typeof requestInput.serialized !== 'string') throw new TypeError('request bytes missing');
  const request = await parseCanonicalRegistrationRequestV2(requestInput.serialized);
  if (canonicalJsonV1(requestInput) !== canonicalJsonV1(request)
    || !sameProject(project, request.assertedProject)
    || !sameHead(authorityHead, request.authorityHead)) {
    throw new TypeError('candidate request snapshot mismatch');
  }
  const expectedRunState = parseExpectedRun(input.expectedRunState, status);
  const previousRun = parsePreviousRun(input.previousRun);
  const runSequence = parseUint64V1(input.runSequence, 'candidate run sequence');
  if (runSequence !== (status === 201 ? '0' : '1')) {
    throw new TypeError('candidate run sequence is invalid');
  }
  const body = status === 201
    ? parseRegistrationBody(input.body, request)
    : await parseChangedReplayBody(input.body, request, project, authorityHead, expectedRunState);
  return deepFreezeV1({
    status,
    candidateKind: input.candidateKind as NormalizedCandidateV1['candidateKind'],
    project, authorityHead, globalSequence, previousGlobal, request, expectedRunState,
    runSequence: runSequence as '0' | '1', previousRun,
    priorControllerStateHeadDigest: parseDigestV1(
      input.priorControllerStateHeadDigest, 'candidate prior controller head',
    ),
    body,
  });
}

function bindConfiguration(
  candidate: NormalizedCandidateV1,
  configuration: NormalizedLockedConfigurationV1,
  state: NormalizedAuthorityStateV1,
): void {
  if (candidate.project.projectAuthorityDigest !== configuration.projectAuthorityDigest
    || candidate.project.principalId !== configuration.projectPrincipalId
    || candidate.project.authenticationPolicyDigest
      !== configuration.projectAuthenticationPolicyDigest
    || state.projectAuthorityDigest !== configuration.projectAuthorityDigest
    || state.projectScopeRole !== configuration.projectScopeRole
    || candidate.authorityHead.configurationEpoch !== configuration.configurationEpoch
    || candidate.authorityHead.configurationDigest !== configuration.configurationDigest
    || candidate.authorityHead.headDigest !== configuration.genesisAuthorityHeadDigest
    || state.activeConfigurationEpoch !== configuration.configurationEpoch
    || state.activeConfigurationDigest !== configuration.configurationDigest
    || state.authorityHeadDigest !== configuration.genesisAuthorityHeadDigest
    || candidate.globalSequence !== state.nextGlobalSequence) {
    throw new TypeError('candidate/configuration/authority-state binding mismatch');
  }
  successorUint64V1(candidate.globalSequence, 'candidate global sequence');
}

function bindGlobalPredecessor(
  candidate: NormalizedCandidateV1,
  configuration: NormalizedLockedConfigurationV1,
  state: NormalizedAuthorityStateV1,
  receipt: NormalizedPredecessorReceiptV1,
): void {
  if (receipt.projectAuthorityDigest !== configuration.projectAuthorityDigest
    || receipt.projectScopeRole !== configuration.projectScopeRole) {
    throw new TypeError('predecessor receipt scope mismatch');
  }
  if (state.lastGlobalSequence === '0') {
    if (receipt.kind !== 'authority-genesis'
      || receipt.configurationEpoch !== configuration.configurationEpoch
      || receipt.configurationDigest !== configuration.configurationDigest
      || receipt.semanticReceiptDigest !== configuration.genesisSemanticReceiptDigest
      || candidate.previousGlobal.kind !== 'authority-genesis'
      || candidate.previousGlobal.eventDigest !== null
      || candidate.previousGlobal.semanticReceiptDigest !== receipt.semanticReceiptDigest) {
      throw new TypeError('genesis predecessor binding mismatch');
    }
  } else if (receipt.kind !== 'semantic-event' || receipt.eventDigest !== state.lastEventDigest
    || candidate.previousGlobal.kind !== 'semantic-event'
    || candidate.previousGlobal.eventDigest !== receipt.eventDigest
    || candidate.previousGlobal.semanticReceiptDigest !== receipt.semanticReceiptDigest) {
    throw new TypeError('semantic predecessor binding mismatch');
  }
}

function bindRun(
  candidate: NormalizedCandidateV1,
  configuration: NormalizedLockedConfigurationV1,
  run: NormalizedRunStateV1,
): void {
  if (run.projectAuthorityDigest !== configuration.projectAuthorityDigest
    || run.projectScopeRole !== configuration.projectScopeRole
    || run.runId !== candidate.request.runId) throw new TypeError('run scope mismatch');
  if (candidate.status === 201) {
    if (run.kind !== 'absent' || candidate.expectedRunState.kind !== 'absent'
      || candidate.previousRun.kind !== 'run-genesis'
      || candidate.previousRun.eventDigest !== null
      || candidate.priorControllerStateHeadDigest
        !== candidate.request.priorControllerStateHeadDigest) {
      throw new TypeError('registration run-genesis binding mismatch');
    }
    return;
  }
  if (run.kind !== 'registered' || candidate.previousRun.kind !== 'run-event'
    || candidate.previousRun.eventDigest !== run.lastRunEventDigest
    || candidate.priorControllerStateHeadDigest !== run.currentControllerStateHeadDigest
    || candidate.request.semanticRequestDigest === run.originalRegistrationRequestDigest
    || BigInt(candidate.globalSequence) <= BigInt(run.lastRunGlobalSequence)) {
    throw new TypeError('changed-replay run binding mismatch');
  }
  if (BigInt(candidate.globalSequence) === BigInt(run.lastRunGlobalSequence) + 1n
    && (candidate.previousGlobal.kind !== 'semantic-event'
      || candidate.previousGlobal.eventDigest !== run.lastRunEventDigest)) {
    throw new TypeError('adjacent global/run predecessor binding mismatch');
  }
  const expectedProjection = {
    kind: 'registered', projectAuthorityDigest: run.projectAuthorityDigest, runId: run.runId,
    originalRegistrationRequestDigest: run.originalRegistrationRequestDigest,
    originalRegistrationRequestSha256: run.originalRegistrationRequestSha256,
    registrationEventDigest: run.originalRegistrationEventDigest,
    lastRunEventDigest: run.lastRunEventDigest,
    lastRunGlobalSequence: run.lastRunGlobalSequence,
    currentControllerStateHeadDigest: run.currentControllerStateHeadDigest,
    lastRunSequence: run.lastRunSequence,
  };
  if (canonicalJsonV1(candidate.expectedRunState) !== canonicalJsonV1(expectedProjection)) {
    throw new TypeError('candidate expected run does not match locked run');
  }
}

function parseExpectedRun(value: unknown, status: 201 | 409): Readonly<Record<string, unknown>> {
  const input = closedRecordV1(value, 'candidate expected run');
  if (status === 201) {
    exactKeysV1(input, ['kind'], 'candidate expected absent run');
    if (input.kind !== 'absent') throw new TypeError('candidate expected run is invalid');
    return Object.freeze({ kind: 'absent' });
  }
  exactKeysV1(input, [
    'kind', 'projectAuthorityDigest', 'runId', 'originalRegistrationRequestDigest',
    'originalRegistrationRequestSha256', 'registrationEventDigest', 'lastRunEventDigest',
    'lastRunGlobalSequence', 'currentControllerStateHeadDigest', 'lastRunSequence',
  ], 'candidate expected registered run');
  if (input.kind !== 'registered') throw new TypeError('candidate expected run is invalid');
  return input;
}

function parseRegistrationBody(
  value: unknown, request: CanonicalRegistrationRequestV2,
): Readonly<Record<string, unknown>> {
  const input = closedRecordV1(value, 'registration candidate body');
  exactKeysV1(input, [
    'claimKeyDigest', 'claimDigest', 'rootedClaimValidationDigest',
  ], 'registration candidate body');
  if (canonicalJsonV1(input) !== canonicalJsonV1(request.claim)) {
    throw new TypeError('registration candidate claim mismatch');
  }
  return deepFreezeV1({ ...request.claim });
}

async function parseChangedReplayBody(
  value: unknown,
  request: CanonicalRegistrationRequestV2,
  project: NormalizedCandidateV1['project'],
  authorityHead: AuthorityHeadRefV2,
  run: Readonly<Record<string, unknown>>,
): Promise<Readonly<Record<string, unknown>>> {
  const input = closedRecordV1(value, 'changed-replay candidate body');
  exactKeysV1(input, [
    'terminalStage', 'outcomeCode', 'registrationEventDigest', 'outcomeEvidenceDigest',
    'leaseEventDigest', 'leaseId', 'fence', 'resourceDisposition', 'attemptId',
    'captureRecordDigest', 'outputEnvelopeDigest', 'cleanupEvidenceDigest',
  ], 'changed-replay candidate body');
  const originalRequest = parseDigestV1(
    run.originalRegistrationRequestDigest, 'candidate original request',
  );
  const originalEvent = parseDigestV1(run.registrationEventDigest, 'candidate registration event');
  const expectedEvidence = await registrationChangedReplayEvidenceDigestV2({
    originalRegistrationRequestDigest: originalRequest,
    originalRegistrationEventDigest: originalEvent,
    changedRegistrationRequestDigest: request.semanticRequestDigest,
    project: {
      projectAuthorityDigest: project.projectAuthorityDigest,
      principalId: project.principalId,
    },
    authorityHead,
  });
  if (input.terminalStage !== 'registration'
    || input.outcomeCode !== 'registration-changed-replay-v2'
    || input.registrationEventDigest !== originalEvent
    || input.outcomeEvidenceDigest !== expectedEvidence
    || ['leaseEventDigest', 'leaseId', 'fence', 'resourceDisposition', 'attemptId',
      'captureRecordDigest', 'outputEnvelopeDigest', 'cleanupEvidenceDigest']
      .some((key) => input[key] !== null)) {
    throw new TypeError('changed-replay candidate body mismatch');
  }
  return deepFreezeV1({
    terminalStage: 'registration', outcomeCode: 'registration-changed-replay-v2',
    registrationEventDigest: originalEvent, outcomeEvidenceDigest: expectedEvidence,
    leaseEventDigest: null, leaseId: null, fence: null, resourceDisposition: null,
    attemptId: null, captureRecordDigest: null, outputEnvelopeDigest: null,
    cleanupEvidenceDigest: null,
  });
}

function parseHead(value: unknown, label: string): AuthorityHeadRefV2 {
  const input = closedRecordV1(value, label);
  exactKeysV1(input, ['configurationEpoch', 'configurationDigest', 'headDigest'], label);
  return deepFreezeV1({
    configurationEpoch: parseUint64V1(input.configurationEpoch, `${label} epoch`),
    configurationDigest: parseDigestV1(input.configurationDigest, `${label} config`),
    headDigest: parseDigestV1(input.headDigest, `${label} digest`),
  });
}

function parsePreviousGlobal(value: unknown): NormalizedCandidateV1['previousGlobal'] {
  const input = closedRecordV1(value, 'candidate previous global');
  exactKeysV1(input, ['kind', 'eventDigest', 'semanticReceiptDigest'], 'candidate previous global');
  if (input.kind !== 'authority-genesis' && input.kind !== 'semantic-event') {
    throw new TypeError('candidate previous-global kind is invalid');
  }
  return deepFreezeV1({
    kind: input.kind,
    eventDigest: input.kind === 'authority-genesis'
      ? input.eventDigest === null ? null : (() => { throw new TypeError('genesis event invalid'); })()
      : parseDigestV1(input.eventDigest, 'candidate predecessor event'),
    semanticReceiptDigest: parseDigestV1(
      input.semanticReceiptDigest, 'candidate predecessor receipt',
    ),
  });
}

function parsePreviousRun(value: unknown): NormalizedCandidateV1['previousRun'] {
  const input = closedRecordV1(value, 'candidate previous run');
  exactKeysV1(input, ['kind', 'eventDigest'], 'candidate previous run');
  if (input.kind !== 'run-genesis' && input.kind !== 'run-event') {
    throw new TypeError('candidate previous-run kind is invalid');
  }
  return deepFreezeV1({
    kind: input.kind,
    eventDigest: input.kind === 'run-genesis'
      ? input.eventDigest === null ? null : (() => { throw new TypeError('run genesis invalid'); })()
      : parseDigestV1(input.eventDigest, 'candidate previous-run event'),
  });
}

function sameProject(
  left: NormalizedCandidateV1['project'], right: CanonicalRegistrationRequestV2['assertedProject'],
): boolean {
  return left.projectAuthorityDigest === right.projectAuthorityDigest
    && left.principalId === right.principalId;
}

function sameHead(left: AuthorityHeadRefV2, right: AuthorityHeadRefV2): boolean {
  return left.configurationEpoch === right.configurationEpoch
    && left.configurationDigest === right.configurationDigest
    && left.headDigest === right.headDigest;
}
