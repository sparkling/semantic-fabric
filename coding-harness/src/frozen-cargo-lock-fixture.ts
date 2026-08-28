// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { writeFile } from 'node:fs/promises';
import { isAbsolute, join, resolve } from 'node:path';
import { gunzipSync } from 'node:zlib';
import { runGitCommand, runGitCommandBytes } from './git-process.js';
import { ISSUE_8_FROZEN_LOCK_DIGEST } from './issue-8-system.js';
import { normalizeAcceptanceTaskPath } from './manifest.js';

export const ISSUE_8_FROZEN_LOCK_FIXTURE_PATH =
  'coding-harness/config/issue-8-baseline-Cargo.lock.gz.b64';

const GIT_OBJECT = /^[a-f0-9]{40,64}$/;
const SHA256 = /^[a-f0-9]{64}$/;
const MAX_GIT_DIAGNOSTIC_BYTES = 4_096;
const MAX_ENCODED_BYTES = 1_000_000;
const MAX_LOCK_BYTES = 10_000_000;
const HISTORICAL_ISSUE_8_BASELINE = 'd510fc952a8dc701d65b1a4f3ad25a8109b98669';
const LEGACY_FROZEN_LOCK_TASK_DIGESTS = Object.freeze({
  'coding-harness/config/issue-8-acceptance.json':
    '614dfb6920cba1810d5c69e93cc11e00f0d72f8bbd96ba2cdb601af099794a37',
  'coding-harness/config/programme-v5-acceptance.json':
    'ba56aaca00729e3c47723763de9507f6cae97dd76276385f3e0fcd5a3fc2f7c3',
} as const);

export const LEGACY_FROZEN_LOCK_TASK_PATHS = Object.freeze(
  Object.keys(LEGACY_FROZEN_LOCK_TASK_DIGESTS),
);

export async function readTaskFrozenCargoLock(input: Readonly<{
  controllerRepositoryRoot: string;
  controllerCommit: string;
  taskPath: string;
  baselineCommit: string;
  expectedDigest: string;
  signal?: AbortSignal;
}>): Promise<string> {
  const taskPath = normalizeAcceptanceTaskPath(input.taskPath);
  if (!GIT_OBJECT.test(input.controllerCommit) || !GIT_OBJECT.test(input.baselineCommit)
    || !SHA256.test(input.expectedDigest) || input.expectedDigest === '0'.repeat(64)) {
    throw new Error('HARNESS_FROZEN_LOCK_TASK_BINDING_INVALID');
  }
  await assertCommitBinding(
    input.controllerRepositoryRoot,
    input.baselineCommit,
    input.controllerCommit,
    input.signal,
  );
  const object = `${input.baselineCommit}:Cargo.lock`;
  const pathEntry = await runGitCommandBytes(input.controllerRepositoryRoot, [
    'ls-tree', '-z', '--name-only', input.baselineCommit, '--', 'Cargo.lock',
  ], { signal: input.signal, maxOutputBytes: MAX_GIT_DIAGNOSTIC_BYTES });
  const expectedEntry = Buffer.from('Cargo.lock\0');
  if (pathEntry.exitCode !== 0 || pathEntry.stderr !== ''
    || (pathEntry.stdout.length !== 0 && !pathEntry.stdout.equals(expectedEntry))) {
    throw new Error('HARNESS_FROZEN_LOCK_BASELINE_BLOB_INVALID');
  }
  if (pathEntry.stdout.length === 0) {
    await assertLegacyBinding(input.controllerRepositoryRoot, {
      controllerCommit: input.controllerCommit,
      taskPath,
      baselineCommit: input.baselineCommit,
      expectedDigest: input.expectedDigest,
      signal: input.signal,
    });
    return await readIssue8FrozenCargoLock({
      controllerRepositoryRoot: input.controllerRepositoryRoot,
      controllerCommit: input.controllerCommit,
      expectedDigest: input.expectedDigest,
      signal: input.signal,
    });
  }
  const type = await runGitCommand(
    input.controllerRepositoryRoot,
    ['cat-file', '-t', object],
    { signal: input.signal, maxOutputBytes: MAX_GIT_DIAGNOSTIC_BYTES },
  );
  if (type.exitCode !== 0 || type.stderr !== '' || type.stdout.trim() !== 'blob') {
    throw new Error('HARNESS_FROZEN_LOCK_BASELINE_BLOB_INVALID');
  }
  return await readTrackedCargoLock(
    input.controllerRepositoryRoot,
    object,
    input.expectedDigest,
    input.signal,
  );
}

export async function readIssue8FrozenCargoLock(input: Readonly<{
  controllerRepositoryRoot: string;
  controllerCommit: string;
  expectedDigest: string;
  signal?: AbortSignal;
}>): Promise<string> {
  if (!GIT_OBJECT.test(input.controllerCommit)) {
    throw new Error('HARNESS_FROZEN_LOCK_FIXTURE_COMMIT_INVALID');
  }
  const object = `${input.controllerCommit}:${ISSUE_8_FROZEN_LOCK_FIXTURE_PATH}`;
  const type = await runGitCommand(input.controllerRepositoryRoot, ['cat-file', '-t', object], {
    signal: input.signal,
    maxOutputBytes: MAX_GIT_DIAGNOSTIC_BYTES,
  });
  if (type.exitCode !== 0 || type.stdout.trim() !== 'blob') {
    throw new Error('HARNESS_FROZEN_LOCK_FIXTURE_BLOB_INVALID');
  }
  const sizeResult = await runGitCommand(
    input.controllerRepositoryRoot,
    ['cat-file', '-s', object],
    { signal: input.signal, maxOutputBytes: MAX_GIT_DIAGNOSTIC_BYTES },
  );
  const size = Number(sizeResult.stdout.trim());
  if (sizeResult.exitCode !== 0 || sizeResult.stderr !== '' || !Number.isSafeInteger(size)
    || size < 2 || size > MAX_ENCODED_BYTES) {
    throw new Error('HARNESS_FROZEN_LOCK_FIXTURE_SIZE_INVALID');
  }
  const fixture = await runGitCommand(input.controllerRepositoryRoot, ['show', object], {
    signal: input.signal,
    maxOutputBytes: MAX_ENCODED_BYTES + 1,
  });
  if (fixture.exitCode !== 0 || Buffer.byteLength(fixture.stdout, 'utf8') !== size) {
    throw new Error('HARNESS_FROZEN_LOCK_FIXTURE_READ_FAILED');
  }
  return decodeFrozenCargoLockFixture(fixture.stdout, input.expectedDigest);
}

async function readTrackedCargoLock(
  repositoryRoot: string,
  object: string,
  expectedDigest: string,
  signal?: AbortSignal,
): Promise<string> {
  const sizeResult = await runGitCommand(repositoryRoot, ['cat-file', '-s', object], {
    signal,
    maxOutputBytes: MAX_GIT_DIAGNOSTIC_BYTES,
  });
  const size = Number(sizeResult.stdout.trim());
  if (sizeResult.exitCode !== 0 || !Number.isSafeInteger(size)
    || size < 1 || size > MAX_LOCK_BYTES) {
    throw new Error('HARNESS_FROZEN_LOCK_BASELINE_SIZE_INVALID');
  }
  const result = await runGitCommandBytes(repositoryRoot, ['cat-file', 'blob', object], {
    signal,
    maxOutputBytes: size + 1,
  });
  if (result.exitCode !== 0 || result.stderr !== '' || result.stdout.length !== size
    || createHash('sha256').update(result.stdout).digest('hex') !== expectedDigest) {
    throw new Error('HARNESS_FROZEN_LOCK_BASELINE_DIGEST_MISMATCH');
  }
  let lockfile: string;
  try {
    lockfile = new TextDecoder('utf-8', { fatal: true }).decode(result.stdout);
  } catch {
    throw new Error('HARNESS_FROZEN_LOCK_BASELINE_UTF8_INVALID');
  }
  if (lockfile.includes('\0') || !Buffer.from(lockfile, 'utf8').equals(result.stdout)) {
    throw new Error('HARNESS_FROZEN_LOCK_BASELINE_UTF8_INVALID');
  }
  return lockfile;
}

async function assertCommitBinding(
  repositoryRoot: string,
  baselineCommit: string,
  controllerCommit: string,
  signal?: AbortSignal,
): Promise<void> {
  for (const commit of [baselineCommit, controllerCommit]) {
    const resolved = await runGitCommand(
      repositoryRoot,
      ['rev-parse', '--verify', `${commit}^{commit}`],
      { signal, maxOutputBytes: MAX_GIT_DIAGNOSTIC_BYTES },
    );
    if (resolved.exitCode !== 0 || resolved.stderr !== '' || resolved.stdout.trim() !== commit) {
      throw new Error('HARNESS_FROZEN_LOCK_TASK_BINDING_INVALID');
    }
  }
  const ancestry = await runGitCommand(
    repositoryRoot,
    ['merge-base', '--is-ancestor', baselineCommit, controllerCommit],
    { signal, maxOutputBytes: MAX_GIT_DIAGNOSTIC_BYTES },
  );
  if (ancestry.exitCode !== 0 || ancestry.stderr !== '' || ancestry.stdout !== '') {
    throw new Error('HARNESS_FROZEN_LOCK_TASK_BINDING_INVALID');
  }
}

async function assertLegacyBinding(
  repositoryRoot: string,
  input: Readonly<{
    controllerCommit: string;
    taskPath: string;
    baselineCommit: string;
    expectedDigest: string;
    signal?: AbortSignal;
  }>,
): Promise<void> {
  const taskDigest = LEGACY_FROZEN_LOCK_TASK_DIGESTS[
    input.taskPath as keyof typeof LEGACY_FROZEN_LOCK_TASK_DIGESTS
  ];
  if (input.baselineCommit !== HISTORICAL_ISSUE_8_BASELINE
    || input.expectedDigest !== ISSUE_8_FROZEN_LOCK_DIGEST
    || taskDigest === undefined) {
    throw new Error('HARNESS_FROZEN_LOCK_BASELINE_BLOB_INVALID');
  }
  const object = `${input.controllerCommit}:${input.taskPath}`;
  const type = await runGitCommand(repositoryRoot, ['cat-file', '-t', object], {
    signal: input.signal,
    maxOutputBytes: MAX_GIT_DIAGNOSTIC_BYTES,
  });
  const sizeResult = await runGitCommand(repositoryRoot, ['cat-file', '-s', object], {
    signal: input.signal,
    maxOutputBytes: MAX_GIT_DIAGNOSTIC_BYTES,
  });
  const size = Number(sizeResult.stdout.trim());
  if (type.exitCode !== 0 || type.stderr !== '' || type.stdout.trim() !== 'blob'
    || sizeResult.exitCode !== 0 || sizeResult.stderr !== ''
    || !Number.isSafeInteger(size) || size < 1 || size > MAX_ENCODED_BYTES) {
    throw new Error('HARNESS_FROZEN_LOCK_LEGACY_TASK_BLOB_INVALID');
  }
  const task = await runGitCommandBytes(repositoryRoot, ['cat-file', 'blob', object], {
    signal: input.signal,
    maxOutputBytes: size + 1,
  });
  if (task.exitCode !== 0 || task.stderr !== '' || task.stdout.length !== size
    || createHash('sha256').update(task.stdout).digest('hex') !== taskDigest) {
    throw new Error('HARNESS_FROZEN_LOCK_LEGACY_TASK_BLOB_INVALID');
  }
}

export function decodeFrozenCargoLockFixture(
  serialized: string,
  expectedDigest: string,
): string {
  if (!SHA256.test(expectedDigest) || expectedDigest === '0'.repeat(64)
    || !/^[A-Za-z0-9+/]+={0,2}\n$/.test(serialized)) {
    throw new Error('HARNESS_FROZEN_LOCK_FIXTURE_INVALID');
  }
  const payload = serialized.slice(0, -1);
  const compressed = Buffer.from(payload, 'base64');
  if (compressed.toString('base64') !== payload) {
    throw new Error('HARNESS_FROZEN_LOCK_FIXTURE_INVALID');
  }
  let lockfile: Buffer;
  try {
    lockfile = gunzipSync(compressed, { maxOutputLength: MAX_LOCK_BYTES });
  } catch {
    throw new Error('HARNESS_FROZEN_LOCK_FIXTURE_INVALID');
  }
  if (lockfile.length < 1 || lockfile.length > MAX_LOCK_BYTES
    || createHash('sha256').update(lockfile).digest('hex') !== expectedDigest) {
    throw new Error('HARNESS_FROZEN_LOCK_FIXTURE_DIGEST_MISMATCH');
  }
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(lockfile);
  } catch {
    throw new Error('HARNESS_FROZEN_LOCK_FIXTURE_INVALID');
  }
}

export async function installPinnedCargoLock(
  workspaceRoot: string,
  contents: string,
  expectedDigest: string,
): Promise<void> {
  if (typeof contents !== 'string') {
    throw new Error('HARNESS_FROZEN_LOCK_FIXTURE_INVALID');
  }
  const bytes = Buffer.from(contents, 'utf8');
  if (!isAbsolute(workspaceRoot) || resolve(workspaceRoot) !== workspaceRoot
    || workspaceRoot.includes('\0') || !SHA256.test(expectedDigest)
    || expectedDigest === '0'.repeat(64) || bytes.length < 1 || bytes.length > MAX_LOCK_BYTES
    || createHash('sha256').update(bytes).digest('hex') !== expectedDigest) {
    throw new Error('HARNESS_FROZEN_LOCK_FIXTURE_DIGEST_MISMATCH');
  }
  await writeFile(join(workspaceRoot, 'Cargo.lock'), bytes, { flag: 'wx', mode: 0o400 });
}
