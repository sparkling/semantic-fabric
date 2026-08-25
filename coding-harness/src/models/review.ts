// SPDX-License-Identifier: MIT

import type { NativeModelCandidate } from './routing.js';
import type { NativeHost } from './types.js';

const REVIEW_HOST_ORDER = Object.freeze([
  'codex',
  'claude-code',
] as const satisfies readonly NativeHost[]);

export interface IndependentReviewEvidence {
  readonly host: NativeHost;
  readonly invocationId: string;
  readonly accepted: boolean;
}

export function requireDistinctHostProposal(
  primary: NativeModelCandidate,
  candidates: readonly NativeModelCandidate[],
): NativeModelCandidate {
  const selected = candidates
    .filter(
      ({ id, host, handles }) =>
        id !== primary.id &&
        host !== primary.host &&
        handles.includes('architecture'),
    )
    .sort((left, right) => left.id.localeCompare(right.id))[0];
  if (selected === undefined) {
    throw new Error('HARNESS_DISTINCT_HOST_PROPOSAL_UNAVAILABLE');
  }
  return selected;
}

export function requireCrossVendorReviewers(
  candidates: readonly NativeModelCandidate[],
): readonly [NativeModelCandidate, NativeModelCandidate] {
  const selected = REVIEW_HOST_ORDER.map((host) => {
    const candidate = candidates
      .filter(
        (entry) => entry.host === host && entry.handles.includes('review'),
      )
      .sort((left, right) => left.id.localeCompare(right.id))[0];
    if (candidate === undefined) {
      throw new Error(`HARNESS_REVIEW_HOST_UNAVAILABLE:${host}`);
    }
    return candidate;
  });
  return Object.freeze(selected) as readonly [
    NativeModelCandidate,
    NativeModelCandidate,
  ];
}

export function assertIndependentReviewEvidence(
  authorInvocationId: string,
  evidence: readonly IndependentReviewEvidence[],
): void {
  if (authorInvocationId.length === 0) {
    throw new Error('HARNESS_REVIEW_AUTHOR_INVOCATION_INVALID');
  }
  const invocationIds = new Set<string>();
  for (const item of evidence) {
    if (
      item.invocationId.length === 0 ||
      item.invocationId === authorInvocationId ||
      invocationIds.has(item.invocationId)
    ) {
      throw new Error('HARNESS_REVIEW_NOT_INDEPENDENT');
    }
    invocationIds.add(item.invocationId);
  }
  for (const host of REVIEW_HOST_ORDER) {
    if (evidence.filter((item) => item.host === host).length !== 1) {
      throw new Error(`HARNESS_REVIEW_EVIDENCE_MISSING:${host}`);
    }
  }
  if (evidence.length !== REVIEW_HOST_ORDER.length) {
    throw new Error('HARNESS_REVIEW_EVIDENCE_CARDINALITY_INVALID');
  }
}
