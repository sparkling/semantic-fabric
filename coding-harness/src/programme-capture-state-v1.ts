// SPDX-License-Identifier: MIT

import {
  SHA256_PATTERN,
  asClosedRecord,
  asDenseArray,
  asInteger,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import { parseTaskOpaqueId } from './acceptance-task-v3.js';
import { digestValue } from './receipts.js';

const GENESIS_DIGEST = '0'.repeat(64);

export type ProgrammeCapturePhaseV1 =
  | 'admitted'
  | 'inputs-attested'
  | 'host-preflight-passed'
  | 'pre-review-complete'
  | 'runner-lease-acquired'
  | 'model-boundary-quiesced'
  | 'capture-preflight-passed'
  | 'capture-attempt-started'
  | 'capture-complete'
  | 'process-cleanup-verified'
  | 'capture-verified'
  | 'capture-record-frozen'
  | 'review-boundary-restored'
  | 'post-review-complete'
  | 'sealed'
  | 'failed'
  | 'cancelled'
  | 'timed-out';

type SimpleSuccessKind =
  | 'attest-inputs'
  | 'pass-host-preflight'
  | 'pass-capture-preflight'
  | 'complete-capture'
  | 'verify-process-cleanup'
  | 'verify-capture'
  | 'seal';

type SimpleEventInput = Readonly<{ kind: SimpleSuccessKind; evidenceDigest: string }>;
type ReviewEventInput = Readonly<{
  kind: 'complete-pre-review' | 'complete-post-review';
  evidenceDigest: string;
  codexReviewDigest: string;
  claudeReviewDigest: string;
}>;
type LeaseEventInput = Readonly<{
  kind: 'acquire-runner-lease'; evidenceDigest: string; leaseDigest: string;
}>;
type QuiescenceEventInput = Readonly<{
  kind: 'quiesce-model-boundary';
  evidenceDigest: string;
  processTerminationDigest: string;
  providerEgressRevocationDigest: string;
}>;
type AttemptEventInput = Readonly<{
  kind: 'start-capture-attempt'; evidenceDigest: string; attemptDigest: string;
}>;
type RecordEventInput = Readonly<{
  kind: 'freeze-capture-record'; evidenceDigest: string; captureRecordDigest: string;
}>;
type RestorationEventInput = Readonly<{
  kind: 'restore-review-boundary';
  evidenceDigest: string;
  providerEgressRestorationDigest: string;
}>;
type TerminalEventInput = Readonly<{
  kind: 'fail' | 'cancel' | 'timeout';
  evidenceDigest: string;
  reasonDigest: string;
  processDispositionDigest: string;
  egressDispositionDigest: string;
  leaseDispositionDigest: string;
}>;
type AdmitEventInput = Readonly<{ kind: 'admit'; evidenceDigest: string }>;

export type ProgrammeCaptureEventInputV1 =
  | SimpleEventInput
  | ReviewEventInput
  | LeaseEventInput
  | QuiescenceEventInput
  | AttemptEventInput
  | RecordEventInput
  | RestorationEventInput
  | TerminalEventInput;

type StoredEventInput = ProgrammeCaptureEventInputV1 | AdmitEventInput;

export type ProgrammeCaptureEventV1 = StoredEventInput & Readonly<{
  sequence: number;
  previousDigest: string;
  digest: string;
}>;

export interface ProgrammeCaptureStateV1 {
  readonly schemaVersion: 1;
  readonly transactionKind: 'programme-capture-v1';
  readonly runId: string;
  readonly taskDigest: string;
  readonly claimDigest: string;
  readonly controller: Readonly<{ commit: string; tree: string }>;
  readonly phase: ProgrammeCapturePhaseV1;
  readonly captureAttempts: 0 | 1;
  readonly events: readonly ProgrammeCaptureEventV1[];
}

export interface ProgrammeCaptureStateInputV1 {
  readonly runId: string;
  readonly taskDigest: string;
  readonly claimDigest: string;
  readonly controller: Readonly<{ commit: string; tree: string }>;
  readonly admissionEvidenceDigest: string;
}

const SUCCESS_TRANSITIONS: Readonly<Record<string, readonly [string, ProgrammeCapturePhaseV1]>> = {
  admitted: ['attest-inputs', 'inputs-attested'],
  'inputs-attested': ['pass-host-preflight', 'host-preflight-passed'],
  'host-preflight-passed': ['complete-pre-review', 'pre-review-complete'],
  'pre-review-complete': ['acquire-runner-lease', 'runner-lease-acquired'],
  'runner-lease-acquired': ['quiesce-model-boundary', 'model-boundary-quiesced'],
  'model-boundary-quiesced': ['pass-capture-preflight', 'capture-preflight-passed'],
  'capture-preflight-passed': ['start-capture-attempt', 'capture-attempt-started'],
  'capture-attempt-started': ['complete-capture', 'capture-complete'],
  'capture-complete': ['verify-process-cleanup', 'process-cleanup-verified'],
  'process-cleanup-verified': ['verify-capture', 'capture-verified'],
  'capture-verified': ['freeze-capture-record', 'capture-record-frozen'],
  'capture-record-frozen': ['restore-review-boundary', 'review-boundary-restored'],
  'review-boundary-restored': ['complete-post-review', 'post-review-complete'],
  'post-review-complete': ['seal', 'sealed'],
};
const TERMINAL_PHASES = new Set<ProgrammeCapturePhaseV1>([
  'sealed', 'failed', 'cancelled', 'timed-out',
]);
const SIMPLE_KINDS = new Set<string>([
  'attest-inputs', 'pass-host-preflight',
  'pass-capture-preflight', 'complete-capture', 'verify-process-cleanup',
  'verify-capture', 'seal',
]);

export function createProgrammeCaptureStateV1(
  value: ProgrammeCaptureStateInputV1,
): ProgrammeCaptureStateV1 {
  const input = asClosedRecord(value, 'programme capture state input V1');
  assertExactKeys(input, [
    'runId', 'taskDigest', 'claimDigest', 'controller', 'admissionEvidenceDigest',
  ], 'programme capture state input V1');
  const identity = parseIdentity(input);
  const admissionEvidenceDigest = parseDigest(
    input.admissionEvidenceDigest,
    'programme capture admission evidence',
  );
  return buildState(identity, [sealEvent(
    { kind: 'admit', evidenceDigest: admissionEvidenceDigest },
    [],
    identityAnchor(identity),
  )]);
}

export function transitionProgrammeCaptureStateV1(
  value: ProgrammeCaptureStateV1,
  eventValue: ProgrammeCaptureEventInputV1,
): ProgrammeCaptureStateV1 {
  const state = parseProgrammeCaptureStateV1(value);
  if (TERMINAL_PHASES.has(state.phase)) throw new Error('HARNESS_CAPTURE_STATE_TERMINAL');
  const event = parseEventInput(eventValue, false) as ProgrammeCaptureEventInputV1;
  nextPhase(state.phase, event.kind);
  validateEventAgainstHistory(state.events, event);
  return buildState(state, [...state.events, sealEvent(event, state.events)]);
}

export function parseProgrammeCaptureStateV1(value: unknown): ProgrammeCaptureStateV1 {
  const input = asClosedRecord(value, 'programme capture state V1');
  assertExactKeys(input, [
    'schemaVersion', 'transactionKind', 'runId', 'taskDigest', 'claimDigest',
    'controller', 'phase', 'captureAttempts', 'events',
  ], 'programme capture state V1');
  if (input.schemaVersion !== 1 || input.transactionKind !== 'programme-capture-v1') {
    throw new TypeError('HARNESS_CAPTURE_STATE_IDENTITY_INVALID');
  }
  const identity = parseIdentity(input);
  const eventValues = asDenseArray(input.events, 'programme capture state V1.events');
  if (eventValues.length === 0) {
    throw new TypeError('HARNESS_CAPTURE_STATE_EVENTS_REQUIRED');
  }
  const events = eventValues.map((event, index) => parseStoredEvent(event, index));
  if (events[0].kind !== 'admit') throw new Error('HARNESS_CAPTURE_STATE_ADMISSION_REQUIRED');
  if (events[0].previousDigest !== identityAnchor(identity)) {
    throw new Error('HARNESS_CAPTURE_EVENT_PREVIOUS_DIGEST_INVALID');
  }
  for (let index = 1; index < events.length; index += 1) {
    if (events[index].previousDigest !== events[index - 1].digest) {
      throw new Error('HARNESS_CAPTURE_EVENT_PREVIOUS_DIGEST_INVALID');
    }
  }
  let phase: ProgrammeCapturePhaseV1 = 'admitted';
  let attempts = 0;
  for (let index = 1; index < events.length; index += 1) {
    const event = events[index];
    validateEventAgainstHistory(events.slice(0, index), event);
    phase = nextPhase(phase, event.kind);
    if (event.kind === 'start-capture-attempt') attempts += 1;
  }
  if (input.phase !== phase || input.captureAttempts !== attempts || attempts > 1) {
    throw new Error('HARNESS_CAPTURE_STATE_DERIVATION_INVALID');
  }
  return buildState(identity, events);
}

function parseIdentity(value: unknown): Pick<
  ProgrammeCaptureStateV1,
  'runId' | 'taskDigest' | 'claimDigest' | 'controller'
> {
  const input = asClosedRecord(value, 'programme capture identity');
  const controller = asClosedRecord(input.controller, 'programme capture identity.controller');
  assertExactKeys(controller, ['commit', 'tree'], 'programme capture identity.controller');
  return {
    runId: parseTaskOpaqueId(input.runId, 'programme capture identity.runId'),
    taskDigest: parseDigest(input.taskDigest, 'programme capture identity.taskDigest'),
    claimDigest: parseDigest(input.claimDigest, 'programme capture identity.claimDigest'),
    controller: {
      commit: parseGitObjectId(controller.commit, 'programme capture identity.controller.commit'),
      tree: parseGitObjectId(controller.tree, 'programme capture identity.controller.tree'),
    },
  };
}

function parseEventInput(value: unknown, admitAllowed: boolean): StoredEventInput {
  const input = asClosedRecord(value, 'programme capture event');
  const kind = input.kind;
  if (typeof kind !== 'string') throw new TypeError('HARNESS_CAPTURE_EVENT_KIND_INVALID');
  const base = { kind, evidenceDigest: parseDigest(input.evidenceDigest, 'programme capture event evidence') };
  if (SIMPLE_KINDS.has(kind) || (admitAllowed && kind === 'admit')) {
    assertExactKeys(input, ['kind', 'evidenceDigest'], 'programme capture event');
    return base as StoredEventInput;
  }
  if (kind === 'complete-pre-review' || kind === 'complete-post-review') {
    assertExactKeys(
      input,
      ['kind', 'evidenceDigest', 'codexReviewDigest', 'claudeReviewDigest'],
      'programme capture event',
    );
    const codexReviewDigest = parseDigest(
      input.codexReviewDigest,
      'programme capture Codex review',
    );
    const claudeReviewDigest = parseDigest(
      input.claudeReviewDigest,
      'programme capture Claude review',
    );
    if (new Set([base.evidenceDigest, codexReviewDigest, claudeReviewDigest]).size !== 3) {
      throw new TypeError('HARNESS_CAPTURE_REVIEW_NOT_DISTINCT');
    }
    return {
      ...base,
      kind,
      codexReviewDigest,
      claudeReviewDigest,
    };
  }
  if (kind === 'quiesce-model-boundary') {
    assertExactKeys(input, [
      'kind', 'evidenceDigest', 'processTerminationDigest',
      'providerEgressRevocationDigest',
    ], 'programme capture event');
    return {
      ...base,
      kind,
      processTerminationDigest: parseDigest(
        input.processTerminationDigest,
        'programme capture process termination',
      ),
      providerEgressRevocationDigest: parseDigest(
        input.providerEgressRevocationDigest,
        'programme capture provider egress revocation',
      ),
    };
  }
  if (kind === 'restore-review-boundary') {
    assertExactKeys(input, [
      'kind', 'evidenceDigest', 'providerEgressRestorationDigest',
    ], 'programme capture event');
    return {
      ...base,
      kind,
      providerEgressRestorationDigest: parseDigest(
        input.providerEgressRestorationDigest,
        'programme capture provider egress restoration',
      ),
    };
  }
  const extraName = singleEventExtraDigestName(kind);
  if (extraName !== null) {
    assertExactKeys(input, ['kind', 'evidenceDigest', extraName], 'programme capture event');
    return {
      ...base,
      kind,
      [extraName]: parseDigest(input[extraName], `programme capture ${extraName}`),
    } as StoredEventInput;
  }
  if (kind === 'fail' || kind === 'cancel' || kind === 'timeout') {
    assertExactKeys(input, [
      'kind', 'evidenceDigest', 'reasonDigest', 'processDispositionDigest',
      'egressDispositionDigest', 'leaseDispositionDigest',
    ], 'programme capture event');
    return {
      ...base,
      kind,
      reasonDigest: parseDigest(input.reasonDigest, 'programme capture terminal reason'),
      processDispositionDigest: parseDigest(
        input.processDispositionDigest,
        'programme capture terminal process disposition',
      ),
      egressDispositionDigest: parseDigest(
        input.egressDispositionDigest,
        'programme capture terminal egress disposition',
      ),
      leaseDispositionDigest: parseDigest(
        input.leaseDispositionDigest,
        'programme capture terminal lease disposition',
      ),
    };
  }
  throw new TypeError('HARNESS_CAPTURE_EVENT_KIND_INVALID');
}

function parseStoredEvent(value: unknown, expectedSequence: number): ProgrammeCaptureEventV1 {
  const input = asClosedRecord(value, `programme capture event[${expectedSequence}]`);
  const kind = typeof input.kind === 'string' ? input.kind : '';
  const extras = storedEventExtraNames(kind);
  assertExactKeys(input, [
    'sequence', 'kind', 'evidenceDigest', ...extras, 'previousDigest', 'digest',
  ], `programme capture event[${expectedSequence}]`);
  const sequence = asInteger(input.sequence, `programme capture event[${expectedSequence}].sequence`);
  if (sequence !== expectedSequence) throw new Error('HARNESS_CAPTURE_EVENT_SEQUENCE_INVALID');
  const previousDigest = parseDigest(
    input.previousDigest,
    `programme capture event[${expectedSequence}].previousDigest`,
  );
  const eventInput = Object.fromEntries([
    ['kind', input.kind],
    ['evidenceDigest', input.evidenceDigest],
    ...extras.map((name) => [name, input[name]]),
  ]);
  const parsed = parseEventInput(eventInput, expectedSequence === 0);
  const digest = parseDigest(input.digest, `programme capture event[${expectedSequence}].digest`);
  const body = { sequence, ...parsed, previousDigest };
  if (digestValue(body) !== digest) throw new Error('HARNESS_CAPTURE_EVENT_DIGEST_MISMATCH');
  return deepFreeze({ ...body, digest }) as ProgrammeCaptureEventV1;
}

function sealEvent(
  input: StoredEventInput,
  prior: readonly ProgrammeCaptureEventV1[],
  identityDigest?: string,
): ProgrammeCaptureEventV1 {
  const sequence = prior.length;
  if (sequence === 0 && identityDigest === undefined) {
    throw new Error('HARNESS_CAPTURE_EVENT_IDENTITY_ANCHOR_REQUIRED');
  }
  const previousDigest = sequence === 0
    ? parseDigest(identityDigest, 'programme capture identity anchor')
    : prior[sequence - 1].digest;
  const body = { sequence, ...input, previousDigest };
  return deepFreeze({ ...body, digest: digestValue(body) }) as ProgrammeCaptureEventV1;
}

function buildState(
  identity: Pick<ProgrammeCaptureStateV1, 'runId' | 'taskDigest' | 'claimDigest' | 'controller'>,
  events: readonly ProgrammeCaptureEventV1[],
): ProgrammeCaptureStateV1 {
  let phase: ProgrammeCapturePhaseV1 = 'admitted';
  let attempts = 0;
  for (const event of events.slice(1)) {
    phase = nextPhase(phase, event.kind);
    if (event.kind === 'start-capture-attempt') attempts += 1;
  }
  if (attempts > 1) throw new Error('HARNESS_CAPTURE_ATTEMPT_LIMIT_EXCEEDED');
  return deepFreeze({
    schemaVersion: 1,
    transactionKind: 'programme-capture-v1',
    runId: identity.runId,
    taskDigest: identity.taskDigest,
    claimDigest: identity.claimDigest,
    controller: { ...identity.controller },
    phase,
    captureAttempts: attempts as 0 | 1,
    events: [...events],
  });
}

function nextPhase(current: ProgrammeCapturePhaseV1, kind: string): ProgrammeCapturePhaseV1 {
  if (TERMINAL_PHASES.has(current)) throw new Error('HARNESS_CAPTURE_STATE_TERMINAL');
  if (kind === 'fail') return 'failed';
  if (kind === 'cancel') return 'cancelled';
  if (kind === 'timeout') return 'timed-out';
  const transition = SUCCESS_TRANSITIONS[current];
  if (transition === undefined || transition[0] !== kind) {
    throw new Error('HARNESS_CAPTURE_STATE_TRANSITION_INVALID');
  }
  return transition[1];
}

function singleEventExtraDigestName(kind: string): string | null {
  if (kind === 'acquire-runner-lease') return 'leaseDigest';
  if (kind === 'start-capture-attempt') return 'attemptDigest';
  if (kind === 'freeze-capture-record') return 'captureRecordDigest';
  return null;
}

function storedEventExtraNames(kind: string): string[] {
  if (kind === 'complete-pre-review' || kind === 'complete-post-review') {
    return ['codexReviewDigest', 'claudeReviewDigest'];
  }
  if (kind === 'quiesce-model-boundary') {
    return ['processTerminationDigest', 'providerEgressRevocationDigest'];
  }
  if (kind === 'restore-review-boundary') return ['providerEgressRestorationDigest'];
  const extra = singleEventExtraDigestName(kind);
  if (extra !== null) return [extra];
  if (kind === 'fail' || kind === 'cancel' || kind === 'timeout') {
    return [
      'reasonDigest', 'processDispositionDigest', 'egressDispositionDigest',
      'leaseDispositionDigest',
    ];
  }
  return [];
}

function validateEventAgainstHistory(
  history: readonly ProgrammeCaptureEventV1[],
  event: StoredEventInput,
): void {
  if (event.kind !== 'complete-post-review') return;
  const preReview = history.find((prior) => prior.kind === 'complete-pre-review');
  if (preReview?.kind !== 'complete-pre-review') {
    throw new Error('HARNESS_CAPTURE_PRE_REVIEW_REQUIRED');
  }
  const preReviewDigests = new Set([
    preReview.evidenceDigest,
    preReview.codexReviewDigest,
    preReview.claudeReviewDigest,
  ]);
  if ([event.evidenceDigest, event.codexReviewDigest, event.claudeReviewDigest]
    .some((digest) => preReviewDigests.has(digest))) {
    throw new Error('HARNESS_CAPTURE_REVIEW_STALE');
  }
}

function parseDigest(value: unknown, label: string): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || value === GENESIS_DIGEST) {
    throw new TypeError(`HARNESS_CAPTURE_DIGEST_INVALID: ${label}`);
  }
  return value;
}

function identityAnchor(
  identity: Pick<ProgrammeCaptureStateV1, 'runId' | 'taskDigest' | 'claimDigest' | 'controller'>,
): string {
  return digestValue({
    schemaVersion: 1,
    transactionKind: 'programme-capture-v1',
    runId: identity.runId,
    taskDigest: identity.taskDigest,
    claimDigest: identity.claimDigest,
    controller: identity.controller,
  });
}

function parseGitObjectId(value: unknown, label: string): string {
  if (typeof value !== 'string' || !/^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(value)) {
    throw new TypeError(`${label} is invalid`);
  }
  return value;
}
