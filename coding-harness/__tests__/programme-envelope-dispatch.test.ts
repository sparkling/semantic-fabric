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
import { parseJsonWithoutDuplicateKeys } from '../src/strict-json.js';

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

  it('recognizes schema v5 without enabling an incomplete parser', () => {
    expect(() => parseProgrammeEnvelope('{"schemaVersion":5}'))
      .toThrow('HARNESS_PROGRAMME_ENVELOPE_V5_NOT_ENABLED');
    expect(() => parseIssue8ProgrammeEnvelope('{"schemaVersion":5}')).toThrow();
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
