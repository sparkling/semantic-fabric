// SPDX-License-Identifier: MIT

import {
  asInteger,
  asNonEmptyString,
  asRecord,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';

export const AGENTIC_QE_LCOV_GAPS_TOOL = 'qe/coverage/gaps' as const;
export const AGENTIC_QE_MAX_MCP_OUTPUT_BYTES = 5_000_000 as const;
const MAX_GAPS = 20;

export function parseNestedCoverageGapResult(value: unknown): unknown {
  const outer = asRecord(value, 'Agentic-QE MCP response');
  const allowedOuterKeys = new Set(['content', 'isError']);
  if (Object.keys(outer).some((key) => !allowedOuterKeys.has(key)) || !('content' in outer)) {
    throw new TypeError('Agentic-QE MCP response has invalid keys');
  }
  if (outer.isError !== undefined && outer.isError !== false) {
    throw new Error('HARNESS_AGENTIC_QE_MCP_ERROR');
  }
  if (!Array.isArray(outer.content) || outer.content.length !== 1) {
    throw new TypeError('Agentic-QE MCP response must contain one text block');
  }
  const block = asRecord(outer.content[0], 'Agentic-QE MCP response block');
  assertExactKeys(block, ['type', 'text'], 'Agentic-QE MCP response block');
  if (block.type !== 'text') throw new TypeError('Agentic-QE MCP response block must be text');
  const text = boundedText(
    block.text,
    'Agentic-QE MCP response text',
    AGENTIC_QE_MAX_MCP_OUTPUT_BYTES,
  );
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new TypeError('HARNESS_AGENTIC_QE_NESTED_JSON_INVALID');
  }
  const result = asRecord(parsed, 'Agentic-QE nested result');
  assertExactKeys(result, ['success', 'data', 'metadata'], 'Agentic-QE nested result');
  if (result.success !== true) throw new Error('HARNESS_AGENTIC_QE_NESTED_RESULT_FAILED');
  const metadata = asRecord(result.metadata, 'Agentic-QE nested metadata');
  assertExactKeys(metadata, [
    'executionTime', 'timestamp', 'requestId', 'domain', 'toolName', 'dataSource',
  ], 'Agentic-QE nested metadata');
  finiteNumber(metadata.executionTime, 'metadata.executionTime', 0);
  canonicalAgenticQeTimestamp(new Date(asNonEmptyString(metadata.timestamp, 'metadata.timestamp')));
  boundedText(metadata.requestId, 'metadata.requestId', 512);
  if (metadata.domain !== 'coverage-analysis'
    || metadata.toolName !== AGENTIC_QE_LCOV_GAPS_TOOL
    || metadata.dataSource !== 'real') {
    throw new Error('HARNESS_AGENTIC_QE_REAL_TOOL_PROVENANCE_INVALID');
  }
  return deepFreeze({
    success: true,
    data: parseGapData(result.data),
    metadata: {
      domain: 'coverage-analysis',
      toolName: AGENTIC_QE_LCOV_GAPS_TOOL,
      dataSource: 'real',
    },
  });
}

export function canonicalAgenticQeTimestamp(value: Date): string {
  if (!(value instanceof Date) || !Number.isFinite(value.getTime())) {
    throw new TypeError('HARNESS_AGENTIC_QE_TIMESTAMP_INVALID');
  }
  return value.toISOString();
}

function parseGapData(value: unknown): unknown {
  const data = asRecord(value, 'Agentic-QE gap data');
  assertExactKeys(
    data,
    ['gaps', 'totalGaps', 'criticalGaps', 'suggestedTests'],
    'Agentic-QE gap data',
  );
  if (!Array.isArray(data.gaps) || data.gaps.length > MAX_GAPS) {
    throw new TypeError('Agentic-QE gap data.gaps is invalid');
  }
  const gaps = data.gaps.map((gap, index) => parseGap(gap, index));
  const totalGaps = asInteger(data.totalGaps, 'Agentic-QE gap data.totalGaps');
  const criticalGaps = asInteger(data.criticalGaps, 'Agentic-QE gap data.criticalGaps');
  if (totalGaps < gaps.length
    || criticalGaps !== gaps.filter(({ severity }) => severity === 'critical').length) {
    throw new TypeError('Agentic-QE gap counts are inconsistent');
  }
  if (!Array.isArray(data.suggestedTests) || data.suggestedTests.length > 5) {
    throw new TypeError('Agentic-QE suggestedTests is invalid');
  }
  const suggestedTests = data.suggestedTests.map((rawSuggestion, index) => {
    const suggestion = asRecord(rawSuggestion, `suggestedTests[${index}]`);
    assertExactKeys(
      suggestion,
      ['file', 'description', 'estimatedCoverageGain', 'priority'],
      `suggestedTests[${index}]`,
    );
    return {
      file: boundedPath(suggestion.file, `suggestedTests[${index}].file`),
      description: boundedText(
        suggestion.description,
        `suggestedTests[${index}].description`,
        20_000,
      ),
      estimatedCoverageGain: finiteNumber(
        suggestion.estimatedCoverageGain,
        `suggestedTests[${index}].estimatedCoverageGain`,
        0,
        100,
      ),
      priority: asInteger(suggestion.priority, `suggestedTests[${index}].priority`, 1),
    };
  });
  return { gaps, totalGaps, criticalGaps, suggestedTests };
}

function parseGap(value: unknown, index: number): {
  file: string;
  lines: number[];
  type: string;
  severity: string;
  riskScore: number;
  reason: string;
} {
  const gap = asRecord(value, `gaps[${index}]`);
  assertExactKeys(
    gap,
    ['file', 'lines', 'type', 'severity', 'riskScore', 'reason'],
    `gaps[${index}]`,
  );
  if (!Array.isArray(gap.lines) || gap.lines.length > 10_000) {
    throw new TypeError(`gaps[${index}].lines is invalid`);
  }
  const lines = gap.lines.map((line, lineIndex) =>
    asInteger(line, `gaps[${index}].lines[${lineIndex}]`, 1));
  if (new Set(lines).size !== lines.length) throw new TypeError(`gaps[${index}].lines has duplicates`);
  if (!['uncovered-line', 'uncovered-branch', 'uncovered-function'].includes(gap.type as string)) {
    throw new TypeError(`gaps[${index}].type is invalid`);
  }
  if (!['critical', 'high', 'medium', 'low'].includes(gap.severity as string)) {
    throw new TypeError(`gaps[${index}].severity is invalid`);
  }
  return {
    file: boundedPath(gap.file, `gaps[${index}].file`),
    lines,
    type: gap.type as string,
    severity: gap.severity as string,
    riskScore: finiteNumber(gap.riskScore, `gaps[${index}].riskScore`, 0, 1),
    reason: boundedText(gap.reason, `gaps[${index}].reason`, 20_000),
  };
}

function boundedPath(value: unknown, label: string): string {
  const path = boundedText(value, label, 4096);
  if (/[\0\r\n]/.test(path)) throw new TypeError(`${label} contains control characters`);
  return path;
}

function boundedText(value: unknown, label: string, maxBytes: number): string {
  const text = asNonEmptyString(value, label);
  if (Buffer.byteLength(text, 'utf8') > maxBytes || text.includes('\0')) {
    throw new TypeError(`${label} exceeds its bound or contains NUL`);
  }
  return text;
}

function finiteNumber(
  value: unknown,
  label: string,
  minimum: number,
  maximum = Number.MAX_VALUE,
): number {
  if (typeof value !== 'number'
    || !Number.isFinite(value)
    || value < minimum
    || value > maximum) {
    throw new TypeError(`${label} is outside its allowed range`);
  }
  return value;
}
