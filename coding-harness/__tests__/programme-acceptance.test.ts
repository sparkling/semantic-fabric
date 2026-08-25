// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  PROGRAMME_ACCEPTANCE_DIMENSIONS,
  scoreProgrammeAcceptance,
} from '../src/programme-acceptance.js';

const digest = (character: string) => character.repeat(64);

function passingInput(overrides: Record<string, unknown> = {}) {
  return {
    schemaVersion: 1,
    authority: 'development-only-no-promotion',
    dimensions: Object.entries(PROGRAMME_ACCEPTANCE_DIMENSIONS).map(([id, maximumPoints], index) => ({
      id,
      verifiedPoints: maximumPoints,
      hardGatePassed: true,
      evidenceDigests: [digest('abcdef'[index % 6]!)],
    })),
    upstreamDiagnostics: [
      {
        target: 'repository',
        degraded: false,
        hardConstraintsPassed: 6,
        hardConstraintsTotal: 6,
        harnessFit: 71,
      },
      {
        target: 'coding-harness',
        degraded: false,
        hardConstraintsPassed: 6,
        hardConstraintsTotal: 6,
        harnessFit: 67,
      },
    ],
    ...overrides,
  };
}

describe('ADR-0037 programme acceptance scoring', () => {
  it('accepts verified programme evidence at the threshold without treating harness-fit as the score', () => {
    const input = passingInput();
    const dimensions = input.dimensions.map((dimension, index) => (
      index === dimensionsLength(input) - 1
        ? { ...dimension, verifiedPoints: (dimension.verifiedPoints as number) - 2 }
        : dimension
    ));
    const result = scoreProgrammeAcceptance({ ...input, dimensions });

    expect(result.score).toBe(98);
    expect(result.maximumScore).toBe(100);
    expect(result.threshold).toBe(98);
    expect(result.status).toBe('ACCEPTED');
    expect(result.upstreamDiagnostics.map(({ harnessFit }) => harnessFit)).toEqual([71, 67]);
    expect(result.fitnessEligible).toBe(false);
  });

  it('does not average away a failed hard gate even above the score threshold', () => {
    const input = passingInput();
    const dimensions = input.dimensions.map((dimension, index) => (
      index === 0 ? { ...dimension, hardGatePassed: false } : dimension
    ));
    const result = scoreProgrammeAcceptance({ ...input, dimensions });

    expect(result.score).toBe(100);
    expect(result.status).toBe('REJECTED');
    expect(result.failedDimensions).toEqual(['policy-and-supply-chain-safety']);
  });

  it('fails closed on degraded or incomplete upstream diagnostic execution', () => {
    const input = passingInput();
    const upstreamDiagnostics = input.upstreamDiagnostics.map((diagnostic, index) => (
      index === 1 ? { ...diagnostic, degraded: true, hardConstraintsPassed: 5 } : diagnostic
    ));
    const result = scoreProgrammeAcceptance({ ...input, upstreamDiagnostics });

    expect(result.score).toBe(100);
    expect(result.status).toBe('REJECTED');
    expect(result.failedDiagnostics).toEqual(['coding-harness']);
  });

  it('rejects duplicate dimensions and evidence-free point claims', () => {
    const input = passingInput();
    expect(() => scoreProgrammeAcceptance({
      ...input,
      dimensions: [...input.dimensions.slice(0, -1), input.dimensions[0]],
    })).toThrow(/exactly once/);
    expect(() => scoreProgrammeAcceptance({
      ...input,
      dimensions: input.dimensions.map((dimension, index) => (
        index === 0 ? { ...dimension, evidenceDigests: [] } : dimension
      )),
    })).toThrow(/evidenceDigests/);
  });

  it('rejects out-of-range points and malformed evidence digests', () => {
    const input = passingInput();
    expect(() => scoreProgrammeAcceptance({
      ...input,
      dimensions: input.dimensions.map((dimension, index) => (
        index === 0 ? { ...dimension, verifiedPoints: 21 } : dimension
      )),
    })).toThrow(/maximum/);
    expect(() => scoreProgrammeAcceptance({
      ...input,
      dimensions: input.dimensions.map((dimension, index) => (
        index === 0 ? { ...dimension, evidenceDigests: ['not-a-digest'] } : dimension
      )),
    })).toThrow(/SHA-256/);
  });
});

function dimensionsLength(input: ReturnType<typeof passingInput>): number {
  return input.dimensions.length;
}
