// SPDX-License-Identifier: MIT
import type { RunResult } from '@metaharness/harness';
import { asRecord, assertExactKeys, normalizeWorkspacePath } from './contracts.js';
import { nonempty, parseDeliveryHandoff, type DeliveryRoute, type NativeHandoff } from './delivery-contracts.js';
import type { DeliveryApiEvidence } from './delivery-api.js';

export interface NativeStageRequest {
  schemaVersion: 1; id: string; taskId: string; thread: string; baseCommit: string;
  stage: 'implementation' | 'review'; attempt: number; sourceDigest: string;
  route: DeliveryRoute; executorId?: string; prerequisiteDigests: string[];
  evidenceDigest: string; repair: boolean;
  requirement: string; scope: string[]; feedback: string[];
  failedApi?: DeliveryApiEvidence;
}
export interface NativeStageResponse {
  schemaVersion: 1; requestId: string; sourceDigest: string; native: NativeHandoff;
  outcome: 'completed' | 'changes-requested' | 'unavailable' | 'cancelled';
  summary: string; issues: string[];
  metering?: { costUsd: number; latencyMs: number; evidenceDigest: string };
  rootApplication?: { proposalDigest: string; sourceBefore: string; sourceAfter: string; rootChangedPaths: string[] };
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
  assertExactKeys(r, ['schemaVersion', 'requestId', 'sourceDigest', 'native', 'outcome', 'summary', 'issues',
    ...('metering' in r ? ['metering'] : []), ...('rootApplication' in r ? ['rootApplication'] : [])], 'native response');
  if (r.schemaVersion !== 1 || !['completed', 'changes-requested', 'unavailable', 'cancelled'].includes(String(r.outcome))) {
    throw new Error('DELIVERY_INVALID_NATIVE_RESPONSE');
  }
  for (const field of ['requestId', 'sourceDigest']) if (!/^[a-f0-9]{64}$/.test(String(r[field]))) {
    throw new Error(`DELIVERY_INVALID_RESPONSE_DIGEST:${field}`);
  }
  const native = parseDeliveryHandoff(r.native);
  let metering: NativeStageResponse['metering'];
  if (r.metering !== undefined) {
    const value = asRecord(r.metering, 'metering');
    assertExactKeys(value, ['costUsd', 'latencyMs', 'evidenceDigest'], 'metering');
    if (typeof value.costUsd !== 'number' || !Number.isFinite(value.costUsd) || value.costUsd < 0
      || typeof value.latencyMs !== 'number' || !Number.isFinite(value.latencyMs) || value.latencyMs < 0
      || typeof value.evidenceDigest !== 'string' || !/^[a-f0-9]{64}$/.test(value.evidenceDigest)) throw new Error('DELIVERY_INVALID_METERING');
    metering = { costUsd: value.costUsd, latencyMs: value.latencyMs, evidenceDigest: value.evidenceDigest };
  }
  if (native.host === 'openrouter' && !metering) throw new Error('DELIVERY_API_METERING_REQUIRED');
  let rootApplication: NativeStageResponse['rootApplication'];
  if (r.rootApplication !== undefined) {
    const value = asRecord(r.rootApplication, 'root application');
    assertExactKeys(value, ['proposalDigest', 'sourceBefore', 'sourceAfter', 'rootChangedPaths'], 'root application');
    for (const key of ['proposalDigest', 'sourceBefore', 'sourceAfter']) if (!/^[a-f0-9]{64}$/.test(String(value[key]))) throw new Error('DELIVERY_INVALID_ROOT_APPLICATION');
    if (value.sourceAfter !== r.sourceDigest || !Array.isArray(value.rootChangedPaths)) throw new Error('DELIVERY_INVALID_ROOT_APPLICATION');
    rootApplication = { proposalDigest: String(value.proposalDigest), sourceBefore: String(value.sourceBefore), sourceAfter: String(value.sourceAfter),
      rootChangedPaths: value.rootChangedPaths.map(path => normalizeWorkspacePath(nonempty(path, 'root change'), 'root change')) };
  }
  if (!Array.isArray(r.issues)) throw new Error('DELIVERY_INVALID_REVIEW_ISSUES');
  const issues = r.issues.map(i => nonempty(i, 'issue'));
  if ((r.outcome === 'completed' && issues.length) || (r.outcome === 'changes-requested' && !issues.length)) {
    throw new Error('DELIVERY_INCONSISTENT_REVIEW_OUTCOME');
  }
  return { schemaVersion: 1, requestId: String(r.requestId), sourceDigest: String(r.sourceDigest),
    native, ...(metering ? { metering } : {}), ...(rootApplication ? { rootApplication } : {}),
    outcome: r.outcome as NativeStageResponse['outcome'], summary: nonempty(r.summary, 'summary'), issues };
}
