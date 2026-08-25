// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  ClaudeCodeSubscriptionAdapter,
  CodexSubscriptionAdapter,
  NativeAuthPreflightError,
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
    return await this.respond(request);
  }
}

const ok = (stdout: string): NativeProcessResult => ({
  exitCode: 0,
  stdout,
  stderr: '',
  timedOut: false,
});

describe('native subscription adapters', () => {
  it('proves both first-party subscriptions without provider fallback', async () => {
    const runner = new FakeRunner((request) => {
      if (request.executable === '/tools/codex') {
        return ok(request.args.includes('--version') ? 'codex-cli 1.2.3' : 'Logged in using ChatGPT');
      }
      return ok(
        request.args.includes('--version')
          ? 'claude-code 4.5.6'
          : JSON.stringify({
              loggedIn: true,
              authMethod: 'claude.ai',
              apiProvider: 'firstParty',
              apiKeySource: null,
            }),
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
      requestedModels: { codex: 'gpt-5.6', claude: 'claude-opus-4-1' },
    });

    expect(evidence.map(({ host }) => host)).toEqual(['codex', 'claude-code']);
    expect(evidence.map(({ authentication }) => authentication)).toEqual([
      'chatgpt-subscription',
      'claude-subscription',
    ]);
    expect(runner.requests).toHaveLength(4);
    for (const request of runner.requests) {
      expect(request.env.OPENAI_API_KEY).toBeUndefined();
      expect(request.env.ANTHROPIC_BASE_URL).toBeUndefined();
      expect(request.env.HTTPS_PROXY).toBeUndefined();
    }
  });

  it('builds ephemeral structured invocations with cancellation propagation', async () => {
    const runner = new FakeRunner(() => ok('{}'));
    const controller = new AbortController();
    const codex = new CodexSubscriptionAdapter({
      executable: '/tools/codex',
      runner,
      sourceEnvironment: { HOME: '/home/tester', PATH: '/usr/bin' },
    });
    const claude = new ClaudeCodeSubscriptionAdapter({
      executable: '/tools/claude',
      runner,
      sourceEnvironment: { HOME: '/home/tester', PATH: '/usr/bin' },
    });

    await codex.invoke({
      cwd: '/repo',
      model: 'gpt-5.6',
      prompt: 'review this patch',
      schema: { type: 'object' },
      schemaPath: '/run/response.schema.json',
      outputPath: '/run/response.json',
      workspaceAccess: 'read',
      timeoutMs: 1_000,
      signal: controller.signal,
    });
    await claude.invoke({
      cwd: '/repo',
      model: 'claude-opus-4-1',
      prompt: 'review this patch',
      schema: { type: 'object' },
      workspaceAccess: 'read',
      timeoutMs: 1_000,
      signal: controller.signal,
    });

    const [codexRequest, claudeRequest] = runner.requests;
    expect(codexRequest?.args).toEqual(
      expect.arrayContaining([
        'exec',
        '--ephemeral',
        '--ignore-user-config',
        '--strict-config',
        '--output-schema',
        '/run/response.schema.json',
      ]),
    );
    expect(codexRequest?.args.join(' ')).toContain('model_provider="openai"');
    expect(claudeRequest?.args).toEqual(
      expect.arrayContaining([
        '-p',
        '--strict-mcp-config',
        '--no-session-persistence',
        '--safe-mode',
      ]),
    );
    expect(claudeRequest?.args).not.toContain('WebSearch');
    expect(codexRequest?.signal).toBe(controller.signal);
    expect(claudeRequest?.signal).toBe(controller.signal);
  });

  it('rejects Claude auth evidence backed by an API key', async () => {
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
});
