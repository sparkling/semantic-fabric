import { mkdtempSync, readFileSync, readdirSync, rmSync, writeSync, writeFileSync } from 'node:fs';
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
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllEnvs(); for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });
async function fixture(api = false, sonnet = false, scope = ['product.txt']) {
  const f = workflowFixture(roots, "import {readFileSync} from 'node:fs'; const pass = readFileSync('../product.txt','utf8') === 'fixed\\n'; if (!pass) console.error('error[E0308]: expected &Router, found &RequestDeadlineService'); process.exit(pass ? 0 : 1);\n");
  f.task.scope = scope;
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

it.each(['Cargo.lock', 'nested/Cargo.lock', 'pnpm-lock.yaml', 'package-lock.json', 'yarn.lock', 'bun.lock', 'bun.lockb', 'poetry.lock', 'uv.lock'])
('admits generated dependency lock %s above source line limit through checks and review', async path => {
  const f = await fixture(false, true, ['product.txt', path]), stages: string[] = [];
  const content = '# generated dependency metadata\n'.repeat(600), before = f.harness.snapshot().digest;
  const outcome = await runDeliveryOutcome(f.candidate.harness, f.task.id, f.task.owner, { execute: async request => {
    stages.push(request.stage);
    const response = result(request);
    if (request.stage === 'implementation') response.changes.push({ path, content });
    return response;
  } });
  expect(outcome.failure).toBeNull(); expect(outcome.success).toBe(true); expect(outcome.status).toBe('candidate-awaiting-integration');
  expect(stages).toEqual(['architecture', 'implementation', 'review']);
  expect(readFileSync(join(f.candidate.harness.root, path), 'utf8')).toBe(content);
  expect(f.harness.snapshot().digest).toBe(before);
});

it.each([
  ['vendor/parser.rs', 'source\n'.repeat(600), 'DELIVERY_FILE_LINE_LIMIT'],
  ['fake-Cargo.lock', '# lock\n'.repeat(600), 'DELIVERY_FILE_LINE_LIMIT'],
  ['Cargo.lock.rs', 'source\n'.repeat(600), 'DELIVERY_FILE_LINE_LIMIT'],
  ['Cargo.lock', 'x'.repeat(512 * 1024 + 1), 'Change outside scope'],
])('retains source and byte guards for %s', async (path, content, failure) => {
  const f = await fixture(false, true, ['product.txt', path]), before = f.candidate.harness.snapshot().digest;
  const outcome = await runDeliveryOutcome(f.candidate.harness, f.task.id, f.task.owner, { execute: async request => {
    const response = result(request);
    if (request.stage === 'implementation') response.changes.push({ path, content });
    return response;
  } });
  expect(outcome.failure).toBe(failure); expect(outcome.success).toBe(false);
  expect(f.candidate.harness.snapshot().digest).toBe(before);
});

it.each([false, true])('executes plan, author, failed checks, capable repair and fresh review (Sonnet=%s)', async sonnet => {
  const f = await fixture(false, sonnet), events: string[] = [], before = f.harness.snapshot().digest;
  const execute: DeliveryExecutor = async (request, _files, context) => {
    events.push(request.stage === 'implementation' && request.repair ? 'repair' : request.stage);
    if (request.repair) {
      expect(request.route).toMatchObject({ host: 'claude-code', model: 'cc/claude-opus-5-5[1m]', effort: 'high' });
      const packet = JSON.parse(renderDeliveryPrompt(request, _files, context));
      expect(packet.toolsAvailable).toBe(false);
      expect(JSON.stringify(packet.context)).toContain('error[E0308]: expected &Router, found &RequestDeadlineService');
      expect(packet.context).toEqual(expect.arrayContaining([expect.objectContaining({ diagnosticData: expect.objectContaining({
        checkId: 'build', stderr: expect.objectContaining({ status: 'verified' }),
      }) })]));
    }
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
    expect(routes[1]).toBe(mode === '402' ? 'codex:gpt-6.1-sol:high' : 'claude-code:cc/claude-opus-5-5[1m]:high');
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
    expect(env).toMatchObject({ CARGO_PROFILE_DEV_DEBUG: '1', CARGO_PROFILE_TEST_DEBUG: '1',
      CARGO_PROFILE_DEV_INCREMENTAL: 'false', CARGO_PROFILE_TEST_INCREMENTAL: 'false' });
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

it.each([0, 1])('keeps discovery journals empty and failures private (inventory exit %i)', async exitCode => {
  const f = await fixture(); const discoveryRoots: string[] = []; let modelCalls = 0;
  vi.spyOn(processes, 'runCommand').mockImplementation(async (argv, _cwd, _env, _out, _err, directory, limits) => {
    if (argv.includes('mcp')) {
      discoveryRoots.push(directory);
      limits.privateCapture!('stdout', Buffer.from(JSON.stringify([{ name: 'ruflo', enabled: true, env: { secret: 'private-value' } }])));
      limits.privateCapture!('stderr', Buffer.from('private-value'));
      return { exitCode, signal: null };
    }
    if (argv.includes('--version')) { writeSync(_out, 'codex-cli fixture'); return { exitCode: 0, signal: null }; }
    modelCalls++;
    expect(argv).toContain('mcp_servers.ruflo.enabled=false');
    return { exitCode: 1, signal: null };
  });
  const action = await f.candidate.harness.next(f.task.id, f.task.owner);
  if (action.kind !== 'native') throw new Error('fixture request missing');
  const request = { ...action.request, route: { host: 'codex' as const, model: 'gpt-6.1-sol', effort: 'high' as const } };
  const error = await createDeliveryExecutor(f.candidate.harness, hash(f.task))(request, [], []).catch(error => error);
  expect(error.message).not.toContain('private-value'); expect(modelCalls).toBe(exitCode === 0 ? 1 : 0);
  expect(discoveryRoots).toHaveLength(1);
  for (const directory of discoveryRoots) {
    expect(readFileSync(join(directory, 'stdout'), 'utf8')).toBe('');
    expect(readFileSync(join(directory, 'stderr'), 'utf8')).toBe('');
  }
});

it.each(['claude-code','codex'] as const)('requests %s streaming and reports inactivity distinctly without retry or fallback', async host => {
  const f = await fixture(); let calls = 0;
  vi.spyOn(processes, 'runCommand').mockImplementation(async (argv, _cwd, _env, out, _err, _dir, _limits, _signal, _stdin, observer) => {
    if (argv.includes('mcp')) { _limits.privateCapture!('stdout', Buffer.from('[]')); return { exitCode: 0, signal: null }; }
    if (argv.includes('--version')) { writeSync(out, 'claude-code 1.0.0'); return { exitCode: 0, signal: null }; }
    if (!argv.includes('--json-schema') && !argv.includes('--output-schema')) { writeSync(out, 'READY'); return { exitCode: 0, signal: null }; }
    calls++;
    if(host==='claude-code'){
      expect(argv[argv.indexOf('--output-format') + 1]).toBe('stream-json');
      expect(argv).toContain('--include-partial-messages'); expect(argv).toContain('--verbose');
    }else {
      expect(argv).toContain('--json');
      for (const [name, value] of Object.entries(processes.ordinaryRustEnvironment({}))) {
        expect(argv).toContain(`shell_environment_policy.set.${name}=${JSON.stringify(value)}`);
      }
    }
    expect(observer).toBeDefined();
    return { exitCode: null, signal: 'SIGTERM', error: 'native-inactivity' };
  });
  const action = await f.candidate.harness.next(f.task.id, f.task.owner);
  if (action.kind !== 'native') throw new Error('fixture request missing');
  const request = { ...action.request, route: { host, model: host==='codex'?'gpt-6-astra':'cc/claude-sonnet-5-5[1m]', effort: 'high' as const } };
  const failure = await createDeliveryExecutor(f.candidate.harness, hash(f.task))(request, [], []).catch(error => error);
  expect(failure.message).toContain(`DELIVERY_NATIVE_STALLED:${host}:${request.route.model}:`);
  expect(failure.message).not.toContain('SUBSCRIPTION_UNAVAILABLE'); expect(calls).toBe(1);
});

it.each([['claude-code',false],['claude-code',true],['claude-code','output'],['codex',false],['codex',true]] as const)('real %s stream preserves final envelope and redacted failure attribution (failure=%s)', async (host,failure) => {
  const f=await fixture();const bin=mkdtempSync(join(tmpdir(),'fabric-fake-native-'));roots.push(bin);
  writeFileSync(join(bin,host==='codex'?'codex':'claude'),`#!${process.execPath}\nconst args=process.argv.slice(2);
if(args.includes('mcp')){console.log(JSON.stringify([{name:'ruflo',enabled:true,env:{token:'private-inventory-value'}}]));process.stderr.write('private-inventory-value');process.exit(0)}
if(args.includes('--version')){console.log('claude-code fake');process.exit(0)}
if(!args.includes('--json-schema')&&!args.includes('--output-schema')){console.log('READY');process.exit(0)}
process.stdin.resume();
process.stdin.on('end',()=>{
  if(${failure==='output'}){console.log(JSON.stringify({type:'result',is_error:true,result:"Claude's response exceeded the 32000 output token maximum"}));process.stderr.write('unrecognized_model\\n');return;}
  if(${failure===true}){process.stderr.write('configured-model-unavailable\\npassword=unknown-secret\\n');process.exitCode=1;return;}
  const value={outcome:'completed',summary:'fake final',issues:[],changes:[]};
  if(${host==='codex'}){
    console.log(JSON.stringify({type:'error',message:'Reconnecting 1/5'}));
    require('node:fs').writeFileSync(args[args.indexOf('--output-last-message')+1],JSON.stringify(value));
    console.log(JSON.stringify({type:'item.completed',item:{id:'response',type:'agent_message',text:JSON.stringify(value)}}));
    console.log(JSON.stringify({type:'turn.completed',usage:{}}));
  }else{
    console.log(JSON.stringify({type:'stream_event',event:{type:'content_block_delta',delta:{type:'text_delta',text:'private body'}}}));
    console.log(JSON.stringify({type:'result',is_error:false,structured_output:value}));
  }
});\n`,{mode:0o700});
  vi.stubEnv('PATH',bin+':'+process.env.PATH);
  const action=await f.candidate.harness.next(f.task.id,f.task.owner);if(action.kind!=='native')throw Error('fixture');
  const request={...action.request,route:{host,model:host==='codex'?'gpt-6-astra':'cc/claude-sonnet-5-5[1m]',effort:'high' as const}};
  const promise=createDeliveryExecutor(f.candidate.harness,hash(f.task))(request,[],[]);
  if(failure==='output'){await expect(promise).rejects.toThrow(/DELIVERY_NATIVE_OUTPUT_EXHAUSTED:claude-code:.*response exceeded the 32000 output token maximum/);}
  else if(failure){const error=await promise.catch(e=>e);expect(error.message).toContain('configured-model-unavailable');expect(error.message).not.toContain('unknown-secret');}
  else expect((await promise).response.summary).toBe('fake final');
  for (const entry of readdirSync(f.candidate.harness.directory).filter(name => name.startsWith('native-'))) {
    const invocation = join(f.candidate.harness.directory, entry);
    for (const process of readdirSync(invocation).filter(name => /^[0-9a-f-]{36}$/.test(name))) {
      for (const log of ['stdout', 'stderr']) expect(readFileSync(join(invocation, process, log), 'utf8')).not.toContain('private-inventory-value');
    }
  }
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
