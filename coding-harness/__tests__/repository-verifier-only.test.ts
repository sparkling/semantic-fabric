// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import type { AcceptanceTask } from '../src/acceptance-task.js';
import { parseTaskContract, type StructuredCommand } from '../src/contracts.js';
import { GitWorktreeSet } from '../src/git-worktrees.js';
import type { OfflineProcessIsolator } from '../src/network.js';
import {
  RepositoryCandidateOperations,
  type RepositoryModelController,
} from '../src/repository-operations.js';
import { candidateExpectationForTask } from '../src/repository-options.js';
import { createTestConfig, TEST_RESOURCE_SCOPE } from './helpers.js';

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function git(cwd: string, args: string[]): string {
  const result = spawnSync('git', args, { cwd, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr);
  return result.stdout.trim();
}

function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'coding-harness-verifier-only-'));
  roots.push(root);
  mkdirSync(join(root, 'src'));
  writeFileSync(join(root, 'src/file.txt'), 'before\n');
  writeFileSync(join(root, 'protected.txt'), 'oracle\n');
  git(root, ['init', '--quiet']);
  git(root, ['config', 'user.email', 'harness@example.invalid']);
  git(root, ['config', 'user.name', 'Harness Test']);
  git(root, ['add', '--all']);
  git(root, ['commit', '--quiet', '-m', 'baseline']);
  const commit = git(root, ['rev-parse', 'HEAD']);
  writeFileSync(join(root, 'src/file.txt'), 'novel accepted implementation\n');
  const patch = `${git(root, ['diff', '--binary', '--', 'src/file.txt'])}\n`;
  git(root, ['reset', '--hard', '--quiet', 'HEAD']);
  return { root, commit, patch };
}

const command: StructuredCommand = {
  tool: 'node',
  executable: process.execPath,
  argv: ['--version'],
  cwd: '.',
  env: {},
  timeoutMs: 2_000,
  maxOutputBytes: 10_000,
};

const offlineIsolator: OfflineProcessIsolator = {
  assertStable() {},
  async terminateAndVerify() {},
  isolate: (source) => ({
    enforcement: 'os-network-namespace',
    mechanism: 'test-no-network',
    resourceScope: TEST_RESOURCE_SCOPE,
    command: {
      ...source,
      executable: '/usr/bin/env',
      args: [source.executable, ...source.args],
    },
  }),
};

const model: RepositoryModelController = {
  architecture: async () => { throw new Error('not used'); },
  implement: async () => { throw new Error('not used'); },
  repair: async () => { throw new Error('not used'); },
  review: async () => { throw new Error('not used'); },
  recoveryEvidence: () => ({ retryCount: 0, breakerState: 'closed', events: [] }),
};

describe('verifier-only repository admission', () => {
  it('accepts a novel tree while retaining worktree identity checks', async () => {
    const source = fixture();
    const parent = mkdtempSync(join(tmpdir(), 'coding-harness-verifier-only-run-'));
    roots.push(parent);
    const worktrees = new GitWorktreeSet({
      repositoryRoot: source.root,
      runRoot: join(parent, 'worktrees'),
    });
    const config = createTestConfig();
    const options = {
      worktrees,
      config,
      baselineCommit: source.commit,
      evaluatorCommit: source.commit,
      candidateExpectation: candidateExpectationForTask({
        schemaVersion: 3,
        taskId: 'verifier-only-admission-0001',
        candidateOracle: { mode: 'verifier-only' },
        evidence: { requiredAdmittedPaths: ['src/file.txt'], generatedOutputs: [] },
      } as unknown as AcceptanceTask),
      taskForWorkspace: (candidateRoot) => parseTaskContract({
        schemaVersion: 1,
        taskId: 'verifier-only-admission-0001',
        runId: 'verifier-only-run-0001',
        workspaceRoot: candidateRoot,
        readablePaths: [],
        mutablePaths: ['src/file.txt', 'build.out'],
        protectedPaths: ['protected.txt'],
        tools: ['node', 'apply_patch'],
        commands: [command],
        network: { mode: 'offline', allowedOrigins: [] },
        authority: 'development-only-no-promotion',
      }, config),
      buildCommands: [command],
      verifierCommands: { public: [command], independent: [command], regression: [command] },
      artifactPaths: ['build.out'],
      model,
      offlineIsolator,
      offlineEnvironment: { PATH: process.env.PATH },
      agenticQeEvidence: async () => [],
      preflightEvidence: async () => ({ passed: true, reasons: [], commands: [], digests: {} }),
      mutationEvidence: async () => ({ passed: true, reasons: [], commands: [], digests: {} }),
    };
    expect(() => new RepositoryCandidateOperations({
      ...options,
      candidateExpectation: { mode: 'verifer-only' } as never,
    })).toThrow('HARNESS_CANDIDATE_EXPECTATION_INVALID');
    const exactExpectation = candidateExpectationForTask({
      schemaVersion: 2,
      taskId: 'verifier-only-admission-0001',
      candidateOracle: {
        mode: 'exact-reference',
        candidate: { commit: source.commit, tree: 'a'.repeat(40) },
      },
    } as unknown as AcceptanceTask);
    expect(() => new RepositoryCandidateOperations({
      ...options,
      candidateExpectation: {
        ...exactExpectation,
        mode: 'verifier-only',
        requiredAdmittedPaths: ['src/file.txt'],
      } as never,
    })).toThrow('HARNESS_CANDIDATE_EXPECTATION_INVALID');
    const operations = new RepositoryCandidateOperations(options);

    await operations.prepare();
    const admission = await operations.admitAndApply({
      payload: source.patch,
      authorInvocationId: 'native-author-0001',
    });

    expect(await operations.validateAdmission(admission)).toEqual([]);
    expect(await operations.validateAdmission({
      ...admission,
      admittedPaths: ['src/file.txt', 'build.out'],
    })).toContain('HARNESS_CANDIDATE_ADMITTED_PATHS_MISMATCH');
    expect(await operations.validateAdmission({
      ...admission,
      admittedPaths: ['build.out'],
    })).toContain('HARNESS_CANDIDATE_ADMITTED_PATHS_MISMATCH');
    expect(await operations.validateAdmission({
      ...admission,
      candidate: { commit: '0'.repeat(40), tree: '0'.repeat(40) },
    })).toContain('HARNESS_ADMISSION_WORKTREE_IDENTITY_MISMATCH');
    await operations.cleanup();
  });
});
