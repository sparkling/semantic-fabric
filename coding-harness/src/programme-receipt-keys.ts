// SPDX-License-Identifier: MIT

import { parseTaskEvidenceId, parseTaskOpaqueId } from './acceptance-task-v3.js';
import { asInteger, asNonEmptyString, normalizeWorkspacePath } from './contracts.js';
import type { VerifierStage } from './candidate-types.js';
import { AGENTIC_QE_PROFILES, type AgenticQeProfile } from './qe-profile.js';

export const RED_BASELINE_RECEIPT_KEY = 'red-baseline' as const;

const VERIFIER_STAGES = new Set<VerifierStage>([
  'public', 'independent', 'regression',
]);

export function receiptArtifactKey(attempt: number, path: string): string {
  return `${attemptPrefix(attempt)}${normalizeWorkspacePath(path, 'receipt artifact path')}`;
}

export function receiptVerifierKey(attempt: number, stage: VerifierStage): string {
  if (!VERIFIER_STAGES.has(stage)) throw new TypeError('receipt verifier stage is invalid');
  return `${attemptPrefix(attempt)}${stage}`;
}

export function receiptGeneratedOutputKey(
  attempt: number,
  stage: VerifierStage,
  evidenceId: string,
): string {
  const id = parseTaskEvidenceId(evidenceId, 'receipt generated-output evidenceId');
  return `${receiptVerifierKey(attempt, stage)}:generated:${id}`;
}

export function receiptMutationKey(attempt: number, mutationId: string): string {
  const id = parseTaskOpaqueId(mutationId, 'receipt mutationId');
  return `${attemptPrefix(attempt)}mutation:${id}`;
}

export function receiptMutationEvidenceKey(attempt: number, name: string): string {
  const evidenceName = asNonEmptyString(name, 'receipt mutation evidence name');
  if (evidenceName === 'mutation') return `${attemptPrefix(attempt)}mutation`;
  if (!evidenceName.startsWith('mutation:')) {
    throw new TypeError('receipt mutation evidence name is invalid');
  }
  return receiptMutationKey(attempt, evidenceName.slice('mutation:'.length));
}

export function receiptQeKey(attempt: number, profile: AgenticQeProfile): string {
  if (!AGENTIC_QE_PROFILES.includes(profile)) throw new TypeError('receipt QE profile is invalid');
  return `${attemptPrefix(attempt)}qe:${profile}`;
}

function attemptPrefix(attempt: number): string {
  return `attempt-${asInteger(attempt, 'receipt attempt')}:`;
}
