// SPDX-License-Identifier: MIT

export const RECEIPT_FAILURE_CODES = Object.freeze([
  'HARNESS_ACCEPTANCE_GATE_FAILED',
  'HARNESS_CLEANUP_FAILED',
  'HARNESS_NATIVE_ARCHITECTURE_RESPONSE_INVALID',
  'HARNESS_NATIVE_CIRCUIT_OPEN',
  'HARNESS_NATIVE_HOST_FAILED',
  'HARNESS_NATIVE_HOST_TIMEOUT',
  'HARNESS_NATIVE_INVOCATION_CANCELLED',
  'HARNESS_NATIVE_ORIGIN_POLICY_DENIED',
  'HARNESS_NATIVE_ORIGIN_UNUSED',
  'HARNESS_NATIVE_PATCH_INVALID',
  'HARNESS_NATIVE_PATCH_RESPONSE_INVALID',
  'HARNESS_NATIVE_RETRY_BUDGET_EXHAUSTED',
  'HARNESS_NATIVE_REVIEW_RESPONSE_INVALID',
  'HARNESS_NATIVE_STRUCTURED_ENVELOPE_INVALID',
  'HARNESS_NATIVE_STRUCTURED_OUTPUT_INVALID',
  'HARNESS_NATIVE_STRUCTURED_OUTPUT_MISSING',
  'HARNESS_PATCH_ADMISSION_INVALID',
  'HARNESS_PATCH_APPLICATION_FAILED',
  'HARNESS_PATCH_EMPTY',
  'HARNESS_PATCH_INVALID',
  'HARNESS_PATCH_PATH_NOT_DECLARED',
  'HARNESS_PATCH_TOO_LARGE',
  'HARNESS_REPAIR_BUDGET_EXHAUSTED',
  'HARNESS_RUNTIME_EVIDENCE_FAILED',
  'HARNESS_TRANSACTION_FAILED',
  'HARNESS_VERIFIER_INDEPENDENT_INFRASTRUCTURE_FAILED',
  'HARNESS_VERIFIER_PUBLIC_INFRASTRUCTURE_FAILED',
  'HARNESS_VERIFIER_REGRESSION_INFRASTRUCTURE_FAILED',
] as const);

export type ReceiptFailureCode = typeof RECEIPT_FAILURE_CODES[number];
type ReceiptStatus = 'pass' | 'fail' | 'gated' | 'cancelled';

const CODE_SET = new Set<string>(RECEIPT_FAILURE_CODES);
const REPAIRABLE_PATCH_FAILURE_CODE_SET = new Set<ReceiptFailureCode>([
  'HARNESS_PATCH_ADMISSION_INVALID',
  'HARNESS_PATCH_EMPTY',
  'HARNESS_PATCH_INVALID',
  'HARNESS_PATCH_PATH_NOT_DECLARED',
  'HARNESS_PATCH_TOO_LARGE',
]);
const GENERIC: ReceiptFailureCode = 'HARNESS_TRANSACTION_FAILED';
const MAX_REASON_BYTES = 4_096;
const MAX_ERROR_NODES = 64;

export function failureCodeForError(error: unknown): ReceiptFailureCode {
  const seen = new Set<Error>();
  let visited = 0;
  const find = (value: unknown): ReceiptFailureCode | null => {
    if (!(value instanceof Error) || seen.has(value) || visited++ >= MAX_ERROR_NODES) return null;
    seen.add(value);
    const nested = value instanceof AggregateError ? [...value.errors, value.cause] : [value.cause];
    if (!(value instanceof AggregateError)) {
      const own = codeFromReason(safeMessage(value));
      if (own !== null) return own;
    }
    for (const entry of nested) {
      const code = find(entry);
      if (code !== null) return code;
    }
    return value instanceof AggregateError ? codeFromReason(safeMessage(value)) : null;
  };
  try { return find(error) ?? GENERIC; } catch { return GENERIC; }
}

export function failureCodeForReason(reason: string | null): ReceiptFailureCode {
  return codeFromReason(reason) ?? GENERIC;
}

export function isRepairablePatchFailure(value: string): value is ReceiptFailureCode {
  return REPAIRABLE_PATCH_FAILURE_CODE_SET.has(value as ReceiptFailureCode);
}

export function repairablePatchFailureForError(error: unknown): ReceiptFailureCode | null {
  if (!(error instanceof Error) || error instanceof AggregateError || Object.hasOwn(error, 'cause')) {
    return null;
  }
  const code = codeFromReason(safeMessage(error));
  return code !== null && isRepairablePatchFailure(code) ? code : null;
}

export function gitApplyFailureCode(
  args: readonly string[], exitCode: number | null,
): ReceiptFailureCode | null {
  if (args[0] !== 'apply') throw new TypeError('git apply arguments required');
  if (exitCode === null || exitCode <= 0) return null;
  return args.includes('--check') || args.includes('--numstat')
    ? 'HARNESS_PATCH_ADMISSION_INVALID' : 'HARNESS_PATCH_APPLICATION_FAILED';
}

export function parseReceiptFailureCode(
  value: unknown,
  status: ReceiptStatus,
): ReceiptFailureCode | null {
  if (status === 'pass') {
    if (value !== null) throw new TypeError('passing receipt cannot have a failureCode');
    return null;
  }
  if (value === null) throw new TypeError('receipt.failureCode must identify a failed transaction');
  if (typeof value !== 'string' || !CODE_SET.has(value)) {
    throw new TypeError('receipt.failureCode is invalid');
  }
  return value as ReceiptFailureCode;
}

function codeFromReason(reason: string | null): ReceiptFailureCode | null {
  if (reason === null || Buffer.byteLength(reason, 'utf8') > MAX_REASON_BYTES) return null;
  const code = /^(HARNESS_[A-Z0-9_]+)(?=[^A-Z0-9_]|$)/.exec(reason)?.[1];
  return code !== undefined && CODE_SET.has(code) ? code as ReceiptFailureCode : null;
}

function safeMessage(error: Error): string | null {
  try { return typeof error.message === 'string' ? error.message : null; } catch { return null; }
}
