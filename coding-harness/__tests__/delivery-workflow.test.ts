// SPDX-License-Identifier: MIT
import { readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { hash } from '@metaharness/harness';
import { DeliveryHarness } from '../src/delivery-runtime.js';
import { deliveryCli } from '../src/delivery-cli.js';
import { atomicJson, git } from '../src/delivery-workspace.js';
import { native, nextRequest, responseFor, workflowFixture } from './delivery-workflow-fixtures.js';

const roots: string[] = [];
afterEach(() => { vi.restoreAllMocks(); for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });
async function started(script?: string) {
  const f = workflowFixture(roots, script); await f.harness.begin(f.task); await f.harness.bind(f.task.id, 'root', native); return f;
}
async function implementation(harness: DeliveryHarness) {
  const request = await nextRequest(harness);
  expect(request.stage).toBe('implementation');
  return harness.submit('task-1', 'root', responseFor(harness, request));
}

describe('native-host delivery workflow', () => {
  it('drives actual commands, independent review and exact scoped commit with real stage kernels', async () => {
    const { root, harness } = await started(); const request = await nextRequest(harness);
    expect(harness.read('task-1').checks).toHaveLength(0);
    writeFileSync(join(root, 'product.txt'), 'implemented\n');
    await harness.submit('task-1', 'root', responseFor(harness, request));
    const review = await nextRequest(harness);
    expect(review.stage).toBe('review'); expect(review.prerequisiteDigests).toHaveLength(3);
    expect((await harness.verify('task-1', 'root')).verdict?.pass).toBe(false);
    await harness.submit('task-1', 'root', responseFor(harness, review));
    expect((await harness.advance('task-1', 'root')).kind).toBe('ready-to-commit');
    expect((await harness.verify('task-1', 'root')).verdict?.pass).toBe(true);
    git(root, 'add', 'product.txt'); git(root, '-c', 'core.hooksPath=/dev/null', 'commit', '-qm', 'scoped');
    const result = await harness.finish('task-1', 'root', git(root, 'rev-parse', 'HEAD'));
    expect(result.status).toBe('complete');
    expect(result.workflow!.results.map(r => [r.kernel.success, r.kernel.receiptsValid, r.kernel.steps.length])).toEqual([[true, true, 1], [true, true, 1]]);
    expect(harness.inspect().active).toBeNull();
  });
  it('returns a durable idempotent pending request without launching another host', async () => {
    const { root, harness } = await started(); const request = await nextRequest(harness);
    expect(await new DeliveryHarness(root).advance('task-1', 'root')).toEqual({ kind: 'native', request });
    expect(harness.read('task-1').workflow!.requests).toHaveLength(1);
    expect(harness.read('task-1').checks).toHaveLength(0);
  });
  it('passes actual check failure/log evidence into repair, stops dependents, and rechecks repaired source', async () => {
    const { root, harness } = await started("import {readFileSync} from 'node:fs'; if (!readFileSync('../product.txt','utf8').includes('fixed')) { console.error('expected fixed'); process.exit(1); }\n");
    await implementation(harness);
    const repair = await nextRequest(harness);
    expect(repair.stage).toBe('implementation'); expect(repair.feedback.join()).toContain('exit=1');
    expect(harness.read('task-1').checks.map(c => c.id)).toEqual(['build']);
    expect(readFileSync(harness.read('task-1').checks[0].stderr, 'utf8')).toContain('expected fixed');
    writeFileSync(join(root, 'product.txt'), 'fixed\n');
    await harness.submit('task-1', 'root', responseFor(harness, repair));
    const review = await nextRequest(harness); expect(review.stage).toBe('review');
    expect(harness.read('task-1').checks.map(c => c.passed)).toEqual([false, true, true]);
    await harness.submit('task-1', 'root', responseFor(harness, review));
    expect((await harness.verify('task-1', 'root')).verdict?.pass).toBe(true);
  });
  it('uses independent review findings as repair feedback and never reruns a failed kernel worker', async () => {
    const { root, harness } = await started(); await implementation(harness);
    const review = await nextRequest(harness);
    const rejected = await harness.submit('task-1', 'root', responseFor(harness, review,
      { outcome: 'changes-requested', summary: 'Boundary case missing', issues: ['Handle empty input'] }));
    expect(rejected.workflow!.results.at(-1)!.kernel.steps[0].status).toBe('failed');
    expect(rejected.workflow!.results.at(-1)!.kernel.poolSnapshot['independent-reviewer'].pulls).toBe(1);
    const repair = await nextRequest(harness); expect(repair.feedback).toContain('Handle empty input');
    writeFileSync(join(root, 'product.txt'), 'fixed empty input\n');
    await harness.submit('task-1', 'root', responseFor(harness, repair));
    const reReview = await nextRequest(harness); expect(reReview.id).not.toBe(review.id);
    await harness.submit('task-1', 'root', responseFor(harness, reReview));
    expect((await harness.verify('task-1', 'root')).verdict?.pass).toBe(true);
  });
  it('pauses no-progress repair until an explicit resume, without imposing subscription quotas', async () => {
    const { harness } = await started('process.exit(1);\n'); await implementation(harness);
    const repair = await nextRequest(harness);
    expect((await harness.submit('task-1', 'root', responseFor(harness, repair))).status).toBe('paused');
    expect((await harness.advance('task-1', 'root')).kind).toBe('paused');
    expect(harness.inspect().active).toBeNull();
    await harness.resume('task-1', 'root'); await harness.bind('task-1', 'root', native);
    expect((await nextRequest(harness)).feedback.join()).toContain('NO_PROGRESS');
  });
  it.each(['unavailable', 'cancelled'] as const)('persists %s and exact native error, with no fallback', async outcome => {
    const { harness } = await started(); const request = await nextRequest(harness);
    const run = await harness.submit('task-1', 'root', responseFor(harness, request,
      { outcome, summary: 'native client exact fixture error' }));
    expect(run.status).toBe('paused');
    expect(run.events.at(-1)?.reason).toBe('codex gpt-5.6-sol: native client exact fixture error');
    expect(run.checks).toHaveLength(0);
  });
  it('rejects another task/route, duplicate responses, and self review', async () => {
    const { harness } = await started(); const request = await nextRequest(harness);
    await expect(harness.submit('task-1', 'root', { ...responseFor(harness, request), requestId: 'a'.repeat(64) })).rejects.toThrow('PENDING_REQUEST');
    await expect(harness.submit('task-1', 'root', responseFor(harness, request,
      { native: { ...native, effort: 'high' } }))).rejects.toThrow('ROUTE_MISMATCH');
    await harness.submit('task-1', 'root', responseFor(harness, request));
    await expect(harness.submit('task-1', 'root', responseFor(harness, request))).rejects.toThrow('PENDING_REQUEST');
    const review = await nextRequest(harness);
    await expect(harness.submit('task-1', 'root', responseFor(harness, review, { native }))).rejects.toThrow('INDEPENDENT_REVIEW');
  });
  it('rejects source changes and rerun-check drift while a review is pending', async () => {
    const { root, harness } = await started(); await implementation(harness);
    const review = await nextRequest(harness);
    await harness.check('task-1', 'root', 'build');
    await expect(harness.submit('task-1', 'root', responseFor(harness, review))).rejects.toThrow('STALE_PREREQUISITES');
    const fresh = await nextRequest(harness); expect(fresh.id).not.toBe(review.id);
    writeFileSync(join(root, 'product.txt'), 'changed during review\n');
    await expect(harness.submit('task-1', 'root', responseFor(harness, fresh))).rejects.toThrow('STALE_REVIEW');
    expect((await nextRequest(harness)).stage).toBe('implementation');
    expect((await harness.verify('task-1', 'root')).verdict?.pass).toBe(false);
  });
  it('requires a new request after a review hold/resume and prevents hidden continuing work', async () => {
    const { harness } = await started(); const stale = await nextRequest(harness);
    await harness.pause('task-1', 'root', 'user review hold');
    await expect(harness.submit('task-1', 'root', responseFor(harness, stale))).rejects.toThrow('WRITER_MISMATCH');
    await harness.resume('task-1', 'root'); await harness.bind('task-1', 'root', native);
    await expect(harness.submit('task-1', 'root', responseFor(harness, stale))).rejects.toThrow('PENDING_REQUEST');
    expect((await nextRequest(harness)).id).not.toBe(stale.id);
  });
  it('cannot reuse an earlier pass when a newer check crashed without a result', async () => {
    const { harness } = await started(); await implementation(harness);
    const review = await nextRequest(harness); await harness.submit('task-1', 'root', responseFor(harness, review));
    const run = harness.read('task-1'); delete run.digest;
    run.events.push({ at: new Date().toISOString(), kind: 'check-start', reason: 'build:2' });
    run.digest = hash(run); atomicJson(join(harness.directory, 'task-1.json'), run);
    expect((await harness.verify('task-1', 'root')).verdict?.pass).toBe(false);
    expect(await harness.next('task-1', 'root')).toEqual({ kind: 'check', checkId: 'build' });
  });
  it('respects a declared Claude reviewer route and exposes next/submit via the real CLI', async () => {
    const { root, task, harness } = workflowFixture(roots);
    task.reviewer = { host: 'claude-code', model: 'sonnet', effort: 'default' };
    await harness.begin(task); await harness.bind('task-1', 'root', native); await implementation(harness);
    const review = await nextRequest(harness); expect(review.route).toEqual(task.reviewer);
    const spy = vi.spyOn(console, 'log').mockImplementation(() => {});
    expect(await deliveryCli([root, 'next', 'task-1', 'root'])).toBe(true);
    expect(JSON.parse(spy.mock.calls.at(-1)![0]).request.id).toBe(review.id);
    const responsePath = join(harness.directory, 'response.json'); writeFileSync(responsePath, JSON.stringify(responseFor(harness, review)));
    expect(await deliveryCli([root, 'submit', 'task-1', 'root', responsePath])).toBe(true);
    expect(await deliveryCli([root, 'advance', 'task-1', 'root'])).toBe(true);
    expect(JSON.parse(spy.mock.calls.at(-1)![0]).kind).toBe('ready-to-commit');
  });
  it.each(['unavailable', 'cancelled'] as const)('resumes %s review at review, without demanding unrelated source edits', async outcome => {
    const { harness } = await started(); await implementation(harness);
    const review = await nextRequest(harness);
    await harness.submit('task-1', 'root', responseFor(harness, review, { outcome, summary: 'exact native error' }));
    await harness.resume('task-1', 'root'); await harness.bind('task-1', 'root', native);
    const retried = await nextRequest(harness);
    expect(retried.stage).toBe('review'); expect(retried.route).toEqual(review.route);
    expect(retried.id).not.toBe(review.id); expect(retried.repair).toBe(false);
    expect(harness.read('task-1').checks).toHaveLength(2);
    await harness.submit('task-1', 'root', responseFor(harness, retried));
    expect((await harness.verify('task-1', 'root')).verdict?.pass).toBe(true);
  });
  it('revalidates repair prerequisites both on next and submit after a check rerun', async () => {
    const { harness } = await started('process.exit(1);\n'); await implementation(harness);
    const repair = await nextRequest(harness);
    await harness.check('task-1', 'root', 'build');
    await expect(harness.submit('task-1', 'root', responseFor(harness, repair))).rejects.toThrow('STALE_PREREQUISITES');
    const fresh = await nextRequest(harness);
    expect(fresh.id).not.toBe(repair.id); expect(fresh.feedback.join()).toContain('attempt 2');
    expect(harness.read('task-1').workflow!.invalidated).toContain(repair.id);
  });
  it('does not allow changes-requested to bypass a no-progress pause', async () => {
    const { harness } = await started('process.exit(1);\n'); await implementation(harness);
    const repair = await nextRequest(harness);
    const result = await harness.submit('task-1', 'root', responseFor(harness, repair,
      { outcome: 'changes-requested', summary: 'Still broken', issues: ['Still needs work'] }));
    expect(result.status).toBe('paused'); expect(result.events.at(-1)?.reason).toContain('NO_PROGRESS');
  });
  it('rejects whitespace executor aliases in bindings and review responses', async () => {
    const f = workflowFixture(roots); await f.harness.begin(f.task);
    await expect(f.harness.bind('task-1', 'root', { ...native, executorId: 'native-sol ' })).rejects.toThrow('INVALID_EXECUTOR_ID');
    await f.harness.bind('task-1', 'root', native); await implementation(f.harness);
    const review = await nextRequest(f.harness);
    await expect(f.harness.submit('task-1', 'root', responseFor(f.harness, review,
      { native: { ...native, executorId: '\tnative-sol' } }))).rejects.toThrow('INVALID_EXECUTOR_ID');
  });
  it('cannot verify an older completed review while an implementation request remains pending', async () => {
    const { root, harness } = await started(); await implementation(harness);
    const review = await nextRequest(harness); await harness.submit('task-1', 'root', responseFor(harness, review));
    writeFileSync(join(root, 'product.txt'), 'new source\n'); await nextRequest(harness);
    writeFileSync(join(root, 'product.txt'), 'before\n');
    expect((await harness.verify('task-1', 'root')).verdict?.pass).toBe(false);
  });
});
