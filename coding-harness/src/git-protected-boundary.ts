// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { lstatSync, realpathSync } from 'node:fs';
import { isAbsolute, resolve } from 'node:path';
import type { HarnessConfig, TaskContract } from './contracts.js';
import { deepFreeze, normalizeWorkspacePath } from './contracts.js';
import { runGitCommand } from './git-process.js';
import {
  assertProtectedInputSnapshot,
  type GateDecision,
  type ProtectedInputBoundary,
} from './policy.js';
import { resolveWorkspacePath, sha256File } from './workspace.js';

const GIT_OBJECT = /^[a-f0-9]{40,64}$/;
const MAX_PROTECTED_BLOB_BYTES = 10_000_000;

export interface GitProtectedInputBoundaryOptions {
  readonly repositoryRoot: string;
  readonly controllerCommit: string;
  readonly evaluatorRoot: string;
  readonly evaluatorPaths: readonly string[];
}

export class GitProtectedInputBoundary implements ProtectedInputBoundary {
  readonly #repositoryRoot: string;
  readonly #repositoryIdentity: Readonly<{ dev: bigint; ino: bigint }>;
  readonly #controllerCommit: string;
  readonly #evaluatorRoot: string;
  readonly #evaluatorPaths: ReadonlySet<string>;

  constructor(options: GitProtectedInputBoundaryOptions) {
    this.#repositoryRoot = canonicalDirectory(options.repositoryRoot, 'REPOSITORY');
    const repositoryStat = lstatSync(this.#repositoryRoot, { bigint: true });
    this.#repositoryIdentity = Object.freeze({
      dev: repositoryStat.dev,
      ino: repositoryStat.ino,
    });
    if (!GIT_OBJECT.test(options.controllerCommit)) {
      throw new TypeError('HARNESS_PROTECTED_CONTROLLER_COMMIT_INVALID');
    }
    this.#controllerCommit = options.controllerCommit;
    this.#evaluatorRoot = canonicalDirectory(options.evaluatorRoot, 'EVALUATOR');
    const evaluatorPaths = options.evaluatorPaths.map((path, index) =>
      normalizeWorkspacePath(path, `evaluatorPaths[${index}]`));
    if (new Set(evaluatorPaths).size !== evaluatorPaths.length) {
      throw new TypeError('HARNESS_PROTECTED_EVALUATOR_PATH_DUPLICATE');
    }
    this.#evaluatorPaths = new Set(evaluatorPaths);
  }

  async capture(
    task: TaskContract,
    _config: HarnessConfig,
  ): Promise<Readonly<Record<string, string>>> {
    await this.#assertRepository();
    const snapshot: Record<string, string> = {};
    for (const path of task.protectedPaths) {
      snapshot[path] = this.#evaluatorPaths.has(path)
        ? evaluatorDigest(this.#evaluatorRoot, path)
        : await this.#controllerBlobDigest(path);
    }
    assertProtectedInputSnapshot(task, snapshot);
    await this.#assertRepository();
    return deepFreeze(snapshot);
  }

  async verify(
    task: TaskContract,
    config: HarnessConfig,
    expected: Readonly<Record<string, string>>,
  ): Promise<GateDecision> {
    try {
      assertProtectedInputSnapshot(task, expected);
      const current = await this.capture(task, config);
      const changed = task.protectedPaths.filter((path) => current[path] !== expected[path]);
      return deepFreeze(changed.length === 0
        ? { allow: true, reasons: ['protected controller blobs and evaluator files match'] }
        : { allow: false, reasons: [`protected inputs changed: ${changed.join(', ')}`] });
    } catch (error) {
      return deepFreeze({
        allow: false,
        reasons: [error instanceof Error ? error.message : String(error)],
      });
    }
  }

  async #controllerBlobDigest(path: string): Promise<string> {
    const object = `${this.#controllerCommit}:${path}`;
    const type = await gitChecked(
      this.#repositoryRoot,
      ['cat-file', '-t', object],
      128,
    );
    if (type.trim() !== 'blob') throw new Error(`HARNESS_PROTECTED_CONTROLLER_BLOB_INVALID:${path}`);
    const sizeText = await gitChecked(
      this.#repositoryRoot,
      ['cat-file', '-s', object],
      128,
    );
    const size = Number(sizeText.trim());
    if (!Number.isSafeInteger(size) || size < 0 || size > MAX_PROTECTED_BLOB_BYTES) {
      throw new Error(`HARNESS_PROTECTED_CONTROLLER_BLOB_SIZE_INVALID:${path}`);
    }
    const value = await gitChecked(
      this.#repositoryRoot,
      ['show', object],
      MAX_PROTECTED_BLOB_BYTES + 1,
    );
    if (Buffer.byteLength(value, 'utf8') !== size) {
      throw new Error(`HARNESS_PROTECTED_CONTROLLER_BLOB_CHANGED:${path}`);
    }
    return sha256(value);
  }

  async #assertRepository(): Promise<void> {
    const current = canonicalDirectory(this.#repositoryRoot, 'REPOSITORY');
    const stat = lstatSync(current, { bigint: true });
    if (stat.dev !== this.#repositoryIdentity.dev || stat.ino !== this.#repositoryIdentity.ino) {
      throw new Error('HARNESS_PROTECTED_REPOSITORY_CHANGED');
    }
    const commit = (await gitChecked(
      current,
      ['rev-parse', '--verify', `${this.#controllerCommit}^{commit}`],
      128,
    )).trim();
    if (commit !== this.#controllerCommit) throw new Error('HARNESS_PROTECTED_CONTROLLER_COMMIT_CHANGED');
  }
}

async function gitChecked(
  cwd: string,
  args: readonly string[],
  maxOutputBytes: number,
): Promise<string> {
  const result = await runGitCommand(cwd, args, { maxOutputBytes });
  if (result.exitCode !== 0) {
    throw new Error(`HARNESS_PROTECTED_GIT_FAILED:${args[0] ?? 'unknown'}`);
  }
  return result.stdout;
}

function evaluatorDigest(root: string, path: string): string {
  const absolute = resolveWorkspacePath(root, path, {
    requireRegularFile: true,
    rejectHardlinks: true,
  });
  return sha256File(absolute);
}

function canonicalDirectory(value: string, label: string): string {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) {
    throw new TypeError(`HARNESS_PROTECTED_${label}_ROOT_INVALID`);
  }
  const stat = lstatSync(value);
  if (!stat.isDirectory() || stat.isSymbolicLink() || realpathSync(value) !== value) {
    throw new Error(`HARNESS_PROTECTED_${label}_ROOT_INVALID`);
  }
  return value;
}

function sha256(value: string): string {
  return createHash('sha256').update(value, 'utf8').digest('hex');
}
