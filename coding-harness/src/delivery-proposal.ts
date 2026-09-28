// SPDX-License-Identifier: MIT
import { createHash } from 'node:crypto';
import { basename, dirname, join, resolve } from 'node:path';
import { hash } from '@metaharness/harness';
import { asRecord, assertExactKeys } from './contracts.js';
import { deliveryApiDigest, parseDeliveryChanges, type DeliveryApiEvidence } from './delivery-api.js';
import { readJson } from './delivery-workspace.js';
import { parseStageResponse, type NativeStageResponse } from './delivery-workflow-contracts.js';
import type { DeliveryHarness } from './delivery-runtime.js';

/** Root verifies applied bytes, then binds its actual source observation to the
 * original paid proposal. This does not edit source or invoke another model. */
export function bindAppliedProposal(harness: DeliveryHarness, id: string, input: unknown): NativeStageResponse {
  const proposal = asRecord(input, 'stored proposal');
  assertExactKeys(proposal, ['response', 'changes', 'evidence', 'evidencePath', 'sourceBefore'], 'stored proposal');
  const response = parseStageResponse(proposal.response), run = harness.read(id);
  const request = run.workflow?.requests.find(request => request.id === response.requestId);
  if (!request || request.route.host !== 'openrouter') throw new Error('DELIVERY_PENDING_API_REQUEST_REQUIRED');
  const path = resolve(String(proposal.evidencePath));
  if (dirname(path) !== join(harness.directory, 'api') || !/^request-[a-f0-9]{64}\.json$/.test(basename(path))) throw new Error('DELIVERY_API_EVIDENCE_PATH');
  const evidence = readJson(path) as DeliveryApiEvidence;
  if (deliveryApiDigest(evidence) !== deliveryApiDigest(proposal.evidence)
    || response.metering?.evidenceDigest !== deliveryApiDigest(evidence)
    || response.metering?.costUsd !== evidence.actualUsd || response.sourceDigest !== request.sourceDigest
    || evidence.status !== 'completed-valid-output' || evidence.stageRequestId !== request.id
    || evidence.taskDigest !== hash(run.task) || evidence.requestedModel !== request.route.model) throw new Error('DELIVERY_API_EVIDENCE_MISMATCH');
  const changes = parseDeliveryChanges(proposal.changes, request.scope, request.stage === 'review');
  if (evidence.proposalDigest !== deliveryApiDigest({ outcome: response.outcome, summary: response.summary, issues: response.issues, changes })) throw new Error('DELIVERY_PROPOSAL_CHANGED');
  const before = asRecord(proposal.sourceBefore, 'proposal source');
  assertExactKeys(before, ['digest', 'files'], 'proposal source');
  const beforeFiles = asRecord(before.files, 'proposal files');
  if (before.digest !== request.sourceDigest || hash(beforeFiles) !== before.digest) throw new Error('DELIVERY_PROPOSAL_SOURCE_MISMATCH');
  const current = harness.snapshot();
  for (const change of changes) {
    const expected = createHash('sha256').update(change.content).digest('hex');
    if (current.files[change.path]?.split(':')[1] !== expected) throw new Error('DELIVERY_PROPOSAL_NOT_APPLIED');
  }
  const proposed = new Set(changes.map(change => change.path));
  const rootChangedPaths = [...new Set([...Object.keys(beforeFiles), ...Object.keys(current.files)])]
    .filter(path => beforeFiles[path] !== current.files[path] && (!proposed.has(path)
      || String(beforeFiles[path]).split(':')[0] !== current.files[path]?.split(':')[0])).sort();
  if (rootChangedPaths.some(path => !request.scope.includes(path))
    || (request.stage === 'review' && current.digest !== request.sourceDigest)) throw new Error('DELIVERY_PROPOSAL_SOURCE_MISMATCH');
  return parseStageResponse({ ...response, sourceDigest: current.digest, rootApplication: {
    proposalDigest: evidence.proposalDigest, sourceBefore: request.sourceDigest, sourceAfter: current.digest, rootChangedPaths,
  } });
}
