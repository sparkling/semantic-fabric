// SPDX-License-Identifier: MIT

import {
  PROGRAMME_ACCEPTANCE_DIMENSIONS,
  PROGRAMME_ACCEPTANCE_THRESHOLD,
  scoreProgrammeAcceptance,
  type ProgrammeAcceptanceResult,
  type ProgrammeDimensionId,
  type UpstreamDiagnosticEvidence,
} from './programme-acceptance.js';
import {
  parseProgrammeGateContractV1,
  type ProgrammeGateContractV1,
} from './programme-gate-contract-v1.js';
import type { ProgrammeGateEvaluationV5 } from './programme-gates-v5.js';
import { ReceiptChain, type Receipt } from './receipts.js';
import {
  SHA256_PATTERN,
  asRecord,
  assertExactKeys,
} from './contracts.js';

const ACCEPTANCE_SCHEMA_VERSION = 2 as const;
const ACCEPTANCE_MAXIMUM_SCORE = 100 as const;
const DIMENSION_ENTRIES = Object.entries(PROGRAMME_ACCEPTANCE_DIMENSIONS) as Array<
  readonly [ProgrammeDimensionId, number]
>;

export interface ProgrammeScoreInputV5 {
  readonly gateContract: ProgrammeGateContractV1;
  readonly receipt: Receipt;
  readonly gateEvaluation: ProgrammeGateEvaluationV5;
}

export function scoreProgrammeReceiptV5(value: ProgrammeScoreInputV5): ProgrammeAcceptanceResult {
  const input = asRecord(value, 'programme v5 score input');
  assertExactKeys(
    input,
    ['gateContract', 'receipt', 'gateEvaluation'],
    'programme v5 score input',
  );
  const gateContract = parseProgrammeGateContractV1(input.gateContract);
  assertAcceptanceContract(gateContract);
  const receipt = verifyScoredReceipt(input.receipt);
  const evaluation = asRecord(input.gateEvaluation, 'programme v5 gate evaluation');
  assertExactKeys(
    evaluation,
    ['dimensions', 'diagnostics'],
    'programme v5 gate evaluation',
  );

  return scoreProgrammeAcceptance({
    schemaVersion: ACCEPTANCE_SCHEMA_VERSION,
    authority: gateContract.receipt.authority,
    receiptDigest: receipt.digest,
    dimensions: parseDimensions(evaluation.dimensions, gateContract),
    upstreamDiagnostics: parseDiagnostics(evaluation.diagnostics, gateContract),
  });
}

function verifyScoredReceipt(value: unknown): Receipt {
  const chain = ReceiptChain.import(JSON.stringify({ schemaVersion: 3, receipts: [value] }));
  const receipt = chain.entries()[0];
  if (chain.length !== 1 || receipt === undefined) {
    throw new Error('HARNESS_PROGRAMME_V5_SCORED_RECEIPT_INVALID');
  }
  return receipt;
}

function assertAcceptanceContract(gateContract: ProgrammeGateContractV1): void {
  const acceptance = gateContract.acceptance;
  const expectedDimensions = DIMENSION_ENTRIES.map(([id, maximumPoints]) => ({
    id,
    maximumPoints,
  }));
  if (gateContract.acceptanceSchemaVersion !== ACCEPTANCE_SCHEMA_VERSION
    || acceptance.threshold !== PROGRAMME_ACCEPTANCE_THRESHOLD
    || acceptance.maximumScore !== ACCEPTANCE_MAXIMUM_SCORE
    || !acceptance.allDimensionsAreHardGates
    || !acceptance.diagnosticGateRequired
    || acceptance.fitnessEligible !== false
    || JSON.stringify(acceptance.dimensions) !== JSON.stringify(expectedDimensions)
    || expectedDimensions.reduce((sum, dimension) => sum + dimension.maximumPoints, 0)
      !== ACCEPTANCE_MAXIMUM_SCORE) {
    throw new Error('HARNESS_PROGRAMME_V5_ACCEPTANCE_CONTRACT_MISMATCH');
  }
}

function parseDimensions(
  value: unknown,
  gateContract: ProgrammeGateContractV1,
): Array<Readonly<{
  id: ProgrammeDimensionId;
  verifiedPoints: number;
  hardGatePassed: boolean;
  evidenceDigests: string[];
}>> {
  if (!Array.isArray(value) || value.length !== gateContract.acceptance.dimensions.length) {
    invalidGateResult();
  }
  return gateContract.acceptance.dimensions.map(({ id, maximumPoints }, index) => {
    const entry = asRecord(value[index], `programme v5 gate dimension[${index}]`);
    assertExactKeys(
      entry,
      ['id', 'passed', 'evidenceDigest'],
      `programme v5 gate dimension[${index}]`,
    );
    if (entry.id !== id || typeof entry.passed !== 'boolean'
      || typeof entry.evidenceDigest !== 'string'
      || !SHA256_PATTERN.test(entry.evidenceDigest)) {
      invalidGateResult();
    }
    return {
      id,
      verifiedPoints: entry.passed ? maximumPoints : 0,
      hardGatePassed: entry.passed,
      evidenceDigests: [entry.evidenceDigest],
    };
  });
}

function parseDiagnostics(
  value: unknown,
  gateContract: ProgrammeGateContractV1,
): UpstreamDiagnosticEvidence[] {
  if (!Array.isArray(value) || value.length !== gateContract.diagnostics.targets.length) {
    invalidGateResult();
  }
  return gateContract.diagnostics.targets.map((targetContract, index) => {
    const entry = asRecord(value[index], `programme v5 diagnostic gate[${index}]`);
    assertExactKeys(entry, [
      'target', 'implementation', 'success', 'degraded', 'exitCode', 'scaffoldReady',
      'hardConstraintsPassed', 'hardConstraintsTotal', 'harnessFit', 'evidenceDigest', 'passed',
    ], `programme v5 diagnostic gate[${index}]`);
    const expectedImplementation = `metaharness@${gateContract.diagnostics.implementation.metaharness}`;
    const passed = entry.success === true
      && entry.degraded === false
      && entry.exitCode === 0
      && entry.scaffoldReady === true
      && entry.hardConstraintsPassed === entry.hardConstraintsTotal;
    if (entry.target !== targetContract.target
      || entry.implementation !== expectedImplementation
      || entry.hardConstraintsTotal !== targetContract.hardConstraintsTotal
      || typeof entry.passed !== 'boolean'
      || entry.passed !== passed) {
      invalidGateResult();
    }
    const { passed: _passed, ...diagnostic } = entry;
    return diagnostic as unknown as UpstreamDiagnosticEvidence;
  });
}

function invalidGateResult(): never {
  throw new Error('HARNESS_PROGRAMME_V5_GATE_RESULT_INVALID');
}
