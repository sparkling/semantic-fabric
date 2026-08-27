// SPDX-License-Identifier: MIT

import { chmodSync, mkdirSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import {
  PROGRAMME_V5_ACCEPTANCE_TASK_PATH,
  assertProgrammeV5ControlledRoot,
  parseProgrammeV5Bootstrap,
  parseProgrammeV5Invocation,
  parseProgrammeV5PolicyReviewInvocation,
  parseProgrammeV5ReplayInvocation,
  verifyProgrammeV5ExpectedPolicyFingerprint,
} from '../src/programme-v5-program-runtime.js';

const roots: string[] = [];
const COMMIT = 'a'.repeat(40);

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('programme v5 trusted runtime inputs', () => {
  it('defaults only to the schema-v5 task and binds an explicit normalized task', () => {
    const invocation = parseProgrammeV5Invocation(invocationArgs());
    expect(invocation.taskPath).toBe(PROGRAMME_V5_ACCEPTANCE_TASK_PATH);
    expect(invocation.expectedPolicy).toEqual({
      controllerCommit: COMMIT,
      taskPath: PROGRAMME_V5_ACCEPTANCE_TASK_PATH,
      fingerprint: 'f'.repeat(64),
    });
    expect(invocation.policyReviewReceipt)
      .toBe(join(invocation.repositoryRoot, 'coding-harness', '.metaharness', 'runs',
        'programme_v5_test_run.policy-review.json'));
    const explicit = parseProgrammeV5Invocation([
      ...invocationArgs(), '--task-path', PROGRAMME_V5_ACCEPTANCE_TASK_PATH,
    ]);
    expect(explicit.taskPath).toBe(PROGRAMME_V5_ACCEPTANCE_TASK_PATH);
    expect(explicit.expectedPolicy.taskPath).toBe(PROGRAMME_V5_ACCEPTANCE_TASK_PATH);
  });

  it('rejects malformed and duplicate task invocations', () => {
    for (const extra of [
      ['--task-path', '../task.json'],
      ['--task-path', PROGRAMME_V5_ACCEPTANCE_TASK_PATH,
        '--task-path', PROGRAMME_V5_ACCEPTANCE_TASK_PATH],
      ['--unknown', PROGRAMME_V5_ACCEPTANCE_TASK_PATH],
    ]) {
      expect(() => parseProgrammeV5Invocation([...invocationArgs(), ...extra])).toThrow();
    }
    for (const [flag, value] of [
      ['--hive-id', 'mesh'],
      ['--consensus-id', 'majority'],
      ['--expected-policy-fingerprint', '0'.repeat(64)],
      ['--expected-policy-fingerprint', 'not-a-digest'],
    ]) {
      const args = invocationArgs();
      args[args.indexOf(flag) + 1] = value;
      expect(() => parseProgrammeV5Invocation(args)).toThrow();
    }
    const missingAnchor = invocationArgs();
    const anchorIndex = missingAnchor.indexOf('--expected-policy-fingerprint');
    missingAnchor.splice(anchorIndex, 2);
    expect(() => parseProgrammeV5Invocation(missingAnchor)).toThrow(
      'HARNESS_PROGRAMME_V5_ARGUMENTS_INVALID',
    );
    const missingReceipt = invocationArgs();
    const receiptIndex = missingReceipt.indexOf('--policy-review-receipt');
    missingReceipt.splice(receiptIndex, 2);
    expect(() => parseProgrammeV5Invocation(missingReceipt)).toThrow(
      'HARNESS_PROGRAMME_V5_ARGUMENTS_INVALID',
    );
    expect(() => parseProgrammeV5Invocation([
      ...invocationArgs(), '--expected-policy-fingerprint', 'e'.repeat(64),
    ])).toThrow('HARNESS_PROGRAMME_V5_ARGUMENTS_INVALID');
  });

  it('parses prepare-only policy review separately from execution authority', () => {
    const reviewArgs = invocationArgs();
    reviewArgs.splice(
      reviewArgs.indexOf('--expected-policy-fingerprint'),
      2,
      '--policy-review',
      'prepare-only',
    );
    reviewArgs.splice(reviewArgs.indexOf('--policy-review-receipt'), 2);
    expect(parseProgrammeV5PolicyReviewInvocation(reviewArgs)).toMatchObject({
      controllerCommit: COMMIT,
      taskPath: PROGRAMME_V5_ACCEPTANCE_TASK_PATH,
      reviewMode: 'prepare-only',
    });
    expect(() => parseProgrammeV5Invocation(reviewArgs))
      .toThrow('HARNESS_PROGRAMME_V5_ARGUMENTS_INVALID');
    reviewArgs[reviewArgs.indexOf('--policy-review') + 1] = 'execute';
    expect(() => parseProgrammeV5PolicyReviewInvocation(reviewArgs))
      .toThrow('HARNESS_PROGRAMME_V5_POLICY_REVIEW_MODE_INVALID');
    expect(() => parseProgrammeV5PolicyReviewInvocation([
      ...reviewArgs,
      '--expected-policy-fingerprint',
      'f'.repeat(64),
    ])).toThrow('HARNESS_PROGRAMME_V5_ARGUMENTS_INVALID');
  });

  it('parses verify-only replay with exact run-scoped receipt paths', () => {
    const args = invocationArgs();
    const repository = args[args.indexOf('--repository') + 1]!;
    args.push(
      '--replay', 'verify-only',
      '--envelope-receipt', join(
        repository, 'coding-harness', '.metaharness', 'runs',
        'programme_v5_test_run.json',
      ),
      '--receipt-path', join(
        repository, 'coding-harness', '.metaharness', 'runs',
        'programme_v5_test_run.replay.json',
      ),
    );
    const replay = parseProgrammeV5ReplayInvocation(args);
    expect(replay).toMatchObject({
      replayMode: 'verify-only',
      runId: 'programme_v5_test_run',
      expectedPolicy: { fingerprint: 'f'.repeat(64) },
    });
    expect(() => parseProgrammeV5Invocation(args))
      .toThrow('HARNESS_PROGRAMME_V5_ARGUMENTS_INVALID');
    args[args.indexOf('--replay') + 1] = 'execute';
    expect(() => parseProgrammeV5ReplayInvocation(args))
      .toThrow('HARNESS_PROGRAMME_V5_REPLAY_MODE_INVALID');
  });

  it('requires the exact trusted bootstrap shape and non-genesis digests', () => {
    const value = bootstrap();
    expect(parseProgrammeV5Bootstrap(value)).toEqual(value);
    const { taskPath: _taskPath, ...missing } = value;
    for (const malformed of [
      missing,
      { ...value, extra: true },
      { ...value, schemaVersion: 2 },
      { ...value, controllerStoreDigest: '0'.repeat(64) },
    ]) {
      expect(() => parseProgrammeV5Bootstrap(malformed)).toThrow();
    }
  });

  it('verifies the external fingerprint and its commit/task binding', () => {
    const invocation = parseProgrammeV5Invocation(invocationArgs());
    expect(verifyProgrammeV5ExpectedPolicyFingerprint(invocation, 'f'.repeat(64)))
      .toBe('f'.repeat(64));
    expect(() => verifyProgrammeV5ExpectedPolicyFingerprint(invocation, 'e'.repeat(64)))
      .toThrow('HARNESS_PROGRAMME_V5_EXPECTED_POLICY_FINGERPRINT_MISMATCH');
    expect(() => verifyProgrammeV5ExpectedPolicyFingerprint({
      ...invocation,
      expectedPolicy: { ...invocation.expectedPolicy, taskPath: 'coding-harness/config/other-acceptance.json' },
    }, 'f'.repeat(64))).toThrow('HARNESS_PROGRAMME_V5_EXPECTED_POLICY_BINDING_INVALID');
  });

  it('returns only the validated controlled child, never its scratch parent', () => {
    const scratch = temporary('programme-v5-scratch-');
    const controlled = temporaryWithin(scratch, 'controlled');
    expect(assertProgrammeV5ControlledRoot(scratch, controlled)).toBe(controlled);
    expect(() => assertProgrammeV5ControlledRoot(scratch, scratch))
      .toThrow('HARNESS_PROGRAMME_V5_CONTROLLED_ROOT_INVALID');
    expect(() => assertProgrammeV5ControlledRoot(scratch, temporary('programme-v5-outside-')))
      .toThrow('HARNESS_PROGRAMME_V5_CONTROLLED_ROOT_INVALID');
  });
});

function invocationArgs(): string[] {
  const repository = temporary('programme-v5-repository-');
  return [
    '--repository', repository,
    '--controller-store', temporary('programme-v5-store-'),
    '--controller-commit', COMMIT,
    '--run-id', 'programme_v5_test_run',
    '--swarm-id', 'programme_v5_test_swarm',
    '--coordination-task-id', 'programme_v5_test_task',
    '--hive-id', 'hierarchical',
    '--consensus-id', 'raft',
    '--expected-policy-fingerprint', 'f'.repeat(64),
    '--policy-review-receipt', join(
      repository, 'coding-harness', '.metaharness', 'runs',
      'programme_v5_test_run.policy-review.json',
    ),
  ];
}

function bootstrap() {
  return {
    schemaVersion: 3 as const,
    source: 'verified-packed-private-runtime' as const,
    controllerCommit: COMMIT,
    taskPath: PROGRAMME_V5_ACCEPTANCE_TASK_PATH,
    controllerStoreDigest: '1'.repeat(64),
    buildManifestDigest: '2'.repeat(64),
    runtimeTreeDigest: '3'.repeat(64),
    nodeDigest: '4'.repeat(64),
    gitDigest: '5'.repeat(64),
  };
}

function temporary(prefix: string): string {
  const root = mkdtempSync(join(tmpdir(), prefix));
  chmodSync(root, 0o700);
  roots.push(root);
  return root;
}

function temporaryWithin(parent: string, name: string): string {
  const root = join(parent, name);
  mkdirSync(root, { mode: 0o700 });
  return root;
}
