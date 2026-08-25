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
  it('proves both first-party subscriptions without provider fallback', async () => {
    const evidenceRoot = mkdtempSync(join(tmpdir(), 'coding-harness-adapter-'));
    roots.push(evidenceRoot);
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
      model: 'gpt-5.6',
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
      model: 'claude-opus-4-1',
      prompt: 'review this patch',
      schema: { type: 'object' },
      workspaceAccess: 'read',
      timeoutMs: 1_000,
      signal: controller.signal,
      operation: 'review',
    });

    const [codexRequest, claudeRequest] = runner.requests;
    expect(codexRequest?.args).toEqual(
      expect.arrayContaining([
        'exec',
        '--ephemeral',
        '--ignore-user-config',
        '--strict-config',
        '--output-schema',
        schemaPath,
      ]),
    );
    expect(codexRequest?.args.join(' ')).toContain('model_provider="openai"');
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
    expect(() => codex.buildInvocation({
      cwd: root,
      model: 'gpt-5.6',
      prompt: 'escape',
      schema: { type: 'object' },
      schemaPath,
      outputPath: join(root, '..', 'escape.json'),
      workspaceAccess: 'read',
      timeoutMs: 1_000,
      operation: 'review',
    })).toThrow('HARNESS_NATIVE_OUTPUT_PATH_OUTSIDE_CWD');
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
});
