// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  parseIssue8ProgrammeEnvelope,
  serializeIssue8ProgrammeEnvelope,
} from '../src/issue-8-programme-envelope.js';
import {
  parseProgrammeEnvelope,
  serializeProgrammeEnvelope,
} from '../src/programme-envelope.js';
import { createProgrammeEnvelopeV6 } from '../src/programme-envelope-v6.js';
import { parseJsonWithoutDuplicateKeys } from '../src/strict-json.js';
import { diagnosticBlob } from './candidate-fixtures.js';
import { programmeV6Fixture } from './programme-v6-fixtures.js';

const fixture = readFileSync(
  new URL('./fixtures/issue-8-programme-envelope-v4.json', import.meta.url),
  'utf8',
);

describe('programme envelope version dispatcher', () => {
  it('delegates the original schema-v4 bytes to the frozen parser', () => {
    const generic = parseProgrammeEnvelope(fixture);
    const legacy = parseIssue8ProgrammeEnvelope(fixture);

    expect(generic).toEqual(legacy);
    expect(generic.envelopeDigest).toBe('1aa77aca451eed9211bcd59390098289b7fd47778a876b9809518c44a1ed79f7');
    expect(serializeProgrammeEnvelope(generic)).toBe(serializeIssue8ProgrammeEnvelope(legacy));

    const padded = `${fixture}${' '.repeat(20_000_001)}`;
    expect(parseProgrammeEnvelope(padded)).toEqual(parseIssue8ProgrammeEnvelope(padded));

    const escaped = fixture.replace('"schemaVersion"', '"schema\\u0056ersion"');
    expect(parseProgrammeEnvelope(escaped)).toEqual(parseIssue8ProgrammeEnvelope(escaped));

    const parsed = JSON.parse(fixture) as Record<string, unknown>;
    const reordered = JSON.stringify({
      authority: parsed.authority,
      ...parsed,
      schemaVersion: parsed.schemaVersion,
    });
    expect(parseProgrammeEnvelope(reordered)).toEqual(parseIssue8ProgrammeEnvelope(reordered));

    const nestedDuplicate = fixture.replace(
      '"step":"candidate-transaction"',
      '"step":"candidate-transaction","step":"candidate-transaction"',
    );
    expect(parseProgrammeEnvelope(nestedDuplicate))
      .toEqual(parseIssue8ProgrammeEnvelope(nestedDuplicate));
  });

  it('requires an externally anchored expectation before dispatching schema v5', () => {
    expect(() => parseProgrammeEnvelope('{"schemaVersion":5}'))
      .toThrow('HARNESS_PROGRAMME_ENVELOPE_V5_POLICY_ANCHOR_REQUIRED');
    expect(() => parseProgrammeEnvelope('{"schemaVersion":5}', { schemaVersion: 4 }))
      .toThrow('HARNESS_PROGRAMME_ENVELOPE_SCHEMA_MISMATCH');
    expect(() => parseProgrammeEnvelope(fixture, {
      schemaVersion: 5,
      policyFingerprint: 'a'.repeat(64),
    })).toThrow('HARNESS_PROGRAMME_ENVELOPE_SCHEMA_DOWNGRADE');
    expect(() => parseIssue8ProgrammeEnvelope('{"schemaVersion":5}')).toThrow();
  });

  it('dispatches V6 only with its exact outer policy anchor and downgrade law', () => {
    const fixtureV6 = programmeV6Fixture('not-started');
    const envelope = createProgrammeEnvelopeV6({
      policy: fixtureV6.policy.snapshot,
      rufloEvidence: fixtureV6.rufloEvidence,
      candidateTransactionEvidence: fixtureV6.candidateTransactionEvidence,
      receipt: fixtureV6.receipt,
      diagnosticBlob,
    }, fixtureV6.policy.fingerprint);
    const expectation = {
      schemaVersion: 6 as const,
      policyFingerprint: fixtureV6.policy.fingerprint,
    };
    const serialized = serializeProgrammeEnvelope(envelope, expectation);

    expect(parseProgrammeEnvelope(serialized, expectation)).toEqual(envelope);
    expect(() => parseProgrammeEnvelope(serialized)).toThrow(/V6_POLICY_ANCHOR_REQUIRED/);
    expect(() => serializeProgrammeEnvelope(envelope as any)).toThrow(/V6_POLICY_ANCHOR_REQUIRED/);
    expect(() => parseProgrammeEnvelope(serialized, {
      schemaVersion: 5, policyFingerprint: fixtureV6.policy.fingerprint,
    })).toThrow('HARNESS_PROGRAMME_ENVELOPE_SCHEMA_MISMATCH');
    expect(() => parseProgrammeEnvelope('{"schemaVersion":5}', expectation))
      .toThrow('HARNESS_PROGRAMME_ENVELOPE_SCHEMA_DOWNGRADE');
    expect(() => parseProgrammeEnvelope(fixture, expectation))
      .toThrow('HARNESS_PROGRAMME_ENVELOPE_SCHEMA_DOWNGRADE');
  });

  it('rejects every malformed supplied expectation before version dispatch', () => {
    const envelope = parseIssue8ProgrammeEnvelope(fixture);
    for (const expectation of [
      null,
      {},
      { schemaVersion: '5' },
      { schemaVersion: 6 },
      { schemaVersion: 4, policyFingerprint: 'a'.repeat(64) },
      { schemaVersion: 5, policyFingerprint: 'not-a-digest' },
      { schemaVersion: 5, policyFingerprint: '0'.repeat(64) },
      { schemaVersion: 5, policyFingerprint: 'a'.repeat(64), extra: true },
      { schemaVersion: 6, policyFingerprint: 'not-a-digest' },
      { schemaVersion: 6, policyFingerprint: '0'.repeat(64) },
      { schemaVersion: 6, policyFingerprint: 'a'.repeat(64), extra: true },
    ]) {
      expect(() => (parseProgrammeEnvelope as any)(fixture, expectation)).toThrow();
      expect(() => (serializeProgrammeEnvelope as any)(envelope, expectation)).toThrow();
    }
  });

  it('rejects unsupported, ambiguous, and malformed serializations', () => {
    for (const serialized of [
      '{}',
      '{"schemaVersion":3}',
      '{"schemaVersion":6}',
      '{"schemaVersion":"4"}',
      '[]',
    ]) {
      expect(() => parseProgrammeEnvelope(serialized)).toThrow();
    }
    expect(() => parseProgrammeEnvelope('{')).toThrow('programme envelope is not valid JSON');
    expect(() => parseProgrammeEnvelope('')).toThrow('programme envelope serialization is invalid');
    for (const duplicate of [
      '{"schemaVersion":4,"schemaVersion":5}',
      '{"schemaVersion":4,"schemaVersion":6}',
      '{"schemaVersion":4,"schema\\u0056ersion":5}',
    ]) {
      expect(() => parseProgrammeEnvelope(duplicate))
        .toThrow('HARNESS_PROGRAMME_ENVELOPE_SCHEMA_AMBIGUOUS');
    }
    expect(() => parseProgrammeEnvelope('{"schemaVersion":4,"extra":true}'))
      .toThrow(/invalid keys/);
  });

  it('rejects escaped and nested duplicate keys in strict policy JSON', () => {
    for (const serialized of [
      '{"outer":{"key":1,"k\\u0065y":2}}',
      '{"items":[{"key":1,"key":2}]}',
    ]) {
      expect(() => parseJsonWithoutDuplicateKeys(serialized, 'policy'))
        .toThrow(/duplicate JSON key: key/);
    }
  });
});
