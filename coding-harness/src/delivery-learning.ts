// SPDX-License-Identifier: MIT
import { existsSync, linkSync, mkdirSync, readdirSync, unlinkSync, writeFileSync } from 'node:fs';
import { randomUUID } from 'node:crypto';
import { join } from 'node:path';
import { hash } from '@metaharness/harness';
import { PersistentRoutedAgentPool, VerifiedRoutingHistory, type RoutingObservation, type NativeModelCandidate } from './models/routing.js';
import type { DeliveryRoute, DeliveryTask } from './delivery-contracts.js';
import { atomicJson, readJson, withOperationLock } from './delivery-workspace.js';

interface Delta { schemaVersion: 1; runId: string; receiptDigest: string; observations: RoutingObservation[]; digest: string }
const reducers = new Map<string, Promise<unknown>>();
async function serialReduce<T>(directory: string, action: () => Promise<T>): Promise<T> {
  const previous = reducers.get(directory) ?? Promise.resolve();
  const current = previous.catch(() => {}).then(() => withOperationLock(directory, action));
  reducers.set(directory, current);
  try { return await current; } finally { if (reducers.get(directory) === current) reducers.delete(directory); }
}
export const nativeRouteId = (route: DeliveryRoute): string => `${route.host}:${route.model}:${route.effort}`;

/** Single-writer reduction over immutable native-only deltas; hybrid runs never call record. */
export async function openDeliveryLearning(directory: string, runId: string, task: DeliveryTask, selected: DeliveryRoute) {
  mkdirSync(directory, { recursive: true, mode: 0o700 });
  const historyFile = join(directory, 'native-routing.json'), deltaRoot = join(directory, 'deltas');
  mkdirSync(deltaRoot, { recursive: true, mode: 0o700 });
  const reduce = () => serialReduce(directory, async () => {
    const rows = new Map<string, RoutingObservation>();
    for (const name of readdirSync(deltaRoot).sort().filter(name => /^[a-zA-Z0-9_-]+\.json$/.test(name))) {
      const value = readJson(join(deltaRoot, name)) as Delta;
      const { digest, ...body } = value;
      if (body.schemaVersion !== 1 || hash(body) !== digest || !/^[a-f0-9]{64}$/.test(body.receiptDigest)
        || !Array.isArray(body.observations) || body.observations.some(row => !/^(codex|claude-code):/.test(row.candidateId))) throw new Error('DELIVERY_LEARNING_DELTA_INVALID');
      const verified = new VerifiedRoutingHistory(body.observations).snapshot().observations;
      for (const row of verified) {
        const key = `${row.runId}:${row.stepKind}`;
        if (rows.has(key)) throw new Error('DELIVERY_LEARNING_DUPLICATE');
        rows.set(key, row);
      }
    }
    const snapshot = { schemaVersion: 1, observations: [...rows.values()] };
    atomicJson(historyFile, { ...snapshot, digest: hash(snapshot) });
    return snapshot.observations;
  });
  const history = new VerifiedRoutingHistory(await reduce());
  const makePool = (routes: DeliveryRoute[], stage: 'architecture' | 'implementation' | 'repair') => {
    const candidates: NativeModelCandidate[] = routes.map(route => {
      if (route.host === 'openrouter') throw new Error('DELIVERY_HYBRID_LEARNING_REFUSED');
      return { id: nativeRouteId(route), host: route.host, model: route.model, handles: [stage],
      ...(route.effort === 'default' ? {} : { reasoningEffort: route.effort }), run: async () => { throw new Error('DELIVERY_ROUTING_ONLY'); } };
    });
    return new PersistentRoutedAgentPool({ runId, task: { id: task.id, digest: hash(task), prompt: task.requirement,
      tags: [task.taskClass], difficulty: 0.5 }, candidates, history,
      embedder: { dimensions: 4, embed: text => {
        const digest = hash(text); return [0, 1, 2, 3].map(index => parseInt(digest.slice(index * 8, index * 8 + 8), 16) / 0xffffffff);
      } } });
  };
  const author = selected.host === 'openrouter' ? undefined : makePool([selected], 'implementation');
  author?.select('implementation');
  return {
    summary: () => ({ snapshotDigest: hash(history.snapshot()), observations: history.snapshot().observations.length,
      ...(author ? { routing: author.routeSnapshot() } : {}), authority: 'native-verifier-observations-only' }),
    selectCreditFallback(): DeliveryRoute {
      const routes: DeliveryRoute[] = [{ host: 'claude-code', model: 'cc/claude-sonnet-5[1m]', effort: 'medium' },
        { host: 'codex', model: 'gpt-5.6-sol', effort: 'medium' }];
      const chosen = makePool(routes, 'implementation').select('implementation');
      return routes.find(route => nativeRouteId(route) === chosen.id)!;
    },
    async record(route: DeliveryRoute, stage: 'implementation' | 'repair', latencyMs: number, receiptDigest: string) {
      if (task.host === 'openrouter') throw new Error('DELIVERY_HYBRID_LEARNING_REFUSED');
      const pool = makePool([route], stage); pool.select(stage);
      const observation = pool.recordVerified(stage, { source: 'deterministic-verifier', quality: 1, accepted: true,
        infrastructureFailure: false, latencyMs: Math.round(latencyMs) });
      const body = { schemaVersion: 1 as const, runId, receiptDigest, observations: [observation] };
      const path = join(deltaRoot, `${runId}.json`);
      if (existsSync(path)) throw new Error('DELIVERY_LEARNING_DELTA_EXISTS');
      const temporary = join(deltaRoot, `${randomUUID()}.pending`);
      writeFileSync(temporary, JSON.stringify({ ...body, digest: hash(body) }), { flag: 'wx', mode: 0o600 });
      try { linkSync(temporary, path); } finally { unlinkSync(temporary); }
      await reduce();
    },
  };
}
