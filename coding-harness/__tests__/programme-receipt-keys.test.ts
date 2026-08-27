// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  recordQeReceiptDigests,
  recordVerifierReceiptDigests,
} from '../src/candidate-receipt-evidence.js';
import { digestValue } from '../src/receipts.js';
import {
  RED_BASELINE_RECEIPT_KEY,
  receiptArtifactKey,
  receiptGeneratedOutputKey,
  receiptMutationEvidenceKey,
  receiptMutationKey,
  receiptQeKey,
  receiptVerifierKey,
} from '../src/programme-receipt-keys.js';

describe('programme receipt keys', () => {
  it('owns every canonical receipt-key formula', () => {
    expect(RED_BASELINE_RECEIPT_KEY).toBe('red-baseline');
    expect(receiptArtifactKey(2, 'target/release/app')).toBe('attempt-2:target/release/app');
    expect(receiptVerifierKey(2, 'independent')).toBe('attempt-2:independent');
    expect(receiptGeneratedOutputKey(2, 'public', 'workspace-tests-earl'))
      .toBe('attempt-2:public:generated:workspace-tests-earl');
    expect(receiptGeneratedOutputKey(2, 'public', 'a--'))
      .toBe('attempt-2:public:generated:a--');
    expect(receiptGeneratedOutputKey(2, 'public', 'a'.repeat(64)))
      .toBe(`attempt-2:public:generated:${'a'.repeat(64)}`);
    expect(receiptMutationKey(2, 'ordinary-subject-bind-prune'))
      .toBe('attempt-2:mutation:ordinary-subject-bind-prune');
    expect(receiptMutationEvidenceKey(2, 'mutation:ordinary-subject-bind-prune'))
      .toBe('attempt-2:mutation:ordinary-subject-bind-prune');
    expect(receiptMutationEvidenceKey(2, 'mutation')).toBe('attempt-2:mutation');
    expect(receiptMutationKey(2, 'Upper_ID')).toBe('attempt-2:mutation:Upper_ID');
    expect(receiptMutationKey(2, 'A'.repeat(128))).toBe(`attempt-2:mutation:${'A'.repeat(128)}`);
    expect(receiptQeKey(2, 'lcov-gap')).toBe('attempt-2:qe:lcov-gap');
  });

  it('rejects ambiguous attempts, stages, profiles, paths, and evidence names', () => {
    expect(() => receiptArtifactKey(-1, 'target/app')).toThrow(/receipt attempt/);
    expect(() => receiptArtifactKey(0, '../target/app')).toThrow(/traversal/);
    expect(() => receiptVerifierKey(0, 'mutation' as never)).toThrow(/stage is invalid/);
    expect(() => receiptGeneratedOutputKey(0, 'public', 'Invalid_ID')).toThrow(/evidenceId/);
    expect(() => receiptMutationKey(0, 'mutation:one')).toThrow(/mutationId/);
    expect(() => receiptMutationEvidenceKey(0, 'one')).toThrow(/evidence name/);
    expect(() => receiptQeKey(0, 'unknown' as never)).toThrow(/profile is invalid/);
  });

  it('derives ordered QE digests without allowing profile drift or collisions', () => {
    const target: Record<string, string> = {};
    const evidence = [
      { profile: 'lcov-gap' }, { profile: 'sast' },
    ] as never;
    const digests = recordQeReceiptDigests({ target, attempt: 1, evidence });
    expect(digests).toEqual(evidence.map(digestValue));
    expect(Object.values(target)).toEqual(digests);
    expect(Object.isFrozen(digests)).toBe(true);
    expect(() => recordQeReceiptDigests({ target, attempt: 1, evidence }))
      .toThrow('HARNESS_QE_DIGEST_COLLISION');
    const duplicateTarget: Record<string, string> = {};
    expect(() => recordQeReceiptDigests({
      target: duplicateTarget, attempt: 1, evidence: [evidence[0], evidence[0]],
    })).toThrow('HARNESS_QE_DIGEST_COLLISION');
    expect(duplicateTarget).toEqual({});
  });

  it('records a verifier and its generated outputs without overwrites', () => {
    const target: Record<string, string> = {};
    const evidence = {
      stage: 'public', digest: 'a', generatedOutputDigests: { 'workspace-tests-earl': 'b' },
    } as never;
    recordVerifierReceiptDigests({ target, attempt: 3, evidence });
    expect(target).toEqual({
      'attempt-3:public': 'a',
      'attempt-3:public:generated:workspace-tests-earl': 'b',
    });
    expect(() => recordVerifierReceiptDigests({ target, attempt: 3, evidence }))
      .toThrow('HARNESS_VERIFIER_DIGEST_COLLISION');
    const generatedCollision = {
      'attempt-3:public:generated:workspace-tests-earl': 'existing',
    };
    expect(() => recordVerifierReceiptDigests({
      target: generatedCollision, attempt: 3, evidence,
    })).toThrow('HARNESS_VERIFIER_DIGEST_COLLISION');
    expect(generatedCollision).toEqual({
      'attempt-3:public:generated:workspace-tests-earl': 'existing',
    });
  });
});
