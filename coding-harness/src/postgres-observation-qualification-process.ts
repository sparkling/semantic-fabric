// SPDX-License-Identifier: MIT

import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { isAbsolute } from 'node:path';

export interface QualificationProcessResult {
  readonly status: number | null;
  readonly signal: NodeJS.Signals | null;
  readonly stdout: Buffer;
  readonly stderr: Buffer;
  readonly timedOut: boolean;
  readonly outputLimitExceeded: boolean;
  readonly spawnError: string | null;
}

export interface QualificationProcessRequest {
  readonly file: string;
  readonly args: readonly string[];
  readonly cwd: string;
  readonly environment: Readonly<Record<string, string>>;
  readonly timeoutMs: number;
  readonly maxOutputBytes: number;
  readonly stdin?: Buffer;
}

export interface QualificationProcessExecutor {
  run(request: QualificationProcessRequest): Promise<QualificationProcessResult>;
}

export const nativeQualificationProcessExecutor: QualificationProcessExecutor = Object.freeze({
  run: runQualificationProcess,
});

export function runQualificationProcess(
  request: QualificationProcessRequest,
): Promise<QualificationProcessResult> {
  validateRequest(request);
  return new Promise((resolveResult) => {
    let child: ChildProcessWithoutNullStreams;
    try {
      child = spawn(request.file, [...request.args], {
        cwd: request.cwd,
        env: { ...request.environment },
        shell: false,
        detached: process.platform !== 'win32',
        stdio: ['pipe', 'pipe', 'pipe'],
      });
    } catch (error) {
      resolveResult(result({ spawnError: message(error) }));
      return;
    }

    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    let stdoutBytes = 0;
    let stderrBytes = 0;
    let retainedBytes = 0;
    let timedOut = false;
    let outputLimitExceeded = false;
    let spawnError: string | null = null;
    let settled = false;
    let killTimer: NodeJS.Timeout | undefined;

    const terminate = () => {
      signalProcessGroup(child, 'SIGTERM');
      killTimer ??= setTimeout(() => signalProcessGroup(child, 'SIGKILL'), 2_000);
      killTimer.unref();
    };
    const capture = (target: Buffer[], chunk: Buffer, stdoutTarget: boolean) => {
      if (stdoutTarget) stdoutBytes += chunk.length;
      else stderrBytes += chunk.length;
      const observed = stdoutBytes + stderrBytes;
      const room = Math.max(0, request.maxOutputBytes - retainedBytes);
      if (room > 0) {
        const kept = chunk.subarray(0, room);
        target.push(kept);
        retainedBytes += kept.length;
      }
      if (observed > request.maxOutputBytes && !outputLimitExceeded) {
        outputLimitExceeded = true;
        terminate();
      }
    };
    child.stdout.on('data', (chunk: Buffer) => capture(stdout, chunk, true));
    child.stderr.on('data', (chunk: Buffer) => capture(stderr, chunk, false));
    child.on('error', (error) => { spawnError = message(error); });
    const timeout = setTimeout(() => {
      timedOut = true;
      terminate();
    }, request.timeoutMs);
    timeout.unref();
    child.on('close', (status, signal) => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      if (killTimer !== undefined) clearTimeout(killTimer);
      resolveResult(result({
        status,
        signal,
        stdout: Buffer.concat(stdout),
        stderr: Buffer.concat(stderr),
        timedOut,
        outputLimitExceeded,
        spawnError,
      }));
    });
    child.stdin.on('error', () => {
      // An early child exit is represented by its status; EPIPE is not fatal here.
    });
    child.stdin.end(request.stdin);
  });
}

function validateRequest(request: QualificationProcessRequest): void {
  if (typeof request.file !== 'string' || !isAbsolute(request.file) || request.file.includes('\0')
    || !Array.isArray(request.args) || request.args.some((value) =>
      typeof value !== 'string' || value.includes('\0'))
    || typeof request.cwd !== 'string' || !isAbsolute(request.cwd) || request.cwd.includes('\0')
    || !Number.isSafeInteger(request.timeoutMs) || request.timeoutMs < 1
    || !Number.isSafeInteger(request.maxOutputBytes) || request.maxOutputBytes < 1
    || (request.stdin !== undefined
      && (!Buffer.isBuffer(request.stdin) || request.stdin.length > request.maxOutputBytes))) {
    throw new TypeError('POSTGRES_QUALIFICATION_PROCESS_REQUEST_INVALID');
  }
  for (const [name, value] of Object.entries(request.environment)) {
    if (!/^[A-Z_][A-Z0-9_]*$/.test(name) || typeof value !== 'string'
      || value.includes('\0')) {
      throw new TypeError('POSTGRES_QUALIFICATION_PROCESS_ENVIRONMENT_INVALID');
    }
  }
}

function result(
  overrides: Partial<QualificationProcessResult> = {},
): QualificationProcessResult {
  return Object.freeze({
    status: null,
    signal: null,
    stdout: Buffer.alloc(0),
    stderr: Buffer.alloc(0),
    timedOut: false,
    outputLimitExceeded: false,
    spawnError: null,
    ...overrides,
  });
}

function signalProcessGroup(
  child: ChildProcessWithoutNullStreams,
  signal: NodeJS.Signals,
): void {
  if (child.pid === undefined) return;
  try {
    if (process.platform === 'win32') child.kill(signal);
    else process.kill(-child.pid, signal);
  } catch {
    try { child.kill(signal); } catch { /* The process already exited. */ }
  }
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
