// SPDX-License-Identifier: MIT

import { randomUUID } from 'node:crypto';
import { existsSync } from 'node:fs';
import { resolve } from 'node:path';
import { normalizeWorkspacePath, type HarnessConfig } from './contracts.js';
import {
  isolateNativeFirstPartyModelTraffic,
  type NativeModelOriginPinningBoundary,
  type NativeModelProcessGrant,
} from './network.js';
import { assertNativeSubscriptionEnvironment } from './models/environment.js';
import type {
  NativeHost,
  NativeProcessRequest,
  NativeProcessResult,
  NativeProcessRunner,
} from './models/types.js';
import {
  isolateNativeModelFilesystem,
  type NativeFilesystemIsolationResult,
  type NativeModelFilesystemBoundary,
} from './native-filesystem.js';
import {
  isolateNativeResources,
  limitsForProcessDeadline,
  type NativeResourceBoundary,
  type NativeResourceIsolationResult,
  type NativeResourceLimits,
} from './resource-boundary.js';
import { executeNativeProcess } from './native-process-execution.js';
import { UnixSocketOriginPinningBoundary } from './native-egress.js';
import { SystemNativeFilesystemBoundary } from './native-system-filesystem.js';
import { SystemdResourceBoundary } from './resource-boundary.js';
import {
  assertCapabilityPath,
  cancelledResult,
  digestValue,
  pathsOverlap,
  validateBoundaryPaths,
  validateDirectory,
  validateExecutable,
  validateLimit,
  type NativeExecutableEvidence,
  type NativeExecutionEvidence,
  type OriginCompletion,
} from './native-process-contracts.js';
export type {
  NativeExecutableEvidence,
  NativeExecutionEvidence,
} from './native-process-contracts.js';
export interface NativeRunnerOptions {
  config: HarnessConfig;
  executables: Readonly<Record<NativeHost, string>>;
  allowedRoots: readonly string[];
  allowedReadRoots: readonly string[];
  allowedWriteRoots: readonly string[];
  forbiddenRoots: readonly string[];
  egressBoundary: NativeModelOriginPinningBoundary;
  filesystemBoundary: NativeModelFilesystemBoundary;
  resourceBoundary: NativeResourceBoundary;
  resourceLimits: NativeResourceLimits;
  maskedWorkspacePaths?: readonly string[];
  maxOutputBytes?: number;
  terminationGraceMs?: number;
}
const MAX_STDIN_BYTES = 1_000_000;
const HOST_NETWORK = Object.freeze({
  codex: Object.freeze({
    authentication: 'chatgpt-subscription',
    provider: 'openai',
    knownOrigins: Object.freeze(['https://api.openai.com', 'https://chatgpt.com']),
  }),
  'claude-code': Object.freeze({
    authentication: 'claude-subscription',
    provider: 'anthropic',
    knownOrigins: Object.freeze(['https://api.anthropic.com', 'https://claude.ai']),
  }),
} as const);
export class BoundedNativeProcessRunner implements NativeProcessRunner {
  readonly #config: HarnessConfig;
  readonly #executables: Readonly<Record<NativeHost, string>>;
  readonly #allowedRoots: readonly string[];
  readonly #allowedReadRoots: readonly string[];
  readonly #allowedWriteRoots: readonly string[];
  readonly #forbiddenRoots: readonly string[];
  readonly #egressBoundary: NativeModelOriginPinningBoundary;
  readonly #filesystemBoundary: NativeModelFilesystemBoundary;
  readonly #resourceBoundary: NativeResourceBoundary;
  readonly #resourceLimits: NativeResourceLimits;
  readonly #maskedWorkspacePaths: readonly string[];
  readonly #maxOutputBytes: number;
  readonly #terminationGraceMs: number;
  readonly #networkEvidence: NativeModelProcessGrant[] = [];
  readonly #filesystemEvidence: NativeFilesystemIsolationResult[] = [];
  readonly #resourceEvidence: NativeResourceIsolationResult[] = [];
  readonly #executionEvidence = new Map<string, NativeExecutionEvidence>();
  readonly #executableEvidence: Readonly<Record<NativeHost, NativeExecutableEvidence>>;
  constructor(options: NativeRunnerOptions) {
    this.#config = options.config;
    const codex = validateExecutable(options.executables.codex, 'codex');
    const claude = validateExecutable(options.executables['claude-code'], 'claude-code');
    this.#executables = Object.freeze({ codex: codex.path, 'claude-code': claude.path });
    this.#executableEvidence = Object.freeze({ codex, 'claude-code': claude });
    if (options.allowedRoots.length === 0) throw new Error('HARNESS_NATIVE_ALLOWED_ROOTS_REQUIRED');
    this.#allowedRoots = Object.freeze(options.allowedRoots.map((root) =>
      validateDirectory(root, [])));
    this.#allowedReadRoots = Object.freeze(options.allowedReadRoots.map((root) =>
      validateDirectory(root, [])));
    this.#allowedWriteRoots = Object.freeze(options.allowedWriteRoots.map((root) =>
      validateDirectory(root, [])));
    this.#forbiddenRoots = Object.freeze(options.forbiddenRoots.map((root) =>
      validateDirectory(root, [])));
    if (this.#allowedReadRoots.length === 0 || this.#allowedWriteRoots.length === 0
      || this.#forbiddenRoots.length === 0) {
      throw new Error('HARNESS_NATIVE_CAPABILITY_ROOTS_REQUIRED');
    }
    if ([...this.#allowedRoots, ...this.#allowedReadRoots, ...this.#allowedWriteRoots]
      .some((root) => this.#forbiddenRoots.some((forbidden) => pathsOverlap(root, forbidden)))) {
      throw new Error('HARNESS_NATIVE_CAPABILITY_ROOT_FORBIDDEN');
    }
    this.#egressBoundary = options.egressBoundary;
    if (this.#egressBoundary === undefined) throw new Error('HARNESS_NATIVE_ORIGIN_BOUNDARY_REQUIRED');
    this.#filesystemBoundary = options.filesystemBoundary;
    if (this.#filesystemBoundary === undefined) {
      throw new Error('HARNESS_NATIVE_FILESYSTEM_BOUNDARY_REQUIRED');
    }
    this.#resourceBoundary = options.resourceBoundary;
    if (typeof this.#resourceBoundary?.terminateAndVerify !== 'function') {
      throw new Error('HARNESS_NATIVE_RESOURCE_BOUNDARY_REQUIRED');
    }
    this.#resourceLimits = options.resourceLimits;
    const maskedWorkspacePaths = (options.maskedWorkspacePaths ?? []).map((path, index) =>
      normalizeWorkspacePath(path, `maskedWorkspacePaths[${index}]`));
    if (new Set(maskedWorkspacePaths).size !== maskedWorkspacePaths.length) {
      throw new Error('HARNESS_NATIVE_MASKED_WORKSPACE_PATH_DUPLICATE');
    }
    this.#maskedWorkspacePaths = Object.freeze(maskedWorkspacePaths);
    this.#maxOutputBytes = validateLimit(
      options.maxOutputBytes ?? options.config.limits.maxOutputBytes,
      options.config.limits.maxOutputBytes,
      'maxOutputBytes',
    );
    this.#terminationGraceMs = validateLimit(
      options.terminationGraceMs ?? options.config.limits.terminationGraceMs,
      options.config.limits.terminationGraceMs,
      'terminationGraceMs',
    );
  }
  async run(request: NativeProcessRequest): Promise<NativeProcessResult> {
    const executionId = `native-run:${randomUUID()}`;
    this.#validateRequest(request);
    const executable = validateExecutable(request.executable, request.host);
    if (executable.digest !== this.#executableEvidence[request.host].digest) {
      throw new Error(`HARNESS_NATIVE_EXECUTABLE_CHANGED:${request.host}`);
    }
    const network = await this.#authorizeNetwork(request);
    let filesystem: NativeFilesystemIsolationResult;
    let resources: NativeResourceIsolationResult;
    try {
      const defaultMaskedPaths = [
        '.git', '.mcp.json', '.mcp', '.claude', '.codex', '.agents',
      ].map((path) => resolve(request.cwd, path)).filter(existsSync);
      const requiredMaskedPaths = this.#maskedWorkspacePaths.map((path) => resolve(request.cwd, path));
      if (requiredMaskedPaths.some((path) => !existsSync(path))) {
        throw new Error('HARNESS_NATIVE_REQUIRED_MASK_PATH_MISSING');
      }
      const maskedPaths = [...new Set([...defaultMaskedPaths, ...requiredMaskedPaths])];
      filesystem = isolateNativeModelFilesystem(network.command, {
        host: request.host,
        workspaceRoot: request.cwd,
        readOnlyRoots: [request.cwd, ...(request.readOnlyPaths ?? [])],
        writablePaths: request.writablePaths ?? [],
        maskedPaths,
        hostFileConfidentiality: true,
        emptyPrivateHome: true,
        hostRootMounted: false,
      }, this.#filesystemBoundary);
      resources = isolateNativeResources(
        filesystem.command,
        limitsForProcessDeadline(
          this.#resourceLimits,
          request.timeoutMs,
          this.#terminationGraceMs,
        ),
        this.#resourceBoundary,
      );
    } catch (error) {
      try {
        await this.#completeNetwork(network.command);
      } catch (cleanupError) {
        throw new AggregateError(
          [error, cleanupError],
          'HARNESS_NATIVE_BOUNDARY_SETUP_AND_CLEANUP_FAILED',
        );
      }
      throw error;
    }
    this.#networkEvidence.push(network);
    this.#filesystemEvidence.push(filesystem);
    this.#resourceEvidence.push(resources);
    if (request.signal?.aborted === true) {
      const completion = await this.#completeNetwork(network.command);
      assertOriginPolicy(completion);
      return this.#record(
        executionId,
        request,
        network,
        filesystem,
        resources,
        cancelledResult(executionId),
        completion,
      );
    }
    let result: NativeProcessResult;
    try {
      result = await executeNativeProcess({
        executionId,
        request,
        resources,
        boundary: this.#resourceBoundary,
        maxOutputBytes: this.#maxOutputBytes,
        terminationGraceMs: this.#terminationGraceMs,
      });
    } catch (error) {
      try {
        await this.#completeNetwork(network.command);
      } catch (cleanupError) {
        throw new AggregateError(
          [error, cleanupError],
          'HARNESS_NATIVE_EXECUTION_AND_CLEANUP_FAILED',
        );
      }
      throw error;
    }
    const completion = await this.#completeNetwork(network.command);
    assertOriginPolicy(completion);
    if (request.purpose === 'model-invocation' && completion.allowedConnections < 1
      && nativeProcessSucceeded(result)) {
      throw new Error('HARNESS_NATIVE_ORIGIN_UNUSED');
    }
    return this.#record(
      executionId,
      request,
      network,
      filesystem,
      resources,
      result,
      completion,
    );
  }

  networkEvidence(): readonly NativeModelProcessGrant[] {
    return Object.freeze([...this.#networkEvidence]);
  }
  filesystemEvidence(): readonly NativeFilesystemIsolationResult[] {
    return Object.freeze([...this.#filesystemEvidence]);
  }

  resourceEvidence(): readonly NativeResourceIsolationResult[] {
    return Object.freeze([...this.#resourceEvidence]);
  }
  executionEvidence(executionId: string): NativeExecutionEvidence {
    const evidence = this.#executionEvidence.get(executionId);
    if (evidence === undefined) throw new Error('HARNESS_NATIVE_EXECUTION_EVIDENCE_UNKNOWN');
    return evidence;
  }
  allExecutionEvidence(): readonly NativeExecutionEvidence[] {
    return Object.freeze([...this.#executionEvidence.values()]);
  }

  hasTrustedSystemBoundaries(): boolean {
    return this.#egressBoundary instanceof UnixSocketOriginPinningBoundary
      && this.#filesystemBoundary instanceof SystemNativeFilesystemBoundary
      && this.#resourceBoundary instanceof SystemdResourceBoundary;
  }
  executableEvidence(): Readonly<Record<NativeHost, NativeExecutableEvidence>> {
    return this.#executableEvidence;
  }

  async #completeNetwork(command: import('./network.js').BoundaryCommand): Promise<OriginCompletion> {
    if (this.#egressBoundary.complete === undefined) {
      throw new Error('HARNESS_NATIVE_ORIGIN_COMPLETION_REQUIRED');
    }
    const value = await this.#egressBoundary.complete(command) as Partial<OriginCompletion>;
    if (!Number.isSafeInteger(value.allowedConnections) || (value.allowedConnections ?? -1) < 0
      || !Number.isSafeInteger(value.deniedConnections) || (value.deniedConnections ?? -1) < 0
      || typeof value.connectDigest !== 'string' || !/^[a-f0-9]{64}$/.test(value.connectDigest)) {
      throw new Error('HARNESS_NATIVE_ORIGIN_COMPLETION_INVALID');
    }
    return Object.freeze(value as OriginCompletion);
  }

  #record(
    executionId: string,
    request: NativeProcessRequest,
    network: NativeModelProcessGrant,
    filesystem: NativeFilesystemIsolationResult,
    resources: NativeResourceIsolationResult,
    result: NativeProcessResult,
    completion: OriginCompletion,
  ): NativeProcessResult {
    if (this.#executionEvidence.has(executionId)) throw new Error('HARNESS_NATIVE_EXECUTION_ID_COLLISION');
    this.#executionEvidence.set(executionId, Object.freeze({
      executionId,
      host: request.host,
      purpose: request.purpose,
      model: request.model,
      operation: request.operation ?? null,
      executable: this.#executableEvidence[request.host],
      environmentDigest: digestValue(request.env),
      exitCode: result.exitCode,
      stdoutDigest: result.stdoutDigest,
      stderrDigest: result.stderrDigest,
      network: Object.freeze({
        enforcement: network.enforcement,
        mechanism: network.mechanism,
        pinnedOrigins: [...network.pinnedOrigins],
        ...completion,
      }),
      filesystem: Object.freeze({
        enforcement: filesystem.enforcement,
        mechanism: filesystem.mechanism,
        workspaceRootDigest: digestValue(filesystem.workspaceRoot),
        mountManifestDigest: filesystem.mountManifestDigest,
        configurationMaskDigest: filesystem.configurationMaskDigest,
        hostFileConfidentiality: filesystem.hostFileConfidentiality,
        emptyPrivateHome: filesystem.emptyPrivateHome,
        privateEphemeralHome: filesystem.privateEphemeralHome,
        hostRootMounted: filesystem.hostRootMounted,
        hostCredentialPathMounted: filesystem.hostCredentialPathMounted,
        gitMetadataMasked: filesystem.maskedPaths.includes(resolve(request.cwd, '.git')),
      }),
      resources: Object.freeze({
        enforcement: resources.enforcement,
        mechanism: resources.mechanism,
        limitsDigest: resources.limitsDigest,
      }),
    }));
    return result;
  }

  async #authorizeNetwork(request: NativeProcessRequest): Promise<NativeModelProcessGrant> {
    const profile = HOST_NETWORK[request.host];
    const allowedOrigins = profile.knownOrigins.filter((origin) =>
      this.#config.firstPartyOrigins.includes(origin));
    return await isolateNativeFirstPartyModelTraffic({
      mode: 'first-party-model',
      channel: 'native-subscription-client',
      host: request.host,
      authentication: profile.authentication,
      allowedOrigins,
      environment: request.env,
      transport: {
        client: request.host,
        provider: profile.provider,
        fallback: 'none',
        baseUrl: null,
        proxy: null,
        gateway: null,
      },
      command: {
        executable: request.executable,
        args: request.args,
        cwd: request.cwd,
        env: request.env,
        writablePaths: request.writablePaths ?? [],
      },
    }, this.#config, this.#egressBoundary);
  }

  #validateRequest(request: NativeProcessRequest): void {
    if (request.executable !== this.#executables[request.host]) {
      throw new Error(`HARNESS_NATIVE_EXECUTABLE_MISMATCH:${request.host}`);
    }
    validateDirectory(request.cwd, this.#allowedRoots);
    assertCapabilityPath(request.cwd, this.#allowedReadRoots, this.#forbiddenRoots, false);
    assertNativeSubscriptionEnvironment(request.host, request.env);
    if (!/^[A-Za-z0-9][A-Za-z0-9._:-]{0,199}$/.test(request.model)) {
      throw new Error('HARNESS_NATIVE_MODEL_INVALID');
    }
    if (request.purpose === 'model-invocation') {
      if (!['architecture', 'implementation', 'repair', 'review'].includes(request.operation ?? '')) {
        throw new Error('HARNESS_NATIVE_OPERATION_REQUIRED');
      }
    } else if (request.operation !== undefined) {
      throw new Error('HARNESS_NATIVE_PREFLIGHT_OPERATION_INVALID');
    }
    if (!Number.isSafeInteger(request.timeoutMs)
      || request.timeoutMs < 1
      || request.timeoutMs > this.#config.limits.maxTimeoutMs) {
      throw new TypeError('HARNESS_NATIVE_TIMEOUT_INVALID');
    }
    for (const [index, argument] of request.args.entries()) {
      if (typeof argument !== 'string' || argument.includes('\0')) {
        throw new TypeError(`HARNESS_NATIVE_ARGUMENT_INVALID:${index}`);
      }
    }
    validateBoundaryPaths(
      request.readOnlyPaths ?? [],
      'READ_ONLY',
      this.#allowedReadRoots,
      this.#forbiddenRoots,
      false,
    );
    validateBoundaryPaths(
      request.writablePaths ?? [],
      'WRITABLE',
      this.#allowedWriteRoots,
      this.#forbiddenRoots,
      true,
    );
    if (request.stdin !== undefined
      && Buffer.byteLength(request.stdin, 'utf8') > MAX_STDIN_BYTES) {
      throw new Error('HARNESS_NATIVE_STDIN_TOO_LARGE');
    }
  }
}

function nativeProcessSucceeded(result: NativeProcessResult): boolean {
  return result.exitCode === 0 && !result.timedOut && result.cancelled !== true
    && result.outputLimitExceeded !== true && result.spawnError === undefined;
}

function assertOriginPolicy(completion: OriginCompletion): void {
  if (completion.deniedConnections !== 0) {
    throw new Error('HARNESS_NATIVE_ORIGIN_POLICY_DENIED');
  }
}
