// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import { verifyFrozenProgrammePolicyV1 } from '../src/programme-policy-v5.js';
import {
  createFrozenProgrammePolicyV2,
  programmePolicyV2Fingerprint,
  verifyFrozenProgrammePolicyV2,
} from '../src/programme-policy-v6.js';
import { digestValue } from '../src/receipts.js';
import { programmeV6Fixture } from './programme-v6-fixtures.js';

describe('frozen schema-v6 programme policy V2', () => {
  it('anchors an exact frozen V1 policy under an independently fingerprinted V2 wrapper', () => {
    const fixture = programmeV6Fixture();
    const base = fixture.policy.base;
    const snapshot = createFrozenProgrammePolicyV2(base.snapshot, base.fingerprint);
    const fingerprint = programmePolicyV2Fingerprint(snapshot);
    const parsed = verifyFrozenProgrammePolicyV2(structuredClone(snapshot), fingerprint);

    expect(parsed.snapshot).toEqual(fixture.policy.snapshot);
    expect(parsed.fingerprint).toBe(fixture.policy.fingerprint);
    expect(parsed.snapshot.basePolicyFingerprint).toBe(base.fingerprint);
    expect(parsed.snapshot.basePolicy).toEqual(base.snapshot);
    expect(parsed.snapshot.gateContract.baseGateContractDigest)
      .toBe(digestValue(base.snapshot.gateContract));
    expect(parsed.snapshot.gateContract.baseGateContract).toEqual(base.snapshot.gateContract);
    for (const value of [parsed, parsed.snapshot, parsed.snapshot.basePolicy,
      parsed.snapshot.gateContract, parsed.base]) expect(Object.isFrozen(value)).toBe(true);
  });

  it.each([
    ['unknown outer key', (value: any) => { value.extra = true; }],
    ['missing outer key', (value: any) => { delete value.gateContract; }],
    ['base fingerprint', (value: any) => { value.basePolicyFingerprint = 'e'.repeat(64); }],
    ['embedded base policy', (value: any) => {
      value.basePolicy.controller.identity.tree = 'e'.repeat(40);
    }],
    ['base gate digest', (value: any) => {
      value.gateContract.baseGateContractDigest = 'e'.repeat(64);
    }],
    ['V2 gate law', (value: any) => {
      value.gateContract.attempts.dispositionRule = 'caller-asserted';
    }],
  ])('rejects %s substitution even under an attacker-selected outer anchor', (_name, mutate) => {
    const value = structuredClone(programmeV6Fixture().policy.snapshot) as any;
    mutate(value);
    expect(() => programmePolicyV2Fingerprint(value)).toThrow();
  });

  it('requires the external V2 anchor and rejects V1/V2 cross-use', () => {
    const policy = programmeV6Fixture().policy;
    expect(() => verifyFrozenProgrammePolicyV2(policy.snapshot, 'e'.repeat(64)))
      .toThrow('HARNESS_PROGRAMME_POLICY_V2_FINGERPRINT_MISMATCH');
    expect(() => verifyFrozenProgrammePolicyV2(
      policy.base.snapshot,
      policy.base.fingerprint,
    )).toThrow();
    expect(() => verifyFrozenProgrammePolicyV1(
      policy.snapshot,
      policy.fingerprint,
    )).toThrow();
  });

  it('contains no provider-dollar spend ceiling in the V2 law', () => {
    const serialized = JSON.stringify(programmeV6Fixture().policy.snapshot);
    expect(serialized).not.toMatch(/spendCeiling|costCeiling|max(?:imum)?Spend|budgetUsd/i);
    expect(programmeV6Fixture().policy.snapshot.gateContract.attempts.maximumRepairs).toBe(2);
  });
});
