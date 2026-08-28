// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  createProgrammeCaptureStateV1,
  parseProgrammeCaptureStateV1,
  transitionProgrammeCaptureStateV1,
  type ProgrammeCaptureEventInputV1,
  type ProgrammeCaptureStateV1,
} from '../src/programme-capture-state-v1.js';
import { digestValue } from '../src/receipts.js';

const digest = (character: string): string => character.repeat(64);

function initialState(): ProgrammeCaptureStateV1 {
  return createProgrammeCaptureStateV1({
    runId: 'capture_run_20260828_0001',
    taskDigest: digest('a'),
    claimDigest: digest('b'),
    controller: { commit: 'c'.repeat(40), tree: 'd'.repeat(40) },
    admissionEvidenceDigest: digest('e'),
  });
}

const successEvents: readonly ProgrammeCaptureEventInputV1[] = [
  { kind: 'attest-inputs', evidenceDigest: digest('1') },
  { kind: 'pass-host-preflight', evidenceDigest: digest('2') },
  {
    kind: 'complete-pre-review',
    evidenceDigest: digest('3'),
    codexReviewDigest: digest('4'),
    claudeReviewDigest: digest('5'),
  },
  { kind: 'acquire-runner-lease', evidenceDigest: digest('8'), leaseDigest: digest('9') },
  {
    kind: 'quiesce-model-boundary',
    evidenceDigest: digest('6'),
    processTerminationDigest: digest('7'),
    providerEgressRevocationDigest: digest('8'),
  },
  { kind: 'pass-capture-preflight', evidenceDigest: digest('7') },
  { kind: 'start-capture-attempt', evidenceDigest: digest('a'), attemptDigest: digest('b') },
  { kind: 'complete-capture', evidenceDigest: digest('c') },
  { kind: 'verify-process-cleanup', evidenceDigest: digest('d') },
  { kind: 'verify-capture', evidenceDigest: digest('e') },
  {
    kind: 'freeze-capture-record',
    evidenceDigest: digest('f'),
    captureRecordDigest: digest('1'),
  },
  {
    kind: 'restore-review-boundary',
    evidenceDigest: digest('2'),
    providerEgressRestorationDigest: digest('3'),
  },
  {
    kind: 'complete-post-review',
    evidenceDigest: digest('8'),
    codexReviewDigest: digest('9'),
    claudeReviewDigest: digest('a'),
  },
  { kind: 'seal', evidenceDigest: digest('6') },
] as const;

function advance(
  state: ProgrammeCaptureStateV1,
  events: readonly ProgrammeCaptureEventInputV1[],
): ProgrammeCaptureStateV1 {
  return events.reduce(transitionProgrammeCaptureStateV1, state);
}

describe('programme capture V1 terminal state machine', () => {
  it('accepts only the exact success order and increments the attempt before capture', () => {
    let state = initialState();
    expect(state.phase).toBe('admitted');
    expect(state.captureAttempts).toBe(0);

    for (const event of successEvents) {
      state = transitionProgrammeCaptureStateV1(state, event);
      if (event.kind === 'start-capture-attempt') {
        expect(state.phase).toBe('capture-attempt-started');
        expect(state.captureAttempts).toBe(1);
      }
    }

    expect(state.phase).toBe('sealed');
    expect(state.captureAttempts).toBe(1);
    expect(state.events).toHaveLength(successEvents.length + 1);
    expect(parseProgrammeCaptureStateV1(structuredClone(state))).toEqual(state);
    expect(Object.isFrozen(state)).toBe(true);
    expect(state.events.every(Object.isFrozen)).toBe(true);
  });

  it('rejects every skipped, reordered, duplicated, or repeated success transition', () => {
    let state = initialState();
    for (let index = 0; index < successEvents.length; index += 1) {
      const expected = successEvents[index];
      for (let other = 0; other < successEvents.length; other += 1) {
        if (other === index) continue;
        expect(() => transitionProgrammeCaptureStateV1(state, successEvents[other]))
          .toThrow(/TRANSITION_INVALID/);
      }
      state = transitionProgrammeCaptureStateV1(state, expected);
      expect(() => transitionProgrammeCaptureStateV1(state, expected))
        .toThrow(/TRANSITION_INVALID|TERMINAL/);
    }
  });

  it('makes failure, cancellation, and timeout terminal before or after attempt start', () => {
    for (const outcome of ['fail', 'cancel', 'timeout'] as const) {
      for (let prefixLength = 0; prefixLength < successEvents.length; prefixLength += 1) {
        const before = advance(initialState(), successEvents.slice(0, prefixLength));
        const terminal = transitionProgrammeCaptureStateV1(before, {
          kind: outcome,
          evidenceDigest: digest('8'),
          reasonDigest: digest('9'),
          processDispositionDigest: digest('a'),
          egressDispositionDigest: digest('b'),
          leaseDispositionDigest: digest('c'),
        });
        expect(terminal.phase).toBe(
          outcome === 'fail' ? 'failed' : outcome === 'cancel' ? 'cancelled' : 'timed-out',
        );
        expect(terminal.captureAttempts).toBe(prefixLength >= 7 ? 1 : 0);
        expect(() => transitionProgrammeCaptureStateV1(terminal, successEvents[0]))
          .toThrow(/TERMINAL/);
        expect(() => transitionProgrammeCaptureStateV1(terminal, {
          kind: outcome,
          evidenceDigest: digest('8'),
          reasonDigest: digest('9'),
          processDispositionDigest: digest('a'),
          egressDispositionDigest: digest('b'),
          leaseDispositionDigest: digest('c'),
        })).toThrow(/TERMINAL/);
      }
    }
  });

  it('rejects mutated state, event keys, identity, digest, or derived attempt count', () => {
    const invalidStates: any[] = [];
    const phase = advance(initialState(), successEvents.slice(0, 8));
    for (const mutate of [
      (state: any) => { state.extra = true; },
      (state: any) => { state.runId = 'short'; },
      (state: any) => { state.taskDigest = digest('0'); },
      (state: any) => { state.controller.commit = 'A'.repeat(40); },
      (state: any) => { state.controller.commit = 'a'.repeat(41); },
      (state: any) => { state.phase = 'sealed'; },
      (state: any) => { state.captureAttempts = 0; },
      (state: any) => { state.events[1].kind = 'verify-capture'; },
      (state: any) => { state.events[1].digest = digest('f'); },
      (state: any) => { state.events[1].previousDigest = digest('f'); },
      (state: any) => { state.events[1].extra = true; },
      (state: any) => { state.events = new Array(state.events.length); },
    ]) {
      const state = structuredClone(phase);
      mutate(state);
      invalidStates.push(state);
    }
    for (const state of invalidStates) {
      expect(() => parseProgrammeCaptureStateV1(state)).toThrow();
      expect(() => transitionProgrammeCaptureStateV1(state, successEvents[8])).toThrow();
    }

    const badEvent = { ...successEvents[8], extra: true } as any;
    expect(() => transitionProgrammeCaptureStateV1(phase, badEvent)).toThrow(/invalid keys/);
    expect(() => transitionProgrammeCaptureStateV1(phase, {
      ...successEvents[8], evidenceDigest: digest('0'),
    })).toThrow(/DIGEST_INVALID/);

    const forked = structuredClone(phase) as any;
    forked.events[2].previousDigest = digest('f');
    const { digest: _oldDigest, ...forkedBody } = forked.events[2];
    forked.events[2].digest = digestValue(forkedBody);
    expect(() => parseProgrammeCaptureStateV1(forked)).toThrow(/PREVIOUS_DIGEST_INVALID/);

    const substitutedIdentity = structuredClone(phase) as any;
    substitutedIdentity.runId = 'capture_run_20260828_9999';
    expect(() => parseProgrammeCaptureStateV1(substitutedIdentity))
      .toThrow(/PREVIOUS_DIGEST_INVALID/);
  });

  it('rejects inherited records, sparse arrays, and unknown creation keys', () => {
    expect(() => createProgrammeCaptureStateV1({
      runId: 'capture_run_20260828_0001',
      taskDigest: digest('a'),
      claimDigest: digest('b'),
      controller: { commit: 'c'.repeat(40), tree: 'd'.repeat(40) },
      admissionEvidenceDigest: digest('e'),
      retry: true,
    } as any)).toThrow(/invalid keys/);

    expect(() => createProgrammeCaptureStateV1(Object.create({
      runId: 'capture_run_20260828_0001',
      taskDigest: digest('a'),
      claimDigest: digest('b'),
      controller: { commit: 'c'.repeat(40), tree: 'd'.repeat(40) },
      admissionEvidenceDigest: digest('e'),
    }))).toThrow(/plain own-key object/);
    expect(() => transitionProgrammeCaptureStateV1(
      initialState(),
      Object.create(successEvents[0]),
    )).toThrow(/plain own-key object/);
  });

  it('rejects stale reviews and boundary, lease, record, or terminal evidence substitution', () => {
    const cases: Array<[number, (event: any) => void]> = [
      [2, (event) => { delete event.codexReviewDigest; }],
      [2, (event) => { event.claudeReviewDigest = digest('0'); }],
      [2, (event) => { event.claudeReviewDigest = event.codexReviewDigest; }],
      [3, (event) => { delete event.leaseDigest; }],
      [4, (event) => { delete event.processTerminationDigest; }],
      [4, (event) => { delete event.providerEgressRevocationDigest; }],
      [6, (event) => { event.attemptDigest = digest('0'); }],
      [10, (event) => { event.captureRecordDigest = digest('0'); }],
      [11, (event) => { delete event.providerEgressRestorationDigest; }],
      [12, (event) => { delete event.claudeReviewDigest; }],
      [13, (event) => { event.envelopeDigest = digest('7'); }],
    ];
    for (const [index, mutate] of cases) {
      const state = advance(initialState(), successEvents.slice(0, index));
      const event = structuredClone(successEvents[index]) as any;
      mutate(event);
      expect(() => transitionProgrammeCaptureStateV1(state, event)).toThrow();
    }

    const beforePostReview = advance(initialState(), successEvents.slice(0, 12));
    expect(() => transitionProgrammeCaptureStateV1(beforePostReview, {
      ...successEvents[12],
      codexReviewDigest: digest('4'),
    })).toThrow(/REVIEW_STALE/);

    for (const missing of [
      'processDispositionDigest',
      'egressDispositionDigest',
      'leaseDispositionDigest',
    ] as const) {
      const event: any = {
        kind: 'fail',
        evidenceDigest: digest('8'),
        reasonDigest: digest('9'),
        processDispositionDigest: digest('a'),
        egressDispositionDigest: digest('b'),
        leaseDispositionDigest: digest('c'),
      };
      delete event[missing];
      expect(() => transitionProgrammeCaptureStateV1(initialState(), event)).toThrow();
    }
  });
});
