// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  lstatSync,
  readFileSync,
  readdirSync,
  realpathSync,
  type BigIntStats,
} from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { isAbsolute, join, relative, resolve, sep } from 'node:path';
import {
  SHA256_PATTERN,
  asNonEmptyString,
  asRecord,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import {
  AGENTIC_QE_MAX_MCP_OUTPUT_BYTES,
  canonicalAgenticQeTimestamp,
} from './agentic-qe-lcov-response.js';
import {
  AGENTIC_QE_SAST_TOOL,
  parseNestedSastResult,
} from './agentic-qe-sast-response.js';
import { parseAgenticQeEvidence, type AgenticQeEvidence } from './evidence.js';
import { runGitCommand, type GitCommandResult } from './git-process.js';
import {
  assertGitMaterializationSafe,
  assertRawIndexMatchesWorkingTree,
} from './git-materialization.js';
import { digestValue } from './receipts.js';

export { AGENTIC_QE_SAST_TOOL } from './agentic-qe-sast-response.js';

const GIT_OBJECT = /^[a-f0-9]{40}(?:[a-f0-9]{24})?$/;
const OPAQUE_ID = /^[A-Za-z0-9_-]{8,128}$/;
const MAX_SNAPSHOT_FILES = 20_000;
const MAX_SNAPSHOT_BYTES = 250_000_000;
export const AGENTIC_QE_SAST_PROFILE = 'sast-only-flat-v1' as const;

export interface ProviderFreeAgenticQeSastMcpRequest {
  readonly executable: 'aqe-mcp';
  readonly transport: 'stdio-mcp';
  readonly method: 'tools/call';
  readonly toolName: typeof AGENTIC_QE_SAST_TOOL;
  readonly arguments: Readonly<{
    target: string;
    sast: true;
    dast: false;
  }>;
  readonly bindings: Readonly<{
    taskId: string;
    runId: string;
    candidateTree: string;
    snapshotSha256: string;
    profile: typeof AGENTIC_QE_SAST_PROFILE;
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

export interface AgenticQeSastInput {
  readonly taskId: string;
  readonly runId: string;
  readonly candidateTree: string;
  readonly candidateRoot: string;
}

export interface AgenticQeSastRunnerIdentity {
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

export interface ProviderFreeAgenticQeSastMcpRunner {
  invoke(request: ProviderFreeAgenticQeSastMcpRequest, signal?: AbortSignal): Promise<unknown>;
  identityEvidence(): AgenticQeSastRunnerIdentity;
  commandDigest(request: ProviderFreeAgenticQeSastMcpRequest): string;
}

export interface AgenticQeSastAdapterOptions {
  readonly runner: ProviderFreeAgenticQeSastMcpRunner;
  readonly clock?: () => Date;
  readonly snapshotParent?: string;
}

type DirectoryIdentity = Readonly<{ dev: bigint; ino: bigint }>;
type SnapshotIdentity = Readonly<{
  root: string;
  directory: DirectoryIdentity;
  sha256: string;
  fileCount: number;
  totalBytes: number;
}>;

export class AgenticQeSastEvidenceAdapter {
  readonly #runner: ProviderFreeAgenticQeSastMcpRunner;
  readonly #clock: () => Date;
  readonly #snapshotParent: string;

  constructor(options: AgenticQeSastAdapterOptions) {
    if (typeof options.runner?.invoke !== 'function'
      || typeof options.runner.identityEvidence !== 'function'
      || typeof options.runner.commandDigest !== 'function') {
      throw new TypeError('HARNESS_AGENTIC_QE_SAST_RUNNER_REQUIRED');
    }
    this.#runner = options.runner;
    this.#clock = options.clock ?? (() => new Date());
    this.#snapshotParent = options.snapshotParent === undefined
      ? tmpdir()
      : normalizedAbsolute(options.snapshotParent, 'snapshotParent');
  }

  async capture(rawInput: AgenticQeSastInput, signal?: AbortSignal): Promise<AgenticQeEvidence> {
    const input = parseInput(rawInput);
    const candidateIdentity = await assertCandidateRoot(
      input.candidateRoot,
      input.candidateTree,
      signal,
    );
    const snapshotRoot = await mkdtemp(join(this.#snapshotParent, 'semantic-fabric-aqe-sast-'));
    let snapshot: SnapshotIdentity | undefined;
    let response: unknown;
    let request: ProviderFreeAgenticQeSastMcpRequest | undefined;
    let runnerIdentity: AgenticQeSastRunnerIdentity | undefined;
    let commandDigest: string | undefined;
    const failures: unknown[] = [];
    try {
      await assertGitMaterializationSafe({ repositoryRoot: input.candidateRoot, signal });
      await gitChecked(input.candidateRoot, [
        'checkout-index', '--all', `--prefix=${snapshotRoot}${sep}`,
      ], signal);
      await assertRawIndexMatchesWorkingTree({
        workspaceRoot: snapshotRoot,
        repositoryRoot: input.candidateRoot,
        signal,
      });
      await assertGitMaterializationSafe({ repositoryRoot: input.candidateRoot, signal });
      snapshot = captureSnapshot(snapshotRoot);
      await assertCandidateRoot(
        input.candidateRoot,
        input.candidateTree,
        signal,
        candidateIdentity,
      );
      request = createRequest(input, snapshot);
      runnerIdentity = parseRunnerIdentity(this.#runner.identityEvidence());
      commandDigest = parseDigest(this.#runner.commandDigest(request), 'command digest');
      response = await this.#runner.invoke(request, signal);
    } catch (error) {
      failures.push(error);
    }
    if (snapshot !== undefined && request !== undefined
      && runnerIdentity !== undefined && commandDigest !== undefined) {
      try {
        assertSnapshotUnchanged(snapshot);
        await assertCandidateRoot(
          input.candidateRoot,
          input.candidateTree,
          undefined,
          candidateIdentity,
        );
        const currentIdentity = parseRunnerIdentity(this.#runner.identityEvidence());
        if (digestValue(currentIdentity) !== digestValue(runnerIdentity)
          || this.#runner.commandDigest(request) !== commandDigest) {
          throw new Error('HARNESS_AGENTIC_QE_SAST_RUNNER_CHANGED');
        }
      } catch (error) {
        failures.push(error);
      }
    }
    try {
      await rm(snapshotRoot, { recursive: true, force: true });
    } catch (error) {
      failures.push(error);
    }
    if (failures.length === 1) throw failures[0];
    if (failures.length > 1) {
      throw new AggregateError(failures, 'HARNESS_AGENTIC_QE_SAST_EXECUTION_FAILED');
    }
    const normalized = parseNestedSastResult(response, request!.arguments.target);
    const capturedAt = canonicalAgenticQeTimestamp(this.#clock());
    return parseAgenticQeEvidence({
      schemaVersion: 1,
      source: 'agentic-qe-local-profile',
      profile: 'sast',
      taskId: input.taskId,
      runId: input.runId,
      candidateTree: input.candidateTree,
      commandDigest,
      outputDigest: digestValue({
        taskId: input.taskId,
        runId: input.runId,
        candidateTree: input.candidateTree,
        snapshotSha256: snapshot!.sha256,
        profile: AGENTIC_QE_SAST_PROFILE,
        toolName: AGENTIC_QE_SAST_TOOL,
        runnerIdentity: digestValue(runnerIdentity),
        result: normalized,
      }),
      providerVariablesStripped: true,
      authoritative: false,
      capturedAt,
    });
  }
}

function createRequest(
  input: AgenticQeSastInput,
  snapshot: SnapshotIdentity,
): ProviderFreeAgenticQeSastMcpRequest {
  return deepFreeze({
    executable: 'aqe-mcp',
    transport: 'stdio-mcp',
    method: 'tools/call',
    toolName: AGENTIC_QE_SAST_TOOL,
    arguments: {
      target: snapshot.root,
      sast: true,
      dast: false,
    },
    bindings: {
      taskId: input.taskId,
      runId: input.runId,
      candidateTree: input.candidateTree,
      snapshotSha256: snapshot.sha256,
      profile: AGENTIC_QE_SAST_PROFILE,
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
        readOnlyPaths: [snapshot.root],
        privateHome: true,
        privateWritableTmp: true,
      },
      timeoutMs: 120_000,
      maxOutputBytes: AGENTIC_QE_MAX_MCP_OUTPUT_BYTES,
    },
  });
}

function parseInput(input: AgenticQeSastInput): AgenticQeSastInput {
  const taskId = opaqueId(input.taskId, 'taskId');
  const runId = opaqueId(input.runId, 'runId');
  if (!GIT_OBJECT.test(input.candidateTree)) {
    throw new TypeError('HARNESS_AGENTIC_QE_SAST_CANDIDATE_TREE_INVALID');
  }
  return deepFreeze({
    taskId,
    runId,
    candidateTree: input.candidateTree,
    candidateRoot: normalizedAbsolute(input.candidateRoot, 'candidateRoot'),
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
    throw new Error('HARNESS_AGENTIC_QE_SAST_CANDIDATE_ROOT_UNTRUSTED');
  }
  const identity = Object.freeze({ dev: stat.dev, ino: stat.ino });
  if (expected !== undefined && (identity.dev !== expected.dev || identity.ino !== expected.ino)) {
    throw new Error('HARNESS_AGENTIC_QE_SAST_CANDIDATE_ROOT_CHANGED');
  }
  const topLevel = (await gitChecked(root, ['rev-parse', '--show-toplevel'], signal)).trim();
  if (topLevel !== root) throw new Error('HARNESS_AGENTIC_QE_SAST_CANDIDATE_ROOT_NOT_WORKTREE');
  const tree = (await gitChecked(root, ['write-tree'], signal)).trim();
  if (tree !== candidateTree) throw new Error('HARNESS_AGENTIC_QE_SAST_CANDIDATE_TREE_MISMATCH');
  const worktree = await runGitCommand(root, ['diff', '--quiet', '--'], {
    signal,
    maxOutputBytes: 4096,
  });
  if (worktree.exitCode === 1) throw new Error('HARNESS_AGENTIC_QE_SAST_SOURCE_CHANGED');
  assertGitSuccess(worktree, 'diff');
  return identity;
}

function captureSnapshot(root: string): SnapshotIdentity {
  const stat = lstatSync(root, { bigint: true });
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(root) !== root) {
    throw new Error('HARNESS_AGENTIC_QE_SAST_SNAPSHOT_UNTRUSTED');
  }
  const hash = createHash('sha256');
  let fileCount = 0;
  let totalBytes = 0;
  const visit = (directory: string): void => {
    for (const entry of readdirSync(directory, { withFileTypes: true })
      .sort((left, right) => left.name < right.name ? -1 : left.name > right.name ? 1 : 0)) {
      const path = join(directory, entry.name);
      const child = relative(root, path).split(sep).join('/');
      const before = lstatSync(path, { bigint: true });
      if (entry.isDirectory() && before.isDirectory() && !before.isSymbolicLink()) {
        hash.update(`D\0${child}\0${String(before.mode)}\0`);
        visit(path);
      } else if (entry.isFile() && before.isFile() && !before.isSymbolicLink() && before.nlink === 1n) {
        const bytes = readFileSync(path);
        const after = lstatSync(path, { bigint: true });
        if (!sameFile(before, after) || BigInt(bytes.length) !== before.size) {
          throw new Error('HARNESS_AGENTIC_QE_SAST_SNAPSHOT_CHANGED');
        }
        fileCount += 1;
        totalBytes += bytes.length;
        if (fileCount > MAX_SNAPSHOT_FILES || totalBytes > MAX_SNAPSHOT_BYTES) {
          throw new Error('HARNESS_AGENTIC_QE_SAST_SNAPSHOT_LIMIT_EXCEEDED');
        }
        hash.update(`F\0${child}\0${String(before.mode)}\0${String(before.size)}\0`);
        hash.update(bytes);
      } else {
        throw new Error('HARNESS_AGENTIC_QE_SAST_SNAPSHOT_ENTRY_INVALID');
      }
    }
  };
  visit(root);
  if (fileCount === 0) throw new Error('HARNESS_AGENTIC_QE_SAST_SNAPSHOT_EMPTY');
  return Object.freeze({
    root,
    directory: Object.freeze({ dev: stat.dev, ino: stat.ino }),
    sha256: hash.digest('hex'),
    fileCount,
    totalBytes,
  });
}

function assertSnapshotUnchanged(expected: SnapshotIdentity): void {
  const current = captureSnapshot(expected.root);
  if (current.directory.dev !== expected.directory.dev
    || current.directory.ino !== expected.directory.ino
    || current.sha256 !== expected.sha256
    || current.fileCount !== expected.fileCount
    || current.totalBytes !== expected.totalBytes) {
    throw new Error('HARNESS_AGENTIC_QE_SAST_SNAPSHOT_CHANGED');
  }
}

function parseRunnerIdentity(value: unknown): AgenticQeSastRunnerIdentity {
  const identity = asRecord(value, 'Agentic-QE SAST runner identity');
  assertExactKeys(identity, ['node', 'bwrap', 'package'], 'Agentic-QE SAST runner identity');
  const node = parseExecutableIdentity(identity.node, 'node');
  const bwrap = parseExecutableIdentity(identity.bwrap, 'bwrap');
  const packageValue = asRecord(identity.package, 'Agentic-QE SAST package identity');
  assertExactKeys(packageValue, [
    'root', 'entryPath', 'name', 'version', 'entrySha256', 'treeSha256',
    'fileCount', 'totalBytes',
  ], 'Agentic-QE SAST package identity');
  if (packageValue.name !== 'agentic-qe') throw new Error('HARNESS_AGENTIC_QE_SAST_PACKAGE_INVALID');
  const fileCount = boundedInteger(packageValue.fileCount, 'package.fileCount', 100_000);
  const totalBytes = boundedInteger(packageValue.totalBytes, 'package.totalBytes', 2_000_000_000);
  return deepFreeze({
    node,
    bwrap,
    package: {
      root: normalizedAbsolute(packageValue.root, 'package.root'),
      entryPath: normalizedAbsolute(packageValue.entryPath, 'package.entryPath'),
      name: 'agentic-qe',
      version: boundedString(packageValue.version, 'package.version', 128),
      entrySha256: parseDigest(packageValue.entrySha256, 'package.entrySha256'),
      treeSha256: parseDigest(packageValue.treeSha256, 'package.treeSha256'),
      fileCount,
      totalBytes,
    },
  });
}

function parseExecutableIdentity(value: unknown, label: string): Readonly<{ path: string; sha256: string }> {
  const input = asRecord(value, `${label} identity`);
  assertExactKeys(input, ['path', 'sha256'], `${label} identity`);
  return {
    path: normalizedAbsolute(input.path, `${label}.path`),
    sha256: parseDigest(input.sha256, `${label}.sha256`),
  };
}

function sameFile(left: BigIntStats, right: BigIntStats): boolean {
  return left.dev === right.dev && left.ino === right.ino && left.mode === right.mode
    && left.size === right.size && left.mtimeMs === right.mtimeMs && left.ctimeMs === right.ctimeMs;
}

async function gitChecked(root: string, args: readonly string[], signal?: AbortSignal): Promise<string> {
  const result = await runGitCommand(root, args, { signal, maxOutputBytes: 4096 });
  assertGitSuccess(result, args[0] ?? 'unknown');
  return result.stdout;
}

function assertGitSuccess(result: GitCommandResult, operation: string): void {
  if (result.exitCode !== 0) throw new Error(`HARNESS_AGENTIC_QE_SAST_GIT_FAILED:${operation}`);
}

function normalizedAbsolute(value: unknown, label: string): string {
  const path = asNonEmptyString(value, label);
  if (!isAbsolute(path) || resolve(path) !== path || path.includes('\0')) {
    throw new TypeError(`${label} must be an absolute normalized path`);
  }
  return path;
}

function parseDigest(value: unknown, label: string): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value)) {
    throw new TypeError(`HARNESS_AGENTIC_QE_SAST_${label.toUpperCase().replaceAll('.', '_')}INVALID`);
  }
  return value;
}

function opaqueId(value: unknown, label: string): string {
  const id = asNonEmptyString(value, label);
  if (!OPAQUE_ID.test(id)) throw new TypeError(`${label} is invalid`);
  return id;
}

function boundedString(value: unknown, label: string, maxBytes: number): string {
  const text = asNonEmptyString(value, label);
  if (Buffer.byteLength(text, 'utf8') > maxBytes || text.includes('\0')) {
    throw new TypeError(`${label} is invalid`);
  }
  return text;
}

function boundedInteger(value: unknown, label: string, maximum: number): number {
  if (!Number.isSafeInteger(value) || (value as number) < 1 || (value as number) > maximum) {
    throw new TypeError(`${label} is invalid`);
  }
  return value as number;
}
