// SPDX-License-Identifier: MIT

import { SHA256_PATTERN, asRecord, assertExactKeys, deepFreeze } from './contracts.js';
import {
  parseIssue8ProgrammeEnvelope,
  serializeIssue8ProgrammeEnvelope,
  type Issue8ProgrammeEnvelope,
} from './issue-8-programme-envelope.js';
import {
  parseProgrammeEnvelopeV5,
  serializeProgrammeEnvelopeV5,
  type ProgrammeEnvelopeV5,
} from './programme-envelope-v5.js';
import { inspectJsonRoot, parseJsonDocument } from './strict-json.js';

export type ProgrammeEnvelope = Issue8ProgrammeEnvelope | ProgrammeEnvelopeV5;
export type ProgrammeEnvelopeExpectation =
  | Readonly<{ schemaVersion: 4 }>
  | Readonly<{ schemaVersion: 5; policyFingerprint: string }>;

export function parseProgrammeEnvelope(serialized: string): Issue8ProgrammeEnvelope;
export function parseProgrammeEnvelope(
  serialized: string,
  expectation: Readonly<{ schemaVersion: 4 }>,
): Issue8ProgrammeEnvelope;
export function parseProgrammeEnvelope(
  serialized: string,
  expectation: Readonly<{ schemaVersion: 5; policyFingerprint: string }>,
): ProgrammeEnvelopeV5;
export function parseProgrammeEnvelope(
  serialized: string,
  expectation?: ProgrammeEnvelopeExpectation,
): ProgrammeEnvelope {
  const expected = parseExpectation(expectation);
  if (typeof serialized !== 'string' || serialized.length === 0) {
    throw new TypeError('programme envelope serialization is invalid');
  }
  const inspected = inspectJsonRoot(serialized, ['schemaVersion'], 'programme envelope');
  if (inspected.topLevelKeyCounts.schemaVersion !== 1) {
    parseJsonDocument(serialized, 'programme envelope');
    throw new TypeError('HARNESS_PROGRAMME_ENVELOPE_SCHEMA_AMBIGUOUS');
  }
  const [version] = inspected.topLevelNumberValues.schemaVersion;
  if (version === 4) {
    if (expected?.schemaVersion === 5) {
      throw new Error('HARNESS_PROGRAMME_ENVELOPE_SCHEMA_DOWNGRADE');
    }
    return parseIssue8ProgrammeEnvelope(serialized);
  }
  if (version === 5) {
    if (expected === undefined) {
      throw new Error('HARNESS_PROGRAMME_ENVELOPE_V5_POLICY_ANCHOR_REQUIRED');
    }
    if (expected.schemaVersion !== 5) {
      throw new Error('HARNESS_PROGRAMME_ENVELOPE_SCHEMA_MISMATCH');
    }
    return parseProgrammeEnvelopeV5(serialized, expected.policyFingerprint);
  }
  asRecord(parseJsonDocument(serialized, 'programme envelope'), 'programme envelope');
  throw new TypeError('HARNESS_PROGRAMME_ENVELOPE_SCHEMA_UNSUPPORTED');
}

export function serializeProgrammeEnvelope(envelope: Issue8ProgrammeEnvelope): string;
export function serializeProgrammeEnvelope(
  envelope: Issue8ProgrammeEnvelope,
  expectation: Readonly<{ schemaVersion: 4 }>,
): string;
export function serializeProgrammeEnvelope(
  envelope: ProgrammeEnvelopeV5,
  expectation: Readonly<{ schemaVersion: 5; policyFingerprint: string }>,
): string;
export function serializeProgrammeEnvelope(
  envelope: ProgrammeEnvelope,
  expectation?: ProgrammeEnvelopeExpectation,
): string {
  const expected = parseExpectation(expectation);
  const input = asRecord(envelope, 'programme envelope');
  if (input.schemaVersion === 4) {
    if (expected?.schemaVersion === 5) {
      throw new Error('HARNESS_PROGRAMME_ENVELOPE_SCHEMA_DOWNGRADE');
    }
    return serializeIssue8ProgrammeEnvelope(envelope as Issue8ProgrammeEnvelope);
  }
  if (input.schemaVersion === 5) {
    if (expected === undefined) {
      throw new Error('HARNESS_PROGRAMME_ENVELOPE_V5_POLICY_ANCHOR_REQUIRED');
    }
    if (expected.schemaVersion !== 5) {
      throw new Error('HARNESS_PROGRAMME_ENVELOPE_SCHEMA_MISMATCH');
    }
    return serializeProgrammeEnvelopeV5(
      envelope as ProgrammeEnvelopeV5,
      expected.policyFingerprint,
    );
  }
  throw new TypeError('HARNESS_PROGRAMME_ENVELOPE_SCHEMA_UNSUPPORTED');
}

function parseExpectation(value: unknown): ProgrammeEnvelopeExpectation | undefined {
  if (value === undefined) return undefined;
  const input = asRecord(value, 'programme envelope expectation');
  if (input.schemaVersion === 4) {
    assertExactKeys(input, ['schemaVersion'], 'programme envelope expectation');
    return deepFreeze({ schemaVersion: 4 });
  }
  if (input.schemaVersion === 5) {
    assertExactKeys(
      input,
      ['schemaVersion', 'policyFingerprint'],
      'programme envelope expectation',
    );
    if (typeof input.policyFingerprint !== 'string'
      || !SHA256_PATTERN.test(input.policyFingerprint)
      || input.policyFingerprint === '0'.repeat(64)) {
      throw new TypeError('HARNESS_PROGRAMME_ENVELOPE_EXPECTATION_INVALID');
    }
    return deepFreeze({ schemaVersion: 5, policyFingerprint: input.policyFingerprint });
  }
  throw new TypeError('HARNESS_PROGRAMME_ENVELOPE_EXPECTATION_INVALID');
}
