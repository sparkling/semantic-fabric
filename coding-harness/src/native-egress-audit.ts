// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';

export type OriginBrokerOutcome =
  | 'origin-admitted'
  | 'transport-connected'
  | 'policy-denied'
  | 'transport-error'
  | 'boundary-error';

export interface OriginBrokerEvent {
  readonly attemptId: number;
  readonly target: string;
  readonly outcome: OriginBrokerOutcome;
}

export interface OriginBrokerEventSummary {
  readonly allowedConnections: number;
  readonly deniedConnections: number;
  readonly transportErrors: number;
  readonly boundaryErrors: number;
}

const OUTCOMES = new Set<OriginBrokerOutcome>([
  'origin-admitted', 'transport-connected', 'policy-denied',
  'transport-error', 'boundary-error',
]);

export function summarizeOriginBrokerEvents(
  events: readonly OriginBrokerEvent[],
): OriginBrokerEventSummary {
  let allowedConnections = 0;
  let deniedConnections = 0;
  let transportErrors = 0;
  let boundaryErrors = 0;
  for (const event of events) {
    assertEvent(event);
    if (event.outcome === 'transport-connected') allowedConnections += 1;
    else if (event.outcome === 'policy-denied') deniedConnections += 1;
    else if (event.outcome === 'transport-error') transportErrors += 1;
    else if (event.outcome === 'boundary-error') boundaryErrors += 1;
  }
  return Object.freeze({
    allowedConnections, deniedConnections, transportErrors, boundaryErrors,
  });
}

export function digestOriginBrokerEvents(events: readonly OriginBrokerEvent[]): string {
  const canonical = events.map((event) => {
    assertEvent(event);
    return { attemptId: event.attemptId, target: event.target, outcome: event.outcome };
  }).sort(compareEvents);
  return createHash('sha256').update(JSON.stringify(canonical)).digest('hex');
}

export function originBrokerTimeoutIsPolicyDenial(bufferedBytes: number): boolean {
  if (!Number.isSafeInteger(bufferedBytes) || bufferedBytes < 0) {
    throw new Error('HARNESS_NATIVE_EGRESS_BUFFER_SIZE_INVALID');
  }
  return bufferedBytes > 0;
}

function assertEvent(event: OriginBrokerEvent): void {
  if (!Number.isSafeInteger(event.attemptId) || event.attemptId < 1
    || event.target.length < 1 || Buffer.byteLength(event.target) > 65_536
    || event.target.includes('\0') || !OUTCOMES.has(event.outcome)) {
    throw new Error('HARNESS_NATIVE_EGRESS_EVENT_INVALID');
  }
}

function compareEvents(left: OriginBrokerEvent, right: OriginBrokerEvent): number {
  if (left.attemptId !== right.attemptId) return left.attemptId - right.attemptId;
  if (left.target !== right.target) return left.target < right.target ? -1 : 1;
  return left.outcome === right.outcome ? 0 : left.outcome < right.outcome ? -1 : 1;
}
