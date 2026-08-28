// SPDX-License-Identifier: MIT

import {
  DEVELOPMENT_AUTHORITY,
  SHA256_PATTERN,
  asRecord,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import {
  createProgrammeGateContractV2,
  parseProgrammeGateContractV2,
  type ProgrammeGateContractV2,
} from './programme-gate-contract-v2.js';
import {
  verifyFrozenProgrammePolicyV1,
  type FrozenProgrammePolicyV1,
  type ParsedProgrammePolicyV1,
} from './programme-policy-v5.js';
import { digestValue } from './receipts.js';

export const PROGRAMME_POLICY_V6_ID = 'semantic-fabric-programme-v6-policy-v2' as const;

export interface FrozenProgrammePolicyV2 {
  readonly schemaVersion: 2;
  readonly policyId: typeof PROGRAMME_POLICY_V6_ID;
  readonly authority: typeof DEVELOPMENT_AUTHORITY;
  readonly basePolicy: FrozenProgrammePolicyV1;
  readonly basePolicyFingerprint: string;
  readonly gateContract: ProgrammeGateContractV2;
}

export interface ParsedProgrammePolicyV2 {
  readonly snapshot: FrozenProgrammePolicyV2;
  readonly fingerprint: string;
  readonly base: ParsedProgrammePolicyV1;
}

const POLICY_KEYS = [
  'schemaVersion', 'policyId', 'authority', 'basePolicy', 'basePolicyFingerprint',
  'gateContract',
] as const;

export function createFrozenProgrammePolicyV2(
  basePolicy: FrozenProgrammePolicyV1,
  expectedBasePolicyFingerprint: string,
): FrozenProgrammePolicyV2 {
  const base = verifyFrozenProgrammePolicyV1(basePolicy, expectedBasePolicyFingerprint);
  return canonicalize({
    schemaVersion: 2,
    policyId: PROGRAMME_POLICY_V6_ID,
    authority: DEVELOPMENT_AUTHORITY,
    basePolicy: base.snapshot,
    basePolicyFingerprint: base.fingerprint,
    gateContract: createProgrammeGateContractV2(base.snapshot.gateContract),
  }).snapshot;
}

export function verifyFrozenProgrammePolicyV2(
  value: unknown,
  expectedFingerprint: string,
): ParsedProgrammePolicyV2 {
  const parsed = canonicalize(value);
  if (parseDigest(expectedFingerprint) !== parsed.fingerprint) {
    throw new Error('HARNESS_PROGRAMME_POLICY_V2_FINGERPRINT_MISMATCH');
  }
  return parsed;
}

export function programmePolicyV2Fingerprint(value: unknown): string {
  return canonicalize(value).fingerprint;
}

function canonicalize(value: unknown): ParsedProgrammePolicyV2 {
  const input = asRecord(value, 'programme policy V2');
  assertExactKeys(input, POLICY_KEYS, 'programme policy V2');
  if (input.schemaVersion !== 2 || input.policyId !== PROGRAMME_POLICY_V6_ID
    || input.authority !== DEVELOPMENT_AUTHORITY) {
    throw new TypeError('HARNESS_PROGRAMME_POLICY_V2_IDENTITY_INVALID');
  }
  const basePolicyFingerprint = parseDigest(input.basePolicyFingerprint);
  const base = verifyFrozenProgrammePolicyV1(input.basePolicy, basePolicyFingerprint);
  const gateContract = parseProgrammeGateContractV2(input.gateContract);
  if (gateContract.baseGateContractDigest !== digestValue(base.snapshot.gateContract)
    || digestValue(gateContract.baseGateContract) !== digestValue(base.snapshot.gateContract)
    || gateContract.attempts.maximumRepairs !== base.snapshot.gateContract.attempts.maximumRepairs) {
    throw new Error('HARNESS_PROGRAMME_POLICY_V2_BASE_CONTRACT_MISMATCH');
  }
  const snapshot = deepFreeze({
    schemaVersion: 2 as const,
    policyId: PROGRAMME_POLICY_V6_ID,
    authority: DEVELOPMENT_AUTHORITY,
    basePolicy: base.snapshot,
    basePolicyFingerprint: base.fingerprint,
    gateContract,
  });
  return deepFreeze({ snapshot, fingerprint: digestValue(snapshot), base });
}

function parseDigest(value: unknown): string {
  if (typeof value !== 'string' || !SHA256_PATTERN.test(value) || value === '0'.repeat(64)) {
    throw new TypeError('HARNESS_PROGRAMME_POLICY_V2_DIGEST_INVALID');
  }
  return value;
}
