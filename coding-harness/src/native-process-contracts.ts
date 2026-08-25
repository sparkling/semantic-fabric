// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { lstatSync, readFileSync, realpathSync, statSync } from 'node:fs';
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path';
import type {
  NativeHost,
  NativeProcessRequest,
  NativeProcessResult,
} from './models/types.js';

export interface NativeExecutableEvidence {
  readonly path: string;
  readonly digest: string;
}

export interface OriginCompletion {
  readonly allowedConnections: number;
  readonly deniedConnections: number;
  readonly connectDigest: string;
}

export interface NativeExecutionEvidence {
  readonly executionId: string;
  readonly host: NativeHost;
  readonly purpose: NativeProcessRequest['purpose'];
  readonly model: string;
  readonly operation: NativeProcessRequest['operation'] | null;
  readonly executable: NativeExecutableEvidence;
  readonly environmentDigest: string;
  readonly exitCode: number | null;
  readonly stdoutDigest: string;
  readonly stderrDigest: string;
  readonly network: Readonly<{
    enforcement: 'origin-pinned-process-boundary'; mechanism: string;
    pinnedOrigins: readonly string[]; allowedConnections: number;
    deniedConnections: number; connectDigest: string;
  }>;
  readonly filesystem: Readonly<{
    enforcement: 'os-filesystem-namespace'; mechanism: string;
    workspaceRootDigest: string; mountManifestDigest: string;
    configurationMaskDigest: string; hostFileConfidentiality: true;
    emptyPrivateHome: true; privateEphemeralHome: true; hostRootMounted: false;
    hostCredentialPathMounted: false; gitMetadataMasked: boolean;
  }>;
  readonly resources: Readonly<{
    enforcement: 'systemd-cgroup-v2'; mechanism: 'systemd-transient-service';
    limitsDigest: string;
  }>;
}

export function validateExecutable(
  value: string,
  host: NativeHost,
): NativeExecutableEvidence {
  if (!isAbsolute(value) || resolve(value) !== value || value.includes('\0')) {
    throw new TypeError(`HARNESS_NATIVE_EXECUTABLE_INVALID:${host}`);
  }
  const stat = lstatSync(value);
  const currentUid = typeof process.getuid === 'function' ? process.getuid() : stat.uid;
  if (!stat.isFile() || stat.isSymbolicLink() || realpathSync(value) !== value
    || stat.nlink !== 1 || (stat.mode & 0o111) === 0 || (stat.mode & 0o022) !== 0
    || (stat.uid !== 0 && stat.uid !== currentUid)) {
    throw new Error(`HARNESS_NATIVE_EXECUTABLE_UNTRUSTED:${host}`);
  }
  return Object.freeze({
    path: value,
    digest: createHash('sha256').update(readFileSync(value)).digest('hex'),
  });
}

export function validateBoundaryPaths(
  paths: readonly string[],
  label: string,
  allowedRoots: readonly string[],
  forbiddenRoots: readonly string[],
  writable: boolean,
): void {
  if (new Set(paths).size !== paths.length) {
    throw new Error(`HARNESS_NATIVE_${label}_PATH_DUPLICATE`);
  }
  for (const path of paths) {
    if (!isAbsolute(path) || resolve(path) !== path || path.includes('\0')) {
      throw new Error(`HARNESS_NATIVE_${label}_PATH_INVALID`);
    }
    assertCapabilityPath(path, allowedRoots, forbiddenRoots, writable);
  }
}

export function assertCapabilityPath(
  path: string,
  allowedRoots: readonly string[],
  forbiddenRoots: readonly string[],
  writable: boolean,
): void {
  const stat = lstatSync(path);
  if (stat.isSymbolicLink() || realpathSync(path) !== path
    || (stat.isFile() && stat.nlink !== 1)
    || (!stat.isFile() && !stat.isDirectory())) {
    throw new Error('HARNESS_NATIVE_CAPABILITY_PATH_UNTRUSTED');
  }
  const parent = realpathSync(dirname(path));
  if (!allowedRoots.some((root) => path === root
      || (contains(root, path) && contains(root, parent)))
    || forbiddenRoots.some((root) => contains(root, path) || contains(path, root))) {
    throw new Error('HARNESS_NATIVE_CAPABILITY_PATH_OUTSIDE_SCOPE');
  }
  if (writable && stat.isDirectory()) {
    throw new Error('HARNESS_NATIVE_WRITABLE_PATH_MUST_BE_FILE');
  }
}

export function pathsOverlap(left: string, right: string): boolean {
  return contains(left, right) || contains(right, left);
}

export function validateDirectory(value: string, allowedRoots: readonly string[]): string {
  if (!isAbsolute(value) || resolve(value) !== value
    || realpathSync(value) !== value || !statSync(value).isDirectory()) {
    throw new TypeError('HARNESS_NATIVE_CWD_INVALID');
  }
  if (allowedRoots.length > 0 && !allowedRoots.some((root) => contains(root, value))) {
    throw new Error('HARNESS_NATIVE_CWD_OUTSIDE_ALLOWED_ROOTS');
  }
  return value;
}

export function validateLimit(value: number, ceiling: number, label: string): number {
  if (!Number.isSafeInteger(value) || value < 1 || value > ceiling) {
    throw new TypeError(`${label} must be a safe integer within the configured ceiling`);
  }
  return value;
}

export function signalProcessGroup(
  child: Readonly<{ pid?: number; kill(signal?: NodeJS.Signals): boolean }>,
  signal: NodeJS.Signals,
): void {
  if (child.pid === undefined) return;
  try {
    if (process.platform !== 'win32') process.kill(-child.pid, signal);
    else child.kill(signal);
  } catch {
    try {
      child.kill(signal);
    } catch {
      // The process already exited.
    }
  }
}

export function cancelledResult(executionId: string): NativeProcessResult {
  return Object.freeze({
    executionId, exitCode: null, stdout: '', stderr: '', timedOut: false,
    cancelled: true, outputLimitExceeded: false,
    stdoutDigest: digestValue(''), stderrDigest: digestValue(''),
  });
}

export function spawnFailure(executionId: string, error: unknown): NativeProcessResult {
  return Object.freeze({
    executionId, exitCode: null, stdout: '', stderr: '', timedOut: false,
    cancelled: false, outputLimitExceeded: false, spawnError: errorMessage(error),
    stdoutDigest: digestValue(''), stderrDigest: digestValue(''),
  });
}

export function digestValue(value: unknown): string {
  const bytes = Buffer.isBuffer(value) ? value : Buffer.from(JSON.stringify(value));
  return createHash('sha256').update(bytes).digest('hex');
}

function contains(root: string, path: string): boolean {
  const delta = relative(root, path);
  return delta === ''
    || (delta !== '..' && !delta.startsWith(`..${sep}`) && !isAbsolute(delta));
}

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
