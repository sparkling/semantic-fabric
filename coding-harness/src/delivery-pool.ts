// SPDX-License-Identifier: MIT
import { runBoundedPool } from '@claude-flow/cli/dist/src/services/bounded-worker-pool.js';
import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve } from 'node:path';
import { normalizeWorkspacePath } from './contracts.js';
import { atomicJson, withSynchronousOperationLock } from './delivery-workspace.js';
import { assertReservationAdmission, candidateHasOperation, saveReservations } from './delivery-cohort-custody.js';
import type { DeliveryHarness } from './delivery-runtime.js';
import type { DeliveryCandidate } from './delivery-candidate.js';

export interface DeliveryReadyCallback<T> {
  id: string; mutationPaths: string[]; resources: string[]; readPaths?: string[]; privateSnapshot?: true;
  run(signal: AbortSignal, record: (candidate: DeliveryCandidate) => void): Promise<T>;
}
export interface DeliveryPoolProgress {
  schemaVersion: 1;
  sequence: number;
  event: 'cohort-started' | 'outcome-started' | 'candidate-created' | 'outcome-settled' | 'cohort-drained';
  recordedAt: string;
  sourceDigest: string;
  evidencePath: string;
  integration: 'settled-outcome-awaits-owner-acceptance';
  taskId?: string;
  candidateRoot?: string;
  evidenceDirectory?: string;
  status?: 'fulfilled' | 'rejected' | 'cancelled';
  sourceRevalidated?: boolean;
}
const overlaps = (a: string, b: string): boolean => a === b || a.startsWith(`${b}/`) || b.startsWith(`${a}/`);
const reservation = ({ id, mutationPaths, resources, readPaths, privateSnapshot }: DeliveryReadyCallback<unknown>) =>
  ({ id, mutationPaths, resources, readPaths, ...(privateSnapshot === true ? { privateSnapshot } : {}) });

export function deliveryPoolIdentity() {
  const entry = fileURLToPath(import.meta.resolve('@claude-flow/cli/dist/src/services/bounded-worker-pool.js'));
  const installed = JSON.parse(readFileSync(resolve(dirname(entry), '../../../package.json'), 'utf8')) as { version: string };
  const lock = JSON.parse(readFileSync(new URL('../package-lock.json', import.meta.url), 'utf8')) as { packages: Record<string, { version?: string }> };
  if (installed.version !== lock.packages['node_modules/@claude-flow/cli']?.version) throw new Error('DELIVERY_POOL_INSTALLATION_MISMATCH');
  return { package: '@claude-flow/cli', version: installed.version, entry,
    sha256: createHash('sha256').update(readFileSync(entry)).digest('hex') };
}

/** Caller selects ready outcomes. Upstream executes; this adapter retains file/resource custody. */
export async function runDeliveryPool<T>(canonical: DeliveryHarness, tasks: readonly DeliveryReadyCallback<T>[],
  options: { maxConcurrency: number; signal?: AbortSignal; observe?: (event: DeliveryPoolProgress) => void }) {
  if (canonical.context.kind !== 'main') throw new Error('DELIVERY_CANONICAL_SOURCE_REQUIRED');
  if (options.signal?.aborted) throw options.signal.reason ?? new Error('DELIVERY_POOL_CANCELLED');
  if (!Number.isSafeInteger(options.maxConcurrency) || options.maxConcurrency < 1) throw new Error('DELIVERY_POOL_INVALID_CONCURRENCY');
  const ids = new Set<string>();
  for (let i = 0; i < tasks.length; i++) {
    const task = tasks[i];
    if (!task.id || ids.has(task.id)) throw new Error('DELIVERY_POOL_DUPLICATE_ID');
    ids.add(task.id);
    for (const path of task.mutationPaths) normalizeWorkspacePath(path, 'pool mutation path');
    for (const path of task.readPaths ?? []) normalizeWorkspacePath(path, 'pool read path');
    if (task.resources.some(value => !value.trim() || value.includes('\0'))) throw new Error('DELIVERY_POOL_INVALID_RESOURCE');
    for (const previous of tasks.slice(0, i)) if (task.mutationPaths.some(path => previous.mutationPaths.some(other => overlaps(path, other)))
      || task.resources.some(resource => previous.resources.includes(resource))) throw new Error('DELIVERY_POOL_RESOURCE_CONFLICT');
  }
  const admission = withSynchronousOperationLock(canonical.directory, () => {
    if (canonical.inspect().active !== null) throw new Error('DELIVERY_WRITER_ALREADY_CLAIMED');
    assertReservationAdmission(canonical, tasks);
    const source = canonical.snapshot();
    const progressDirectory = mkdtempSync(join(canonical.directory, 'pool-'));
    saveReservations(progressDirectory, tasks.map(reservation));
    return { source, progressDirectory };
  });
    const { source: baseline, progressDirectory } = admission;
    const source = baseline.digest, started: Promise<unknown>[] = [];
    const evidence = new Map<string, { candidateRoot: string; evidenceDirectory: string }>();
    const settled = new Set<string>();
    const retained = new Set<string>();
    const settle = (id: string) => {
      const candidate = evidence.get(id);
      if (candidate && candidateHasOperation(candidate.evidenceDirectory)) retained.add(id);
      settled.add(id);
      saveReservations(progressDirectory, tasks.filter(task => !settled.has(task.id) || retained.has(task.id))
        .map(task => ({ ...reservation(task), ...(retained.has(task.id) ? { retainedDirectory: evidence.get(task.id)!.evidenceDirectory } : {}) })));
    };
    let sequence = 0;
    const report = (event: DeliveryPoolProgress['event'], detail: Pick<DeliveryPoolProgress,
      'taskId' | 'candidateRoot' | 'evidenceDirectory' | 'status' | 'sourceRevalidated'> = {}) => {
      const evidencePath = join(progressDirectory, `${String(++sequence).padStart(6, '0')}.json`);
      const value: DeliveryPoolProgress = { schemaVersion: 1, sequence, event, recordedAt: new Date().toISOString(),
        sourceDigest: source, evidencePath, integration: 'settled-outcome-awaits-owner-acceptance', ...detail };
      atomicJson(evidencePath, value);
      // Observers receive a copy; progress consumers cannot rewrite durable execution evidence.
      try { void Promise.resolve(options.observe?.(structuredClone(value))).catch(() => {}); }
      catch { /* Observability is not acceptance authority. */ }
    };
    const began = performance.now();
    report('cohort-started');
    try {
      const result = await runBoundedPool(tasks.map(task => ({ id: task.id, run: (signal: AbortSignal) => {
        if (signal.aborted || options.signal?.aborted) return Promise.reject(signal.reason ?? options.signal?.reason);
        const work = Promise.resolve().then(() => {
          if (signal.aborted || options.signal?.aborted) throw signal.reason ?? options.signal?.reason ?? new Error('DELIVERY_POOL_CANCELLED');
          report('outcome-started', { taskId: task.id });
          return task.run(signal, candidate => {
          if (candidate.harness.context.kind !== 'candidate' || evidence.has(task.id)) throw new Error('DELIVERY_POOL_CANDIDATE_IDENTITY');
          evidence.set(task.id, { candidateRoot: candidate.harness.root, evidenceDirectory: candidate.harness.directory });
          report('candidate-created', { taskId: task.id, ...evidence.get(task.id) });
          });
        }).then(value => {
          settle(task.id);
          report('outcome-settled', { taskId: task.id, ...evidence.get(task.id),
            status: signal.aborted || options.signal?.aborted ? 'cancelled' : 'fulfilled' });
          return value;
        }, error => {
          settle(task.id);
          report('outcome-settled', { taskId: task.id, ...evidence.get(task.id),
            status: signal.aborted || options.signal?.aborted ? 'cancelled' : 'rejected' });
          throw error;
        });
        started.push(work); return work;
      } })), options);
      await Promise.allSettled(started);
      for (const row of result.results) if (!settled.has(row.id)) {
        settle(row.id);
        report('outcome-settled', { taskId: row.id, ...evidence.get(row.id), status: row.status });
      }
      // Private completion is not canonical acceptance; integration revalidates canonical inputs.
      report('cohort-drained', { sourceRevalidated: false });
      return { ...result, results: result.results.map(result => ({ ...result, ...evidence.get(result.id) })),
        durationMs: performance.now() - began, progressDirectory };
    } catch (error) {
      await Promise.allSettled(started);
      // Preserve the original failure if snapshot or evidence storage also fails.
      try {
        report('cohort-drained', { sourceRevalidated: false });
      } catch { /* Original error wins. */ }
      throw error;
    } finally {
      // Upstream cancellation may return before a noncooperative callback terminates.
      await Promise.allSettled(started);
    }
}
