// SPDX-License-Identifier: MIT

import {
  DEVELOPMENT_AUTHORITY,
  SHA256_PATTERN,
  asInteger,
  asNonEmptyString,
  asRecord,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import { digestValue } from './receipts.js';

export const METAHARNESS_DIAGNOSTICS_PATH =
  'coding-harness/config/metaharness-diagnostics.json' as const;

export interface MetaHarnessDiagnosticTarget {
  target: 'repository' | 'coding-harness';
  repositoryPath: '.' | 'coding-harness';
  success: boolean;
  degraded: boolean;
  exitCode: number;
  schema: number;
  harnessFit: number;
  compileConfidence: number;
  taskCoverage: number;
  toolSafety: number;
  memoryUsefulness: number;
  scaffoldReady: boolean;
  hardConstraintsPassed: number;
  hardConstraintsTotal: number;
  archetype: string;
  template: string;
  recommendedMode: string;
  generatedAt: string;
  durationMs: number;
}

export interface MetaHarnessDiagnosticSnapshot {
  schemaVersion: 1;
  authority: typeof DEVELOPMENT_AUTHORITY;
  source: 'ruflo-metaharness-score-mcp';
  capturedAt: string;
  implementation: {
    ruflo: '3.38.20';
    claudeFlowCli: '3.34.0';
    metaharnessRange: '~0.3.0';
    metaharness: '0.3.2';
    wrapperDigest: string;
    bridgeDigest: string;
    scorecardDigest: string;
    analyzerDigest: string;
  };
  targets: MetaHarnessDiagnosticTarget[];
  digest: string;
}

const IMPLEMENTATION = Object.freeze({
  ruflo: '3.38.20',
  claudeFlowCli: '3.34.0',
  metaharnessRange: '~0.3.0',
  metaharness: '0.3.2',
  wrapperDigest: 'e14b64f1bcd51c61d3a33c0f1c6712c248e71726f104cbab1d0e0ffc26d775d5',
  bridgeDigest: '9bd511d2ed8b52d40a911113169f8cb9075a124e0a49b966c3975e6959cd0302',
  scorecardDigest: '89af201762b2e1284ca0715bc77bbb5c76a9703113ac7e139f2be282db423a31',
  analyzerDigest: 'da6050f1db03a5f8a267074b6f8f06f05d7c8c5ea158b391edd9443cff9a55f9',
} as const);

export function parseMetaHarnessDiagnosticSnapshot(value: unknown): MetaHarnessDiagnosticSnapshot {
  const input = asRecord(value, 'MetaHarness diagnostic snapshot');
  assertExactKeys(input, [
    'schemaVersion', 'authority', 'source', 'capturedAt', 'implementation', 'targets', 'digest',
  ], 'MetaHarness diagnostic snapshot');
  if (input.schemaVersion !== 1 || input.authority !== DEVELOPMENT_AUTHORITY
    || input.source !== 'ruflo-metaharness-score-mcp') {
    throw new TypeError('MetaHarness diagnostic snapshot identity is invalid');
  }
  const capturedAt = timestamp(input.capturedAt, 'MetaHarness diagnostic snapshot.capturedAt');
  const implementation = parseImplementation(input.implementation);
  if (!Array.isArray(input.targets)) {
    throw new TypeError('MetaHarness diagnostic snapshot.targets must be an array');
  }
  const targets = input.targets.map((target, index) => parseTarget(target, index));
  const names = targets.map(({ target }) => target);
  if (targets.length !== 2 || new Set(names).size !== 2
    || !names.includes('repository') || !names.includes('coding-harness')) {
    throw new TypeError('MetaHarness diagnostic snapshot must contain both targets exactly once');
  }
  const body = {
    schemaVersion: 1 as const,
    authority: DEVELOPMENT_AUTHORITY,
    source: 'ruflo-metaharness-score-mcp' as const,
    capturedAt,
    implementation,
    targets,
  };
  if (typeof input.digest !== 'string' || !SHA256_PATTERN.test(input.digest)
    || digestValue(body) !== input.digest) {
    throw new TypeError('MetaHarness diagnostic snapshot.digest is invalid');
  }
  return deepFreeze({ ...body, digest: input.digest });
}

function parseImplementation(value: unknown): MetaHarnessDiagnosticSnapshot['implementation'] {
  const input = asRecord(value, 'MetaHarness diagnostic snapshot.implementation');
  assertExactKeys(input, Object.keys(IMPLEMENTATION), 'MetaHarness diagnostic snapshot.implementation');
  for (const [key, expected] of Object.entries(IMPLEMENTATION)) {
    if (input[key] !== expected) {
      throw new TypeError(`MetaHarness diagnostic implementation mismatch: ${key}`);
    }
  }
  return { ...IMPLEMENTATION };
}

function parseTarget(value: unknown, index: number): MetaHarnessDiagnosticTarget {
  const label = `MetaHarness diagnostic snapshot.targets[${index}]`;
  const input = asRecord(value, label);
  assertExactKeys(input, [
    'target', 'repositoryPath', 'success', 'degraded', 'exitCode', 'schema',
    'harnessFit', 'compileConfidence', 'taskCoverage', 'toolSafety', 'memoryUsefulness',
    'scaffoldReady', 'hardConstraintsPassed', 'hardConstraintsTotal', 'archetype',
    'template', 'recommendedMode', 'generatedAt', 'durationMs',
  ], label);
  if (input.target !== 'repository' && input.target !== 'coding-harness') {
    throw new TypeError(`${label}.target is invalid`);
  }
  const repositoryPath = input.target === 'repository' ? '.' : 'coding-harness';
  if (input.repositoryPath !== repositoryPath) throw new TypeError(`${label}.repositoryPath is invalid`);
  if (typeof input.success !== 'boolean' || typeof input.degraded !== 'boolean'
    || typeof input.scaffoldReady !== 'boolean') {
    throw new TypeError(`${label} boolean evidence is invalid`);
  }
  const hardConstraintsTotal = asInteger(input.hardConstraintsTotal, `${label}.hardConstraintsTotal`, 1);
  const hardConstraintsPassed = asInteger(input.hardConstraintsPassed, `${label}.hardConstraintsPassed`);
  if (input.schema !== 1 || hardConstraintsTotal !== 6
    || hardConstraintsPassed > hardConstraintsTotal) {
    throw new TypeError(`${label} schema or hard-constraint cardinality is invalid`);
  }
  return {
    target: input.target,
    repositoryPath,
    success: input.success,
    degraded: input.degraded,
    exitCode: asInteger(input.exitCode, `${label}.exitCode`),
    schema: 1,
    harnessFit: score(input.harnessFit, `${label}.harnessFit`),
    compileConfidence: score(input.compileConfidence, `${label}.compileConfidence`),
    taskCoverage: score(input.taskCoverage, `${label}.taskCoverage`),
    toolSafety: score(input.toolSafety, `${label}.toolSafety`),
    memoryUsefulness: score(input.memoryUsefulness, `${label}.memoryUsefulness`),
    scaffoldReady: input.scaffoldReady,
    hardConstraintsPassed,
    hardConstraintsTotal,
    archetype: asNonEmptyString(input.archetype, `${label}.archetype`),
    template: asNonEmptyString(input.template, `${label}.template`),
    recommendedMode: asNonEmptyString(input.recommendedMode, `${label}.recommendedMode`),
    generatedAt: timestamp(input.generatedAt, `${label}.generatedAt`),
    durationMs: asInteger(input.durationMs, `${label}.durationMs`),
  };
}

function score(value: unknown, label: string): number {
  const parsed = asInteger(value, label);
  if (parsed > 100) throw new TypeError(`${label} cannot exceed 100`);
  return parsed;
}

function timestamp(value: unknown, label: string): string {
  const text = asNonEmptyString(value, label);
  const date = new Date(text);
  if (!Number.isFinite(date.valueOf()) || date.toISOString() !== text) {
    throw new TypeError(`${label} must be a canonical ISO timestamp`);
  }
  return text;
}
