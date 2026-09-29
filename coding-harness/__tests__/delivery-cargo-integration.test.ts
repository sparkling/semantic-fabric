import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, expect, it } from 'vitest';
import { dispatchDeliveryReady } from '../src/delivery-ready.js';
import { runDeliveryOutcome } from '../src/delivery-runner.js';
import { runIntegrationReview } from '../src/delivery-integration-review.js';
import { requiredDeliveryInputs } from '../src/delivery-lineage.js';
import { git } from '../src/delivery-workspace.js';
import type { DeliveryHarness } from '../src/delivery-runtime.js';
import type { DeliveryPoolProgress } from '../src/delivery-pool.js';
import type { DeliveryExecutor } from '../src/delivery-executor.js';
import { native, workflowFixture } from './delivery-workflow-fixtures.js';

const roots: string[] = [];
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });
const deferred = () => { let resolve!: () => void; const promise = new Promise<void>(yes => { resolve = yes; }); return { promise, resolve }; };
function fixture(conflict = false, readsA = false) {
  const f = workflowFixture(roots), parentDirectory = mkdtempSync(join(tmpdir(), 'cargo-candidates-')); roots.push(parentDirectory);
  mkdirSync(join(f.root, 'src'));
  writeFileSync(join(f.root, '.gitignore'), '.metaharness/\ntarget/\n');
  writeFileSync(join(f.root, 'Cargo.toml'), '[package]\nname="delivery-fixture"\nversion="0.1.0"\nedition="2021"\n');
  writeFileSync(join(f.root, 'Cargo.lock'), 'version = 4\n\n[[package]]\nname = "delivery-fixture"\nversion = "0.1.0"\n');
  writeFileSync(join(f.root, 'src/lib.rs'), `pub mod a; pub mod b; pub mod child;\n#[test] fn combined() { assert!(${conflict
    ? 'a::value() + b::value() <= 3' : 'a::value() > 0 && b::value() > 0 && child::value() > 0'}); }\n`);
  for (const name of ['a', 'b', 'child']) writeFileSync(join(f.root, `src/${name}.rs`), 'pub fn value() -> u32 { 1 }\n');
  git(f.root, 'add', '.'); git(f.root, 'commit', '-qm', 'real Rust fixture');
  const outcomes = ['a', 'b'].map((name, i) => ({ task: { ...f.task, id: `cargo-${name}`, scope: [`src/${name}.rs`],
    readPaths: ['src/lib.rs', ...(i === 1 && readsA ? ['src/a.rs'] : [])],
    requested: { host: native.host, model: native.model, effort: native.effort }, selectionReason: 'Fixture pinned route',
    checks: [{ id: 'build', kind: 'build', argv: ['cargo', 'check', '--lib', '--offline', '--locked', '--target-dir', 'target'], cwd: '.' },
      { id: 'public', kind: 'acceptance', argv: ['cargo', 'test', '--lib', '--offline', '--locked', '--target-dir', 'target'], cwd: '.' }] },
    handoff: native, resources: [`private-${name}`] }));
  return { ...f, manifest: { schemaVersion: 1, parentDirectory, mode: 'run', maxConcurrency: 2, outcomes } };
}
const execute: DeliveryExecutor = async request => ({
  response: { schemaVersion: 1, requestId: request.id, sourceDigest: request.sourceDigest,
    native: { ...native, ...request.route, executorId: request.executorId ?? 'candidate-reviewer' },
    outcome: 'completed', summary: 'Injected native stage; real Cargo checks', issues: [] },
  changes: request.stage === 'implementation' ? [{ path: request.scope[0], content: 'pub fn value() -> u32 { 2 }\n' }] : [],
  ...(request.stage === 'architecture' ? { plan: { summary: 'Scoped fixture', files: request.scope, tests: ['build', 'public'] } } : {}),
});
const candidateRun = async (h: DeliveryHarness, _mode: string, id: string, owner: string, signal: AbortSignal) =>
  (await runDeliveryOutcome(h, id, owner, { execute, signal })).success;
function input(event: DeliveryPoolProgress) {
  const original = JSON.parse(readFileSync(join(event.evidenceDirectory!, `${event.taskId}.json`), 'utf8'));
  return { candidateRoot: event.candidateRoot, id: event.taskId, owner: original.task.owner, expectedDigest: original.digest };
}
async function finish(h: DeliveryHarness, id: string) {
  const run = h.read(id);
  expect((await h.verify(id, run.task.owner)).verdict?.pass).toBe(true);
  git(h.root, 'add', '--', ...run.task.scope); git(h.root, 'commit', '-qm', 'accept fixture');
  await h.finish(id, run.task.owner, git(h.root, 'rev-parse', 'HEAD'));
}
async function accept(h: DeliveryHarness, event: DeliveryPoolProgress) {
  const run = await h.integrate(input(event));
  expect((await runIntegrationReview(h, run.task.id, run.task.owner, { execute: async (...args) => {
    expect(args[0].stage).toBe('review');
    const result = await execute(...args); result.response.native.executorId = 'current-source-reviewer'; return result;
  } })).success).toBe(true);
  await finish(h, run.task.id);
}
it('accepts/refills real Cargo same-crate lanes early, then rechecks preserved sibling without author/planner replay', async () => {
  const f = fixture(), parent = deferred(), release = deferred(), events: DeliveryPoolProgress[] = [];
  const pending = dispatchDeliveryReady(f.harness, f.manifest, async (h, mode, id, owner, signal) => {
    const result = await candidateRun(h, mode, id, owner, signal);
    if (id === 'cargo-b') await release.promise;
    return result;
  }, undefined, event => { events.push(event); if (event.event === 'outcome-settled' && event.taskId === 'cargo-a') parent.resolve(); });
  try {
    await parent.promise;
    await accept(f.harness, events.find(event => event.event === 'outcome-settled' && event.taskId === 'cargo-a')!);
    expect(events.some(event => event.event === 'cohort-drained')).toBe(false);
    const child = { ...f.manifest.outcomes[0], task: { ...f.manifest.outcomes[0].task, id: 'cargo-child', scope: ['src/child.rs'] },
      resources: ['private-child'], acceptedParent: 'cargo-a', acceptedInputs: ['src/a.rs'] };
    const childEvents: DeliveryPoolProgress[] = [];
    const childResult = await dispatchDeliveryReady(f.harness, { ...f.manifest, outcomes: [child] }, candidateRun, undefined, event => childEvents.push(event));
    expect(childResult.results[0].status).toBe('fulfilled');
    await accept(f.harness, childEvents.find(event => event.event === 'outcome-settled')!);
  } finally { release.resolve(); }
  expect((await pending).results.every(result => result.status === 'fulfilled')).toBe(true);
  const event = events.find(event => event.event === 'outcome-settled' && event.taskId === 'cargo-b')!;
  const originalPath = join(event.evidenceDirectory!, 'cargo-b.json'), original = readFileSync(originalPath, 'utf8');
  await f.harness.integrate(input(event));
  for (const check of f.harness.read('cargo-b').task.checks) await f.harness.check('cargo-b', 'root', check.id);
  expect((await f.harness.verify('cargo-b', 'root')).verdict?.pass).toBe(false);
  const stages: string[] = [];
  const result = await runIntegrationReview(f.harness, 'cargo-b', 'root', { execute: async (request, files, checks, signal) => {
    stages.push(request.stage);
    expect(files.find(file => file.path === 'src/a.rs')?.content).toContain('{ 2 }');
    expect(files.find(file => file.path === 'src/child.rs')?.content).toContain('{ 2 }');
    const result = await execute(request, files, checks, signal); result.response.native.executorId = 'current-source-reviewer'; return result;
  } });
  expect(result.success).toBe(true); expect(stages).toEqual(['review']);
  expect(readFileSync(originalPath, 'utf8')).toBe(original);
  await finish(f.harness, 'cargo-b');
}, 30_000);

it('rejects actual declared semantic dependency changes despite green old Cargo checks', async () => {
  const f = fixture(false, true), events: DeliveryPoolProgress[] = [];
  await dispatchDeliveryReady(f.harness, f.manifest, candidateRun, undefined, event => events.push(event));
  await accept(f.harness, events.find(event => event.event === 'outcome-settled' && event.taskId === 'cargo-a')!);
  await expect(f.harness.integrate(input(events.find(event => event.event === 'outcome-settled' && event.taskId === 'cargo-b')!)))
    .rejects.toThrow('INPUT_CHANGED');
}, 30_000);

it('fails current-source Cargo revalidation without invoking review or modifying preserved candidate evidence', async () => {
  const f = fixture(true), events: DeliveryPoolProgress[] = [];
  await dispatchDeliveryReady(f.harness, f.manifest, candidateRun, undefined, event => events.push(event));
  await accept(f.harness, events.find(event => event.event === 'outcome-settled' && event.taskId === 'cargo-a')!);
  const event = events.find(event => event.event === 'outcome-settled' && event.taskId === 'cargo-b')!;
  const originalPath = join(event.evidenceDirectory!, 'cargo-b.json'), original = readFileSync(originalPath, 'utf8');
  await f.harness.integrate(input(event));
  const result = await runIntegrationReview(f.harness, 'cargo-b', 'root', { execute: async () => { throw new Error('review must not run'); } });
  expect(result).toEqual({ success: false, failure: 'DELIVERY_INTEGRATION_CHECK_FAILED' });
  expect(f.harness.read('cargo-b').checks.at(-1)?.passed).toBe(false);
  expect(f.harness.read('cargo-b').status).toBe('paused');
  expect(readFileSync(originalPath, 'utf8')).toBe(original);
}, 30_000);

it('keeps Cargo manifests, toolchain/config, build scripts and harness evaluators as hard pins', () => {
  const paths = ['Cargo.toml', 'Cargo.lock', '.cargo/config.toml', 'rust-toolchain.toml', 'crates/a/build.rs',
    'coding-harness/check.mjs', 'tests/query.rq', 'src/a.rs'];
  expect(requiredDeliveryInputs(Object.fromEntries(paths.map(path => [path, 'digest'])), []))
    .toEqual(paths.slice(0, -1));
});

it('binds fresh review to new checks, refuses original reviewer identity, and preserves rejected review evidence', async () => {
  const f = fixture(), events: DeliveryPoolProgress[] = [];
  await dispatchDeliveryReady(f.harness, f.manifest, candidateRun, undefined, event => events.push(event));
  await accept(f.harness, events.find(event => event.event === 'outcome-settled' && event.taskId === 'cargo-a')!);
  const event = events.find(event => event.event === 'outcome-settled' && event.taskId === 'cargo-b')!;
  await f.harness.integrate(input(event));
  const action = await f.harness.advance('cargo-b', 'root');
  expect(action.kind).toBe('native'); if (action.kind !== 'native') throw new Error('review required');
  expect(f.harness.read('cargo-b').checks.map(check => [check.id, check.passed])).toEqual([['build', true], ['public', true]]);
  const result = await execute(action.request, [], []);
  await expect(f.harness.submit('cargo-b', 'root', result.response)).rejects.toThrow('FRESH_INTEGRATION_REVIEW');
  result.response.native.executorId = 'fresh-current-reviewer';
  await f.harness.check('cargo-b', 'root', 'public');
  await expect(f.harness.submit('cargo-b', 'root', result.response)).rejects.toThrow('STALE_PREREQUISITES');
  const rejected = await runIntegrationReview(f.harness, 'cargo-b', 'root', { execute: async (...args) => {
    const result = await execute(...args); result.response.native.executorId = 'fresh-current-reviewer';
    result.response.outcome = 'changes-requested'; result.response.issues = ['Combined semantic interaction needs repair'];
    return result;
  } });
  expect(rejected.success).toBe(false);
  const run = f.harness.read('cargo-b');
  expect(run.status).toBe('paused'); expect(run.workflow?.results.at(-1)?.accepted).toBe(false);
  expect(run.workflow?.results.at(-1)?.response.issues).toEqual(['Combined semantic interaction needs repair']);
  expect(run.integration?.original.verdict?.pass).toBe(true);
}, 30_000);
