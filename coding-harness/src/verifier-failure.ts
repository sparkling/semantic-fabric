// SPDX-License-Identifier: MIT

import type { VerifierStage } from './candidate-types.js';
import type { ReceiptFailureCode } from './failure-code.js';

const CODES = Object.freeze({
  public: 'HARNESS_VERIFIER_PUBLIC_INFRASTRUCTURE_FAILED',
  independent: 'HARNESS_VERIFIER_INDEPENDENT_INFRASTRUCTURE_FAILED',
  regression: 'HARNESS_VERIFIER_REGRESSION_INFRASTRUCTURE_FAILED',
} as const satisfies Readonly<Record<VerifierStage, ReceiptFailureCode>>);

export function normalizeVerifierFailure(
  stage: VerifierStage,
  cause: unknown,
  signal: AbortSignal,
): unknown {
  if (signal.aborted) return signal.reason ?? cause;
  return new Error(CODES[stage], { cause });
}
