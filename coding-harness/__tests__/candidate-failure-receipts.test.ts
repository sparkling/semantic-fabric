// SPDX-License-Identifier: MIT

import { describe, expect, it, vi } from 'vitest';
import { CandidateTransaction } from '../src/candidate.js';
import {
  createIssue8ProgrammeEnvelope,
  finalizeIssue8ProgrammeOutcome,
  ISSUE_8_SAFE_TRANSACTION_REASON_CODES,
} from '../src/issue-8-programme-envelope.js';
import { RECEIPT_FAILURE_CODES } from '../src/failure-code.js';
import { NativeCancellationError } from '../src/models/recovery.js';
import {
  context, diagnosticBlob, operations,
} from './candidate-fixtures.js';

describe('candidate failure receipts', () => {
  it('admits every current receipt failure code at the live programme boundary', () => {
    expect(ISSUE_8_SAFE_TRANSACTION_REASON_CODES).toEqual(RECEIPT_FAILURE_CODES);
  });

  it('persists and cross-checks only the sanitized primary transaction failure code', async () => {
    for (const [failure, expected] of [
      [new Error('HARNESS_NATIVE_STRUCTURED_OUTPUT_MISSING:private output'),
        'HARNESS_NATIVE_STRUCTURED_OUTPUT_MISSING'],
      [new Error('HARNESS_MODEL_SECRET:must-not-escape'), 'HARNESS_TRANSACTION_FAILED'],
      [new Error(`HARNESS_NATIVE_HOST_FAILED:${'x'.repeat(4_096)}`),
        'HARNESS_TRANSACTION_FAILED'],
    ] as const) {
      const target = operations([]);
      target.implement = vi.fn(async () => { throw failure; });
      const result = await new CandidateTransaction({
        context, operations: target, maxRepairs: 0,
        now: () => '2026-08-25T12:02:00.000Z',
      }).execute();
      const envelope = createIssue8ProgrammeEnvelope(result.receipt, diagnosticBlob);

      expect(result.receipt.failureCode).toBe(expected);
      expect(finalizeIssue8ProgrammeOutcome({
        transactionStatus: result.status, transactionReason: result.reason, envelope,
      })).toEqual({ status: 'fail', reason: expected });
      expect(() => finalizeIssue8ProgrammeOutcome({
        transactionStatus: result.status,
        transactionReason: 'HARNESS_CLEANUP_FAILED:conflicting code',
        envelope,
      })).toThrow('HARNESS_ISSUE_8_FAILURE_CODE_MISMATCH');
    }
  });

  it('persists the primary verifier infrastructure stage without leaking its cause', async () => {
    for (const [failedStage, expected] of [
      ['public', 'HARNESS_VERIFIER_PUBLIC_INFRASTRUCTURE_FAILED'],
      ['independent', 'HARNESS_VERIFIER_INDEPENDENT_INFRASTRUCTURE_FAILED'],
      ['regression', 'HARNESS_VERIFIER_REGRESSION_INFRASTRUCTURE_FAILED'],
    ] as const) {
      const target = operations([]);
      target.verify = vi.fn(async (stage, _build, signal) => {
        if (stage === failedStage) {
          const cause = new Error('private verifier setup detail');
          cause.name = 'AbortError';
          throw cause;
        }
        await new Promise<void>((resolve) => {
          if (signal?.aborted === true) resolve();
          else signal?.addEventListener('abort', () => resolve(), { once: true });
        });
        throw new Error('private sibling cleanup detail');
      });
      const result = await new CandidateTransaction({
        context, operations: target, maxRepairs: 0,
        now: () => '2026-08-25T12:02:00.000Z',
      }).execute();

      expect(result.status).toBe('fail');
      expect(result.reason).toBe(expected);
      expect(result.receipt.failureCode).toBe(expected);
      expect(JSON.stringify(result.receipt)).not.toContain('private verifier setup detail');
      expect(JSON.stringify(result.receipt)).not.toContain('private sibling cleanup detail');
      expect(target.verify).toHaveBeenCalledTimes(3);
      expect(target.repair).not.toHaveBeenCalled();
    }
  });

  it('persists an opaque native final-review failure as a safe stage code', async () => {
    const target = operations([]);
    target.review = vi.fn(async (host, _build, signal) => {
      if (host === 'codex') throw new Error('private provider parser detail');
      await new Promise<void>((resolve) => {
        if (signal?.aborted === true) resolve();
        else signal?.addEventListener('abort', () => resolve(), { once: true });
      });
      throw new Error('private sibling cancellation detail');
    });
    const result = await new CandidateTransaction({
      context, operations: target, maxRepairs: 1,
      now: () => '2026-08-25T12:02:00.000Z',
    }).execute();

    expect(result.status).toBe('fail');
    expect(result.reason).toBe('HARNESS_NATIVE_REVIEW_FAILED');
    expect(result.receipt.failureCode).toBe('HARNESS_NATIVE_REVIEW_FAILED');
    expect(result.receipt.reviewDigests).toEqual([]);
    const envelope = createIssue8ProgrammeEnvelope(result.receipt, diagnosticBlob);
    expect(finalizeIssue8ProgrammeOutcome({
      transactionStatus: result.status, transactionReason: result.reason, envelope,
    })).toEqual({ status: 'fail', reason: 'HARNESS_NATIVE_REVIEW_FAILED' });
    expect(JSON.stringify(result.receipt)).not.toContain('private provider parser detail');
    expect(JSON.stringify(result.receipt)).not.toContain('private sibling cancellation detail');
  });

  it('preserves an explicit native cancellation at the final-review boundary', async () => {
    const target = operations([]);
    target.review = vi.fn(async () => { throw new NativeCancellationError(); });
    const result = await new CandidateTransaction({
      context, operations: target, maxRepairs: 1,
      now: () => '2026-08-25T12:02:00.000Z',
    }).execute();

    expect(result.status).toBe('cancelled');
    expect(result.reason).toBe('HARNESS_NATIVE_INVOCATION_CANCELLED');
    expect(result.receipt.failureCode).toBe('HARNESS_NATIVE_INVOCATION_CANCELLED');
  });
});
