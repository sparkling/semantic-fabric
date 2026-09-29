import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { afterEach, expect, it, vi } from 'vitest';
import { deliveryCli } from '../src/delivery-cli.js';
import { dispatchDeliveryReady } from '../src/delivery-ready.js';
import { native, workflowFixture } from './delivery-workflow-fixtures.js';
import * as executor from '../src/delivery-executor.js';
import type { DeliveryPoolProgress } from '../src/delivery-pool.js';
import { runDeliveryOutcome } from '../src/delivery-runner.js';

const roots: string[] = [];
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.unstubAllEnvs(); for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });
function fixture() {
  const f = workflowFixture(roots), parentDirectory = mkdtempSync(join(tmpdir(), 'fabric-ready-')); roots.push(parentDirectory);
  writeFileSync(join(f.root, 'other.txt'), 'second input');
  const manifest = { schemaVersion: 1, parentDirectory, maxConcurrency: 2, mode: 'packet',
    outcomes: ['product.txt', 'other.txt'].map((path, index) => ({
      task: { ...f.task, id: `ready-${index}`, scope: [path], requested: { host: native.host, model: native.model, effort: native.effort },
        selectionReason: 'Explicit fixture executor' }, handoff: native, resources: [`resource-${index}`],
    })) };
  return { ...f, manifest, parentDirectory };
}

it.each([false, true])('reports actual candidate completion and refills before slow sibling finishes (failure=%s)', async failure => {
  const f = fixture(), progress: DeliveryPoolProgress[] = [];
  writeFileSync(join(f.root, 'third.txt'), 'third input');
  const manifest = { ...f.manifest, mode: 'run', outcomes: [...f.manifest.outcomes, {
    ...f.manifest.outcomes[0], task: { ...f.manifest.outcomes[0].task, id: 'ready-2', scope: ['third.txt'] }, resources: ['third'],
  }] };
  let release!: () => void, refilled!: () => void;
  const slow = new Promise<void>(resolve => { release = resolve; });
  const nextStarted = new Promise<void>(resolve => { refilled = resolve; });
  vi.spyOn(executor, 'createDeliveryExecutor').mockReturnValue(async request => {
    if (request.taskId === 'ready-1' && request.stage === 'architecture') await slow;
    if (request.taskId === 'ready-0' && failure) throw new Error('fixture native failure');
    return { response: { schemaVersion: 1, requestId: request.id, sourceDigest: request.sourceDigest,
      native: { ...native, ...request.route, executorId: request.executorId ?? 'fresh-reviewer' },
      outcome: 'completed', summary: 'Injected native boundary', issues: [] },
      changes: request.stage === 'implementation' ? [{ path: request.scope[0], content: 'fixed\n' }] : [],
      ...(request.stage === 'architecture' ? { plan: { summary: 'Fix source', files: request.scope, tests: ['build', 'public'] } } : {}) };
  });
  let returned = false;
  const pending = dispatchDeliveryReady(f.harness, manifest,
    async (candidate, _mode, id, owner, signal) => (await runDeliveryOutcome(candidate, id, owner, { signal })).success,
    undefined, event => {
      progress.push(event);
      if (event.event === 'outcome-started' && event.taskId === 'ready-2') refilled();
    }).then(result => { returned = true; return result; });
  try {
    await nextStarted;
    expect(returned).toBe(false);
    expect(progress.find(event => event.event === 'outcome-settled' && event.taskId === 'ready-0'))
      .toMatchObject({ status: failure ? 'rejected' : 'fulfilled', candidateRoot: expect.any(String), evidenceDirectory: expect.any(String) });
    expect(progress.some(event => event.event === 'outcome-settled' && event.taskId === 'ready-1')).toBe(false);
    expect(progress.some(event => event.event === 'cohort-drained')).toBe(false);
    expect(f.harness.inspect().operation).not.toBeNull();
    for (const event of progress) expect(JSON.parse(readFileSync(event.evidencePath, 'utf8'))).toEqual(event);
  } finally { release(); }
  const result = await pending;
  expect(result.results.map(row => row.status)).toEqual([failure ? 'rejected' : 'fulfilled', 'fulfilled', 'fulfilled']);
  expect(progress.at(-1)).toMatchObject({ event: 'cohort-drained', sourceRevalidated: true,
    integration: 'await-cohort-return-and-owner-acceptance' });
  expect(f.harness.inspect()).toEqual({ active: null, operation: null });
});

it('keeps durable progress independent of observer errors and refuses canonical source drift', async () => {
  const f = fixture(), events: DeliveryPoolProgress[] = [];
  await expect(dispatchDeliveryReady(f.harness, { ...f.manifest, outcomes: [f.manifest.outcomes[0]] }, async () => {
    writeFileSync(join(f.root, 'product.txt'), 'unexpected drift'); return true;
  }, undefined, async event => { events.push(structuredClone(event)); event.sourceDigest = 'observer mutation'; throw new Error('observer'); }))
    .rejects.toThrow('DELIVERY_POOL_CANONICAL_SOURCE_CHANGED');
  expect(events.at(-1)).toMatchObject({ event: 'cohort-drained', sourceRevalidated: false });
  for (const event of events) expect(JSON.parse(readFileSync(event.evidencePath, 'utf8'))).toEqual(event);
  expect(f.harness.inspect().operation).toBeNull();
});

it('ordinary ready run reaches complete candidate lifecycle through CLI without model calls', async () => {
  const f = fixture(), before = f.harness.snapshot().digest;
  vi.spyOn(executor, 'createDeliveryExecutor').mockReturnValue(async request => ({
    response: { schemaVersion: 1, requestId: request.id, sourceDigest: request.sourceDigest,
      native: { ...native, ...request.route, executorId: request.executorId ?? 'fresh-reviewer' }, outcome: 'completed', summary: 'Injected CLI executor', issues: [] },
    changes: request.stage === 'implementation' ? [{ path: request.scope[0], content: 'fixed\n' }] : [],
    ...(request.stage === 'architecture' ? { plan: { summary: 'Fix admitted source', files: request.scope, tests: ['build', 'public'] } } : {}),
  }));
  const manifest = join(f.parentDirectory, 'run.json'); writeFileSync(manifest, JSON.stringify({ ...f.manifest, mode: 'run' }));
  const output = vi.spyOn(console, 'log').mockImplementation(() => {});
  const success = await deliveryCli([f.root, 'ready', manifest]);
  expect(success, JSON.stringify(output.mock.calls)).toBe(true);
  const result = JSON.parse(output.mock.calls.at(-1)![0]);
  expect(result.results.every((row: { value: { status: string } }) => row.value.status === 'candidate-awaiting-integration')).toBe(true);
  expect(f.harness.snapshot().digest).toBe(before); expect(f.harness.inspect().active).toBeNull();
});

it('ordinary CLI prepares independent source-bound packets through the upstream pool', async () => {
  const f = fixture(), before = f.harness.snapshot().digest;
  const manifestPath = join(f.parentDirectory, 'ready.json'); writeFileSync(manifestPath, JSON.stringify(f.manifest));
  const output = vi.spyOn(console, 'log').mockImplementation(() => {});
  expect(await deliveryCli([f.root, 'ready', manifestPath])).toBe(true);
  const result = JSON.parse(output.mock.calls.at(-1)![0]);
  expect(result.results).toHaveLength(2);
  for (const row of result.results) {
    expect(row.status).toBe('fulfilled'); expect(row.value.status).toBe('awaiting-executor');
    const packets = readdirSync(row.evidenceDirectory).filter(name => name.startsWith('packet-'));
    expect(packets).toHaveLength(1);
    const packet = JSON.parse(readFileSync(join(row.evidenceDirectory, packets[0]), 'utf8'));
    expect(packet.request.taskId).toBe(row.id); expect(packet.request.sourceDigest).toBe(before);
  }
  expect(f.harness.snapshot().digest).toBe(before); expect(f.harness.inspect().active).toBeNull();
});

it('ready dispatch overlaps callbacks, drains cancellation, and retains candidates without accepting them', async () => {
  const f = fixture(), controller = new AbortController(); let count = 0, release!: () => void, started!: () => void;
  const events: DeliveryPoolProgress[] = [];
  const barrier = new Promise<void>(resolve => { started = resolve; });
  const end = new Promise<void>(resolve => { release = resolve; });
  let settled = false;
  const pending = dispatchDeliveryReady(f.harness, f.manifest, async candidate => {
    expect(candidate.context.kind).toBe('candidate'); if (++count === 2) started(); await end; return true;
  }, controller.signal, event => { events.push(event); }).then(result => { settled = true; return result; });
  await barrier; controller.abort(); await new Promise(resolve => setImmediate(resolve));
  expect(settled).toBe(false); expect(f.harness.inspect().operation).not.toBeNull();
  expect(events.some(event => event.event === 'outcome-settled' || event.event === 'cohort-drained')).toBe(false);
  release(); const result = await pending;
  expect(result.results.every(row => row.status === 'cancelled' && row.candidateRoot)).toBe(true);
  expect(events.filter(event => event.event === 'outcome-settled').map(event => event.status)).toEqual(['cancelled', 'cancelled']);
  expect(f.harness.inspect().operation).toBeNull();
});

it('rejects conflicts before candidate creation and blocks nonaccepted dependencies', async () => {
  const f = fixture(), execute = vi.fn(async () => true);
  f.manifest.outcomes[1].resources = f.manifest.outcomes[0].resources;
  await expect(dispatchDeliveryReady(f.harness, f.manifest, execute)).rejects.toThrow('RESOURCE_CONFLICT');
  expect(readdirSync(f.parentDirectory)).toEqual([]); expect(execute).not.toHaveBeenCalled();
  f.manifest.outcomes[1].resources = ['independent'];
  await f.harness.begin(f.task); await f.harness.pause(f.task.id, f.task.owner, 'fixture hold');
  const result = await dispatchDeliveryReady(f.harness, { ...f.manifest, outcomes: [{ ...f.manifest.outcomes[0], acceptedParent: f.task.id }] }, execute);
  expect(result.results[0]).toMatchObject({ status: 'rejected', error: 'DELIVERY_ACCEPTED_MAIN_SOURCE_REQUIRED' });
  expect(execute).not.toHaveBeenCalled();
});

it('rejects malformed concurrency and native proposal manifests before execution', async () => {
  const f = fixture(), execute = vi.fn(async () => true);
  await expect(dispatchDeliveryReady(f.harness, { ...f.manifest, maxConcurrency: 0 }, execute)).rejects.toThrow('INVALID_READY_MANIFEST');
  await expect(dispatchDeliveryReady(f.harness, { ...f.manifest, mode: 'propose' }, execute)).rejects.toThrow('PENDING_API_REQUEST_REQUIRED');
  expect(execute).not.toHaveBeenCalled();
});

it('pre-abort creates no candidates and mismatched handoff retains failure without canonical writes', async () => {
  const f = fixture(), before = f.harness.snapshot().digest, execute = vi.fn(async () => true);
  const controller = new AbortController(); controller.abort(new Error('fixture stop'));
  await expect(dispatchDeliveryReady(f.harness, f.manifest, execute, controller.signal)).rejects.toThrow('fixture stop');
  expect(readdirSync(f.parentDirectory)).toEqual([]);
  const first = f.manifest.outcomes[0];
  const result = await dispatchDeliveryReady(f.harness, { ...f.manifest, outcomes: [{ ...first,
    handoff: { ...native, model: 'gpt-6-astra' } }] }, execute);
  expect(result.results[0]).toMatchObject({ status: 'rejected', error: 'DELIVERY_NATIVE_ROUTE_MISMATCH' });
  expect(result.results[0].candidateRoot).toBeTruthy(); expect(execute).not.toHaveBeenCalled();
  expect(f.harness.snapshot().digest).toBe(before);
  expect(f.harness.inspect()).toEqual({ active: null, operation: null });
});

it('resumes retained candidates across real CLI processes through checks and independent review', () => {
  const f = fixture(), entry = fileURLToPath(new URL('../dist/delivery-cli.js', import.meta.url));
  const run = (root: string, command: string, ...args: string[]) => JSON.parse(execFileSync(process.execPath,
    [entry, root, command, ...args], { encoding: 'utf8' }));
  const manifestPath = join(f.parentDirectory, 'ready.json');
  writeFileSync(manifestPath, JSON.stringify({ ...f.manifest, outcomes: [f.manifest.outcomes[0]] }));
  // ready emits packet metadata followed by the cohort result.
  const processResult = spawnSync(process.execPath, [entry, f.root, 'ready', manifestPath], { encoding: 'utf8' });
  expect(processResult.status).toBe(0);
  const progress = processResult.stderr.trim().split('\n').map(line => JSON.parse(line));
  expect(progress.map(event => event.event)).toEqual(['cohort-started', 'outcome-started', 'candidate-created', 'outcome-settled', 'cohort-drained']);
  expect(progress.every(event => event.type === 'delivery-pool-progress' && existsSync(event.evidencePath))).toBe(true);
  const emitted = processResult.stdout;
  const cohort = JSON.parse(emitted.slice(emitted.indexOf('\n') + 1));
  const root = cohort.results[0].candidateRoot, task = f.manifest.outcomes[0].task;
  expect(run(root, 'status', task.id).status).toBe('active');
  for (const stage of ['implementation', 'review']) {
    const action = run(root, 'advance', task.id, task.owner);
    expect(action.request.stage).toBe(stage);
    const responsePath = join(f.parentDirectory, `${stage}.json`);
    writeFileSync(responsePath, JSON.stringify({ schemaVersion: 1, requestId: action.request.id,
      sourceDigest: action.request.sourceDigest, native: { ...native, executorId: stage === 'review' ? 'fresh-reviewer' : native.executorId },
      outcome: 'completed', summary: 'Injected deterministic subprocess fixture', issues: [] }));
    expect(run(root, 'submit', task.id, task.owner, responsePath).workflow.results.at(-1).accepted).toBe(true);
  }
  expect(run(root, 'verify', task.id, task.owner).verdict.pass).toBe(true);
  expect(run(root, 'advance', task.id, task.owner).kind).toBe('ready-to-commit');
  writeFileSync(join(f.root, 'product.txt'), 'canonical drift');
  expect(() => run(root, 'status', task.id)).toThrow();
});

it('submits API proposals after reopening with canonical accounting custody', async () => {
  const f = fixture(), route = { host: 'openrouter', model: 'deepseek/deepseek-v4.1-flash', effort: 'high' };
  const manifestPath = join(f.parentDirectory, 'ready.json');
  const first = f.manifest.outcomes[0];
  writeFileSync(manifestPath, JSON.stringify({ ...f.manifest, mode: 'propose', outcomes: [{ ...first,
    task: { ...first.task, ...{ host: route.host, requested: route } },
    handoff: { ...route, executorId: 'api-author', authentication: 'openrouter-api', observation: 'Injected API fixture' } }] }));
  vi.stubEnv('OPENROUTER_API_KEY', 'fixture-key');
  vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ id: 'generation-fixture', model: route.model,
    usage: { cost: 0.01, prompt_tokens: 100, completion_tokens: 50 }, choices: [{ finish_reason: 'stop', message: {
      content: JSON.stringify({ outcome: 'completed', summary: 'Injected proposal', issues: [], changes: [] }) } }] }))));
  const output = vi.spyOn(console, 'log').mockImplementation(() => {});
  expect(await deliveryCli([f.root, 'ready', manifestPath])).toBe(true);
  const cohort = JSON.parse(output.mock.calls.at(-1)![0]), root = cohort.results[0].candidateRoot;
  const proposalPath = JSON.parse(output.mock.calls[0][0]).proposalPath;
  const proposal = JSON.parse(readFileSync(proposalPath, 'utf8'));
  expect(proposal.evidencePath.startsWith(join(f.harness.directory, 'api'))).toBe(true);
  expect(await deliveryCli([root, 'submit', first.task.id, first.task.owner, proposalPath])).toBe(true);
});
