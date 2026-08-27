// SPDX-License-Identifier: MIT

import { chmodSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import {
  prepareReviewableProgrammeV5Policy,
  prepareTrustedProgrammeV5,
} from '../src/programme-v5-program.js';
import { PROGRAMME_V5_ACCEPTANCE_TASK_PATH } from '../src/programme-v5-program-runtime.js';

const COMMIT = 'a'.repeat(40);
const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('trusted programme-v5 preparation boundary', () => {
  it('rejects task or commit divergence before preparing a transaction', async () => {
    await expect(prepareTrustedProgrammeV5(invocationArgs(), {
      ...bootstrap(),
      taskPath: 'coding-harness/config/alternate-programme-v5-acceptance.json',
    })).rejects.toThrow('HARNESS_PROGRAMME_V5_BOOTSTRAP_BINDING_MISMATCH');
    await expect(prepareTrustedProgrammeV5(invocationArgs(), {
      ...bootstrap(),
      controllerCommit: 'b'.repeat(40),
    })).rejects.toThrow('HARNESS_PROGRAMME_V5_BOOTSTRAP_BINDING_MISMATCH');
  });

  it('binds prepare-only policy review before any transaction preparation', async () => {
    const review = invocationArgs();
    review.splice(
      review.indexOf('--expected-policy-fingerprint'),
      2,
      '--policy-review',
      'prepare-only',
    );
    review.splice(review.indexOf('--policy-review-receipt'), 2);
    await expect(prepareReviewableProgrammeV5Policy(review, {
      ...bootstrap(),
      controllerCommit: 'b'.repeat(40),
    })).rejects.toThrow('HARNESS_PROGRAMME_V5_BOOTSTRAP_BINDING_MISMATCH');
    await expect(prepareReviewableProgrammeV5Policy(invocationArgs(), bootstrap()))
      .rejects.toThrow('HARNESS_PROGRAMME_V5_ARGUMENTS_INVALID');
  });
});

function invocationArgs(): string[] {
  const repository = temporary('programme-v5-program-repository-');
  return [
    '--repository', repository,
    '--controller-store', temporary('programme-v5-program-store-'),
    '--controller-commit', COMMIT,
    '--run-id', 'programme_v5_program_run',
    '--swarm-id', 'programme_v5_program_swarm',
    '--coordination-task-id', 'programme_v5_program_task',
    '--hive-id', 'hierarchical',
    '--consensus-id', 'raft',
    '--expected-policy-fingerprint', 'f'.repeat(64),
    '--policy-review-receipt', join(
      repository, 'coding-harness', '.metaharness', 'runs',
      'programme_v5_program_run.policy-review.json',
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
