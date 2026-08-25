// SPDX-License-Identifier: MIT

import { existsSync, realpathSync, statSync } from 'node:fs';
import { isAbsolute, relative, resolve, sep } from 'node:path';
import { resolveWorkspacePath } from '../workspace.js';
import type {
  ClaudeInvocationRequest,
  NativePreflightRequest,
  NativeProcessResult,
} from './types.js';

const MAX_PROMPT_BYTES = 1_000_000;

export function assertAdapterExecutable(executable: string): void {
  if (executable.trim().length === 0 || executable.includes('\0')) {
    throw new Error('HARNESS_NATIVE_EXECUTABLE_INVALID');
  }
}

export function validatePreflightRequest(request: NativePreflightRequest): void {
  validateAbsolutePath(request.cwd, 'CWD');
  validateModel(request.requestedModel);
}

export function validateInvocation(request: ClaudeInvocationRequest): void {
  validateAbsolutePath(request.cwd, 'CWD');
  validateModel(request.model);
  if (request.prompt.trim().length === 0
    || request.prompt.includes('\0')
    || Buffer.byteLength(request.prompt, 'utf8') > MAX_PROMPT_BYTES) {
    throw new Error('HARNESS_NATIVE_PROMPT_INVALID');
  }
  if (request.schema === null || Array.isArray(request.schema)
    || !Number.isSafeInteger(request.timeoutMs) || request.timeoutMs < 1) {
    throw new Error('HARNESS_NATIVE_INVOCATION_INVALID');
  }
}

export function validateScopedPath(
  root: string,
  path: string,
  field: string,
  requireFile: boolean,
): void {
  validateAbsolutePath(root, 'CWD');
  validateAbsolutePath(path, field);
  const delta = relative(root, path);
  if (delta === '' || delta === '..' || delta.startsWith(`..${sep}`) || isAbsolute(delta)) {
    throw new Error(`HARNESS_NATIVE_${field}_OUTSIDE_CWD`);
  }
  const workspacePath = delta.split(sep).join('/');
  try {
    const absolute = resolveWorkspacePath(root, workspacePath, requireFile
      ? { requireRegularFile: true, rejectHardlinks: true }
      : { allowMissingLeaf: true });
    if (!requireFile && existsSync(absolute)) {
      resolveWorkspacePath(root, workspacePath, {
        requireRegularFile: true,
        rejectHardlinks: true,
      });
    }
  } catch (error) {
    throw new Error(`HARNESS_NATIVE_${field}_INVALID`, { cause: error });
  }
}

export function validateRoot(path: string, field: string): string {
  validateAbsolutePath(path, field);
  if (realpathSync(path) !== path || !statSync(path).isDirectory()) {
    throw new Error(`HARNESS_NATIVE_${field}_INVALID`);
  }
  return path;
}

export function processSucceeded(result: NativeProcessResult): boolean {
  return /^native-run:[0-9a-f-]{36}$/.test(result.executionId)
    && /^[a-f0-9]{64}$/.test(result.stdoutDigest)
    && /^[a-f0-9]{64}$/.test(result.stderrDigest)
    && result.exitCode === 0 && !result.timedOut && result.cancelled !== true
    && result.outputLimitExceeded !== true && result.spawnError === undefined;
}

export function signalAborted(signal?: AbortSignal): boolean {
  return signal?.aborted === true;
}

export function parseRecord(text: string): Record<string, unknown> | null {
  try {
    const value = JSON.parse(text) as unknown;
    return value !== null && typeof value === 'object' && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

function validateModel(model: string): void {
  if (!/^[A-Za-z0-9][A-Za-z0-9._:-]{0,199}$/.test(model)) {
    throw new Error('HARNESS_NATIVE_MODEL_INVALID');
  }
}

function validateAbsolutePath(path: string, field: string): void {
  if (!isAbsolute(path) || resolve(path) !== path || path.includes('\0')) {
    throw new Error(`HARNESS_NATIVE_${field}_INVALID`);
  }
}
