// SPDX-License-Identifier: MIT
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream, writeSync } from 'node:fs';
import { updateOperationChild } from './delivery-workspace.js';

/** Environment for trusted repository checks, not a model invocation or sandbox. */
export function buildCheckEnvironment(): Record<string, string> {
  const env: Record<string, string> = {};
  for (const name of Object.keys(process.env)) {
    if (/^(?:PATH|HOME|USER|LOGNAME|SHELL|LANG|LC_ALL|LC_CTYPE|TERM|NO_COLOR|TMPDIR|TMP|TEMP|CI|RUSTUP_HOME|RUSTUP_TOOLCHAIN|CARGO_HOME|CARGO_TARGET_DIR|RUSTFLAGS|CARGO_ENCODED_RUSTFLAGS|RUST_BACKTRACE|CARGO_BUILD_JOBS|CARGO_INCREMENTAL|CARGO_NET_OFFLINE|SF_TEST_[A-Z0-9_]+)$/.test(name)
      && !/(?:API_KEY|TOKEN|SECRET|OPENROUTER|PROXY|BASE_URL)/.test(name)) env[name] = process.env[name]!;
  }
  return env;
}
export function checkEnvironmentEvidence(environment: Record<string, string>): Record<string, string> {
  const evidence = { ...environment };
  if (evidence.PATH === undefined) return evidence;
  const seen = new Set<string>();
  evidence.PATH = evidence.PATH.split(':').filter(entry => {
    if (/(?:^|\/)\.codex\/tmp\/arg0\/codex-arg0[^/]*$/.test(entry) || seen.has(entry)) return false;
    seen.add(entry); return true;
  }).join(':');
  return evidence;
}
export async function logDigest(path: string): Promise<string> {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest('hex');
}
export interface NativeCommandObserver {
  start(pid: number | undefined): void;
  stdout(data: Buffer): void;
  stderr(data: Buffer): void;
  tick(): string | undefined;
  finish(): { stdout: string; stderr: string; error?: string | undefined };
}
export function runCommand(argv: string[], cwd: string, env: Record<string, string>, out: number, err: number,
  directory: string, limits: { timeoutMs?: number; maxOutputBytes?: number }, signal?: AbortSignal, stdin?: string, native?: NativeCommandObserver,
): Promise<{ exitCode: number | null; signal: string | null; error?: string }> {
  return new Promise(resolve => {
    if (signal?.aborted) { resolve({ exitCode: null, signal: null, error: 'cancelled' }); return; }
    updateOperationChild(directory, 'starting');
    const child = spawn(argv[0], argv.slice(1), { cwd, env, detached: true, shell: false, stdio: ['pipe', 'pipe', 'pipe'] });
    child.stdin.on('error', () => {}); child.stdin.end(stdin);
    updateOperationChild(directory, 'running', child.pid);
    let error: string | undefined;
    let termination: ReturnType<typeof setTimeout> | undefined;
    const stop = (sig: NodeJS.Signals) => {
      if (child.pid) { try { process.kill(-child.pid, sig); } catch {} }
    };
    const cancel = (why: string) => {
      if (error) return;
      error = why; stop('SIGTERM');
      termination = setTimeout(() => stop('SIGKILL'), 1000); termination.unref();
    };
    const abort = () => cancel('cancelled');
    signal?.addEventListener('abort', abort, { once: true });
    const timeout = setTimeout(() => cancel('check-timeout'), limits.timeoutMs ?? 1_800_000);
    let bytes = 0;
    const maximum = limits.maxOutputBytes ?? 10_000_000;
    const copy = (fd: number, data: Buffer) => {
      const available = Math.max(0, maximum - bytes);
      const keep = data.subarray(0, available);
      let offset = 0;
      try { while (offset < keep.length) offset += writeSync(fd, keep, offset, keep.length - offset); }
      catch (e) { cancel(`log-write-failed:${String(e)}`); }
      bytes += data.length;
      if (bytes > maximum) cancel('check-output-limit');
    };
    const observe = (action: () => void) => { try { action(); } catch { cancel('native-progress-write-failed'); } };
    if (native) observe(() => native.start(child.pid));
    const inactivity = native ? setInterval(() => observe(() => {
      const reason = native.tick(); if (reason) cancel(reason);
    }), 50) : undefined;
    child.stdout.on('data', data => native ? observe(() => native.stdout(data)) : copy(out, data));
    child.stderr.on('data', data => native ? observe(() => native.stderr(data)) : copy(err, data));
    const clean = () => {
      clearTimeout(timeout); if (inactivity) clearInterval(inactivity); if (termination) clearTimeout(termination);
      signal?.removeEventListener('abort', abort);
    };
    child.once('error', e => { error = e.message; });
    child.once('close', async (code, sig) => {
      const alive = () => {
        if (!child.pid) return false;
        try { process.kill(-child.pid, 0); return true; }
        catch (e) { return (e as NodeJS.ErrnoException).code !== 'ESRCH'; }
      };
      if (alive()) { error ??= 'check-descendant-survived'; stop('SIGKILL'); }
      const deadline = performance.now() + 1000;
      while (alive() && performance.now() < deadline) await new Promise(r => setTimeout(r, 20));
      if (alive()) { error = 'check-process-group-unconfirmed'; updateOperationChild(directory, 'unconfirmed', child.pid); }
      if (native) {
        try {
          const final = native.finish();
          error ??= final.error;
          const write = (fd: number, value: string) => {
            const data = Buffer.from(value); let offset = 0;
            while (offset < data.length) offset += writeSync(fd, data, offset, data.length - offset);
          };
          write(out, final.stdout); write(err, final.stderr);
        } catch { error ??= 'native-progress-write-failed'; }
      }
      clean(); resolve({ exitCode: code, signal: sig, ...(error ? { error } : {}) });
    });
  });
}
