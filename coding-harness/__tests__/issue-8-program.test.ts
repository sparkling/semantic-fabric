// SPDX-License-Identifier: MIT

import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import {
  trustedControllerMain,
  type TrustedBootstrapEvidence,
} from '../src/issue-8-program.js';

const TASK = 'coding-harness/config/issue-8-acceptance.json';
const COMMIT = 'a'.repeat(40);
const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('trusted controller task bootstrap binding', () => {
  it('requires the exact schema-v3 bootstrap shape', async () => {
    const invocation = invocationArgs();
    const valid = bootstrap();
    const { taskPath: _taskPath, ...missingTaskPath } = valid;

    for (const value of [
      { ...valid, schemaVersion: 2 },
      missingTaskPath,
      { ...valid, extra: true },
      { ...valid, taskPath: '../task.json' },
    ]) {
      await expect(trustedControllerMain(invocation, value)).rejects.toThrow();
    }
  });

  it('rejects task-path or controller-commit divergence before execution', async () => {
    const invocation = invocationArgs();
    await expect(trustedControllerMain(invocation, {
      ...bootstrap(),
      taskPath: 'coding-harness/config/m0-acceptance.json',
    })).rejects.toThrow('HARNESS_ISSUE_8_BOOTSTRAP_BINDING_MISMATCH');
    await expect(trustedControllerMain(invocation, {
      ...bootstrap(),
      controllerCommit: 'b'.repeat(40),
    })).rejects.toThrow('HARNESS_ISSUE_8_BOOTSTRAP_BINDING_MISMATCH');
  });

  it('rejects a malformed invocation task path before execution', async () => {
    await expect(trustedControllerMain(
      [...invocationArgs(), '--task-path', '/absolute-task.json'],
      bootstrap(),
    )).rejects.toThrow();
  });
});

function invocationArgs(): string[] {
  return [
    '--repository', temporary('trusted-program-repository-'),
    '--controller-store', temporary('trusted-program-store-'),
    '--controller-commit', COMMIT,
    '--run-id', 'bootstrap_test_run',
    '--swarm-id', 'bootstrap_test_swarm',
    '--coordination-task-id', 'bootstrap_test_task',
    '--hive-id', 'bootstrap_test_hive',
    '--consensus-id', 'bootstrap_test_consensus',
  ];
}

function bootstrap(): TrustedBootstrapEvidence {
  return {
    schemaVersion: 3,
    source: 'verified-packed-private-runtime',
    controllerCommit: COMMIT,
    taskPath: TASK,
    controllerStoreDigest: '1'.repeat(64),
    buildManifestDigest: '2'.repeat(64),
    runtimeTreeDigest: '3'.repeat(64),
    nodeDigest: '4'.repeat(64),
    gitDigest: '5'.repeat(64),
  };
}

function temporary(prefix: string): string {
  const root = mkdtempSync(join(tmpdir(), prefix));
  roots.push(root);
  return root;
}
