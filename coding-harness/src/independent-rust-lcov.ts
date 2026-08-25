// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import {
  chmodSync,
  lstatSync,
  realpathSync,
} from 'node:fs';
import { mkdir, mkdtemp } from 'node:fs/promises';
import { isAbsolute, join, relative, resolve, sep } from 'node:path';
import type { AgenticQeLcovArtifact } from './agentic-qe-lcov.js';
import {
  parseStructuredCommand,
  type HarnessConfig,
  type StructuredCommand,
} from './contracts.js';
import { runGitCommand } from './git-process.js';
import { runStructuredProcess, type ProcessResult } from './process.js';
import { digestValue } from './receipts.js';
import type { OfflineProcessIsolator } from './network.js';
import { sha256File } from './workspace.js';

export const CARGO_LLVM_COV_VERSION = 'cargo-llvm-cov 0.8.7' as const;
const GIT_OBJECT = /^[a-f0-9]{40,64}$/;
const SAFE_CARGO_NAME = /^[A-Za-z0-9_-]{1,128}$/;
const MAX_LCOV_BYTES = 20_000_000;

export interface IndependentRustLcovGeneratorOptions {
  readonly config: HarnessConfig;
  readonly rustProfile: Readonly<{
    cargoExecutable: string;
    environment: Readonly<Record<string, string>>;
    isolator: OfflineProcessIsolator;
  }>;
  readonly packageName: string;
  readonly testTarget: string;
  readonly timeoutMs?: number;
  readonly maxOutputBytes?: number;
}

export interface IndependentRustLcovInput {
  readonly controlledRoot: string;
  readonly candidateRoot: string;
  readonly candidateTree: string;
  readonly outputRoot: string;
  readonly signal?: AbortSignal;
}

export class IndependentRustLcovGenerator {
  readonly #config: HarnessConfig;
  readonly #profile: IndependentRustLcovGeneratorOptions['rustProfile'];
  readonly #packageName: string;
  readonly #testTarget: string;
  readonly #timeoutMs: number;
  readonly #maxOutputBytes: number;

  constructor(options: IndependentRustLcovGeneratorOptions) {
    if (!SAFE_CARGO_NAME.test(options.packageName)
      || !SAFE_CARGO_NAME.test(options.testTarget)) {
      throw new TypeError('HARNESS_INDEPENDENT_LCOV_TARGET_INVALID');
    }
    this.#config = options.config;
    this.#profile = options.rustProfile;
    this.#packageName = options.packageName;
    this.#testTarget = options.testTarget;
    this.#timeoutMs = bounded(
      options.timeoutMs ?? this.#config.limits.maxTimeoutMs,
      this.#config.limits.maxTimeoutMs,
      'timeoutMs',
    );
    this.#maxOutputBytes = bounded(
      options.maxOutputBytes ?? this.#config.limits.maxOutputBytes,
      this.#config.limits.maxOutputBytes,
      'maxOutputBytes',
    );
  }

  async capture(input: IndependentRustLcovInput): Promise<AgenticQeLcovArtifact> {
    const controlledRoot = privateDirectory(input.controlledRoot, 'CONTROLLED_ROOT');
    const candidateRoot = containedDirectory(
      controlledRoot,
      input.candidateRoot,
      'CANDIDATE_ROOT',
    );
    const outputRoot = containedDirectory(
      controlledRoot,
      input.outputRoot,
      'OUTPUT_ROOT',
    );
    if (pathsOverlap(candidateRoot, outputRoot) || !GIT_OBJECT.test(input.candidateTree)) {
      throw new Error('HARNESS_INDEPENDENT_LCOV_INPUT_INVALID');
    }
    const candidateIdentity = await assertCandidate(
      candidateRoot,
      input.candidateTree,
      input.signal,
    );
    const leaseRoot = await mkdtemp(join(outputRoot, 'agentic-qe-lcov-'));
    const targetRoot = join(leaseRoot, 'target');
    const lcovPath = join(leaseRoot, 'lcov.info');
    await mkdir(targetRoot, { mode: 0o700 });

    const versionCommand = this.#command(['llvm-cov', '--version'], {});
    const version = await this.#execute(
      versionCommand,
      candidateRoot,
      [leaseRoot],
      input.signal,
    );
    assertSuccessful(version, 'VERSION');
    if (`${version.stdout}${version.stderr}`.trim() !== CARGO_LLVM_COV_VERSION) {
      throw new Error('HARNESS_INDEPENDENT_LCOV_VERSION_MISMATCH');
    }

    const coverageCommand = this.#command([
      'llvm-cov',
      '--lcov',
      '--output-path',
      lcovPath,
      '--offline',
      '--locked',
      '--package',
      this.#packageName,
      '--test',
      this.#testTarget,
    ], { CARGO_TARGET_DIR: targetRoot });
    const coverage = await this.#execute(
      coverageCommand,
      candidateRoot,
      [leaseRoot],
      input.signal,
    );
    assertSuccessful(coverage, 'COVERAGE');
    const lcovDigest = sealLcov(lcovPath);
    await assertCandidate(
      candidateRoot,
      input.candidateTree,
      input.signal,
      candidateIdentity,
    );
    return Object.freeze({
      path: lcovPath,
      sha256: lcovDigest,
      provenance: 'independent-direct-coverage',
      coverageCommandDigest: digestValue({
        generatorVersion: CARGO_LLVM_COV_VERSION,
        candidateTree: input.candidateTree,
        versionCommand,
        version: stableResult(version),
        coverageCommand,
        coverage: stableResult(coverage),
        lcovSha256: lcovDigest,
      }),
      generatorVersion: CARGO_LLVM_COV_VERSION,
    });
  }

  #command(
    argv: readonly string[],
    extraEnvironment: Readonly<Record<string, string>>,
  ): StructuredCommand {
    return parseStructuredCommand({
      tool: 'cargo',
      executable: this.#profile.cargoExecutable,
      argv: [...argv],
      cwd: '.',
      env: { ...this.#profile.environment, ...extraEnvironment },
      timeoutMs: this.#timeoutMs,
      maxOutputBytes: this.#maxOutputBytes,
    }, this.#config, ['cargo']);
  }

  async #execute(
    command: StructuredCommand,
    candidateRoot: string,
    writablePaths: readonly string[],
    signal?: AbortSignal,
  ): Promise<ProcessResult> {
    return await runStructuredProcess(command, {
      workspaceRoot: candidateRoot,
      config: this.#config,
      declaredTools: ['cargo'],
      sourceEnvironment: {},
      signal,
      boundary: {
        kind: 'offline-candidate',
        isolator: this.#profile.isolator,
        writablePaths,
      },
    });
  }
}

async function assertCandidate(
  root: string,
  expectedTree: string,
  signal?: AbortSignal,
  expectedIdentity?: Readonly<{ dev: bigint; ino: bigint }>,
): Promise<Readonly<{ dev: bigint; ino: bigint }>> {
  const stat = lstatSync(root, { bigint: true });
  const identity = Object.freeze({ dev: stat.dev, ino: stat.ino });
  if (expectedIdentity !== undefined
    && (identity.dev !== expectedIdentity.dev || identity.ino !== expectedIdentity.ino)) {
    throw new Error('HARNESS_INDEPENDENT_LCOV_CANDIDATE_ROOT_CHANGED');
  }
  const top = await git(root, ['rev-parse', '--show-toplevel'], signal);
  const tree = await git(root, ['write-tree'], signal);
  const diff = await runGitCommand(root, ['diff', '--quiet', '--'], { signal });
  if (top !== root || tree !== expectedTree || diff.exitCode !== 0) {
    throw new Error('HARNESS_INDEPENDENT_LCOV_CANDIDATE_CHANGED');
  }
  return identity;
}

async function git(root: string, args: readonly string[], signal?: AbortSignal): Promise<string> {
  const result = await runGitCommand(root, args, { signal, maxOutputBytes: 4096 });
  if (result.exitCode !== 0) throw new Error(`HARNESS_INDEPENDENT_LCOV_GIT_FAILED:${args[0]}`);
  return result.stdout.trim();
}

function sealLcov(path: string): string {
  const stat = lstatSync(path);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1
    || realpathSync(path) !== path || stat.size < 1 || stat.size > MAX_LCOV_BYTES) {
    throw new Error('HARNESS_INDEPENDENT_LCOV_OUTPUT_INVALID');
  }
  chmodSync(path, 0o444);
  return sha256File(path);
}

function stableResult(result: ProcessResult): unknown {
  return {
    success: result.success,
    exitCode: result.exitCode,
    signal: result.signal,
    timedOut: result.timedOut,
    cancelled: result.cancelled,
    outputLimitExceeded: result.outputLimitExceeded,
    spawnError: result.spawnError,
    stdoutDigest: sha256(result.stdout),
    stderrDigest: sha256(result.stderr),
  };
}

function assertSuccessful(result: ProcessResult, stage: string): void {
  if (!result.success || result.exitCode !== 0 || result.signal !== null
    || result.timedOut || result.cancelled || result.outputLimitExceeded
    || result.spawnError !== null) {
    throw new Error(`HARNESS_INDEPENDENT_LCOV_${stage}_FAILED`);
  }
}

function privateDirectory(value: string, label: string): string {
  const path = canonicalDirectory(value, label);
  const stat = lstatSync(path);
  const uid = process.getuid?.() ?? stat.uid;
  if (stat.uid !== uid || (stat.mode & 0o077) !== 0) {
    throw new Error(`HARNESS_INDEPENDENT_LCOV_${label}_INVALID`);
  }
  return path;
}

function containedDirectory(root: string, value: string, label: string): string {
  const path = canonicalDirectory(value, label);
  if (!contains(root, path) || path === root) {
    throw new Error(`HARNESS_INDEPENDENT_LCOV_${label}_OUTSIDE_ROOT`);
  }
  return path;
}

function canonicalDirectory(value: string, label: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) {
    throw new TypeError(`HARNESS_INDEPENDENT_LCOV_${label}_INVALID`);
  }
  const stat = lstatSync(value);
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(value) !== value) {
    throw new Error(`HARNESS_INDEPENDENT_LCOV_${label}_INVALID`);
  }
  return value;
}

function contains(root: string, child: string): boolean {
  const delta = relative(root, child);
  return delta === ''
    || (delta !== '..' && !delta.startsWith(`..${sep}`) && !isAbsolute(delta));
}

function pathsOverlap(left: string, right: string): boolean {
  return contains(left, right) || contains(right, left);
}

function bounded(value: number, ceiling: number, label: string): number {
  if (!Number.isSafeInteger(value) || value < 1 || value > ceiling) {
    throw new TypeError(`HARNESS_INDEPENDENT_LCOV_${label.toUpperCase()}_INVALID`);
  }
  return value;
}

function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}
