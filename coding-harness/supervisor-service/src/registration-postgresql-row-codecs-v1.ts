// SPDX-License-Identifier: MIT

import { isProxy } from 'node:util/types';
import {
  asRecord,
  assertExactKeys,
  deepFreeze,
  parseCanonicalPrettyJson,
  parseDigest,
  parseOpaqueId,
  parseUint64,
  sha256Text,
} from './closed-json.js';
import { validateStoredRegistrationEventEnvelopeV2 }
  from './registration-event-envelope-v2.js';
import type { ExactCommittedResultReadV1 } from './registration-ports-v1.js';
import {
  REGISTRATION_CONTENT_TYPE_V2,
  REGISTRATION_REQUEST_MAX_BYTES_V2,
  REGISTRATION_RESULT_MAX_BYTES_V2,
} from './registration-protocol-v2.js';

const EVENT_ENVELOPE_MAX_BYTES_V2 = 65_536;
export const POSTGRES_EXACT_RESULT_RAW_ROW_KEYS_V1 = Object.freeze([
  'result_project_authority_digest',
  'result_semantic_request_digest',
  'result_run_id',
  'result_serialized_request',
  'result_serialized_request_sha256',
  'result_response_status_text',
  'result_response_content_type',
  'result_serialized_response',
  'result_serialized_response_sha256',
  'result_current_event_digest',
  'current_event_project_authority_digest',
  'current_event_digest',
  'current_event_kind',
  'current_event_semantic_request_digest',
  'current_event_prior_controller_state_head_digest',
  'current_event_serialized_envelope',
  'current_event_serialized_envelope_sha256',
  'run_project_authority_digest',
  'run_run_id',
  'run_original_registration_request_digest',
  'run_original_registration_event_digest',
  'original_event_project_authority_digest',
  'original_event_digest',
  'original_event_semantic_request_digest',
  'original_event_global_sequence_text',
  'original_event_serialized_envelope',
  'original_event_serialized_envelope_sha256',
] as const);

type RawRowKey = typeof POSTGRES_EXACT_RESULT_RAW_ROW_KEYS_V1[number];
type SnapshotRow = Readonly<Record<RawRowKey, unknown>>;

interface EventRefsV1 {
  readonly projectAuthorityDigest: string;
  readonly semanticRequestDigest: string;
  readonly runId: string;
  readonly eventDigest: string;
  readonly eventKind: string;
  readonly globalSequence: string;
  readonly priorControllerStateHeadDigest: string;
}

/** Decode only the fixed left-joined exact-result query; zero roots alone are absent. */
export async function decodePostgresExactResultRowsV1(
  value: unknown,
): Promise<ExactCommittedResultReadV1> {
  let rows: readonly SnapshotRow[];
  try { rows = snapshotRows(value); }
  catch { return indeterminate(); }
  if (rows.length === 0) return Object.freeze({ kind: 'absent' });
  if (rows.length !== 1) return indeterminate();
  try { return await decodeRow(rows[0]!); }
  catch { return indeterminate(); }
}

async function decodeRow(row: SnapshotRow): Promise<ExactCommittedResultReadV1> {
  const project = digest(row, 'result_project_authority_digest');
  const semanticRequest = digest(row, 'result_semantic_request_digest');
  const resultRunId = opaque(row, 'result_run_id');
  const currentEventDigest = digest(row, 'current_event_digest');
  const originalEventDigest = digest(row, 'original_event_digest');
  const originalRequestDigest = digest(
    row, 'run_original_registration_request_digest',
  );
  const originalSequence = uint64(row, 'original_event_global_sequence_text');
  if (originalSequence === '0') throw new TypeError('original event sequence is invalid');

  const statusText = text(row, 'result_response_status_text');
  if (statusText !== '201' && statusText !== '409') {
    throw new TypeError('stored response status is invalid');
  }
  const status = Number(statusText) as 201 | 409;
  if (text(row, 'result_response_content_type') !== REGISTRATION_CONTENT_TYPE_V2) {
    throw new TypeError('stored response content type is invalid');
  }

  const request = canonicalBytes(
    row, 'result_serialized_request', REGISTRATION_REQUEST_MAX_BYTES_V2,
  );
  const response = canonicalBytes(
    row, 'result_serialized_response', REGISTRATION_RESULT_MAX_BYTES_V2,
  );
  const currentEnvelope = canonicalBytes(
    row, 'current_event_serialized_envelope', EVENT_ENVELOPE_MAX_BYTES_V2,
  );
  const originalEnvelope = canonicalBytes(
    row, 'original_event_serialized_envelope', EVENT_ENVELOPE_MAX_BYTES_V2,
  );

  const expectedHashes = [
    digest(row, 'result_serialized_request_sha256'),
    digest(row, 'result_serialized_response_sha256'),
    digest(row, 'current_event_serialized_envelope_sha256'),
    digest(row, 'original_event_serialized_envelope_sha256'),
  ];
  const actualHashes = await Promise.all([
    sha256Text(request.text),
    sha256Text(response.text),
    sha256Text(currentEnvelope.text),
    sha256Text(originalEnvelope.text),
  ]);
  if (actualHashes.some((hash, index) => hash !== expectedHashes[index])) {
    throw new TypeError('stored canonical byte hash is invalid');
  }

  const current = eventRefs(currentEnvelope.record, 'current event');
  const original = eventRefs(originalEnvelope.record, 'original event');
  const responseEnvelope = response.record.serializedEventEnvelope;
  if (responseEnvelope !== currentEnvelope.text
    || response.record.responseStatus !== status
    || response.record.responseContentType !== REGISTRATION_CONTENT_TYPE_V2) {
    throw new TypeError('stored result/event binding is invalid');
  }

  requireEqual(project, digest(row, 'current_event_project_authority_digest'));
  requireEqual(project, digest(row, 'run_project_authority_digest'));
  requireEqual(project, digest(row, 'original_event_project_authority_digest'));
  requireEqual(project, current.projectAuthorityDigest);
  requireEqual(project, original.projectAuthorityDigest);
  requireEqual(semanticRequest, digest(row, 'current_event_semantic_request_digest'));
  requireEqual(semanticRequest, current.semanticRequestDigest);
  requireEqual(originalRequestDigest, digest(
    row, 'original_event_semantic_request_digest',
  ));
  requireEqual(originalRequestDigest, original.semanticRequestDigest);
  requireEqual(resultRunId, opaque(row, 'run_run_id'));
  requireEqual(resultRunId, current.runId);
  requireEqual(resultRunId, original.runId);
  requireEqual(currentEventDigest, digest(row, 'result_current_event_digest'));
  requireEqual(currentEventDigest, current.eventDigest);
  requireEqual(current.eventKind, text(row, 'current_event_kind'));
  requireEqual(originalEventDigest, digest(
    row, 'run_original_registration_event_digest',
  ));
  requireEqual(originalEventDigest, original.eventDigest);
  requireEqual(originalSequence, original.globalSequence);
  requireEqual(
    current.priorControllerStateHeadDigest,
    digest(row, 'current_event_prior_controller_state_head_digest'),
  );
  if (original.eventKind !== 'claim-registered-v2') {
    throw new TypeError('stored original event kind is invalid');
  }
  await validateOriginalRegistrationEnvelope(
    originalEnvelope.text, originalEnvelope.record, {
      projectAuthorityDigest: project,
      semanticRequestDigest: originalRequestDigest,
      runId: resultRunId,
      eventDigest: originalEventDigest,
      globalSequence: originalSequence,
    },
  );

  if (status === 201) {
    if (current.eventKind !== 'claim-registered-v2'
      || semanticRequest !== originalRequestDigest
      || currentEventDigest !== originalEventDigest
      || currentEnvelope.text !== originalEnvelope.text) {
      throw new TypeError('stored registration provenance is invalid');
    }
  } else if (current.eventKind !== 'capture-run-terminal-v2'
    || semanticRequest === originalRequestDigest
    || currentEventDigest === originalEventDigest) {
    throw new TypeError('stored changed-replay provenance is invalid');
  }

  return deepFreeze({
    kind: 'found',
    row: {
      projectAuthorityDigest: project,
      semanticRequestDigest: semanticRequest,
      originalRegistrationRequestDigest: originalRequestDigest,
      originalRegistrationEventDigest: originalEventDigest,
      originalRegistrationGlobalSequence: originalSequence,
      changedReplayPriorControllerStateHeadDigest: status === 201
        ? null : current.priorControllerStateHeadDigest,
      serializedRequest: request.text,
      serializedRequestSha256: expectedHashes[0]!,
      responseStatus: status,
      responseContentType: REGISTRATION_CONTENT_TYPE_V2,
      serializedResponse: response.text,
      serializedResponseSha256: expectedHashes[1]!,
    },
  });
}

function snapshotRows(value: unknown): readonly SnapshotRow[] {
  if (isProxy(value) || !Array.isArray(value)
    || Object.getPrototypeOf(value) !== Array.prototype
    || Object.getOwnPropertySymbols(value).length !== 0) {
    throw new TypeError('joined rows must be a plain array');
  }
  const descriptors = Object.getOwnPropertyDescriptors(value);
  const indexed = Object.keys(descriptors).filter((key) => key !== 'length');
  if (indexed.length !== value.length
    || indexed.some((key, index) => key !== String(index))) {
    throw new TypeError('joined rows must be dense');
  }
  return indexed.map((key) => {
    const descriptor = descriptors[key];
    if (descriptor === undefined || !descriptor.enumerable || !('value' in descriptor)) {
      throw new TypeError('joined row must be data');
    }
    return snapshotRow(descriptor.value);
  });
}

function snapshotRow(value: unknown): SnapshotRow {
  if (value === null || typeof value !== 'object' || Array.isArray(value)
    || isProxy(value) || Object.getPrototypeOf(value) !== Object.prototype
    || Object.getOwnPropertySymbols(value).length !== 0) {
    throw new TypeError('joined row must be a plain record');
  }
  const descriptors = Object.getOwnPropertyDescriptors(value);
  assertExactKeys(descriptors, POSTGRES_EXACT_RESULT_RAW_ROW_KEYS_V1, 'joined row');
  for (const key of POSTGRES_EXACT_RESULT_RAW_ROW_KEYS_V1) {
    const descriptor = descriptors[key];
    if (descriptor === undefined || !descriptor.enumerable || !('value' in descriptor)) {
      throw new TypeError('joined row fields must be enumerable data');
    }
  }
  return Object.freeze(Object.fromEntries(POSTGRES_EXACT_RESULT_RAW_ROW_KEYS_V1.map((key) => {
    const descriptor = descriptors[key]!;
    return [key, snapshotValue(descriptor.value)];
  }))) as SnapshotRow;
}

function snapshotValue(value: unknown): unknown {
  if (isProxy(value)) throw new TypeError('joined row value must not be a Proxy');
  if (value === null || typeof value === 'string'
    || typeof value === 'number' || typeof value === 'bigint') return value;
  if (!(value instanceof Uint8Array)) throw new TypeError('joined row value is invalid');
  const copy = structuredClone(value);
  if (!(copy instanceof Uint8Array)) throw new TypeError('joined bytea is invalid');
  return new Uint8Array(copy);
}

async function validateOriginalRegistrationEnvelope(
  serialized: string,
  envelope: Record<string, unknown>,
  expected: Readonly<{
    projectAuthorityDigest: string;
    semanticRequestDigest: string;
    runId: string;
    eventDigest: string;
    globalSequence: string;
  }>,
): Promise<void> {
  const event = asRecord(envelope.event, 'original registration event');
  const project = asRecord(event.project, 'original registration event project');
  const authorityHead = asRecord(
    event.authorityHead, 'original registration event authority head',
  );
  const claim = asRecord(event.body, 'original registration event claim');
  await validateStoredRegistrationEventEnvelopeV2(serialized, {
    semanticRequestDigest: expected.semanticRequestDigest,
    originalRegistrationRequestDigest: expected.semanticRequestDigest,
    originalRegistrationEventDigest: expected.eventDigest,
    originalRegistrationGlobalSequence: expected.globalSequence,
    changedReplayPriorControllerStateHeadDigest: null,
    projectAuthorityDigest: expected.projectAuthorityDigest,
    principalId: parseOpaqueId(project.principalId, 'original event project principal'),
    runId: expected.runId,
    authorityHead: {
      configurationEpoch: parseUint64(
        authorityHead.configurationEpoch, 'original event configuration epoch',
      ),
      configurationDigest: parseDigest(
        authorityHead.configurationDigest, 'original event configuration digest',
      ),
      headDigest: parseDigest(authorityHead.headDigest, 'original event authority head'),
    },
    priorControllerStateHeadDigest: parseDigest(
      event.priorControllerStateHeadDigest, 'original event prior state digest',
    ),
    claim: {
      claimKeyDigest: parseDigest(claim.claimKeyDigest, 'original event claim-key digest'),
      claimDigest: parseDigest(claim.claimDigest, 'original event claim digest'),
      rootedClaimValidationDigest: parseDigest(
        claim.rootedClaimValidationDigest, 'original event rooted-claim digest',
      ),
    },
  }, 201);
}

function canonicalBytes(
  row: SnapshotRow,
  key: RawRowKey,
  maximum: number,
): Readonly<{ text: string; record: Record<string, unknown> }> {
  const value = row[key];
  if (!(value instanceof Uint8Array) || value.byteLength === 0 || value.byteLength > maximum) {
    throw new TypeError(`${key} byte bounds are invalid`);
  }
  let decoded: string;
  try { decoded = new TextDecoder('utf-8', { fatal: true }).decode(value); }
  catch { throw new TypeError(`${key} is not UTF-8`); }
  const encoded = new TextEncoder().encode(decoded);
  if (!equalBytes(value, encoded)) throw new TypeError(`${key} is not byte-identical UTF-8`);
  return Object.freeze({
    text: decoded,
    record: parseCanonicalPrettyJson(decoded, maximum, key),
  });
}

function eventRefs(value: Record<string, unknown>, label: string): EventRefsV1 {
  const event = asRecord(value.event, label);
  const project = asRecord(event.project, `${label} project`);
  return Object.freeze({
    projectAuthorityDigest: parseDigest(
      project.projectAuthorityDigest, `${label} project digest`,
    ),
    semanticRequestDigest: parseDigest(
      event.semanticRequestDigest, `${label} request digest`,
    ),
    runId: parseOpaqueId(event.runId, `${label} run ID`),
    eventDigest: parseDigest(event.eventDigest, `${label} digest`),
    eventKind: textValue(event.eventKind, `${label} kind`),
    globalSequence: parseUint64(event.globalSequence, `${label} global sequence`),
    priorControllerStateHeadDigest: parseDigest(
      event.priorControllerStateHeadDigest, `${label} prior state digest`,
    ),
  });
}

function digest(row: SnapshotRow, key: RawRowKey): string {
  const value = row[key];
  if (!(value instanceof Uint8Array) || value.byteLength !== 32) {
    throw new TypeError(`${key} must be a SHA-256 bytea`);
  }
  return parseDigest(
    [...value].map((byte) => byte.toString(16).padStart(2, '0')).join(''), key,
  );
}

function text(row: SnapshotRow, key: RawRowKey): string {
  return textValue(row[key], key);
}

function textValue(value: unknown, label: string): string {
  if (typeof value !== 'string') throw new TypeError(`${label} must be text`);
  return value;
}

function opaque(row: SnapshotRow, key: RawRowKey): string {
  return parseOpaqueId(row[key], key);
}

function uint64(row: SnapshotRow, key: RawRowKey): string {
  return parseUint64(row[key], key);
}

function requireEqual(left: string, right: string): void {
  if (left !== right) throw new TypeError('joined provenance is inconsistent');
}

function equalBytes(left: Uint8Array, right: Uint8Array): boolean {
  return left.byteLength === right.byteLength
    && left.every((byte, index) => byte === right[index]);
}

function indeterminate(): ExactCommittedResultReadV1 {
  return Object.freeze({ kind: 'indeterminate' });
}
