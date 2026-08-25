// SPDX-License-Identifier: MIT

import { isAbsolute, relative, resolve, sep } from 'node:path';
import { existsSync, realpathSync, statSync } from 'node:fs';
import { resolveWorkspacePath } from '../workspace.js';
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

const PREFLIGHT_TIMEOUT_MS = 30_000;
const MAX_PROMPT_BYTES = 1_000_000;
const MAX_SCHEMA_BYTES = 64_000;

const CODEX_FIXED_CONFIG = Object.freeze([
  'model_provider="openai"',
  'approval_policy="never"',
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
    assertAdapterOptions(options);
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
    const [login, version] = await Promise.all([
      this.#runner.run(
        this.#processRequest(
          ['-c', 'model_provider="openai"', 'login', 'status'],
          request.cwd,
          PREFLIGHT_TIMEOUT_MS,
          request.signal,
        ),
      ),
      this.#runner.run(
        this.#processRequest(
          ['--version'],
          request.cwd,
          PREFLIGHT_TIMEOUT_MS,
          request.signal,
        ),
      ),
    ]);
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
    stdin?: string,
    readOnlyPaths?: readonly string[],
    writablePaths?: readonly string[],
  ): NativeProcessRequest {
    assertNativeSubscriptionEnvironment(this.host, this.#environment);
    return Object.freeze({
      host: this.host,
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
    assertAdapterOptions(options);
    this.#executable = options.executable;
    this.#runner = options.runner;
    this.#environment = buildNativeSubscriptionEnvironment(
      this.host,
      options.sourceEnvironment,
    );
  }

  async preflight(request: NativePreflightRequest): Promise<NativeAuthEvidence> {
    validatePreflightRequest(request);
    const [login, version] = await Promise.all([
      this.#runner.run(
        this.#processRequest(
          ['auth', 'status', '--json'],
          request.cwd,
          PREFLIGHT_TIMEOUT_MS,
          request.signal,
        ),
      ),
      this.#runner.run(
        this.#processRequest(
          ['--version'],
          request.cwd,
          PREFLIGHT_TIMEOUT_MS,
          request.signal,
        ),
      ),
    ]);
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
            'Read,Edit,Write,Glob,Grep',
            '--disallowedTools',
            'Bash,WebFetch,WebSearch,Task',
          ]
        : [
            '--permission-mode',
            'dontAsk',
            '--tools',
            'Read,Glob,Grep',
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
        '--strict-mcp-config',
        '--no-session-persistence',
        '--safe-mode',
        '--disable-slash-commands',
        '--no-chrome',
      ],
      request.cwd,
      request.timeoutMs,
      request.signal,
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
    stdin?: string,
  ): NativeProcessRequest {
    assertNativeSubscriptionEnvironment(this.host, this.#environment);
    return Object.freeze({
      host: this.host,
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
  const evidence = await Promise.all([
    input.codex.preflight({
      cwd: input.cwd,
      requestedModel: input.requestedModels.codex,
      signal: input.signal,
    }),
    input.claude.preflight({
      cwd: input.cwd,
      requestedModel: input.requestedModels.claude,
      signal: input.signal,
    }),
  ]);
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

function assertAdapterOptions(options: AdapterOptions): void {
  if (options.executable.trim().length === 0 || options.executable.includes('\0')) {
    throw new Error('HARNESS_NATIVE_EXECUTABLE_INVALID');
  }
}

function validatePreflightRequest(request: NativePreflightRequest): void {
  validateAbsolutePath(request.cwd, 'CWD');
  validateModel(request.requestedModel);
}

function validateInvocation(request: ClaudeInvocationRequest): void {
  validateAbsolutePath(request.cwd, 'CWD');
  validateModel(request.model);
  if (
    request.prompt.trim().length === 0 ||
    request.prompt.includes('\0') ||
    Buffer.byteLength(request.prompt, 'utf8') > MAX_PROMPT_BYTES
  ) {
    throw new Error('HARNESS_NATIVE_PROMPT_INVALID');
  }
  if (
    request.schema === null ||
    Array.isArray(request.schema) ||
    !Number.isSafeInteger(request.timeoutMs) ||
    request.timeoutMs < 1
  ) {
    throw new Error('HARNESS_NATIVE_INVOCATION_INVALID');
  }
}

function validateModel(model: string): void {
  if (!/^[A-Za-z0-9][A-Za-z0-9._:-]{0,199}$/.test(model)) {
    throw new Error('HARNESS_NATIVE_MODEL_INVALID');
  }
}

function validateAbsolutePath(path: string, field: string): void {
  if (!isAbsolute(path) || resolve(path) !== path || path.includes('\0')) {
    throw new Error(`HARNESS_NATIVE_${field}_INVALID`);
  }
}

function validateScopedPath(
  root: string,
  path: string,
  field: string,
  requireFile: boolean,
): void {
  validateAbsolutePath(root, 'CWD');
  validateAbsolutePath(path, field);
  const delta = relative(root, path);
  if (delta === '' || delta === '..' || delta.startsWith(`..${sep}`) || isAbsolute(delta)) {
    throw new Error(`HARNESS_NATIVE_${field}_OUTSIDE_CWD`);
  }
  const workspacePath = delta.split(sep).join('/');
  try {
    const absolute = resolveWorkspacePath(root, workspacePath, requireFile
      ? { requireRegularFile: true, rejectHardlinks: true }
      : { allowMissingLeaf: true });
    if (!requireFile && existsSync(absolute)) {
      resolveWorkspacePath(root, workspacePath, {
        requireRegularFile: true,
        rejectHardlinks: true,
      });
    }
  } catch (error) {
    throw new Error(`HARNESS_NATIVE_${field}_INVALID`, { cause: error });
  }
}

function validateRoot(path: string, field: string): string {
  validateAbsolutePath(path, field);
  if (realpathSync(path) !== path || !statSync(path).isDirectory()) {
    throw new Error(`HARNESS_NATIVE_${field}_INVALID`);
  }
  return path;
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

function processSucceeded(result: NativeProcessResult): boolean {
  return result.exitCode === 0 && !result.timedOut && result.cancelled !== true
    && result.outputLimitExceeded !== true && result.spawnError === undefined;
}
function signalAborted(signal?: AbortSignal): boolean {
  return signal?.aborted === true;
}
function parseRecord(text: string): Record<string, unknown> | null {
  try {
    const value = JSON.parse(text) as unknown;
    return value !== null && typeof value === 'object' && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}
