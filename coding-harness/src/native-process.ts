// SPDX-License-Identifier: MIT

import { spawn } from 'node:child_process';
import type { ChildProcessByStdio } from 'node:child_process';
import { createHash } from 'node:crypto';
import { lstatSync, readFileSync, realpathSync, statSync } from 'node:fs';
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path';
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

export interface NativeRunnerOptions {
  config: HarnessConfig;
  executables: Readonly<Record<NativeHost, string>>;
  allowedRoots: readonly string[];
  allowedReadRoots: readonly string[];
  allowedWriteRoots: readonly string[];
  forbiddenRoots: readonly string[];
  egressBoundary: NativeModelOriginPinningBoundary;
  filesystemBoundary: NativeModelFilesystemBoundary;
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
  readonly #maxOutputBytes: number;
  readonly #terminationGraceMs: number;
  readonly #networkEvidence: NativeModelProcessGrant[] = [];
  readonly #filesystemEvidence: NativeFilesystemIsolationResult[] = [];
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
    this.#validateRequest(request);
    const executable = validateExecutable(request.executable, request.host);
    if (executable.digest !== this.#executableEvidence[request.host].digest) {
      throw new Error(`HARNESS_NATIVE_EXECUTABLE_CHANGED:${request.host}`);
    }
    const network = this.#authorizeNetwork(request);
    const filesystem = isolateNativeModelFilesystem(network.command, {
      host: request.host,
      workspaceRoot: request.cwd,
      readOnlyRoots: [request.cwd, ...(request.readOnlyPaths ?? [])],
      writablePaths: request.writablePaths ?? [],
      maskedPaths: [resolve(request.cwd, '.git')],
      hostFileConfidentiality: true,
      emptyPrivateHome: true,
      hostRootMounted: false,
    }, this.#filesystemBoundary);
    this.#networkEvidence.push(network);
    this.#filesystemEvidence.push(filesystem);
    if (request.signal?.aborted === true) return cancelledResult();

    return await new Promise((resolveResult) => {
      let child: NativeChild;
      try {
        child = spawn(filesystem.command.executable, [...filesystem.command.args], {
          cwd: filesystem.command.cwd,
          env: { ...filesystem.command.env },
          shell: false,
          detached: process.platform !== 'win32',
          stdio: ['pipe', 'pipe', 'pipe'],
        });
      } catch (error) {
        resolveResult(spawnFailure(error));
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
          exitCode,
          stdout: Buffer.concat(stdout).toString('utf8'),
          stderr: Buffer.concat(stderr).toString('utf8'),
          timedOut,
          cancelled,
          outputLimitExceeded,
          ...(spawnError === undefined ? {} : { spawnError }),
        }));
      });
    });
  }

  networkEvidence(): readonly NativeModelProcessGrant[] {
    return Object.freeze([...this.#networkEvidence]);
  }

  filesystemEvidence(): readonly NativeFilesystemIsolationResult[] {
    return Object.freeze([...this.#filesystemEvidence]);
  }

  executableEvidence(): Readonly<Record<NativeHost, NativeExecutableEvidence>> {
    return this.#executableEvidence;
  }

  #authorizeNetwork(request: NativeProcessRequest): NativeModelProcessGrant {
    const profile = HOST_NETWORK[request.host];
    const allowedOrigins = profile.knownOrigins.filter((origin) =>
      this.#config.firstPartyOrigins.includes(origin));
    return isolateNativeFirstPartyModelTraffic({
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

export interface NativeExecutableEvidence {
  readonly path: string;
  readonly digest: string;
}

function validateExecutable(value: string, host: NativeHost): NativeExecutableEvidence {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) {
    throw new TypeError(`HARNESS_NATIVE_EXECUTABLE_INVALID:${host}`);
  }
  const stat = lstatSync(value);
  const currentUid = typeof process.getuid === 'function' ? process.getuid() : stat.uid;
  if (!stat.isFile() || stat.isSymbolicLink() || realpathSync(value) !== value
    || stat.nlink !== 1 || (stat.mode & 0o111) === 0 || (stat.mode & 0o022) !== 0
    || (stat.uid !== 0 && stat.uid !== currentUid)) {
    throw new Error(`HARNESS_NATIVE_EXECUTABLE_UNTRUSTED:${host}`);
  }
  return Object.freeze({
    path: value,
    digest: createHash('sha256').update(readFileSync(value)).digest('hex'),
  });
}

function validateBoundaryPaths(
  paths: readonly string[],
  label: string,
  allowedRoots: readonly string[],
  forbiddenRoots: readonly string[],
  writable: boolean,
): void {
  if (new Set(paths).size !== paths.length) throw new Error(`HARNESS_NATIVE_${label}_PATH_DUPLICATE`);
  for (const path of paths) {
    if (!isAbsolute(path) || resolve(path) !== path || path.includes('\0')) {
      throw new Error(`HARNESS_NATIVE_${label}_PATH_INVALID`);
    }
    assertCapabilityPath(path, allowedRoots, forbiddenRoots, writable);
  }
}

function assertCapabilityPath(
  path: string,
  allowedRoots: readonly string[],
  forbiddenRoots: readonly string[],
  writable: boolean,
): void {
  const stat = lstatSync(path);
  if (stat.isSymbolicLink() || realpathSync(path) !== path
    || (stat.isFile() && stat.nlink !== 1)
    || (!stat.isFile() && !stat.isDirectory())) {
    throw new Error('HARNESS_NATIVE_CAPABILITY_PATH_UNTRUSTED');
  }
  const parent = realpathSync(dirname(path));
  if (!allowedRoots.some((root) => path === root
      || (contains(root, path) && contains(root, parent)))
    || forbiddenRoots.some((root) => contains(root, path) || contains(path, root))) {
    throw new Error('HARNESS_NATIVE_CAPABILITY_PATH_OUTSIDE_SCOPE');
  }
  if (writable && stat.isDirectory()) throw new Error('HARNESS_NATIVE_WRITABLE_PATH_MUST_BE_FILE');
}

function pathsOverlap(left: string, right: string): boolean {
  return contains(left, right) || contains(right, left);
}

function validateDirectory(value: string, allowedRoots: readonly string[]): string {
  if (!isAbsolute(value) || resolve(value) !== value || realpathSync(value) !== value || !statSync(value).isDirectory()) {
    throw new TypeError('HARNESS_NATIVE_CWD_INVALID');
  }
  if (allowedRoots.length > 0 && !allowedRoots.some((root) => contains(root, value))) {
    throw new Error('HARNESS_NATIVE_CWD_OUTSIDE_ALLOWED_ROOTS');
  }
  return value;
}

function contains(root: string, path: string): boolean {
  const delta = relative(root, path);
  return delta === '' || (delta !== '..' && !delta.startsWith(`..${sep}`) && !isAbsolute(delta));
}

function validateLimit(value: number, ceiling: number, label: string): number {
  if (!Number.isSafeInteger(value) || value < 1 || value > ceiling) {
    throw new TypeError(`${label} must be a safe integer within the configured ceiling`);
  }
  return value;
}

function signalProcessGroup(child: NativeChild, signal: NodeJS.Signals): void {
  if (child.pid === undefined) return;
  try {
    if (process.platform !== 'win32') process.kill(-child.pid, signal);
    else child.kill(signal);
  } catch {
    try {
      child.kill(signal);
    } catch {
      // The process already exited.
    }
  }
}

function cancelledResult(): NativeProcessResult {
  return Object.freeze({
    exitCode: null,
    stdout: '',
    stderr: '',
    timedOut: false,
    cancelled: true,
    outputLimitExceeded: false,
  });
}

function spawnFailure(error: unknown): NativeProcessResult {
  return Object.freeze({
    exitCode: null,
    stdout: '',
    stderr: '',
    timedOut: false,
    cancelled: false,
    outputLimitExceeded: false,
    spawnError: errorMessage(error),
  });
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
