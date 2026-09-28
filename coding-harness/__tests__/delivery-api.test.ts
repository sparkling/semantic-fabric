import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { hash } from '@metaharness/harness';
import { createDeliveryApi, DELIVERY_API_DEFAULTS, renderDeliveryPrompt } from '../src/delivery-api.js';
import { parseDeliveryHandoff, parseDeliveryTask, route, selectDeliveryRoute } from '../src/delivery-contracts.js';
import type { NativeStageRequest } from '../src/delivery-workflow-contracts.js';
import { verifyNativeStage } from '../src/delivery-stage.js';
import { deliveryCli } from '../src/delivery-cli.js';
import { DeliveryHarness } from '../src/delivery-runtime.js';
import { atomicJson, processIdentity, sourceSnapshot } from '../src/delivery-workspace.js';
import { nextRequest, responseFor, workflowFixture } from './delivery-workflow-fixtures.js';

const roots: string[] = [];
afterEach(() => { vi.restoreAllMocks(); for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });

const request = (): NativeStageRequest => ({ schemaVersion: 1, id: 'a'.repeat(64), taskId: 'task-api', thread: 'owner', baseCommit: 'b'.repeat(40),
  stage: 'implementation', attempt: 1, sourceDigest: 'c'.repeat(64), route: { host: 'openrouter', model: DELIVERY_API_DEFAULTS.model, effort: 'high' },
  executorId: 'api-author', prerequisiteDigests: [], evidenceDigest: 'd'.repeat(64), repair: false,
  requirement: 'Fix admitted source', scope: ['source.ts'], feedback: ['author-only feedback'] });
const files = [{ path: 'source.ts', content: 'before' }];
const valid = { outcome: 'completed', summary: 'proposal', issues: [], changes: [{ path: 'source.ts', content: 'after' }] };
const response = (content: unknown = valid, model = DELIVERY_API_DEFAULTS.model): Response => new Response(JSON.stringify({
  id: 'generation-1', model, usage: { cost: 0.01, prompt_tokens: 100, completion_tokens: 50 },
  choices: [{ finish_reason: 'stop', message: { content: typeof content === 'string' ? content : JSON.stringify(content) } }],
}));
function fixture(fetcher: typeof fetch) {
  const directory = mkdtempSync(join(tmpdir(), 'fabric-api-'));
  roots.push(directory);
  return { directory, invoke: createDeliveryApi({ directory, fetch: fetcher, apiKey: () => 'fake-key' }) };
}
describe('ordinary API transport', () => {
  it('defaults new tasks to API without rewriting explicit native assignments', () => {
    const input = { schemaVersion: 1, id: 'default-api', requirement: 'repair', owner: 'root', thread: 'root',
      taskClass: 'implementation', scope: ['source.ts'], checks: [
        { id: 'test', kind: 'acceptance', argv: ['npm', 'test'], cwd: 'coding-harness' },
        { id: 'build', kind: 'build', argv: ['npm', 'run', 'build'], cwd: 'coding-harness' }] };
    expect(selectDeliveryRoute(parseDeliveryTask(input))).toEqual(request().route);
    expect(selectDeliveryRoute(parseDeliveryTask({ ...input, host: 'claude-code' })).host).toBe('claude-code');
  });
  it('admits explicit API identity and configured native aliases without faking native authentication', () => {
    expect(route({ host: 'claude-code', model: 'cc/claude-opus-5-5[1m]', effort: 'high' }).host).toBe('claude-code');
    expect(parseDeliveryHandoff({ ...request().route, executorId: 'api', authentication: 'openrouter-api', observation: 'actual' }).host).toBe('openrouter');
    expect(() => parseDeliveryHandoff({ ...request().route, executorId: 'api', authentication: 'native-subscription', observation: 'wrong' })).toThrow();
    expect(() => route({ host: 'claude-code', model: DELIVERY_API_DEFAULTS.model, effort: 'high' })).toThrow();
  });
  it('bounds real dispatch, preserves known cost and exact snapshot, and refuses replay', async () => {
    let calls = 0;
    const f = fixture(async (_url, options) => {
      calls++; const body = JSON.parse(String(options?.body));
      expect(body.max_tokens).toBe(131072); expect(body.reasoning).toEqual({ effort: 'high', exclude: true });
      expect(body.provider.max_price).toEqual({ prompt: 0.5, completion: 2 });
      return response(valid, 'deepseek/deepseek-v4.1-flash-20260910');
    });
    const out = await f.invoke(request(), files, [], 'task-digest');
    expect(out.evidence.actualUsd).toBe(0.01); expect(out.evidence.inputTokens).toBe(100);
    expect(out.evidence.maximumUsd).toBeLessThanOrEqual(1); expect(DELIVERY_API_DEFAULTS.maxTotalUsd).toBeNull();
    expect(out.response.native.authentication).toBe('openrouter-api');
    expect(out.response.metering?.costUsd).toBe(0.01);
    const stage = await verifyNativeStage(request(), out.response, []);
    expect(stage.kernel.totalCostUsd).toBe(0.01);
    await expect(f.invoke(request(), files, [], 'task-digest')).rejects.toThrow('request-replay-refused');
    await expect(f.invoke(request(), [{ path: 'source.ts', content: 'different prompt' }], [{ passed: false }], 'task-digest')).rejects.toThrow('request-replay-refused');
    expect(calls).toBe(1);
    expect(readdirSync(f.directory).some(p => readFileSync(join(f.directory, p), 'utf8').includes('fake-key'))).toBe(false);
  });
  it('completed malformed output holds this task, preserves usage and leaves another task eligible', async () => {
    let calls = 0; const f = fixture(async () => ++calls === 1 ? response('invalid JSON') : response());
    await expect(f.invoke(request(), files, [], 'task-a')).rejects.toMatchObject({ code: 'completed-invalid-output', evidence: { actualUsd: 0.01 } });
    await expect(f.invoke(request(), files, [], 'task-a')).rejects.toMatchObject({ code: 'task-output-held', evidence: { actualUsd: 0.01 } });
    expect((await f.invoke(request(), files, [], 'task-b')).changes).toEqual(valid.changes);
  });
  it('rejects contradictory changes-requested edits before admitting paid output', async () => {
    const f = fixture(async () => response({ ...valid, outcome: 'changes-requested', issues: ['Source needs proposed edits'] }));
    await expect(f.invoke(request(), files, [], 'contradictory-task')).rejects.toMatchObject({
      code: 'completed-invalid-output', evidence: { actualUsd: 0.01, providerRequestId: 'generation-1' },
    });
    await expect(f.invoke(request(), files, [], 'contradictory-task')).rejects.toThrow('task-output-held');
  });
  it('distinguishes credit, auth and unknown outcomes without accepting unsafe native fallback', async () => {
    for (const [status, code] of [[402, 'confirmed-credit-rejection'], [401, 'authentication-rejected']] as const) {
      const f = fixture(async () => new Response(JSON.stringify({ error: { code: status } }), { status }));
      await expect(f.invoke(request(), files, [], 'task')).rejects.toMatchObject({ code, evidence: { actualUsd: 0 } });
    }
    const f = fixture(async () => { throw new Error('network lost'); });
    await expect(f.invoke(request(), files, [], 'task-a')).rejects.toThrow('completion-unknown');
    await expect(f.invoke(request(), files, [], 'task-b')).rejects.toThrow('completion-unknown-held');
  });
  it('validates source scope and preserves accounting on unexpected resolved models', async () => {
    const f = fixture(async () => response({ ...valid, changes: [{ path: '../escape', content: 'no' }] }));
    await expect(f.invoke(request(), files, [], 'task')).rejects.toMatchObject({ code: 'completed-invalid-output', evidence: { actualUsd: 0.01 } });
    const g = fixture(async () => response(valid, 'unrelated/model'));
    await expect(g.invoke(request(), files, [], 'task')).rejects.toMatchObject({ code: 'completed-model-mismatch', evidence: { actualUsd: 0.01 } });
    await expect(g.invoke({ ...request(), id: 'e'.repeat(64) }, files, [], 'task')).rejects.toMatchObject({ code: 'completed-model-mismatch', evidence: { actualUsd: 0.01 } });
  });
  it('keeps confirmed charged HTTP errors infrastructure-only and unrelated tasks eligible', async () => {
    for (const status of [402, 503]) {
      let calls = 0;
      const f = fixture(async () => ++calls === 1 ? new Response(JSON.stringify({
        id: 'known-http-generation', model: 'deepseek/deepseek-v4.1-flash-20260910',
        usage: { cost: 0.01, prompt_tokens: 100, completion_tokens: 0 }, error: { code: status },
      }), { status }) : response());
      await expect(f.invoke(request(), files, [], 'charged-http-task')).rejects.toMatchObject({
        code: 'completed-http-error', evidence: { status: 'completed-http-error', actualUsd: 0.01,
          providerRequestId: 'known-http-generation', resolvedModel: 'deepseek/deepseek-v4.1-flash-20260910' } });
      expect(readdirSync(f.directory)).not.toContain('unknown-charge.json');
      await expect(f.invoke({ ...request(), id: 'e'.repeat(64) }, files, [], 'charged-http-task')).rejects.toThrow('completed-http-error');
      expect((await f.invoke(request(), files, [], 'unrelated-http-task')).evidence.actualUsd).toBe(0.01);
      expect(calls).toBe(2);
    }
  });
  it('does not dispatch oversized/pre-aborted packets and keeps review context independent', async () => {
    let calls = 0; const f = fixture(async () => { calls++; return response(); });
    await expect(f.invoke(request(), [{ path: 'source.ts', content: 'x'.repeat(2000000) }], [], 'task')).rejects.toThrow('request-cost-bound');
    const controller = new AbortController(); controller.abort();
    await expect(f.invoke(request(), files, [], 'task', controller.signal)).rejects.toThrow('cancelled-before-dispatch');
    expect(calls).toBe(0);
    expect(renderDeliveryPrompt({ ...request(), stage: 'review' }, files, [{ passed: true }])).not.toContain('author-only feedback');
  });
  it('retains crash custody across fresh request identities and permits live concurrent dispatch', async () => {
    const crashed = fixture(async () => { throw new Error('must not dispatch'); });
    writeFileSync(join(crashed.directory, 'request-abandoned.json'), JSON.stringify({ status: 'reserved', owner: { pid: process.pid, start: 'stale-process' } }));
    await expect(crashed.invoke({ ...request(), id: 'e'.repeat(64) }, files, [], 'new-task')).rejects.toThrow('completion-unknown-held');
    let calls = 0;
    let finish!: () => void;
    const gate = new Promise<void>(resolve => { finish = resolve; });
    const active = fixture(async () => { calls++; await gate; return response(); });
    const first = active.invoke(request(), files, [], 'task-a');
    const second = active.invoke({ ...request(), id: 'f'.repeat(64) }, files, [], 'task-b');
    expect(calls).toBe(2);
    finish(); await Promise.all([first, second]);
    expect(readdirSync(active.directory)).not.toContain('unknown-charge.json');
  });
  it('does not treat zombie or dead process start times as live custody', () => {
    for (const state of ['Z', 'X', 'x']) {
      expect(processIdentity(42, () => `42 (worker) ${[state, ...Array(18).fill('0'), '1234'].join(' ')}`)).toBeUndefined();
    }
    expect(processIdentity(42, () => `42 (worker) ${['S', ...Array(18).fill('0'), '1234'].join(' ')}`)).toBe('1234');
  });
  it('keeps interrupted completed validation paid and task-held without blocking another task', async () => {
    const f = fixture(async () => response());
    const first = await f.invoke(request(), files, [], 'task-a');
    atomicJson(first.evidencePath, { ...first.evidence, status: 'completed-awaiting-validation' });
    await expect(f.invoke({ ...request(), id: 'e'.repeat(64) }, files, [], 'task-a')).rejects.toMatchObject({
      code: 'completed-validation-interrupted', evidence: { actualUsd: 0.01 } });
    expect((await f.invoke(request(), files, [], 'task-b')).evidence.actualUsd).toBe(0.01);
  });
});

describe('source-bound native fallback', () => {
  const opus = { host: 'claude-code' as const, model: 'cc/claude-opus-5-5[1m]', effort: 'high' as const,
    executorId: 'native-opus-repair', authentication: 'native-subscription' as const, observation: 'actual native fixture' };
  async function started(script?: string, extraScope: string[] = []) {
    const f = workflowFixture(roots, script); f.task.host = 'openrouter'; f.task.scope.push(...extraScope);
    await f.harness.begin(f.task);
    await f.harness.bind(f.task.id, 'root', { ...request().route, executorId: 'api-author', authentication: 'openrouter-api', observation: 'API proposal executor' });
    const pending = await nextRequest(f.harness);
    const directory = join(f.harness.directory, 'api');
    return { ...f, pending, directory };
  }
  async function failed(fetcher: typeof fetch) {
    const f = await started(); const { directory, pending } = f;
    await expect(createDeliveryApi({ directory, fetch: fetcher, apiKey: () => 'fake' })(pending,
      [{ path: 'product.txt', content: 'before\n' }], [], hash(f.harness.read(f.task.id).task))).rejects.toThrow();
    const evidencePath = join(directory, readdirSync(directory).find(name => /^request-.*\.json$/.test(name))!);
    return { ...f, pending, evidencePath };
  }
  async function authored(content = 'fixed\n', script?: string, extraScope: string[] = []) {
    const f = await started(script, extraScope), sourceBefore = sourceSnapshot(f.root);
    const out = await createDeliveryApi({ directory: f.directory, apiKey: () => 'fake', fetch: async () => response({ ...valid,
      changes: [{ path: 'product.txt', content }] }) })(f.pending, [{ path: 'product.txt', content: 'before\n' }], [], hash(f.harness.read(f.task.id).task));
    const path = join(f.harness.directory, `proposal-${out.evidence.requestId}.json`);
    atomicJson(path, { ...out, sourceBefore });
    return { ...f, out, path, content };
  }
  it('binds exact root-applied proposal bytes through submit without a second paid dispatch', async () => {
    vi.spyOn(console, 'log').mockImplementation(() => {});
    const f = await authored('fixed\n', undefined, ['generated.json']);
    await expect(deliveryCli([f.root, 'submit', f.task.id, 'root', f.path])).rejects.toThrow('PROPOSAL_NOT_APPLIED');
    writeFileSync(join(f.root, 'product.txt'), f.content);
    writeFileSync(join(f.root, 'generated.json'), '{"rootGenerated":true}\n');
    expect(await deliveryCli([f.root, 'submit', f.task.id, 'root', f.path])).toBe(true);
    const result = f.harness.read(f.task.id).workflow!.results.at(-1)!;
    expect(result.response.rootApplication?.rootChangedPaths).toEqual(['generated.json']);
    expect(result.response.sourceDigest).toBe(sourceSnapshot(f.root).digest);
    expect(result.kernel.totalCostUsd).toBe(0.01);
    expect(readdirSync(f.directory).filter(name => name.startsWith('request-'))).toHaveLength(1);
  });
  it('routes failed candidate checks to capable native repair without charging another API request', async () => {
    vi.spyOn(console, 'log').mockImplementation(() => {});
    const f = await authored('bad\n', "import {readFileSync} from 'node:fs'; process.exit(readFileSync('../product.txt','utf8').includes('fixed') ? 0 : 1);\n");
    writeFileSync(join(f.root, 'product.txt'), f.content);
    await deliveryCli([f.root, 'submit', f.task.id, 'root', f.path]);
    const repair = await nextRequest(f.harness); expect(repair.repair).toBe(true);
    await expect(deliveryCli([f.root, 'propose', f.task.id, 'root'])).rejects.toThrow('CAPABLE_NATIVE_REPAIR_REQUIRED');
    const log = f.harness.read(f.task.id).checks.at(-1)!.stderr, original = readFileSync(log);
    writeFileSync(log, 'changed evidence');
    await expect(f.harness.repair(f.task.id, 'root', opus)).rejects.toThrow('STALE_PREREQUISITES');
    writeFileSync(log, original);
    await f.harness.repair(f.task.id, 'root', opus);
    const native = await nextRequest(f.harness); expect(native.route.model).toBe(opus.model);
    writeFileSync(join(f.root, 'product.txt'), 'fixed\n');
    const run = await f.harness.submit(f.task.id, 'root', responseFor(f.harness, native));
    expect(run.workflow!.results.map(result => result.kernel.totalCostUsd)).toEqual([0.01, 0]);
  });
  it('retains original check custody through two rejected repairs', async () => {
    vi.spyOn(console, 'log').mockImplementation(() => {});
    const f = await authored('bad\n', 'process.exit(1);\n');
    writeFileSync(join(f.root, 'product.txt'), f.content);
    await deliveryCli([f.root, 'submit', f.task.id, 'root', f.path]);
    await nextRequest(f.harness);
    const log = f.harness.read(f.task.id).checks.at(-1)!.stderr;
    await f.harness.repair(f.task.id, 'root', opus);
    for (let attempt = 0; attempt < 2; attempt++) {
      const repair = await nextRequest(f.harness);
      writeFileSync(join(f.root, 'product.txt'), `still bad ${attempt}\n`);
      await f.harness.submit(f.task.id, 'root', responseFor(f.harness, repair,
        { outcome: 'changes-requested', summary: 'Repair remains incomplete', issues: ['not fixed'] }));
    }
    writeFileSync(log, 'tampered original check');
    await expect(nextRequest(f.harness)).rejects.toThrow('STALE_PREREQUISITES');
  });
  it('repairs malformed output with Opus, preserves paid failure and joins fresh API review', async () => {
    const f = await failed(async () => response('invalid JSON'));
    await f.harness.fallback(f.task.id, 'root', f.pending.id, f.evidencePath, opus);
    const repair = await nextRequest(f.harness);
    expect(repair.route).toMatchObject({ model: opus.model, effort: 'high' });
    expect(repair.failedApi?.actualUsd).toBe(0.01);
    expect(f.harness.read(f.task.id).workflow?.invalidated).toContain(f.pending.id);
    writeFileSync(join(f.root, 'product.txt'), 'fixed\n');
    const implemented = await f.harness.submit(f.task.id, 'root', responseFor(f.harness, repair));
    expect(implemented.workflow?.results.at(-1)?.kernel.totalCostUsd).toBe(0.01);
    const review = await nextRequest(f.harness);
    expect(review.route).toEqual(request().route);
    const proposal = await createDeliveryApi({ directory: join(f.harness.directory, 'api'), fetch: async () => response({ ...valid, changes: [] }), apiKey: () => 'fake' })(
      review, [{ path: 'product.txt', content: 'fixed\n' }], [], hash(f.harness.read(f.task.id).task));
    await f.harness.submit(f.task.id, 'root', proposal.response);
    expect((await f.harness.verify(f.task.id, 'root')).verdict?.pass).toBe(true);
  });
  it('permits medium Sol for confirmed402, never Opus solely for credit exhaustion', async () => {
    const f = await failed(async () => new Response(JSON.stringify({ error: { code: 402 } }), { status: 402 }));
    await expect(f.harness.fallback(f.task.id, 'root', f.pending.id, f.evidencePath, opus)).rejects.toThrow('FALLBACK_NOT_AUTHORIZED');
    await expect(f.harness.fallback(f.task.id, 'root', f.pending.id, f.evidencePath,
      { ...opus, model: 'cc/claude-sonnet-4-6', effort: 'medium' })).rejects.toThrow('FALLBACK_NOT_AUTHORIZED');
    await f.harness.fallback(f.task.id, 'root', f.pending.id, f.evidencePath,
      { ...opus, host: 'codex', model: 'gpt-5.6-sol', effort: 'medium', executorId: 'native-credit-sol' });
    expect((await nextRequest(f.harness)).route.model).toBe('gpt-5.6-sol');
  });
  it('refuses auth, unknown completion, model-integrity and stale-source fallback', async () => {
    for (const fetcher of [async () => new Response(JSON.stringify({ error: { code: 401 } }), { status: 401 }),
      async () => { throw new Error('network lost'); }, async () => response(valid, 'unrelated/model')]) {
      const f = await failed(fetcher);
      await expect(f.harness.fallback(f.task.id, 'root', f.pending.id, f.evidencePath, opus)).rejects.toThrow();
    }
    const f = await failed(async () => response('invalid JSON'));
    writeFileSync(join(f.root, 'product.txt'), 'drift\n');
    await expect(f.harness.fallback(f.task.id, 'root', f.pending.id, f.evidencePath, opus)).rejects.toThrow('PENDING_API_REQUEST');
    const fresh = await nextRequest(f.harness); expect(fresh.id).not.toBe(f.pending.id);
    expect(fresh.sourceDigest).toBe(sourceSnapshot(f.root).digest);
    vi.spyOn(DeliveryHarness.prototype, 'next').mockResolvedValue({ kind: 'native', request: f.pending });
    await expect(deliveryCli([f.root, 'packet', f.task.id, 'root'])).rejects.toThrow('API_SOURCE_CHANGED');
    await expect(deliveryCli([f.root, 'propose', f.task.id, 'root'])).rejects.toThrow('API_SOURCE_CHANGED');
  });
});
