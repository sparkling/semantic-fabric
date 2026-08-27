// SPDX-License-Identifier: MIT

import type { ArchitectureEvidence } from './candidate-types.js';
import type { AgenticQeEvidence, AgenticQeProfile } from './evidence.js';
import { receiptArtifactKey } from './programme-receipt-keys.js';
import type { GitIdentity } from './receipts.js';

export function assertSameIdentity(
  left: GitIdentity,
  right: GitIdentity,
  message: string,
): void {
  if (left.commit !== right.commit || left.tree !== right.tree) throw new Error(message);
}

export function assertNonEmptyRecord(
  value: Readonly<Record<string, string>>,
  error: string,
): void {
  if (Object.keys(value).length === 0) throw new Error(error);
}

export function assertProtectedInputSet(
  prepared: Readonly<Record<string, string>>,
  expected: Readonly<Record<string, string>>,
): void {
  assertNonEmptyRecord(prepared, 'HARNESS_PROTECTED_INPUTS_REQUIRED');
  const actualPaths = Object.keys(prepared).sort();
  const expectedPaths = Object.keys(expected).sort();
  if (JSON.stringify(actualPaths) !== JSON.stringify(expectedPaths)
    || actualPaths.some((path) => prepared[path] !== expected[path])) {
    throw new Error('HARNESS_PROTECTED_INPUT_SET_MISMATCH');
  }
}

export function assertDualHostArchitecture(architecture: ArchitectureEvidence): void {
  const hosts = new Set(architecture.invocations.map(({ host }) => host));
  const ids = architecture.invocations.map(({ invocationId }) => invocationId);
  if (architecture.invocations.length < 2
    || hosts.size !== 2 || !hosts.has('codex') || !hosts.has('claude-code')
    || new Set(ids).size !== ids.length) {
    throw new Error('HARNESS_NATIVE_DUAL_HOST_ARCHITECTURE_REQUIRED');
  }
}

export function assertRequiredQeProfiles(
  evidence: readonly AgenticQeEvidence[],
  required: readonly AgenticQeProfile[],
): void {
  if (required.length === 0 || new Set(required).size !== required.length) {
    throw new Error('HARNESS_REQUIRED_QE_PROFILES_INVALID');
  }
  const actual = new Set(evidence.map(({ profile }) => profile));
  const missing = required.filter((profile) => !actual.has(profile));
  if (missing.length > 0) {
    throw new Error(`HARNESS_REQUIRED_QE_PROFILES_MISSING:${missing.join(',')}`);
  }
  const extra = [...actual].filter((profile) => !required.includes(profile));
  if (extra.length > 0) {
    throw new Error(`HARNESS_REQUIRED_QE_PROFILES_EXTRA:${extra.join(',')}`);
  }
  const actualOrder = evidence.map(({ profile }) => profile);
  if (JSON.stringify(actualOrder) !== JSON.stringify(required)) {
    throw new Error('HARNESS_REQUIRED_QE_PROFILE_ORDER_MISMATCH');
  }
}

export function prefixArtifacts(
  artifacts: Readonly<Record<string, string>>,
  repairCount: number,
): Record<string, string> {
  return Object.fromEntries(Object.entries(artifacts).map(([name, digest]) => [
    receiptArtifactKey(repairCount, name),
    digest,
  ]));
}

export function runtimeTrustUnavailable(error: unknown): boolean {
  return error instanceof Error
    && error.message.includes('HARNESS_NATIVE_TRUSTED_RUNTIME_UNAVAILABLE');
}
