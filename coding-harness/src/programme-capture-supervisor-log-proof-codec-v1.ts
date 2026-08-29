// SPDX-License-Identifier: MIT

import { isProxy } from 'node:util/types';
import { asClosedRecord, assertExactKeys } from './contracts.js';
import {
  PROGRAMME_CAPTURE_SUPERVISOR_LOG_PROOF_INPUT_KEYS_V1,
  verifyProgrammeCaptureSupervisorRegistrationLogProofV1,
  type ProgrammeCaptureSupervisorRegistrationLogProofInputV1,
  type ProgrammeCaptureSupervisorRegistrationLogProofValidationV1,
} from './programme-capture-supervisor-log-proof-v1.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export const PROGRAMME_CAPTURE_SUPERVISOR_LOG_PROOF_VALIDATION_MAX_BYTES_V1 = 32_768;

export async function createProgrammeCaptureSupervisorRegistrationLogProofValidationBlobV1(
  value: ProgrammeCaptureSupervisorRegistrationLogProofInputV1,
): Promise<string> {
  return serialize(await verifyProgrammeCaptureSupervisorRegistrationLogProofV1(value));
}

export async function replayProgrammeCaptureSupervisorRegistrationLogProofValidationV1(
  value: ProgrammeCaptureSupervisorRegistrationLogProofInputV1 & Readonly<{
    serializedValidation: string;
  }>,
): Promise<ProgrammeCaptureSupervisorRegistrationLogProofValidationV1> {
  const input = closedRecord(value, 'supervisor registration log proof replay input');
  assertExactKeys(input, [
    'serializedValidation', ...PROGRAMME_CAPTURE_SUPERVISOR_LOG_PROOF_INPUT_KEYS_V1,
  ], 'supervisor registration log proof replay input');
  const serializedValidation = parseCanonicalValidation(input.serializedValidation);
  const verificationInput = Object.fromEntries(
    PROGRAMME_CAPTURE_SUPERVISOR_LOG_PROOF_INPUT_KEYS_V1.map((key) => [key, input[key]]),
  ) as unknown as ProgrammeCaptureSupervisorRegistrationLogProofInputV1;
  const replayed = await verifyProgrammeCaptureSupervisorRegistrationLogProofV1(
    verificationInput,
  );
  if (serialize(replayed) !== serializedValidation) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_LOG_PROOF_VALIDATION_REPLAY_MISMATCH');
  }
  return replayed;
}

function serialize(value: ProgrammeCaptureSupervisorRegistrationLogProofValidationV1): string {
  return `${JSON.stringify(value, null, 2)}\n`;
}

function parseCanonicalValidation(value: unknown): string {
  if (typeof value !== 'string'
    || Buffer.byteLength(value, 'utf8')
      > PROGRAMME_CAPTURE_SUPERVISOR_LOG_PROOF_VALIDATION_MAX_BYTES_V1
    || decodeCanonicalUtf8(value) !== value) {
    throw new TypeError('supervisor log proof validation must be bounded canonical UTF-8 JSON');
  }
  const parsed = parseJsonWithoutDuplicateKeys(value, 'supervisor log proof validation');
  if (`${JSON.stringify(parsed, null, 2)}\n` !== value) {
    throw new Error('HARNESS_CAPTURE_SUPERVISOR_LOG_PROOF_VALIDATION_CANONICAL_REQUIRED');
  }
  return value;
}

function closedRecord(value: unknown, label: string): Record<string, unknown> {
  if (isProxy(value)) throw new TypeError(`${label} must not be a Proxy`);
  return asClosedRecord(value, label);
}

function decodeCanonicalUtf8(value: string): string {
  try { return new TextDecoder('utf-8', { fatal: true }).decode(Buffer.from(value, 'utf8')); }
  catch { return ''; }
}
