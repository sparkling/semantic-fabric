// SPDX-License-Identifier: MIT

import { asRecord } from './contracts.js';
import {
  parseIssue8ProgrammeEnvelope,
  serializeIssue8ProgrammeEnvelope,
  type Issue8ProgrammeEnvelope,
} from './issue-8-programme-envelope.js';
import { inspectJsonRoot, parseJsonDocument } from './strict-json.js';

export type ProgrammeEnvelope = Issue8ProgrammeEnvelope;

export function parseProgrammeEnvelope(serialized: string): ProgrammeEnvelope {
  if (typeof serialized !== 'string' || serialized.length === 0) {
    throw new TypeError('programme envelope serialization is invalid');
  }
  const inspected = inspectJsonRoot(serialized, ['schemaVersion'], 'programme envelope');
  if (inspected.topLevelKeyCounts.schemaVersion !== 1) {
    parseJsonDocument(serialized, 'programme envelope');
    throw new TypeError('HARNESS_PROGRAMME_ENVELOPE_SCHEMA_AMBIGUOUS');
  }
  const [version] = inspected.topLevelNumberValues.schemaVersion;
  if (version === 4) return parseIssue8ProgrammeEnvelope(serialized);
  if (version === 5) throw new Error('HARNESS_PROGRAMME_ENVELOPE_V5_NOT_ENABLED');
  asRecord(parseJsonDocument(serialized, 'programme envelope'), 'programme envelope');
  throw new TypeError('HARNESS_PROGRAMME_ENVELOPE_SCHEMA_UNSUPPORTED');
}

export function serializeProgrammeEnvelope(envelope: ProgrammeEnvelope): string {
  return serializeIssue8ProgrammeEnvelope(envelope);
}
