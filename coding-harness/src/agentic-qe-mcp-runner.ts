// SPDX-License-Identifier: MIT

import { spawn, type ChildProcessByStdio } from 'node:child_process';
import { existsSync, lstatSync, realpathSync } from 'node:fs';
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path';
import type { Readable, Writable } from 'node:stream';
import { signalProcessGroup } from './native-process-contracts.js';
import type {
  ProviderFreeAgenticQeMcpRequest,
  ProviderFreeAgenticQeMcpRunner,
} from './agentic-qe-lcov.js';
import {
  assertAgenticQePackageStable,
  assertStableExecutable,
  captureAgenticQePackage,
  captureStableExecutable,
  type AgenticQePackageIdentity,
  type PackageIdentityLimits,
  type StableExecutableIdentity,
} from './agentic-qe-mcp-identity.js';
import {
  AGENTIC_QE_MCP_INITIALIZE_ID,
  AGENTIC_QE_MCP_SHUTDOWN_ID,
  AGENTIC_QE_MCP_TOOL_CALL_ID,
  initializeMessage,
  initializedAndToolMessages,
  parseRpcResponse,
  shutdownMessage,
  validateInitializeResult,
  validateShutdownResult,
} from './agentic-qe-mcp-protocol.js';
import { validateProviderFreeMcpRequest } from './agentic-qe-mcp-request.js';

type McpChild = ChildProcessByStdio<Writable, Readable, Readable>;

export interface SystemAgenticQeMcpRunnerOptions {
  readonly nodeExecutable: string;
  readonly aqeMcpExecutable: string;
  readonly aqePackageRoot: string;
  readonly bwrapExecutable: string;
  readonly hardTimeoutMs?: number;
  readonly maxOutputBytes?: number;
  readonly terminationGraceMs?: number;
  readonly maxPackageFiles?: number;
  readonly maxPackageBytes?: number;
}

export interface AgenticQeMcpRunnerIdentityEvidence {
  readonly node: Readonly<{ path: string; sha256: string }>;
  readonly bwrap: Readonly<{ path: string; sha256: string }>;
  readonly package: Readonly<{
    root: string;
    entryPath: string;
    name: 'agentic-qe';
    version: string;
    entrySha256: string;
    treeSha256: string;
    fileCount: number;
    totalBytes: number;
  }>;
}

interface RuntimeMount {
  readonly source: string;
  readonly destination: string;
}

const MAX_TIMEOUT_MS = 120_000;
const MAX_OUTPUT_BYTES = 5_000_000;
const MAX_TERMINATION_GRACE_MS = 5_000;
const DEFAULT_PACKAGE_FILES = 20_000;
const DEFAULT_PACKAGE_BYTES = 500_000_000;

export class SystemAgenticQeMcpRunner implements ProviderFreeAgenticQeMcpRunner {
  readonly #node: StableExecutableIdentity;
  readonly #bwrap: StableExecutableIdentity;
  readonly #package: AgenticQePackageIdentity;
  readonly #packageLimits: PackageIdentityLimits;
  readonly #runtimeMounts: readonly RuntimeMount[];
  readonly #hardTimeoutMs: number;
  readonly #maxOutputBytes: number;
  readonly #terminationGraceMs: number;

  constructor(options: SystemAgenticQeMcpRunnerOptions) {
    if (process.platform !== 'linux') {
      throw new Error('HARNESS_AGENTIC_QE_BWRAP_OS_UNAVAILABLE');
    }
    this.#node = captureStableExecutable(options.nodeExecutable, 'NODE');
    this.#bwrap = captureStableExecutable(options.bwrapExecutable, 'BWRAP');
    this.#packageLimits = Object.freeze({
      maxFiles: limit(options.maxPackageFiles ?? DEFAULT_PACKAGE_FILES, 100_000, 'maxPackageFiles'),
      maxBytes: limit(options.maxPackageBytes ?? DEFAULT_PACKAGE_BYTES, 2_000_000_000, 'maxPackageBytes'),
    });
    this.#package = captureAgenticQePackage(
      options.aqePackageRoot,
      options.aqeMcpExecutable,
      this.#packageLimits,
    );
    this.#runtimeMounts = runtimeLibraryMounts();
    this.#hardTimeoutMs = limit(
      options.hardTimeoutMs ?? MAX_TIMEOUT_MS,
      MAX_TIMEOUT_MS,
      'hardTimeoutMs',
    );
    this.#maxOutputBytes = limit(
      options.maxOutputBytes ?? MAX_OUTPUT_BYTES,
      MAX_OUTPUT_BYTES,
      'maxOutputBytes',
    );
    this.#terminationGraceMs = limit(
      options.terminationGraceMs ?? 250,
      MAX_TERMINATION_GRACE_MS,
      'terminationGraceMs',
    );
    this.#assertStable();
  }

  async invoke(
    rawRequest: ProviderFreeAgenticQeMcpRequest,
    signal?: AbortSignal,
  ): Promise<unknown> {
    if (signal?.aborted === true) throw abortError();
    const request = validateProviderFreeMcpRequest(rawRequest);
    this.#assertDisjointInputs(request);
    this.#assertStable();
    try {
      return await runMcpSession(
        this.#bwrap.path,
        bubblewrapArguments(request, this.#node, this.#package, this.#runtimeMounts),
        request,
        this.#package.version,
        this.#hardTimeoutMs,
        this.#maxOutputBytes,
        this.#terminationGraceMs,
        signal,
      );
    } finally {
      this.#assertStable();
    }
  }

  identityEvidence(): AgenticQeMcpRunnerIdentityEvidence {
    return Object.freeze({
      node: Object.freeze({ path: this.#node.path, sha256: this.#node.sha256 }),
      bwrap: Object.freeze({ path: this.#bwrap.path, sha256: this.#bwrap.sha256 }),
      package: Object.freeze({
        root: this.#package.root,
        entryPath: this.#package.entryPath,
        name: this.#package.name,
        version: this.#package.version,
        entrySha256: this.#package.entrySha256,
        treeSha256: this.#package.treeSha256,
        fileCount: this.#package.fileCount,
        totalBytes: this.#package.totalBytes,
      }),
    });
  }

  #assertStable(): void {
    assertStableExecutable(this.#node, 'NODE');
    assertStableExecutable(this.#bwrap, 'BWRAP');
    assertAgenticQePackageStable(this.#package, this.#packageLimits);
  }

  #assertDisjointInputs(request: ProviderFreeAgenticQeMcpRequest): void {
    const inputs = [request.arguments.target, request.arguments.coverageFile];
    if (inputs.some((input) => pathsOverlap(input, this.#package.root)
      || input === this.#node.path || input === this.#bwrap.path)) {
      throw new Error('HARNESS_AGENTIC_QE_RUNTIME_INPUT_OVERLAP');
    }
  }
}

function bubblewrapArguments(
  request: ProviderFreeAgenticQeMcpRequest,
  node: StableExecutableIdentity,
  packageIdentity: AgenticQePackageIdentity,
  runtimeMounts: readonly RuntimeMount[],
): string[] {
  const inputMounts: RuntimeMount[] = [{
    source: request.arguments.target,
    destination: request.arguments.target,
  }, {
    source: request.arguments.coverageFile,
    destination: request.arguments.coverageFile,
  }];
  const mounts = dedupeMounts([
    ...runtimeMounts,
    { source: node.path, destination: node.path },
    { source: packageIdentity.root, destination: packageIdentity.root },
    ...inputMounts,
  ]);
  const directories = parentDirectories([
    '/dev', '/proc', '/run', '/tmp', '/home/harness',
    ...mounts.map(({ destination }) => destination),
  ]);
  const environment = {
    HOME: '/home/harness',
    TMPDIR: '/tmp',
    PATH: '/nonexistent',
    LANG: 'C',
    LC_ALL: 'C',
    TZ: 'UTC',
    NO_COLOR: '1',
    AQE_MEMORY_BACKEND: 'memory',
    AQE_LLM_ROUTER_DISABLED: '1',
    AQE_SESSION_CACHE: 'off',
    AQE_LOOP_DETECTION_ENABLED: 'false',
    AQE_MEMORY_PATH: '/tmp/agentic-qe/memory',
    AQE_STORAGE_PATH: '/tmp/agentic-qe/storage',
    AQE_PROJECT_ROOT: request.arguments.target,
    WS_NO_BUFFER_UTIL: '1',
    WS_NO_UTF_8_VALIDATE: '1',
  } as const;
  return [
    '--die-with-parent', '--new-session', '--unshare-all', '--unshare-net',
    '--clearenv', '--tmpfs', '/', '--hostname', 'semantic-fabric-agentic-qe',
    '--cap-drop', 'ALL',
    ...directories.flatMap((path) => ['--dir', path]),
    '--dev', '/dev', '--proc', '/proc', '--tmpfs', '/run', '--tmpfs', '/tmp',
    '--tmpfs', '/home/harness',
    ...mounts.flatMap(({ source, destination }) => ['--ro-bind', source, destination]),
    ...Object.entries(environment).flatMap(([name, value]) => ['--setenv', name, value]),
    '--chdir', request.arguments.target, '--',
    node.path, packageIdentity.entryPath,
  ];
}

async function runMcpSession(
  executable: string,
  args: readonly string[],
  request: ProviderFreeAgenticQeMcpRequest,
  packageVersion: string,
  timeoutMs: number,
  maxOutputBytes: number,
  terminationGraceMs: number,
  signal?: AbortSignal,
): Promise<unknown> {
  return await new Promise<unknown>((resolveResult, reject) => {
    let child: McpChild;
    try {
      child = spawn(executable, [...args], {
        cwd: '/',
        env: {},
        shell: false,
        detached: true,
        stdio: ['pipe', 'pipe', 'pipe'],
      });
    } catch (error) {
      reject(error);
      return;
    }
    let expectedId: 1 | 2 | 3 | 4 = AGENTIC_QE_MCP_INITIALIZE_ID;
    let toolResult: unknown;
    let pending = Buffer.alloc(0);
    let observedBytes = 0;
    let protocolError: unknown;
    let spawnError: unknown;
    let timedOut = false;
    let cancelled = false;
    let outputExceeded = false;
    let terminating = false;
    let killTimer: NodeJS.Timeout | undefined;

    const terminate = () => {
      if (terminating) return;
      terminating = true;
      signalProcessGroup(child, 'SIGTERM');
      killTimer = setTimeout(
        () => signalProcessGroup(child, 'SIGKILL'),
        terminationGraceMs,
      );
      killTimer.unref();
    };
    const failProtocol = (error: unknown) => {
      protocolError ??= error;
      terminate();
    };
    const write = (message: string) => {
      if (!child.stdin.destroyed && !child.stdin.writableEnded) child.stdin.write(message);
      else failProtocol(new Error('HARNESS_AGENTIC_QE_MCP_STDIN_CLOSED'));
    };
    const acceptLine = (bytes: Buffer) => {
      try {
        const lineBytes = bytes.at(-1) === 13 ? bytes.subarray(0, -1) : bytes;
        if (lineBytes.length === 0) throw new Error('HARNESS_AGENTIC_QE_MCP_EMPTY_RESPONSE');
        const line = new TextDecoder('utf-8', { fatal: true }).decode(lineBytes);
        if (expectedId === 4) throw new Error('HARNESS_AGENTIC_QE_MCP_EXTRA_RESPONSE');
        const result = parseRpcResponse(line, expectedId);
        if (expectedId === AGENTIC_QE_MCP_INITIALIZE_ID) {
          validateInitializeResult(result, packageVersion);
          expectedId = AGENTIC_QE_MCP_TOOL_CALL_ID;
          write(initializedAndToolMessages(request));
        } else if (expectedId === AGENTIC_QE_MCP_TOOL_CALL_ID) {
          toolResult = result;
          expectedId = AGENTIC_QE_MCP_SHUTDOWN_ID;
          write(shutdownMessage());
        } else {
          validateShutdownResult(result);
          expectedId = 4;
          child.stdin.end();
        }
      } catch (error) {
        failProtocol(error);
      }
    };
    const capture = (chunk: Buffer, stdout: boolean) => {
      observedBytes += chunk.length;
      if (observedBytes > maxOutputBytes) {
        outputExceeded = true;
        terminate();
        return;
      }
      if (!stdout || protocolError !== undefined) return;
      pending = Buffer.concat([pending, chunk]);
      let newline = pending.indexOf(10);
      while (newline !== -1) {
        const line = pending.subarray(0, newline);
        pending = pending.subarray(newline + 1);
        acceptLine(line);
        newline = pending.indexOf(10);
      }
    };

    child.stdout.on('data', (chunk: Buffer) => capture(chunk, true));
    child.stderr.on('data', (chunk: Buffer) => capture(chunk, false));
    child.on('error', (error) => { spawnError = error; });
    child.stdin.on('error', (error) => {
      if (!terminating) failProtocol(error);
    });
    const timeout = setTimeout(() => {
      timedOut = true;
      terminate();
    }, timeoutMs);
    timeout.unref();
    const abort = () => {
      cancelled = true;
      terminate();
    };
    signal?.addEventListener('abort', abort, { once: true });
    write(initializeMessage());

    child.on('close', (exitCode) => {
      clearTimeout(timeout);
      if (killTimer !== undefined) clearTimeout(killTimer);
      signal?.removeEventListener('abort', abort);
      if (cancelled) reject(abortError());
      else if (timedOut) reject(new Error('HARNESS_AGENTIC_QE_MCP_TIMEOUT'));
      else if (outputExceeded) reject(new Error('HARNESS_AGENTIC_QE_MCP_OUTPUT_LIMIT_EXCEEDED'));
      else if (spawnError !== undefined) reject(spawnError);
      else if (protocolError !== undefined) reject(protocolError);
      else if (pending.length !== 0 || expectedId !== 4 || toolResult === undefined) {
        reject(new Error('HARNESS_AGENTIC_QE_MCP_PROTOCOL_INCOMPLETE'));
      } else if (exitCode !== 0) {
        reject(new Error(`HARNESS_AGENTIC_QE_MCP_EXIT:${String(exitCode)}`));
      } else {
        resolveResult(toolResult);
      }
    });
  });
}

function runtimeLibraryMounts(): readonly RuntimeMount[] {
  const mounts: RuntimeMount[] = [];
  for (const [sourceValue, destinations] of [
    ['/usr/lib', ['/usr/lib', '/lib']],
    ['/usr/lib64', ['/usr/lib64', '/lib64']],
  ] as const) {
    if (!existsSync(sourceValue)) continue;
    const source = canonicalDirectory(sourceValue);
    for (const destination of destinations) mounts.push(Object.freeze({ source, destination }));
  }
  if (mounts.length === 0) throw new Error('HARNESS_AGENTIC_QE_NODE_RUNTIME_UNAVAILABLE');
  return Object.freeze(mounts);
}

function canonicalDirectory(value: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) {
    throw new Error('HARNESS_AGENTIC_QE_NODE_RUNTIME_INVALID');
  }
  const path = realpathSync(value);
  const stat = lstatSync(path);
  if (!stat.isDirectory() || stat.isSymbolicLink() || (stat.mode & 0o022) !== 0) {
    throw new Error('HARNESS_AGENTIC_QE_NODE_RUNTIME_INVALID');
  }
  return path;
}

function dedupeMounts(mounts: readonly RuntimeMount[]): RuntimeMount[] {
  const destinations = new Set<string>();
  return mounts.filter(({ destination }) => {
    if (destinations.has(destination)) return false;
    destinations.add(destination);
    return true;
  });
}

function parentDirectories(paths: readonly string[]): string[] {
  const output = new Set<string>();
  for (const path of paths) {
    let current = (() => {
      try { return lstatSync(path).isFile() ? dirname(path) : path; } catch { return path; }
    })();
    while (current !== '/') {
      output.add(current);
      current = dirname(current);
    }
  }
  return [...output].sort((left, right) => left.split(sep).length - right.split(sep).length);
}

function pathsOverlap(left: string, right: string): boolean {
  return contains(left, right) || contains(right, left);
}

function contains(root: string, child: string): boolean {
  const delta = relative(root, child);
  return delta === '' || (delta !== '..' && !delta.startsWith(`..${sep}`) && !isAbsolute(delta));
}

function limit(value: number, maximum: number, label: string): number {
  if (!Number.isSafeInteger(value) || value < 1 || value > maximum) {
    throw new TypeError(`HARNESS_AGENTIC_QE_${label.toUpperCase()}_INVALID`);
  }
  return value;
}

function abortError(): Error {
  const error = new Error('HARNESS_AGENTIC_QE_MCP_CANCELLED');
  error.name = 'AbortError';
  return error;
}
