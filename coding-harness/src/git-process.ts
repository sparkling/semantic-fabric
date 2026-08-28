// SPDX-License-Identifier: MIT

import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { lstatSync, readFileSync, realpathSync } from 'node:fs';
import { isAbsolute, resolve } from 'node:path';

export interface GitCommandResult {
  readonly stdout: string;
  readonly stderr: string;
  readonly exitCode: number | null;
}

export interface GitCommandBytesResult {
  readonly stdout: Buffer;
  readonly stderr: string;
  readonly exitCode: number | null;
}

interface RawGitCommandResult {
  readonly stdout: Buffer;
  readonly stderr: Buffer;
  readonly exitCode: number | null;
}

interface GitCommandOptions {
  readonly stdin?: string;
  readonly signal?: AbortSignal;
  readonly environment?: Readonly<Record<string, string>>;
  readonly timeoutMs?: number;
  readonly maxOutputBytes?: number;
}

const DEFAULT_MAX_OUTPUT_BYTES = 10_000_000;
const DEFAULT_TIMEOUT_MS = 30_000;
const GIT_EXECUTABLE = process.platform === 'win32'
  ? 'C:\\Program Files\\Git\\cmd\\git.exe'
  : '/usr/bin/git';
const GIT_IDENTITY = validateGitExecutable(GIT_EXECUTABLE);
const GIT_CONFIG_ARGS = Object.freeze([
  '-c', 'core.hooksPath=/dev/null',
  '-c', 'core.fsmonitor=false',
  '-c', 'core.pager=cat',
] as const);
const EXTRA_ENVIRONMENT = new Set([
  'GIT_INDEX_FILE', 'GIT_AUTHOR_NAME', 'GIT_AUTHOR_EMAIL', 'GIT_AUTHOR_DATE',
  'GIT_COMMITTER_NAME', 'GIT_COMMITTER_EMAIL', 'GIT_COMMITTER_DATE',
]);

export async function runGitCommand(
  cwd: string,
  args: readonly string[],
  options: GitCommandOptions = {},
): Promise<GitCommandResult> {
  const result = await runRawGitCommand(cwd, args, options);
  return Object.freeze({
    stdout: result.stdout.toString('utf8'),
    stderr: result.stderr.toString('utf8'),
    exitCode: result.exitCode,
  });
}

export async function runGitCommandBytes(
  cwd: string,
  args: readonly string[],
  options: GitCommandOptions = {},
): Promise<GitCommandBytesResult> {
  const result = await runRawGitCommand(cwd, args, options);
  return Object.freeze({
    stdout: result.stdout,
    stderr: result.stderr.toString('utf8'),
    exitCode: result.exitCode,
  });
}

function runRawGitCommand(
  cwd: string,
  args: readonly string[],
  options: GitCommandOptions,
): Promise<RawGitCommandResult> {
  return new Promise((resolveResult, reject) => {
    if (options.signal?.aborted === true) {
      reject(gitAbortError());
      return;
    }
    assertGitExecutableStable();
    const child = spawn(GIT_IDENTITY.path, gitArguments(args), {
      cwd,
      shell: false,
      detached: process.platform !== 'win32',
      stdio: ['pipe', 'pipe', 'pipe'],
      env: gitEnvironment(options.environment),
    });
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    let bytes = 0;
    let settled = false;
    let aborted = false;
    let timedOut = false;
    const terminate = () => signalProcessGroup(child, 'SIGKILL');
    const timeout = setTimeout(() => {
      timedOut = true;
      terminate();
    }, validateLimit(options.timeoutMs ?? DEFAULT_TIMEOUT_MS, 'timeoutMs'));
    timeout.unref();
    const outputLimit = validateLimit(
      options.maxOutputBytes ?? DEFAULT_MAX_OUTPUT_BYTES,
      'maxOutputBytes',
    );
    const capture = (target: Buffer[], chunk: Buffer) => {
      bytes += chunk.length;
      if (bytes > outputLimit) terminate();
      else target.push(chunk);
    };
    child.stdout.on('data', (chunk: Buffer) => capture(stdout, chunk));
    child.stderr.on('data', (chunk: Buffer) => capture(stderr, chunk));
    const abort = () => {
      aborted = true;
      terminate();
    };
    options.signal?.addEventListener('abort', abort, { once: true });
    child.on('error', (error) => settle(() => reject(error)));
    child.on('close', (exitCode) => settle(() => {
      if (aborted) {
        reject(gitAbortError());
        return;
      }
      if (timedOut) {
        reject(new Error('HARNESS_GIT_TIMEOUT'));
        return;
      }
      if (bytes > outputLimit) {
        reject(new Error('HARNESS_GIT_OUTPUT_LIMIT_EXCEEDED'));
        return;
      }
      resolveResult(Object.freeze({
        stdout: Buffer.concat(stdout),
        stderr: Buffer.concat(stderr),
        exitCode,
      }));
    }));
    child.stdin.on('error', () => {
      // Early Git exit is represented by exit status; EPIPE must not crash the controller.
    });
    child.stdin.end(options.stdin);

    function settle(action: () => void): void {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      options.signal?.removeEventListener('abort', abort);
      action();
    }
  });
}

export function gitAbortError(): Error {
  const error = new Error('HARNESS_GIT_CANCELLED');
  error.name = 'AbortError';
  return error;
}

export function gitExecutableEvidence(): Readonly<{ path: string; digest: string }> {
  return GIT_IDENTITY;
}

function gitEnvironment(extra?: Readonly<Record<string, string>>): NodeJS.ProcessEnv {
  const environment: NodeJS.ProcessEnv = {
    PATH: process.platform === 'win32' ? 'C:\\Windows\\System32' : '/usr/bin:/bin',
    HOME: process.platform === 'win32' ? 'C:\\nonexistent' : '/nonexistent',
    LANG: 'C',
    LC_ALL: 'C',
    GIT_CONFIG_NOSYSTEM: '1',
    GIT_CONFIG_GLOBAL: process.platform === 'win32' ? 'NUL' : '/dev/null',
    GIT_NO_REPLACE_OBJECTS: '1',
    GIT_NO_LAZY_FETCH: '1',
    GIT_ATTR_NOSYSTEM: '1',
    GIT_TERMINAL_PROMPT: '0',
    GIT_ASKPASS: process.platform === 'win32' ? 'cmd.exe /c exit 1' : '/bin/false',
    GIT_PAGER: 'cat',
    PAGER: 'cat',
  };
  for (const [name, value] of Object.entries(extra ?? {})) {
    if (!EXTRA_ENVIRONMENT.has(name) || value.includes('\0')) {
      throw new Error(`HARNESS_GIT_ENVIRONMENT_FORBIDDEN:${name}`);
    }
    environment[name] = value;
  }
  return environment;
}

function validateGitExecutable(path: string): Readonly<{ path: string; digest: string }> {
  if (!isAbsolute(path) || resolve(path) !== path) throw new Error('HARNESS_GIT_EXECUTABLE_INVALID');
  const stat = lstatSync(path);
  const currentUid = typeof process.getuid === 'function' ? process.getuid() : stat.uid;
  if (!stat.isFile() || stat.isSymbolicLink() || realpathSync(path) !== path || stat.nlink !== 1
    || (stat.mode & 0o111) === 0 || (stat.mode & 0o022) !== 0
    || (stat.uid !== 0 && stat.uid !== currentUid)) {
    throw new Error('HARNESS_GIT_EXECUTABLE_UNTRUSTED');
  }
  return Object.freeze({
    path,
    digest: createHash('sha256').update(readFileSync(path)).digest('hex'),
  });
}

function assertGitExecutableStable(): void {
  if (validateGitExecutable(GIT_IDENTITY.path).digest !== GIT_IDENTITY.digest) {
    throw new Error('HARNESS_GIT_EXECUTABLE_CHANGED');
  }
}

function gitArguments(args: readonly string[]): string[] {
  const command = args[0] === 'diff'
    ? ['diff', '--no-ext-diff', ...args.slice(1)]
    : [...args];
  return [...GIT_CONFIG_ARGS, ...command];
}

function signalProcessGroup(child: ReturnType<typeof spawn>, signal: NodeJS.Signals): void {
  if (child.pid === undefined) return;
  try {
    if (process.platform !== 'win32') process.kill(-child.pid, signal);
    else child.kill(signal);
  } catch {
    try { child.kill(signal); } catch { /* Already exited. */ }
  }
}

function validateLimit(value: number, label: string): number {
  if (!Number.isSafeInteger(value) || value < 1) throw new TypeError(`${label} must be positive`);
  return value;
}
