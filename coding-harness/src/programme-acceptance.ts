// SPDX-License-Identifier: MIT

import {
  DEVELOPMENT_AUTHORITY,
  SHA256_PATTERN,
  asInteger,
  asRecord,
  asUniqueStrings,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';

export const PROGRAMME_ACCEPTANCE_THRESHOLD = 98 as const;

export const PROGRAMME_ACCEPTANCE_DIMENSIONS = Object.freeze({
  'policy-and-supply-chain-safety': 20,
  'evaluator-integrity': 15,
  'evolution-containment': 5,
  'patched-candidate-verification': 20,
  'dual-host-control-plane': 15,
  'reliability-and-receipts': 15,
  'ruflo-and-qe-integration': 10,
} as const);

export type ProgrammeDimensionId = keyof typeof PROGRAMME_ACCEPTANCE_DIMENSIONS;
export type DiagnosticTarget = 'repository' | 'coding-harness';

export interface ProgrammeDimensionEvidence {
  id: ProgrammeDimensionId;
  verifiedPoints: number;
  maximumPoints: number;
  hardGatePassed: boolean;
  evidenceDigests: string[];
}

export interface UpstreamDiagnosticEvidence {
  target: DiagnosticTarget;
  implementation: 'metaharness@0.3.2';
  success: boolean;
  degraded: boolean;
  exitCode: number;
  scaffoldReady: boolean;
  hardConstraintsPassed: number;
  hardConstraintsTotal: number;
  harnessFit: number;
  evidenceDigest: string;
}

export interface ProgrammeAcceptanceResult {
  schemaVersion: 2;
  authority: typeof DEVELOPMENT_AUTHORITY;
  receiptDigest: string;
  score: number;
  maximumScore: 100;
  threshold: typeof PROGRAMME_ACCEPTANCE_THRESHOLD;
  status: 'ACCEPTED' | 'REJECTED';
  hardGatesPassed: boolean;
  diagnosticGatePassed: boolean;
  failedDimensions: ProgrammeDimensionId[];
  failedDiagnostics: DiagnosticTarget[];
  dimensions: ProgrammeDimensionEvidence[];
  upstreamDiagnostics: UpstreamDiagnosticEvidence[];
  fitnessEligible: false;
}

const DIMENSION_IDS = Object.keys(PROGRAMME_ACCEPTANCE_DIMENSIONS) as ProgrammeDimensionId[];
const DIAGNOSTIC_TARGETS: DiagnosticTarget[] = ['repository', 'coding-harness'];

export function scoreProgrammeAcceptance(value: unknown): ProgrammeAcceptanceResult {
  const input = asRecord(value, 'programme acceptance');
  assertExactKeys(
    input,
    ['schemaVersion', 'authority', 'receiptDigest', 'dimensions', 'upstreamDiagnostics'],
    'programme acceptance',
  );
  if (input.schemaVersion !== 2) throw new TypeError('programme acceptance.schemaVersion must be 2');
  if (input.authority !== DEVELOPMENT_AUTHORITY) {
    throw new TypeError('programme acceptance.authority cannot grant promotion');
  }
  if (typeof input.receiptDigest !== 'string' || !SHA256_PATTERN.test(input.receiptDigest)) {
    throw new TypeError('programme acceptance.receiptDigest must be a lowercase SHA-256 digest');
  }

  const dimensions = parseDimensions(input.dimensions);
  const upstreamDiagnostics = parseDiagnostics(input.upstreamDiagnostics);
  const score = dimensions.reduce((total, dimension) => total + dimension.verifiedPoints, 0);
  const failedDimensions = dimensions
    .filter(({ hardGatePassed }) => !hardGatePassed)
    .map(({ id }) => id);
  const failedDiagnostics = upstreamDiagnostics
    .filter((diagnostic) => !diagnostic.success || diagnostic.exitCode !== 0
      || diagnostic.degraded || !diagnostic.scaffoldReady
      || diagnostic.hardConstraintsPassed !== diagnostic.hardConstraintsTotal)
    .map(({ target }) => target);
  const hardGatesPassed = failedDimensions.length === 0;
  const diagnosticGatePassed = failedDiagnostics.length === 0;

  return deepFreeze({
    schemaVersion: 2,
    authority: DEVELOPMENT_AUTHORITY,
    receiptDigest: input.receiptDigest,
    score,
    maximumScore: 100,
    threshold: PROGRAMME_ACCEPTANCE_THRESHOLD,
    status: score >= PROGRAMME_ACCEPTANCE_THRESHOLD && hardGatesPassed && diagnosticGatePassed
      ? 'ACCEPTED'
      : 'REJECTED',
    hardGatesPassed,
    diagnosticGatePassed,
    failedDimensions,
    failedDiagnostics,
    dimensions,
    upstreamDiagnostics,
    fitnessEligible: false,
  });
}

function parseDimensions(value: unknown): ProgrammeDimensionEvidence[] {
  if (!Array.isArray(value)) throw new TypeError('programme acceptance.dimensions must be an array');
  const parsed = value.map((entry, index) => parseDimension(entry, index));
  assertExactlyOnce(parsed.map(({ id }) => id), DIMENSION_IDS, 'programme dimension');
  const byId = new Map(parsed.map((dimension) => [dimension.id, dimension]));
  return DIMENSION_IDS.map((id) => byId.get(id) as ProgrammeDimensionEvidence);
}

function parseDimension(value: unknown, index: number): ProgrammeDimensionEvidence {
  const label = `programme acceptance.dimensions[${index}]`;
  const input = asRecord(value, label);
  assertExactKeys(input, ['id', 'verifiedPoints', 'hardGatePassed', 'evidenceDigests'], label);
  if (typeof input.id !== 'string' || !(input.id in PROGRAMME_ACCEPTANCE_DIMENSIONS)) {
    throw new TypeError(`${label}.id is not an ADR-0037 dimension`);
  }
  const id = input.id as ProgrammeDimensionId;
  const maximumPoints = PROGRAMME_ACCEPTANCE_DIMENSIONS[id];
  const verifiedPoints = asInteger(input.verifiedPoints, `${label}.verifiedPoints`);
  if (verifiedPoints > maximumPoints) {
    throw new TypeError(`${label}.verifiedPoints exceeds the dimension maximum of ${maximumPoints}`);
  }
  if (typeof input.hardGatePassed !== 'boolean') {
    throw new TypeError(`${label}.hardGatePassed must be a boolean`);
  }
  const evidenceDigests = asUniqueStrings(input.evidenceDigests, `${label}.evidenceDigests`);
  evidenceDigests.forEach((digest, digestIndex) => {
    if (!SHA256_PATTERN.test(digest)) {
      throw new TypeError(`${label}.evidenceDigests[${digestIndex}] must be a lowercase SHA-256 digest`);
    }
  });
  return {
    id,
    verifiedPoints,
    maximumPoints,
    hardGatePassed: input.hardGatePassed,
    evidenceDigests,
  };
}

function parseDiagnostics(value: unknown): UpstreamDiagnosticEvidence[] {
  if (!Array.isArray(value)) {
    throw new TypeError('programme acceptance.upstreamDiagnostics must be an array');
  }
  const parsed = value.map((entry, index) => parseDiagnostic(entry, index));
  assertExactlyOnce(parsed.map(({ target }) => target), DIAGNOSTIC_TARGETS, 'diagnostic target');
  const byTarget = new Map(parsed.map((diagnostic) => [diagnostic.target, diagnostic]));
  return DIAGNOSTIC_TARGETS.map((target) => byTarget.get(target) as UpstreamDiagnosticEvidence);
}

function parseDiagnostic(value: unknown, index: number): UpstreamDiagnosticEvidence {
  const label = `programme acceptance.upstreamDiagnostics[${index}]`;
  const input = asRecord(value, label);
  assertExactKeys(input, [
    'target', 'implementation', 'success', 'degraded', 'exitCode', 'scaffoldReady',
    'hardConstraintsPassed', 'hardConstraintsTotal', 'harnessFit', 'evidenceDigest',
  ], label);
  if (input.target !== 'repository' && input.target !== 'coding-harness') {
    throw new TypeError(`${label}.target is invalid`);
  }
  if (input.implementation !== 'metaharness@0.3.2') {
    throw new TypeError(`${label}.implementation is not the audited diagnostic implementation`);
  }
  if (typeof input.success !== 'boolean' || typeof input.degraded !== 'boolean'
    || typeof input.scaffoldReady !== 'boolean') {
    throw new TypeError(`${label} status evidence must be boolean`);
  }
  const exitCode = asInteger(input.exitCode, `${label}.exitCode`);
  const hardConstraintsTotal = asInteger(input.hardConstraintsTotal, `${label}.hardConstraintsTotal`, 1);
  const hardConstraintsPassed = asInteger(input.hardConstraintsPassed, `${label}.hardConstraintsPassed`);
  if (hardConstraintsPassed > hardConstraintsTotal) {
    throw new TypeError(`${label}.hardConstraintsPassed exceeds hardConstraintsTotal`);
  }
  const harnessFit = asInteger(input.harnessFit, `${label}.harnessFit`);
  if (harnessFit > 100) throw new TypeError(`${label}.harnessFit cannot exceed 100`);
  if (typeof input.evidenceDigest !== 'string' || !SHA256_PATTERN.test(input.evidenceDigest)) {
    throw new TypeError(`${label}.evidenceDigest must be a lowercase SHA-256 digest`);
  }
  return {
    target: input.target,
    implementation: 'metaharness@0.3.2',
    success: input.success,
    degraded: input.degraded,
    exitCode,
    scaffoldReady: input.scaffoldReady,
    hardConstraintsPassed,
    hardConstraintsTotal,
    harnessFit,
    evidenceDigest: input.evidenceDigest,
  };
}

function assertExactlyOnce<T extends string>(actual: T[], expected: T[], label: string): void {
  if (actual.length !== expected.length
    || new Set(actual).size !== actual.length
    || expected.some((entry) => !actual.includes(entry))) {
    throw new TypeError(`${label}s must include every required value exactly once`);
  }
}
