// SPDX-License-Identifier: MIT

import { isProxy } from 'node:util/types';
import { asClosedRecord, assertExactKeys, deepFreeze } from './contracts.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export const PROGRAMME_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_MAX_BYTES_V2 = 4_096;
export const PROGRAMME_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_CONTENT_TYPE_V2 =
  'application/json; charset=utf-8' as const;
export const PROGRAMME_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_OUTCOMES_V2 = Object.freeze([
  'registration-not-admitted-v2',
  'registration-authority-pending-v2',
  'registration-closed-v2',
  'transaction-resolution-unknown-v2',
] as const);

export type ProgrammeCaptureSupervisorTransportOutcomeV2 =
  typeof PROGRAMME_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_OUTCOMES_V2[number];
export type ProgrammeCaptureSupervisorRecoveryDirectiveV2 =
  | 'new-authority-bound-request-required'
  | 'new-authority-bound-request-after-ready'
  | 'new-run-required'
  | 'exact-result-lookup-only';

export interface ProgrammeCaptureSupervisorTransportResponseV2 {
  readonly schemaVersion: 2;
  readonly transactionKind: 'programme-capture-v2';
  readonly responseKind: 'supervisor-registration-non-semantic-response-v2';
  readonly outcomeCode: ProgrammeCaptureSupervisorTransportOutcomeV2;
  readonly responseStatus: 403 | 409 | 500 | 503;
  readonly responseContentType:
    typeof PROGRAMME_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_CONTENT_TYPE_V2;
  readonly recoveryDirective: ProgrammeCaptureSupervisorRecoveryDirectiveV2;
}

const OUTCOME_MAPPING = Object.freeze({
  'registration-not-admitted-v2': Object.freeze({
    responseStatus: 403 as const,
    recoveryDirective: 'new-authority-bound-request-required' as const,
  }),
  'registration-authority-pending-v2': Object.freeze({
    responseStatus: 503 as const,
    recoveryDirective: 'new-authority-bound-request-after-ready' as const,
  }),
  'registration-closed-v2': Object.freeze({
    responseStatus: 409 as const,
    recoveryDirective: 'new-run-required' as const,
  }),
  'transaction-resolution-unknown-v2': Object.freeze({
    responseStatus: 500 as const,
    recoveryDirective: 'exact-result-lookup-only' as const,
  }),
});

export function buildProgrammeCaptureSupervisorTransportResponseV2(
  value: unknown,
): ProgrammeCaptureSupervisorTransportResponseV2 {
  const input = closed(value, 'supervisor transport-response input');
  assertExactKeys(input, ['outcomeCode'], 'supervisor transport-response input');
  return normalizedResponse(input);
}

export function parseProgrammeCaptureSupervisorTransportResponseV2(
  value: unknown,
): ProgrammeCaptureSupervisorTransportResponseV2 {
  const input = closed(value, 'supervisor transport response');
  assertExactKeys(input, [
    'schemaVersion', 'transactionKind', 'responseKind', 'outcomeCode',
    'responseStatus', 'responseContentType', 'recoveryDirective',
  ], 'supervisor transport response');
  if (input.schemaVersion !== 2 || input.transactionKind !== 'programme-capture-v2'
    || input.responseKind !== 'supervisor-registration-non-semantic-response-v2') {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_IDENTITY_INVALID');
  }
  const response = normalizedResponse(input);
  if (input.responseStatus !== response.responseStatus
    || input.responseContentType !== response.responseContentType
    || input.recoveryDirective !== response.recoveryDirective) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_MAPPING_INVALID');
  }
  return response;
}

export function serializeProgrammeCaptureSupervisorTransportResponseV2(
  value: unknown,
): string {
  return `${JSON.stringify(
    parseProgrammeCaptureSupervisorTransportResponseV2(value), null, 2,
  )}\n`;
}

export function parseProgrammeCaptureSupervisorTransportResponseBlobV2(
  serialized: string,
): ProgrammeCaptureSupervisorTransportResponseV2 {
  assertCanonicalBlob(serialized);
  const response = parseProgrammeCaptureSupervisorTransportResponseV2(
    parseJsonWithoutDuplicateKeys(serialized, 'supervisor transport response'),
  );
  if (serializeProgrammeCaptureSupervisorTransportResponseV2(response) !== serialized) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_CANONICAL_REQUIRED');
  }
  return response;
}

function normalizedResponse(
  input: Record<string, unknown>,
): ProgrammeCaptureSupervisorTransportResponseV2 {
  if (typeof input.outcomeCode !== 'string'
    || !PROGRAMME_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_OUTCOMES_V2.includes(
      input.outcomeCode as ProgrammeCaptureSupervisorTransportOutcomeV2,
    )) throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_OUTCOME_INVALID');
  const outcomeCode = input.outcomeCode as ProgrammeCaptureSupervisorTransportOutcomeV2;
  const mapping = OUTCOME_MAPPING[outcomeCode];
  return deepFreeze({
    schemaVersion: 2 as const,
    transactionKind: 'programme-capture-v2' as const,
    responseKind: 'supervisor-registration-non-semantic-response-v2' as const,
    outcomeCode,
    responseStatus: mapping.responseStatus,
    responseContentType: PROGRAMME_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_CONTENT_TYPE_V2,
    recoveryDirective: mapping.recoveryDirective,
  });
}

function assertCanonicalBlob(serialized: string): void {
  if (typeof serialized !== 'string'
    || Buffer.byteLength(serialized, 'utf8')
      > PROGRAMME_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_MAX_BYTES_V2
    || decodeCanonicalUtf8(serialized) !== serialized) {
    throw new TypeError('HARNESS_CAPTURE_SUPERVISOR_TRANSPORT_RESPONSE_CANONICAL_INVALID');
  }
}

function decodeCanonicalUtf8(value: string): string {
  try { return new TextDecoder('utf-8', { fatal: true }).decode(Buffer.from(value, 'utf8')); }
  catch { return ''; }
}

function closed(value: unknown, label: string): Record<string, unknown> {
  if (isProxy(value)) throw new TypeError(`${label} must not be a Proxy`);
  return asClosedRecord(value, label);
}
