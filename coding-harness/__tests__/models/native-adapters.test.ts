// SPDX-License-Identifier: MIT

import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { afterEach, describe, expect, it } from 'vitest';
import {
  ClaudeCodeSubscriptionAdapter,
  CodexSubscriptionAdapter,
  NativeAuthPreflightError,
  NativeHostInvocationError,
  TransientNativeHostInvocationError,
  preflightNativeSubscriptions,
} from '../../src/models/native-adapters.js';
import type {
  NativeProcessRequest,
  NativeProcessResult,
  NativeProcessRunner,
} from '../../src/models/types.js';

class FakeRunner implements NativeProcessRunner {
  readonly requests: NativeProcessRequest[] = [];

  constructor(
    private readonly respond: (
      request: NativeProcessRequest,
    ) => NativeProcessResult | Promise<NativeProcessResult>,
  ) {}

  async run(request: NativeProcessRequest): Promise<NativeProcessResult> {
    this.requests.push(request);
    if (request.purpose === 'configuration-discovery') return ok('[]');
    return await this.respond(request);
  }
}

let sequence = 0;
const ok = (stdout: string): NativeProcessResult => ({
  executionId: `native-run:00000000-0000-4000-8000-${String(++sequence).padStart(12, '0')}`,
  exitCode: 0,
  stdout,
  stderr: '',
  timedOut: false,
  stdoutDigest: createHash('sha256').update(stdout).digest('hex'),
  stderrDigest: createHash('sha256').update('').digest('hex'),
});

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe('native subscription adapters', () => {
  it('preserves configured gateway aliases and explicit Claude medium effort', () => {
    const adapter = new ClaudeCodeSubscriptionAdapter({ executable: '/tools/claude', runner: new FakeRunner(() => ok('{}')),
      sourceEnvironment: { HOME: '/home/tester' } });
    const request = adapter.buildInvocation({ cwd: '/repo', model: 'cc/claude-sonnet-5[1m]', reasoningEffort: 'medium',
      prompt: 'bounded task', schema: {}, workspaceAccess: 'read', timeoutMs: 1000, operation: 'implementation' });
    expect(request.args[request.args.indexOf('--model') + 1]).toBe('cc/claude-sonnet-5[1m]');
    expect(request.args[request.args.indexOf('--effort') + 1]).toBe('medium');
  });

  it('checks both configured subscriptions without provider fallback', async () => {
    const evidenceRoot = mkdtempSync(join(tmpdir(), 'coding-harness-adapter-'));
    roots.push(evidenceRoot);
    const runner = new FakeRunner((request) => {
      if (request.executable === '/tools/codex') {
        if (request.args.includes('--version')) return ok('codex-cli 1.2.3');
        expect(request.args).toContain('exec');
        expect(request.args[request.args.indexOf('--model') + 1]).toBe('gpt-5.6-sol');
        expect(request.args).not.toContain('login');
        return { ...ok('READY'), stderr: 'Not logged in' };
      }
      return ok(
        request.args.includes('--version')
          ? 'claude-code 4.5.6'
          : 'READY',
      );
    });
    const environment = {
      HOME: '/home/tester',
      PATH: '/usr/bin',
      OPENAI_API_KEY: 'stripped',
      ANTHROPIC_BASE_URL: 'https://gateway.invalid',
      HTTPS_PROXY: 'http://proxy.invalid',
    };
    const codex = new CodexSubscriptionAdapter({
      executable: '/tools/codex',
      runner,
      sourceEnvironment: environment,
      evidenceRoot,
    });
    const claude = new ClaudeCodeSubscriptionAdapter({
      executable: '/tools/claude',
      runner,
      sourceEnvironment: environment,
    });

    const evidence = await preflightNativeSubscriptions({
      codex,
      claude,
      cwd: '/repo',
      requestedModels: { codex: 'gpt-5.6-sol', claude: 'claude-sonnet-4-6' },
    });

    expect(evidence.map(({ host }) => host)).toEqual(['codex', 'claude-code']);
    expect(evidence.map(({ authentication }) => authentication)).toEqual([
      'chatgpt-subscription',
      'claude-subscription',
    ]);
    expect(runner.requests).toHaveLength(5);
    for (const request of runner.requests) {
      expect(request.env.OPENAI_API_KEY).toBeUndefined();
      expect(request.env.ANTHROPIC_BASE_URL).toBe(request.host === 'claude-code' ? 'https://gateway.invalid' : undefined);
      expect(request.env.HTTPS_PROXY).toBeUndefined();
      if (request.host === 'codex') {
        expect(request.args.filter((argument) => argument === 'analytics.enabled=false'))
          .toHaveLength(1);
        expect(request.args.filter((argument) => argument === 'otel.metrics_exporter="none"'))
          .toHaveLength(1);
      }
      expect(request.env.CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC).toBe(
        request.host === 'claude-code' ? '1' : undefined,
      );
    }
  });

  it('builds ephemeral structured invocations with cancellation propagation', async () => {
    const runner = new FakeRunner(() => ok('{}'));
    const controller = new AbortController();
    const root = mkdtempSync(join(tmpdir(), 'coding-harness-adapter-'));
    roots.push(root);
    const codex = new CodexSubscriptionAdapter({
      executable: '/tools/codex',
      runner,
      sourceEnvironment: { HOME: '/home/tester', PATH: '/usr/bin' },
      evidenceRoot: root,
    });
    const claude = new ClaudeCodeSubscriptionAdapter({
      executable: '/tools/claude',
      runner,
      sourceEnvironment: { HOME: '/home/tester', PATH: '/usr/bin' },
    });
    const schemaPath = join(root, 'response.schema.json');
    const outputPath = join(root, 'response.json');
    writeFileSync(schemaPath, JSON.stringify({ type: 'object' }));

    await codex.invoke({
      cwd: root,
      model: 'gpt-5.6-sol',
      prompt: 'review this patch',
      schema: { type: 'object' },
      schemaPath,
      outputPath,
      workspaceAccess: 'read',
      timeoutMs: 1_000,
      signal: controller.signal,
      operation: 'review',
    });
    await claude.invoke({
      cwd: root,
      model: 'claude-sonnet-4-6',
      prompt: 'review this patch',
      schema: { type: 'object' },
      workspaceAccess: 'read',
      timeoutMs: 1_000,
      signal: controller.signal,
      operation: 'review',
    });

    const [codexRequest, claudeRequest] = runner.requests.filter(request => request.purpose === 'model-invocation');
    expect(codexRequest?.args).toEqual(
      expect.arrayContaining([
        'exec',
        '--ephemeral',
        '--strict-config',
        '--skip-git-repo-check',
        '--output-schema',
        schemaPath,
      ]),
    );
    expect(codexRequest?.args.join(' ')).not.toContain('model_provider="openai"');
    expect(codexRequest?.args).not.toContain('--ignore-user-config');
    expect(codexRequest?.args.join(' ')).not.toContain('model_reasoning_effort=');
    expect(codexRequest?.args.join(' ')).toContain('analytics.enabled=false');
    expect(codexRequest?.args.join(' ')).toContain('otel.metrics_exporter="none"');
    expect(claudeRequest?.args).toEqual(
      expect.arrayContaining([
        '-p',
        '',
        'Edit,Write,Bash,WebFetch,WebSearch,Task',
        '--strict-mcp-config',
        '--no-session-persistence',
        '--safe-mode',
      ]),
    );
    expect(claudeRequest?.args).not.toContain('WebSearch');
    expect(codexRequest?.signal).toBe(controller.signal);
    expect(claudeRequest?.signal).toBe(controller.signal);
    expect(codexRequest?.env.CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC).toBeUndefined();
    expect(claudeRequest?.env.CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC).toBe('1');
    for (const operation of ['architecture', 'implementation', 'repair', 'review'] as const) {
      const request = codex.buildInvocation({
        cwd: root,
        model: 'gpt-5.6-sol',
        prompt: `${operation} the admitted change`,
        schema: { type: 'object' },
        schemaPath,
        outputPath,
        workspaceAccess: 'read',
        timeoutMs: 1_000,
        operation,
      });
      expect(request.args.filter((value) => value.startsWith('model_reasoning_effort=')))
        .toEqual([]);
    }
    expect(() => codex.buildInvocation({
      cwd: root,
      model: 'gpt-5.6-sol',
      prompt: 'escape',
      schema: { type: 'object' },
      schemaPath,
      outputPath: join(root, '..', 'escape.json'),
      workspaceAccess: 'read',
      timeoutMs: 1_000,
      operation: 'review',
    })).toThrow('HARNESS_NATIVE_OUTPUT_PATH_OUTSIDE_CWD');
  });

  it('forwards every explicit Astra effort unchanged for every operation, including max and ultra', async () => {
    const root = mkdtempSync(join(tmpdir(), 'coding-harness-effort-'));
    roots.push(root);
    const schemaPath = join(root, 'response.schema.json');
    writeFileSync(schemaPath, '{}');
    const adapter = new CodexSubscriptionAdapter({
      executable: '/tools/codex', runner: new FakeRunner(() => ok('{}')),
      sourceEnvironment: { HOME: '/home/tester' }, evidenceRoot: root,
    });
    const base = {
      cwd: root, model: 'gpt-6-astra', prompt: 'bounded task', schema: {}, schemaPath,
      outputPath: join(root, 'response.json'), workspaceAccess: 'read' as const, timeoutMs: 1_000,
    };
    await adapter.invoke({ ...base, operation: 'review' });
    for (const operation of ['architecture', 'implementation', 'repair', 'review'] as const) {
      for (const reasoningEffort of ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'] as const) {
        const request = adapter.buildInvocation({ ...base, operation, reasoningEffort });
        expect(request.args.filter(value => value.startsWith('model_reasoning_effort=')))
          .toEqual([`model_reasoning_effort="${reasoningEffort}"`]);
        expect(request.model).toBe('gpt-6-astra');
      }
    }
    for (const reasoningEffort of ['', 'invalid', 'ultra"', null, 0]) {
      expect(() => adapter.buildInvocation({
        ...base, operation: 'review', reasoningEffort,
      } as never)).toThrow('HARNESS_NATIVE_REASONING_EFFORT_INVALID');
    }
  });

  it('rejects auth metadata in place of a successful Claude readiness response', async () => {
    const runner = new FakeRunner((request) =>
      ok(
        request.args.includes('--version')
          ? 'claude-code 4.5.6'
          : JSON.stringify({
              loggedIn: true,
              authMethod: 'apiKey',
              apiProvider: 'firstParty',
              apiKeySource: 'ANTHROPIC_API_KEY',
            }),
      ),
    );
    const adapter = new ClaudeCodeSubscriptionAdapter({
      executable: '/tools/claude',
      runner,
      sourceEnvironment: { HOME: '/home/tester' },
    });

    await expect(
      adapter.preflight({ cwd: '/repo', requestedModel: 'claude-opus-4-1' }),
    ).rejects.toBeInstanceOf(NativeAuthPreflightError);
  });

  it('rejects successful exit codes that breached a native process hard limit', async () => {
    for (const failure of [{ outputLimitExceeded: true }, { spawnError: 'late spawn fault' }]) {
      const runner = new FakeRunner(() => ({ ...ok('claude-code 4.5.6'), ...failure }));
      const adapter = new ClaudeCodeSubscriptionAdapter({
        executable: '/tools/claude', runner, sourceEnvironment: { HOME: '/home/tester' },
      });
      await expect(adapter.preflight({
        cwd: '/repo', requestedModel: 'claude-opus-4-1',
      })).rejects.toBeInstanceOf(NativeAuthPreflightError);
    }
  });

  it('classifies a failed bounded invocation as a native host failure', async () => {
    const root = mkdtempSync(join(tmpdir(), 'coding-harness-adapter-'));
    roots.push(root);
    const schemaPath = join(root, 'response.schema.json');
    const outputPath = join(root, 'response.json');
    writeFileSync(schemaPath, JSON.stringify({ type: 'object' }));
    const runner = new FakeRunner(() => ({ ...ok(''), exitCode: 2 }));
    const codex = new CodexSubscriptionAdapter({
      executable: '/tools/codex',
      runner,
      sourceEnvironment: { HOME: '/home/tester', PATH: '/usr/bin' },
      evidenceRoot: root,
    });

    await expect(codex.invoke({
      cwd: root,
      model: 'gpt-5.6-sol',
      prompt: 'review this patch',
      schema: { type: 'object' },
      schemaPath,
      outputPath,
      workspaceAccess: 'read',
      timeoutMs: 1_000,
      operation: 'review',
    })).rejects.toMatchObject({
      name: NativeHostInvocationError.name,
      message: 'HARNESS_NATIVE_HOST_FAILED:codex',
      host: 'codex',
    });
  });

  it('classifies full deadline exhaustion for the bounded same-host retry', async () => {
    const root = mkdtempSync(join(tmpdir(), 'coding-harness-adapter-'));
    roots.push(root);
    const schemaPath = join(root, 'response.schema.json');
    const outputPath = join(root, 'response.json');
    writeFileSync(schemaPath, JSON.stringify({ type: 'object' }));
    const runner = new FakeRunner(() => ({ ...ok(''), timedOut: true }));
    const codex = new CodexSubscriptionAdapter({
      executable: '/tools/codex',
      runner,
      sourceEnvironment: { HOME: '/home/tester', PATH: '/usr/bin' },
      evidenceRoot: root,
    });

    await expect(codex.invoke({
      cwd: root,
      model: 'gpt-5.6-sol',
      prompt: 'review this patch',
      schema: { type: 'object' },
      schemaPath,
      outputPath,
      workspaceAccess: 'read',
      timeoutMs: 1_000,
      operation: 'review',
    })).rejects.toMatchObject({
      name: TransientNativeHostInvocationError.name,
      message: 'HARNESS_NATIVE_HOST_TIMEOUT:codex',
      host: 'codex',
    });
  });
});
