// SPDX-License-Identifier: MIT
import { constants, existsSync, lstatSync, realpathSync, readdirSync } from 'node:fs';
import { chmod, copyFile, mkdir, rm } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import {
  isForbiddenEnvironmentName,
  parseStructuredCommand,
  type HarnessConfig,
  type StructuredCommand,
} from './contracts.js';
import { runGitCommand } from './git-process.js';
import {
  assertGitMaterializationSafe,
  assertRawIndexMatchesWorkingTree,
} from './git-materialization.js';
import {
  parseFrozenCargoMetadata,
  type FrozenRegistryPackage,
} from './frozen-cargo-metadata.js';
import type { OfflineProcessIsolator } from './network.js';
import { runStructuredProcess, type ProcessResult } from './process.js';
import { digestValue, type GitIdentity } from './receipts.js';
import { ParentedResourceCleanupError } from './resource-cleanup.js';
import { resolveWorkspacePath, sha256File } from './workspace.js';
const CARGO_LOCK = 'Cargo.lock' as const;
const GIT_OBJECT = /^[a-f0-9]{40,64}$/;
const MAX_SNAPSHOT_ENTRIES = 100_000;
const MAX_LOCK_BYTES = 10_000_000;
type ExecutionKind = 'structured-offline' | 'native-offline';
type DirectoryIdentity = Readonly<{ dev: number; ino: number }>;
export interface FrozenCargoLockCommandRequest {
  readonly command: StructuredCommand;
  readonly workspaceRoot: string;
  readonly writablePaths: readonly string[];
  readonly network: Readonly<{ mode: 'offline'; allowedOrigins: readonly [] }>;
  readonly signal?: AbortSignal;
}
export interface FrozenCargoLockExecution {
  readonly kind: ExecutionKind;
  readonly network: Readonly<{ mode: 'offline'; allowedOrigins: readonly [] }>;
  readonly commandDigest: string;
  readonly result: ProcessResult;
}
export interface FrozenCargoLockExecutor {
  readonly kind: ExecutionKind;
  execute(request: FrozenCargoLockCommandRequest): Promise<FrozenCargoLockExecution>;
}
export interface FrozenCargoLockFile {
  readonly sourcePath: string;
  readonly workspacePath: typeof CARGO_LOCK;
  readonly digest: string;
}
export interface PreparedFrozenCargoLock {
  readonly lockfile: FrozenCargoLockFile;
  readonly registryPackages: readonly FrozenRegistryPackage[];
  readonly baseline: GitIdentity;
  readonly source: GitIdentity;
  assertStable(): void;
  cleanup(): Promise<void>;
}
export interface FrozenCargoLockPreparationInput {
  readonly repositoryRoot: string;
  readonly scratchRoot: string;
  readonly baseline: GitIdentity;
  readonly source: GitIdentity;
  readonly cargoExecutable: string;
  readonly cargoEnvironment: Readonly<Record<string, string>>;
  readonly config: HarnessConfig;
  readonly executor: FrozenCargoLockExecutor;
  readonly timeoutMs?: number;
  readonly maxOutputBytes?: number;
  readonly expectedDigest?: string;
  readonly targetTriple?: string;
  readonly signal?: AbortSignal;
}
export function createStructuredFrozenCargoLockExecutor(options: Readonly<{
  config: HarnessConfig;
  offlineIsolator: OfflineProcessIsolator;
  sourceEnvironment: Readonly<Record<string, string | undefined>>;
}>): FrozenCargoLockExecutor {
  return Object.freeze({
    kind: 'structured-offline' as const,
    async execute(request: FrozenCargoLockCommandRequest): Promise<FrozenCargoLockExecution> {
      assertOfflineNetwork(request.network);
      const result = await runStructuredProcess(request.command, {
        workspaceRoot: request.workspaceRoot,
        config: options.config,
        declaredTools: ['cargo'],
        sourceEnvironment: { ...options.sourceEnvironment },
        signal: request.signal,
        boundary: {
          kind: 'offline-candidate',
          isolator: options.offlineIsolator,
          writablePaths: [...request.writablePaths],
        },
      });
      return Object.freeze({
        kind: 'structured-offline',
        network: Object.freeze({ mode: 'offline', allowedOrigins: Object.freeze([]) as [] }),
        commandDigest: frozenCargoLockCommandDigest(request),
        result,
      });
    },
  });
}
export function frozenCargoLockCommandDigest(request: FrozenCargoLockCommandRequest): string {
  return digestValue({
    command: request.command,
    workspaceRoot: request.workspaceRoot,
    writablePaths: [...request.writablePaths],
    network: request.network,
  });
}
export async function prepareFrozenCargoLock(
  input: FrozenCargoLockPreparationInput,
): Promise<PreparedFrozenCargoLock> {
  const repositoryRoot = canonicalDirectory(input.repositoryRoot, 'HARNESS_FROZEN_LOCK_REPOSITORY_INVALID');
  const scratchRoot = normalizedAbsolute(input.scratchRoot, 'scratchRoot');
  const scratchParent = dirname(scratchRoot);
  if (scratchParent === scratchRoot || pathsOverlap(repositoryRoot, scratchRoot)) {
    throw new Error('HARNESS_FROZEN_LOCK_SCRATCH_OVERLAP');
  }
  const parentIdentity = privateDirectoryIdentity(
    scratchParent,
    'HARNESS_FROZEN_LOCK_SCRATCH_PARENT_INVALID',
  );
  assertAbsent(scratchRoot, 'HARNESS_FROZEN_LOCK_SCRATCH_EXISTS');
  assertIdentityShape(input.baseline, 'baseline');
  assertIdentityShape(input.source, 'source');
  const baseline = await verifiedIdentity(repositoryRoot, input.baseline, 'BASELINE', input.signal);
  const source = await verifiedIdentity(repositoryRoot, input.source, 'SOURCE', input.signal);
  await assertSourceCompatibility(repositoryRoot, baseline, source, input.signal);
  const execution = validateExecutionInput(input);
  let scratchIdentity: DirectoryIdentity | null = null;
  try {
    assertSameDirectory(
      privateDirectoryIdentity(scratchParent, 'HARNESS_FROZEN_LOCK_SCRATCH_PARENT_CHANGED'),
      parentIdentity,
      'HARNESS_FROZEN_LOCK_SCRATCH_PARENT_CHANGED',
    );
    await mkdir(scratchRoot, { mode: 0o700 });
    scratchIdentity = privateDirectoryIdentity(scratchRoot, 'HARNESS_FROZEN_LOCK_SCRATCH_INVALID');
    const workspaceRoot = join(scratchRoot, 'workspace');
    const targetRoot = join(scratchRoot, 'target');
    const frozenRoot = join(scratchRoot, 'frozen');
    await mkdir(workspaceRoot, { mode: 0o700 });
    await mkdir(targetRoot, { mode: 0o700 });
    await mkdir(frozenRoot, { mode: 0o700 });
    const indexPath = join(scratchRoot, 'baseline.index');
    await assertGitMaterializationSafe({ repositoryRoot,
      commits: [baseline.commit, source.commit], signal: input.signal });
    await gitChecked(
      repositoryRoot,
      ['read-tree', baseline.commit],
      input.signal,
      { GIT_INDEX_FILE: indexPath },
    );
    await gitChecked(
      repositoryRoot,
      [`--work-tree=${workspaceRoot}`, 'checkout-index', '--all', '--force', `--prefix=${workspaceRoot}${sep}`],
      input.signal,
      { GIT_INDEX_FILE: indexPath },
    );
    await assertRawIndexMatchesWorkingTree({ workspaceRoot, repositoryRoot,
      environment: { GIT_INDEX_FILE: indexPath }, signal: input.signal });
    await assertGitMaterializationSafe({ repositoryRoot,
      commits: [baseline.commit, source.commit], signal: input.signal });
    assertOwnedScratch(scratchRoot, scratchIdentity, scratchParent, parentIdentity);
    validateSnapshot(workspaceRoot);
    resolveWorkspacePath(workspaceRoot, 'Cargo.toml', {
      requireRegularFile: true,
      rejectHardlinks: true,
    });
    const environment = {
      ...execution.environment,
      CARGO_NET_OFFLINE: 'true',
      CARGO_TARGET_DIR: targetRoot,
    };
    await executeCargo(input, workspaceRoot, [workspaceRoot, targetRoot], environment,
      ['generate-lockfile', '--offline'], 'GENERATE');
    const generatedLock = validateLockfile(workspaceRoot);
    const generatedIdentity = fileIdentity(generatedLock);
    const generatedDigest = sha256File(generatedLock);
    if (input.expectedDigest !== undefined && generatedDigest !== input.expectedDigest) {
      throw new Error('HARNESS_FROZEN_LOCK_DIGEST_MISMATCH');
    }
    const metadataArguments = ['metadata', '--locked', '--offline'];
    if (input.targetTriple !== undefined) {
      if (!/^[A-Za-z0-9_][A-Za-z0-9_.-]{2,127}$/.test(input.targetTriple)) {
        throw new Error('HARNESS_FROZEN_LOCK_TARGET_INVALID');
      }
      metadataArguments.push('--filter-platform', input.targetTriple);
    }
    metadataArguments.push('--format-version', '1');
    const metadata = await executeCargo(input, workspaceRoot, [workspaceRoot, targetRoot], environment,
      metadataArguments, 'METADATA');
    const registryPackages = parseFrozenCargoMetadata(metadata.stdout, workspaceRoot, targetRoot);
    if (!sameFile(generatedIdentity, fileIdentity(generatedLock))
      || generatedDigest !== sha256File(generatedLock)) {
      throw new Error('HARNESS_FROZEN_LOCK_CHANGED_DURING_METADATA');
    }
    const sourcePath = join(frozenRoot, CARGO_LOCK);
    await copyFile(generatedLock, sourcePath, constants.COPYFILE_EXCL);
    await chmod(sourcePath, 0o444);
    const digest = validateImmutableLock(sourcePath);
    assertOwnedScratch(scratchRoot, scratchIdentity, scratchParent, parentIdentity);
    return new FrozenCargoLockLease(
      Object.freeze({ sourcePath, workspacePath: CARGO_LOCK, digest }),
      registryPackages,
      baseline,
      source,
      scratchRoot,
      scratchIdentity,
      scratchParent,
      parentIdentity,
    );
  } catch (error) {
    if (scratchIdentity === null) throw error;
    try {
      assertOwnedScratch(scratchRoot, scratchIdentity, scratchParent, parentIdentity);
      await rm(scratchRoot, { recursive: true, force: true });
    } catch (cleanupError) {
      throw new ParentedResourceCleanupError(
        [error, cleanupError], 'HARNESS_FROZEN_LOCK_PREPARE_AND_CLEANUP_FAILED',
      );
    }
    throw error;
  }
}
class FrozenCargoLockLease implements PreparedFrozenCargoLock {
  #disposed = false;
  constructor(
    readonly lockfile: FrozenCargoLockFile,
    readonly registryPackages: readonly FrozenRegistryPackage[],
    readonly baseline: GitIdentity,
    readonly source: GitIdentity,
    private readonly scratchRoot: string,
    private readonly scratchIdentity: DirectoryIdentity,
    private readonly scratchParent: string,
    private readonly parentIdentity: DirectoryIdentity,
  ) {
    Object.freeze(this.lockfile);
    Object.freeze(this.registryPackages);
    Object.freeze(this.baseline);
    Object.freeze(this.source);
  }
  assertStable(): void {
    if (this.#disposed) throw new Error('HARNESS_FROZEN_LOCK_DISPOSED');
    assertOwnedScratch(
      this.scratchRoot,
      this.scratchIdentity,
      this.scratchParent,
      this.parentIdentity,
    );
    if (validateImmutableLock(this.lockfile.sourcePath) !== this.lockfile.digest) {
      throw new Error('HARNESS_FROZEN_LOCK_DIGEST_CHANGED');
    }
  }
  async cleanup(): Promise<void> {
    if (this.#disposed) return;
    assertOwnedScratch(
      this.scratchRoot,
      this.scratchIdentity,
      this.scratchParent,
      this.parentIdentity,
    );
    await rm(this.scratchRoot, { recursive: true, force: true });
    this.#disposed = true;
  }
}
async function executeCargo(
  input: FrozenCargoLockPreparationInput,
  workspaceRoot: string,
  writablePaths: readonly string[],
  environment: Readonly<Record<string, string>>,
  argv: readonly string[],
  stage: 'GENERATE' | 'METADATA',
): Promise<ProcessResult> {
  const command = parseStructuredCommand({
    tool: 'cargo',
    executable: input.cargoExecutable,
    argv: [...argv],
    cwd: '.',
    env: { ...environment },
    timeoutMs: input.timeoutMs ?? Math.min(300_000, input.config.limits.maxTimeoutMs),
    maxOutputBytes: input.maxOutputBytes ?? Math.min(10_000_000, input.config.limits.maxOutputBytes),
  }, input.config, ['cargo']);
  const request: FrozenCargoLockCommandRequest = Object.freeze({
    command,
    workspaceRoot,
    writablePaths: Object.freeze([...writablePaths]),
    network: Object.freeze({ mode: 'offline', allowedOrigins: Object.freeze([]) as [] }),
    signal: input.signal,
  });
  const evidence = await input.executor.execute(request);
  if (evidence?.kind !== input.executor.kind) throw new Error('HARNESS_FROZEN_LOCK_EXECUTOR_MISMATCH');
  assertOfflineNetwork(evidence.network);
  if (evidence.commandDigest !== frozenCargoLockCommandDigest(request)) {
    throw new Error('HARNESS_FROZEN_LOCK_COMMAND_BINDING_MISMATCH');
  }
  if (!normallyCompleted(evidence.result)) throw new Error(`HARNESS_FROZEN_LOCK_${stage}_FAILED`);
  return evidence.result;
}
function validateExecutionInput(input: FrozenCargoLockPreparationInput): Readonly<{
  environment: Readonly<Record<string, string>>;
}> {
  if (input.executor === undefined
    || !['structured-offline', 'native-offline'].includes(input.executor.kind)) {
    throw new Error('HARNESS_FROZEN_LOCK_OFFLINE_EXECUTOR_REQUIRED');
  }
  if (!isAbsolute(input.cargoExecutable) || resolve(input.cargoExecutable) !== input.cargoExecutable
    || input.cargoExecutable.includes('\0')) {
    throw new Error('HARNESS_FROZEN_LOCK_CARGO_EXECUTABLE_INVALID');
  }
  const environment: Record<string, string> = {};
  for (const [name, value] of Object.entries(input.cargoEnvironment)) {
    if (isForbiddenEnvironmentName(name, input.config) || value.includes('\0')) {
      throw new Error(`HARNESS_FROZEN_LOCK_ENVIRONMENT_FORBIDDEN:${name}`);
    }
    environment[name] = value;
  }
  if (environment.CARGO_NET_OFFLINE !== 'true') {
    throw new Error('HARNESS_FROZEN_LOCK_NETWORK_MISMATCH');
  }
  if (environment.CARGO_TARGET_DIR !== undefined) {
    throw new Error('HARNESS_FROZEN_LOCK_TARGET_PATH_PREDECLARED');
  }
  return Object.freeze({ environment: Object.freeze(environment) });
}
async function verifiedIdentity(
  repositoryRoot: string,
  declared: GitIdentity,
  label: 'BASELINE' | 'SOURCE',
  signal?: AbortSignal,
): Promise<GitIdentity> {
  const commit = (await gitChecked(
    repositoryRoot,
    ['rev-parse', '--verify', `${declared.commit}^{commit}`],
    signal,
  )).stdout.trim();
  const tree = (await gitChecked(
    repositoryRoot,
    ['rev-parse', '--verify', `${declared.commit}^{tree}`],
    signal,
  )).stdout.trim();
  if (commit !== declared.commit) throw new Error(`HARNESS_FROZEN_LOCK_${label}_COMMIT_MISMATCH`);
  if (tree !== declared.tree) throw new Error(`HARNESS_FROZEN_LOCK_${label}_TREE_MISMATCH`);
  return Object.freeze({ commit, tree });
}
async function assertSourceCompatibility(
  repositoryRoot: string,
  baseline: GitIdentity,
  source: GitIdentity,
  signal?: AbortSignal,
): Promise<void> {
  const ancestry = await runGitCommand(
    repositoryRoot,
    ['merge-base', '--is-ancestor', baseline.commit, source.commit],
    { signal },
  );
  if (ancestry.exitCode !== 0) throw new Error('HARNESS_FROZEN_LOCK_SOURCE_NOT_DESCENDANT');
  const manifests = await runGitCommand(repositoryRoot, [
    'diff', '--quiet', baseline.commit, source.commit, '--',
    'Cargo.toml', 'Cargo.lock', '.cargo/config', '.cargo/config.toml',
    'rust-toolchain', 'rust-toolchain.toml', ':(glob)**/Cargo.toml',
  ], { signal });
  if (manifests.exitCode !== 0) throw new Error('HARNESS_FROZEN_LOCK_SOURCE_MANIFEST_MISMATCH');
}
async function gitChecked(
  cwd: string,
  args: readonly string[],
  signal?: AbortSignal,
  environment?: Readonly<Record<string, string>>,
) {
  const result = await runGitCommand(cwd, args, { signal, environment });
  if (result.exitCode !== 0) throw new Error(`HARNESS_FROZEN_LOCK_GIT_FAILED:${args[0]}`);
  return result;
}
function validateSnapshot(root: string): void {
  const pending = [root];
  let entries = 0;
  while (pending.length > 0) {
    const directory = pending.pop()!;
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      entries += 1;
      if (entries > MAX_SNAPSHOT_ENTRIES) throw new Error('HARNESS_FROZEN_LOCK_SNAPSHOT_TOO_LARGE');
      if (entry.name === '.git') throw new Error('HARNESS_FROZEN_LOCK_GIT_METADATA_EXPOSED');
      const path = join(directory, entry.name);
      const stat = lstatSync(path);
      if (stat.isSymbolicLink() || (!stat.isDirectory() && !stat.isFile())
        || (stat.isFile() && stat.nlink > 1)) {
        throw new Error('HARNESS_FROZEN_LOCK_SNAPSHOT_ENTRY_INVALID');
      }
      if (stat.isDirectory()) pending.push(path);
    }
  }
}
function validateLockfile(workspaceRoot: string): string {
  let path: string;
  try {
    path = resolveWorkspacePath(workspaceRoot, CARGO_LOCK, {
      requireRegularFile: true,
      rejectHardlinks: true,
    });
  } catch {
    throw new Error('HARNESS_FROZEN_LOCK_MISSING');
  }
  const stat = lstatSync(path);
  if (stat.size < 1 || stat.size > MAX_LOCK_BYTES) throw new Error('HARNESS_FROZEN_LOCK_SIZE_INVALID');
  return path;
}
function validateImmutableLock(path: string): string {
  const stat = lstatSync(path);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1
    || realpathSync(path) !== path || (stat.mode & 0o222) !== 0
    || stat.size < 1 || stat.size > MAX_LOCK_BYTES) {
    throw new Error('HARNESS_FROZEN_LOCK_IMMUTABILITY_INVALID');
  }
  return sha256File(path);
}
function normallyCompleted(result: ProcessResult): boolean {
  return result?.success === true && result.exitCode === 0 && result.signal === null
    && result.timedOut === false && result.cancelled === false
    && result.outputLimitExceeded === false && result.spawnError === null
    && typeof result.stdout === 'string' && typeof result.stderr === 'string';
}
function assertOfflineNetwork(value: FrozenCargoLockExecution['network']): void {
  if (value?.mode !== 'offline' || !Array.isArray(value.allowedOrigins)
    || value.allowedOrigins.length !== 0) {
    throw new Error('HARNESS_FROZEN_LOCK_NETWORK_MISMATCH');
  }
}
function assertIdentityShape(value: GitIdentity, label: string): void {
  if (value === null || typeof value !== 'object'
    || !GIT_OBJECT.test(value.commit) || !GIT_OBJECT.test(value.tree)) {
    throw new TypeError(`HARNESS_FROZEN_LOCK_${label.toUpperCase()}_IDENTITY_INVALID`);
  }
}
function fileIdentity(path: string): Readonly<{ dev: number; ino: number }> {
  const stat = lstatSync(path);
  return Object.freeze({ dev: stat.dev, ino: stat.ino });
}
function sameFile(left: Readonly<{ dev: number; ino: number }>, right: Readonly<{ dev: number; ino: number }>): boolean {
  return left.dev === right.dev && left.ino === right.ino;
}
function assertOwnedScratch(
  scratchRoot: string,
  scratchIdentity: DirectoryIdentity,
  scratchParent: string,
  parentIdentity: DirectoryIdentity,
): void {
  assertSameDirectory(
    privateDirectoryIdentity(scratchParent, 'HARNESS_FROZEN_LOCK_SCRATCH_PARENT_CHANGED'),
    parentIdentity,
    'HARNESS_FROZEN_LOCK_SCRATCH_PARENT_CHANGED',
  );
  assertSameDirectory(
    privateDirectoryIdentity(scratchRoot, 'HARNESS_FROZEN_LOCK_SCRATCH_CHANGED'),
    scratchIdentity,
    'HARNESS_FROZEN_LOCK_SCRATCH_CHANGED',
  );
}
function privateDirectoryIdentity(path: string, error: string): DirectoryIdentity {
  const canonical = canonicalDirectory(path, error);
  const stat = lstatSync(canonical);
  const uid = process.getuid?.();
  if ((stat.mode & 0o077) !== 0 || (uid !== undefined && stat.uid !== uid)) throw new Error(error);
  return Object.freeze({ dev: stat.dev, ino: stat.ino });
}
function canonicalDirectory(value: string, error: string): string {
  const path = normalizedAbsolute(value, error);
  const stat = lstatSync(path);
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(path) !== path) throw new Error(error);
  return path;
}
function normalizedAbsolute(value: string, label: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) {
    throw new TypeError(`${label} must be an absolute normalized path`);
  }
  return value;
}
function assertAbsent(path: string, error: string): void {
  if (existsSync(path)) throw new Error(error);
}
function assertSameDirectory(left: DirectoryIdentity, right: DirectoryIdentity, error: string): void {
  if (left.dev !== right.dev || left.ino !== right.ino) throw new Error(error);
}
function pathsOverlap(left: string, right: string): boolean {
  const delta = relative(left, right);
  const inverse = relative(right, left);
  return delta === '' || (!delta.startsWith(`..${sep}`) && delta !== '..' && !isAbsolute(delta))
    || (!inverse.startsWith(`..${sep}`) && inverse !== '..' && !isAbsolute(inverse));
}
