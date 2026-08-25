// SPDX-License-Identifier: MIT

import {
  assertNativeSubscriptionEnvironment,
  buildNativeSubscriptionEnvironment,
} from './environment.js';
import {
  NativeCancellationError,
  TransientNativeHostError,
} from './recovery.js';
import type {
  ClaudeInvocationRequest,
  CodexInvocationRequest,
  NativeAuthEvidence,
  NativePreflightRequest,
  NativeProcessRequest,
  NativeProcessResult,
  NativeProcessRunner,
  NativeSubscriptionAdapter,
} from './types.js';
import {
  assertAdapterExecutable,
  parseRecord,
  processSucceeded,
  signalAborted,
  validateInvocation,
  validatePreflightRequest,
  validateRoot,
  validateScopedPath,
} from './native-adapter-contracts.js';
import { runAbortableCohort } from '../parallel.js';

const PREFLIGHT_TIMEOUT_MS = 30_000;
const MAX_SCHEMA_BYTES = 64_000;
const CODEX_ESSENTIAL_TRAFFIC_CONFIG = Object.freeze([
  'analytics.enabled=false',
  'otel.metrics_exporter="none"',
] as const);

const CODEX_FIXED_CONFIG = Object.freeze([
  'model_provider="openai"',
  'model_reasoning_effort="low"',
  'approval_policy="never"',
  ...CODEX_ESSENTIAL_TRAFFIC_CONFIG,
  'project_doc_max_bytes=0',
  'project_doc_fallback_filenames=[]',
  'web_search="disabled"',
  'agents.enabled=false',
  'apps._default.enabled=false',
  'mcp_servers={}',
  'features.apps=false',
  'features.auth_elicitation=false',
  'features.browser_use=false',
  'features.computer_use=false',
  'features.hooks=false',
  'features.image_generation=false',
  'features.multi_agent=false',
  'features.plugins=false',
  'features.remote_plugin=false',
  'features.skill_search=false',
  'features.shell_tool=false',
  'features.unified_exec=false',
  'features.code_mode=false',
  'features.code_mode_only=false',
  'features.standalone_web_search=false',
] as const);

interface AdapterOptions {
  readonly executable: string;
  readonly runner: NativeProcessRunner;
  readonly sourceEnvironment: Readonly<Record<string, string | undefined>>;
}

interface CodexAdapterOptions extends AdapterOptions {
  readonly evidenceRoot: string;
}

export class NativeAuthPreflightError extends Error {
  readonly host: 'codex' | 'claude-code';

  constructor(host: 'codex' | 'claude-code', message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = 'NativeAuthPreflightError';
    this.host = host;
  }
}

export class NativeHostInvocationError extends Error {
  readonly host: 'codex' | 'claude-code';

  constructor(host: 'codex' | 'claude-code', message: string) {
    super(message);
    this.name = 'NativeHostInvocationError';
    this.host = host;
  }
}

export class CodexSubscriptionAdapter implements NativeSubscriptionAdapter {
  readonly host = 'codex' as const;
  readonly #executable: string;
  readonly #runner: NativeProcessRunner;
  readonly #environment: Readonly<Record<string, string>>;
  readonly #evidenceRoot: string;

  constructor(options: CodexAdapterOptions) {
    assertAdapterExecutable(options.executable);
    this.#executable = options.executable;
    this.#runner = options.runner;
    this.#environment = buildNativeSubscriptionEnvironment(
      this.host,
      options.sourceEnvironment,
    );
    this.#evidenceRoot = validateRoot(options.evidenceRoot, 'EVIDENCE_ROOT');
  }

  async preflight(request: NativePreflightRequest): Promise<NativeAuthEvidence> {
    validatePreflightRequest(request);
    const [login, version] = await runAbortableCohort([
      async (cohortSignal) => await this.#runner.run(
        this.#processRequest(
          [
            ...CODEX_ESSENTIAL_TRAFFIC_CONFIG.flatMap((value) => ['-c', value]),
            '-c', 'model_provider="openai"', 'login', 'status',
          ],
          request.cwd,
          PREFLIGHT_TIMEOUT_MS,
          cohortSignal,
          'authentication-preflight',
          request.requestedModel,
        ),
      ),
      async (cohortSignal) => await this.#runner.run(
        this.#processRequest(
          [
            ...CODEX_ESSENTIAL_TRAFFIC_CONFIG.flatMap((value) => ['-c', value]),
            '--version',
          ],
          request.cwd,
          PREFLIGHT_TIMEOUT_MS,
          cohortSignal,
          'version-preflight',
          request.requestedModel,
        ),
      ),
    ] as const, request.signal);
    const status = `${login.stdout}${login.stderr}`.trim();
    if (!processSucceeded(login) || status !== 'Logged in using ChatGPT') {
      throw new NativeAuthPreflightError(
        this.host,
        'HARNESS_CODEX_SUBSCRIPTION_UNAVAILABLE',
      );
    }
    const clientVersion = captureVersion(version, this.host);
    return Object.freeze({
      host: this.host,
      requestedModel: request.requestedModel,
      authentication: 'chatgpt-subscription',
      clientVersion,
      fallback: 'none',
      subscriptionCostUsd: 0,
      preflightExecutionIds: [login.executionId, version.executionId] as const,
    });
  }

  buildInvocation(request: CodexInvocationRequest): NativeProcessRequest {
    validateInvocation(request);
    validateScopedPath(this.#evidenceRoot, request.schemaPath, 'SCHEMA_PATH', true);
    validateScopedPath(this.#evidenceRoot, request.outputPath, 'OUTPUT_PATH', false);
    if (request.schemaPath === request.outputPath) {
      throw new Error('HARNESS_NATIVE_OUTPUT_PATH_INVALID');
    }
    const args = [
      'exec',
      '--cd',
      request.cwd,
      '--ephemeral',
      '--ignore-user-config',
      '--ignore-rules',
      '--strict-config',
      '--skip-git-repo-check',
      '--sandbox',
      request.workspaceAccess === 'read' ? 'read-only' : 'workspace-write',
      '--model',
      request.model,
      '--output-schema',
      request.schemaPath,
      '--output-last-message',
      request.outputPath,
      '--json',
      '--color',
      'never',
      ...CODEX_FIXED_CONFIG.flatMap((value) => ['-c', value]),
      '-',
    ];
    return this.#processRequest(
      args,
      request.cwd,
      request.timeoutMs,
      request.signal,
      'model-invocation',
      request.model,
      request.operation,
      `${request.prompt}\n`,
      [request.schemaPath],
      [request.outputPath],
    );
  }

  async invoke(request: CodexInvocationRequest): Promise<NativeProcessResult> {
    return await invokeChecked(this.#runner, this.buildInvocation(request));
  }

  #processRequest(
    args: readonly string[],
    cwd: string,
    timeoutMs: number,
    signal?: AbortSignal,
    purpose: NativeProcessRequest['purpose'] = 'model-invocation',
    model = 'unknown',
    operation?: NativeProcessRequest['operation'],
    stdin?: string,
    readOnlyPaths?: readonly string[],
    writablePaths?: readonly string[],
  ): NativeProcessRequest {
    assertNativeSubscriptionEnvironment(this.host, this.#environment);
    return Object.freeze({
      host: this.host,
      purpose,
      model,
      ...(operation === undefined ? {} : { operation }),
      executable: this.#executable,
      args: Object.freeze([...args]),
      cwd,
      env: this.#environment,
      timeoutMs,
      ...(stdin === undefined ? {} : { stdin }),
      ...(readOnlyPaths === undefined ? {} : { readOnlyPaths }),
      ...(writablePaths === undefined ? {} : { writablePaths }),
      ...(signal === undefined ? {} : { signal }),
    });
  }
}

export class ClaudeCodeSubscriptionAdapter implements NativeSubscriptionAdapter {
  readonly host = 'claude-code' as const;
  readonly #executable: string;
  readonly #runner: NativeProcessRunner;
  readonly #environment: Readonly<Record<string, string>>;

  constructor(options: AdapterOptions) {
    assertAdapterExecutable(options.executable);
    this.#executable = options.executable;
    this.#runner = options.runner;
    this.#environment = buildNativeSubscriptionEnvironment(
      this.host,
      options.sourceEnvironment,
    );
  }

  async preflight(request: NativePreflightRequest): Promise<NativeAuthEvidence> {
    validatePreflightRequest(request);
    const [login, version] = await runAbortableCohort([
      async (cohortSignal) => await this.#runner.run(
        this.#processRequest(
          ['auth', 'status', '--json'],
          request.cwd,
          PREFLIGHT_TIMEOUT_MS,
          cohortSignal,
          'authentication-preflight',
          request.requestedModel,
        ),
      ),
      async (cohortSignal) => await this.#runner.run(
        this.#processRequest(
          ['--version'],
          request.cwd,
          PREFLIGHT_TIMEOUT_MS,
          cohortSignal,
          'version-preflight',
          request.requestedModel,
        ),
      ),
    ] as const, request.signal);
    const status = parseRecord(login.stdout);
    if (
      !processSucceeded(login) ||
      status?.loggedIn !== true ||
      status.authMethod !== 'claude.ai' ||
      status.apiProvider !== 'firstParty' ||
      (status.apiKeySource !== undefined && status.apiKeySource !== null)
    ) {
      throw new NativeAuthPreflightError(
        this.host,
        'HARNESS_CLAUDE_SUBSCRIPTION_UNAVAILABLE',
      );
    }
    const clientVersion = captureVersion(version, this.host);
    return Object.freeze({
      host: this.host,
      requestedModel: request.requestedModel,
      authentication: 'claude-subscription',
      clientVersion,
      fallback: 'none',
      subscriptionCostUsd: 0,
      preflightExecutionIds: [login.executionId, version.executionId] as const,
    });
  }

  buildInvocation(request: ClaudeInvocationRequest): NativeProcessRequest {
    validateInvocation(request);
    const schema = JSON.stringify(request.schema);
    if (Buffer.byteLength(schema, 'utf8') > MAX_SCHEMA_BYTES) {
      throw new Error('HARNESS_NATIVE_SCHEMA_TOO_LARGE');
    }
    const toolArguments =
      request.workspaceAccess === 'write'
        ? [
            '--permission-mode',
            'acceptEdits',
            '--tools',
            '',
            '--disallowedTools',
            'Bash,WebFetch,WebSearch,Task',
          ]
        : [
            '--permission-mode',
            'dontAsk',
            '--tools',
            '',
            '--disallowedTools',
            'Edit,Write,Bash,WebFetch,WebSearch,Task',
          ];
    return this.#processRequest(
      [
        '-p',
        '--model',
        request.model,
        '--effort',
        'high',
        '--input-format',
        'text',
        '--output-format',
        'json',
        '--json-schema',
        schema,
        ...toolArguments,
        '--mcp-config',
        '{"mcpServers":{}}',
        '--strict-mcp-config',
        '--no-session-persistence',
        '--safe-mode',
        '--disable-slash-commands',
        '--no-chrome',
      ],
      request.cwd,
      request.timeoutMs,
      request.signal,
      'model-invocation',
      request.model,
      request.operation,
      `${request.prompt}\n`,
    );
  }

  async invoke(request: ClaudeInvocationRequest): Promise<NativeProcessResult> {
    return await invokeChecked(this.#runner, this.buildInvocation(request));
  }

  #processRequest(
    args: readonly string[],
    cwd: string,
    timeoutMs: number,
    signal?: AbortSignal,
    purpose: NativeProcessRequest['purpose'] = 'model-invocation',
    model = 'unknown',
    operation?: NativeProcessRequest['operation'],
    stdin?: string,
  ): NativeProcessRequest {
    assertNativeSubscriptionEnvironment(this.host, this.#environment);
    return Object.freeze({
      host: this.host,
      purpose,
      model,
      ...(operation === undefined ? {} : { operation }),
      executable: this.#executable,
      args: Object.freeze([...args]),
      cwd,
      env: this.#environment,
      timeoutMs,
      ...(stdin === undefined ? {} : { stdin }),
      ...(signal === undefined ? {} : { signal }),
    });
  }
}

export async function preflightNativeSubscriptions(input: {
  readonly codex: CodexSubscriptionAdapter;
  readonly claude: ClaudeCodeSubscriptionAdapter;
  readonly cwd: string;
  readonly requestedModels: Readonly<{ codex: string; claude: string }>;
  readonly signal?: AbortSignal;
}): Promise<readonly [NativeAuthEvidence, NativeAuthEvidence]> {
  const evidence = await runAbortableCohort([
    async (cohortSignal) => await input.codex.preflight({
      cwd: input.cwd,
      requestedModel: input.requestedModels.codex,
      signal: cohortSignal,
    }),
    async (cohortSignal) => await input.claude.preflight({
      cwd: input.cwd,
      requestedModel: input.requestedModels.claude,
      signal: cohortSignal,
    }),
  ] as const, input.signal);
  if (evidence[0].host === evidence[1].host) {
    throw new Error('HARNESS_NATIVE_PREFLIGHT_HOST_COLLISION');
  }
  return Object.freeze(evidence) as readonly [NativeAuthEvidence, NativeAuthEvidence];
}

async function invokeChecked(
  runner: NativeProcessRunner,
  request: NativeProcessRequest,
): Promise<NativeProcessResult> {
  if (signalAborted(request.signal)) throw new NativeCancellationError();
  const result = await runner.run(request);
  if (signalAborted(request.signal) || result.cancelled === true) {
    throw new NativeCancellationError();
  }
  if (result.timedOut) {
    throw new TransientNativeHostError(
      `HARNESS_NATIVE_HOST_TIMEOUT:${request.host}`,
    );
  }
  if (!processSucceeded(result)) {
    throw new NativeHostInvocationError(
      request.host,
      `HARNESS_NATIVE_HOST_FAILED:${request.host}`,
    );
  }
  return result;
}

function captureVersion(
  result: NativeProcessResult,
  host: 'codex' | 'claude-code',
): string {
  const version = `${result.stdout}${result.stderr}`.trim();
  if (!processSucceeded(result) || version.length === 0 || version.length > 500) {
    throw new NativeAuthPreflightError(
      host,
      `HARNESS_NATIVE_VERSION_UNAVAILABLE:${host}`,
    );
  }
  return version;
}
