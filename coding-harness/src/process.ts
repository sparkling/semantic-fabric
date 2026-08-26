// SPDX-License-Identifier: MIT

import { spawn } from 'node:child_process';
import type { ChildProcessByStdio } from 'node:child_process';
import type { Readable } from 'node:stream';
import type { HarnessConfig, StructuredCommand } from './contracts.js';
import { isForbiddenEnvironmentName, parseStructuredCommand } from './contracts.js';
import {
  isolateOfflineCandidateCommand,
  type BoundaryCommand,
  type OfflineProcessIsolator,
} from './network.js';
import { resolveWorkspacePath } from './workspace.js';
import type { WritableFileOverlay } from './writable-overlays.js';

export interface ProcessContext {
  workspaceRoot: string;
  config: HarnessConfig;
  declaredTools: readonly string[];
  sourceEnvironment?: NodeJS.ProcessEnv;
  signal?: AbortSignal;
  boundary:
    | { kind: 'trusted-control-plane' }
    | {
        kind: 'offline-candidate';
        isolator: OfflineProcessIsolator;
        writablePaths: readonly string[];
        writableOverlays?: readonly WritableFileOverlay[];
      };
}

export interface ProcessResult {
  success: boolean;
  exitCode: number | null;
  signal: NodeJS.Signals | null;
  stdout: string;
  stderr: string;
  startedAt: string;
  durationMs: number;
  timedOut: boolean;
  cancelled: boolean;
  outputLimitExceeded: boolean;
  spawnError: string | null;
}

type StructuredChild = ChildProcessByStdio<null, Readable, Readable>;

export function sanitizeEnvironment(
  source: NodeJS.ProcessEnv,
  overrides: Readonly<Record<string, string>>,
  config: HarnessConfig,
): NodeJS.ProcessEnv {
  const allowed = new Set(config.environment.allow);
  const clean: NodeJS.ProcessEnv = {};
  for (const [rawName, value] of Object.entries(source)) {
    const name = rawName.toUpperCase();
    if (typeof value === 'string' && allowed.has(name) && !isForbiddenEnvironmentName(name, config)) {
      clean[name] = value;
    }
  }
  for (const [rawName, value] of Object.entries(overrides)) {
    const name = rawName.toUpperCase();
    if (!allowed.has(name) || isForbiddenEnvironmentName(name, config)) {
      throw new Error(`environment variable ${rawName} is not allowed`);
    }
    clean[name] = value;
  }
  return clean;
}

export async function runStructuredProcess(
  input: StructuredCommand,
  context: ProcessContext,
): Promise<ProcessResult> {
  const command = parseStructuredCommand(input, context.config, context.declaredTools);
  const cwd = resolveWorkspacePath(context.workspaceRoot, command.cwd, {
    allowRoot: true,
    requireDirectory: true,
  });
  const started = Date.now();
  const startedAt = new Date(started).toISOString();
  if (context.signal?.aborted) return cancelledBeforeStart(startedAt);

  const env = sanitizeEnvironment(context.sourceEnvironment ?? process.env, command.env, context.config);
  if (context.boundary === undefined) throw new Error('HARNESS_PROCESS_BOUNDARY_REQUIRED');
  const isolation = context.boundary.kind === 'trusted-control-plane'
    ? null
    : await isolateOfflineCandidateCommand({
      mode: 'offline',
      channel: 'candidate-command',
      stage: 'candidate-execution',
      deterministic: true,
      allowedOrigins: [],
      command: boundaryCommand({
        executable: command.executable,
        args: command.argv,
        cwd,
        env: definedEnvironment(env),
        writablePaths: context.boundary.writablePaths,
      }, context.boundary.writableOverlays),
    }, context.boundary.isolator);
  const launch = isolation?.command
    ?? { executable: command.executable, args: command.argv, cwd, env };
  if (context.boundary.kind === 'offline-candidate') await context.boundary.isolator.assertStable();
  const launchEnvironment = context.boundary.kind === 'offline-candidate'
    ? (context.boundary.isolator.launchEnvironment?.(definedEnvironment(launch.env))
      ?? definedEnvironment(launch.env))
    : launch.env;
  if (context.signal?.aborted) return cancelledBeforeStart(startedAt);
  const offlineBoundary = context.boundary.kind === 'offline-candidate'
    ? context.boundary
    : null;
  return new Promise((resolveResult, rejectResult) => {
    let child: StructuredChild;
    try {
      child = spawn(launch.executable, [...launch.args], {
        cwd: launch.cwd,
        env: { ...launchEnvironment },
        shell: false,
        detached: process.platform !== 'win32',
        stdio: ['ignore', 'pipe', 'pipe'],
      });
    } catch (error) {
      resolveResult(finishedResult({ started, startedAt, spawnError: errorMessage(error) }));
      return;
    }

    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    let capturedBytes = 0;
    let observedBytes = 0;
    let timedOut = false;
    let cancelled = false;
    let outputLimitExceeded = false;
    let spawnError: string | null = null;
    let terminationPromise: Promise<boolean> | undefined;
    let releasePromise: Promise<boolean> | undefined;
    let killTimer: NodeJS.Timeout | undefined;
    let timeout: NodeJS.Timeout | undefined;
    let settled = false;

    const clearController = () => {
      if (timeout) clearTimeout(timeout);
      if (killTimer) clearTimeout(killTimer);
      context.signal?.removeEventListener('abort', abort);
    };
    const failTermination = () => {
      if (settled) return;
      settled = true;
      clearController();
      child.stdout.destroy();
      child.stderr.destroy();
      rejectResult(new Error('HARNESS_OFFLINE_RESOURCE_TERMINATION_FAILED'));
    };
    const verifyReleased = (): Promise<boolean> | undefined => {
      if (terminationPromise === undefined || offlineBoundary === null) {
        return undefined;
      }
      releasePromise ??= terminationPromise.then(async (initiallyReleased) => {
        try {
          await offlineBoundary.isolator.terminateAndVerify(isolation!.resourceScope);
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
      if (isolation !== null) {
        if (terminationPromise !== undefined) return;
        const requested = offlineBoundary !== null
          ? Promise.resolve().then(() =>
            offlineBoundary.isolator.terminateAndVerify(isolation.resourceScope))
          : Promise.reject(new Error('HARNESS_OFFLINE_RESOURCE_TERMINATION_REQUIRED'));
        terminationPromise = requested.then(() => true, () => false);
        if (child.exitCode !== null || child.signalCode !== null) observeRelease();
      }
      signalProcessGroup(child, 'SIGTERM');
      killTimer ??= setTimeout(
        () => signalProcessGroup(child, 'SIGKILL'),
        context.config.limits.terminationGraceMs,
      );
      killTimer.unref();
    };
    const capture = (target: Buffer[], chunk: Buffer) => {
      observedBytes += chunk.length;
      const room = Math.max(0, command.maxOutputBytes - capturedBytes);
      if (room > 0) {
        const kept = chunk.subarray(0, room);
        target.push(kept);
        capturedBytes += kept.length;
      }
      if (observedBytes > command.maxOutputBytes && !outputLimitExceeded) {
        outputLimitExceeded = true;
        terminate();
      }
    };
    child.stdout.on('data', (chunk: Buffer) => capture(stdout, chunk));
    child.stderr.on('data', (chunk: Buffer) => capture(stderr, chunk));
    child.on('error', (error) => {
      spawnError = errorMessage(error);
    });

    timeout = setTimeout(() => {
      timedOut = true;
      terminate();
    }, command.timeoutMs);
    timeout.unref();

    const abort = () => {
      cancelled = true;
      terminate();
    };
    context.signal?.addEventListener('abort', abort, { once: true });
    child.on('exit', observeRelease);

    child.on('close', async (exitCode, signal) => {
      if (settled) return;
      clearController();
      if (terminationPromise !== undefined && offlineBoundary !== null) {
        if (!(await verifyReleased())) {
          failTermination();
          return;
        }
      }
      if (settled) return;
      settled = true;
      if (offlineBoundary !== null) {
        try {
          await offlineBoundary.isolator.assertStable();
        } catch (error) {
          spawnError ??= errorMessage(error);
        }
      }
      resolveResult(finishedResult({
        started,
        startedAt,
        exitCode,
        signal,
        stdout: Buffer.concat(stdout).toString('utf8'),
        stderr: Buffer.concat(stderr).toString('utf8'),
        timedOut,
        cancelled,
        outputLimitExceeded,
        spawnError,
      }));
    });
  });
}

function boundaryCommand(
  command: BoundaryCommand,
  overlays: readonly WritableFileOverlay[] | undefined,
): BoundaryCommand {
  return overlays === undefined || overlays.length === 0
    ? command
    : { ...command, writableOverlays: overlays };
}

function definedEnvironment(
  environment: Readonly<Record<string, string | undefined>>,
): Readonly<Record<string, string>> {
  return Object.freeze(Object.fromEntries(
    Object.entries(environment).filter((entry): entry is [string, string] =>
      typeof entry[1] === 'string'),
  ));
}

function signalProcessGroup(child: StructuredChild, signal: NodeJS.Signals): void {
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

function finishedResult(
  values: Partial<ProcessResult> & Pick<ProcessResult, 'startedAt'> & { started: number },
): ProcessResult {
  const result = {
    success: false,
    exitCode: null,
    signal: null,
    stdout: '',
    stderr: '',
    durationMs: Math.max(0, Date.now() - values.started),
    timedOut: false,
    cancelled: false,
    outputLimitExceeded: false,
    spawnError: null,
    ...values,
  };
  const { started: _started, ...publicResult } = result;
  publicResult.success = publicResult.exitCode === 0
    && !publicResult.timedOut
    && !publicResult.cancelled
    && !publicResult.outputLimitExceeded
    && publicResult.spawnError === null;
  return publicResult;
}

function cancelledBeforeStart(startedAt: string): ProcessResult {
  return {
    success: false,
    exitCode: null,
    signal: null,
    stdout: '',
    stderr: '',
    startedAt,
    durationMs: 0,
    timedOut: false,
    cancelled: true,
    outputLimitExceeded: false,
    spawnError: null,
  };
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
