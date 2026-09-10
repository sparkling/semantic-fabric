// SPDX-License-Identifier: MIT
import type { RunResult } from '@metaharness/harness';
import { asRecord, assertExactKeys } from './contracts.js';
import { executorIdentity, nonempty, route, type DeliveryRoute, type NativeHandoff } from './delivery-contracts.js';

export interface NativeStageRequest {
  schemaVersion: 1; id: string; taskId: string; thread: string; baseCommit: string;
  stage: 'implementation' | 'review'; attempt: number; sourceDigest: string;
  route: DeliveryRoute; executorId?: string; prerequisiteDigests: string[];
  evidenceDigest: string; repair: boolean;
  requirement: string; scope: string[]; feedback: string[];
}
export interface NativeStageResponse {
  schemaVersion: 1; requestId: string; sourceDigest: string; native: NativeHandoff;
  outcome: 'completed' | 'changes-requested' | 'unavailable' | 'cancelled';
  summary: string; issues: string[];
}
export interface NativeStageResult {
  request: NativeStageRequest; response: NativeStageResponse; kernel: RunResult;
  accepted: boolean; reasons: string[];
}
export interface DeliveryWorkflow {
  requests: NativeStageRequest[]; results: NativeStageResult[]; invalidated: string[];
}
export type DeliveryAction = { kind: 'native'; request: NativeStageRequest }
  | { kind: 'check'; checkId: string }
  | { kind: 'ready-to-commit'; sourceDigest: string }
  | { kind: 'paused'; reason: string };

export function parseStageResponse(value: unknown): NativeStageResponse {
  const r = asRecord(value, 'native response');
  assertExactKeys(r, ['schemaVersion', 'requestId', 'sourceDigest', 'native', 'outcome', 'summary', 'issues'], 'native response');
  if (r.schemaVersion !== 1 || !['completed', 'changes-requested', 'unavailable', 'cancelled'].includes(String(r.outcome))) {
    throw new Error('DELIVERY_INVALID_NATIVE_RESPONSE');
  }
  for (const field of ['requestId', 'sourceDigest']) if (!/^[a-f0-9]{64}$/.test(String(r[field]))) {
    throw new Error(`DELIVERY_INVALID_RESPONSE_DIGEST:${field}`);
  }
  const n = asRecord(r.native, 'native identity');
  assertExactKeys(n, ['host', 'model', 'effort', 'executorId', 'authentication', 'observation'], 'native identity');
  if (n.authentication !== 'native-subscription') throw new Error('DELIVERY_NATIVE_SUBSCRIPTION_REQUIRED');
  if (!Array.isArray(r.issues)) throw new Error('DELIVERY_INVALID_REVIEW_ISSUES');
  const issues = r.issues.map(i => nonempty(i, 'issue'));
  if ((r.outcome === 'completed' && issues.length) || (r.outcome === 'changes-requested' && !issues.length)) {
    throw new Error('DELIVERY_INCONSISTENT_REVIEW_OUTCOME');
  }
  return { schemaVersion: 1, requestId: String(r.requestId), sourceDigest: String(r.sourceDigest),
    native: { ...route({ host: n.host, model: n.model, effort: n.effort }),
      executorId: executorIdentity(n.executorId), authentication: n.authentication,
      observation: nonempty(n.observation, 'native observation') },
    outcome: r.outcome as NativeStageResponse['outcome'], summary: nonempty(r.summary, 'summary'), issues };
}
