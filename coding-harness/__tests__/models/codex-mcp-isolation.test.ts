// SPDX-License-Identifier: MIT
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, expect, it } from 'vitest';
import { CodexSubscriptionAdapter } from '../../src/models/native-adapters.js';
import { CodexMcpInventory } from '../../src/models/codex-mcp-isolation.js';
import type { NativeProcessRequest, NativeProcessResult } from '../../src/models/types.js';

const roots: string[] = [];
afterEach(() => roots.splice(0).forEach(root => rmSync(root, { recursive: true, force: true })));
const ok = (stdout: string): NativeProcessResult => ({ executionId: 'native-run:00000000-0000-4000-8000-000000000000', exitCode: 0, stdout,
  stderr: '', timedOut: false, stdoutDigest: '0'.repeat(64), stderrDigest: '0'.repeat(64) });
function fixture(run: (request: NativeProcessRequest) => Promise<NativeProcessResult>, inventory?: CodexMcpInventory) {
  const root = mkdtempSync(join(tmpdir(), 'fabric-mcp-')); roots.push(root);
  const schemaPath = join(root, 'schema.json'); writeFileSync(schemaPath, '{}');
  const adapter = new CodexSubscriptionAdapter({ executable: '/tools/codex', runner: { run }, evidenceRoot: root,
    sourceEnvironment: { HOME: '/home/fixture', CODEX_HOME: '/home/fixture/.codex' }, mcpInventory: inventory });
  const request = { cwd: '/repo', model: 'gpt-6.1-sol', reasoningEffort: 'high' as const, prompt: 'review', schema: {},
    schemaPath, outputPath: join(root, 'response.json'), workspaceAccess: 'read' as const,
    timeoutMs: 1000, operation: 'review' as const };
  return { adapter, request };
}

it('coalesces cold discovery across adapters, then disables every inherited server in auth and invocation', async () => {
  const inventory = new CodexMcpInventory(); const requests: NativeProcessRequest[] = [];
  let release!: () => void;
  const hold = new Promise<void>(resolve => { release = resolve; });
  const run = async (request: NativeProcessRequest) => {
    requests.push(request);
    if (request.purpose === 'configuration-discovery') {
      await hold;
      return ok(JSON.stringify([{ name: 'ruflo', enabled: true, transport: { secret: 'private-value' } },
        { name: 'brain', enabled: false }, { name: 'ruflo', enabled: true }]));
    }
    return ok(request.args.includes('--version') ? 'codex-cli fixture' : 'READY');
  };
  const first = fixture(run, inventory), second = fixture(run, inventory);
  expect(() => first.adapter.buildInvocation(first.request)).toThrow('HARNESS_CODEX_MCP_CONFIGURATION_REQUIRED');
  const calls = [first.adapter.preflight({ cwd: '/repo', requestedModel: first.request.model }),
    second.adapter.invoke(second.request)];
  expect(requests.filter(r => r.purpose === 'configuration-discovery')).toHaveLength(1);
  release(); await Promise.all(calls); await first.adapter.invoke(first.request);
  expect(requests.filter(r => r.purpose === 'configuration-discovery')).toHaveLength(1);
  const discovery = requests[0];
  expect(discovery.args).toEqual(expect.arrayContaining(['mcp', 'list', '--json', 'features.plugins=false', 'features.hooks=false']));
  for (const request of requests.filter(r => ['authentication-preflight', 'model-invocation'].includes(r.purpose))) {
    expect(request.args).toContain('mcp_servers.ruflo.enabled=false');
    expect(request.args).toContain('mcp_servers.brain.enabled=false');
    expect(request.args).not.toContain('mcp_servers={}');
    expect(request.args.join(' ')).not.toContain('private-value');
    expect(request.args.join(' ')).not.toContain('model_provider=');
    expect(request.env.CODEX_HOME).toBe('/home/fixture/.codex');
  }
  expect(first.adapter.buildInvocation(first.request).args).toContain('model_reasoning_effort="high"');
  await second.adapter.invoke({ ...second.request, cwd: '/other-repo' });
  expect(requests.filter(r => r.purpose === 'configuration-discovery')).toHaveLength(2);
});

it.each(['bad-json', 'bad-name', 'bad-enabled', 'exit', 'throw', 'timeout', 'output-limit'])
('refuses %s inventory without private output, cause or model invocation; retries discovery on next call', async failure => {
  const requests: NativeProcessRequest[] = []; let failed = true;
  const { adapter, request } = fixture(async input => {
    requests.push(input);
    if (input.purpose !== 'configuration-discovery') return ok('{}');
    if (!failed) return ok('[]');
    if (failure === 'throw') throw new Error('private-value');
    return { ...ok(failure === 'bad-json' ? 'private-value' : JSON.stringify([
      { name: failure === 'bad-name' ? 'private.value' : 'safe', enabled: failure === 'bad-enabled' ? 'private-value' : true }])),
      stderr: 'private-value', exitCode: failure === 'exit' ? 1 : 0,
      timedOut: failure === 'timeout', outputLimitExceeded: failure === 'output-limit' };
  });
  const error = await adapter.invoke(request).catch(error => error);
  expect(error.message).toBe('HARNESS_CODEX_MCP_CONFIGURATION_INVALID'); expect(error.cause).toBeUndefined();
  expect(requests.map(r => r.purpose)).toEqual(['configuration-discovery']);
  failed = false; await adapter.invoke(request);
  expect(requests.map(r => r.purpose)).toEqual(['configuration-discovery', 'configuration-discovery', 'model-invocation']);
});

it('does not share pending inventory between cancellation scopes', async () => {
  const inventory = new CodexMcpInventory(); let discoveries = 0; let release!: () => void;
  const hold = new Promise<void>(resolve => { release = resolve; });
  const run = async (request: NativeProcessRequest) => {
    if (request.purpose === 'configuration-discovery') { discoveries++; await hold; return { ...ok('[]'), cancelled: request.signal?.aborted }; }
    return ok('{}');
  };
  const first = fixture(run, inventory), second = fixture(run, inventory);
  const controller = new AbortController(), other = new AbortController();
  const left = first.adapter.invoke({ ...first.request, signal: controller.signal }).catch(error => error);
  const right = second.adapter.invoke({ ...second.request, signal: other.signal });
  expect(discoveries).toBe(2); controller.abort(); release();
  expect((await left).name).toBe('NativeCancellationError'); await right;
});

it.each(['check-process-group-unconfirmed', 'HARNESS_NATIVE_RESOURCE_TERMINATION_FAILED'])
('preserves secret-safe cleanup refusal %s', async marker => {
  const { adapter, request } = fixture(async () => {
    if (marker === 'HARNESS_NATIVE_RESOURCE_TERMINATION_FAILED') throw new Error(marker, { cause: new Error('private-value') });
    return { ...ok('private-value'), spawnError: marker };
  });
  const error = await adapter.invoke(request).catch(error => error);
  expect(error.message).toBe(marker === 'check-process-group-unconfirmed' ? 'HARNESS_CODEX_MCP_CLEANUP_UNCONFIRMED' : marker);
  expect(error.cause).toBeUndefined();
});
