// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { materializeEvaluatorCommit } from '../src/evaluator.js';

const roots: string[] = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function git(cwd: string, args: string[]): string {
  const result = spawnSync('git', args, { cwd, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr);
  return result.stdout.trim();
}

describe('frozen evaluator materialization', () => {
  it('creates an evaluator-only commit and leaves implementation changes excluded', async () => {
    const repositoryRoot = mkdtempSync(join(tmpdir(), 'coding-harness-evaluator-repo-'));
    const scratchParent = mkdtempSync(join(tmpdir(), 'coding-harness-evaluator-scratch-'));
    roots.push(repositoryRoot, scratchParent);
    mkdirSync(join(repositoryRoot, 'src'));
    mkdirSync(join(repositoryRoot, 'tests'));
    writeFileSync(join(repositoryRoot, 'src/fix.txt'), 'before\n');
    git(repositoryRoot, ['init', '--quiet']);
    git(repositoryRoot, ['config', 'user.email', 'harness@example.invalid']);
    git(repositoryRoot, ['config', 'user.name', 'Harness Test']);
    git(repositoryRoot, ['add', '--all']);
    git(repositoryRoot, ['commit', '--quiet', '-m', 'baseline']);
    const baselineCommit = git(repositoryRoot, ['rev-parse', 'HEAD']);
    writeFileSync(join(repositoryRoot, 'src/fix.txt'), 'after\n');
    writeFileSync(join(repositoryRoot, 'tests/evaluator.txt'), 'red oracle\n');
    git(repositoryRoot, ['add', '--all']);
    git(repositoryRoot, ['commit', '--quiet', '-m', 'source fix']);
    const referenceCandidateCommit = git(repositoryRoot, ['rev-parse', 'HEAD']);

    const evaluator = await materializeEvaluatorCommit({
      repositoryRoot,
      scratchRoot: join(scratchParent, 'scratch'),
      baselineCommit,
      source: { mode: 'exact-reference', referenceCandidateCommit },
      evaluatorPaths: ['tests/evaluator.txt'],
      implementationPaths: ['src/fix.txt'],
      taskId: 'task_evaluator_0001',
    });

    expect(git(repositoryRoot, ['show', `${evaluator.commit}:tests/evaluator.txt`])).toBe('red oracle');
    expect(git(repositoryRoot, ['show', `${evaluator.commit}:src/fix.txt`])).toBe('before');
    expect(git(repositoryRoot, ['diff', '--name-only', baselineCommit, evaluator.commit])).toBe(
      'tests/evaluator.txt',
    );
    expect(git(repositoryRoot, ['rev-parse', '--verify', evaluator.ref])).toBe(evaluator.commit);
  });

  it('copies only evaluator paths from a descendant controller commit', async () => {
    const repositoryRoot = mkdtempSync(join(tmpdir(), 'coding-harness-evaluator-repo-'));
    const scratchParent = mkdtempSync(join(tmpdir(), 'coding-harness-evaluator-scratch-'));
    roots.push(repositoryRoot, scratchParent);
    mkdirSync(join(repositoryRoot, 'src'));
    mkdirSync(join(repositoryRoot, 'tests'));
    mkdirSync(join(repositoryRoot, 'private'));
    writeFileSync(join(repositoryRoot, 'src/fix.txt'), 'before\n');
    writeFileSync(join(repositoryRoot, 'private/controller.txt'), 'before\n');
    git(repositoryRoot, ['init', '--quiet']);
    git(repositoryRoot, ['config', 'user.email', 'harness@example.invalid']);
    git(repositoryRoot, ['config', 'user.name', 'Harness Test']);
    git(repositoryRoot, ['add', '--all']);
    git(repositoryRoot, ['commit', '--quiet', '-m', 'baseline']);
    const baselineCommit = git(repositoryRoot, ['rev-parse', 'HEAD']);
    writeFileSync(join(repositoryRoot, 'src/fix.txt'), 'controller implementation\n');
    writeFileSync(join(repositoryRoot, 'tests/evaluator.txt'), 'sealed oracle\n');
    writeFileSync(join(repositoryRoot, 'private/controller.txt'), 'controller-only secret\n');
    git(repositoryRoot, ['add', '--all']);
    git(repositoryRoot, ['commit', '--quiet', '-m', 'controller']);
    const controllerCommit = git(repositoryRoot, ['rev-parse', 'HEAD']);

    const evaluator = await materializeEvaluatorCommit({
      repositoryRoot,
      scratchRoot: join(scratchParent, 'verifier-only'),
      baselineCommit,
      source: { mode: 'verifier-only', controllerCommit },
      evaluatorPaths: ['tests/evaluator.txt'],
      implementationPaths: ['src/fix.txt'],
      taskId: 'task_evaluator_0002',
    });

    expect(git(repositoryRoot, ['show', `${evaluator.commit}:tests/evaluator.txt`])).toBe('sealed oracle');
    expect(git(repositoryRoot, ['show', `${evaluator.commit}:src/fix.txt`])).toBe('before');
    expect(git(repositoryRoot, ['show', `${evaluator.commit}:private/controller.txt`])).toBe('before');
    expect(git(repositoryRoot, ['diff', '--name-only', baselineCommit, evaluator.commit])).toBe(
      'tests/evaluator.txt',
    );

    const orphanCommit = git(repositoryRoot, [
      'commit-tree', `${controllerCommit}^{tree}`, '-m', 'unrelated controller',
    ]);
    await expect(materializeEvaluatorCommit({
      repositoryRoot,
      scratchRoot: join(scratchParent, 'verifier-orphan'),
      baselineCommit,
      source: { mode: 'verifier-only', controllerCommit: orphanCommit },
      evaluatorPaths: ['tests/evaluator.txt'],
      implementationPaths: ['src/fix.txt'],
      taskId: 'task_evaluator_0005',
    })).rejects.toThrow('HARNESS_EVALUATOR_SOURCE_BASELINE_NOT_ANCESTOR');
  });

  it('keeps exact-reference source paths closed and requires every evaluator path', async () => {
    const repositoryRoot = mkdtempSync(join(tmpdir(), 'coding-harness-evaluator-repo-'));
    const scratchParent = mkdtempSync(join(tmpdir(), 'coding-harness-evaluator-scratch-'));
    roots.push(repositoryRoot, scratchParent);
    mkdirSync(join(repositoryRoot, 'src'));
    mkdirSync(join(repositoryRoot, 'tests'));
    writeFileSync(join(repositoryRoot, 'src/fix.txt'), 'before\n');
    git(repositoryRoot, ['init', '--quiet']);
    git(repositoryRoot, ['config', 'user.email', 'harness@example.invalid']);
    git(repositoryRoot, ['config', 'user.name', 'Harness Test']);
    git(repositoryRoot, ['add', '--all']);
    git(repositoryRoot, ['commit', '--quiet', '-m', 'baseline']);
    const baselineCommit = git(repositoryRoot, ['rev-parse', 'HEAD']);
    writeFileSync(join(repositoryRoot, 'src/fix.txt'), 'after\n');
    writeFileSync(join(repositoryRoot, 'tests/unrelated.txt'), 'not declared\n');
    git(repositoryRoot, ['add', '--all']);
    git(repositoryRoot, ['commit', '--quiet', '-m', 'invalid source']);
    const referenceCandidateCommit = git(repositoryRoot, ['rev-parse', 'HEAD']);

    await expect(materializeEvaluatorCommit({
      repositoryRoot,
      scratchRoot: join(scratchParent, 'exact-invalid'),
      baselineCommit,
      source: { mode: 'exact-reference', referenceCandidateCommit },
      evaluatorPaths: ['tests/evaluator.txt'],
      implementationPaths: ['src/fix.txt'],
      taskId: 'task_evaluator_0003',
    })).rejects.toThrow('HARNESS_SOURCE_FIX_PATH_MISMATCH');

    await expect(materializeEvaluatorCommit({
      repositoryRoot,
      scratchRoot: join(scratchParent, 'verifier-invalid'),
      baselineCommit,
      source: { mode: 'verifier-only', controllerCommit: referenceCandidateCommit },
      evaluatorPaths: ['tests/evaluator.txt'],
      implementationPaths: ['src/fix.txt'],
      taskId: 'task_evaluator_0004',
    })).rejects.toThrow('HARNESS_EVALUATOR_SOURCE_PATH_MISMATCH');
  });
});
