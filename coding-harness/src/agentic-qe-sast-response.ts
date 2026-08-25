// SPDX-License-Identifier: MIT

import {
  asInteger,
  asNonEmptyString,
  asRecord,
  assertExactKeys,
  deepFreeze,
} from './contracts.js';
import { isAbsolute, relative, sep } from 'node:path';

export const AGENTIC_QE_SAST_TOOL = 'security_scan_comprehensive' as const;
const MAX_FINDINGS = 2_500;
const SEVERITIES = ['critical', 'high', 'medium', 'low', 'informational'] as const;
type Severity = typeof SEVERITIES[number];
type NormalizedVulnerability = Readonly<Record<string, unknown> & { severity: Severity }>;

export interface NormalizedAgenticQeSastResult {
  readonly success: true;
  readonly data: Readonly<{
    summary: Readonly<{
      vulnerabilities: number;
      critical: number;
      high: number;
      medium: number;
      low: number;
    }>;
    vulnerabilities: readonly unknown[];
    recommendations: readonly string[];
  }>;
}

export function parseNestedSastResult(
  value: unknown,
  expectedRoot?: string,
): NormalizedAgenticQeSastResult {
  const outer = asRecord(value, 'Agentic-QE SAST MCP response');
  const allowedOuterKeys = new Set(['content', 'isError']);
  if (Object.keys(outer).some((key) => !allowedOuterKeys.has(key)) || !('content' in outer)) {
    throw new TypeError('Agentic-QE SAST MCP response has invalid keys');
  }
  if (outer.isError !== undefined && outer.isError !== false) {
    throw new Error('HARNESS_AGENTIC_QE_SAST_MCP_ERROR');
  }
  if (!Array.isArray(outer.content) || outer.content.length !== 1) {
    throw new TypeError('Agentic-QE SAST MCP response must contain one text block');
  }
  const block = asRecord(outer.content[0], 'Agentic-QE SAST MCP response block');
  assertExactKeys(block, ['type', 'text'], 'Agentic-QE SAST MCP response block');
  if (block.type !== 'text') throw new TypeError('Agentic-QE SAST MCP response block must be text');
  const text = boundedText(block.text, 'Agentic-QE SAST MCP response text', 5_000_000);
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new TypeError('HARNESS_AGENTIC_QE_SAST_NESTED_JSON_INVALID');
  }
  const result = asRecord(parsed, 'Agentic-QE SAST nested result');
  assertExactKeys(result, ['success', 'data'], 'Agentic-QE SAST nested result');
  if (result.success !== true) throw new Error('HARNESS_AGENTIC_QE_SAST_RESULT_FAILED');
  const data = parseData(result.data, expectedRoot);
  return deepFreeze({ success: true, data });
}

function parseData(value: unknown, expectedRoot?: string): NormalizedAgenticQeSastResult['data'] {
  const data = asRecord(value, 'Agentic-QE SAST data');
  const allowed = new Set([
    'taskId', 'status', 'vulnerabilities', 'critical', 'high', 'medium', 'low',
    'topVulnerabilities', 'recommendations', 'duration', 'savedFiles',
  ]);
  const required = [
    'taskId', 'status', 'vulnerabilities', 'critical', 'high', 'medium', 'low',
    'topVulnerabilities', 'recommendations', 'duration',
  ];
  if (Object.keys(data).some((key) => !allowed.has(key))
    || required.some((key) => !Object.prototype.hasOwnProperty.call(data, key))) {
    throw new TypeError('Agentic-QE SAST data has invalid keys');
  }
  boundedText(data.taskId, 'Agentic-QE SAST data.taskId', 512);
  if (data.status !== 'completed') throw new Error('HARNESS_AGENTIC_QE_SAST_STATUS_INVALID');
  finiteNumber(data.duration, 'Agentic-QE SAST data.duration', 0);
  if (data.savedFiles !== undefined
    && (!Array.isArray(data.savedFiles) || data.savedFiles.length > 100)) {
    throw new TypeError('Agentic-QE SAST data.savedFiles is invalid');
  }
  if (!Array.isArray(data.topVulnerabilities) || data.topVulnerabilities.length > 10) {
    throw new TypeError('Agentic-QE SAST vulnerabilities are invalid');
  }
  const vulnerabilities = data.topVulnerabilities
    .map((finding, index) => parseVulnerability(finding, index, expectedRoot))
    .sort(stableCompare);
  const summary = parseSummary(data);
  if (!Array.isArray(data.recommendations) || data.recommendations.length > 20) {
    throw new TypeError('Agentic-QE SAST recommendations are invalid');
  }
  const recommendations = data.recommendations
    .map((item, index) => boundedText(item, `recommendations[${index}]`, 20_000))
    .sort();
  return { summary, vulnerabilities, recommendations };
}

function parseSummary(
  data: Readonly<Record<string, unknown>>,
): NormalizedAgenticQeSastResult['data']['summary'] {
  const output = {
    vulnerabilities: boundedCount(data.vulnerabilities, 'data.vulnerabilities', MAX_FINDINGS),
    critical: boundedCount(data.critical, 'data.critical', MAX_FINDINGS),
    high: boundedCount(data.high, 'data.high', MAX_FINDINGS),
    medium: boundedCount(data.medium, 'data.medium', MAX_FINDINGS),
    low: boundedCount(data.low, 'data.low', MAX_FINDINGS),
  };
  const classified = output.critical + output.high + output.medium + output.low;
  if (classified > output.vulnerabilities) {
    throw new Error('HARNESS_AGENTIC_QE_SAST_SUMMARY_INCONSISTENT');
  }
  return output;
}

function parseVulnerability(
  value: unknown,
  index: number,
  expectedRoot?: string,
): NormalizedVulnerability {
  const finding = asRecord(value, `vulnerabilities[${index}]`);
  const allowed = new Set(['type', 'severity', 'file', 'line', 'description']);
  const required = ['type', 'severity', 'file', 'description'];
  if (Object.keys(finding).some((key) => !allowed.has(key))
    || required.some((key) => !Object.prototype.hasOwnProperty.call(finding, key))) {
    throw new TypeError(`vulnerabilities[${index}] has invalid keys`);
  }
  if (!SEVERITIES.includes(finding.severity as Severity)) {
    throw new TypeError(`vulnerabilities[${index}].severity is invalid`);
  }
  const output: Record<string, unknown> = {
    type: boundedText(finding.type, `vulnerabilities[${index}].type`, 4096),
    severity: finding.severity as Severity,
    file: parsePath(finding.file, `vulnerabilities[${index}].file`, expectedRoot),
    description: boundedText(finding.description, `vulnerabilities[${index}].description`, 20_000),
  };
  if (finding.line !== undefined) output.line = asInteger(finding.line, 'finding.line', 1);
  return output as NormalizedVulnerability;
}

function parsePath(value: unknown, label: string, expectedRoot?: string): string {
  const file = boundedText(value, label, 4096);
  let normalized = file;
  if (isAbsolute(file)) {
    if (expectedRoot === undefined || !isAbsolute(expectedRoot)) throw new TypeError(`${label} is invalid`);
    const delta = relative(expectedRoot, file);
    if (delta === '' || delta === '..' || delta.startsWith(`..${sep}`) || isAbsolute(delta)) {
      throw new TypeError(`${label} is invalid`);
    }
    normalized = delta.split(sep).join('/');
  }
  if (normalized.split(/[\\/]/).includes('..') || /[\0\r\n]/.test(normalized)) {
    throw new TypeError(`${label} is invalid`);
  }
  return normalized;
}

function boundedCount(value: unknown, label: string, maximum: number): number {
  const count = asInteger(value, label, 0);
  if (count > maximum) throw new TypeError(`${label} is outside its allowed range`);
  return count;
}

function boundedText(value: unknown, label: string, maxBytes: number): string {
  const text = asNonEmptyString(value, label);
  if (Buffer.byteLength(text, 'utf8') > maxBytes || text.includes('\0')) {
    throw new TypeError(`${label} exceeds its bound or contains NUL`);
  }
  return text;
}

function finiteNumber(value: unknown, label: string, minimum: number): number {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < minimum) {
    throw new TypeError(`${label} is outside its allowed range`);
  }
  return value;
}

function stableCompare(left: unknown, right: unknown): number {
  const leftValue = JSON.stringify(left);
  const rightValue = JSON.stringify(right);
  return leftValue < rightValue ? -1 : leftValue > rightValue ? 1 : 0;
}
