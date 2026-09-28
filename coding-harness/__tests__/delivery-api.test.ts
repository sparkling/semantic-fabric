import { mkdtempSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { describe, expect, it } from 'vitest';
import { createDeliveryApi, DELIVERY_API_DEFAULTS, renderDeliveryPrompt } from '../src/delivery-api.js';
import { parseDeliveryHandoff, parseDeliveryTask, route, selectDeliveryRoute } from '../src/delivery-contracts.js';
import type { NativeStageRequest } from '../src/delivery-workflow-contracts.js';
import { verifyNativeStage } from '../src/delivery-stage.js';

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
    expect(JSON.stringify(stage.kernel)).toContain('0.01');
    await expect(f.invoke(request(), files, [], 'task-digest')).rejects.toThrow('request-replay-refused');
    expect(calls).toBe(1);
    expect(readdirSync(f.directory).some(p => readFileSync(join(f.directory, p), 'utf8').includes('fake-key'))).toBe(false);
  });
  it('completed malformed output holds this task, preserves usage and leaves another task eligible', async () => {
    let calls = 0; const f = fixture(async () => ++calls === 1 ? response('invalid JSON') : response());
    await expect(f.invoke(request(), files, [], 'task-a')).rejects.toMatchObject({ code: 'completed-invalid-output', evidence: { actualUsd: 0.01 } });
    await expect(f.invoke(request(), files, [], 'task-a')).rejects.toMatchObject({ code: 'task-output-held', evidence: { actualUsd: 0.01 } });
    expect((await f.invoke(request(), files, [], 'task-b')).changes).toEqual(valid.changes);
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
});
