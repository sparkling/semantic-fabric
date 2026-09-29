import { mkdtempSync, readFileSync, rmSync, unlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, expect, it } from 'vitest';
import { dispatchDeliveryReady } from '../src/delivery-ready.js';
import { runDeliveryOutcome } from '../src/delivery-runner.js';
import { git } from '../src/delivery-workspace.js';
import { activeReservations } from '../src/delivery-cohort-custody.js';
import { reopenDeliveryCandidate } from '../src/delivery-candidate.js';
import type { DeliveryHarness } from '../src/delivery-runtime.js';
import type { DeliveryPoolProgress } from '../src/delivery-pool.js';
import { native, workflowFixture } from './delivery-workflow-fixtures.js';

const roots: string[] = [];
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });
const deferred = () => { let resolve!: () => void; const promise = new Promise<void>(yes => { resolve = yes; }); return { promise, resolve }; };
function fixture() {
  const f = workflowFixture(roots), parentDirectory = mkdtempSync(join(tmpdir(), 'early-accept-')); roots.push(parentDirectory);
  for (const path of ['slow.txt', 'child.txt']) writeFileSync(join(f.root, path), 'before\n');
  git(f.root, 'add', '.'); git(f.root, 'commit', '-qm', 'fixture scopes');
  const outcomes = ['product.txt', 'slow.txt'].map((path, i) => ({
    task: { ...f.task, id: `early-${i}`, scope: [path], readPaths: ['coding-harness/check.mjs'],
      requested: { host: native.host, model: native.model, effort: native.effort }, selectionReason: 'Injected fixture' },
    handoff: native, resources: [`resource-${i}`],
  }));
  return { ...f, manifest: { schemaVersion: 1, parentDirectory, maxConcurrency: 2, mode: 'run', outcomes } };
}
async function execute(candidate: DeliveryHarness, _mode: string, id: string, owner: string, signal: AbortSignal) {
  return (await runDeliveryOutcome(candidate, id, owner, { signal, execute: async (request, files) => ({
    response: { schemaVersion: 1, requestId: request.id, sourceDigest: request.sourceDigest,
      native: { ...native, ...request.route, executorId: request.executorId ?? 'fresh-review' }, outcome: 'completed', summary: 'Injected boundary', issues: [] },
    changes: request.stage === 'implementation' ? [{ path: request.scope[0], content: request.taskId === 'child'
      ? files.find(row => row.path === 'product.txt')!.content! + 'child\n' : 'fixed\n' }] : [],
    ...(request.stage === 'architecture' ? { plan: { summary: 'Scoped fix', files: request.scope, tests: ['build', 'public'] } } : {}),
  }) })).success;
}
async function accept(harness: DeliveryHarness, event: DeliveryPoolProgress, whilePrepared?: () => void) {
  const original = JSON.parse(readFileSync(join(event.evidenceDirectory!, `${event.taskId}.json`), 'utf8'));
  const run = await harness.integrate({ candidateRoot: event.candidateRoot, id: event.taskId, owner: original.task.owner, expectedDigest: original.digest });
  whilePrepared?.();
  for (const check of run.task.checks) await harness.check(run.task.id, run.task.owner, check.id);
  expect((await harness.verify(run.task.id, run.task.owner)).verdict?.pass).toBe(true);
  git(harness.root, 'add', '--', ...run.task.scope); git(harness.root, 'commit', '-qm', 'accepted fixture');
  await harness.finish(run.task.id, run.task.owner, git(harness.root, 'rev-parse', 'HEAD'));
}
it('accepts parent and launches accepted-input child while disjoint sibling stays active', async () => {
  const f = fixture(), release = deferred(), parent = deferred(), events: DeliveryPoolProgress[] = [];
  let slow: DeliveryHarness | undefined;
  const cohort = dispatchDeliveryReady(f.harness, f.manifest, async (candidate, mode, id, owner, signal) => {
    if (id === 'early-1') { slow = candidate; await release.promise; }
    return execute(candidate, mode, id, owner, signal);
  }, undefined, event => { events.push(event); if (event.taskId === 'early-0' && event.event === 'outcome-settled') parent.resolve(); });
  try {
    await parent.promise;
    expect(activeReservations(f.harness).map(row => row.id)).toEqual(['early-1']);
    const settled = events.find(row => row.taskId === 'early-0' && row.event === 'outcome-settled')!;
    await accept(f.harness, settled, () => {
      expect(() => slow!.context.assert()).not.toThrow();
      expect(() => reopenDeliveryCandidate(slow!.root)).not.toThrow();
    });
    expect(events.some(row => row.event === 'cohort-drained')).toBe(false);
    const child = { ...f.manifest.outcomes[0], task: { ...f.manifest.outcomes[0].task, id: 'child', scope: ['child.txt'] },
      acceptedParent: 'early-0', acceptedInputs: ['product.txt'], resources: ['child-resource'] };
    const childEvents: DeliveryPoolProgress[] = [];
    const result = await dispatchDeliveryReady(f.harness, { ...f.manifest, outcomes: [child] }, execute, undefined, event => childEvents.push(event));
    expect(result.results[0].status).toBe('fulfilled');
    await accept(f.harness, childEvents.find(row => row.event === 'outcome-settled')!);
    expect(readFileSync(join(f.root, 'child.txt'), 'utf8')).toBe('fixed\nchild\n');
    expect(events.some(row => row.taskId === 'early-1' && row.event === 'outcome-settled')).toBe(false);
    expect(() => slow!.context.assert()).not.toThrow();
  } finally { release.resolve(); }
  expect((await cohort).results.every(row => row.status === 'fulfilled')).toBe(true);
  await accept(f.harness, events.find(row => row.taskId === 'early-1' && row.event === 'outcome-settled')!);
});

it.each(['read', 'write', 'resource'])('rejects conflicting cross-cohort %s admission while sibling runs', async kind => {
  const f = fixture(), started = deferred(), release = deferred();
  const pending = dispatchDeliveryReady(f.harness, { ...f.manifest, outcomes: [f.manifest.outcomes[1]] }, async () => {
    started.resolve(); await release.promise; return true;
  });
  try {
    await started.promise;
    const conflict = structuredClone(f.manifest.outcomes[0]);
    if (kind === 'read') conflict.task.readPaths.push('slow.txt');
    if (kind === 'write') conflict.task.scope = ['slow.txt'];
    if (kind === 'resource') conflict.resources = f.manifest.outcomes[1].resources;
    await expect(dispatchDeliveryReady(f.harness, { ...f.manifest, outcomes: [conflict] }, execute)).rejects.toThrow('RESOURCE_CONFLICT');
  } finally { release.resolve(); await pending; }
});

it('refuses early integration against active reader and refuses unaccepted canonical drift', async () => {
  const f = fixture(), release = deferred(), parent = deferred(), events: DeliveryPoolProgress[] = [];
  f.manifest.outcomes[1].task.readPaths.push('product.txt');
  let slow: DeliveryHarness | undefined;
  const pending = dispatchDeliveryReady(f.harness, f.manifest, async (candidate, mode, id, owner, signal) => {
    if (id === 'early-1') { slow = candidate; await release.promise; return true; }
    return execute(candidate, mode, id, owner, signal);
  }, undefined, event => { events.push(event); if (event.taskId === 'early-0' && event.event === 'outcome-settled') parent.resolve(); });
  try {
    await parent.promise;
    await expect(accept(f.harness, events.find(row => row.event === 'outcome-settled')!)).rejects.toThrow('ACTIVE_DEPENDENCY');
    writeFileSync(join(f.root, 'child.txt'), 'arbitrary');
    expect(() => slow!.context.assert()).not.toThrow();
  } finally { release.resolve(); }
  expect((await pending).results.every(row => row.status === 'fulfilled')).toBe(true);
  await expect(accept(f.harness, events.find(row => row.taskId === 'early-0' && row.event === 'outcome-settled')!))
    .rejects.toThrow('DIRTY_CANONICAL');
  git(f.root, 'add', 'child.txt'); git(f.root, 'commit', '-qm', 'unaccepted drift');
  await expect(accept(f.harness, events.find(row => row.taskId === 'early-0' && row.event === 'outcome-settled')!))
    .rejects.toThrow('UNACCEPTED_DRIFT');
});

it('retains resource custody after a callback leaves an unconfirmed child lock', async () => {
  const f = fixture();
  const result = await dispatchDeliveryReady(f.harness, { ...f.manifest, outcomes: [f.manifest.outcomes[0]] }, async candidate => {
    writeFileSync(join(candidate.directory, 'operation.lock'), JSON.stringify({ phase: 'unconfirmed', childPid: 12345 }));
    throw new Error('check-process-group-unconfirmed');
  });
  expect(result.results[0].status).toBe('rejected');
  expect(activeReservations(f.harness).map(row => row.id)).toEqual(['early-0']);
  const other = { ...f.manifest.outcomes[1], resources: ['resource-0'] };
  await expect(dispatchDeliveryReady(f.harness, { ...f.manifest, outcomes: [other] }, execute)).rejects.toThrow('RESOURCE_CONFLICT');
  // Simulate existing explicit process-custody reconciliation after the child is proved stopped.
  unlinkSync(join(result.results[0].evidenceDirectory!, 'operation.lock'));
  expect(activeReservations(f.harness)).toEqual([]);
});

it('blocks newly introduced evaluator paths and canonical non-integration resume while a lane runs', async () => {
  const f = fixture(), started = deferred(), release = deferred();
  await f.harness.begin(f.task); await f.harness.pause(f.task.id, f.task.owner, 'paused fixture');
  const pending = dispatchDeliveryReady(f.harness, { ...f.manifest, outcomes: [f.manifest.outcomes[1]] }, async () => {
    started.resolve(); await release.promise; return true;
  });
  try {
    await started.promise;
    await expect(f.harness.resume(f.task.id, f.task.owner)).rejects.toThrow('ACTIVE_CANDIDATES');
    const conflict = structuredClone(f.manifest.outcomes[0]); conflict.task.scope = ['coding-harness/new-check.mjs'];
    await expect(dispatchDeliveryReady(f.harness, { ...f.manifest, outcomes: [conflict] }, execute)).rejects.toThrow('RESOURCE_CONFLICT');
  } finally { release.resolve(); await pending; }
});

it('reserves implicit accepted-parent inputs before child execution', async () => {
  const f = fixture(), events: DeliveryPoolProgress[] = [];
  await dispatchDeliveryReady(f.harness, { ...f.manifest, outcomes: [f.manifest.outcomes[0]] }, execute, undefined, event => events.push(event));
  await accept(f.harness, events.find(row => row.event === 'outcome-settled')!);
  const started = deferred(), release = deferred();
  const child = { ...f.manifest.outcomes[1], task: { ...f.manifest.outcomes[1].task, id: 'child', scope: ['child.txt'] }, acceptedParent: 'early-0' };
  const pending = dispatchDeliveryReady(f.harness, { ...f.manifest, outcomes: [child] }, async () => {
    started.resolve(); await release.promise; return true;
  });
  try {
    await started.promise;
    expect(activeReservations(f.harness)[0].readPaths).toContain('product.txt');
    await expect(dispatchDeliveryReady(f.harness, { ...f.manifest, outcomes: [f.manifest.outcomes[0]] }, execute)).rejects.toThrow('RESOURCE_CONFLICT');
  } finally { release.resolve(); await pending; }
});

it('reopens and settles private candidate while canonical Git index is locked', async () => {
  const f = fixture(), ready = deferred(), release = deferred();
  const pending = dispatchDeliveryReady(f.harness, { ...f.manifest, outcomes: [f.manifest.outcomes[0]] }, async (candidate, mode, id, owner, signal) => {
    const result = await execute(candidate, mode, id, owner, signal);
    ready.resolve(); await release.promise;
    expect(() => reopenDeliveryCandidate(candidate.root)).not.toThrow();
    return result;
  });
  await ready.promise;
  const lock = join(f.root, '.git/index.lock');
  writeFileSync(lock, 'concurrent main commit');
  try {
    release.resolve();
    expect((await pending).results[0].status).toBe('fulfilled');
  } finally { unlinkSync(lock); }
});

it('refuses paused integration resume when disjoint candidate owns same resource', async () => {
  const f = fixture(), events: DeliveryPoolProgress[] = [];
  await dispatchDeliveryReady(f.harness, { ...f.manifest, outcomes: [f.manifest.outcomes[0]] }, execute, undefined, event => events.push(event));
  const event = events.find(row => row.event === 'outcome-settled')!;
  const original = JSON.parse(readFileSync(join(event.evidenceDirectory!, `${event.taskId}.json`), 'utf8'));
  await f.harness.integrate({ candidateRoot: event.candidateRoot, id: event.taskId, owner: original.task.owner, expectedDigest: original.digest });
  await f.harness.pause(original.task.id, original.task.owner, 'pause integration');
  const ready = deferred(), release = deferred();
  const other = { ...f.manifest.outcomes[1], resources: ['resource-0'] };
  const pending = dispatchDeliveryReady(f.harness, { ...f.manifest, outcomes: [other] }, async () => {
    ready.resolve(); await release.promise; return true;
  });
  try {
    await ready.promise;
    await expect(f.harness.resume(original.task.id, original.task.owner)).rejects.toThrow('ACTIVE_DEPENDENCY');
  } finally { release.resolve(); await pending; }
  await expect(f.harness.resume(original.task.id, original.task.owner)).resolves.toMatchObject({ status: 'active' });
});
