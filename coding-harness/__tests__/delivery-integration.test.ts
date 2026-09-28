import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { randomUUID } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import * as fs from 'node:fs';
import { hash } from '@metaharness/harness';
import { createDeliveryCandidate } from '../src/delivery-candidate.js';
import { DeliveryHarness } from '../src/delivery-runtime.js';
import { git } from '../src/delivery-workspace.js';
import { native, workflowFixture } from './delivery-workflow-fixtures.js';

vi.mock('node:fs', async importOriginal => {
  const actual = await importOriginal<typeof import('node:fs')>();
  return { ...actual, renameSync: vi.fn(actual.renameSync) };
});

const roots: string[] = [];
afterEach(() => { vi.restoreAllMocks(); for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });
async function fixture(readPaths: string[] | null = ['coding-harness/check.mjs'], combined = false, newFirst = false) {
  const f = workflowFixture(roots);
  writeFileSync(join(f.root, 'other.txt'), 'before\n'); git(f.root, 'add', 'other.txt'); git(f.root, 'commit', '-qm', 'second scope');
  const parentDirectory = mkdtempSync(join(tmpdir(), 'fabric-integration-')); roots.push(parentDirectory);
  const candidates = [];
  for (const [i, path] of (combined ? ['product.txt'] : [newFirst ? 'new.txt' : 'product.txt', 'other.txt']).entries()) {
    const task = { ...f.task, id: `candidate-${i}`, scope: combined ? ['product.txt', 'other.txt'] : [path], ...(readPaths ? { readPaths } : {}) };
    const candidate = createDeliveryCandidate(f.harness, { parentDirectory, scope: task.scope, readPaths: readPaths ?? undefined });
    const h = candidate.harness;
    await h.begin(task); await h.bind(task.id, task.owner, native);
    let action = await h.advance(task.id, task.owner);
    if (action.kind !== 'native') throw new Error('missing implementation');
    for (const scoped of task.scope) writeFileSync(join(h.root, scoped), 'fixed\n');
    await h.submit(task.id, task.owner, { schemaVersion: 1, requestId: action.request.id, sourceDigest: h.snapshot().digest,
      native, outcome: 'completed', summary: 'injected author', issues: [] });
    action = await h.advance(task.id, task.owner);
    if (action.kind !== 'native') throw new Error('missing review');
    await h.submit(task.id, task.owner, { schemaVersion: 1, requestId: action.request.id, sourceDigest: h.snapshot().digest,
      native: { ...native, executorId: 'fresh-reviewer' }, outcome: 'completed', summary: 'injected independent review', issues: [] });
    const run = await h.verify(task.id, task.owner);
    candidates.push({ task, candidate, original: structuredClone(run), input: {
      candidateRoot: h.root, id: task.id, owner: task.owner, expectedDigest: run.digest!,
    } });
  }
  return { ...f, parentDirectory, candidates };
}
async function accept(h: DeliveryHarness, input: Awaited<ReturnType<typeof fixture>>['candidates'][number]['input']) {
  const run = await h.integrate(input);
  for (const check of run.task.checks) await h.check(run.task.id, run.task.owner, check.id);
  expect((await h.verify(run.task.id, run.task.owner)).verdict?.pass).toBe(true);
  git(h.root, 'add', '--', ...run.task.scope); git(h.root, 'commit', '-qm', 'accept candidate');
  return h.finish(run.task.id, run.task.owner, git(h.root, 'rev-parse', 'HEAD'));
}
it('integrates two reviewed same-base siblings serially and releases exact accepted child inputs', async () => {
  const f = await fixture();
  for (const row of f.candidates) {
    expect(() => createDeliveryCandidate(f.harness, { parentDirectory: f.parentDirectory,
      scope: ['product.txt'], acceptedParent: row.task.id })).toThrow();
    const run = await accept(f.harness, row.input);
    expect(run.status).toBe('complete');
    expect(hash(run.workflow)).toBe(hash(row.original.workflow));
    expect(JSON.parse(readFileSync(join(row.candidate.harness.directory, `${row.task.id}.json`), 'utf8'))).toEqual(row.original);
  }
  const child = createDeliveryCandidate(f.harness, { parentDirectory: f.parentDirectory, scope: ['other.txt'],
    acceptedParent: f.candidates[0].task.id, acceptedInputs: ['product.txt'] });
  expect(readFileSync(join(child.harness.root, 'product.txt'), 'utf8')).toBe('fixed\n');
  expect(child.acceptedSource?.commit).toBe(f.harness.read(f.candidates[0].task.id).commit);
});
it('defaults to full read dependencies and refuses stale sibling dependencies', async () => {
  const f = await fixture(null); await accept(f.harness, f.candidates[0].input);
  await expect(f.harness.integrate(f.candidates[1].input)).rejects.toThrow('INPUT_CHANGED');
});
it('default read closure includes files newly introduced by accepted siblings', async () => {
  const f = await fixture(null, false, true); await accept(f.harness, f.candidates[0].input);
  await expect(f.harness.integrate(f.candidates[1].input)).rejects.toThrow('INPUT_CHANGED');
});
it.each(['dirty', 'evaluator', 'source', 'review', 'digest'])('refuses %s tampering before canonical source writes', async kind => {
  const f = await fixture(), row = f.candidates[0], before = readFileSync(join(f.root, 'product.txt'), 'utf8');
  if (kind === 'dirty') writeFileSync(join(f.root, 'unrelated.txt'), 'dirty');
  if (kind === 'evaluator') { writeFileSync(join(f.root, 'coding-harness/check.mjs'), 'process.exit(0); // changed\n'); git(f.root, 'add', '.'); git(f.root, 'commit', '-qm', 'changed evaluator'); }
  if (kind === 'source') writeFileSync(join(row.candidate.harness.root, 'product.txt'), 'tampered\n');
  if (kind === 'review') { const run = structuredClone(row.original); run.workflow!.results.pop(); delete run.digest; run.digest = hash(run);
    writeFileSync(join(row.candidate.harness.directory, `${row.task.id}.json`), JSON.stringify(run)); row.input.expectedDigest = run.digest; }
  if (kind === 'digest') row.input.expectedDigest = '0'.repeat(64);
  await expect(f.harness.integrate(row.input)).rejects.toThrow();
  expect(readFileSync(join(f.root, 'product.txt'), 'utf8')).toBe(before);
  expect(f.harness.inspect().active).toBeNull();
});
it('resumes prepared integration without model work and refuses foreign owner', async () => {
  const f = await fixture(), row = f.candidates[0];
  await f.harness.integrate(row.input);
  const restarted = new DeliveryHarness(f.root);
  await expect(restarted.integrate({ ...row.input, owner: 'intruder' })).rejects.toThrow('OWNER');
  await restarted.integrate(row.input);
  expect((await restarted.next(row.task.id, row.task.owner)).kind).toBe('check');
  expect((await accept(restarted, row.input)).status).toBe('complete');
});
it('refuses committed unrelated drift and automatically pins omitted evaluator inputs and new runtime files', async () => {
  for (const path of ['unrelated.txt', 'coding-harness/check.mjs', 'coding-harness/new-runtime.mjs']) {
    const f = await fixture([]);
    writeFileSync(join(f.root, path), 'changed\n'); git(f.root, 'add', '--', path); git(f.root, 'commit', '-qm', 'unaccepted drift');
    await expect(f.harness.integrate(f.candidates[0].input)).rejects.toThrow(path === 'unrelated.txt' ? 'UNACCEPTED_DRIFT' : 'INPUT_CHANGED');
  }
});
it('recovers durable partially applied source after interruption without replaying model work', async () => {
  const f = await fixture([], true), row = f.candidates[0], { renameSync: rename } = await vi.importActual<typeof import('node:fs')>('node:fs');
  const fault = vi.mocked(fs.renameSync).mockImplementation((from, to) => {
    if (to === join(f.root, 'other.txt')) throw new Error('injected interruption');
    return rename(from, to);
  });
  await expect(f.harness.integrate(row.input)).rejects.toThrow('injected interruption'); fault.mockImplementation(rename);
  expect(readFileSync(join(f.root, 'product.txt'), 'utf8')).toBe('fixed\n');
  expect(readFileSync(join(f.root, 'other.txt'), 'utf8')).toBe('before\n');
  expect(f.harness.read(row.task.id).integration?.phase).toBe('applying');
  const restarted = new DeliveryHarness(f.root);
  expect((await accept(restarted, row.input)).status).toBe('complete');
});
it('finishes exact commit after restart and preserves receipt evidence independently of candidate logs', async () => {
  const f = await fixture(), row = f.candidates[0], run = await f.harness.integrate(row.input);
  for (const check of run.task.checks) await f.harness.check(run.task.id, run.task.owner, check.id);
  for (const check of row.original.checks) writeFileSync(check.stdout, 'candidate log later changed');
  git(f.root, 'add', '--', ...run.task.scope); git(f.root, 'commit', '-qm', 'accepted before interruption');
  const restarted = new DeliveryHarness(f.root);
  expect((await restarted.finish(run.task.id, run.task.owner, git(f.root, 'rev-parse', 'HEAD'))).status).toBe('complete');
});
it('preserves negative and superseded outcome history without treating it as current acceptance', async () => {
  const f = await fixture(), row = f.candidates[0], directory = join(row.candidate.harness.directory, 'runner'); mkdirSync(directory);
  const history = [false, true].map(success => {
    const body = { taskDigest: hash(row.task), sourceAfter: '0'.repeat(64), success,
      kernel: row.original.workflow!.results.at(-1)!.kernel };
    const receipt = { ...body, digest: hash(body) };
    writeFileSync(join(directory, `outcome-${randomUUID()}.json`), JSON.stringify(receipt)); return receipt;
  });
  const accepted = await accept(f.harness, row.input);
  expect(accepted.integration?.outcomeHistory).toHaveLength(2);
  expect(accepted.integration?.outcomeReceipts).toEqual([]);
  expect(accepted.integration?.outcomeHistory).toEqual(expect.arrayContaining(history));
});
