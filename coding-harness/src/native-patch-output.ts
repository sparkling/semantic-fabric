// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  asNonEmptyString,
  asRecord,
  assertExactKeys,
} from './contracts.js';
import type { NativeModelOperation } from './evidence.js';

export const NATIVE_PATCH_MAX_CHARS = 256_000;
export const NATIVE_PATCH_MAX_BYTES = 1_000_000;

export function parseNativePatchResponse(value: unknown): string {
  const input = asRecord(value, 'patch response');
  assertExactKeys(input, ['patch'], 'patch response');
  return parseNativePatchPayload(input.patch, 'patch response.patch');
}

export function parseNativePatchPayload(value: unknown, label: string): string {
  const payload = asNonEmptyString(value, label);
  if (Buffer.byteLength(payload, 'utf8') > NATIVE_PATCH_MAX_BYTES
    || Array.from(payload).length > NATIVE_PATCH_MAX_CHARS
    || !payload.startsWith('diff --git ')) {
    throw new Error('HARNESS_NATIVE_PATCH_INVALID');
  }
  return payload;
}

export function patchPayloadSha256ForNativeOutput(
  operation: NativeModelOperation,
  output: unknown,
): string | null {
  if (operation !== 'implementation' && operation !== 'repair') return null;
  return createHash('sha256')
    .update(parseNativePatchResponse(output), 'utf8')
    .digest('hex');
}
