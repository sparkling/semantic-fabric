// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { afterEach, describe, expect, it } from 'vitest';
import { CandidateTransaction, type CandidateTransactionContext } from '../src/candidate.js';
import { parseTaskContract, type StructuredCommand } from '../src/contracts.js';
import { GitWorktreeSet } from '../src/git-worktrees.js';
import type { OfflineProcessIsolator } from '../src/network.js';
import {
  RepositoryCandidateOperations,
  type RepositoryModelController,
} from '../src/repository-operations.js';
import type { ProtectedInputBoundary } from '../src/policy.js';
import { digestValue, type CommandEvidence } from '../src/receipts.js';
import { createTestConfig } from './helpers.js';

const roots: string[] = [];
const processFixture = join(
  dirname(fileURLToPath(import.meta.url)),
  'fixtures/process-fixture.mjs',
);

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function git(cwd: string, args: string[]): string {
  const result = spawnSync('git', args, { cwd, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr);
  return result.stdout.trim();
}

function repository(): { root: string; commit: string; sourceTree: string; patch: string } {
  const root = mkdtempSync(join(tmpdir(), 'coding-harness-operations-repo-'));
  roots.push(root);
  mkdirSync(join(root, 'src'));
  writeFileSync(join(root, 'src/file.txt'), 'before\n');
  writeFileSync(join(root, 'protected.txt'), 'oracle\n');
  git(root, ['init', '--quiet']);
  git(root, ['config', 'user.email', 'harness@example.invalid']);
  git(root, ['config', 'user.name', 'Harness Test']);
  git(root, ['add', '--', 'src/file.txt', 'protected.txt']);
  git(root, ['commit', '--quiet', '-m', 'fixture']);
  const commit = git(root, ['rev-parse', 'HEAD']);
  writeFileSync(join(root, 'src/file.txt'), 'after\n');
  const patch = `${git(root, ['diff', '--binary', '--', 'src/file.txt'])}\n`;
  git(root, ['add', '--', 'src/file.txt']);
  const sourceTree = git(root, ['write-tree']);
  git(root, ['reset', '--hard', '--quiet', 'HEAD']);
  return { root, commit, sourceTree, patch };
}

function command(mode: 'artifact' | 'success'): StructuredCommand {
  return {
    tool: 'node',
    executable: process.execPath,
    argv: mode === 'artifact'
      ? [processFixture, mode, 'build.out']
      : [processFixture, mode],
    cwd: '.',
    env: {},
    timeoutMs: 2_000,
    maxOutputBytes: 10_000,
  };
}

const offlineIsolator: OfflineProcessIsolator = {
  assertStable() {},
  isolate: (source) => ({
    enforcement: 'os-network-namespace',
    mechanism: 'test-no-network',
    command: {
      ...source,
      executable: '/usr/bin/env',
      args: [source.executable, ...source.args],
    },
  }),
};

function context(
  baseline: { commit: string; tree: string },
  protectedDigest: string,
): CandidateTransactionContext {
  return {
    runId: 'run-operations-0001',
    taskId: 'task-operations-0001',
    authority: 'development-only-no-promotion',
    identities: { controller: baseline, baseline, evaluator: baseline },
    protectedInputs: { 'protected.txt': protectedDigest },
    route: {
      snapshotDigest: 'b'.repeat(64),
      frozenAt: '2026-08-25T12:00:00.000Z',
      routerVersion: '@metaharness/router@0.4.0',
    },
    hosts: [
      {
        host: 'codex', model: 'gpt-5', role: 'implementation-review', clientVersion: 'codex 1',
        authClass: 'native-openai-subscription', subscriptionCostUsd: 0,
      },
      {
        host: 'claude-code', model: 'claude-sonnet', role: 'architecture-review', clientVersion: 'claude 1',
        authClass: 'native-anthropic-subscription', subscriptionCostUsd: 0,
      },
    ],
    toolVersions: { git: 'test', node: process.version },
    requiredQeProfiles: ['lcov-gap', 'sast'],
    rufloEvidence: {
      schemaVersion: 1,
      source: 'ruflo-coordination-ledger',
      taskId: 'task-operations-0001',
      runId: 'run-operations-0001',
      swarmId: 'swarm-0001',
      coordinationTaskId: 'ruflo-task-0001',
      hookIds: [],
      traceIds: ['trace-0001'],
      routeSnapshotDigest: 'b'.repeat(64),
      authoritative: false,
      capturedAt: '2026-08-25T12:00:00.000Z',
    },
  };
}

describe('repository operations integration', () => {
  it('runs admitted code through offline build, three gates, reviews, and a receipt', async () => {
    const fixture = repository();
    const parent = mkdtempSync(join(tmpdir(), 'coding-harness-operations-run-'));
    roots.push(parent);
    const worktrees = new GitWorktreeSet({
      repositoryRoot: fixture.root,
      runRoot: join(parent, 'run'),
    });
    const config = createTestConfig();
    const build = command('artifact');
    const verify = command('success');
    const baseline = {
      commit: fixture.commit,
      tree: git(fixture.root, ['rev-parse', `${fixture.commit}^{tree}`]),
    };
    const model: RepositoryModelController = {
      architecture: async () => ({
        value: { plan: true },
        critiqueDigests: [digestValue('critique')],
        invocations: [
          { invocationId: 'architecture-codex', host: 'codex' },
          { invocationId: 'architecture-claude', host: 'claude-code' },
        ],
      }),
      implement: async () => ({ payload: fixture.patch, authorInvocationId: 'author-0001' }),
      repair: async () => {
        throw new Error('repair should not be called');
      },
      review: async (host, candidateBuild) => ({
        host,
        invocationId: `review-${host}`,
        candidate: candidateBuild.candidate,
        accepted: true,
        digest: digestValue({ host, review: 'accepted' }),
        reasons: [],
      }),
      recoveryEvidence: () => ({ retryCount: 0, breakerState: 'closed', events: [] }),
    };
    const operations = new RepositoryCandidateOperations({
      worktrees,
      config,
      baselineCommit: fixture.commit,
      evaluatorCommit: fixture.commit,
      expectedCandidate: { commit: fixture.commit, tree: fixture.sourceTree },
      taskForWorkspace: (candidateRoot) => parseTaskContract({
        schemaVersion: 1,
        taskId: 'task-operations-0001',
        runId: 'run-operations-0001',
        workspaceRoot: candidateRoot,
        readablePaths: [],
        mutablePaths: ['src/file.txt', 'build.out'],
        protectedPaths: ['protected.txt'],
        tools: ['node', 'apply_patch', 'git'],
        commands: [build, verify],
        network: { mode: 'offline', allowedOrigins: [] },
        authority: 'development-only-no-promotion',
      }, config),
      buildCommands: [build],
      verifierCommands: { public: [verify], independent: [verify], regression: [verify] },
      artifactPaths: ['build.out'],
      model,
      offlineIsolator,
      offlineEnvironment: { PATH: process.env.PATH, HOME: '/home/harness' },
      agenticQeEvidence: async (candidateBuild) => ['lcov-gap', 'sast'].map((profile) => ({
        schemaVersion: 1, source: 'agentic-qe-local-profile', profile,
        taskId: 'task-operations-0001', runId: 'run-operations-0001',
        candidateTree: candidateBuild.candidate.tree,
        commandDigest: digestValue(`qe-command-${profile}`),
        outputDigest: digestValue(`qe-output-${profile}`),
        providerVariablesStripped: true, authoritative: false,
        capturedAt: '2026-08-25T12:00:30.000Z',
      })),
      preflightEvidence: async (prepared) => ({
        passed: true,
        reasons: [],
        commands: [acceptanceCommand('red-baseline', 0, prepared.evaluator.tree, 101)],
        digests: { 'red-baseline': digestValue('red-baseline') },
      }),
      mutationEvidence: async (candidateBuild) => ({
        passed: true,
        reasons: [],
        commands: [acceptanceCommand(
          'mutation',
          candidateBuild.commands[0].attempt,
          candidateBuild.candidate.tree,
          101,
        )],
        digests: { mutation: digestValue('mutation') },
      }),
    });
    const transaction = new CandidateTransaction({
      context: context(
        baseline,
        createHash('sha256').update(readFileSync(join(fixture.root, 'protected.txt'))).digest('hex'),
      ),
      operations,
      maxRepairs: 0,
      now: () => '2026-08-25T12:01:00.000Z',
    });

    const result = await transaction.execute();

    expect(result.status, result.reason ?? '').toBe('gated');
    expect(result.reason).toContain('HARNESS_NATIVE_TRUSTED_RUNTIME_UNAVAILABLE');
    expect(result.receipt.commands).toHaveLength(3);
    expect(result.receipt.commands.find(({ stage }) => stage === 'build')).toMatchObject({
      attempt: 0,
      candidateTree: result.receipt.identities.candidate.tree,
      spawnErrorDigest: null,
    });
    expect(Object.keys(result.receipt.verifierDigests).sort()).toEqual([
      'attempt-0:independent', 'attempt-0:mutation', 'attempt-0:public',
      'attempt-0:regression', 'red-baseline',
    ]);
    expect(result.receipt.protectedInputs['protected.txt']).toMatch(/^[a-f0-9]{64}$/);
    expect(transaction.receipts.verify()).toEqual({ ok: true });
    expect(result.receipt.coordination.traceIds).toEqual(['trace-0001']);
  });

  it('uses a composite protected boundary with explicitly pre-prepared historical worktrees', async () => {
    const fixture = repository();
    const parent = mkdtempSync(join(tmpdir(), 'coding-harness-composite-boundary-'));
    roots.push(parent);
    const controllerRoot = join(parent, 'controller');
    mkdirSync(controllerRoot, { mode: 0o700 });
    writeFileSync(join(controllerRoot, 'controller.txt'), 'trusted controller\n');
    const worktrees = new GitWorktreeSet({
      repositoryRoot: fixture.root,
      runRoot: join(parent, 'run'),
    });
    const prePrepared = await worktrees.prepare(fixture.commit, fixture.commit);
    expect(() => readFileSync(join(prePrepared.candidateRoot, 'controller.txt'))).toThrow();

    const config = createTestConfig(['controller.txt', 'protected.txt']);
    const capture = async () => Object.freeze({
      'controller.txt': fileDigest(join(controllerRoot, 'controller.txt')),
      'protected.txt': fileDigest(join(prePrepared.evaluatorRoot, 'protected.txt')),
    });
    const protectedInputBoundary: ProtectedInputBoundary = {
      capture,
      verify: async (_task, _config, expected) => {
        const current = await capture();
        const changed = Object.keys(expected).filter((path) => expected[path] !== current[path]);
        return changed.length === 0
          ? { allow: true, reasons: ['composite protected inputs match'] }
          : { allow: false, reasons: [`protected inputs changed: ${changed.join(', ')}`] };
      },
    };
    const build = command('artifact');
    const verify = command('success');
    const model: RepositoryModelController = {
      architecture: async () => { throw new Error('not used'); },
      implement: async () => { throw new Error('not used'); },
      repair: async () => { throw new Error('not used'); },
      review: async () => { throw new Error('not used'); },
      recoveryEvidence: () => ({ retryCount: 0, breakerState: 'closed', events: [] }),
    };
    const operations = new RepositoryCandidateOperations({
      worktrees,
      config,
      baselineCommit: fixture.commit,
      evaluatorCommit: fixture.commit,
      expectedCandidate: { commit: fixture.commit, tree: fixture.sourceTree },
      taskForWorkspace: (candidateRoot) => parseTaskContract({
        schemaVersion: 1,
        taskId: 'task-composite-0001',
        runId: 'run-composite-0001',
        workspaceRoot: candidateRoot,
        readablePaths: [],
        mutablePaths: ['src/file.txt', 'build.out'],
        protectedPaths: ['controller.txt', 'protected.txt'],
        tools: ['node', 'apply_patch', 'git'],
        commands: [build, verify],
        network: { mode: 'offline', allowedOrigins: [] },
        authority: 'development-only-no-promotion',
      }, config),
      buildCommands: [build],
      verifierCommands: { public: [verify], independent: [verify], regression: [verify] },
      artifactPaths: ['build.out'],
      model,
      offlineIsolator,
      offlineEnvironment: { PATH: process.env.PATH, HOME: '/home/harness' },
      protectedInputBoundary,
      agenticQeEvidence: async () => [],
      preflightEvidence: async () => ({ passed: true, reasons: [], commands: [], digests: {} }),
      mutationEvidence: async () => ({ passed: true, reasons: [], commands: [], digests: {} }),
    });

    const prepared = await operations.prepare();
    expect(prepared.candidate).toEqual(prePrepared.candidate);
    expect(Object.keys(prepared.protectedInputs).sort()).toEqual(['controller.txt', 'protected.txt']);
    expect((await operations.verifyProtectedInputs()).allow).toBe(true);

    writeFileSync(join(controllerRoot, 'controller.txt'), 'mutated controller\n');
    expect((await operations.verifyProtectedInputs()).reasons.join(' ')).toContain('controller.txt');
    writeFileSync(join(controllerRoot, 'controller.txt'), 'trusted controller\n');
    writeFileSync(join(prePrepared.evaluatorRoot, 'protected.txt'), 'mutated evaluator\n');
    expect((await operations.verifyProtectedInputs()).reasons.join(' ')).toContain('protected.txt');
    await operations.cleanup();
  });

  it('rejects Agentic-QE evidence that mutates the independent verifier source', async () => {
    const fixture = repository();
    const parent = mkdtempSync(join(tmpdir(), 'coding-harness-qe-mutation-'));
    roots.push(parent);
    const worktrees = new GitWorktreeSet({
      repositoryRoot: fixture.root,
      runRoot: join(parent, 'run'),
    });
    const config = createTestConfig();
    const buildCommand = command('artifact');
    const verifyCommand = command('success');
    const model: RepositoryModelController = {
      architecture: async () => { throw new Error('not used'); },
      implement: async () => { throw new Error('not used'); },
      repair: async () => { throw new Error('not used'); },
      review: async () => { throw new Error('not used'); },
      recoveryEvidence: () => ({ retryCount: 0, breakerState: 'closed', events: [] }),
    };
    const operations = new RepositoryCandidateOperations({
      worktrees,
      config,
      baselineCommit: fixture.commit,
      evaluatorCommit: fixture.commit,
      expectedCandidate: { commit: fixture.commit, tree: fixture.sourceTree },
      taskForWorkspace: (candidateRoot) => parseTaskContract({
        schemaVersion: 1,
        taskId: 'task-qe-mutation-0001',
        runId: 'run-qe-mutation-0001',
        workspaceRoot: candidateRoot,
        readablePaths: [],
        mutablePaths: ['src/file.txt', 'build.out'],
        protectedPaths: ['protected.txt'],
        tools: ['node', 'apply_patch', 'git'],
        commands: [buildCommand, verifyCommand],
        network: { mode: 'offline', allowedOrigins: [] },
        authority: 'development-only-no-promotion',
      }, config),
      buildCommands: [buildCommand],
      verifierCommands: {
        public: [verifyCommand], independent: [verifyCommand], regression: [verifyCommand],
      },
      artifactPaths: ['build.out'],
      model,
      offlineIsolator,
      offlineEnvironment: { PATH: process.env.PATH, HOME: '/home/harness' },
      agenticQeEvidence: async () => {
        writeFileSync(
          join(worktrees.verifierRoot('independent'), 'src/file.txt'),
          'tampered by QE\n',
        );
        return [];
      },
      preflightEvidence: async () => ({ passed: true, reasons: [], commands: [], digests: {} }),
      mutationEvidence: async () => ({ passed: true, reasons: [], commands: [], digests: {} }),
    });
    try {
      await operations.prepare();
      await operations.resetCandidate();
      const admission = await operations.admitAndApply({
        payload: fixture.patch,
        authorInvocationId: 'author-qe-mutation',
      });
      const candidateBuild = await operations.build(admission, 0);
      await expect(operations.agenticQeEvidence(candidateBuild)).rejects.toThrow(
        'HARNESS_VERIFIER_SOURCE_MUTATED:independent',
      );
    } finally {
      await operations.cleanup();
    }
  });
});

function fileDigest(path: string): string {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

function acceptanceCommand(
  stage: 'red-baseline' | 'mutation',
  attempt: number,
  candidateTree: string,
  exitCode: number,
): CommandEvidence {
  return {
    stage, attempt, candidateTree, tool: 'node', executable: process.execPath,
    argv: [], cwd: '.', exitCode, signal: null, durationMs: 1,
    stdoutDigest: digestValue('stdout'), stderrDigest: digestValue('stderr'),
    timedOut: false, cancelled: false, outputLimitExceeded: false, spawnErrorDigest: null,
  };
}
