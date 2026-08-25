// SPDX-License-Identifier: MIT
import { createHash } from 'node:crypto';
import { describe, expect, it, vi } from 'vitest';
import {
  CandidateBuildFailure,
  CandidateTransaction,
} from '../src/candidate.js';
import {
  createIssue8ProgrammeEnvelope,
  finalizeIssue8ProgrammeOutcome,
  parseIssue8ProgrammeEnvelope,
  serializeIssue8ProgrammeEnvelope,
} from '../src/issue-8-programme-envelope.js';
import { NativeCancellationError } from '../src/models/recovery.js';
import { METAHARNESS_DIAGNOSTICS_PATH } from '../src/metaharness-diagnostics.js';
import { ReceiptChain, digestValue } from '../src/receipts.js';
import {
  commandEvidence,
  context,
  diagnosticBlob,
  diagnosticSnapshot,
  digest,
  identity,
  operations,
} from './candidate-fixtures.js';
describe('patched candidate transaction', () => {
  it('re-admits, rebuilds, and reruns all parallel verifiers after repair', async () => {
    const events: string[] = [];
    const transaction = new CandidateTransaction({
      context,
      operations: operations(events),
      maxRepairs: 1,
      now: () => '2026-08-25T12:02:00.000Z',
    });

    const result = await transaction.execute();

    expect(result.status).toBe('pass');
    expect(result.repairCount).toBe(1);
    expect(result.finalPatch).toBe('patch-two');
    expect(events).toEqual([
      'prepare', 'architecture', 'implement',
      'reset', 'admit', 'validate', 'build',
      'verify:public', 'verify:independent', 'verify:regression', 'repair',
      'reset', 'admit', 'validate', 'build',
      'verify:public', 'verify:independent', 'verify:regression',
      'review:codex', 'review:claude-code', 'qe', 'protected', 'audit', 'validate', 'cleanup',
    ]);
    expect(result.receipt.status).toBe('pass');
    expect(result.receipt.failureCode).toBeNull();
    expect(result.receipt.recovery.repairCount).toBe(1);
    expect(transaction.receipts.verify()).toEqual({ ok: true });
    const envelope = createIssue8ProgrammeEnvelope(result.receipt, diagnosticBlob);
    expect(envelope.programmeAcceptance.status).toBe('ACCEPTED');
    expect(envelope.programmeAcceptance.score).toBe(100);
    expect(parseIssue8ProgrammeEnvelope(serializeIssue8ProgrammeEnvelope(envelope)))
      .toEqual(envelope);
    expect(finalizeIssue8ProgrammeOutcome({
      transactionStatus: result.status, transactionReason: result.reason, envelope,
    })).toEqual({ status: 'pass', reason: null });
    const tampered = JSON.parse(JSON.stringify(envelope));
    tampered.programmeAcceptance.score = 99;
    expect(() => parseIssue8ProgrammeEnvelope(JSON.stringify(tampered)))
      .toThrow('HARNESS_ISSUE_8_PROGRAMME_ENVELOPE_DIGEST_INVALID');
    const { digest: _oldDigest, ...diagnosticBody } = JSON.parse(JSON.stringify(diagnosticSnapshot));
    diagnosticBody.targets[0].degraded = true;
    const degradedDiagnostics = { ...diagnosticBody, digest: digestValue(diagnosticBody) };
    const degradedBlob = `${JSON.stringify(degradedDiagnostics, null, 2)}\n`;
    const degradedBlobDigest = createHash('sha256').update(degradedBlob).digest('hex');
    const {
      sequence: _sequence, previousDigest: _previousDigest, digest: _receiptDigest, ...draft
    } = result.receipt;
    const degradedReceipt = new ReceiptChain().append({
      ...draft,
      protectedInputs: {
        ...draft.protectedInputs,
        [METAHARNESS_DIAGNOSTICS_PATH]: degradedBlobDigest,
      },
    });
    const gated = createIssue8ProgrammeEnvelope(degradedReceipt, degradedBlob);
    expect(finalizeIssue8ProgrammeOutcome({
      transactionStatus: result.status, transactionReason: result.reason, envelope: gated,
    })).toEqual({
      status: 'gated', reason: 'HARNESS_ISSUE_8_PROGRAMME_ACCEPTANCE_REJECTED',
    });
    expect(() => createIssue8ProgrammeEnvelope(result.receipt, `${diagnosticBlob} `))
      .toThrow('HARNESS_ISSUE_8_PROGRAMME_DIAGNOSTIC_BLOB_MISMATCH');
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

  it('routes an exact-source admission mismatch through the bounded repair loop', async () => {
    const target = operations([]);
    let validations = 0;
    target.validateAdmission = vi.fn(async () => {
      validations += 1;
      return validations === 1 ? ['HARNESS_CANDIDATE_SOURCE_FIX_MISMATCH'] : [];
    });
    target.verify = vi.fn(async (stage, build) => ({
      stage,
      candidate: build.candidate,
      passed: true,
      digest: digest(stage === 'public' ? '5' : stage === 'independent' ? '6' : '7'),
      reasons: [],
    }));

    const result = await new CandidateTransaction({
      context, operations: target, maxRepairs: 1,
      now: () => '2026-08-25T12:02:00.000Z',
    }).execute();

    expect(result.status, result.reason ?? '').toBe('pass');
    expect(result.repairCount).toBe(1);
    expect(target.build).toHaveBeenCalledTimes(1);
    expect(target.repair).toHaveBeenCalledWith(
      expect.anything(),
      ['HARNESS_CANDIDATE_SOURCE_FIX_MISMATCH'],
      1,
      undefined,
    );
  });

  it('prefixes mutation evidence when a final admission check triggers repair', async () => {
    const target = operations([]);
    let validations = 0;
    target.validateAdmission = vi.fn(async () => {
      validations += 1;
      return validations === 2 ? ['HARNESS_FINAL_ADMISSION_CHANGED'] : [];
    });
    target.verify = vi.fn(async (stage, build) => ({
      stage,
      candidate: build.candidate,
      passed: true,
      digest: digest(stage === 'public' ? '5' : stage === 'independent' ? '6' : '7'),
      reasons: [],
    }));

    const result = await new CandidateTransaction({
      context, operations: target, maxRepairs: 1,
      now: () => '2026-08-25T12:02:00.000Z',
    }).execute();

    expect(result.status, result.reason ?? '').toBe('pass');
    expect(result.repairCount).toBe(1);
    expect(result.receipt.verifierDigests).toHaveProperty('attempt-0:mutation');
    expect(result.receipt.verifierDigests).toHaveProperty('attempt-1:mutation');
  });

  it('requires both issue-8 QE profiles before the final audits can pass', async () => {
    const target = operations([]);
    target.verify = vi.fn(async (stage, build) => ({
      stage,
      candidate: build.candidate,
      passed: true,
      digest: digest(stage === 'public' ? '5' : stage === 'independent' ? '6' : '7'),
      reasons: [],
    }));
    target.agenticQeEvidence = vi.fn(async (build) => [{
      schemaVersion: 1,
      source: 'agentic-qe-local-profile',
      profile: 'lcov-gap',
      taskId: context.taskId,
      runId: context.runId,
      candidateTree: build.candidate.tree,
      commandDigest: digest('a'),
      outputDigest: digest('b'),
      providerVariablesStripped: true,
      authoritative: false,
      capturedAt: '2026-08-25T12:01:00.000Z',
    }]);

    const result = await new CandidateTransaction({
      context, operations: target, maxRepairs: 1,
    }).execute();

    expect(result.status).toBe('fail');
    expect(result.reason).toContain('HARNESS_REQUIRED_QE_PROFILES_MISSING:sast');
    expect(target.verifyProtectedInputs).not.toHaveBeenCalled();
  });

  it('rejects stale build identities and emits a failure receipt', async () => {
    const events: string[] = [];
    const target = operations(events);
    target.build = vi.fn(async () => ({
      candidate: identity('1'),
      commands: [commandEvidence(0, identity('1').tree)],
      artifactDigests: { binary: digest('3') },
    }));
    const result = await new CandidateTransaction({
      context,
      operations: target,
      maxRepairs: 0,
      now: () => '2026-08-25T12:02:00.000Z',
    }).execute();

    expect(result.status).toBe('fail');
    expect(result.finalPatch).toBeNull();
    expect(result.reason).toMatch(/STALE_BUILD_IDENTITY/);
    expect(result.receipt.status).toBe('fail');
    expect(result.receipt.failureCode).toBe('HARNESS_TRANSACTION_FAILED');
    expect(createIssue8ProgrammeEnvelope(
      result.receipt, diagnosticBlob,
    ).programmeAcceptance.status)
      .toBe('REJECTED');
  });

  it('records cancellation even when it occurs before a patch exists', async () => {
    const events: string[] = [];
    const target = operations(events);
    target.implement = vi.fn(async () => {
      throw new NativeCancellationError();
    });
    const result = await new CandidateTransaction({
      context,
      operations: target,
      maxRepairs: 1,
      now: () => '2026-08-25T12:02:00.000Z',
    }).execute();

    expect(result.status).toBe('cancelled');
    expect(result.finalPatch).toBeNull();
    expect(result.receipt.status).toBe('cancelled');
    expect(result.receipt.failureCode).toBe('HARNESS_NATIVE_INVOCATION_CANCELLED');
    expect(result.receipt.recovery.cancelled).toBe(true);
  });

  it('emits a cancellation receipt before worktree preparation starts', async () => {
    const events: string[] = [];
    const controller = new AbortController();
    controller.abort('cancel before prepare');
    const target = operations(events);
    const result = await new CandidateTransaction({
      context,
      operations: target,
      maxRepairs: 1,
      signal: controller.signal,
      now: () => '2026-08-25T12:02:00.000Z',
    }).execute();

    expect(result.status).toBe('cancelled');
    expect(result.receipt.identities.candidate).toEqual(context.identities.evaluator);
    expect(target.prepare).not.toHaveBeenCalled();
  });

  it('cannot pass an explicit verifier rejection with an empty reason list', async () => {
    const events: string[] = [];
    const target = operations(events);
    target.verify = vi.fn(async (stage, build) => ({
      stage,
      candidate: build.candidate,
      passed: stage !== 'independent',
      digest: digest(stage === 'public' ? '5' : stage === 'independent' ? '6' : '7'),
      reasons: [],
    }));

    const result = await new CandidateTransaction({
      context,
      operations: target,
      maxRepairs: 0,
      now: () => '2026-08-25T12:02:00.000Z',
    }).execute();

    expect(result.status).toBe('fail');
    expect(result.reason).toContain('HARNESS_VERIFIER_REJECTED_WITHOUT_REASON');
    expect(target.review).not.toHaveBeenCalled();
  });

  it('requires distinct native hosts for architecture evidence', async () => {
    const target = operations([]);
    target.architecture = vi.fn(async () => ({
      value: {},
      critiqueDigests: [digest('f')],
      invocations: [
        { invocationId: 'architecture-one', host: 'codex' },
        { invocationId: 'architecture-two', host: 'codex' },
      ],
    }));
    const result = await new CandidateTransaction({ context, operations: target, maxRepairs: 0 }).execute();
    expect(result.status).toBe('fail');
    expect(result.reason).toContain('HARNESS_NATIVE_DUAL_HOST_ARCHITECTURE_REQUIRED');
  });

  it('requires the prepared protected-input set to match controller context', async () => {
    for (const protectedInputs of [{}, { 'protected.txt': digest('b') }]) {
      const target = operations([]);
      target.prepare = vi.fn(async () => ({
        baseline: identity('1'), evaluator: identity('2'), candidate: identity('2'), protectedInputs,
      }));
      const result = await new CandidateTransaction({ context, operations: target, maxRepairs: 0 }).execute();
      expect(result.status).toBe('fail');
      expect(result.reason).toContain('HARNESS_PROTECTED_INPUT');
    }
  });

  it('rejects duplicate native host declarations', () => {
    expect(() => new CandidateTransaction({
      context: { ...context, hosts: [...context.hosts, context.hosts[0]] },
      operations: operations([]), maxRepairs: 0,
    })).toThrow('HARNESS_TRANSACTION_DUAL_HOST_EVIDENCE_REQUIRED');
  });

  it('cannot pass an explicit review rejection with an empty reason list', async () => {
    const events: string[] = [];
    const target = operations(events);
    target.verify = vi.fn(async (stage, build) => ({
      stage,
      candidate: build.candidate,
      passed: true,
      digest: digest(stage === 'public' ? '5' : stage === 'independent' ? '6' : '7'),
      reasons: [],
    }));
    target.review = vi.fn(async (host, build) => ({
      host,
      invocationId: `review-${host}`,
      candidate: build.candidate,
      accepted: host !== 'codex',
      digest: digest(host === 'codex' ? '8' : '9'),
      reasons: [],
    }));

    const result = await new CandidateTransaction({
      context,
      operations: target,
      maxRepairs: 0,
      now: () => '2026-08-25T12:02:00.000Z',
    }).execute();

    expect(result.status).toBe('fail');
    expect(result.reason).toContain('HARNESS_REVIEW_REJECTED_WITHOUT_REASON');
    expect(target.verifyProtectedInputs).not.toHaveBeenCalled();
  });

  it('rejects empty or stale acceptance evidence', async () => {
    const missing = operations([]);
    missing.preflightEvidence = vi.fn(async () => ({
      passed: true, reasons: [], commands: [], digests: {},
    }));
    const missingResult = await new CandidateTransaction({
      context, operations: missing, maxRepairs: 1,
    }).execute();
    expect(missingResult.status).toBe('fail');
    expect(missingResult.reason).toContain('HARNESS_ACCEPTANCE_EVIDENCE_INCOMPLETE');

    const stale = operations([]);
    stale.mutationEvidence = vi.fn(async (build) => ({
      passed: true,
      reasons: [],
      commands: [{ ...commandEvidence(0, build.candidate.tree, 'mutation'), exitCode: 101 }],
      digests: { mutation: digest('c') },
    }));
    const staleResult = await new CandidateTransaction({
      context, operations: stale, maxRepairs: 1,
    }).execute();
    expect(staleResult.status).toBe('fail');
    expect(staleResult.reason).toContain('HARNESS_ACCEPTANCE_COMMAND_BINDING_MISMATCH');
  });

  it('does not count recovery events as native execution evidence', async () => {
    const target = operations([]);
    target.recoveryEvidence = vi.fn(() => ({
      retryCount: 2,
      breakerState: 'closed',
      recoveryEvents: [{ outcome: 'transient-retry' }, { outcome: 'success' }],
    }));
    target.runtimeEvidence = vi.fn(() => ({ nativeEvidence: {} }));
    const result = await new CandidateTransaction({
      context, operations: target, maxRepairs: 1,
    }).execute();
    expect(result.status).toBe('fail');
    expect(result.reason).toContain('HARNESS_RUNTIME_EVIDENCE_FAILED');
    expect(result.receipt.failureCode).toBe('HARNESS_RUNTIME_EVIDENCE_FAILED');
    expect(result.receipt.recovery.retryCount).toBe(2);
  });

  it('turns cleanup failure into a terminal failed receipt', async () => {
    const target = operations([]);
    target.cleanup = vi.fn(async () => {
      throw new Error('resource lease remained active');
    });
    const result = await new CandidateTransaction({
      context, operations: target, maxRepairs: 1,
    }).execute();
    expect(result.status).toBe('fail');
    expect(result.finalPatch).toBeNull();
    expect(result.reason).toContain('HARNESS_CLEANUP_FAILED:resource lease remained active');
    expect(result.receipt.failureCode).toBe('HARNESS_CLEANUP_FAILED');
  });

  it('preserves failed build evidence and repairs from the compiler failure', async () => {
    const events: string[] = [];
    const target = operations(events);
    const successfulBuild = target.build;
    target.build = vi.fn(async (admission, attempt, signal) => {
      if (attempt > 0) return await successfulBuild(admission, attempt, signal);
      throw new CandidateBuildFailure({
        candidate: admission.candidate,
        commands: [{ ...commandEvidence(attempt, admission.candidate.tree), exitCode: 101 }],
        artifactDigests: { binary: digest('3') },
      }, ['cargo exited 101']);
    });

    const result = await new CandidateTransaction({
      context,
      operations: target,
      maxRepairs: 1,
      now: () => '2026-08-25T12:02:00.000Z',
    }).execute();

    expect(result.status).toBe('pass');
    expect(result.receipt.commands.filter(({ stage }) => stage === 'build')
      .map(({ exitCode }) => exitCode)).toEqual([101, 0]);
    expect(result.receipt.patchDigests).toHaveLength(2);
    expect(result.receipt.artifactDigests).toHaveProperty('attempt-0:binary');
    expect(result.receipt.artifactDigests).toHaveProperty('attempt-1:binary');
  });
});
