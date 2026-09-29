// SPDX-License-Identifier: MIT
import { createHash, randomUUID } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { closeSync, existsSync, lstatSync, mkdirSync, openSync, readFileSync, realpathSync, renameSync,
  unlinkSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { hash } from '@metaharness/harness';
import { resolveWorkspacePath } from './workspace.js';
import { parseJsonWithoutDuplicateKeys } from './strict-json.js';

export function git(root: string, ...args: string[]): string {
  const literal = args[0] === 'check-ignore' ? [] : ['--literal-pathspecs'];
  return execFileSync('git', [...literal, '-C', root, ...args], { encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 }).trimEnd();
}
export function mainRoot(root: string): string {
  const actual = realpathSync(root);
  if (git(actual, 'rev-parse', '--show-toplevel') !== actual
    || git(actual, 'symbolic-ref', '--short', 'HEAD') !== 'main') throw new Error('DELIVERY_MAIN_ONLY');
  const gitDir = git(actual, 'rev-parse', '--absolute-git-dir');
  for (const name of ['MERGE_HEAD', 'CHERRY_PICK_HEAD', 'REVERT_HEAD', 'rebase-merge', 'rebase-apply', 'index.lock']) {
    if (existsSync(join(gitDir, name))) throw new Error(`DELIVERY_GIT_OPERATION:${name}`);
  }
  return actual;
}
export interface SourceSnapshot { digest: string; files: Record<string, string> }
export function sourceSnapshot(root: string): SourceSnapshot {
  const names = git(root, 'ls-files', '-z', '--cached', '--others', '--exclude-standard').split('\0').filter(Boolean);
  const files: Record<string, string> = {};
  for (const name of [...new Set(names)].sort()) {
    // Managed memory is never opened, even if accidentally tracked.
    if (/^(\.swarm|\.hive-mind)\//.test(name)
      || /^\.claude-flow\/(?:embeddings\.json|system\/)/.test(name)
      || /(^|\/)memory\.db(?:-|$)/.test(name)) continue;
    const path = join(root, name);
    if (!existsSync(path)) continue;
    if (!lstatSync(path).isFile()) throw new Error(`DELIVERY_NONREGULAR_SOURCE:${name}`);
    resolveWorkspacePath(root, name, { requireRegularFile: true });
    const bytes = readFileSync(path);
    files[name] = `${lstatSync(path).mode & 0o111 ? '100755' : '100644'}:${createHash('sha256').update(bytes).digest('hex')}`;
  }
  return { files, digest: hash(files) };
}
export function outsideDigest(snapshot: SourceSnapshot, scope: string[]): string {
  const allowed = new Set(scope);
  return hash(Object.fromEntries(Object.entries(snapshot.files).filter(([p]) => !allowed.has(p))));
}
export function evidenceDirectory(root: string): string {
  for (const path of ['.metaharness', '.metaharness/delivery']) {
    const absolute = resolve(root, path);
    if (!existsSync(absolute)) mkdirSync(absolute, { mode: 0o700 });
    resolveWorkspacePath(root, path, { requireDirectory: true });
  }
  if (git(root, 'check-ignore', '.metaharness/delivery/probe') !== '.metaharness/delivery/probe') {
    throw new Error('DELIVERY_EVIDENCE_MUST_BE_IGNORED');
  }
  return join(root, '.metaharness/delivery');
}
export function readJson(path: string): unknown {
  if (!lstatSync(path).isFile() || lstatSync(path).isSymbolicLink()) throw new Error('DELIVERY_INVALID_RECORD_FILE');
  return parseJsonWithoutDuplicateKeys(readFileSync(path, 'utf8'), 'delivery record');
}
export function atomicJson(path: string, value: unknown): void {
  if (existsSync(path) && (!lstatSync(path).isFile() || lstatSync(path).isSymbolicLink())) {
    throw new Error('DELIVERY_INVALID_RECORD_FILE');
  }
  const temporary = `${path}.${randomUUID()}.tmp`;
  writeFileSync(temporary, JSON.stringify(value, null, 2) + '\n', { flag: 'wx', mode: 0o600 });
  renameSync(temporary, path);
}
export async function withOperationLock<T>(directory: string, action: () => Promise<T>): Promise<T> {
  const lease = acquireLease(directory);
  const path = join(directory, 'operation.lock');
  const nonce = randomUUID();
  try { writeFileSync(path, JSON.stringify({ pid: process.pid, start: processIdentity(process.pid), nonce, phase: 'idle' }), { flag: 'wx', mode: 0o600 }); }
  catch (error) { closeSync(lease); throw new Error('DELIVERY_OPERATION_BUSY: inspect owner; never start another writer', { cause: error }); }
  try { return await action(); } finally {
    try {
      if (existsSync(path)) {
        const lock = readJson(path) as OperationLock;
        if (lock.nonce === nonce && lock.phase !== 'unconfirmed') unlinkSync(path);
      }
    } finally { closeSync(lease); }
  }
}
/** Same OS lease for brief synchronous source/custody transactions, without a microtask lock gap. */
export function withSynchronousOperationLock<T>(directory: string, action: () => T): T {
  const lease = acquireLease(directory), path = join(directory, 'operation.lock'), nonce = randomUUID();
  try { writeFileSync(path, JSON.stringify({ pid: process.pid, start: processIdentity(process.pid), nonce, phase: 'idle' }), { flag: 'wx', mode: 0o600 }); }
  catch (error) { closeSync(lease); throw new Error('DELIVERY_OPERATION_BUSY: inspect owner; never start another writer', { cause: error }); }
  try { return action(); } finally {
    try { if (existsSync(path) && (readJson(path) as OperationLock).nonce === nonce) unlinkSync(path); }
    finally { closeSync(lease); }
  }
}
// Linux flock is held by this process's shared open file description, not a
// second long-lived writer. Kernel release on exit makes recovery itself recoverable.
function acquireLease(directory: string): number {
  const path = join(directory, 'operation.lease');
  if (existsSync(path) && (!lstatSync(path).isFile() || lstatSync(path).isSymbolicLink())) throw new Error('DELIVERY_INVALID_LEASE');
  const fd = openSync(path, 'a', 0o600);
  try { execFileSync('flock', ['-n', '3'], { stdio: ['ignore', 'pipe', 'pipe', fd] }); }
  catch (error) { closeSync(fd); throw new Error('DELIVERY_OPERATION_BUSY: OS lease unavailable', { cause: error }); }
  return fd;
}
interface OperationLock { pid: number; start: string; nonce: string; phase: string; childPid?: number }
export function processIdentity(pid: number, readStat: (path: string, encoding: 'utf8') => string = readFileSync): string | undefined {
  if (!Number.isSafeInteger(pid) || pid < 1) throw new Error('DELIVERY_INVALID_PID');
  try {
    const stat = readStat(`/proc/${pid}/stat`, 'utf8');
    const fields = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
    return ['Z', 'X', 'x'].includes(fields[0]) ? undefined : fields[19];
  } catch (e) { if ((e as NodeJS.ErrnoException).code === 'ENOENT') return undefined; throw e; }
}
export function updateOperationChild(directory: string, phase: string, childPid?: number): void {
  const path = join(directory, 'operation.lock');
  const lock = readJson(path) as OperationLock;
  if (lock.pid !== process.pid || lock.start !== processIdentity(process.pid)) throw new Error('DELIVERY_OPERATION_OWNER_CHANGED');
  atomicJson(path, { ...lock, phase, ...(childPid ? { childPid } : {}) });
}
export function recoverOperation(directory: string, nonce: string): void {
  const lease = acquireLease(directory);
  try {
    const path = join(directory, 'operation.lock');
    if (!existsSync(path)) {
      if (nonce !== 'none') throw new Error('DELIVERY_LOCK_CHANGED');
      return;
    }
    const lock = readJson(path) as OperationLock;
    if (lock.nonce !== nonce || !/^[0-9a-f-]{36}$/.test(nonce) || !lock.start) throw new Error('DELIVERY_LOCK_CHANGED');
    if (processIdentity(lock.pid) === lock.start) throw new Error('DELIVERY_OWNER_STILL_ALIVE');
    if (lock.phase === 'starting') throw new Error('DELIVERY_CHILD_OWNERSHIP_UNCERTAIN');
    if (lock.childPid) {
      try { process.kill(-lock.childPid, 0); throw new Error('DELIVERY_CHILD_STILL_ALIVE'); }
      catch (e) { if ((e as NodeJS.ErrnoException).code !== 'ESRCH') throw e; }
    }
    renameSync(path, join(directory, `${nonce}.recovered-lock.json`));
  } finally { closeSync(lease); }
}
