// SPDX-License-Identifier: MIT

import { spawn, type ChildProcessByStdio } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  closeSync, constants, fstatSync, lstatSync, openSync, readSync, realpathSync, type BigIntStats,
} from 'node:fs';
import { isAbsolute, dirname, resolve } from 'node:path';
import type { Readable, Writable } from 'node:stream';
import {
  SHA256_PATTERN, asNonEmptyString, asRecord, asUniqueStrings, assertExactKeys,
} from './contracts.js';
import {
  PROGRAMME_V5_RUFLO_CLI_IDENTITY,
  PROGRAMME_V5_RUFLO_MCP_IDENTITY,
  PROGRAMME_V5_RUFLO_NODE_IDENTITY,
  parseProgrammeV5RufloEvidence,
  parseProgrammeV5RufloSwarmStatus,
  parseProgrammeV5RufloTaskStatus,
  programmeV5RufloCaptureBindingDigest,
  programmeV5RufloRequests,
  programmeV5RufloSnapshotDigest,
  type ProgrammeV5RufloEvidence, type ProgrammeV5RufloSwarmStatus,
  type ProgrammeV5RufloTaskStatus,
} from './programme-v5-ruflo-contract.js';
import { createProgrammeV5RufloPrivateRuntime } from './programme-v5-ruflo-runtime.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export { PROGRAMME_V5_RUFLO_CLI_IDENTITY, PROGRAMME_V5_RUFLO_MCP_IDENTITY,
  PROGRAMME_V5_RUFLO_NODE_IDENTITY, parseProgrammeV5RufloEvidence,
  validProgrammeV5RufloBinding, type ProgrammeV5RufloEvidence,
} from './programme-v5-ruflo-contract.js';

export interface ProgrammeV5RufloCollectorInput {
  readonly repositoryRoot: string;
  readonly taskId: string;
  readonly runId: string;
  readonly routeSnapshotDigest: string;
  readonly captureNonce: string;
  readonly transactionStartedAt: string;
  readonly swarmId: string;
  readonly coordinationTaskId: string;
  readonly hookIds: readonly string[];
  readonly traceIds: readonly string[];
  readonly timeoutMs?: number;
  readonly maxOutputBytes?: number;
  readonly terminationGraceMs?: number;
  readonly signal?: AbortSignal;
}

type McpChild = ChildProcessByStdio<Writable, Readable, Readable>;
const PACKAGE_MANIFEST_PATH = dirname(dirname(PROGRAMME_V5_RUFLO_CLI_IDENTITY.entryPath))
  + '/package.json';
const OPAQUE_ID = /^[A-Za-z0-9_-]{8,160}$/;
const DEFAULT_TIMEOUT_MS = 15_000;
const MAX_TIMEOUT_MS = 30_000;
const DEFAULT_OUTPUT_BYTES = 1_048_576;
const MAX_OUTPUT_BYTES = 4_194_304;
const DEFAULT_TERMINATION_GRACE_MS = 500;

interface SessionResult {
  taskStatus: ProgrammeV5RufloTaskStatus; swarmStatus: ProgrammeV5RufloSwarmStatus;
}

export async function collectProgrammeV5RufloEvidence(
  input: ProgrammeV5RufloCollectorInput,
): Promise<ProgrammeV5RufloEvidence> {
  const bindings = parseCollectorInput(input);
  if (input.signal?.aborted) throw abortError();
  const captureBindingDigest = programmeV5RufloCaptureBindingDigest(bindings);
  const identityBefore = inspectPinnedCliIdentity();
  const nodeBefore = inspectPinnedNodeIdentity();
  const requests = programmeV5RufloRequests(bindings.coordinationTaskId, bindings.swarmId);
  const runtime = createProgrammeV5RufloPrivateRuntime(bindings.repositoryRoot);
  let session: SessionResult;
  try {
    session = await runLocalMcpSession({
      runtime, taskStatusRequest: requests.taskStatusRequest,
      swarmStatusRequest: requests.swarmStatusRequest,
      timeoutMs: boundedOption(input.timeoutMs, DEFAULT_TIMEOUT_MS, 100, MAX_TIMEOUT_MS, 'TIMEOUT'),
      maxOutputBytes: boundedOption(input.maxOutputBytes, DEFAULT_OUTPUT_BYTES, 4_096,
        MAX_OUTPUT_BYTES, 'OUTPUT_LIMIT'),
      terminationGraceMs: boundedOption(input.terminationGraceMs,
        DEFAULT_TERMINATION_GRACE_MS, 100, 2_000, 'TERMINATION_GRACE'),
      signal: input.signal,
    });
  } finally { runtime.cleanup(); }
  const identityAfter = inspectPinnedCliIdentity();
  const nodeAfter = inspectPinnedNodeIdentity();
  if (JSON.stringify(identityAfter) !== JSON.stringify(identityBefore)
    || JSON.stringify(nodeAfter) !== JSON.stringify(nodeBefore)) {
    throw new Error('HARNESS_PROGRAMME_V5_RUFLO_ENTRY_CHANGED');
  }
  return parseProgrammeV5RufloEvidence({
    schemaVersion: 2,
    source: 'ruflo-coordination-ledger',
    taskId: bindings.taskId,
    runId: bindings.runId,
    swarmId: bindings.swarmId,
    coordinationTaskId: bindings.coordinationTaskId,
    hookIds: bindings.hookIds,
    traceIds: bindings.traceIds,
    routeSnapshotDigest: bindings.routeSnapshotDigest,
    captureNonce: bindings.captureNonce,
    transactionStartedAt: bindings.transactionStartedAt,
    captureBindingDigest,
    mcp: PROGRAMME_V5_RUFLO_MCP_IDENTITY,
    cli: identityAfter,
    ...requests,
    taskStatus: session.taskStatus,
    taskStatusDigest: programmeV5RufloSnapshotDigest(session.taskStatus),
    swarmStatus: session.swarmStatus,
    swarmStatusDigest: programmeV5RufloSnapshotDigest(session.swarmStatus),
    providerVariablesStripped: true,
    authoritative: false,
    capturedAt: new Date().toISOString(),
  });
}

function parseCollectorInput(input: ProgrammeV5RufloCollectorInput): Readonly<{
  repositoryRoot: string;
  taskId: string;
  runId: string;
  routeSnapshotDigest: string;
  captureNonce: string;
  transactionStartedAt: string;
  swarmId: string;
  coordinationTaskId: string;
  hookIds: readonly string[];
  traceIds: readonly string[];
}> {
  const repositoryRoot = canonicalDirectory(input.repositoryRoot);
  const routeSnapshotDigest = asNonEmptyString(
    input.routeSnapshotDigest, 'programme v5 Ruflo collector.routeSnapshotDigest',
  );
  if (!SHA256_PATTERN.test(routeSnapshotDigest) || routeSnapshotDigest === '0'.repeat(64)) {
    throw new TypeError('HARNESS_PROGRAMME_V5_RUFLO_ROUTE_DIGEST_INVALID');
  }
  return Object.freeze({
    repositoryRoot,
    taskId: opaqueId(input.taskId, 'taskId'),
    runId: opaqueId(input.runId, 'runId'),
    routeSnapshotDigest,
    captureNonce: input.captureNonce,
    transactionStartedAt: input.transactionStartedAt,
    swarmId: opaqueId(input.swarmId, 'swarmId'),
    coordinationTaskId: opaqueId(input.coordinationTaskId, 'coordinationTaskId'),
    hookIds: Object.freeze(asUniqueStrings(input.hookIds, 'programme v5 Ruflo collector.hookIds', true)),
    traceIds: Object.freeze(asUniqueStrings(input.traceIds, 'programme v5 Ruflo collector.traceIds', true)),
  });
}

function inspectPinnedCliIdentity(): typeof PROGRAMME_V5_RUFLO_CLI_IDENTITY {
  const entryDigest = stableProgrammeV5RufloFileDigest(
    PROGRAMME_V5_RUFLO_CLI_IDENTITY.entryPath, true,
  );
  if (entryDigest !== PROGRAMME_V5_RUFLO_CLI_IDENTITY.entryDigest) {
    throw new Error('HARNESS_PROGRAMME_V5_RUFLO_ENTRY_DIGEST_MISMATCH');
  }
  const manifestFile = readStableFile(PACKAGE_MANIFEST_PATH, false, 1_048_576, true);
  const manifest = asRecord(parseJsonWithoutDuplicateKeys(
    new TextDecoder('utf-8', { fatal: true }).decode(manifestFile.bytes),
    'installed Ruflo package manifest',
  ), 'installed Ruflo package manifest');
  const bins = asRecord(manifest.bin, 'installed Ruflo package manifest.bin');
  if (manifest.name !== PROGRAMME_V5_RUFLO_CLI_IDENTITY.packageName
    || manifest.version !== PROGRAMME_V5_RUFLO_CLI_IDENTITY.packageVersion
    || bins['claude-flow-mcp'] !== 'bin/mcp-server.js') {
    throw new Error('HARNESS_PROGRAMME_V5_RUFLO_PACKAGE_IDENTITY_MISMATCH');
  }
  return PROGRAMME_V5_RUFLO_CLI_IDENTITY;
}

function inspectPinnedNodeIdentity(): typeof PROGRAMME_V5_RUFLO_NODE_IDENTITY {
  const digest = stableProgrammeV5RufloFileDigest(
    PROGRAMME_V5_RUFLO_NODE_IDENTITY.path, true, 500_000_000,
  );
  if (digest !== PROGRAMME_V5_RUFLO_NODE_IDENTITY.digest) {
    throw new Error('HARNESS_PROGRAMME_V5_RUFLO_NODE_IDENTITY_MISMATCH');
  }
  return PROGRAMME_V5_RUFLO_NODE_IDENTITY;
}

export function stableProgrammeV5RufloFileDigest(
  path: string,
  executable = false,
  maxBytes = 1_048_576,
): string {
  return readStableFile(path, executable, maxBytes, false).digest;
}

async function runLocalMcpSession(input: Readonly<{
  runtime: ReturnType<typeof createProgrammeV5RufloPrivateRuntime>;
  taskStatusRequest: ReturnType<typeof programmeV5RufloRequests>['taskStatusRequest'];
  swarmStatusRequest: ReturnType<typeof programmeV5RufloRequests>['swarmStatusRequest'];
  timeoutMs: number;
  maxOutputBytes: number;
  terminationGraceMs: number;
  signal?: AbortSignal;
}>): Promise<SessionResult> {
  return await new Promise<SessionResult>((resolveResult, reject) => {
    let child: McpChild;
    try {
      child = spawn(input.runtime.executable, [...input.runtime.args], {
        cwd: input.runtime.cwd,
        env: { ...input.runtime.environment },
        shell: false,
        detached: process.platform !== 'win32',
        stdio: ['pipe', 'pipe', 'pipe'],
      });
    } catch (error) {
      reject(error);
      return;
    }
    let expectedId = 1;
    let taskStatus: ProgrammeV5RufloTaskStatus | undefined;
    let swarmStatus: ProgrammeV5RufloSwarmStatus | undefined;
    let pending = Buffer.alloc(0);
    let observedBytes = 0;
    let protocolError: unknown;
    let spawnError: unknown;
    let timedOut = false;
    let cancelled = false;
    let outputExceeded = false;
    let terminating = false;
    let inputClosed = false;
    let killTimer: NodeJS.Timeout | undefined;

    const terminate = () => {
      if (terminating) return;
      terminating = true;
      signalProcess(child, 'SIGTERM');
      killTimer = setTimeout(
        () => signalProcess(child, 'SIGKILL'),
        input.terminationGraceMs,
      );
      killTimer.unref();
    };
    const failProtocol = (error: unknown) => {
      protocolError ??= error;
      terminate();
    };
    const write = (message: unknown) => {
      if (child.stdin.destroyed || child.stdin.writableEnded) {
        failProtocol(new Error('HARNESS_PROGRAMME_V5_RUFLO_MCP_STDIN_CLOSED'));
        return;
      }
      child.stdin.write(`${JSON.stringify(message)}\n`);
    };
    const acceptLine = (bytes: Buffer) => {
      try {
        const lineBytes = bytes.at(-1) === 13 ? bytes.subarray(0, -1) : bytes;
        if (lineBytes.length === 0) throw new Error('HARNESS_PROGRAMME_V5_RUFLO_MCP_EMPTY_RESPONSE');
        const line = new TextDecoder('utf-8', { fatal: true }).decode(lineBytes);
        if (expectedId > 3) throw new Error('HARNESS_PROGRAMME_V5_RUFLO_MCP_EXTRA_RESPONSE');
        const result = parseRpcResult(line, expectedId);
        if (expectedId === 1) {
          validateInitializeResult(result);
          expectedId = 2;
          write({ jsonrpc: '2.0', method: 'notifications/initialized', params: {} });
          write(input.taskStatusRequest);
        } else if (expectedId === 2) {
          taskStatus = parseProgrammeV5RufloTaskStatus(
            extractToolResult(result, 'task_status'),
            input.taskStatusRequest.params.arguments.taskId,
          );
          expectedId = 3;
          write(input.swarmStatusRequest);
        } else {
          swarmStatus = parseProgrammeV5RufloSwarmStatus(
            extractToolResult(result, 'swarm_status'),
            input.swarmStatusRequest.params.arguments.swarmId,
          );
          expectedId = 4;
          inputClosed = true;
          child.stdin.end();
        }
      } catch (error) {
        failProtocol(error);
      }
    };
    const capture = (chunk: Buffer, stdout: boolean) => {
      observedBytes += chunk.length;
      if (observedBytes > input.maxOutputBytes) {
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
    child.once('error', (error) => { spawnError = error; });
    child.stdin.on('error', (error) => {
      if (!terminating && !inputClosed) failProtocol(error);
    });
    const timeout = setTimeout(() => {
      timedOut = true;
      terminate();
    }, input.timeoutMs);
    timeout.unref();
    const abort = () => {
      cancelled = true;
      terminate();
    };
    input.signal?.addEventListener('abort', abort, { once: true });
    write({
      jsonrpc: '2.0', id: 1, method: 'initialize',
      params: {
        protocolVersion: PROGRAMME_V5_RUFLO_MCP_IDENTITY.protocolVersion,
        capabilities: {},
        clientInfo: { name: 'semantic-fabric-coding-harness', version: '1.0.0' },
      },
    });

    child.once('close', (exitCode) => {
      clearTimeout(timeout);
      if (killTimer !== undefined) clearTimeout(killTimer);
      input.signal?.removeEventListener('abort', abort);
      if (cancelled) reject(abortError());
      else if (timedOut) reject(new Error('HARNESS_PROGRAMME_V5_RUFLO_MCP_TIMEOUT'));
      else if (outputExceeded) reject(new Error('HARNESS_PROGRAMME_V5_RUFLO_MCP_OUTPUT_LIMIT_EXCEEDED'));
      else if (spawnError !== undefined) reject(spawnError);
      else if (protocolError !== undefined) reject(protocolError);
      else if (pending.length !== 0 || expectedId !== 4
        || taskStatus === undefined || swarmStatus === undefined) {
        reject(new Error('HARNESS_PROGRAMME_V5_RUFLO_MCP_PROTOCOL_INCOMPLETE'));
      } else if (exitCode !== 0) {
        reject(new Error(`HARNESS_PROGRAMME_V5_RUFLO_MCP_EXIT:${String(exitCode)}`));
      } else {
        resolveResult({ taskStatus, swarmStatus });
      }
    });
  });
}

function parseRpcResult(line: string, expectedId: number): unknown {
  const response = exactRecord(
    parseJsonWithoutDuplicateKeys(line, 'programme v5 Ruflo MCP response'),
    ['jsonrpc', 'id', 'result'],
    'programme v5 Ruflo MCP response',
  );
  if (response.jsonrpc !== '2.0' || response.id !== expectedId) {
    throw new Error('HARNESS_PROGRAMME_V5_RUFLO_MCP_RESPONSE_BINDING_MISMATCH');
  }
  return response.result;
}

function validateInitializeResult(value: unknown): void {
  const result = exactRecord(value, [
    'protocolVersion', 'serverInfo', 'capabilities',
  ], 'programme v5 Ruflo MCP initialize result');
  const server = exactRecord(result.serverInfo, ['name', 'version'], 'programme v5 Ruflo MCP server');
  const capabilities = exactRecord(result.capabilities, ['tools', 'resources'],
    'programme v5 Ruflo MCP capabilities');
  const tools = exactRecord(capabilities.tools, ['listChanged'], 'programme v5 Ruflo MCP tools capability');
  const resources = exactRecord(capabilities.resources, ['subscribe', 'listChanged'],
    'programme v5 Ruflo MCP resources capability');
  if (result.protocolVersion !== PROGRAMME_V5_RUFLO_MCP_IDENTITY.protocolVersion
    || server.name !== PROGRAMME_V5_RUFLO_MCP_IDENTITY.serverName
    || server.version !== PROGRAMME_V5_RUFLO_MCP_IDENTITY.serverVersion
    || tools.listChanged !== true || resources.subscribe !== true || resources.listChanged !== true) {
    throw new Error('HARNESS_PROGRAMME_V5_RUFLO_MCP_IDENTITY_MISMATCH');
  }
}

function extractToolResult(value: unknown, tool: string): unknown {
  const result = exactRecord(value, ['content'], `programme v5 Ruflo ${tool} result`);
  if (!Array.isArray(result.content) || result.content.length !== 1) {
    throw new Error(`HARNESS_PROGRAMME_V5_RUFLO_${tool.toUpperCase()}_RESULT_INVALID`);
  }
  const content = exactRecord(result.content[0], ['type', 'text'], `programme v5 Ruflo ${tool} content`);
  if (content.type !== 'text' || typeof content.text !== 'string') {
    throw new Error(`HARNESS_PROGRAMME_V5_RUFLO_${tool.toUpperCase()}_RESULT_INVALID`);
  }
  return parseJsonWithoutDuplicateKeys(content.text, `programme v5 Ruflo ${tool} snapshot`);
}

function exactRecord(value: unknown, keys: readonly string[], label: string): Record<string, unknown> {
  const input = asRecord(value, label);
  const ownKeys = Reflect.ownKeys(input);
  if (Object.getPrototypeOf(input) !== Object.prototype
    || ownKeys.some((key) => typeof key !== 'string'
      || !Object.prototype.propertyIsEnumerable.call(input, key))) {
    throw new TypeError(`${label} has invalid keys`);
  }
  assertExactKeys(input, keys, label);
  if (ownKeys.length !== keys.length) throw new TypeError(`${label} has invalid keys`);
  return input;
}

function readStableFile(
  path: string, executable: boolean, maxBytes: number, capture: boolean,
): Readonly<{ digest: string; bytes: Buffer }> {
  if (!isAbsolute(path) || resolve(path) !== path || path.includes('\0')
    || !Number.isSafeInteger(maxBytes) || maxBytes < 1) {
    throw new Error('HARNESS_PROGRAMME_V5_RUFLO_LOCAL_FILE_UNTRUSTED');
  }
  const before = lstatSync(path, { bigint: true });
  const uid = BigInt(process.getuid?.() ?? Number(before.uid));
  if (!before.isFile() || before.isSymbolicLink() || before.nlink !== 1n
    || realpathSync(path) !== path || before.size < 1n || before.size > BigInt(maxBytes)
    || (executable && (before.mode & 0o111n) === 0n)
    || (before.uid !== 0n && before.uid !== uid) || (before.mode & 0o022n) !== 0n) {
    throw new Error('HARNESS_PROGRAMME_V5_RUFLO_LOCAL_FILE_UNTRUSTED');
  }
  const descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const opened = fstatSync(descriptor, { bigint: true });
    if (!sameStableFile(before, opened)) throw new Error('HARNESS_PROGRAMME_V5_RUFLO_LOCAL_FILE_CHANGED');
    const hash = createHash('sha256');
    const chunks: Buffer[] = [];
    const buffer = Buffer.allocUnsafe(Math.min(maxBytes, 1024 * 1024));
    let offset = 0n;
    while (offset < opened.size) {
      const count = readSync(descriptor, buffer, 0,
        Math.min(buffer.length, Number(opened.size - offset)), Number(offset));
      if (count === 0) break;
      const bytes = buffer.subarray(0, count);
      hash.update(bytes);
      if (capture) chunks.push(Buffer.from(bytes));
      offset += BigInt(count);
    }
    const after = fstatSync(descriptor, { bigint: true });
    if (offset !== opened.size || !sameStableFile(opened, after)) {
      throw new Error('HARNESS_PROGRAMME_V5_RUFLO_LOCAL_FILE_CHANGED');
    }
    return Object.freeze({
      digest: hash.digest('hex'), bytes: capture ? Buffer.concat(chunks) : Buffer.alloc(0),
    });
  } finally {
    closeSync(descriptor);
  }
}

function sameStableFile(left: BigIntStats, right: BigIntStats): boolean {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.nlink === right.nlink && left.uid === right.uid && left.gid === right.gid
    && left.size === right.size && left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}

function canonicalDirectory(value: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) {
    throw new TypeError('HARNESS_PROGRAMME_V5_RUFLO_REPOSITORY_INVALID');
  }
  const stat = lstatSync(value);
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(value) !== value) {
    throw new TypeError('HARNESS_PROGRAMME_V5_RUFLO_REPOSITORY_INVALID');
  }
  return value;
}

function opaqueId(value: unknown, label: string): string {
  const id = asNonEmptyString(value, `programme v5 Ruflo collector.${label}`);
  if (!OPAQUE_ID.test(id)) throw new TypeError(`HARNESS_PROGRAMME_V5_RUFLO_${label.toUpperCase()}_INVALID`);
  return id;
}

function boundedOption(
  value: number | undefined,
  fallback: number,
  minimum: number,
  maximum: number,
  label: string,
): number {
  const parsed = value ?? fallback;
  if (!Number.isSafeInteger(parsed) || parsed < minimum || parsed > maximum) {
    throw new TypeError(`HARNESS_PROGRAMME_V5_RUFLO_${label}_INVALID`);
  }
  return parsed;
}

function signalProcess(child: McpChild, signal: NodeJS.Signals): void {
  try {
    if (process.platform !== 'win32' && child.pid !== undefined) process.kill(-child.pid, signal);
    else child.kill(signal);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ESRCH') child.kill(signal);
  }
}

function abortError(): Error {
  const error = new Error('HARNESS_PROGRAMME_V5_RUFLO_MCP_CANCELLED');
  error.name = 'AbortError';
  return error;
}
