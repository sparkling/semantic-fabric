// SPDX-License-Identifier: MIT

import { describe, expect, it, vi } from 'vitest';
import { CandidateTransaction } from '../src/candidate.js';
import { parseCandidateTransactionEvidenceV1 } from '../src/candidate-transaction-evidence-v1.js';
import { digestValue } from '../src/receipts.js';
import { context, digest, operations } from './candidate-fixtures.js';

describe('candidate transaction evidence V1', () => {
  it('round-trips full native and repair evidence against the pass receipt', async () => {
    const result = await repairedTransaction();
    expect(result.status, result.reason ?? '').toBe('pass');
    expect(result.transactionEvidence).not.toBeNull();

    const parsed = parseCandidateTransactionEvidenceV1(
      structuredClone(result.transactionEvidence),
      result.receipt,
    );
    expect(parsed).toEqual(result.transactionEvidence);
    expect(parsed.receiptBinding).toMatchObject({
      receiptDigest: result.receipt.digest,
      repairCount: 1,
      patchHistoryDigest: digestValue(result.receipt.patchDigests),
      nativeRuntimeEvidenceDigest: result.receipt.coordination.nativeRuntimeEvidenceDigest,
    });
    expect(parsed.nativeRuntimeEvidence.schemaVersion).toBe(2);
    expect(parsed.repairTransitions).toHaveLength(1);
    expect(Object.isFrozen(parsed)).toBe(true);
  });

  it('supports an unrepaired pass without inventing transition evidence', async () => {
    const target = operations([]);
    target.verify = vi.fn(async (stage, build) => ({
      stage, candidate: build.candidate, passed: true,
      digest: digest(stage === 'public' ? '5' : stage === 'independent' ? '6' : '7'),
      reasons: [],
    }));
    const result = await new CandidateTransaction({
      context, operations: target, maxRepairs: 1,
      now: () => '2026-08-25T12:02:00.000Z',
    }).execute();

    expect(result.status, result.reason ?? '').toBe('pass');
    expect(result.transactionEvidence?.repairTransitions).toEqual([]);
    expect(result.transactionEvidence?.receiptBinding.repairCount).toBe(0);
  });

  it('rejects downgrade, loose keys, receipt substitution, and rehashed transition claims', async () => {
    const result = await repairedTransaction();
    const original = result.transactionEvidence as NonNullable<typeof result.transactionEvidence>;
    const attacks: Array<(value: any) => void> = [
      (value) => { value.schemaVersion = 0; },
      (value) => { value.extra = true; },
      (value) => { value.receiptBinding.receiptDigest = digest('f'); rehash(value); },
      (value) => {
        value.repairTransitions[0].buildDisposition = 'failed';
        value.repairTransitions[0].digest = digestValue(transitionBody(value.repairTransitions[0]));
        rehash(value);
      },
      (value) => {
        value.nativeInvocationBindings.find((entry: any) => entry.operation === 'implementation')
          .patchPayloadSha256 = digest('f');
        rehash(value);
      },
      (value) => {
        const reviews = value.nativeInvocationBindings.filter((entry: any) =>
          entry.operation === 'review');
        const firstReview = value.nativeInvocationBindings.indexOf(reviews[0]);
        value.nativeInvocationBindings.splice(firstReview, 1);
        value.nativeInvocationBindings.unshift(reviews[0]);
        rehash(value);
      },
    ];
    for (const attack of attacks) {
      const tampered = structuredClone(original) as any;
      attack(tampered);
      expect(() => parseCandidateTransactionEvidenceV1(tampered, result.receipt)).toThrow();
    }
  });

  it('never emits authoritative sidecar evidence for a failed transaction', async () => {
    const target = operations([]);
    target.prepare = vi.fn(async () => { throw new Error('prepare failed'); });
    const result = await new CandidateTransaction({
      context, operations: target, maxRepairs: 1,
    }).execute();
    expect(result.status).toBe('fail');
    expect(result.transactionEvidence).toBeNull();
  });
});

async function repairedTransaction() {
  return await new CandidateTransaction({
    context, operations: operations([]), maxRepairs: 1,
    now: () => '2026-08-25T12:02:00.000Z',
  }).execute();
}

function rehash(value: any): void {
  value.repairTransitionsDigest = digestValue(value.repairTransitions);
  const { evidenceDigest: _digest, ...body } = value;
  value.evidenceDigest = digestValue(body);
}

function transitionBody(value: any): unknown {
  const { digest: _digest, ...body } = value;
  return body;
}
