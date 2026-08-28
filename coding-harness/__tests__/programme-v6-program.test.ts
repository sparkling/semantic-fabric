// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import { canonicalProgrammePolicyJson } from '../src/programme-v5-driver-support.js';
import {
  createReviewableProgrammeV6Policy,
} from '../src/programme-v6-program.js';
import {
  verifyFrozenProgrammePolicyV2,
} from '../src/programme-policy-v6.js';
import { programmeV6Fixture } from './programme-v6-fixtures.js';

describe('programme V6 trusted policy composition', () => {
  it('wraps the exact frozen V5 base in the reviewed V6 policy', () => {
    const fixture = programmeV6Fixture();
    const review = createReviewableProgrammeV6Policy(
      canonicalProgrammePolicyJson(fixture.policy.base.snapshot),
      fixture.policy.base.fingerprint,
    );

    expect(review.policyFingerprint).toBe(fixture.policy.fingerprint);
    expect(verifyFrozenProgrammePolicyV2(
      JSON.parse(review.policyBlob), review.policyFingerprint,
    ).snapshot).toEqual(fixture.policy.snapshot);
    expect(canonicalProgrammePolicyJson(JSON.parse(review.policyBlob))).toBe(review.policyBlob);
  });

  it('rejects non-canonical base bytes instead of silently normalizing them', () => {
    const fixture = programmeV6Fixture();
    const canonical = canonicalProgrammePolicyJson(fixture.policy.base.snapshot);

    expect(() => createReviewableProgrammeV6Policy(
      `${canonical}\n`, fixture.policy.base.fingerprint,
    )).toThrow('HARNESS_PROGRAMME_V6_BASE_POLICY_SERIALIZATION_INVALID');
  });

  it('rejects a base policy/fingerprint substitution', () => {
    const fixture = programmeV6Fixture();

    expect(() => createReviewableProgrammeV6Policy(
      canonicalProgrammePolicyJson(fixture.policy.base.snapshot),
      'f'.repeat(64),
    )).toThrow('HARNESS_PROGRAMME_POLICY_FINGERPRINT_MISMATCH');
  });
});
