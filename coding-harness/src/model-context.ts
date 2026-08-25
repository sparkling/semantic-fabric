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
import { deepFreeze, normalizeWorkspacePath, pathsOverlap } from './contracts.js';
import { runGitCommand, type GitCommandResult } from './git-process.js';
import { resolveWorkspacePath } from './workspace.js';

const GIT_OBJECT = /^[a-f0-9]{40,64}$/;
const MAX_MODEL_CONTEXT_BYTES = 1_000_000;
const MAX_GIT_METADATA_BYTES = 64_000;
const MAX_MODEL_CONTEXT_PATHS = 128;
const MAX_MODEL_CONTEXT_PATH_BYTES = 16_384;

export interface ModelSourceFile {
  readonly path: string;
  readonly digest: string;
  readonly content: string;
}

export interface DeclaredImplementationContext {
  readonly schemaVersion: 1;
  readonly kind: 'declared-implementation-source';
  readonly headCommit: string;
  readonly indexTree: string;
  readonly files: readonly ModelSourceFile[];
  readonly digest: string;
}

export interface AdmittedImplementationContext {
  readonly schemaVersion: 1;
  readonly kind: 'admitted-implementation';
  readonly headCommit: string;
  readonly indexTree: string;
  readonly files: readonly ModelSourceFile[];
  readonly stagedPaths: readonly string[];
  readonly stagedDiff: string;
  readonly stagedDiffDigest: string;
  readonly digest: string;
}

export interface ModelContextProvider {
  declaredSource(signal?: AbortSignal): Promise<DeclaredImplementationContext>;
  admittedSource(signal?: AbortSignal): Promise<AdmittedImplementationContext>;
}

export interface RepositoryModelContextProviderOptions {
  readonly candidateRoot: string;
  readonly implementationPaths: readonly string[];
  readonly evaluatorPaths: readonly string[];
  readonly maxTotalBytes: number;
}

type RootBinding = Readonly<{ dev: bigint; ino: bigint; headCommit: string }>;
type FileIdentity = Readonly<{
  dev: bigint;
  ino: bigint;
  mode: bigint;
  size: bigint;
  mtimeNs: bigint;
  ctimeNs: bigint;
}>;
type CapturedFile = Readonly<{
  modelFile: ModelSourceFile;
  absolutePath: string;
  identity: FileIdentity;
}>;

export class RepositoryModelContextProvider implements ModelContextProvider {
  readonly #root: string;
  readonly #implementationPaths: readonly string[];
  readonly #evaluatorPaths: readonly string[];
  readonly #maxTotalBytes: number;
  #binding: RootBinding | null = null;

  constructor(options: RepositoryModelContextProviderOptions) {
    if (!isAbsolute(options.candidateRoot)
      || resolve(options.candidateRoot) !== options.candidateRoot
      || options.candidateRoot.includes('\0')) {
      throw new TypeError('HARNESS_MODEL_CONTEXT_ROOT_INVALID');
    }
    this.#root = options.candidateRoot;
    this.#implementationPaths = normalizePaths(
      options.implementationPaths,
      'implementationPaths',
      false,
    );
    this.#evaluatorPaths = normalizePaths(options.evaluatorPaths, 'evaluatorPaths', true);
    if (this.#implementationPaths.some((implementationPath) =>
      this.#evaluatorPaths.some((evaluatorPath) => pathsOverlap(implementationPath, evaluatorPath)))) {
      throw new Error('HARNESS_MODEL_CONTEXT_EVALUATOR_OVERLAP');
    }
    if (!Number.isSafeInteger(options.maxTotalBytes)
      || options.maxTotalBytes < 1
      || options.maxTotalBytes > MAX_MODEL_CONTEXT_BYTES) {
      throw new TypeError('HARNESS_MODEL_CONTEXT_BYTE_LIMIT_INVALID');
    }
    this.#maxTotalBytes = options.maxTotalBytes;
  }

  async declaredSource(signal?: AbortSignal): Promise<DeclaredImplementationContext> {
    const captured = await this.#capture(false, signal);
    const unsigned = {
      schemaVersion: 1 as const,
      kind: 'declared-implementation-source' as const,
      headCommit: captured.headCommit,
      indexTree: captured.indexTree,
      files: captured.files,
    };
    return deepFreeze({ ...unsigned, digest: digest(unsigned) });
  }

  async admittedSource(signal?: AbortSignal): Promise<AdmittedImplementationContext> {
    const captured = await this.#capture(true, signal);
    const unsigned = {
      schemaVersion: 1 as const,
      kind: 'admitted-implementation' as const,
      headCommit: captured.headCommit,
      indexTree: captured.indexTree,
      files: captured.files,
      stagedPaths: captured.stagedPaths,
      stagedDiff: captured.stagedDiff,
      stagedDiffDigest: digest(captured.stagedDiff),
    };
    return deepFreeze({ ...unsigned, digest: digest(unsigned) });
  }

  async #capture(admitted: boolean, signal?: AbortSignal): Promise<Readonly<{
    headCommit: string;
    indexTree: string;
    files: readonly ModelSourceFile[];
    stagedPaths: readonly string[];
    stagedDiff: string;
  }>> {
    const binding = await this.#assertRootBinding(signal);
    const stagedPaths = await this.#stagedPaths(signal);
    if (!admitted && stagedPaths.length > 0) {
      throw new Error('HARNESS_MODEL_CONTEXT_EXPECTED_CLEAN_SOURCE');
    }
    if (admitted && stagedPaths.length === 0) {
      throw new Error('HARNESS_MODEL_CONTEXT_STAGED_DIFF_REQUIRED');
    }
    const indexTree = await this.#indexTree(signal);
    await this.#assertNoUnstagedImplementationChanges(signal);

    let sourceBytes = 0;
    const capturedFiles = this.#implementationPaths.map((path) => {
      const captured = captureFile(this.#root, path, this.#maxTotalBytes - sourceBytes);
      sourceBytes += Buffer.byteLength(captured.modelFile.content, 'utf8');
      return captured;
    });
    const stagedDiff = admitted ? await this.#stagedDiff(signal) : '';
    if (sourceBytes + Buffer.byteLength(stagedDiff, 'utf8') > this.#maxTotalBytes) {
      throw new Error('HARNESS_MODEL_CONTEXT_BYTE_LIMIT_EXCEEDED');
    }

    const finalStagedPaths = await this.#stagedPaths(signal);
    const finalIndexTree = await this.#indexTree(signal);
    await this.#assertNoUnstagedImplementationChanges(signal);
    for (const file of capturedFiles) assertFileUnchanged(file);
    const finalBinding = await this.#assertRootBinding(signal);
    if (JSON.stringify(stagedPaths) !== JSON.stringify(finalStagedPaths)
      || indexTree !== finalIndexTree
      || binding.headCommit !== finalBinding.headCommit) {
      throw new Error('HARNESS_MODEL_CONTEXT_CHANGED_DURING_CAPTURE');
    }
    return deepFreeze({
      headCommit: binding.headCommit,
      indexTree,
      files: capturedFiles.map(({ modelFile }) => modelFile),
      stagedPaths,
      stagedDiff,
    });
  }

  async #assertRootBinding(signal?: AbortSignal): Promise<RootBinding> {
    const stat = lstatSync(this.#root, { bigint: true });
    if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(this.#root) !== this.#root) {
      throw new Error('HARNESS_MODEL_CONTEXT_ROOT_UNTRUSTED');
    }
    const topLevel = (await this.#gitChecked(['rev-parse', '--show-toplevel'], signal)).trim();
    if (topLevel !== this.#root) throw new Error('HARNESS_MODEL_CONTEXT_ROOT_NOT_WORKTREE');
    const headCommit = (await this.#gitChecked(['rev-parse', '--verify', 'HEAD'], signal)).trim();
    if (!GIT_OBJECT.test(headCommit)) throw new Error('HARNESS_MODEL_CONTEXT_HEAD_INVALID');
    const current = Object.freeze({ dev: stat.dev, ino: stat.ino, headCommit });
    if (this.#binding === null) this.#binding = current;
    else if (this.#binding.dev !== current.dev
      || this.#binding.ino !== current.ino
      || this.#binding.headCommit !== current.headCommit) {
      throw new Error('HARNESS_MODEL_CONTEXT_ROOT_CHANGED');
    }
    return this.#binding;
  }

  async #stagedPaths(signal?: AbortSignal): Promise<readonly string[]> {
    const output = await this.#gitChecked(
      ['diff', '--cached', '--name-only', '-z', '--'],
      signal,
    );
    const paths = output.split('\0').filter(Boolean)
      .map((path, index) => normalizeWorkspacePath(path, `stagedPaths[${index}]`))
      .sort();
    if (new Set(paths).size !== paths.length) throw new Error('HARNESS_MODEL_CONTEXT_STAGED_PATH_DUPLICATE');
    for (const path of paths) {
      if (this.#evaluatorPaths.some((evaluatorPath) => pathsOverlap(path, evaluatorPath))) {
        throw new Error(`HARNESS_MODEL_CONTEXT_EVALUATOR_STAGED:${path}`);
      }
      if (!this.#implementationPaths.includes(path)) {
        throw new Error(`HARNESS_MODEL_CONTEXT_STAGED_PATH_FORBIDDEN:${path}`);
      }
    }
    return Object.freeze(paths);
  }

  async #indexTree(signal?: AbortSignal): Promise<string> {
    const tree = (await this.#gitChecked(['write-tree'], signal)).trim();
    if (!GIT_OBJECT.test(tree)) throw new Error('HARNESS_MODEL_CONTEXT_INDEX_TREE_INVALID');
    return tree;
  }

  async #assertNoUnstagedImplementationChanges(signal?: AbortSignal): Promise<void> {
    const result = await runGitCommand(
      this.#root,
      ['diff', '--quiet', '--', ...this.#literalImplementationPathspecs()],
      { signal, maxOutputBytes: MAX_GIT_METADATA_BYTES },
    );
    if (result.exitCode === 1) throw new Error('HARNESS_MODEL_CONTEXT_UNSTAGED_SOURCE');
    assertGitSuccess(result, 'diff');
  }

  async #stagedDiff(signal?: AbortSignal): Promise<string> {
    let diff: string;
    try {
      diff = await this.#gitChecked([
        'diff', '--cached', '--binary', '--full-index', '--no-renames', '--',
        ...this.#literalImplementationPathspecs(),
      ], signal, this.#maxTotalBytes);
    } catch (error) {
      if (error instanceof Error && error.message === 'HARNESS_GIT_OUTPUT_LIMIT_EXCEEDED') {
        throw new Error('HARNESS_MODEL_CONTEXT_BYTE_LIMIT_EXCEEDED');
      }
      throw error;
    }
    if (diff.length === 0) throw new Error('HARNESS_MODEL_CONTEXT_STAGED_DIFF_REQUIRED');
    return diff;
  }

  #literalImplementationPathspecs(): string[] {
    return this.#implementationPaths.map((path) => `:(literal)${path}`);
  }

  async #gitChecked(
    args: readonly string[],
    signal?: AbortSignal,
    maxOutputBytes = MAX_GIT_METADATA_BYTES,
  ): Promise<string> {
    const result = await runGitCommand(this.#root, args, {
      signal,
      maxOutputBytes,
    });
    assertGitSuccess(result, args[0] ?? 'unknown');
    return result.stdout;
  }
}

function normalizePaths(
  values: readonly string[],
  label: string,
  allowEmpty: boolean,
): readonly string[] {
  if (!Array.isArray(values) || (!allowEmpty && values.length === 0)) {
    throw new TypeError(`${label} must be ${allowEmpty ? 'an' : 'a non-empty'} array`);
  }
  if (values.length > MAX_MODEL_CONTEXT_PATHS) {
    throw new TypeError(`${label} exceeds the path-count limit`);
  }
  const paths = values.map((path, index) => normalizeWorkspacePath(path, `${label}[${index}]`));
  if (paths.reduce((bytes, path) => bytes + Buffer.byteLength(path, 'utf8'), 0)
    > MAX_MODEL_CONTEXT_PATH_BYTES) {
    throw new TypeError(`${label} exceeds the path-byte limit`);
  }
  if (new Set(paths).size !== paths.length) throw new TypeError(`${label} must not contain duplicates`);
  for (const [index, path] of paths.entries()) {
    if (paths.some((other, otherIndex) => index !== otherIndex && pathsOverlap(path, other))) {
      throw new TypeError(`${label} must contain exact non-overlapping paths`);
    }
  }
  return Object.freeze([...paths].sort());
}

function captureFile(root: string, path: string, remainingBytes: number): CapturedFile {
  if (remainingBytes < 0) throw new Error('HARNESS_MODEL_CONTEXT_BYTE_LIMIT_EXCEEDED');
  const absolutePath = resolveWorkspacePath(root, path, {
    requireRegularFile: true,
    rejectHardlinks: true,
  });
  const descriptor = openSync(absolutePath, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const before = fileIdentity(descriptor);
    if (before.size > BigInt(remainingBytes)) {
      throw new Error('HARNESS_MODEL_CONTEXT_BYTE_LIMIT_EXCEEDED');
    }
    const bytes = readBoundedFile(descriptor, before.size, path);
    const after = fileIdentity(descriptor);
    if (!sameFileIdentity(before, after) || BigInt(bytes.length) !== before.size) {
      throw new Error(`HARNESS_MODEL_CONTEXT_FILE_CHANGED:${path}`);
    }
    const content = decodeSource(bytes, path);
    const modelFile = deepFreeze({ path, digest: digest(bytes), content });
    return Object.freeze({ modelFile, absolutePath, identity: before });
  } finally {
    closeSync(descriptor);
  }
}

function assertFileUnchanged(captured: CapturedFile): void {
  const descriptor = openSync(
    captured.absolutePath,
    constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0),
  );
  try {
    const before = fileIdentity(descriptor);
    const bytes = readBoundedFile(descriptor, before.size, captured.modelFile.path);
    const after = fileIdentity(descriptor);
    if (!sameFileIdentity(captured.identity, before)
      || !sameFileIdentity(before, after)
      || digest(bytes) !== captured.modelFile.digest) {
      throw new Error(`HARNESS_MODEL_CONTEXT_FILE_CHANGED:${captured.modelFile.path}`);
    }
  } finally {
    closeSync(descriptor);
  }
  const pathStat = lstatSync(captured.absolutePath, { bigint: true });
  if (pathStat.isSymbolicLink()
    || !pathStat.isFile()
    || pathStat.nlink !== 1n
    || realpathSync(captured.absolutePath) !== captured.absolutePath
    || pathStat.dev !== captured.identity.dev
    || pathStat.ino !== captured.identity.ino) {
    throw new Error(`HARNESS_MODEL_CONTEXT_FILE_CHANGED:${captured.modelFile.path}`);
  }
}

function fileIdentity(descriptor: number): FileIdentity {
  const stat = fstatSync(descriptor, { bigint: true });
  if (!stat.isFile() || stat.nlink !== 1n) throw new Error('HARNESS_MODEL_CONTEXT_FILE_UNTRUSTED');
  return Object.freeze({
    dev: stat.dev,
    ino: stat.ino,
    mode: stat.mode,
    size: stat.size,
    mtimeNs: stat.mtimeNs,
    ctimeNs: stat.ctimeNs,
  });
}

function readBoundedFile(descriptor: number, expectedSize: bigint, path: string): Buffer {
  if (expectedSize > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new Error('HARNESS_MODEL_CONTEXT_BYTE_LIMIT_EXCEEDED');
  }
  const size = Number(expectedSize);
  const target = Buffer.allocUnsafe(size + 1);
  let offset = 0;
  while (offset < target.length) {
    const bytesRead = readSync(descriptor, target, offset, target.length - offset, offset);
    if (bytesRead === 0) break;
    offset += bytesRead;
  }
  if (offset !== size) throw new Error(`HARNESS_MODEL_CONTEXT_FILE_CHANGED:${path}`);
  return target.subarray(0, size);
}

function sameFileIdentity(left: FileIdentity, right: FileIdentity): boolean {
  return left.dev === right.dev
    && left.ino === right.ino
    && left.mode === right.mode
    && left.size === right.size
    && left.mtimeNs === right.mtimeNs
    && left.ctimeNs === right.ctimeNs;
}

function decodeSource(bytes: Buffer, path: string): string {
  let content: string;
  try {
    content = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch {
    throw new Error(`HARNESS_MODEL_CONTEXT_SOURCE_NOT_UTF8:${path}`);
  }
  if (content.includes('\0')) throw new Error(`HARNESS_MODEL_CONTEXT_SOURCE_BINARY:${path}`);
  return content;
}

function assertGitSuccess(result: GitCommandResult, command: string): void {
  if (result.exitCode !== 0) throw new Error(`HARNESS_MODEL_CONTEXT_GIT_FAILED:${command}`);
}

function digest(value: unknown): string {
  const bytes = Buffer.isBuffer(value) ? value : Buffer.from(JSON.stringify(value), 'utf8');
  return createHash('sha256').update(bytes).digest('hex');
}
