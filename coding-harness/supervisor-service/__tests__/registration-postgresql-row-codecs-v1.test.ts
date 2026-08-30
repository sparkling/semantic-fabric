// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { describe, expect, it } from 'vitest';
import { decodePostgresExactResultRowsV1 } from
  '../src/registration-postgresql-row-codecs-v1.js';
import {
  canonical,
  canonicalPretty,
  coherentlyMutatedRegistrationEnvelope,
  exactStoredResult,
  REGISTERED_RUN,
  registrationEnvelope,
  sha256Text,
} from './registration-fixtures.js';

const MAX_UINT64 = '18446744073709551615';

describe('PostgreSQL exact-result joined-row codec V1', () => {
  it('distinguishes a true root miss from complete immutable 201 and 409 rows', async () => {
    expect(await decodePostgresExactResultRowsV1([])).toEqual({ kind: 'absent' });

    for (const status of [201, 409] as const) {
      const decoded = await decodePostgresExactResultRowsV1([rawRow(status)]);
      expect(decoded).toEqual(codecStoredResult(status));
      expect(Object.isFrozen(decoded)).toBe(true);
      if (decoded.kind === 'found') expect(Object.isFrozen(decoded.row)).toBe(true);
    }
  });

  it('snapshots driver rows and bytea before the first asynchronous hash', async () => {
    const raw = rawRow(409);
    const response = raw.result_serialized_response as Buffer;
    const pending = decodePostgresExactResultRowsV1([raw]);
    response.fill(0);
    raw.result_response_status_text = '201';
    raw.run_original_registration_event_digest = Buffer.alloc(32, 0);

    expect(await pending).toEqual(codecStoredResult(409));
  });

  it.each([
    ['multiple roots', () => [rawRow(201), rawRow(201)]],
    ['sparse root', () => new Array(1)],
    ['non-array result', () => ({ rows: [rawRow(201)] })],
    ['missing current event', () => rowsWith('current_event_digest', null)],
    ['missing run', () => rowsWith('run_project_authority_digest', null)],
    ['missing original event', () => rowsWith('original_event_digest', null)],
    ['extra alias', () => rowsWith('unexpected_alias', 'x')],
    ['missing alias', () => {
      const row = rawRow(201); delete row.original_event_global_sequence_text; return [row];
    }],
  ])('treats %s as damage, never absence', async (_label, makeRows) => {
    expect(await decodePostgresExactResultRowsV1(makeRows())).toEqual({
      kind: 'indeterminate',
    });
  });

  it.each([
    ['31-byte digest', 'result_project_authority_digest', Buffer.alloc(31, 1)],
    ['33-byte digest', 'current_event_digest', Buffer.alloc(33, 1)],
    ['zero digest', 'original_event_digest', Buffer.alloc(32, 0)],
    ['foreign current project', 'current_event_project_authority_digest', digestBytes('foreign')],
    ['foreign run project', 'run_project_authority_digest', digestBytes('foreign-run')],
    ['foreign original project', 'original_event_project_authority_digest', digestBytes('foreign-original')],
    ['wrong current event reference', 'result_current_event_digest', digestBytes('wrong-current')],
    ['wrong current request', 'current_event_semantic_request_digest', digestBytes('wrong-request')],
    ['wrong original request', 'original_event_semantic_request_digest', digestBytes('wrong-original-request')],
    ['wrong run ID', 'run_run_id', 'foreign_run_20260830'],
    ['wrong original event reference', 'run_original_registration_event_digest', digestBytes('wrong-original-event')],
  ])('rejects %s without synthesizing provenance', async (_label, key, value) => {
    expect(await decodePostgresExactResultRowsV1(rowsWith(key, value))).toEqual({
      kind: 'indeterminate',
    });
  });

  it.each([
    ['invalid UTF-8', Buffer.from([0xc3, 0x28])],
    ['UTF-8 BOM', Buffer.concat([Buffer.from([0xef, 0xbb, 0xbf]), Buffer.from('{}\n')])],
    ['CRLF JSON', Buffer.from('{\r\n}\r\n')],
    ['missing final LF', Buffer.from('{}')],
    ['embedded NUL', Buffer.from('{"x":"\u0000"}\n')],
    ['oversized response', Buffer.alloc(196_609, 0x78)],
  ])('rejects %s even when its raw byte hash matches', async (_label, bytes) => {
    const row = rawRow(201);
    setBytesAndHash(row, 'result_serialized_response', bytes);
    expect(await decodePostgresExactResultRowsV1([row])).toEqual({ kind: 'indeterminate' });
  });

  it('rejects request, response, and event byte/hash disagreement', async () => {
    for (const hashKey of [
      'result_serialized_request_sha256',
      'result_serialized_response_sha256',
      'current_event_serialized_envelope_sha256',
      'original_event_serialized_envelope_sha256',
    ]) {
      expect(await decodePostgresExactResultRowsV1(
        rowsWith(hashKey, digestBytes(`bad-${hashKey}`)),
      )).toEqual({ kind: 'indeterminate' });
    }
  });

  it('rejects otherwise-valid matching-hash noncanonical bytes in every column', async () => {
    const request = rawRow(409);
    setBytesAndHash(request, 'result_serialized_request', withoutFinalLf(
      request, 'result_serialized_request',
    ));

    const response = rawRow(409);
    setBytesAndHash(response, 'result_serialized_response', withoutFinalLf(
      response, 'result_serialized_response',
    ));

    const current = rawRow(409);
    setCurrentEnvelope(
      current, withoutFinalLf(current, 'current_event_serialized_envelope').toString(),
    );

    const original = rawRow(409);
    setBytesAndHash(original, 'original_event_serialized_envelope', withoutFinalLf(
      original, 'original_event_serialized_envelope',
    ));

    for (const row of [request, response, current, original]) {
      expect(await decodePostgresExactResultRowsV1([row]))
        .toEqual({ kind: 'indeterminate' });
    }
  });

  it.each([
    '01', '+1', '-1', '1.0', '1e0', ' 1', `${MAX_UINT64}0`, 0, 1, 1n,
  ])('rejects noncanonical selected uint64 value %s', async (value) => {
    expect(await decodePostgresExactResultRowsV1(
      rowsWith('original_event_global_sequence_text', value),
    )).toEqual({ kind: 'indeterminate' });
  });

  it('accepts the maximum uint64 selected as canonical text', async () => {
    const row = rawRow(409);
    setOriginalEventSequence(row, MAX_UINT64);
    const decoded = await decodePostgresExactResultRowsV1([row]);

    expect(decoded.kind).toBe('found');
    if (decoded.kind === 'found') {
      expect(decoded.row.originalRegistrationGlobalSequence).toBe(MAX_UINT64);
    }
  });

  it('requires byte-identical original/current envelope provenance for 201', async () => {
    const row = rawRow(201);
    const envelope = JSON.parse(bytesText(row, 'current_event_serialized_envelope'));
    envelope.signature.valueBase64Url = 'B'.repeat(86);
    setCurrentEnvelope(row, canonicalPretty(envelope));

    expect(await decodePostgresExactResultRowsV1([row]))
      .toEqual({ kind: 'indeterminate' });
  });

  it.each([
    ['current event kind', (row: Record<string, unknown>) => {
      mutateCurrentEvent(row, (event) => { event.eventKind = 'claim-registered-v2'; });
    }],
    ['current semantic request', (row: Record<string, unknown>) => {
      const original = bytesHex(row, 'run_original_registration_request_digest');
      mutateCurrentEvent(row, (event) => { event.semanticRequestDigest = original; });
      row.result_semantic_request_digest = hexBytes(original);
      mutateResponse(row, (response) => { response.semanticRequestDigest = original; });
    }],
    ['current event digest', (row: Record<string, unknown>) => {
      const original = bytesHex(row, 'run_original_registration_event_digest');
      mutateCurrentEvent(row, (event) => { event.eventDigest = original; }, false);
    }],
  ])('requires status-specific 409 %s provenance', async (_label, mutate) => {
    const row = rawRow(409);
    mutate(row);
    expect(await decodePostgresExactResultRowsV1([row]))
      .toEqual({ kind: 'indeterminate' });
  });

  it('binds stored event bytes to event aliases and exact response bytes', async () => {
    const currentMismatch = rawRow(409);
    setBytesAndHash(
      currentMismatch,
      'current_event_serialized_envelope',
      Buffer.from(registrationEnvelope(201)),
    );
    expect(await decodePostgresExactResultRowsV1([currentMismatch]))
      .toEqual({ kind: 'indeterminate' });

    const originalMismatch = rawRow(409);
    const original = JSON.parse(registrationEnvelope(201));
    original.event.eventDigest = sha256Text('coherent-looking-foreign-event');
    setBytesAndHash(
      originalMismatch,
      'original_event_serialized_envelope',
      Buffer.from(canonicalPretty(original)),
    );
    expect(await decodePostgresExactResultRowsV1([originalMismatch]))
      .toEqual({ kind: 'indeterminate' });
  });

  it.each([
    ['authority flag', true, (envelope: any) => {
      envelope.event.externalAdministrationVerified = true;
    }],
    ['claim body schema', true, (envelope: any) => {
      envelope.event.body.unreviewedClaimAuthority = false;
    }],
    ['signature algorithm', false, (envelope: any) => {
      envelope.signature.algorithm = 'rsa-pss';
    }],
    ['unknown event field', true, (envelope: any) => {
      envelope.event.unreviewedAuthority = false;
    }],
  ])('rejects fully rehashed original-event %s damage', async (
    _label, rehash, mutate,
  ) => {
    const row = rawRow(409);
    mutateOriginalEnvelope(row, mutate, rehash);
    expect(await decodePostgresExactResultRowsV1([row]))
      .toEqual({ kind: 'indeterminate' });
  });

  it('rejects outer-array, row, and bytea Proxies before invoking traps', async () => {
    let arrayTraps = 0;
    const rows = new Proxy([rawRow(201)], trappingHandler(() => { arrayTraps += 1; }));
    expect(await decodePostgresExactResultRowsV1(rows))
      .toEqual({ kind: 'indeterminate' });
    expect(arrayTraps).toBe(0);

    const accessor = rawRow(201);
    let getterReads = 0;
    Object.defineProperty(accessor, 'current_event_digest', {
      enumerable: true,
      get: () => { getterReads += 1; return Buffer.alloc(32, 1); },
    });
    expect(await decodePostgresExactResultRowsV1([accessor]))
      .toEqual({ kind: 'indeterminate' });
    expect(getterReads).toBe(0);

    let rowTraps = 0;
    const proxy = new Proxy(rawRow(201), trappingHandler(() => { rowTraps += 1; }));
    expect(await decodePostgresExactResultRowsV1([proxy]))
      .toEqual({ kind: 'indeterminate' });
    expect(rowTraps).toBe(0);

    const byteaRow = rawRow(201);
    let byteaTraps = 0;
    byteaRow.result_project_authority_digest = new Proxy(
      byteaRow.result_project_authority_digest as Buffer,
      trappingHandler(() => { byteaTraps += 1; }),
    );
    expect(await decodePostgresExactResultRowsV1([byteaRow]))
      .toEqual({ kind: 'indeterminate' });
    expect(byteaTraps).toBe(0);
  });
});

function rawRow(status: 201 | 409): Record<string, unknown> {
  const stored = codecStoredResult(status).row;
  const response = JSON.parse(stored.serializedResponse);
  const currentEnvelope = String(response.serializedEventEnvelope);
  const currentEvent = JSON.parse(currentEnvelope).event;
  const originalEnvelope = status === 201 ? currentEnvelope : storedOriginalEnvelope();
  const originalEvent = JSON.parse(originalEnvelope).event;
  return {
    result_project_authority_digest: hexBytes(stored.projectAuthorityDigest),
    result_semantic_request_digest: hexBytes(stored.semanticRequestDigest),
    result_run_id: currentEvent.runId,
    result_serialized_request: Buffer.from(stored.serializedRequest),
    result_serialized_request_sha256: hexBytes(stored.serializedRequestSha256),
    result_response_status_text: String(stored.responseStatus),
    result_response_content_type: stored.responseContentType,
    result_serialized_response: Buffer.from(stored.serializedResponse),
    result_serialized_response_sha256: hexBytes(stored.serializedResponseSha256),
    result_current_event_digest: hexBytes(String(currentEvent.eventDigest)),
    current_event_project_authority_digest: hexBytes(stored.projectAuthorityDigest),
    current_event_digest: hexBytes(String(currentEvent.eventDigest)),
    current_event_kind: currentEvent.eventKind,
    current_event_semantic_request_digest: hexBytes(stored.semanticRequestDigest),
    current_event_prior_controller_state_head_digest:
      hexBytes(String(currentEvent.priorControllerStateHeadDigest)),
    current_event_serialized_envelope: Buffer.from(currentEnvelope),
    current_event_serialized_envelope_sha256: digestBytes(currentEnvelope),
    run_project_authority_digest: hexBytes(stored.projectAuthorityDigest),
    run_run_id: currentEvent.runId,
    run_original_registration_request_digest:
      hexBytes(stored.originalRegistrationRequestDigest),
    run_original_registration_event_digest: hexBytes(stored.originalRegistrationEventDigest),
    original_event_project_authority_digest: hexBytes(stored.projectAuthorityDigest),
    original_event_digest: hexBytes(stored.originalRegistrationEventDigest),
    original_event_semantic_request_digest:
      hexBytes(stored.originalRegistrationRequestDigest),
    original_event_global_sequence_text: stored.originalRegistrationGlobalSequence,
    original_event_serialized_envelope: Buffer.from(originalEnvelope),
    original_event_serialized_envelope_sha256: digestBytes(originalEnvelope),
  };
}

function codecStoredResult(status: 201 | 409): ReturnType<typeof exactStoredResult> {
  if (status === 201) return exactStoredResult(201);
  const originalEnvelope = storedOriginalEnvelope();
  const originalEvent = JSON.parse(originalEnvelope).event;
  const currentEnvelope = coherentlyMutatedRegistrationEnvelope(409, (event) => {
    event.previousGlobal.eventDigest = originalEvent.eventDigest;
    event.previousRun.eventDigest = originalEvent.eventDigest;
    event.body.registrationEventDigest = originalEvent.eventDigest;
    event.body.outcomeEvidenceDigest = sha256Text(canonical({
      domain:
        'semantic-fabric/programme-capture/supervisor-registration-changed-replay-evidence-v2',
      originalRegistrationRequestDigest: REGISTERED_RUN.originalRegistrationRequestDigest,
      originalRegistrationEventDigest: originalEvent.eventDigest,
      changedRegistrationRequestDigest: event.semanticRequestDigest,
      project: event.project,
      authorityHead: event.authorityHead,
    }));
  });
  return exactStoredResult(409, { serializedEventEnvelope: currentEnvelope }, {
    originalRegistrationEventDigest: originalEvent.eventDigest,
  });
}

function storedOriginalEnvelope(): string {
  return coherentlyMutatedRegistrationEnvelope(201, (event) => {
    event.semanticRequestDigest = REGISTERED_RUN.originalRegistrationRequestDigest;
  });
}

function rowsWith(key: string, value: unknown): unknown[] {
  return [{ ...rawRow(201), [key]: value }];
}

function hexBytes(value: string): Buffer {
  return Buffer.from(value, 'hex');
}

function digestBytes(value: string): Buffer {
  return createHash('sha256').update(value).digest();
}

function setBytesAndHash(
  row: Record<string, unknown>, key: string, bytes: Buffer,
): void {
  row[key] = Buffer.from(bytes);
  row[`${key}_sha256`] = createHash('sha256').update(bytes).digest();
}

function setOriginalEventSequence(row: Record<string, unknown>, value: string): void {
  const envelope = JSON.parse(
    (row.original_event_serialized_envelope as Buffer).toString('utf8'),
  );
  envelope.event.globalSequence = value;
  if (value !== '1') {
    envelope.event.previousGlobal = {
      kind: 'semantic-event',
      eventDigest: sha256Text('original-event-predecessor'),
      semanticReceiptDigest: sha256Text('original-event-predecessor-receipt'),
    };
  }
  const { eventDigest: _ignored, ...eventBody } = envelope.event;
  envelope.event.eventDigest = sha256Text(canonical({
    domain: 'semantic-fabric/programme-capture/supervisor-run-event-digest-v2',
    event: eventBody,
  }));
  const bytes = Buffer.from(canonicalPretty(envelope));
  setBytesAndHash(row, 'original_event_serialized_envelope', bytes);
  row.original_event_global_sequence_text = value;
  row.original_event_digest = hexBytes(envelope.event.eventDigest);
  row.run_original_registration_event_digest = hexBytes(envelope.event.eventDigest);
}

function setCurrentEnvelope(row: Record<string, unknown>, serialized: string): void {
  const event = JSON.parse(serialized).event;
  setBytesAndHash(row, 'current_event_serialized_envelope', Buffer.from(serialized));
  row.current_event_digest = hexBytes(String(event.eventDigest));
  row.result_current_event_digest = hexBytes(String(event.eventDigest));
  row.current_event_kind = event.eventKind;
  row.current_event_semantic_request_digest = hexBytes(String(event.semanticRequestDigest));
  row.current_event_prior_controller_state_head_digest =
    hexBytes(String(event.priorControllerStateHeadDigest));
  mutateResponse(row, (response) => { response.serializedEventEnvelope = serialized; });
}

function mutateCurrentEvent(
  row: Record<string, unknown>,
  mutate: (event: Record<string, any>) => void,
  rehash = true,
): void {
  const envelope = JSON.parse(bytesText(row, 'current_event_serialized_envelope'));
  mutate(envelope.event);
  if (rehash) {
    const { eventDigest: _ignored, ...eventBody } = envelope.event;
    envelope.event.eventDigest = sha256Text(canonical({
      domain: 'semantic-fabric/programme-capture/supervisor-run-event-digest-v2',
      event: eventBody,
    }));
  }
  setCurrentEnvelope(row, canonicalPretty(envelope));
}

function mutateResponse(
  row: Record<string, unknown>, mutate: (response: Record<string, any>) => void,
): void {
  const response = JSON.parse(bytesText(row, 'result_serialized_response'));
  mutate(response);
  const { resultDigest: _ignored, ...body } = response;
  response.resultDigest = sha256Text(canonical({
    domain: 'semantic-fabric/programme-capture/supervisor-service-result-digest-v2',
    result: body,
  }));
  setBytesAndHash(row, 'result_serialized_response', Buffer.from(canonicalPretty(response)));
}

function mutateOriginalEnvelope(
  row: Record<string, unknown>,
  mutate: (envelope: Record<string, any>) => void,
  rehash: boolean,
): void {
  const envelope = JSON.parse(bytesText(row, 'original_event_serialized_envelope'));
  mutate(envelope);
  if (rehash) {
    const { eventDigest: _ignored, ...eventBody } = envelope.event;
    envelope.event.eventDigest = sha256Text(canonical({
      domain: 'semantic-fabric/programme-capture/supervisor-run-event-digest-v2',
      event: eventBody,
    }));
  }
  setBytesAndHash(
    row, 'original_event_serialized_envelope', Buffer.from(canonicalPretty(envelope)),
  );
  row.original_event_digest = hexBytes(envelope.event.eventDigest);
  row.run_original_registration_event_digest = hexBytes(envelope.event.eventDigest);
}

function bytesText(row: Record<string, unknown>, key: string): string {
  return (row[key] as Buffer).toString('utf8');
}

function bytesHex(row: Record<string, unknown>, key: string): string {
  return (row[key] as Buffer).toString('hex');
}

function withoutFinalLf(row: Record<string, unknown>, key: string): Buffer {
  const value = row[key] as Buffer;
  expect(value[value.length - 1]).toBe(0x0a);
  return value.subarray(0, -1);
}

function trappingHandler(onTrap: () => void): ProxyHandler<any> {
  return {
    get: (target, key, receiver) => {
      onTrap(); return Reflect.get(target, key, receiver);
    },
    getPrototypeOf: (target) => {
      onTrap(); return Reflect.getPrototypeOf(target);
    },
    ownKeys: (target) => {
      onTrap(); return Reflect.ownKeys(target);
    },
    getOwnPropertyDescriptor: (target, key) => {
      onTrap(); return Reflect.getOwnPropertyDescriptor(target, key);
    },
  };
}
