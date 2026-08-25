// SPDX-License-Identifier: MIT
import { createHash } from 'node:crypto';
import { constants, existsSync, lstatSync, readFileSync, realpathSync } from 'node:fs';
import { chmod, copyFile, mkdir, rm } from 'node:fs/promises';
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path';
import { normalizeWorkspacePath } from './contracts.js';
import {
  assertGitMaterializationSafe,
  assertRawIndexMatchesWorkingTree,
} from './git-materialization.js';
import { gitAbortError, runGitCommand, type GitCommandResult } from './git-process.js';
import type { GitIdentity } from './receipts.js';
export interface PreparedWorktrees {
  baseline: GitIdentity;
  evaluator: GitIdentity;
  candidate: GitIdentity;
  candidateRoot: string;
  evaluatorRoot: string;
  verifierRoots: Readonly<Record<VerifierWorkspace, string>>;
}
export interface AppliedPatch {
  candidate: GitIdentity;
  patchDigest: string;
  admittedPaths: string[];
}
const GIT_OBJECT = /^[a-f0-9]{40,64}$/;
const MAX_PATCH_BYTES = 10_000_000;
export type VerifierWorkspace = 'public' | 'independent' | 'regression';
export type ExecutionLane = 'evaluator' | 'candidate' | VerifierWorkspace;
const VERIFIER_WORKSPACES = Object.freeze([
  'public', 'independent', 'regression'] as const satisfies readonly VerifierWorkspace[]);
type DirectoryIdentity = Readonly<{ dev: bigint; ino: bigint }>;
export class GitWorktreeSet {
  readonly #repositoryRoot: string;
  readonly #repositoryIdentity: DirectoryIdentity;
  readonly #runRoot: string;
  readonly #runParent: string;
  readonly #parentIdentity: DirectoryIdentity;
  readonly #candidateRoot: string;
  readonly #evaluatorRoot: string;
  readonly #verifierRoots: Readonly<Record<VerifierWorkspace, string>>;
  readonly #outputRoots: Readonly<Record<ExecutionLane, string>>;
  readonly #registered = new Set<string>();
  readonly #installedOverlayPaths = new Set<string>();
  #baseline: GitIdentity | null = null;
  #evaluator: GitIdentity | null = null;
  #preparation: PreparedWorktrees | null = null;
  #runRootIdentity: DirectoryIdentity | null = null;
  constructor(input: { repositoryRoot: string; runRoot: string }) {
    this.#repositoryRoot = normalizedAbsolute(input.repositoryRoot, 'repositoryRoot');
    this.#repositoryIdentity = directoryIdentity(
      this.#repositoryRoot, 'HARNESS_REPOSITORY_ROOT_INVALID',
    );
    this.#runRoot = normalizedAbsolute(input.runRoot, 'runRoot');
    if (pathsOverlap(this.#repositoryRoot, this.#runRoot)) throw new Error('HARNESS_WORKTREE_ROOT_OVERLAP');
    this.#runParent = dirname(this.#runRoot);
    if (this.#runParent === dirname(this.#runParent)) throw new Error('HARNESS_WORKTREE_ROOT_BROAD');
    this.#parentIdentity = directoryIdentity(this.#runParent, 'HARNESS_WORKTREE_PARENT_UNTRUSTED', 'trusted');
    assertAbsent(this.#runRoot);
    this.#candidateRoot = resolve(this.#runRoot, 'candidate');
    this.#evaluatorRoot = resolve(this.#runRoot, 'evaluator');
    this.#verifierRoots = Object.freeze({
      public: resolve(this.#runRoot, 'verifier-public'),
      independent: resolve(this.#runRoot, 'verifier-independent'),
      regression: resolve(this.#runRoot, 'verifier-regression'),
    });
    this.#outputRoots = Object.freeze({
      evaluator: resolve(this.#runRoot, 'outputs', 'evaluator'),
      candidate: resolve(this.#runRoot, 'outputs', 'candidate'),
      public: resolve(this.#runRoot, 'outputs', 'public'),
      independent: resolve(this.#runRoot, 'outputs', 'independent'),
      regression: resolve(this.#runRoot, 'outputs', 'regression'),
    });
  }
  async prepare(
    baselineCommit: string,
    evaluatorCommit: string,
    signal?: AbortSignal,
  ): Promise<PreparedWorktrees> {
    assertGitObject(baselineCommit, 'baselineCommit');
    assertGitObject(evaluatorCommit, 'evaluatorCommit');
    if (this.#preparation !== null) {
      if (baselineCommit !== this.#preparation.baseline.commit
        || evaluatorCommit !== this.#preparation.evaluator.commit) {
        throw new Error('HARNESS_WORKTREE_PREPARATION_IDENTITY_MISMATCH');
      }
      return this.#preparation;
    }
    this.#assertTrustedParent();
    assertAbsent(this.#runRoot);
    await this.#assertCommit(baselineCommit, signal);
    await this.#assertCommit(evaluatorCommit, signal);
    await this.#assertMaterializationSafe([baselineCommit, evaluatorCommit], signal);
    await mkdir(this.#runRoot, { mode: 0o700 });
    try {
      this.#runRootIdentity = directoryIdentity(this.#runRoot, 'HARNESS_WORKTREE_ROOT_NOT_PRIVATE', 'private');
      this.#assertTrustedParent();
      for (const lane of ['evaluator', 'candidate', ...VERIFIER_WORKSPACES] as const) {
        await mkdir(this.#outputRoots[lane], { recursive: true, mode: 0o700 });
      }
      await this.#addWorktree(this.#candidateRoot, evaluatorCommit, signal);
      await this.#addWorktree(this.#evaluatorRoot, evaluatorCommit, signal);
      for (const stage of VERIFIER_WORKSPACES) {
        await this.#addWorktree(this.#verifierRoots[stage], evaluatorCommit, signal);
      }
      this.#baseline = await this.#identityForCommit(baselineCommit, signal);
      this.#evaluator = await this.#identityAt(this.#evaluatorRoot, false, signal);
      const candidate = await this.#identityAt(this.#candidateRoot, true, signal);
      if (!sameIdentity(candidate, this.#evaluator)) throw new Error('HARNESS_CANDIDATE_NOT_FROZEN_EVALUATOR');
      await this.#assertClean(this.#candidateRoot);
      await this.#assertClean(this.#evaluatorRoot);
      for (const stage of VERIFIER_WORKSPACES) await this.#assertClean(this.#verifierRoots[stage]);
      await this.#assertMaterializationSafe([baselineCommit, evaluatorCommit], signal);
      this.#preparation = Object.freeze({
        baseline: this.#baseline,
        evaluator: this.#evaluator,
        candidate,
        candidateRoot: this.#candidateRoot,
        evaluatorRoot: this.#evaluatorRoot,
        verifierRoots: { ...this.#verifierRoots },
      });
      return this.#preparation;
    } catch (error) {
      try {
        await this.dispose();
      } catch (cleanupError) {
        throw new AggregateError([error, cleanupError], 'HARNESS_WORKTREE_PREPARE_AND_CLEANUP_FAILED');
      }
      throw error;
    }
  }
  async admitAndApply(
    patch: string,
    mutablePaths: readonly string[],
    signal?: AbortSignal,
  ): Promise<AppliedPatch> {
    this.#assertPrepared();
    if (typeof patch !== 'string' || patch.length === 0 || patch.includes('\0')) {
      throw new TypeError('HARNESS_PATCH_INVALID');
    }
    if (Buffer.byteLength(patch, 'utf8') > MAX_PATCH_BYTES) throw new Error('HARNESS_PATCH_TOO_LARGE');
    const allowed = new Set(mutablePaths.map((path, index) =>
      normalizeWorkspacePath(path, `mutablePaths[${index}]`)));
    for (const root of this.#mutableRoots()) await this.#assertClean(root, signal);
    const numstat = await this.#git(
      this.#candidateRoot,
      ['apply', '--numstat', '-z', '--whitespace=error-all', '-'],
      patch,
      signal,
    );
    const admittedPaths = parseNumstat(numstat.stdout);
    if (admittedPaths.length === 0) throw new Error('HARNESS_PATCH_EMPTY');
    const undeclared = admittedPaths.filter((path) => !allowed.has(path));
    if (undeclared.length > 0) throw new Error(`HARNESS_PATCH_PATH_NOT_DECLARED:${undeclared.join(',')}`);
    try {
      for (const root of this.#mutableRoots()) {
        await this.#applyChecked(root, patch, admittedPaths, signal);
      }
      const candidate = await this.#identityAt(this.#candidateRoot, true, signal);
      if (candidate.tree === this.#evaluator?.tree) throw new Error('HARNESS_PATCH_TREE_UNCHANGED');
      for (const stage of VERIFIER_WORKSPACES) {
        const verifier = await this.#identityAt(this.#verifierRoots[stage], true, signal);
        if (!sameIdentity(candidate, verifier)) throw new Error(`HARNESS_VERIFIER_PATCH_DIVERGED:${stage}`);
      }
      return Object.freeze({
        candidate,
        patchDigest: createHash('sha256').update(patch, 'utf8').digest('hex'),
        admittedPaths: Object.freeze([...admittedPaths]) as unknown as string[],
      });
    } catch (error) {
      await this.resetCandidate(signal);
      throw error;
    }
  }
  async resetCandidate(signal?: AbortSignal): Promise<void> {
    this.#assertPrepared();
    await this.#assertMaterializationSafe([this.#evaluator!.commit], signal);
    for (const root of this.#overlayRoots()) {
      this.#assertOwnedRunRoot();
      this.#assertControlledPath(root);
      await this.#git(root, ['reset', '--hard', '--quiet', this.#evaluator!.commit], undefined, signal);
      await this.#git(root, ['clean', '-fdx', '--'], undefined, signal);
      await this.#assertClean(root, signal);
    }
    await this.#assertMaterializationSafe([this.#evaluator!.commit], signal);
    for (const lane of ['candidate', ...VERIFIER_WORKSPACES] as const) {
      const output = this.#outputRoots[lane];
      this.#assertOwnedRunRoot();
      this.#assertControlledPath(output);
      await rm(output, { recursive: true, force: true });
      await mkdir(output, { recursive: true, mode: 0o700 });
    }
  }
  async baselineIdentity(): Promise<GitIdentity> {
    this.#assertPrepared();
    return { ...this.#baseline! };
  }
  async candidateIdentity(signal?: AbortSignal): Promise<GitIdentity> {
    this.#assertPrepared();
    return await this.#identityAt(this.#candidateRoot, true, signal);
  }
  candidateRoot(): string {
    this.#assertPrepared();
    return this.#candidateRoot;
  }
  evaluatorRoot(): string {
    this.#assertPrepared();
    return this.#evaluatorRoot;
  }
  verifierRoot(stage: VerifierWorkspace): string {
    this.#assertPrepared();
    return this.#verifierRoots[stage];
  }
  outputRoot(lane: ExecutionLane): string {
    this.#assertPrepared();
    return this.#outputRoots[lane];
  }
  controlledRoot(): string {
    this.#assertPrepared();
    return this.#runRoot;
  }
  async verifierIdentity(stage: VerifierWorkspace, signal?: AbortSignal): Promise<GitIdentity> {
    this.#assertPrepared();
    return await this.#identityAt(this.#verifierRoots[stage], true, signal);
  }
  async assertCandidateSourceStable(
    allowedUntrackedPaths: readonly string[] = [],
    signal?: AbortSignal,
  ): Promise<void> {
    this.#assertPrepared();
    await this.#assertSourceStable(
      this.#candidateRoot,
      [...allowedUntrackedPaths, ...this.#installedOverlayPaths],
      'HARNESS_CANDIDATE_SOURCE_MUTATED_AFTER_ADMISSION',
      signal,
    );
  }
  async assertVerifierSourceStable(stage: VerifierWorkspace, signal?: AbortSignal): Promise<void> {
    this.#assertPrepared();
    await this.#assertSourceStable(
      this.#verifierRoots[stage],
      [...this.#installedOverlayPaths],
      `HARNESS_VERIFIER_SOURCE_MUTATED:${stage}`,
      signal,
    );
  }
  async installFrozenOverlay(
    sourcePath: string,
    workspacePath: string,
    digest: string,
    signal?: AbortSignal,
  ): Promise<void> {
    this.#assertPrepared();
    assertActive(signal);
    const source = normalizedAbsolute(sourcePath, 'overlay source');
    const sourceStat = lstatSync(source);
    if (sourceStat.isSymbolicLink() || !sourceStat.isFile() || sourceStat.nlink > 1
      || realpathSync(source) !== source || fileDigest(source) !== digest) {
      throw new Error('HARNESS_FROZEN_OVERLAY_SOURCE_INVALID');
    }
    const relativePath = normalizeWorkspacePath(workspacePath, 'overlay workspace path');
    for (const root of this.#overlayRoots()) {
      assertActive(signal);
      const target = resolve(root, relativePath);
      this.#assertControlledPath(target);
      if (!existsSync(target)) {
        await copyFile(source, target, constants.COPYFILE_EXCL);
      }
      validateFrozenFile(target, digest, 'HARNESS_FROZEN_OVERLAY_DIGEST_MISMATCH');
      await chmod(target, 0o444);
    }
    this.#installedOverlayPaths.add(relativePath);
  }
  verifyFrozenOverlay(workspacePath: string, digest: string): void {
    this.#assertPrepared();
    const relativePath = normalizeWorkspacePath(workspacePath, 'overlay workspace path');
    for (const root of this.#overlayRoots()) {
      const target = resolve(root, relativePath);
      this.#assertControlledPath(target);
      validateFrozenFile(target, digest, 'HARNESS_FROZEN_OVERLAY_CHANGED', true);
    }
  }
  async dispose(): Promise<void> {
    const failures: Error[] = [];
    if (this.#runRootIdentity !== null) this.#assertOwnedRunRoot();
    const registered = await this.#registeredWorktreePaths();
    for (const path of [...this.#registered]) {
      this.#assertOwnedRunRoot();
      this.#assertControlledPath(path);
      if (!existsSync(path) && !registered.has(path)) continue;
      try {
        await this.#git(this.#repositoryRoot, ['worktree', 'remove', '--force', '--', path]);
      } catch (error) {
        failures.push(error instanceof Error ? error : new Error(String(error)));
      }
    }
    const remaining = await this.#registeredWorktreePaths();
    for (const path of this.#registered) {
      if (remaining.has(path)) failures.push(new Error(`HARNESS_WORKTREE_CLEANUP_STALE:${path}`));
    }
    if (failures.length > 0) {
      throw new AggregateError(failures, 'HARNESS_WORKTREE_CLEANUP_FAILED');
    }
    this.#registered.clear();
    this.#installedOverlayPaths.clear();
    this.#preparation = null;
    this.#baseline = null;
    this.#evaluator = null;
    if (this.#runRootIdentity !== null) {
      this.#assertOwnedRunRoot();
      await rm(this.#runRoot, { recursive: true, force: true });
      this.#runRootIdentity = null;
    }
  }
  async #addWorktree(path: string, commit: string, signal?: AbortSignal): Promise<void> {
    this.#assertOwnedRunRoot();
    try {
      await this.#git(this.#repositoryRoot, ['worktree', 'add', '--detach', '--', path, commit], undefined, signal);
    } catch (error) {
      if ((await this.#registeredWorktreePaths()).has(path)) this.#registered.add(path);
      throw error;
    }
    this.#registered.add(path);
  }
  async #assertSourceStable(
    root: string,
    allowedUntrackedPaths: readonly string[],
    error: string,
    signal?: AbortSignal,
  ): Promise<void> {
    this.#assertRepositoryRoot();
    const diff = await runGitCommand(root, ['diff', '--quiet', '--'], { signal });
    if (diff.exitCode !== 0) throw new Error(error);
    const untracked = parseNullPaths((await this.#git(
      root,
      ['ls-files', '--others', '--exclude-standard', '-z', '--'],
      undefined,
      signal,
    )).stdout);
    const allowed = new Set(allowedUntrackedPaths);
    if (untracked.some((path) => !allowed.has(path))) throw new Error(error);
    try {
      await assertRawIndexMatchesWorkingTree({ workspaceRoot: root, signal });
    } catch {
      throw new Error(error);
    }
  }
  async #applyChecked(
    root: string,
    patch: string,
    admittedPaths: readonly string[],
    signal?: AbortSignal,
  ): Promise<void> {
    await this.#git(root, ['apply', '--check', '--index', '--whitespace=error-all', '-'], patch, signal);
    await this.#git(root, ['apply', '--index', '--whitespace=error-all', '-'], patch, signal);
    const changed = parseNullPaths((await this.#git(
      root,
      ['diff', '--cached', '--name-only', '-z', '--'],
      undefined,
      signal,
    )).stdout);
    if (JSON.stringify(changed) !== JSON.stringify([...admittedPaths].sort())) {
      throw new Error('HARNESS_PATCH_ADMISSION_CHANGED');
    }
    await assertRawIndexMatchesWorkingTree({ workspaceRoot: root, signal });
  }
  #mutableRoots(): readonly string[] {
    return [this.#candidateRoot, ...VERIFIER_WORKSPACES.map((stage) => this.#verifierRoots[stage])];
  }
  #overlayRoots(): readonly string[] {
    return [this.#evaluatorRoot, ...this.#mutableRoots()];
  }
  async #identityForCommit(commit: string, signal?: AbortSignal): Promise<GitIdentity> {
    const resolvedCommit = (await this.#git(this.#repositoryRoot, ['rev-parse', '--verify', `${commit}^{commit}`], undefined, signal)).stdout.trim();
    const tree = (await this.#git(this.#repositoryRoot, ['rev-parse', '--verify', `${commit}^{tree}`], undefined, signal)).stdout.trim();
    assertGitObject(resolvedCommit, 'resolved commit');
    assertGitObject(tree, 'resolved tree');
    return Object.freeze({ commit: resolvedCommit, tree });
  }
  async #identityAt(cwd: string, indexTree: boolean, signal?: AbortSignal): Promise<GitIdentity> {
    const commit = (await this.#git(cwd, ['rev-parse', '--verify', 'HEAD'], undefined, signal)).stdout.trim();
    const tree = (await this.#git(cwd, indexTree ? ['write-tree'] : ['rev-parse', '--verify', 'HEAD^{tree}'], undefined, signal)).stdout.trim();
    assertGitObject(commit, 'worktree commit');
    assertGitObject(tree, 'worktree tree');
    return Object.freeze({ commit, tree });
  }
  async #assertCommit(commit: string, signal?: AbortSignal): Promise<void> {
    await this.#git(this.#repositoryRoot, ['cat-file', '-e', `${commit}^{commit}`], undefined, signal);
  }
  async #assertMaterializationSafe(commits: readonly string[], signal?: AbortSignal): Promise<void> {
    await assertGitMaterializationSafe({ repositoryRoot: this.#repositoryRoot, commits, signal });
  }
  async #assertClean(cwd: string, signal?: AbortSignal): Promise<void> {
    const status = await this.#git(cwd, ['status', '--porcelain=v1', '-z', '--untracked-files=all'], undefined, signal);
    if (status.stdout !== '') throw new Error('HARNESS_WORKTREE_NOT_CLEAN');
    await assertRawIndexMatchesWorkingTree({ workspaceRoot: cwd, signal });
  }
  #assertPrepared(): void {
    if (this.#preparation === null || this.#baseline === null || this.#evaluator === null) {
      throw new Error('HARNESS_WORKTREES_NOT_PREPARED');
    }
    this.#assertOwnedRunRoot();
  }
  #assertTrustedParent(): void {
    const current = directoryIdentity(this.#runParent, 'HARNESS_WORKTREE_PARENT_CHANGED', 'trusted');
    if (!sameDirectory(current, this.#parentIdentity)) throw new Error('HARNESS_WORKTREE_PARENT_CHANGED');
  }
  #assertRepositoryRoot(): void {
    const current = directoryIdentity(this.#repositoryRoot, 'HARNESS_REPOSITORY_ROOT_CHANGED');
    if (!sameDirectory(current, this.#repositoryIdentity)) throw new Error('HARNESS_REPOSITORY_ROOT_CHANGED');
  }
  #assertOwnedRunRoot(): void {
    if (this.#runRootIdentity === null) throw new Error('HARNESS_WORKTREE_ROOT_OWNERSHIP_UNPROVEN');
    this.#assertTrustedParent();
    const current = directoryIdentity(this.#runRoot, 'HARNESS_WORKTREE_ROOT_OWNERSHIP_CHANGED', 'private');
    if (!sameDirectory(current, this.#runRootIdentity)) throw new Error('HARNESS_WORKTREE_ROOT_OWNERSHIP_CHANGED');
  }
  #assertControlledPath(path: string): void {
    const child = relative(this.#runRoot, path);
    if (child === '' || child.startsWith(`..${sep}`) || child === '..' || isAbsolute(child)) {
      throw new Error('HARNESS_WORKTREE_PATH_UNCONTROLLED');
    }
  }
  async #registeredWorktreePaths(): Promise<Set<string>> {
    const result = await this.#git(this.#repositoryRoot, ['worktree', 'list', '--porcelain']);
    return new Set(result.stdout.split('\n')
      .filter((line) => line.startsWith('worktree '))
      .map((line) => line.slice('worktree '.length)));
  }
  async #git(cwd: string, args: string[], stdin?: string, signal?: AbortSignal): Promise<GitCommandResult> {
    this.#assertRepositoryRoot();
    const result = await runGitCommand(cwd, args, { stdin, signal });
    if (result.exitCode !== 0) throw new Error(`HARNESS_GIT_COMMAND_FAILED:${args[0]}`);
    return result;
  }
}
function assertActive(signal?: AbortSignal): void {
  if (signal?.aborted === true) throw gitAbortError();
}
function validateFrozenFile(path: string, digest: string, error: string, requireReadOnly = false): void {
  const stat = lstatSync(path);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink > 1
    || realpathSync(path) !== path || fileDigest(path) !== digest
    || (requireReadOnly && (stat.mode & 0o222) !== 0)) throw new Error(error);
}
function fileDigest(path: string): string {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}
function parseNumstat(value: string): string[] {
  const paths: string[] = [];
  for (const record of value.split('\0').filter(Boolean)) {
    const match = /^(?:\d+|-)\t(?:\d+|-)\t(.+)$/.exec(record);
    if (match === null) throw new Error('HARNESS_PATCH_RENAME_OR_NUMSTAT_INVALID');
    paths.push(normalizeWorkspacePath(match[1], 'patch path'));
  }
  return [...new Set(paths)].sort();
}
function parseNullPaths(value: string): string[] {
  return [...new Set(value.split('\0').filter(Boolean).map((path) =>
    normalizeWorkspacePath(path, 'changed path')))].sort();
}
function normalizedAbsolute(value: string, label: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) throw new TypeError(`${label} must be an absolute normalized path`);
  return value;
}
function assertAbsent(path: string): void {
  try {
    lstatSync(path);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return;
    throw error;
  }
  throw new Error('HARNESS_WORKTREE_ROOT_NOT_ABSENT');
}
function directoryIdentity(path: string, error: string, mode?: 'private' | 'trusted'): DirectoryIdentity {
  const stat = lstatSync(path, { bigint: true });
  const uid = process.getuid?.();
  const unsafeMode = mode === 'private' ? (stat.mode & 0o077n) !== 0n
    : mode === 'trusted' ? (stat.mode & 0o022n) !== 0n : false;
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(path) !== path || unsafeMode
    || (mode !== undefined && uid !== undefined && stat.uid !== BigInt(uid))) throw new Error(error);
  return Object.freeze({ dev: stat.dev, ino: stat.ino });
}
function sameDirectory(left: DirectoryIdentity, right: DirectoryIdentity): boolean { return left.dev === right.dev && left.ino === right.ino; }
function pathsOverlap(left: string, right: string): boolean {
  return left === right || left.startsWith(`${right}${sep}`) || right.startsWith(`${left}${sep}`);
}
function assertGitObject(value: string, label: string): void {
  if (!GIT_OBJECT.test(value)) throw new TypeError(`${label} is not a Git object ID`);
}
function sameIdentity(left: GitIdentity, right: GitIdentity): boolean {
  return left.commit === right.commit && left.tree === right.tree;
}
