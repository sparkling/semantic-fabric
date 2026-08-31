// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { parseTaskContract } from '../src/contracts.js';
import { GitProtectedInputBoundary } from '../src/git-protected-boundary.js';
import { createTestConfig } from './helpers.js';

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('Git-backed composite protected boundary', () => {
  it('binds controller blobs and live evaluator files without requiring one historical tree', async () => {
    const fixture = repository();
    const config = createTestConfig(['controller.txt', 'tests/evaluator.txt']);
    const task = parseTaskContract({
      schemaVersion: 1,
      taskId: 'task-protected-0001',
      runId: 'run-protected-0001',
      workspaceRoot: fixture.evaluatorRoot,
      readablePaths: [],
      mutablePaths: ['src/implementation.txt'],
      protectedPaths: ['controller.txt', 'tests/evaluator.txt'],
      tools: ['git'],
      commands: [],
      network: { mode: 'offline', allowedOrigins: [] },
      authority: 'development-only-no-promotion',
    }, config);
    const boundary = new GitProtectedInputBoundary({
      repositoryRoot: fixture.repositoryRoot,
      controllerCommit: fixture.commit,
      evaluatorRoot: fixture.evaluatorRoot,
      evaluatorPaths: ['tests/evaluator.txt'],
    });

    const snapshot = await boundary.capture(task, config);

    expect(snapshot['controller.txt']).toBe(sha256('trusted controller\n'));
    expect(snapshot['tests/evaluator.txt']).toBe(sha256('sealed evaluator\n'));
    expect((await boundary.verify(task, config, snapshot)).allow).toBe(true);
    writeFileSync(join(fixture.evaluatorRoot, 'tests/evaluator.txt'), 'mutated evaluator\n');
    const decision = await boundary.verify(task, config, snapshot);
    expect(decision.allow).toBe(false);
    expect(decision.reasons.join(' ')).toContain('tests/evaluator.txt');
  });

  it('hashes an exact non-UTF8 gzip Git blob without string re-encoding', async () => {
    const fixture = repository();
    const path = 'controller-payload.gz';
    const config = createTestConfig([path]);
    const task = parseTaskContract({
      schemaVersion: 1,
      taskId: 'task-protected-binary-0001',
      runId: 'run-protected-binary-0001',
      workspaceRoot: fixture.evaluatorRoot,
      readablePaths: [], mutablePaths: ['src/implementation.txt'],
      protectedPaths: [path], tools: ['git'], commands: [],
      network: { mode: 'offline', allowedOrigins: [] },
      authority: 'development-only-no-promotion',
    }, config);
    const boundary = new GitProtectedInputBoundary({
      repositoryRoot: fixture.repositoryRoot,
      controllerCommit: fixture.commit,
      evaluatorRoot: fixture.evaluatorRoot,
      evaluatorPaths: [],
    });

    expect(() => new TextDecoder('utf-8', { fatal: true }).decode(fixture.binary)).toThrow();
    const snapshot = await boundary.capture(task, config);
    expect(snapshot).toEqual({
      [path]: sha256(fixture.binary),
    });
    await expect(boundary.verify(task, config, snapshot)).resolves.toEqual({
      allow: true,
      reasons: ['protected controller blobs and evaluator files match'],
    });
  });
});

function repository(): Readonly<{
  repositoryRoot: string;
  evaluatorRoot: string;
  commit: string;
  binary: Buffer;
}> {
  const repositoryRoot = mkdtempSync(join(tmpdir(), 'harness-protected-repository-'));
  const evaluatorRoot = mkdtempSync(join(tmpdir(), 'harness-protected-evaluator-'));
  roots.push(repositoryRoot, evaluatorRoot);
  mkdirSync(join(repositoryRoot, 'src'));
  writeFileSync(join(repositoryRoot, 'controller.txt'), 'trusted controller\n');
  const binary = readFileSync(new URL(
    '../config/programme-v5-ruflo-schema-v2-memory-bridge.js.gz', import.meta.url,
  ));
  writeFileSync(join(repositoryRoot, 'controller-payload.gz'), binary);
  writeFileSync(join(repositoryRoot, 'src/implementation.txt'), 'before\n');
  git(repositoryRoot, ['init', '--quiet']);
  git(repositoryRoot, [
    'add', '--', 'controller.txt', 'controller-payload.gz', 'src/implementation.txt',
  ]);
  git(repositoryRoot, ['commit', '--quiet', '-m', 'controller'], identityEnvironment());
  const commit = git(repositoryRoot, ['rev-parse', 'HEAD']).trim();
  mkdirSync(join(evaluatorRoot, 'src'));
  mkdirSync(join(evaluatorRoot, 'tests'));
  writeFileSync(join(evaluatorRoot, 'src/implementation.txt'), 'before\n');
  writeFileSync(join(evaluatorRoot, 'tests/evaluator.txt'), 'sealed evaluator\n');
  return { repositoryRoot, evaluatorRoot, commit, binary };
}

function git(cwd: string, args: readonly string[], environment = process.env): string {
  const result = spawnSync('git', args, { cwd, env: environment, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr);
  return result.stdout;
}

function identityEnvironment(): NodeJS.ProcessEnv {
  return {
    ...process.env,
    GIT_AUTHOR_NAME: 'Harness Test',
    GIT_AUTHOR_EMAIL: 'harness@example.invalid',
    GIT_AUTHOR_DATE: '2000-01-01T00:00:00Z',
    GIT_COMMITTER_NAME: 'Harness Test',
    GIT_COMMITTER_EMAIL: 'harness@example.invalid',
    GIT_COMMITTER_DATE: '2000-01-01T00:00:00Z',
  };
}

function sha256(value: string | Buffer): string {
  return createHash('sha256').update(value).digest('hex');
}
