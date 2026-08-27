// SPDX-License-Identifier: MIT

import {
  SHA256_PATTERN,
  asRecord,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import {
  bindNativeRuntimeEvidence,
  type NativeInvocationExpectation,
  type NativeRuntimeEvidence,
} from './evidence.js';
import type { HostEvidence } from './receipts.js';

export interface NativeRuntimeEvidenceV2 extends Omit<
  NativeRuntimeEvidence,
  'schemaVersion' | 'invocations'
> {
  readonly schemaVersion: 2;
  readonly invocations: ReadonlyArray<
    NativeRuntimeEvidence['invocations'][number] & Readonly<{
      patchPayloadSha256: string | null;
    }>
  >;
}

const TOP_LEVEL_KEYS = [
  'schemaVersion', 'source', 'taskId', 'runId', 'hosts', 'invocations',
] as const;
const INVOCATION_KEYS = [
  'invocationId', 'host', 'model', 'operation', 'candidateTree', 'environmentDigest',
  'outputDigest', 'patchPayloadSha256', 'exitCode', 'network', 'filesystem', 'resources',
] as const;

export function bindNativeRuntimeEvidenceV2(input: Readonly<{
  value: unknown;
  taskId: string;
  runId: string;
  hosts: readonly HostEvidence[];
  expectations: readonly NativeInvocationExpectation[];
}>): NativeRuntimeEvidenceV2 {
  const value = asRecord(input.value, 'native runtime evidence V2');
  assertExactKeys(value, TOP_LEVEL_KEYS, 'native runtime evidence V2');
  if (value.schemaVersion !== 2 || value.source !== 'trusted-native-runtime') {
    throw new TypeError('native runtime evidence V2 provenance is invalid');
  }
  if (!Array.isArray(value.invocations) || value.invocations.length === 0) {
    throw new TypeError('native runtime evidence V2 invocations must be a non-empty array');
  }
  const patchDigests = new Map<string, string | null>();
  const projectedInvocations = value.invocations.map((entry, index) => {
    const label = `native runtime evidence V2.invocations[${index}]`;
    const invocation = asRecord(entry, label);
    assertExactKeys(invocation, INVOCATION_KEYS, label);
    const digest = patchDigestForOperation(
      invocation.operation,
      invocation.patchPayloadSha256,
      `${label}.patchPayloadSha256`,
    );
    const invocationId = invocation.invocationId;
    if (typeof invocationId !== 'string' || invocationId.trim() === '') {
      throw new TypeError(`${label}.invocationId must be a non-empty string`);
    }
    patchDigests.set(invocationId, digest);
    const projected = { ...invocation };
    delete projected.patchPayloadSha256;
    return projected;
  });
  const boundV1 = bindNativeRuntimeEvidence({
    ...input,
    value: {
      ...value,
      schemaVersion: 1,
      invocations: projectedInvocations,
    },
  });
  const expectations = new Map(input.expectations.map((entry) => [entry.invocationId, entry]));
  const invocations = boundV1.invocations.map((invocation) => {
    const patchPayloadSha256 = patchDigests.get(invocation.invocationId);
    const expected = expectations.get(invocation.invocationId);
    if (patchPayloadSha256 === undefined || expected === undefined) {
      throw new Error('HARNESS_NATIVE_INVOCATION_SET_MISMATCH');
    }
    const patchOperation = invocation.operation === 'implementation'
      || invocation.operation === 'repair';
    if (patchOperation) {
      if (expected.patchPayloadSha256 !== patchPayloadSha256) {
        throw new Error('HARNESS_NATIVE_PATCH_PAYLOAD_BINDING_MISMATCH');
      }
    } else if (expected.patchPayloadSha256 !== undefined
      && expected.patchPayloadSha256 !== null) {
      throw new Error('HARNESS_NATIVE_PATCH_PAYLOAD_UNEXPECTED');
    }
    return deepFreeze({ ...invocation, patchPayloadSha256 });
  });
  return deepFreeze({
    ...boundV1,
    schemaVersion: 2,
    invocations,
  });
}

function patchDigestForOperation(
  operation: unknown,
  value: unknown,
  label: string,
): string | null {
  const patchOperation = operation === 'implementation' || operation === 'repair';
  if (!patchOperation) {
    if (value !== null) throw new TypeError(`${label} must be null for non-patch operations`);
    return null;
  }
  if (typeof value !== 'string'
    || !SHA256_PATTERN.test(value)
    || value === '0'.repeat(64)) {
    throw new TypeError(`${label} must be a non-genesis SHA-256 digest`);
  }
  return value;
}
