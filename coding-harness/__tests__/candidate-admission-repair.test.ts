// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { describe, expect, it, vi } from 'vitest';
import { CandidateTransaction } from '../src/candidate.js';
import { digestValue } from '../src/receipts.js';
import { context, digest, identity, operations } from './candidate-fixtures.js';

describe('pre-admission candidate repair', () => {
  it('rebuilds one rejected patch from a clean evaluator and records both attempts', async () => {
    const target = operations([]);
    const admit = target.admitAndApply;
    let admissions = 0;
    target.admitAndApply = vi.fn(async (...args) => {
      admissions += 1;
      if (admissions === 1) throw new Error('HARNESS_PATCH_ADMISSION_INVALID:private detail');
      return await admit(...args);
    });
    target.verify = passingVerifiers();

    const result = await transaction(target, 1).execute();

    expect(result.status, result.reason ?? '').toBe('pass');
    expect(result.repairCount).toBe(1);
    expect(target.resetCandidate).toHaveBeenCalledTimes(3);
    expect(target.admitAndApply).toHaveBeenCalledTimes(2);
    expect(target.repair).toHaveBeenCalledWith(
      expect.anything(), ['HARNESS_PATCH_ADMISSION_INVALID'], 1, 'pre-admission', undefined,
    );
    expect(result.receipt.patchDigests).toHaveLength(2);
    expect(result.receipt.patchDigests.at(-1)).toBe(result.receipt.patchDigest);
    const native = vi.mocked(target.runtimeEvidence).mock.calls[0]?.[0] ?? [];
    expect(native.find(({ operation }) => operation === 'repair')?.candidateTree)
      .toBe(context.identities.evaluator.tree);
    expect(result.repairTransitions).toHaveLength(1);
    const [transition] = result.repairTransitions;
    expect(transition).toMatchObject({
      schemaVersion: 1,
      fromAttempt: 0,
      toAttempt: 1,
      phase: 'pre-admission',
      trigger: 'patch-admission',
      buildDisposition: 'not-started',
      sourcePatchDigest: result.receipt.patchDigests[0],
      replacementPatchDigest: result.receipt.patchDigests[1],
      sourceCandidate: context.identities.evaluator,
      repairResetIdentity: context.identities.evaluator,
      resetIdentity: context.identities.evaluator,
      reasonDigests: [digestValue('HARNESS_PATCH_ADMISSION_INVALID')],
      nativeInvocation: {
        invocationId: 'repair-0001',
        operation: 'repair',
        candidateTree: context.identities.evaluator.tree,
      },
    });
    const { digest: transitionDigest, ...transitionBody } = transition;
    expect(transitionDigest).toBe(digestValue(transitionBody));
  });

  it('clears stale admission evidence before the repair budget is exhausted', async () => {
    const target = operations([]);
    let admissions = 0;
    target.admitAndApply = vi.fn(async (patch) => {
      admissions += 1;
      if (admissions === 2) throw new Error('HARNESS_PATCH_ADMISSION_INVALID');
      return {
        candidate: identity('3'),
        patchDigest: createHash('sha256').update(patch.payload).digest('hex'),
        admittedPaths: ['crates/sf-sparql/src/unfold.rs'],
      };
    });
    let validations = 0;
    target.validateAdmission = vi.fn(async () => {
      validations += 1;
      return validations === 1 ? ['HARNESS_CANDIDATE_SOURCE_FIX_MISMATCH'] : [];
    });

    const result = await transaction(target, 1).execute();

    expect(result.status).toBe('fail');
    expect(result.receipt.failureCode).toBe('HARNESS_REPAIR_BUDGET_EXHAUSTED');
    expect(result.repairCount).toBe(1);
    expect(target.repair).toHaveBeenCalledTimes(1);
    expect(target.resetCandidate).toHaveBeenCalledTimes(3);
    expect(result.receipt.identities.candidate).toEqual(context.identities.evaluator);
    expect(result.receipt.admittedPaths).toEqual([]);
    expect(result.receipt.patchDigest).toBeNull();
    expect(result.receipt.patchDigests).toHaveLength(2);
    expect(result.repairTransitions).toEqual([]);
  });

  it('does not model-repair a terminal patch application or wrapped admission failure', async () => {
    for (const failure of [
      new Error('HARNESS_PATCH_APPLICATION_FAILED'),
      new Error('HARNESS_WORKTREE_RESET_FAILED', {
        cause: new Error('HARNESS_PATCH_ADMISSION_INVALID'),
      }),
      new Error('HARNESS_PATCH_ADMISSION_INVALID', {
        cause: new Error('HARNESS_WORKTREE_RESET_FAILED'),
      }),
      new AggregateError([
        new Error('HARNESS_PATCH_ADMISSION_INVALID'),
        new Error('HARNESS_WORKTREE_RESET_FAILED'),
      ], 'HARNESS_PATCH_AND_ROLLBACK_FAILED'),
    ]) {
      const target = operations([]);
      target.admitAndApply = vi.fn(async () => { throw failure; });

      const result = await transaction(target, 1).execute();

      expect(result.status).toBe('fail');
      expect(target.repair).not.toHaveBeenCalled();
      expect(target.resetCandidate).toHaveBeenCalledTimes(1);
      expect(result.receipt.admittedPaths).toEqual([]);
      expect(result.receipt.patchDigest).toBeNull();
    }
  });

  it('requires a verified clean reset before invoking pre-admission repair', async () => {
    const target = operations([]);
    const reset = target.resetCandidate;
    let resets = 0;
    target.resetCandidate = vi.fn(async (...args) => {
      resets += 1;
      if (resets === 2) throw new Error('HARNESS_WORKTREE_RESET_FAILED');
      return await reset(...args);
    });
    target.admitAndApply = vi.fn(async () => {
      throw new Error('HARNESS_PATCH_ADMISSION_INVALID');
    });

    const result = await transaction(target, 1).execute();

    expect(result.status).toBe('fail');
    expect(result.reason).toContain('HARNESS_WORKTREE_RESET_FAILED');
    expect(result.repairCount).toBe(0);
    expect(target.repair).not.toHaveBeenCalled();
    expect(result.receipt.admittedPaths).toEqual([]);
    expect(result.receipt.patchDigest).toBeNull();
    expect(result.repairTransitions).toEqual([]);
  });

  it('rejects a reset that does not restore the frozen evaluator identity', async () => {
    const target = operations([]);
    target.resetCandidate = vi.fn(async () => identity('9'));

    const result = await transaction(target, 1).execute();

    expect(result.status).toBe('fail');
    expect(result.reason).toContain('HARNESS_CANDIDATE_RESET_IDENTITY_MISMATCH');
    expect(target.admitAndApply).not.toHaveBeenCalled();
    expect(target.repair).not.toHaveBeenCalled();
    expect(result.repairTransitions).toEqual([]);
  });

  it('rejects an admission whose patch digest is not the submitted patch', async () => {
    const target = operations([]);
    const admit = target.admitAndApply;
    target.admitAndApply = vi.fn(async (...args) => ({
      ...await admit(...args),
      patchDigest: digest('f'),
    }));

    const result = await transaction(target, 1).execute();

    expect(result.status).toBe('fail');
    expect(result.reason).toContain('HARNESS_PATCH_ADMISSION_DIGEST_MISMATCH');
    expect(target.validateAdmission).not.toHaveBeenCalled();
    expect(target.build).not.toHaveBeenCalled();
    expect(target.repair).not.toHaveBeenCalled();
    expect(result.receipt.identities.candidate).toEqual(context.identities.evaluator);
    expect(result.receipt.admittedPaths).toEqual([]);
    expect(result.receipt.patchDigest).toBeNull();
    expect(result.repairTransitions).toEqual([]);
  });

  it('commits repair count and transition evidence only after a valid repair returns', async () => {
    for (const [repair, expectedInvocations] of [
      [vi.fn(async () => { throw new Error('repair failed'); }), 0],
      [vi.fn(async () => ({ payload: 'patch-one', authorInvocationId: 'repair-0001' })), 1],
    ] as const) {
      const target = operations([]);
      target.admitAndApply = vi.fn(async () => {
        throw new Error('HARNESS_PATCH_ADMISSION_INVALID');
      });
      target.repair = repair;

      const result = await transaction(target, 1).execute();

      expect(result.status).toBe('fail');
      expect(result.repairCount).toBe(0);
      expect(result.receipt.recovery.repairCount).toBe(0);
      expect(result.repairTransitions).toEqual([]);
      const expectations = vi.mocked(target.runtimeEvidence).mock.calls[0]?.[0] ?? [];
      expect(expectations.filter(({ operation }) => operation === 'repair'))
        .toHaveLength(expectedInvocations);
      expect(result.reason).not.toContain('HARNESS_RUNTIME_EVIDENCE_FAILED');
    }
  });

  it('records one rejected submission without calling a zero-budget repair', async () => {
    const target = operations([]);
    target.admitAndApply = vi.fn(async () => {
      throw new Error('HARNESS_PATCH_ADMISSION_INVALID');
    });

    const result = await transaction(target, 0).execute();

    expect(result.status).toBe('fail');
    expect(result.receipt.failureCode).toBe('HARNESS_REPAIR_BUDGET_EXHAUSTED');
    expect(result.repairCount).toBe(0);
    expect(target.repair).not.toHaveBeenCalled();
    expect(target.resetCandidate).toHaveBeenCalledTimes(2);
    expect(result.receipt.patchDigests).toHaveLength(1);
    expect(result.receipt.patchDigest).toBeNull();
  });

  it('does not consume repair budget when cancellation lands before regeneration', async () => {
    const controller = new AbortController();
    const target = operations([]);
    target.admitAndApply = vi.fn(async () => {
      controller.abort('cancel before repair');
      throw new Error('HARNESS_PATCH_ADMISSION_INVALID');
    });

    const result = await new CandidateTransaction({
      context, operations: target, maxRepairs: 0, signal: controller.signal,
    }).execute();

    expect(result.status).toBe('cancelled');
    expect(result.repairCount).toBe(0);
    expect(target.repair).not.toHaveBeenCalled();
    expect(result.receipt.recovery.cancelled).toBe(true);
  });
});

function transaction(target: ReturnType<typeof operations>, maxRepairs: number): CandidateTransaction {
  return new CandidateTransaction({
    context, operations: target, maxRepairs,
    now: () => '2026-08-25T12:02:00.000Z',
  });
}

function passingVerifiers() {
  return vi.fn(async (stage: 'public' | 'independent' | 'regression', build: {
    candidate: { commit: string; tree: string };
  }) => ({
    stage,
    candidate: build.candidate,
    passed: true,
    digest: digest(stage === 'public' ? '5' : stage === 'independent' ? '6' : '7'),
    reasons: [],
  }));
}
