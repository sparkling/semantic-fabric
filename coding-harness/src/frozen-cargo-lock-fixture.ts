// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { writeFile } from 'node:fs/promises';
import { isAbsolute, join, resolve } from 'node:path';
import { gunzipSync } from 'node:zlib';
import { runGitCommand } from './git-process.js';

export const ISSUE_8_FROZEN_LOCK_FIXTURE_PATH =
  'coding-harness/config/issue-8-baseline-Cargo.lock.gz.b64';

const GIT_OBJECT = /^[a-f0-9]{40,64}$/;
const SHA256 = /^[a-f0-9]{64}$/;
const MAX_GIT_DIAGNOSTIC_BYTES = 4_096;
const MAX_ENCODED_BYTES = 1_000_000;
const MAX_LOCK_BYTES = 10_000_000;

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
  if (sizeResult.exitCode !== 0 || !Number.isSafeInteger(size)
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
