// SPDX-License-Identifier: MIT

import { describe, expect, it } from 'vitest';
import {
  PROGRAMME_ACCEPTANCE_DIMENSIONS,
  type ProgrammeDimensionId,
} from '../src/programme-acceptance.js';
import { DEVELOPMENT_AUTHORITY } from '../src/contracts.js';
import { createProgrammeGateContractV1 } from '../src/programme-gate-contract-v1.js';
import type { ProgrammeGateEvaluationV5 } from '../src/programme-gates-v5.js';
import { scoreProgrammeReceiptV5 } from '../src/programme-score-v5.js';
import { ReceiptChain, type Receipt, type ReceiptDraft } from '../src/receipts.js';

const digest = (character: string) => character.repeat(64);

describe('schema-v5 programme scoring', () => {
  it('awards all seven hard-gate dimensions and accepts exactly 100/100', () => {
    const result = score(evaluation());

    expect(result).toMatchObject({
      schemaVersion: 2,
      receiptDigest: scoredReceipt().digest,
      score: 100,
      maximumScore: 100,
      threshold: 98,
      status: 'ACCEPTED',
      hardGatesPassed: true,
      diagnosticGatePassed: true,
      fitnessEligible: false,
    });
    expect(result.dimensions.map(({ id, verifiedPoints }) => [id, verifiedPoints]))
      .toEqual(Object.entries(PROGRAMME_ACCEPTANCE_DIMENSIONS));
  });

  for (const [failedId, maximumPoints] of Object.entries(PROGRAMME_ACCEPTANCE_DIMENSIONS) as Array<
    [ProgrammeDimensionId, number]
  >) {
    it(`zeros the complete ${failedId} dimension on failure`, () => {
      const gates = evaluation();
      const dimensions = gates.dimensions.map((dimension) => (
        dimension.id === failedId ? { ...dimension, passed: false } : dimension
      ));
      const result = score({ ...gates, dimensions });

      expect(result.score).toBe(100 - maximumPoints);
      expect(result.status).toBe('REJECTED');
      expect(result.failedDimensions).toEqual([failedId]);
      expect(result.dimensions.find(({ id }) => id === failedId)).toMatchObject({
        verifiedPoints: 0,
        maximumPoints,
        hardGatePassed: false,
      });
    });
  }

  it('keeps the frozen 98/100 threshold and gives diagnostics no points', () => {
    const gates = evaluation();
    const diagnostics = gates.diagnostics.map((diagnostic, index) => (
      index === 1 ? { ...diagnostic, degraded: true, passed: false } : diagnostic
    ));
    const result = score({ ...gates, diagnostics });

    expect(result.score).toBe(100);
    expect(result.threshold).toBe(98);
    expect(result.maximumScore).toBe(100);
    expect(result.status).toBe('REJECTED');
    expect(result.failedDiagnostics).toEqual(['coding-harness']);

    const weaker = structuredClone(createProgrammeGateContractV1(2)) as any;
    weaker.acceptance.threshold = 97;
    expect(() => scoreProgrammeReceiptV5({
      gateContract: weaker,
      receipt: scoredReceipt(),
      gateEvaluation: gates,
    })).toThrow();

    const reweighted = structuredClone(createProgrammeGateContractV1(2)) as any;
    reweighted.acceptance.dimensions[0].maximumPoints = 19;
    expect(() => scoreProgrammeReceiptV5({
      gateContract: reweighted,
      receipt: scoredReceipt(),
      gateEvaluation: gates,
    })).toThrow();
  });

  it('fails closed on malformed, reordered, or inconsistent gate results', () => {
    const gates = evaluation();
    expect(() => score({
      ...gates,
      dimensions: [...gates.dimensions].reverse(),
    })).toThrow('HARNESS_PROGRAMME_V5_GATE_RESULT_INVALID');
    expect(() => score({
      ...gates,
      dimensions: gates.dimensions.map((dimension, index) => (
        index === 0 ? { ...dimension, evidenceDigest: 'not-a-digest' } : dimension
      )),
    })).toThrow('HARNESS_PROGRAMME_V5_GATE_RESULT_INVALID');
    expect(() => score({
      ...gates,
      diagnostics: gates.diagnostics.map((diagnostic, index) => (
        index === 0 ? { ...diagnostic, passed: false } : diagnostic
      )),
    })).toThrow('HARNESS_PROGRAMME_V5_GATE_RESULT_INVALID');
  });

  it('returns a recursively immutable acceptance result', () => {
    const result = score(evaluation());

    for (const value of [
      result,
      result.dimensions,
      result.dimensions[0],
      result.dimensions[0]?.evidenceDigests,
      result.upstreamDiagnostics,
      result.upstreamDiagnostics[0],
      result.failedDimensions,
      result.failedDiagnostics,
    ]) expect(Object.isFrozen(value)).toBe(true);
    expect(() => result.dimensions.push(result.dimensions[0]!)).toThrow();
  });

  it('rejects incomplete, genesis, stale, or extended receipt objects', () => {
    const receipt = scoredReceipt();
    const invalid = [
      { digest: receipt.digest },
      { ...receipt, digest: '0'.repeat(64) },
      { ...receipt, runId: 'tampered-run' },
      { ...receipt, unexpected: true },
    ];
    for (const candidate of invalid) {
      expect(() => scoreProgrammeReceiptV5({
        gateContract: createProgrammeGateContractV1(2),
        receipt: candidate as Receipt,
        gateEvaluation: evaluation(),
      })).toThrow();
    }
  });
});

function score(gateEvaluation: ProgrammeGateEvaluationV5) {
  return scoreProgrammeReceiptV5({
    gateContract: createProgrammeGateContractV1(2),
    receipt: scoredReceipt(),
    gateEvaluation,
  });
}

function scoredReceipt(): Receipt {
  const identity = (character: string) => ({
    commit: character.repeat(40), tree: character.repeat(40),
  });
  const draft: ReceiptDraft = {
    schemaVersion: 3,
    runId: 'score-run-0001',
    taskId: 'score-task-0001',
    step: 'candidate-transaction',
    status: 'fail',
    failureCode: 'HARNESS_TRANSACTION_FAILED',
    authority: DEVELOPMENT_AUTHORITY,
    issuedAt: '2026-08-27T08:00:00.000Z',
    identities: {
      controller: identity('1'), baseline: identity('2'),
      evaluator: identity('3'), candidate: identity('4'),
    },
    protectedInputs: {},
    route: {
      snapshotDigest: digest('5'),
      frozenAt: '2026-08-27T07:59:00.000Z',
      routerVersion: '@metaharness/router@0.4.0',
    },
    hosts: [], admittedPaths: [], patchDigest: null, patchDigests: [],
    toolVersions: {}, commands: [], artifactDigests: {}, verifierDigests: {},
    critiqueDigests: [], reviewDigests: [],
    recovery: { retryCount: 0, breakerState: 'closed', cancelled: false, repairCount: 0 },
    coordination: {
      swarmId: null, taskId: null, hookIds: [], traceIds: [],
      agenticQeEvidenceDigests: [], nativeEvidenceDigests: [],
      nativeRuntimeEvidenceDigest: null,
    },
  };
  return new ReceiptChain().append(draft);
}

function evaluation(): ProgrammeGateEvaluationV5 {
  return {
    dimensions: Object.keys(PROGRAMME_ACCEPTANCE_DIMENSIONS).map((id, index) => ({
      id: id as ProgrammeDimensionId,
      passed: true,
      evidenceDigest: digest('abcdef0'[index]!),
    })),
    diagnostics: [
      diagnostic('repository', 71, '1'),
      diagnostic('coding-harness', 67, '2'),
    ],
  };
}

function diagnostic(
  target: 'repository' | 'coding-harness',
  harnessFit: number,
  evidence: string,
) {
  return {
    target,
    implementation: 'metaharness@0.3.2' as const,
    success: true,
    degraded: false,
    exitCode: 0,
    scaffoldReady: true,
    hardConstraintsPassed: 6,
    hardConstraintsTotal: 6,
    harnessFit,
    evidenceDigest: digest(evidence),
    passed: true,
  };
}
