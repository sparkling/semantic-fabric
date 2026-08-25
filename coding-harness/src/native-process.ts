// SPDX-License-Identifier: MIT

import { spawn } from 'node:child_process';
import type { ChildProcessByStdio } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { existsSync } from 'node:fs';
import { resolve } from 'node:path';
import type { Readable, Writable } from 'node:stream';
import type { HarnessConfig } from './contracts.js';
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
  type NativeResourceBoundary,
  type NativeResourceIsolationResult,
  type NativeResourceLimits,
} from './resource-boundary.js';
import { UnixSocketOriginPinningBoundary } from './native-egress.js';
import { SystemNativeFilesystemBoundary } from './native-system-filesystem.js';
import { SystemdResourceBoundary } from './resource-boundary.js';
import {
  assertCapabilityPath,
  cancelledResult,
  digestValue,
  errorMessage,
  pathsOverlap,
  signalProcessGroup,
  spawnFailure,
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
  maxOutputBytes?: number;
  terminationGraceMs?: number;
}

type NativeChild = ChildProcessByStdio<Writable, Readable, Readable>;

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
    if (this.#resourceBoundary === undefined) {
      throw new Error('HARNESS_NATIVE_RESOURCE_BOUNDARY_REQUIRED');
    }
    this.#resourceLimits = options.resourceLimits;
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
      const maskedPaths = [
        '.git', '.mcp.json', '.mcp', '.claude', '.codex', '.agents',
      ].map((path) => resolve(request.cwd, path)).filter(existsSync);
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
        this.#resourceLimits,
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

    const result = await new Promise<NativeProcessResult>((resolveResult) => {
      let child: NativeChild;
      try {
        child = spawn(resources.command.executable, [...resources.command.args], {
          cwd: resources.command.cwd,
          env: {
            ...(this.#resourceBoundary.launchEnvironment?.(resources.command.env)
              ?? resources.command.env),
          },
          shell: false,
          detached: process.platform !== 'win32',
          stdio: ['pipe', 'pipe', 'pipe'],
        });
      } catch (error) {
        resolveResult(spawnFailure(executionId, error));
        return;
      }

      const stdout: Buffer[] = [];
      const stderr: Buffer[] = [];
      let capturedBytes = 0;
      let observedBytes = 0;
      let timedOut = false;
      let cancelled = false;
      let outputLimitExceeded = false;
      let spawnError: string | undefined;
      let killTimer: NodeJS.Timeout | undefined;
      let settled = false;

      const terminate = () => {
        signalProcessGroup(child, 'SIGTERM');
        killTimer ??= setTimeout(
          () => signalProcessGroup(child, 'SIGKILL'),
          this.#terminationGraceMs,
        );
        killTimer.unref();
      };
      const capture = (target: Buffer[], chunk: Buffer) => {
        observedBytes += chunk.length;
        const room = Math.max(0, this.#maxOutputBytes - capturedBytes);
        if (room > 0) {
          const kept = chunk.subarray(0, room);
          target.push(kept);
          capturedBytes += kept.length;
        }
        if (observedBytes > this.#maxOutputBytes && !outputLimitExceeded) {
          outputLimitExceeded = true;
          terminate();
        }
      };
      child.stdout.on('data', (chunk: Buffer) => capture(stdout, chunk));
      child.stderr.on('data', (chunk: Buffer) => capture(stderr, chunk));
      child.on('error', (error) => {
        spawnError = errorMessage(error);
      });
      child.stdin.on('error', () => {
        // EPIPE is reflected by process failure or cancellation evidence.
      });
      child.stdin.end(request.stdin ?? '');

      const timeout = setTimeout(() => {
        timedOut = true;
        terminate();
      }, request.timeoutMs);
      timeout.unref();
      const abort = () => {
        cancelled = true;
        terminate();
      };
      request.signal?.addEventListener('abort', abort, { once: true });

      child.on('close', (exitCode) => {
        if (settled) return;
        settled = true;
        clearTimeout(timeout);
        if (killTimer) clearTimeout(killTimer);
        request.signal?.removeEventListener('abort', abort);
        resolveResult(Object.freeze({
          executionId,
          exitCode,
          stdout: Buffer.concat(stdout).toString('utf8'),
          stderr: Buffer.concat(stderr).toString('utf8'),
          timedOut,
          cancelled,
          outputLimitExceeded,
          ...(spawnError === undefined ? {} : { spawnError }),
          stdoutDigest: digestValue(Buffer.concat(stdout)),
          stderrDigest: digestValue(Buffer.concat(stderr)),
        }));
      });
    });
    const completion = await this.#completeNetwork(network.command);
    if (request.purpose === 'model-invocation'
      && (completion.allowedConnections < 1 || completion.deniedConnections !== 0)) {
      throw new Error('HARNESS_NATIVE_ORIGIN_ENFORCEMENT_FAILED');
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
        hostRootMounted: filesystem.hostRootMounted,
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
