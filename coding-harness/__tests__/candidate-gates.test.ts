// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import { assertRequiredQeProfiles } from '../src/candidate-gates.js';
import type { AgenticQeEvidence, AgenticQeProfile } from '../src/evidence.js';

function evidence(profile: AgenticQeProfile): AgenticQeEvidence {
  return {
    schemaVersion: 1,
    source: 'agentic-qe-local-profile',
    profile,
    taskId: 'task-qe-gates-0001',
    runId: 'run-qe-gates-0001',
    candidateTree: 'a'.repeat(40),
    commandDigest: 'b'.repeat(64),
    outputDigest: 'c'.repeat(64),
    providerVariablesStripped: true,
    authoritative: false,
    capturedAt: '2026-08-27T00:00:00.000Z',
  };
}

describe('task-bound QE profile gates', () => {
  it('requires the exact declared profile set and canonical order', () => {
    const required: AgenticQeProfile[] = ['lcov-gap', 'sast'];
    expect(() => assertRequiredQeProfiles(
      [evidence('lcov-gap'), evidence('sast')],
      required,
    )).not.toThrow();
    expect(() => assertRequiredQeProfiles(
      [evidence('lcov-gap')],
      required,
    )).toThrow('HARNESS_REQUIRED_QE_PROFILES_MISSING:sast');
    expect(() => assertRequiredQeProfiles(
      [evidence('lcov-gap'), evidence('sast'), evidence('quality-contract')],
      required,
    )).toThrow('HARNESS_REQUIRED_QE_PROFILES_EXTRA:quality-contract');
    expect(() => assertRequiredQeProfiles(
      [evidence('sast'), evidence('lcov-gap')],
      required,
    )).toThrow('HARNESS_REQUIRED_QE_PROFILE_ORDER_MISMATCH');
  });
});
