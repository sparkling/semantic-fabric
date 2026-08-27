// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  failureCodeForError,
  normalizeNativeReviewError,
  gitApplyFailureCode,
  isRepairablePatchFailure,
} from '../src/failure-code.js';
import { ReceiptChain, digestValue, parseReceiptDraft } from '../src/receipts.js';

const gitCommit = 'a'.repeat(40);
const gitTree = 'b'.repeat(40);
const timestamp = '2026-08-25T00:00:00.000Z';

function draft(step = 'build') {
  return {
    schemaVersion: 3,
    runId: 'run-0001',
    taskId: 'task-0001',
    step,
    status: 'fail',
    failureCode: 'HARNESS_TRANSACTION_FAILED' as string | null,
    authority: 'development-only-no-promotion',
    issuedAt: timestamp,
    identities: {
      controller: { commit: gitCommit, tree: gitTree },
      baseline: { commit: gitCommit, tree: gitTree },
      evaluator: { commit: gitCommit, tree: gitTree },
      candidate: { commit: gitCommit, tree: gitTree },
    },
    protectedInputs: { 'protected.txt': digestValue('protected') },
    route: {
      snapshotDigest: digestValue('route'),
      frozenAt: timestamp,
      routerVersion: '0.4.0',
    },
    hosts: [{
      host: 'codex',
      model: 'gpt-native',
      role: 'implementer',
      clientVersion: '1.0.0',
      authClass: 'native-openai-subscription',
      subscriptionCostUsd: 0,
    }],
    admittedPaths: ['src/change.ts'],
    patchDigest: digestValue('patch'),
    patchDigests: [digestValue('patch')],
    toolVersions: { node: process.version },
    commands: [{
      stage: 'build',
      attempt: 0,
      candidateTree: gitTree,
      tool: 'node',
      executable: 'node',
      argv: ['--version'],
      cwd: '.',
      exitCode: 0,
      signal: null,
      durationMs: 10,
      stdoutDigest: digestValue('stdout'),
      stderrDigest: digestValue('stderr'),
      timedOut: false,
      cancelled: false,
      outputLimitExceeded: false,
      spawnErrorDigest: null,
    }],
    artifactDigests: { build: digestValue('artifact') },
    verifierDigests: { unit: digestValue('verifier') },
    critiqueDigests: [digestValue('critique')],
    reviewDigests: [digestValue('review')],
    recovery: {
      retryCount: 0,
      breakerState: 'closed',
      cancelled: false,
      repairCount: 0,
    },
    coordination: {
      swarmId: 'swarm-1',
      taskId: 'ruflo-task-1',
      hookIds: ['route-1'],
      traceIds: ['trace-1'],
      agenticQeEvidenceDigests: [digestValue('qe')],
      nativeEvidenceDigests: [digestValue('native')],
      nativeRuntimeEvidenceDigest: null,
    },
  };
}

function passDraft() {
  const value = draft('candidate-transaction');
  const candidateTree = 'c'.repeat(40);
  value.status = 'pass';
  value.failureCode = null;
  value.identities.candidate = { commit: 'd'.repeat(40), tree: candidateTree };
  value.hosts = [
    value.hosts[0],
    {
      host: 'claude-code', model: 'claude-native', role: 'reviewer', clientVersion: '1.0.0',
      authClass: 'native-anthropic-subscription', subscriptionCostUsd: 0,
    },
  ];
  const command = value.commands[0];
  value.commands = [
    { ...command, stage: 'red-baseline', candidateTree: gitTree, exitCode: 101 },
    { ...command, stage: 'build', candidateTree, exitCode: 0 },
    { ...command, stage: 'mutation', candidateTree, exitCode: 101 },
  ];
  value.verifierDigests = {
    'red-baseline': digestValue('red'), 'attempt-0:mutation': digestValue('mutation'),
    'attempt-0:public': digestValue('public'),
    'attempt-0:independent': digestValue('independent'),
    'attempt-0:regression': digestValue('regression'),
  };
  value.reviewDigests = [digestValue('codex-review'), digestValue('claude-review')];
  value.coordination.nativeEvidenceDigests = ['host-a', 'host-b', 'invoke-a', 'invoke-b']
    .map(digestValue);
  value.coordination.nativeRuntimeEvidenceDigest = digestValue('native-runtime');
  return value;
}

describe('strict receipt schema', () => {
  it('requires fixed development-only authority and rejects unknown fields', () => {
    expect(() => parseReceiptDraft({ ...draft(), authority: 'publish' })).toThrow(/promotion authority/);
    expect(() => parseReceiptDraft({ ...draft(), extra: true })).toThrow(/invalid keys/);
  });

  it('binds a finite code-only failure taxonomy to receipt status', () => {
    expect(() => parseReceiptDraft({ ...draft(), failureCode: null })).toThrow(
      /failureCode must identify a failed transaction/,
    );
    expect(() => parseReceiptDraft({ ...passDraft(), failureCode: 'HARNESS_CLEANUP_FAILED' }))
      .toThrow(/passing receipt cannot have a failureCode/);
    expect(() => parseReceiptDraft({ ...draft(), failureCode: 'HARNESS_MODEL_SECRET' }))
      .toThrow(/failureCode is invalid/);
    expect(() => parseReceiptDraft({
      ...draft(), failureCode: 'HARNESS_NATIVE_HOST_FAILED:private detail',
    })).toThrow(/failureCode is invalid/);
  });

  it('extracts only bounded allowlisted codes from an Error graph', () => {
    expect(failureCodeForError(new Error('opaque', {
      cause: new Error('HARNESS_NATIVE_STRUCTURED_OUTPUT_MISSING:private detail'),
    }))).toBe('HARNESS_NATIVE_STRUCTURED_OUTPUT_MISSING');
    expect(failureCodeForError(new AggregateError([
      new Error('HARNESS_CLEANUP_FAILED:private detail'),
    ], 'opaque'))).toBe('HARNESS_CLEANUP_FAILED');
    expect(failureCodeForError(new Error('HARNESS_MODEL_SECRET:private detail')))
      .toBe('HARNESS_TRANSACTION_FAILED');
    expect(failureCodeForError({ message: 'HARNESS_NATIVE_HOST_FAILED' }))
      .toBe('HARNESS_TRANSACTION_FAILED');
    expect(failureCodeForError(new Error(`HARNESS_NATIVE_HOST_FAILED:${'x'.repeat(4_096)}`)))
      .toBe('HARNESS_TRANSACTION_FAILED');
    for (const code of [
      'HARNESS_NATIVE_EVIDENCE_FILE_CHANGED',
      'HARNESS_NATIVE_EVIDENCE_FILE_INVALID',
      'HARNESS_NATIVE_INVOCATION_EXECUTION_MISMATCH',
      'HARNESS_NATIVE_OUTPUT_DIGEST_INVALID',
      'HARNESS_NATIVE_REVIEW_CONTRADICTORY',
      'HARNESS_NATIVE_REVIEW_LIMIT_EXCEEDED',
      'HARNESS_NATIVE_REVIEW_REASON_REQUIRED',
    ]) expect(failureCodeForError(new Error(code))).toBe(code);
  });

  it('retains safe review errors and assigns an opaque failure to the review stage', () => {
    const exact = new Error('HARNESS_NATIVE_HOST_TIMEOUT:codex');
    expect(normalizeNativeReviewError(exact)).toBe(exact);
    expect(failureCodeForError(normalizeNativeReviewError(
      new Error('private provider parser detail'),
    ))).toBe('HARNESS_NATIVE_REVIEW_FAILED');
  });

  it('separates model-correctable admission failures from terminal application failures', () => {
    for (const code of [
      'HARNESS_PATCH_ADMISSION_INVALID',
      'HARNESS_PATCH_EMPTY',
      'HARNESS_PATCH_INVALID',
      'HARNESS_PATCH_PATH_NOT_DECLARED',
      'HARNESS_PATCH_TOO_LARGE',
    ]) expect(isRepairablePatchFailure(code)).toBe(true);
    for (const code of [
      'HARNESS_PATCH_APPLICATION_FAILED',
      'HARNESS_PATCH_ADMISSION_CHANGED',
      'HARNESS_PATCH_TREE_UNCHANGED',
      'HARNESS_PATCH_POLICY_GATE',
      'HARNESS_GIT_COMMAND_TIMEOUT',
      'HARNESS_GIT_COMMAND_CANCELLED',
      'HARNESS_NATIVE_PATCH_INVALID',
      'HARNESS_NATIVE_PATCH_RESPONSE_INVALID',
      'HARNESS_CLEANUP_FAILED',
      'HARNESS_NATIVE_HOST_TIMEOUT',
      'HARNESS_NATIVE_INVOCATION_CANCELLED',
    ]) expect(isRepairablePatchFailure(code)).toBe(false);
    expect(gitApplyFailureCode(['apply', '--numstat'], 1)).toBe('HARNESS_PATCH_ADMISSION_INVALID');
    expect(gitApplyFailureCode(['apply', '--check', '--index'], 1)).toBe(
      'HARNESS_PATCH_ADMISSION_INVALID',
    );
    expect(gitApplyFailureCode(['apply', '--index'], 1)).toBe('HARNESS_PATCH_APPLICATION_FAILED');
    expect(gitApplyFailureCode(['apply', '--check'], null)).toBeNull();
    expect(gitApplyFailureCode(['apply', '--check'], 0)).toBeNull();
    expect(() => gitApplyFailureCode(['status'], 1)).toThrow('git apply arguments required');
  });

  it('rejects a structurally hashed pass without candidate transaction evidence', () => {
    expect(() => parseReceiptDraft({ ...draft(), status: 'pass', failureCode: null })).toThrow(
      'HARNESS_PASS_RECEIPT_EVIDENCE_INCOMPLETE',
    );
  });

  it('rejects contradictory or duplicated pass evidence', () => {
    expect(parseReceiptDraft(passDraft()).status).toBe('pass');
    for (const invalid of [
      { recovery: { ...passDraft().recovery, cancelled: true } },
      { recovery: { ...passDraft().recovery, breakerState: 'open' } },
      { patchDigests: [...passDraft().patchDigests, digestValue('extra-patch')] },
      { reviewDigests: Array(2).fill(digestValue('same-review')) },
      { coordination: {
        ...passDraft().coordination,
        nativeEvidenceDigests: Array(4).fill(digestValue('same-native')),
      } },
    ]) {
      expect(() => parseReceiptDraft({ ...passDraft(), ...invalid })).toThrow(
        'HARNESS_PASS_RECEIPT_EVIDENCE_INCOMPLETE',
      );
    }
  });

  it('binds native host authentication and rejects indirect gateways', () => {
    const invalid = draft();
    invalid.hosts[0].model = 'openrouter/gpt';
    expect(() => parseReceiptDraft(invalid)).toThrow(/indirect gateway/);

    const wrongAuth = draft();
    wrongAuth.hosts[0].authClass = 'native-anthropic-subscription';
    expect(() => parseReceiptDraft(wrongAuth)).toThrow(/does not match/);
  });
});

describe('receipt chain', () => {
  it('rejects legacy v1 chains after the evidence-shape version bump', () => {
    expect(() => ReceiptChain.import(JSON.stringify({ schemaVersion: 1, receipts: [] })))
      .toThrow(/schemaVersion 3/);
  });

  it('chains canonical receipts and round-trips verified evidence', () => {
    const chain = new ReceiptChain();
    const first = chain.append(draft('build'));
    const second = chain.append(draft('verify'));
    expect(first.previousDigest).toBe('0'.repeat(64));
    expect(second.previousDigest).toBe(first.digest);
    expect(chain.verify()).toEqual({ ok: true });

    const restored = ReceiptChain.import(chain.export());
    expect(restored.verify()).toEqual({ ok: true });
    expect(restored.headDigest).toBe(chain.headDigest);
  });

  it('detects body tampering during import', () => {
    const chain = new ReceiptChain();
    chain.append(draft());
    const exported = JSON.parse(chain.export());
    exported.receipts[0].failureCode = 'HARNESS_CLEANUP_FAILED';
    expect(() => ReceiptChain.import(JSON.stringify(exported))).toThrow(/digest does not match body/);
  });

  it('does not expose mutable receipt entries', () => {
    const chain = new ReceiptChain();
    const receipt = chain.append(draft());
    expect(Object.isFrozen(receipt)).toBe(true);
    expect(() => { (receipt as { step: string }).step = 'tampered'; }).toThrow();
    expect(chain.verify()).toEqual({ ok: true });
  });
});
