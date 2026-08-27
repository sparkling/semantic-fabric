// SPDX-License-Identifier: MIT

import type { VerifierEvidence } from './candidate-types.js';
import type { AgenticQeEvidence } from './evidence.js';
import {
  receiptGeneratedOutputKey,
  receiptQeKey,
  receiptVerifierKey,
} from './programme-receipt-keys.js';
import { digestValue } from './receipts.js';

export function recordVerifierReceiptDigests(input: Readonly<{
  target: Record<string, string>;
  attempt: number;
  evidence: VerifierEvidence;
}>): void {
  const entries = [
    [receiptVerifierKey(input.attempt, input.evidence.stage), input.evidence.digest],
    ...Object.entries(input.evidence.generatedOutputDigests ?? {}).map(([name, digest]) => [
      receiptGeneratedOutputKey(input.attempt, input.evidence.stage, name), digest,
    ] as const),
  ] as const;
  if (entries.some(([key]) => key in input.target)) {
    throw new Error('HARNESS_VERIFIER_DIGEST_COLLISION');
  }
  Object.assign(input.target, Object.fromEntries(entries));
}

export function recordQeReceiptDigests(input: Readonly<{
  target: Record<string, string>;
  attempt: number;
  evidence: readonly AgenticQeEvidence[];
}>): readonly string[] {
  const entries = input.evidence.map((qe) => [
    receiptQeKey(input.attempt, qe.profile), digestValue(qe),
  ] as const);
  const keys = entries.map(([key]) => key);
  if (new Set(keys).size !== keys.length || keys.some((key) => key in input.target)) {
    throw new Error('HARNESS_QE_DIGEST_COLLISION');
  }
  Object.assign(input.target, Object.fromEntries(entries));
  return Object.freeze(entries.map(([, digest]) => digest));
}
