// SPDX-License-Identifier: MIT

import {
  asRecord, assertExactKeys, cloneClosedRecord, ClosedJsonHashError, deepFreeze,
  parseDigest, parseOpaqueId, parseUint64, sha256Text,
} from './closed-json.js';
import {
  fixedRegistrationTransportResponseV2,
  parseAuthorityHead,
  parseCanonicalRegistrationRequestV2,
  registrationChangedReplayEvidenceDigestV2,
  REGISTRATION_CONTENT_TYPE_V2,
  REGISTRATION_RESULT_MAX_BYTES_V2,
  validateStoredRegistrationResultV2,
  type AuthorityHeadRefV2,
  type CanonicalRegistrationRequestV2,
  type FixedRegistrationTransportResponseV2,
  type RegistrationTransportOutcomeV2,
} from './registration-protocol-v2.js';
import type {
  AuthenticatedTransportPeerV1,
  SupervisorRegistrationDecisionPortsV1,
  TrustedProjectBindingV1,
} from './registration-ports-v1.js';
type ExactResponseDecision = Readonly<{
  decisionKind: 'exact-response'; authority: 'none'; mutationAuthorized: false;
  response: Readonly<{ status: 201 | 409;
    contentType: typeof REGISTRATION_CONTENT_TYPE_V2; body: string }>;
}>;
type FixedResponseDecision = Readonly<{
  decisionKind: 'fixed-response'; authority: 'none'; mutationAuthorized: false;
  response: FixedRegistrationTransportResponseV2;
}>;
type IndeterminateDecision = Readonly<{
  decisionKind: 'indeterminate'; authority: 'none'; mutationAuthorized: false;
  response: FixedRegistrationTransportResponseV2;
}>;
type CandidateDecision = Readonly<{
  decisionKind: 'append-registration-candidate' | 'append-changed-replay-candidate';
  authority: 'none'; mutationAuthorized: false; candidate: Readonly<Record<string, unknown>>;
}>;
export type SupervisorRegistrationDecisionV1 =
  | ExactResponseDecision | FixedResponseDecision | IndeterminateDecision | CandidateDecision;
export type SupervisorRegistrationExactPhaseDecisionV1 =
  | ExactResponseDecision | FixedResponseDecision | IndeterminateDecision
  | Readonly<{
    decisionKind: 'exact-miss'; authority: 'none'; mutationAuthorized: false;
    request: CanonicalRegistrationRequestV2; project: TrustedProjectBindingV1;
  }>;
type ExactPhasePortsV1 = Pick<SupervisorRegistrationDecisionPortsV1,
  'mapAuthenticatedPeer' | 'lookupExactCommittedResult'>;

export async function decideSupervisorRegistrationV1(
  serializedRequest: string,
  authenticatedPeer: AuthenticatedTransportPeerV1,
  ports: SupervisorRegistrationDecisionPortsV1,
): Promise<SupervisorRegistrationDecisionV1> {
  const exactPhase = await decideSupervisorRegistrationExactPhaseV1(
    serializedRequest, authenticatedPeer, ports,
  );
  if (exactPhase.decisionKind !== 'exact-miss') return exactPhase;
  const { request, project } = exactPhase;
  if (request.assertedProject.projectAuthorityDigest !== project.projectAuthorityDigest) {
    return fixed('registration-not-admitted-v2');
  }
  const headRead = await read(() => ports.readActiveAuthorityHead(deepFreeze({
    projectAuthorityDigest: project.projectAuthorityDigest,
  })));
  const head = parseHeadRead(headRead);
  if (head === 'not-admitted') return fixed('registration-not-admitted-v2');
  if (head === null) return indeterminate();
  if (!sameProject(head.project, project)) return indeterminate();
  if (request.assertedProject.principalId !== project.principalId
    || !sameHead(head.authorityHead, request.authorityHead)) {
    return fixed('registration-not-admitted-v2');
  }
  const receiptRead = await read(() => ports.readRequiredPredecessorReceipt(deepFreeze({
    projectAuthorityDigest: project.projectAuthorityDigest,
    requiredPredecessor: head.requiredPredecessor,
  })));
  const receipt = parseReceiptRead(receiptRead, head.requiredPredecessor);
  if (receipt === null) return indeterminate();
  const runRead = await read(() => ports.readRunState(deepFreeze({
    projectAuthorityDigest: project.projectAuthorityDigest,
    runId: request.runId,
  })));
  const run = parseRunRead(runRead, project.projectAuthorityDigest, request.runId);
  if (run === null) return indeterminate();
  if (!runAndGlobalHeadAgree(run, head)) return indeterminate();
  if (receipt === 'pending') {
    if (run.kind !== 'absent' && (
      run.originalRegistrationRequestDigest === request.semanticRequestDigest
      || run.firstChangedReplayRequestDigest === request.semanticRequestDigest
    )) return indeterminate();
    return fixed('registration-authority-pending-v2');
  }
  const common = {
    transactionScope: 'same-serializable-transaction-required', project,
    authorityHead: head.authorityHead,
    expectedNextGlobalSequence: head.expectedNextGlobalSequence,
    previousGlobal: receipt.previousGlobal, request,
  };
  if (run.kind === 'absent') {
    return candidate('append-registration-candidate', {
      ...common,
      candidateKind: 'claim-registered-v2',
      expectedRunState: { kind: 'absent' },
      runSequence: '0',
      previousRun: { kind: 'run-genesis', eventDigest: null },
      priorControllerStateHeadDigest: request.priorControllerStateHeadDigest,
      resourceTransition: null,
      body: request.claim,
    });
  }
  if (run.kind === 'registered') {
    if (run.originalRegistrationRequestDigest === request.semanticRequestDigest) {
      return indeterminate();
    }
    let outcomeEvidenceDigest: string;
    try {
      outcomeEvidenceDigest = await registrationChangedReplayEvidenceDigestV2({
        originalRegistrationRequestDigest: run.originalRegistrationRequestDigest,
        originalRegistrationEventDigest: run.registrationEventDigest,
        changedRegistrationRequestDigest: request.semanticRequestDigest,
        project: {
          projectAuthorityDigest: project.projectAuthorityDigest, principalId: project.principalId,
        },
        authorityHead: head.authorityHead,
      });
    } catch { return indeterminate(); }
    return candidate('append-changed-replay-candidate', {
      ...common,
      candidateKind: 'capture-run-terminal-v2',
      expectedRunState: run,
      runSequence: '1',
      previousRun: { kind: 'run-event', eventDigest: run.registrationEventDigest },
      priorControllerStateHeadDigest: run.currentControllerStateHeadDigest,
      resourceTransition: null,
      body: {
        terminalStage: 'registration',
        outcomeCode: 'registration-changed-replay-v2',
        registrationEventDigest: run.registrationEventDigest,
        outcomeEvidenceDigest,
        leaseEventDigest: null,
        leaseId: null,
        fence: null,
        resourceDisposition: null,
        attemptId: null,
        captureRecordDigest: null,
        outputEnvelopeDigest: null,
        cleanupEvidenceDigest: null,
      },
    });
  }
  if (run.originalRegistrationRequestDigest === request.semanticRequestDigest
    || run.firstChangedReplayRequestDigest === request.semanticRequestDigest) {
    return indeterminate();
  }
  return fixed('registration-closed-v2');
}
export async function decideSupervisorRegistrationExactPhaseV1(
  serializedRequest: string,
  authenticatedPeer: AuthenticatedTransportPeerV1,
  ports: ExactPhasePortsV1,
): Promise<SupervisorRegistrationExactPhaseDecisionV1> {
  if (!immutableAuthenticatedPeer(authenticatedPeer)) {
    return fixed('registration-not-admitted-v2');
  }
  let request: CanonicalRegistrationRequestV2;
  try { request = await parseCanonicalRegistrationRequestV2(serializedRequest); }
  catch (error) {
    return error instanceof ClosedJsonHashError || !(error instanceof TypeError)
      ? indeterminate() : fixed('registration-not-admitted-v2');
  }
  const mapping = await read(() => ports.mapAuthenticatedPeer(authenticatedPeer));
  const project = parseMapping(mapping);
  if (project === 'not-admitted') return fixed('registration-not-admitted-v2');
  if (project === null) return indeterminate();
  const exactRead = await read(() => ports.lookupExactCommittedResult(deepFreeze({
    projectAuthorityDigest: project.projectAuthorityDigest,
    semanticRequestDigest: request.semanticRequestDigest,
  })));
  const exact = await parseExactRead(exactRead, request, project);
  if (exact.kind === 'found') return exact.decision;
  if (exact.kind === 'indeterminate') return indeterminate();
  return deepFreeze({
    decisionKind: 'exact-miss', authority: 'none', mutationAuthorized: false,
    request, project,
  });
}
function immutableAuthenticatedPeer(peer: AuthenticatedTransportPeerV1): boolean {
  return typeof peer === 'symbol';
}
function fixed(outcome: RegistrationTransportOutcomeV2): FixedResponseDecision {
  return deepFreeze({
    decisionKind: 'fixed-response', authority: 'none', mutationAuthorized: false,
    response: fixedRegistrationTransportResponseV2(outcome),
  });
}
function indeterminate(): IndeterminateDecision {
  return deepFreeze({
    decisionKind: 'indeterminate', authority: 'none', mutationAuthorized: false,
    response: fixedRegistrationTransportResponseV2('transaction-resolution-unknown-v2'),
  });
}
function candidate(
  decisionKind: CandidateDecision['decisionKind'],
  value: Record<string, unknown>,
): CandidateDecision {
  return deepFreeze({
    decisionKind, authority: 'none', mutationAuthorized: false,
    candidate: deepFreeze(value),
  });
}
async function read(operation: () => Promise<unknown>): Promise<unknown> {
  try { return cloneClosedRecord(await operation(), 'registration adapter result'); }
  catch { return null; }
}
function parseMapping(value: unknown): TrustedProjectBindingV1 | 'not-admitted' | null {
  try {
    const input = asRecord(value, 'project mapping');
    if (input.kind === 'not-admitted') {
      assertExactKeys(input, ['kind'], 'project mapping denial');
      return 'not-admitted';
    }
    if (input.kind === 'indeterminate') return null;
    assertExactKeys(input, ['kind', 'project'], 'project mapping');
    if (input.kind !== 'mapped') return null;
    return parseTrustedProject(input.project, 'mapped project');
  } catch { return null; }
}

async function parseExactRead(
  value: unknown,
  request: CanonicalRegistrationRequestV2,
  project: TrustedProjectBindingV1,
): Promise<{ kind: 'absent' | 'indeterminate' } | {
  kind: 'found'; decision: ExactResponseDecision;
}> {
  try {
    const input = asRecord(value, 'exact result read');
    if (input.kind === 'absent') {
      assertExactKeys(input, ['kind'], 'exact result miss');
      return { kind: 'absent' };
    }
    if (input.kind !== 'found') return { kind: 'indeterminate' };
    assertExactKeys(input, ['kind', 'row'], 'exact result read');
    const row = asRecord(input.row, 'exact result row');
    assertExactKeys(row, [
      'projectAuthorityDigest', 'semanticRequestDigest',
      'originalRegistrationRequestDigest', 'originalRegistrationEventDigest',
      'originalRegistrationGlobalSequence',
      'changedReplayPriorControllerStateHeadDigest',
      'serializedRequest',
      'serializedRequestSha256', 'responseStatus', 'responseContentType',
      'serializedResponse', 'serializedResponseSha256',
    ], 'exact result row');
    const status = row.responseStatus;
    if (status !== 201 && status !== 409) throw new TypeError('stored status invalid');
    const responseBody = typeof row.serializedResponse === 'string'
      ? row.serializedResponse : '';
    if (responseBody.length === 0 || responseBody.length > REGISTRATION_RESULT_MAX_BYTES_V2) {
      throw new TypeError('stored response byte bounds invalid');
    }
    const originalRegistrationRequestDigest = parseDigest(
      row.originalRegistrationRequestDigest, 'stored original request digest',
    );
    const originalRegistrationEventDigest = parseDigest(
      row.originalRegistrationEventDigest, 'stored original event digest',
    );
    const originalRegistrationGlobalSequence = parseUint64(
      row.originalRegistrationGlobalSequence, 'stored original global sequence',
    );
    if (originalRegistrationGlobalSequence === '0') throw new TypeError('invalid sequence');
    const changedReplayPriorControllerStateHeadDigest = status === 201
      ? row.changedReplayPriorControllerStateHeadDigest === null ? null
        : (() => { throw new TypeError('unexpected changed-replay prior state'); })()
      : parseDigest(row.changedReplayPriorControllerStateHeadDigest, 'changed-replay prior state');
    if (parseDigest(row.projectAuthorityDigest, 'stored project digest')
        !== project.projectAuthorityDigest
      || request.assertedProject.projectAuthorityDigest !== project.projectAuthorityDigest
      || parseDigest(row.semanticRequestDigest, 'stored request digest')
        !== request.semanticRequestDigest
      || row.serializedRequest !== request.serialized
      || parseDigest(row.serializedRequestSha256, 'stored request byte digest')
        !== request.serializedSha256
      || await sha256Text(String(row.serializedRequest)) !== request.serializedSha256
      || row.responseContentType !== REGISTRATION_CONTENT_TYPE_V2
      || parseDigest(row.serializedResponseSha256, 'stored response byte digest')
        !== await sha256Text(responseBody)) {
      throw new TypeError('stored exact result binding invalid');
    }
    await validateStoredRegistrationResultV2(responseBody, {
      semanticRequestDigest: request.semanticRequestDigest,
      originalRegistrationRequestDigest,
      originalRegistrationEventDigest,
      originalRegistrationGlobalSequence,
      changedReplayPriorControllerStateHeadDigest,
      projectAuthorityDigest: project.projectAuthorityDigest,
      principalId: request.assertedProject.principalId,
      runId: request.runId,
      authorityHead: request.authorityHead,
      priorControllerStateHeadDigest: request.priorControllerStateHeadDigest,
      claim: request.claim,
    }, status);
    return { kind: 'found', decision: deepFreeze({
      decisionKind: 'exact-response', authority: 'none', mutationAuthorized: false,
      response: { status, contentType: REGISTRATION_CONTENT_TYPE_V2, body: responseBody },
    }) };
  } catch { return { kind: 'indeterminate' }; }
}
interface ActiveHeadRead {
  readonly project: TrustedProjectBindingV1;
  readonly authorityHead: AuthorityHeadRefV2;
  readonly expectedNextGlobalSequence: string;
  readonly requiredPredecessor: Readonly<{
    kind: 'authority-genesis' | 'semantic-event'; eventDigest: string | null;
  }>;
}
function parseHeadRead(value: unknown): ActiveHeadRead | 'not-admitted' | null {
  try {
    const input = asRecord(value, 'active authority head read');
    if (input.kind === 'not-admitted') {
      assertExactKeys(input, ['kind'], 'authority head denial');
      return 'not-admitted';
    }
    if (input.kind !== 'active') return null;
    assertExactKeys(input, [
      'kind', 'project', 'authorityHead', 'expectedNextGlobalSequence',
      'requiredPredecessor',
    ], 'active authority head');
    const sequence = parseUint64(input.expectedNextGlobalSequence, 'next global sequence');
    if (sequence === '0') throw new TypeError('next global sequence must be positive');
    const requiredPredecessor = parsePredecessor(
      input.requiredPredecessor, 'required predecessor', false,
    );
    if ((sequence === '1') !== (requiredPredecessor.kind === 'authority-genesis')) {
      throw new TypeError('next global sequence and predecessor disagree');
    }
    return deepFreeze({
      project: parseTrustedProject(input.project, 'authority-head project'),
      authorityHead: parseAuthorityHead(input.authorityHead, 'active authority head'),
      expectedNextGlobalSequence: sequence,
      requiredPredecessor,
    });
  } catch { return null; }
}
function parseReceiptRead(
  value: unknown,
  expected: ActiveHeadRead['requiredPredecessor'],
): { previousGlobal: Readonly<Record<string, unknown>> } | 'pending' | null {
  try {
    const input = asRecord(value, 'predecessor receipt read');
    if (input.kind === 'pending') {
      assertExactKeys(input, ['kind'], 'pending predecessor receipt');
      return 'pending';
    }
    if (input.kind !== 'ready') return null;
    assertExactKeys(input, ['kind', 'previousGlobal'], 'ready predecessor receipt');
    const previousGlobal = parsePredecessor(
      input.previousGlobal, 'ready previous-global receipt', true,
    );
    if (previousGlobal.kind !== expected.kind
      || previousGlobal.eventDigest !== expected.eventDigest) return null;
    return deepFreeze({ previousGlobal });
  } catch { return null; }
}
type RunRead = Readonly<Record<string, unknown>> & {
  readonly kind: 'absent' | 'registered' | 'advanced-or-closed';
  readonly originalRegistrationRequestDigest?: string;
  readonly firstChangedReplayRequestDigest?: string | null;
  readonly registrationEventDigest?: string;
  readonly lastRunEventDigest?: string;
  readonly lastRunGlobalSequence?: string;
  readonly currentControllerStateHeadDigest?: string;
};
function parseRunRead(value: unknown, project: string, runId: string): RunRead | null {
  try {
    const input = asRecord(value, 'registration run read');
    if (input.kind === 'absent') {
      assertExactKeys(input, ['kind'], 'absent registration run');
      return deepFreeze({ kind: 'absent' });
    }
    const common = [
      'kind', 'projectAuthorityDigest', 'runId', 'originalRegistrationRequestDigest',
      'registrationEventDigest', 'lastRunEventDigest',
      'lastRunGlobalSequence', 'currentControllerStateHeadDigest', 'lastRunSequence',
    ];
    if (input.kind === 'registered') {
      assertExactKeys(
        input, [...common, 'originalRegistrationRequestSha256'], 'registered run',
      );
      if (input.lastRunSequence !== '0') throw new TypeError('registered sequence invalid');
      parseDigest(input.originalRegistrationRequestSha256, 'original request byte digest');
    } else if (input.kind === 'advanced-or-closed') {
      assertExactKeys(input, [...common, 'firstChangedReplayRequestDigest'], 'closed run');
      const lastRunSequence = parseUint64(input.lastRunSequence, 'closed run sequence');
      if (lastRunSequence === '0') {
        throw new TypeError('closed run sequence invalid');
      }
      if (input.firstChangedReplayRequestDigest !== null) {
        parseDigest(input.firstChangedReplayRequestDigest, 'first changed request digest');
        if (lastRunSequence !== '1') {
          throw new TypeError('changed-replay terminal sequence invalid');
        }
      }
    } else return null;
    if (parseDigest(input.projectAuthorityDigest, 'run project digest') !== project
      || parseOpaqueId(input.runId, 'stored run ID') !== runId) return null;
    const originalRequestDigest = parseDigest(
      input.originalRegistrationRequestDigest, 'original request digest',
    );
    const registrationEventDigest = parseDigest(
      input.registrationEventDigest, 'registration event digest',
    );
    const lastRunEventDigest = parseDigest(input.lastRunEventDigest, 'last run event digest');
    const lastRunGlobalSequence = parseUint64(
      input.lastRunGlobalSequence, 'last run global sequence',
    );
    if (BigInt(lastRunGlobalSequence) < BigInt(String(input.lastRunSequence)) + 1n) {
      throw new TypeError('run-local and global sequences disagree');
    }
    if (input.kind === 'registered' && lastRunEventDigest !== registrationEventDigest) {
      throw new TypeError('registered last run event mismatch');
    }
    if (input.kind === 'advanced-or-closed'
      && input.firstChangedReplayRequestDigest === originalRequestDigest) {
      throw new TypeError('changed-replay request must differ from original');
    }
    parseDigest(input.currentControllerStateHeadDigest, 'current controller state digest');
    return deepFreeze(input) as RunRead;
  } catch { return null; }
}
function runAndGlobalHeadAgree(run: RunRead, head: ActiveHeadRead): boolean {
  if (run.kind === 'absent') return true;
  const nextGlobal = BigInt(head.expectedNextGlobalSequence);
  const minimumNextGlobal = BigInt(String(run.lastRunGlobalSequence)) + 1n;
  return nextGlobal >= minimumNextGlobal
    && (nextGlobal !== minimumNextGlobal
      || (head.requiredPredecessor.kind === 'semantic-event'
        && head.requiredPredecessor.eventDigest === run.lastRunEventDigest));
}

function parseTrustedProject(value: unknown, label: string): TrustedProjectBindingV1 {
  const project = asRecord(value, label);
  assertExactKeys(
    project, ['projectAuthorityDigest', 'principalId', 'authenticationPolicyDigest'], label,
  );
  return deepFreeze({
    projectAuthorityDigest: parseDigest(
      project.projectAuthorityDigest, `${label} authority digest`,
    ),
    principalId: parseOpaqueId(project.principalId, `${label} principal ID`),
    authenticationPolicyDigest: parseDigest(
      project.authenticationPolicyDigest, `${label} authentication policy digest`,
    ),
  });
}

function parsePredecessor(
  value: unknown,
  label: string,
  withReceipt: boolean,
): Readonly<{ kind: 'authority-genesis' | 'semantic-event'; eventDigest: string | null;
  semanticReceiptDigest?: string }> {
  const predecessor = asRecord(value, label);
  assertExactKeys(
    predecessor,
    withReceipt ? ['kind', 'eventDigest', 'semanticReceiptDigest'] : ['kind', 'eventDigest'],
    label,
  );
  if (predecessor.kind !== 'authority-genesis' && predecessor.kind !== 'semantic-event') {
    throw new TypeError(`${label} kind is invalid`);
  }
  const eventDigest = predecessor.kind === 'authority-genesis'
    ? predecessor.eventDigest === null ? null : (() => { throw new TypeError(`${label} invalid`); })()
    : parseDigest(predecessor.eventDigest, `${label} event digest`);
  return deepFreeze({
    kind: predecessor.kind,
    eventDigest,
    ...(withReceipt ? {
      semanticReceiptDigest: parseDigest(
        predecessor.semanticReceiptDigest, `${label} semantic receipt digest`,
      ),
    } : {}),
  });
}

function sameProject(left: TrustedProjectBindingV1, right: TrustedProjectBindingV1): boolean {
  return left.projectAuthorityDigest === right.projectAuthorityDigest
    && left.principalId === right.principalId
    && left.authenticationPolicyDigest === right.authenticationPolicyDigest;
}

function sameHead(left: AuthorityHeadRefV2, right: AuthorityHeadRefV2): boolean {
  return left.configurationEpoch === right.configurationEpoch
    && left.configurationDigest === right.configurationDigest
    && left.headDigest === right.headDigest;
}
