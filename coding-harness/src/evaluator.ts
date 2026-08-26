// SPDX-License-Identifier: MIT

import { existsSync, realpathSync, statSync } from 'node:fs';
import { mkdir, rm } from 'node:fs/promises';
import { isAbsolute, join, relative, resolve, sep } from 'node:path';
import { normalizeWorkspacePath } from './contracts.js';
import { runGitCommand } from './git-process.js';
import type { GitIdentity } from './receipts.js';

const GIT_OBJECT = /^[a-f0-9]{40,64}$/;
const TASK_ID = /^[A-Za-z0-9_-]{8,128}$/;
const FIXED_DATE = '2000-01-01T00:00:00Z';

export interface EvaluatorIdentity extends GitIdentity {
  readonly ref: string;
}

export async function materializeEvaluatorCommit(input: Readonly<{
  repositoryRoot: string;
  scratchRoot: string;
  baselineCommit: string;
  referenceCandidateCommit: string;
  evaluatorPaths: readonly string[];
  implementationPaths: readonly string[];
  taskId: string;
  signal?: AbortSignal;
}>): Promise<EvaluatorIdentity> {
  const repositoryRoot = canonicalDirectory(input.repositoryRoot, 'REPOSITORY_ROOT');
  const scratchRoot = normalizedAbsolute(input.scratchRoot, 'scratchRoot');
  if (contains(repositoryRoot, scratchRoot) || contains(scratchRoot, repositoryRoot)) {
    throw new Error('HARNESS_EVALUATOR_SCRATCH_OVERLAP');
  }
  if (existsSync(scratchRoot)) throw new Error('HARNESS_EVALUATOR_SCRATCH_EXISTS');
  assertGitObject(input.baselineCommit, 'baselineCommit');
  assertGitObject(input.referenceCandidateCommit, 'referenceCandidateCommit');
  if (!TASK_ID.test(input.taskId)) throw new Error('HARNESS_EVALUATOR_TASK_ID_INVALID');
  const evaluatorRef = `refs/metaharness/evaluators/${input.taskId}`;
  const evaluatorPaths = normalizedUniquePaths(input.evaluatorPaths, 'evaluatorPaths');
  const implementationPaths = normalizedUniquePaths(input.implementationPaths, 'implementationPaths');
  if (evaluatorPaths.some((path) => implementationPaths.includes(path))) {
    throw new Error('HARNESS_EVALUATOR_IMPLEMENTATION_PATH_OVERLAP');
  }
  const expectedSourcePaths = [...evaluatorPaths, ...implementationPaths].sort();
  await gitChecked(repositoryRoot, ['cat-file', '-e', `${input.baselineCommit}^{commit}`], input.signal);
  await gitChecked(repositoryRoot, ['cat-file', '-e', `${input.referenceCandidateCommit}^{commit}`], input.signal);
  const sourcePaths = parseNullPaths((await gitChecked(
    repositoryRoot,
    ['diff', '--name-only', '-z', input.baselineCommit, input.referenceCandidateCommit, '--'],
    input.signal,
  )).stdout);
  assertExactPaths(sourcePaths, expectedSourcePaths, 'HARNESS_SOURCE_FIX_PATH_MISMATCH');
  const patch = (await gitChecked(
    repositoryRoot,
    ['diff', '--binary', '--full-index', input.baselineCommit, input.referenceCandidateCommit, '--', ...evaluatorPaths],
    input.signal,
  )).stdout;
  if (patch.trim().length === 0) throw new Error('HARNESS_EVALUATOR_PATCH_EMPTY');

  await mkdir(scratchRoot, { mode: 0o700 });
  const indexPath = join(scratchRoot, 'evaluator.index');
  const indexEnvironment = { GIT_INDEX_FILE: indexPath };
  try {
    await gitChecked(repositoryRoot, ['read-tree', input.baselineCommit], input.signal, undefined, indexEnvironment);
    await gitChecked(
      repositoryRoot,
      ['apply', '--cached', '--whitespace=error-all', '-'],
      input.signal,
      patch,
      indexEnvironment,
    );
    const tree = (await gitChecked(
      repositoryRoot,
      ['write-tree'],
      input.signal,
      undefined,
      indexEnvironment,
    )).stdout.trim();
    assertGitObject(tree, 'evaluator tree');
    const commitEnvironment = {
      ...indexEnvironment,
      GIT_AUTHOR_NAME: 'Semantic Fabric Harness',
      GIT_AUTHOR_EMAIL: 'harness@example.invalid',
      GIT_AUTHOR_DATE: FIXED_DATE,
      GIT_COMMITTER_NAME: 'Semantic Fabric Harness',
      GIT_COMMITTER_EMAIL: 'harness@example.invalid',
      GIT_COMMITTER_DATE: FIXED_DATE,
    };
    const commit = (await gitChecked(
      repositoryRoot,
      ['commit-tree', '--no-gpg-sign', tree, '-p', input.baselineCommit],
      input.signal,
      `Frozen evaluator for ${input.taskId}\n`,
      commitEnvironment,
    )).stdout.trim();
    assertGitObject(commit, 'evaluator commit');
    await verifyEvaluatorSplit({
      repositoryRoot,
      baselineCommit: input.baselineCommit,
      referenceCandidateCommit: input.referenceCandidateCommit,
      evaluatorCommit: commit,
      evaluatorPaths,
      implementationPaths,
      signal: input.signal,
    });
    await retainEvaluatorRef(repositoryRoot, evaluatorRef, commit, input.signal);
    return Object.freeze({ commit, tree, ref: evaluatorRef });
  } finally {
    await rm(scratchRoot, { recursive: true, force: true });
  }
}

async function retainEvaluatorRef(
  repositoryRoot: string,
  ref: string,
  commit: string,
  signal?: AbortSignal,
): Promise<void> {
  const existing = await runGitCommand(
    repositoryRoot,
    ['rev-parse', '--verify', ref],
    { signal },
  );
  if (existing.exitCode === 0) {
    if (existing.stdout.trim() !== commit) throw new Error('HARNESS_EVALUATOR_REF_CONFLICT');
    return;
  }
  await gitChecked(
    repositoryRoot,
    ['update-ref', ref, commit, '0'.repeat(40)],
    signal,
  );
}

async function verifyEvaluatorSplit(input: Readonly<{
  repositoryRoot: string;
  baselineCommit: string;
  referenceCandidateCommit: string;
  evaluatorCommit: string;
  evaluatorPaths: readonly string[];
  implementationPaths: readonly string[];
  signal?: AbortSignal;
}>): Promise<void> {
  const evaluatorDiff = parseNullPaths((await gitChecked(
    input.repositoryRoot,
    ['diff', '--name-only', '-z', input.baselineCommit, input.evaluatorCommit, '--'],
    input.signal,
  )).stdout);
  assertExactPaths(evaluatorDiff, [...input.evaluatorPaths].sort(), 'HARNESS_EVALUATOR_PATH_MISMATCH');
  const remainingDiff = parseNullPaths((await gitChecked(
    input.repositoryRoot,
    ['diff', '--name-only', '-z', input.evaluatorCommit, input.referenceCandidateCommit, '--'],
    input.signal,
  )).stdout);
  assertExactPaths(
    remainingDiff,
    [...input.implementationPaths].sort(),
    'HARNESS_IMPLEMENTATION_PATH_MISMATCH',
  );
}

async function gitChecked(
  cwd: string,
  args: readonly string[],
  signal?: AbortSignal,
  stdin?: string,
  environment?: Readonly<Record<string, string>>,
) {
  const result = await runGitCommand(cwd, args, { signal, stdin, environment });
  if (result.exitCode !== 0) throw new Error(`HARNESS_EVALUATOR_GIT_FAILED:${args[0]}`);
  return result;
}

function normalizedUniquePaths(values: readonly string[], label: string): string[] {
  if (values.length === 0) throw new Error(`HARNESS_${label.toUpperCase()}_REQUIRED`);
  const paths = values.map((value, index) => normalizeWorkspacePath(value, `${label}[${index}]`));
  if (new Set(paths).size !== paths.length) throw new Error(`HARNESS_${label.toUpperCase()}_DUPLICATE`);
  return paths.sort();
}

function parseNullPaths(value: string): string[] {
  return [...new Set(value.split('\0').filter(Boolean)
    .map((path) => normalizeWorkspacePath(path, 'Git changed path')))].sort();
}

function assertExactPaths(actual: readonly string[], expected: readonly string[], error: string): void {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(error);
}

function assertGitObject(value: string, label: string): void {
  if (!GIT_OBJECT.test(value)) throw new TypeError(`${label} is not a Git object ID`);
}

function canonicalDirectory(value: string, label: string): string {
  const path = normalizedAbsolute(value, label);
  if (!statSync(path).isDirectory() || realpathSync(path) !== path) {
    throw new Error(`HARNESS_EVALUATOR_${label}_INVALID`);
  }
  return path;
}

function normalizedAbsolute(value: string, label: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) {
    throw new TypeError(`${label} must be an absolute normalized path`);
  }
  return value;
}

function contains(root: string, child: string): boolean {
  const delta = relative(root, child);
  return delta === '' || (delta !== '..' && !delta.startsWith(`..${sep}`) && !isAbsolute(delta));
}
