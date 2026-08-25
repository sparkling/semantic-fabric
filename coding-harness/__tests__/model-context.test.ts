// SPDX-License-Identifier: MIT

import { link, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { runGitCommand } from '../src/git-process.js';
import { RepositoryModelContextProvider } from '../src/model-context.js';

const SEALED_EVALUATOR_FIXTURE = 'SEALED_EVALUATOR_FIXTURE_MUST_NOT_LEAK';
const IMPLEMENTATION_PATH = 'src/implementation.rs';
const EVALUATOR_PATH = 'tests/sealed_evaluator.rs';
const roots: string[] = [];

afterEach(async () => {
  await Promise.all(roots.splice(0).map(async (root) => await rm(root, {
    recursive: true,
    force: true,
  })));
});

describe('repository model context provider', () => {
  it('captures only declared source and the exact filtered staged diff', async () => {
    const root = await repositoryFixture();
    const provider = new RepositoryModelContextProvider({
      candidateRoot: root,
      implementationPaths: [IMPLEMENTATION_PATH],
      evaluatorPaths: [EVALUATOR_PATH],
      maxTotalBytes: 100_000,
    });

    const declared = await provider.declaredSource();
    expect(declared.files.map(({ path }) => path)).toEqual([IMPLEMENTATION_PATH]);
    expect(declared.files[0]?.content).toContain('ORIGINAL_IMPLEMENTATION');
    expect(JSON.stringify(declared)).not.toContain(SEALED_EVALUATOR_FIXTURE);

    await writeFile(join(root, IMPLEMENTATION_PATH), 'fn implementation() { /* ADMITTED_FIX */ }\n');
    await git(root, ['add', '--', IMPLEMENTATION_PATH]);
    const expectedDiff = (await git(root, [
      'diff', '--cached', '--binary', '--full-index', '--no-renames', '--',
      `:(literal)${IMPLEMENTATION_PATH}`,
    ])).stdout;
    const admitted = await provider.admittedSource();

    expect(admitted.files[0]?.content).toContain('ADMITTED_FIX');
    expect(admitted.stagedPaths).toEqual([IMPLEMENTATION_PATH]);
    expect(admitted.stagedDiff).toBe(expectedDiff);
    expect(admitted.stagedDiff).toContain('ADMITTED_FIX');
    expect(JSON.stringify(admitted)).not.toContain(SEALED_EVALUATOR_FIXTURE);
  });

  it('rejects staged evaluator content instead of hiding it from the filtered diff', async () => {
    const root = await repositoryFixture();
    const provider = providerFor(root);
    await writeFile(join(root, EVALUATOR_PATH), `${SEALED_EVALUATOR_FIXTURE}\nchanged\n`);
    await git(root, ['add', '--', EVALUATOR_PATH]);

    await expect(provider.admittedSource()).rejects.toThrow(
      `HARNESS_MODEL_CONTEXT_EVALUATOR_STAGED:${EVALUATOR_PATH}`,
    );
  });

  it('rejects non-local hard-linked implementation files', async () => {
    const root = await emptyRepository();
    await mkdir(join(root, 'src'));
    await mkdir(join(root, 'tests'));
    const outside = join(root, 'outside.rs');
    await writeFile(outside, 'fn implementation() {}\n');
    await link(outside, join(root, IMPLEMENTATION_PATH));
    await writeFile(join(root, EVALUATOR_PATH), `${SEALED_EVALUATOR_FIXTURE}\n`);
    await git(root, ['add', '--', IMPLEMENTATION_PATH, EVALUATOR_PATH]);
    await commit(root);

    await expect(providerFor(root).declaredSource()).rejects.toThrow(/hard-linked|FILE_UNTRUSTED/);
  });

  it('fails closed when the context byte budget cannot hold the declared source', async () => {
    const root = await repositoryFixture();
    const provider = new RepositoryModelContextProvider({
      candidateRoot: root,
      implementationPaths: [IMPLEMENTATION_PATH],
      evaluatorPaths: [EVALUATOR_PATH],
      maxTotalBytes: 8,
    });

    await expect(provider.declaredSource()).rejects.toThrow(
      'HARNESS_MODEL_CONTEXT_BYTE_LIMIT_EXCEEDED',
    );
  });
});

function providerFor(root: string): RepositoryModelContextProvider {
  return new RepositoryModelContextProvider({
    candidateRoot: root,
    implementationPaths: [IMPLEMENTATION_PATH],
    evaluatorPaths: [EVALUATOR_PATH],
    maxTotalBytes: 100_000,
  });
}

async function repositoryFixture(): Promise<string> {
  const root = await emptyRepository();
  await mkdir(join(root, 'src'));
  await mkdir(join(root, 'tests'));
  await writeFile(join(root, IMPLEMENTATION_PATH), 'fn implementation() { /* ORIGINAL_IMPLEMENTATION */ }\n');
  await writeFile(join(root, EVALUATOR_PATH), `${SEALED_EVALUATOR_FIXTURE}\n`);
  await git(root, ['add', '--', IMPLEMENTATION_PATH, EVALUATOR_PATH]);
  await commit(root);
  return root;
}

async function emptyRepository(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), 'semantic-fabric-model-context-'));
  roots.push(root);
  await git(root, ['init', '--quiet']);
  return root;
}

async function commit(root: string): Promise<void> {
  await git(root, ['commit', '--quiet', '-m', 'fixture'], {
    GIT_AUTHOR_NAME: 'Harness Test',
    GIT_AUTHOR_EMAIL: 'harness@example.invalid',
    GIT_AUTHOR_DATE: '2000-01-01T00:00:00Z',
    GIT_COMMITTER_NAME: 'Harness Test',
    GIT_COMMITTER_EMAIL: 'harness@example.invalid',
    GIT_COMMITTER_DATE: '2000-01-01T00:00:00Z',
  });
}

async function git(
  root: string,
  args: readonly string[],
  environment?: Readonly<Record<string, string>>,
) {
  const result = await runGitCommand(root, args, { environment });
  if (result.exitCode !== 0) throw new Error(`fixture Git failed: ${result.stderr}`);
  return result;
}
