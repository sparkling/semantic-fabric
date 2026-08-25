// SPDX-License-Identifier: MIT

import { spawn } from 'node:child_process';
import type { ChildProcessByStdio } from 'node:child_process';
import type { Readable, Writable } from 'node:stream';
import type { NativeProcessRequest, NativeProcessResult } from './models/types.js';
import {
  digestValue,
  errorMessage,
  signalProcessGroup,
  spawnFailure,
} from './native-process-contracts.js';
import {
  terminateNativeResourceScope,
  type NativeResourceBoundary,
  type NativeResourceIsolationResult,
} from './resource-boundary.js';

type NativeChild = ChildProcessByStdio<Writable, Readable, Readable>;

export interface NativeExecutionOptions {
  readonly executionId: string;
  readonly request: NativeProcessRequest;
  readonly resources: NativeResourceIsolationResult;
  readonly boundary: NativeResourceBoundary;
  readonly maxOutputBytes: number;
  readonly terminationGraceMs: number;
}

export async function executeNativeProcess(
  options: NativeExecutionOptions,
): Promise<NativeProcessResult> {
  const { executionId, request, resources, boundary } = options;
  return await new Promise<NativeProcessResult>((resolveResult, rejectResult) => {
    let child: NativeChild;
    try {
      child = spawn(resources.command.executable, [...resources.command.args], {
        cwd: resources.command.cwd,
        env: { ...(boundary.launchEnvironment?.(resources.command.env) ?? resources.command.env) },
        shell: false,
        detached: process.platform !== 'win32',
        stdio: ['pipe', 'pipe', 'pipe'],
      });
    } catch (error) {
      resolveResult(spawnFailure(executionId, error));
      return;
    }
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    let capturedBytes = 0;
    let observedBytes = 0;
    let timedOut = false;
    let cancelled = false;
    let outputLimitExceeded = false;
    let spawnError: string | undefined;
    let terminationPromise: Promise<boolean> | undefined;
    let releasePromise: Promise<boolean> | undefined;
    let killTimer: NodeJS.Timeout | undefined;
    let timeout: NodeJS.Timeout | undefined;
    let settled = false;

    const clearController = () => {
      if (timeout) clearTimeout(timeout);
      if (killTimer) clearTimeout(killTimer);
      request.signal?.removeEventListener('abort', abort);
    };
    const failTermination = () => {
      if (settled) return;
      settled = true;
      clearController();
      child.stdin.destroy();
      child.stdout.destroy();
      child.stderr.destroy();
      rejectResult(new Error('HARNESS_NATIVE_RESOURCE_TERMINATION_FAILED'));
    };
    const verifyReleased = (): Promise<boolean> | undefined => {
      if (terminationPromise === undefined) return undefined;
      releasePromise ??= terminationPromise.then(async (initiallyReleased) => {
        try {
          await terminateNativeResourceScope(boundary, resources);
          return initiallyReleased;
        } catch {
          return false;
        }
      });
      return releasePromise;
    };
    const observeRelease = () => {
      const release = verifyReleased();
      if (release !== undefined) void release.then((released) => {
        if (!released) failTermination();
      });
    };
    const terminate = () => {
      if (terminationPromise !== undefined) return;
      terminationPromise = Promise.resolve()
        .then(() => terminateNativeResourceScope(boundary, resources))
        .then(() => true, () => false);
      if (child.exitCode !== null || child.signalCode !== null) observeRelease();
      signalProcessGroup(child, 'SIGTERM');
      killTimer ??= setTimeout(
        () => signalProcessGroup(child, 'SIGKILL'),
        options.terminationGraceMs,
      );
      killTimer.unref();
    };
    const capture = (target: Buffer[], chunk: Buffer) => {
      observedBytes += chunk.length;
      const room = Math.max(0, options.maxOutputBytes - capturedBytes);
      if (room > 0) {
        const kept = chunk.subarray(0, room);
        target.push(kept);
        capturedBytes += kept.length;
      }
      if (observedBytes > options.maxOutputBytes && !outputLimitExceeded) {
        outputLimitExceeded = true;
        terminate();
      }
    };
    child.stdout.on('data', (chunk: Buffer) => capture(stdout, chunk));
    child.stderr.on('data', (chunk: Buffer) => capture(stderr, chunk));
    child.on('error', (error) => { spawnError = errorMessage(error); });
    child.stdin.on('error', () => { /* Failure is captured in process evidence. */ });
    child.stdin.end(request.stdin ?? '');

    timeout = setTimeout(() => {
      timedOut = true;
      terminate();
    }, request.timeoutMs);
    timeout.unref();
    const abort = () => {
      cancelled = true;
      terminate();
    };
    request.signal?.addEventListener('abort', abort, { once: true });
    child.on('exit', observeRelease);
    child.on('close', async (exitCode) => {
      if (settled) return;
      clearController();
      if (terminationPromise !== undefined && !(await verifyReleased())) {
        failTermination();
        return;
      }
      if (settled) return;
      settled = true;
      const stdoutBytes = Buffer.concat(stdout);
      const stderrBytes = Buffer.concat(stderr);
      resolveResult(Object.freeze({
        executionId,
        exitCode,
        stdout: stdoutBytes.toString('utf8'),
        stderr: stderrBytes.toString('utf8'),
        timedOut,
        cancelled,
        outputLimitExceeded,
        ...(spawnError === undefined ? {} : { spawnError }),
        stdoutDigest: digestValue(stdoutBytes),
        stderrDigest: digestValue(stderrBytes),
      }));
    });
  });
}
