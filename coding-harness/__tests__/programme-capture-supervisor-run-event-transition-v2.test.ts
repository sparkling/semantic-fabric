// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  deriveProgrammeCaptureSupervisorRunStateV2,
} from '../src/programme-capture-supervisor-run-event-transition-v2.js';
import {
  validRunHistory,
  withEventDigest,
} from './programme-capture-supervisor-run-event-v2-fixtures.js';

describe('programme capture supervisor run-event V2 transition replay', () => {
  it('derives every legal terminal or awaiting-witness phase', () => {
    expect(deriveProgrammeCaptureSupervisorRunStateV2(validRunHistory()).phase)
      .toBe('final-witnessed');
    expect(deriveProgrammeCaptureSupervisorRunStateV2(
      validRunHistory().slice(0, 4),
    ).phase).toBe('candidate-success-awaiting-final');
    expect(deriveProgrammeCaptureSupervisorRunStateV2(
      validRunHistory({ failure: true }),
    ).phase).toBe('failed-final-optional');
    expect(deriveProgrammeCaptureSupervisorRunStateV2(
      validRunHistory({ failure: true, includeFinal: true }),
    ).phase).toBe('final-witnessed');
    for (const stage of ['registration', 'pre-lease', 'leased-pre-start'] as const) {
      expect(deriveProgrammeCaptureSupervisorRunStateV2(
        validRunHistory({ preStartTerminal: stage }),
      ).phase).toBe('pre-start-terminal');
    }
  });

  it('rejects sequence, order, state-head, and identity substitutions', () => {
    const valid = validRunHistory();
    for (const mutate of [
      (events: any[]) => { [events[1], events[2]] = [events[2], events[1]]; },
      (events: any[]) => { events.push(events[4]); },
      (events: any[]) => { events[2].runSequence = '3'; events[2] = withEventDigest(events[2]); },
      (events: any[]) => { events[2].globalSequence = '2'; events[2] = withEventDigest(events[2]); },
      (events: any[]) => { events[2].priorControllerStateHeadDigest =
        events[0].priorControllerStateHeadDigest; events[2] = withEventDigest(events[2]); },
      (events: any[]) => { events[2].runId = 'another_run_20260829';
        events[2] = withEventDigest(events[2]); },
    ]) {
      const changed = structuredClone(valid) as any[];
      mutate(changed);
      expect(() => deriveProgrammeCaptureSupervisorRunStateV2(changed)).toThrow();
    }
  });

  it('rejects stale resource predecessors and cross-stage reference substitutions', () => {
    const valid = validRunHistory();
    for (const mutate of [
      (events: any[]) => { events[2].resourceTransition.members[0].priorState.eventDigest =
        events[0].eventDigest; events[2] = withEventDigest(events[2]); },
      (events: any[]) => { events[2].body.leaseId = 'another_lease_20260829';
        events[2] = withEventDigest(events[2]); },
      (events: any[]) => { events[3].body.startEventDigest = events[1].eventDigest;
        events[3] = withEventDigest(events[3]); },
      (events: any[]) => { events[4].body.captureRecordDigest =
        events[0].eventDigest; events[4] = withEventDigest(events[4]); },
      (events: any[]) => { events[4].body.postReview.codexReceiptDigest =
        (events[1].body as any).preReview.codexReceiptDigest;
        events[4] = withEventDigest(events[4]); },
    ]) {
      const changed = structuredClone(valid) as any[];
      mutate(changed);
      expect(() => deriveProgrammeCaptureSupervisorRunStateV2(changed)).toThrow();
    }
  });

  it('rejects every event after a pre-start terminal and a final witness', () => {
    const terminal = validRunHistory({ preStartTerminal: 'registration' });
    const candidate = validRunHistory()[1];
    expect(() => deriveProgrammeCaptureSupervisorRunStateV2([...terminal, candidate]))
      .toThrow();
    const completed = validRunHistory();
    expect(() => deriveProgrammeCaptureSupervisorRunStateV2([...completed, completed[4]]))
      .toThrow();
  });

  it('rejects empty, sparse, oversized, and Proxy histories without invoking traps', () => {
    expect(() => deriveProgrammeCaptureSupervisorRunStateV2([])).toThrow();
    expect(() => deriveProgrammeCaptureSupervisorRunStateV2(new Array(2))).toThrow();
    expect(() => deriveProgrammeCaptureSupervisorRunStateV2([
      ...validRunHistory(), ...validRunHistory(),
    ])).toThrow();
    let trapCalls = 0;
    const proxy = new Proxy(validRunHistory(), {
      getPrototypeOf() { trapCalls += 1; return Array.prototype; },
    });
    expect(() => deriveProgrammeCaptureSupervisorRunStateV2(proxy)).toThrow(/Proxy/);
    expect(trapCalls).toBe(0);
  });
});
