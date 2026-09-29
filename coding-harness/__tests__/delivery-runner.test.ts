import { mkdtempSync, readFileSync, readdirSync, rmSync, writeSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, expect, it, vi } from 'vitest';
import { hash } from '@metaharness/harness';
import { createDeliveryCandidate } from '../src/delivery-candidate.js';
import { runDeliveryOutcome } from '../src/delivery-runner.js';
import type { DeliveryExecutor } from '../src/delivery-executor.js';
import type { NativeStageRequest } from '../src/delivery-workflow-contracts.js';
import { createDeliveryApi, DELIVERY_API_DEFAULTS, renderDeliveryPrompt } from '../src/delivery-api.js';
import { native, workflowFixture } from './delivery-workflow-fixtures.js';
import { DELIVERY_ROOT_POLICY } from '../src/delivery-policy.js';
import { claudeStructuredError, createDeliveryExecutor } from '../src/delivery-executor.js';
import * as processes from '../src/delivery-process.js';
import { selectDeliveryRoute } from '../src/delivery-contracts.js';

const roots: string[] = [];
afterEach(() => { vi.restoreAllMocks(); for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });
async function fixture(api = false, sonnet = false) {
  const f = workflowFixture(roots, "import {readFileSync} from 'node:fs'; process.exit(readFileSync('../product.txt','utf8') === 'fixed\\n' ? 0 : 1);\n");
  const parentDirectory = mkdtempSync(join(tmpdir(), 'fabric-runner-')); roots.push(parentDirectory);
  if (api) f.task.host = 'openrouter';
  if (sonnet) f.task.host = 'claude-code';
  const candidate = createDeliveryCandidate(f.harness, { parentDirectory, scope: f.task.scope });
  await candidate.harness.begin(f.task); await candidate.harness.bind(f.task.id, f.task.owner, api ? {
    host: 'openrouter', model: DELIVERY_API_DEFAULTS.model, effort: 'high', executorId: 'api-author', authentication: 'openrouter-api', observation: 'Injected API transport' } : sonnet ? { ...native, ...selectDeliveryRoute(f.task) } : native);
  return { ...f, candidate };
}
function result(request: NativeStageRequest, content = 'fixed\n') {
  return { response: { schemaVersion: 1 as const, requestId: request.id, sourceDigest: request.sourceDigest,
    native: { ...request.route, executorId: request.executorId ?? 'independent-reviewer',
      authentication: 'native-subscription' as const, observation: 'Injected executor' }, outcome: 'completed' as const,
    summary: 'Injected outcome', issues: [], metering: { costUsd: 0, latencyMs: 1, evidenceDigest: hash(request) } },
    changes: request.stage === 'implementation' ? [{ path: 'product.txt', content }] : [],
    ...(request.stage === 'architecture' ? { plan: { summary: 'Fix source', files: ['product.txt'], tests: ['build', 'public'] } } : {}) };
}

it.each([false, true])('executes plan, author, failed checks, capable repair and fresh review (Sonnet=%s)', async sonnet => {
  const f = await fixture(false, sonnet), events: string[] = [], before = f.harness.snapshot().digest;
  const execute: DeliveryExecutor = async (request, _files, context) => {
    events.push(request.stage === 'implementation' && request.repair ? 'repair' : request.stage);
    if (request.repair) expect(request.route).toMatchObject({ host: 'claude-code', model: 'cc/claude-opus-5-5[1m]', effort: 'high' });
    else if (sonnet) expect(request.route).toEqual({ host: 'claude-code', model: 'cc/claude-sonnet-5-5[1m]', effort: 'medium' });
    if (request.stage === 'review') {
      expect(context.some(value => typeof value === 'object' && value !== null && 'plan' in value)).toBe(false);
      expect(f.candidate.harness.read(f.task.id).checks.filter(check => check.sourceAfter === request.sourceDigest && check.passed)).toHaveLength(2);
    }
    return result(request, request.repair ? 'fixed\n' : 'bad\n');
  };
  const outcome = await runDeliveryOutcome(f.candidate.harness, f.task.id, f.task.owner, { execute });
  expect(outcome.success).toBe(true); expect(events).toEqual(['architecture', 'implementation', 'repair', 'review']);
  expect(outcome.status).toBe('candidate-awaiting-integration'); expect(outcome.kernel.receiptsValid).toBe(true);
  expect(f.harness.snapshot().digest).toBe(before); expect(f.harness.inspect().active).toBeNull();
  expect(readFileSync(join(f.root, 'product.txt'), 'utf8')).toBe('before\n');
  expect(readdirSync(join(f.harness.directory, 'learning', hash(DELIVERY_ROOT_POLICY), 'deltas'))).toHaveLength(1);
});

it('stops stale planner identity before authoring or checks', async () => {
  const f = await fixture(); let calls = 0;
  const outcome = await runDeliveryOutcome(f.candidate.harness, f.task.id, f.task.owner, { execute: async request => {
    calls++; return { ...result(request), response: { ...result(request).response, requestId: 'f'.repeat(64) } };
  } });
  expect(outcome.success).toBe(false); expect(outcome.failure).toBe('DELIVERY_EXECUTOR_IDENTITY_MISMATCH');
  expect(calls).toBe(1); expect(f.candidate.harness.read(f.task.id).checks).toEqual([]);
  expect(readdirSync(join(f.harness.directory, 'learning', hash(DELIVERY_ROOT_POLICY), 'deltas'))).toEqual([]);
});

it('retains exact native errors and stops without review, fallback or source writes', async () => {
  const f = await fixture(), before = f.candidate.harness.snapshot().digest;
  const failure = 'DELIVERY_NATIVE_STOP:codex:gpt-5.6-sol:requested model unavailable';
  const outcome = await runDeliveryOutcome(f.candidate.harness, f.task.id, f.task.owner, { execute: async () => { throw new Error(failure); } });
  expect(outcome.failure).toBe(failure); expect(outcome.success).toBe(false);
  expect(f.candidate.harness.snapshot().digest).toBe(before);
  expect(f.candidate.harness.read(f.task.id).status).toBe('paused');
});

it('rejects stale native implementation identity before applying returned bytes', async () => {
  const f = await fixture(), before = f.candidate.harness.snapshot().digest;
  const outcome = await runDeliveryOutcome(f.candidate.harness, f.task.id, f.task.owner, { execute: async request => {
    const response = result(request);
    if (request.stage === 'implementation') response.response.native.model = 'wrong-model';
    return response;
  } });
  expect(outcome.failure).toBe('DELIVERY_EXECUTOR_IDENTITY_MISMATCH'); expect(f.candidate.harness.snapshot().digest).toBe(before);
});

it('planning prompt example has exactly the parsed proposal fields', () => {
  const request = { stage: 'architecture', route: native, scope: ['product.txt'], feedback: [] } as unknown as NativeStageRequest;
  const prompt = JSON.parse(renderDeliveryPrompt(request, [], [{ id: 'build' }]));
  expect(Object.keys(prompt.response).sort()).toEqual(['changes', 'issues', 'outcome', 'plan', 'summary']);
  expect(prompt.context).toEqual([{ id: 'build' }]);
});

it('composes real API proposals, application custody, review and accounting without native training', async () => {
  const f = await fixture(true), stages: string[] = [];
  const api = createDeliveryApi({ directory: f.candidate.harness.context.apiDirectory!, apiKey: () => 'test-only', fetch: async (_url, options) => {
    const body = JSON.parse(String(options?.body)), prompt = JSON.parse(body.messages.at(-1).content);
    const stage = prompt.mode; stages.push(stage);
    return new Response(JSON.stringify({ id: `mock-${stages.length}`, model: DELIVERY_API_DEFAULTS.model,
      usage: { cost: 0.01, prompt_tokens: 100, completion_tokens: 50 }, choices: [{ finish_reason: 'stop', message: { content: JSON.stringify({
        outcome: 'completed', summary: 'Mock verified proposal', issues: [], changes: stage === 'implementation' ? [{ path: 'product.txt', content: 'fixed\n' }] : [],
        ...(stage === 'architecture' ? { plan: { summary: 'Fix source', files: ['product.txt'], tests: ['build', 'public'] } } : {}),
      }) } }] }));
  } });
  const outcome = await runDeliveryOutcome(f.candidate.harness, f.task.id, f.task.owner, {
    execute: (request, files, context, signal) => api(request, files, context, hash(f.task), signal),
  });
  expect(outcome.failure).toBeNull(); expect(outcome.success).toBe(true);
  expect(stages).toEqual(['architecture', 'implementation', 'review']); expect(outcome.knownActualUsd).toBeCloseTo(0.03);
  expect(outcome.kernel.totalCostUsd).toBeCloseTo(0.03);
  expect(readdirSync(join(f.harness.directory, 'learning', hash(DELIVERY_ROOT_POLICY), 'deltas'))).toEqual([]);
  expect(readFileSync(join(f.root, 'product.txt'), 'utf8')).toBe('before\n');
  expect(readdirSync(f.candidate.harness.context.apiDirectory!).filter(name => name.startsWith('request-'))).toHaveLength(3);
});

it.each(['invalid-plan', '402', '401'])('classifies %s planner response with exact native fallback and charge custody', async mode => {
  const f = await fixture(true), routes: string[] = [];
  const api = createDeliveryApi({ directory: f.candidate.harness.context.apiDirectory!, apiKey: () => 'test-only', fetch: async (_url, options) => {
    const prompt = JSON.parse(JSON.parse(String(options?.body)).messages[0].content);
    if (prompt.mode === 'architecture' && mode !== 'invalid-plan') {
      const status = Number(mode); return new Response(JSON.stringify({ error: { code: status } }), { status });
    }
    const stage = prompt.mode;
    return new Response(JSON.stringify({ id: `mock-${routes.length}`, model: DELIVERY_API_DEFAULTS.model,
      usage: { cost: 0.01, prompt_tokens: 100, completion_tokens: 50 }, choices: [{ finish_reason: 'stop', message: { content: JSON.stringify({
        outcome: 'completed', summary: 'Mock proposal', issues: [], changes: stage === 'implementation' ? [{ path: 'product.txt', content: 'fixed\n' }] : [],
        ...(stage === 'architecture' ? { plan: { summary: 'Invalid planner check', files: ['product.txt'], tests: ['undeclared'] } } : {}),
      }) } }] }));
  } });
  const outcome = await runDeliveryOutcome(f.candidate.harness, f.task.id, f.task.owner, { execute: async (request, files, context, signal) => {
    routes.push(`${request.route.host}:${request.route.model}:${request.route.effort}`);
    return request.route.host === 'openrouter' ? api(request, files, context, hash(f.task), signal) : result(request);
  } });
  if (mode === '401') { expect(outcome.failure).toBe('authentication-rejected'); expect(routes).toHaveLength(1); }
  else {
    expect(outcome.failure).toBeNull();
    expect(routes[1]).toBe(mode === '402' ? 'claude-code:cc/claude-sonnet-5[1m]:medium' : 'claude-code:cc/claude-opus-5-5[1m]:high');
    expect(outcome.knownActualUsd).toBeCloseTo(mode === '402' ? 0.02 : 0.03);
  }
});

it('preserves auth provider error when concurrent successful version preflight finishes last', async () => {
  const f = await fixture(); let release!: () => void;
  const barrier = new Promise<void>(resolve => { release = resolve; });
  vi.spyOn(processes, 'runCommand').mockImplementation(async (argv, _cwd, _env, out, err) => {
    if (argv.includes('--version')) { await barrier; writeSync(out, 'claude-code 1.0.0'); return { exitCode: 0, signal: null }; }
    writeSync(err, 'configured gateway: requested model unavailable'); release(); return { exitCode: 1, signal: null };
  });
  const action = await f.candidate.harness.next(f.task.id, f.task.owner);
  if (action.kind !== 'native') throw new Error('fixture request missing');
  const request = { ...action.request, route: { host: 'claude-code' as const, model: 'cc/claude-opus-5-5[1m]', effort: 'high' as const } };
  await expect(createDeliveryExecutor(f.candidate.harness, hash(f.task))(request, [], [])).rejects.toThrow('configured gateway: requested model unavailable');
});

it.each([0, 1])('attributes structured Claude output exhaustion before stderr warnings (exit %i)', async exitCode => {
  const f = await fixture();
  vi.spyOn(processes, 'runCommand').mockImplementation(async (argv, _cwd, env, out, err) => {
    expect(env.CLAUDE_CODE_MAX_OUTPUT_TOKENS).toBe('128000');
    expect(env).not.toHaveProperty('MAX_THINKING_TOKENS');
    if (argv.includes('--version')) { writeSync(out, 'claude-code 1.0.0'); return { exitCode: 0, signal: null }; }
    if (!argv.includes('--json-schema')) { writeSync(out, 'READY'); return { exitCode: 0, signal: null }; }
    writeSync(out, JSON.stringify({ type: 'result', subtype: 'success', is_error: true,
      result: "API Error: Claude's response exceeded the 32000 output token maximum. To configure this behavior, set the CLAUDE_CODE_MAX_OUTPUT_TOKENS environment variable." }));
    writeSync(err, 'unrecognized_model diagnostic');
    return { exitCode, signal: null };
  });
  const action = await f.candidate.harness.next(f.task.id, f.task.owner);
  if (action.kind !== 'native') throw new Error('fixture request missing');
  const request = { ...action.request, route: { host: 'claude-code' as const, model: 'cc/claude-sonnet-5-5[1m]', effort: 'high' as const } };
  await expect(createDeliveryExecutor(f.candidate.harness, hash(f.task))(request, [], []))
    .rejects.toThrow(/DELIVERY_NATIVE_OUTPUT_EXHAUSTED:claude-code:.*Claude's response exceeded the 32000 output token maximum/);
});

it('bounds and sanitizes structured errors without exposing known environment credentials', () => {
  const message = claudeStructuredError(JSON.stringify({ is_error: true, result: 'secret-value\n' + 'x'.repeat(3000) }), { ANTHROPIC_AUTH_TOKEN: 'secret-value' });
  expect(message).toHaveLength(2000);
  expect(message).toMatch(/^\[redacted\] /);
  expect(claudeStructuredError('{"is_error":false,"result":"success"}', {})).toBeUndefined();
  expect(claudeStructuredError('not json', {})).toBeUndefined();
});

it.each([
  { message: "Claude's response exceeded the 32000 output token maximum", error: 'check-timeout' },
  { message: "Claude's response exceeded the 32000 output token maximum", error: 'cancelled' },
  { message: "Claude's response exceeded the 32000 output token maximum", error: 'check-output-limit' },
  { message: "Claude's response exceeded the 32000 output token maximum", error: 'spawn failed' },
  { message: 'Authentication token limit exceeded', error: undefined },
  { message: 'Quota token limit exceeded', error: undefined },
])('does not misclassify process errors or token quotas as output exhaustion: $message/$error', async ({ message, error }) => {
  const f = await fixture();
  vi.spyOn(processes, 'runCommand').mockImplementation(async (argv, _cwd, _env, out, err) => {
    if (argv.includes('--version')) { writeSync(out, 'claude-code 1.0.0'); return { exitCode: 0, signal: null }; }
    if (!argv.includes('--json-schema')) { writeSync(out, 'READY'); return { exitCode: 0, signal: null }; }
    writeSync(out, JSON.stringify({ is_error: true, result: message })); writeSync(err, 'unrecognized_model');
    return { exitCode: 1, signal: null, ...(error ? { error } : {}) };
  });
  const action = await f.candidate.harness.next(f.task.id, f.task.owner);
  if (action.kind !== 'native') throw new Error('fixture request missing');
  const request = { ...action.request, route: { host: 'claude-code' as const, model: 'cc/claude-sonnet-5-5[1m]', effort: 'high' as const } };
  const failure = await createDeliveryExecutor(f.candidate.harness, hash(f.task))(request, [], []).catch(error => error);
  expect(failure.message).toContain('DELIVERY_NATIVE_STOP:');
  expect(failure.message).toContain(error ?? message);
  expect(failure.message).not.toContain('DELIVERY_NATIVE_OUTPUT_EXHAUSTED');
});
