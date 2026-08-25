// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  closeSync,
  constants,
  fstatSync,
  lstatSync,
  openSync,
  readSync,
  realpathSync,
} from 'node:fs';
import { isAbsolute, resolve } from 'node:path';
import {
  SHA256_PATTERN,
  asNonEmptyString,
  deepFreeze,
} from './contracts.js';
import {
  AGENTIC_QE_LCOV_GAPS_TOOL,
  AGENTIC_QE_MAX_MCP_OUTPUT_BYTES,
  canonicalAgenticQeTimestamp,
  parseNestedCoverageGapResult,
} from './agentic-qe-lcov-response.js';
import {
  parseAgenticQeEvidence,
  type AgenticQeEvidence,
} from './evidence.js';
import { runGitCommand, type GitCommandResult } from './git-process.js';
import { digestValue } from './receipts.js';

export { AGENTIC_QE_LCOV_GAPS_TOOL } from './agentic-qe-lcov-response.js';

const GIT_OBJECT = /^[a-f0-9]{40}(?:[a-f0-9]{24})?$/;
const OPAQUE_ID = /^[A-Za-z0-9_-]{8,128}$/;
const DEFAULT_MAX_LCOV_BYTES = 20_000_000;
const MAX_LCOV_BYTES = 50_000_000;
const MAX_GAPS = 20;

export interface AgenticQeLcovArtifact {
  readonly path: string;
  readonly sha256: string;
  readonly provenance: 'independent-direct-coverage';
}

export interface AgenticQeLcovGapInput {
  readonly taskId: string;
  readonly runId: string;
  readonly candidateTree: string;
  readonly candidateRoot: string;
  readonly lcov: AgenticQeLcovArtifact;
}

export interface ProviderFreeAgenticQeMcpRequest {
  readonly executable: 'aqe-mcp';
  readonly transport: 'stdio-mcp';
  readonly method: 'tools/call';
  readonly toolName: typeof AGENTIC_QE_LCOV_GAPS_TOOL;
  readonly arguments: Readonly<{
    target: string;
    coverageFile: string;
    coverageFormat: 'lcov';
    language: 'rust';
    minRisk: 0;
    limit: 20;
    prioritization: 'complexity';
    includeGhost: false;
  }>;
  readonly bindings: Readonly<{
    taskId: string;
    runId: string;
    candidateTree: string;
    lcovSha256: string;
  }>;
  readonly runtime: Readonly<{
    network: 'offline';
    environment: Readonly<{
      inheritance: 'none';
      variables: Readonly<{
        AQE_MEMORY_BACKEND: 'memory';
        AQE_LLM_ROUTER_DISABLED: '1';
        AQE_SESSION_CACHE: 'off';
        AQE_LOOP_DETECTION_ENABLED: 'false';
      }>;
    }>;
    filesystem: Readonly<{
      inputAccess: 'read-only';
      readOnlyPaths: readonly string[];
      privateHome: true;
      privateWritableTmp: true;
    }>;
    timeoutMs: 120_000;
    maxOutputBytes: typeof AGENTIC_QE_MAX_MCP_OUTPUT_BYTES;
  }>;
}

export interface ProviderFreeAgenticQeMcpRunner {
  invoke(
    request: ProviderFreeAgenticQeMcpRequest,
    signal?: AbortSignal,
  ): Promise<unknown>;
}

export interface AgenticQeLcovGapAdapterOptions {
  readonly runner: ProviderFreeAgenticQeMcpRunner;
  readonly clock?: () => Date;
  readonly maxLcovBytes?: number;
}

type DirectoryIdentity = Readonly<{ dev: bigint; ino: bigint }>;
type FileIdentity = Readonly<{
  dev: bigint;
  ino: bigint;
  mode: bigint;
  size: bigint;
  mtimeNs: bigint;
  ctimeNs: bigint;
}>;
type LcovCapture = Readonly<{
  path: string;
  sha256: string;
  identity: FileIdentity;
}>;

export class AgenticQeLcovGapEvidenceAdapter {
  readonly #runner: ProviderFreeAgenticQeMcpRunner;
  readonly #clock: () => Date;
  readonly #maxLcovBytes: number;

  constructor(options: AgenticQeLcovGapAdapterOptions) {
    if (typeof options.runner?.invoke !== 'function') {
      throw new TypeError('HARNESS_AGENTIC_QE_RUNNER_REQUIRED');
    }
    const maxLcovBytes = options.maxLcovBytes ?? DEFAULT_MAX_LCOV_BYTES;
    if (!Number.isSafeInteger(maxLcovBytes)
      || maxLcovBytes < 1
      || maxLcovBytes > MAX_LCOV_BYTES) {
      throw new TypeError('HARNESS_AGENTIC_QE_LCOV_LIMIT_INVALID');
    }
    this.#runner = options.runner;
    this.#clock = options.clock ?? (() => new Date());
    this.#maxLcovBytes = maxLcovBytes;
  }

  async capture(
    rawInput: AgenticQeLcovGapInput,
    signal?: AbortSignal,
  ): Promise<AgenticQeEvidence> {
    const input = parseInput(rawInput);
    const rootIdentity = await assertCandidateRoot(
      input.candidateRoot,
      input.candidateTree,
      signal,
    );
    const lcov = captureLcov(input.lcov.path, this.#maxLcovBytes);
    if (lcov.sha256 !== input.lcov.sha256) {
      throw new Error('HARNESS_AGENTIC_QE_LCOV_DIGEST_MISMATCH');
    }
    const readOnlyPaths = [...new Set([input.candidateRoot, input.lcov.path])].sort();
    const request: ProviderFreeAgenticQeMcpRequest = deepFreeze({
      executable: 'aqe-mcp',
      transport: 'stdio-mcp',
      method: 'tools/call',
      toolName: AGENTIC_QE_LCOV_GAPS_TOOL,
      arguments: {
        target: input.candidateRoot,
        coverageFile: input.lcov.path,
        coverageFormat: 'lcov',
        language: 'rust',
        minRisk: 0,
        limit: MAX_GAPS,
        prioritization: 'complexity',
        includeGhost: false,
      },
      bindings: {
        taskId: input.taskId,
        runId: input.runId,
        candidateTree: input.candidateTree,
        lcovSha256: lcov.sha256,
      },
      runtime: {
        network: 'offline',
        environment: {
          inheritance: 'none',
          variables: {
            AQE_MEMORY_BACKEND: 'memory',
            AQE_LLM_ROUTER_DISABLED: '1',
            AQE_SESSION_CACHE: 'off',
            AQE_LOOP_DETECTION_ENABLED: 'false',
          },
        },
        filesystem: {
          inputAccess: 'read-only',
          readOnlyPaths,
          privateHome: true,
          privateWritableTmp: true,
        },
        timeoutMs: 120_000,
        maxOutputBytes: AGENTIC_QE_MAX_MCP_OUTPUT_BYTES,
      },
    });

    let response: unknown;
    try {
      response = await this.#runner.invoke(request, signal);
    } finally {
      await assertCandidateRoot(
        input.candidateRoot,
        input.candidateTree,
        signal,
        rootIdentity,
      );
      assertLcovUnchanged(lcov, this.#maxLcovBytes);
    }
    const normalized = parseNestedCoverageGapResult(response);
    const capturedAt = canonicalAgenticQeTimestamp(this.#clock());
    return parseAgenticQeEvidence({
      schemaVersion: 1,
      source: 'agentic-qe-local-profile',
      profile: 'lcov-gap',
      taskId: input.taskId,
      runId: input.runId,
      candidateTree: input.candidateTree,
      commandDigest: digestValue({ request, lcovSha256: lcov.sha256 }),
      outputDigest: digestValue({
        taskId: input.taskId,
        runId: input.runId,
        candidateTree: input.candidateTree,
        lcovSha256: lcov.sha256,
        toolName: AGENTIC_QE_LCOV_GAPS_TOOL,
        result: normalized,
      }),
      providerVariablesStripped: true,
      authoritative: false,
      capturedAt,
    });
  }
}

function parseInput(input: AgenticQeLcovGapInput): AgenticQeLcovGapInput {
  const taskId = opaqueId(input.taskId, 'taskId');
  const runId = opaqueId(input.runId, 'runId');
  if (!GIT_OBJECT.test(input.candidateTree)) {
    throw new TypeError('HARNESS_AGENTIC_QE_CANDIDATE_TREE_INVALID');
  }
  const candidateRoot = normalizedAbsolute(input.candidateRoot, 'candidateRoot');
  if (input.lcov?.provenance !== 'independent-direct-coverage') {
    throw new TypeError('HARNESS_AGENTIC_QE_LCOV_PROVENANCE_INVALID');
  }
  const path = normalizedAbsolute(input.lcov.path, 'lcov.path');
  if (!SHA256_PATTERN.test(input.lcov.sha256)) {
    throw new TypeError('HARNESS_AGENTIC_QE_LCOV_DIGEST_INVALID');
  }
  return deepFreeze({
    taskId,
    runId,
    candidateTree: input.candidateTree,
    candidateRoot,
    lcov: { path, sha256: input.lcov.sha256, provenance: 'independent-direct-coverage' },
  });
}

async function assertCandidateRoot(
  root: string,
  candidateTree: string,
  signal?: AbortSignal,
  expected?: DirectoryIdentity,
): Promise<DirectoryIdentity> {
  const stat = lstatSync(root, { bigint: true });
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(root) !== root) {
    throw new Error('HARNESS_AGENTIC_QE_CANDIDATE_ROOT_UNTRUSTED');
  }
  const identity = Object.freeze({ dev: stat.dev, ino: stat.ino });
  if (expected !== undefined && (identity.dev !== expected.dev || identity.ino !== expected.ino)) {
    throw new Error('HARNESS_AGENTIC_QE_CANDIDATE_ROOT_CHANGED');
  }
  const topLevel = (await gitChecked(root, ['rev-parse', '--show-toplevel'], signal)).trim();
  if (topLevel !== root) throw new Error('HARNESS_AGENTIC_QE_CANDIDATE_ROOT_NOT_WORKTREE');
  const tree = (await gitChecked(root, ['write-tree'], signal)).trim();
  if (tree !== candidateTree) throw new Error('HARNESS_AGENTIC_QE_CANDIDATE_TREE_MISMATCH');
  const worktree = await runGitCommand(root, ['diff', '--quiet', '--'], {
    signal,
    maxOutputBytes: 4096,
  });
  if (worktree.exitCode === 1) throw new Error('HARNESS_AGENTIC_QE_CANDIDATE_SOURCE_CHANGED');
  assertGitSuccess(worktree, 'diff');
  return identity;
}

function captureLcov(path: string, maxBytes: number): LcovCapture {
  const stat = lstatSync(path, { bigint: true });
  if (!stat.isFile()
    || stat.isSymbolicLink()
    || stat.nlink !== 1n
    || realpathSync(path) !== path
    || stat.size < 1n
    || stat.size > BigInt(maxBytes)) {
    throw new Error('HARNESS_AGENTIC_QE_LCOV_UNTRUSTED');
  }
  const descriptor = openSync(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const before = fileIdentity(descriptor);
    const bytes = readBounded(descriptor, before.size, maxBytes);
    const after = fileIdentity(descriptor);
    if (!sameFileIdentity(before, after)) throw new Error('HARNESS_AGENTIC_QE_LCOV_CHANGED');
    validateLcov(bytes);
    return Object.freeze({
      path,
      sha256: createHash('sha256').update(bytes).digest('hex'),
      identity: before,
    });
  } finally {
    closeSync(descriptor);
  }
}

function assertLcovUnchanged(captured: LcovCapture, maxBytes: number): void {
  const current = captureLcov(captured.path, maxBytes);
  if (!sameFileIdentity(captured.identity, current.identity)
    || current.sha256 !== captured.sha256) {
    throw new Error('HARNESS_AGENTIC_QE_LCOV_CHANGED');
  }
}

function validateLcov(bytes: Buffer): void {
  let text: string;
  try {
    text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch {
    throw new Error('HARNESS_AGENTIC_QE_LCOV_NOT_UTF8');
  }
  if (text.includes('\0')) throw new Error('HARNESS_AGENTIC_QE_LCOV_BINARY');
  const lines = text.split(/\r?\n/);
  if (!lines.some((line) => line.startsWith('SF:'))
    || !lines.some((line) => /^DA:\d+,\d+/.test(line))
    || !lines.includes('end_of_record')) {
    throw new Error('HARNESS_AGENTIC_QE_LCOV_STRUCTURE_INVALID');
  }
}

function readBounded(descriptor: number, expectedSize: bigint, maxBytes: number): Buffer {
  if (expectedSize > BigInt(maxBytes)) throw new Error('HARNESS_AGENTIC_QE_LCOV_TOO_LARGE');
  const size = Number(expectedSize);
  const target = Buffer.allocUnsafe(size + 1);
  let offset = 0;
  while (offset < target.length) {
    const read = readSync(descriptor, target, offset, target.length - offset, offset);
    if (read === 0) break;
    offset += read;
  }
  if (offset !== size) throw new Error('HARNESS_AGENTIC_QE_LCOV_CHANGED');
  return target.subarray(0, size);
}

function fileIdentity(descriptor: number): FileIdentity {
  const stat = fstatSync(descriptor, { bigint: true });
  if (!stat.isFile() || stat.nlink !== 1n) throw new Error('HARNESS_AGENTIC_QE_LCOV_UNTRUSTED');
  return Object.freeze({
    dev: stat.dev,
    ino: stat.ino,
    mode: stat.mode,
    size: stat.size,
    mtimeNs: stat.mtimeNs,
    ctimeNs: stat.ctimeNs,
  });
}

function sameFileIdentity(left: FileIdentity, right: FileIdentity): boolean {
  return left.dev === right.dev
    && left.ino === right.ino
    && left.mode === right.mode
    && left.size === right.size
    && left.mtimeNs === right.mtimeNs
    && left.ctimeNs === right.ctimeNs;
}

async function gitChecked(root: string, args: readonly string[], signal?: AbortSignal): Promise<string> {
  const result = await runGitCommand(root, args, { signal, maxOutputBytes: 4096 });
  assertGitSuccess(result, args[0] ?? 'unknown');
  return result.stdout;
}

function assertGitSuccess(result: GitCommandResult, operation: string): void {
  if (result.exitCode !== 0) throw new Error(`HARNESS_AGENTIC_QE_GIT_FAILED:${operation}`);
}

function normalizedAbsolute(value: unknown, label: string): string {
  const path = asNonEmptyString(value, label);
  if (!isAbsolute(path) || resolve(path) !== path || path.includes('\0')) {
    throw new TypeError(`${label} must be an absolute normalized path`);
  }
  return path;
}

function opaqueId(value: unknown, label: string): string {
  const id = asNonEmptyString(value, label);
  if (!OPAQUE_ID.test(id)) throw new TypeError(`${label} is invalid`);
  return id;
}
