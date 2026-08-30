// SPDX-License-Identifier: MIT

import { parseTaskOpaqueId } from './acceptance-task-v3.js';
import {
  SHA256_PATTERN,
  asClosedRecord,
  assertExactKeys,
} from './contracts.js';
import { digestValue } from './receipts.js';

export function programmeCaptureRunClaimKeyDigestV1(value: Readonly<{
  projectAuthorityDigest: string;
  runId: string;
}>): string {
  const input = asClosedRecord(value, 'programme capture run-claim key input');
  assertExactKeys(
    input, ['projectAuthorityDigest', 'runId'], 'programme capture run-claim key input',
  );
  return digestValue({
    schemaVersion: 1,
    transactionKind: 'programme-capture-v1',
    keyKind: 'run-claim-key-v1',
    projectAuthorityDigest: parseProgrammeCaptureDigestV1(
      input.projectAuthorityDigest, 'programme capture project authority digest',
    ),
    runId: parseTaskOpaqueId(input.runId, 'programme capture run-claim runId'),
  });
}

export function parseProgrammeCaptureDigestV1(value: unknown, label: string): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || /^0+$/.test(value)) {
    throw new TypeError(`${label} must be a non-zero lowercase SHA-256 digest`);
  }
  return value;
}
