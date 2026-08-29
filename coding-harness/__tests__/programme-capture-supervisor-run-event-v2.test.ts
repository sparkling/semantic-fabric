// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  parseProgrammeCaptureSupervisorRunEventBlobV2,
  parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2,
  parseProgrammeCaptureSupervisorRunEventV2,
  serializeProgrammeCaptureSupervisorRunEventEnvelopeV2,
  serializeProgrammeCaptureSupervisorRunEventV2,
} from '../src/programme-capture-supervisor-run-event-codec-v2.js';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_MAX_BYTES_V2,
} from '../src/programme-capture-supervisor-run-event-contracts-v2.js';
import {
  signedEnvelope,
  validRunHistory,
  withEventDigest,
} from './programme-capture-supervisor-run-event-v2-fixtures.js';

describe('programme capture supervisor semantic run-event V2', () => {
  it('parses all six closed event kinds without granting authority', () => {
    const histories = [
      validRunHistory(),
      validRunHistory({ preStartTerminal: 'registration' }),
    ];
    const events = histories.flat();
    expect(new Set(events.map(({ eventKind }) => eventKind))).toEqual(new Set([
      'claim-registered-v2',
      'runner-lease-granted-v2',
      'capture-attempt-start-committed-v2',
      'capture-run-terminal-v2',
      'capture-attempt-terminal-v2',
      'capture-final-witness-v2',
    ]));
    for (const event of events) {
      const serialized = serializeProgrammeCaptureSupervisorRunEventV2(event);
      const parsed = parseProgrammeCaptureSupervisorRunEventBlobV2(serialized);
      expect(parsed).toEqual(event);
      expect(serialized).toBe(`${JSON.stringify(event, null, 2)}\n`);
      expect(parsed).toMatchObject({
        verificationScope: 'service-signed-structure-only',
        serviceSignatureVerified: false,
        priorGlobalEventVerified: false,
        resourceHighWaterVerified: false,
        resourceFencingVerified: false,
        stateTransitionAuthorized: false,
        attemptStartAuthorized: false,
        captureAuthorized: false,
        importAuthorized: false,
        promotionAuthorized: false,
        releaseAuthorized: false,
      });
      expect(Object.isFrozen(parsed)).toBe(true);
      expect(Object.isFrozen(parsed.body)).toBe(true);
    }
  });

  it('enforces the three legal pre-start terminal shapes', () => {
    for (const stage of ['registration', 'pre-lease', 'leased-pre-start'] as const) {
      const history = validRunHistory({ preStartTerminal: stage });
      const terminal = history.at(-1)!;
      expect(terminal.eventKind).toBe('capture-run-terminal-v2');
      expect((terminal.body as any).terminalStage).toBe(stage);
      expect((terminal.body as any).attemptId).toBeNull();
      expect((terminal.body as any).captureRecordDigest).toBeNull();
      expect(terminal.resourceTransition === null).toBe(stage !== 'leased-pre-start');
    }

    const partialLeaseReferences = structuredClone(
      validRunHistory({ preStartTerminal: 'registration' }).at(-1)!,
    ) as any;
    partialLeaseReferences.body.leaseEventDigest = '1'.repeat(64);
    expect(() => parseProgrammeCaptureSupervisorRunEventV2(partialLeaseReferences))
      .toThrow(/LEASE_REFS_INVALID/);
  });

  it('requires one single-use lease with separated native-host pre-reviews', () => {
    const lease = validRunHistory()[1];
    expect(lease.body).toMatchObject({
      lease: {
        maxAttempts: 1,
        renew: false,
        releaseForReuse: false,
        reassign: false,
        reclaim: false,
        retry: false,
      },
    });
    for (const mutate of [
      (body: any) => { body.lease.retry = true; },
      (body: any) => { body.lease.maxAttempts = 2; },
      (body: any) => { body.lease.notAfter = body.lease.serviceIssuedAt; },
      (body: any) => {
        body.lease.serviceIssuedAt = '+010000-01-01T00:00:00.000Z';
        body.lease.notAfter = '9999-12-31T23:59:59.999Z';
      },
      (body: any) => { body.preReview.claudeReceiptDigest = body.preReview.codexReceiptDigest; },
    ]) {
      const changed = structuredClone(lease) as any;
      mutate(changed.body);
      expect(() => parseProgrammeCaptureSupervisorRunEventV2(withEventDigest(changed)))
        .toThrow();
    }
  });

  it('requires safe cleanup before resource release and a complete candidate result', () => {
    const terminal = validRunHistory()[3];
    for (const mutate of [
      (body: any) => { body.outputEnvelopeDigest = null; },
      (body: any) => { body.processDisposition.kind = 'runner-lost'; },
      (body: any) => { body.egressDisposition.kind = 'violation-detected'; },
      (body: any) => { body.cleanup.resourceCleanupDigest = null; },
      (body: any) => { body.resourceDisposition.kind = 'quarantined'; },
    ]) {
      const changed = structuredClone(terminal) as any;
      mutate(changed.body);
      expect(() => parseProgrammeCaptureSupervisorRunEventV2(withEventDigest(changed)))
        .toThrow();
    }
    const failed = validRunHistory({ failure: true })[3];
    expect(() => parseProgrammeCaptureSupervisorRunEventV2(failed)).not.toThrow();

    for (const mutate of [
      (body: any) => { body.outcomeCode = 'process-failed-v2'; },
      (body: any) => { body.outcomeCode = 'attempt-timeout-v2'; },
      (body: any) => { body.outcomeCode = 'runner-lost-v2'; },
      (body: any) => { body.outcomeCode = 'output-missing-v2'; },
      (body: any) => { body.outcomeCode = 'cleanup-failed-v2'; },
      (body: any) => { body.outcomeCode = 'egress-violation-v2'; },
      (body: any) => { body.outcomeCode = 'fence-invalidated-v2'; },
    ]) {
      const contradictory = structuredClone(terminal) as any;
      mutate(contradictory.body);
      expect(() => parseProgrammeCaptureSupervisorRunEventV2(
        withEventDigest(contradictory),
      )).toThrow(/OUTCOME_DISPOSITION_MISMATCH/);
    }
  });

  it('binds conflict sets to enrollment, sorted resources, and the physical parent', () => {
    const lease = validRunHistory()[1];
    const resource = lease.resourceTransition!;
    expect(resource.members.map(({ resourceId }) => resourceId)).toEqual([
      'numa_parent_20260829', 'resource_cpu_20260829',
    ]);
    for (const mutate of [
      (changed: any) => { changed.resourceTransition.members.reverse(); },
      (changed: any) => { changed.resourceTransition.members[1].resourceId =
        changed.resourceTransition.members[0].resourceId; },
      (changed: any) => { changed.resourceTransition.physicalParentId = 'absent_parent_20260829'; },
      (changed: any) => { changed.resourceTransition.runnerEnrollmentRecordDigest =
        '0'.repeat(64); },
      (changed: any) => { changed.resourceTransition.fence = '01'; },
    ]) {
      const changed = structuredClone(lease) as any;
      mutate(changed);
      expect(() => parseProgrammeCaptureSupervisorRunEventV2(withEventDigest(changed)))
        .toThrow();
    }
  });

  it('rejects kind substitutions, unknown fields, and authority escalation after redigesting', () => {
    const registration = validRunHistory()[0];
    for (const mutate of [
      (changed: any) => { changed.captureAuthorized = true; },
      (changed: any) => { changed.body.extra = true; },
      (changed: any) => { changed.eventKind = 'runner-lease-granted-v2'; },
      (changed: any) => { changed.previousRun = {
        kind: 'run-event', eventDigest: changed.eventDigest,
      }; },
      (changed: any) => { changed.globalSequence = '2'; },
      (changed: any) => { changed.semanticRequestDigest = '0'.repeat(64); },
    ]) {
      const changed = structuredClone(registration) as any;
      mutate(changed);
      expect(() => parseProgrammeCaptureSupervisorRunEventV2(withEventDigest(changed)))
        .toThrow();
    }
  });

  it('rejects noncanonical, duplicate, oversized, and hostile values', () => {
    const event = validRunHistory()[0];
    const canonical = serializeProgrammeCaptureSupervisorRunEventV2(event);
    for (const invalid of [
      JSON.stringify(event), `${canonical} `, `\ufeff${canonical}`,
      canonical.replace('"schemaVersion": 2,', '"schemaVersion": 2,\n  "schemaVersion": 2,'),
      ' '.repeat(PROGRAMME_CAPTURE_SUPERVISOR_RUN_EVENT_MAX_BYTES_V2 + 1),
    ]) expect(() => parseProgrammeCaptureSupervisorRunEventBlobV2(invalid)).toThrow();

    let trapCalls = 0;
    const proxy = new Proxy(event, {
      getPrototypeOf() { trapCalls += 1; return Object.prototype; },
      ownKeys() { trapCalls += 1; return []; },
    });
    expect(() => parseProgrammeCaptureSupervisorRunEventV2(proxy)).toThrow(/Proxy/);
    expect(trapCalls).toBe(0);

    const nestedProxy = {
      ...event,
      body: new Proxy(event.body as object, {
        getPrototypeOf() { trapCalls += 1; return Object.prototype; },
        ownKeys() { trapCalls += 1; return []; },
      }),
    };
    expect(() => parseProgrammeCaptureSupervisorRunEventV2(nestedProxy)).toThrow(/Proxy/);
    expect(trapCalls).toBe(0);

    const accessor = structuredClone(event) as any;
    Object.defineProperty(accessor.body, 'claimDigest', {
      enumerable: true, get: () => '0'.repeat(64),
    });
    expect(() => parseProgrammeCaptureSupervisorRunEventV2(accessor))
      .toThrow(/plain own-key object/);
  });

  it('round-trips a bounded canonical Ed25519 envelope', () => {
    const event = validRunHistory()[0];
    const serialized = signedEnvelope(event);
    const parsed = parseProgrammeCaptureSupervisorRunEventEnvelopeBlobV2(serialized);
    expect(parsed.event).toEqual(event);
    expect(serializeProgrammeCaptureSupervisorRunEventEnvelopeV2(parsed)).toBe(serialized);
    expect(Object.isFrozen(parsed.signature)).toBe(true);
  });
});
