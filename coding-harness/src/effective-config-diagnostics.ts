// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { realpathSync } from 'node:fs';
import { resolve } from 'node:path';
import { asNonEmptyString, asRecord, deepFreeze } from './contracts.js';
import type {
  AuditSeverity,
  ConfigurationScope,
  UpstreamDiagnostic,
  UpstreamDiagnosticTool,
} from './effective-config.js';

const MAX_RAW_DIAGNOSTIC_BYTES = 2_000_000;
const VERSION = /^ruflo-metaharness@\d+\.\d+\.\d+\/metaharness@\d+\.\d+\.\d+$/;

export interface CapturedUpstreamDiagnostic {
  readonly target: ConfigurationScope;
  readonly tool: UpstreamDiagnosticTool;
  readonly toolVersion: string;
  readonly exitCode: number;
  readonly rawOutput: string;
}

export function deriveUpstreamDiagnostics(input: Readonly<{
  repositoryRoot: string;
  snapshotDigest: string;
  captures: readonly CapturedUpstreamDiagnostic[];
}>): ReadonlyMap<ConfigurationScope, readonly UpstreamDiagnostic[]> {
  if (!/^[a-f0-9]{64}$/.test(input.snapshotDigest) || input.snapshotDigest === '0'.repeat(64)) {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_SNAPSHOT_DIGEST_INVALID');
  }
  const expected = new Set([
    'repository:mcp-scan', 'repository:threat-model',
    'coding-harness:mcp-scan', 'coding-harness:threat-model',
  ]);
  if (input.captures.length !== expected.size) {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_DIAGNOSTICS_REQUIRED');
  }
  const repositoryRoot = realpathSync(input.repositoryRoot);
  const output = new Map<ConfigurationScope, UpstreamDiagnostic[]>();
  for (const capture of input.captures) {
    const key = `${capture.target}:${capture.tool}`;
    if (!expected.delete(key)) throw new Error('HARNESS_EFFECTIVE_CONFIG_DIAGNOSTIC_DUPLICATE');
    const diagnostic = deriveDiagnostic(capture, repositoryRoot, input.snapshotDigest);
    output.set(capture.target, [...(output.get(capture.target) ?? []), diagnostic]);
  }
  if (expected.size !== 0) throw new Error('HARNESS_EFFECTIVE_CONFIG_DIAGNOSTICS_REQUIRED');
  for (const entries of output.values()) entries.sort((left, right) => left.tool.localeCompare(right.tool));
  return output;
}

function deriveDiagnostic(
  capture: CapturedUpstreamDiagnostic,
  repositoryRoot: string,
  snapshotDigest: string,
): UpstreamDiagnostic {
  if (!VERSION.test(capture.toolVersion)) throw new Error('HARNESS_EFFECTIVE_CONFIG_TOOL_VERSION_INVALID');
  if (!Number.isSafeInteger(capture.exitCode) || capture.exitCode < 0 || capture.exitCode > 255) {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_DIAGNOSTIC_EXIT_INVALID');
  }
  if (typeof capture.rawOutput !== 'string' || capture.rawOutput.trim() === ''
    || Buffer.byteLength(capture.rawOutput, 'utf8') > MAX_RAW_DIAGNOSTIC_BYTES
    || capture.rawOutput.includes('\0')) {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_DIAGNOSTIC_OUTPUT_INVALID');
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(capture.rawOutput) as unknown;
  } catch {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_DIAGNOSTIC_JSON_INVALID');
  }
  const envelope = asRecord(parsed, 'upstream diagnostic envelope');
  if (envelope.degraded !== false || envelope.exitCode !== capture.exitCode) {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_DIAGNOSTIC_ENVELOPE_INVALID');
  }
  const data = asRecord(envelope.data, 'upstream diagnostic data');
  const expectedRoot = capture.target === 'repository'
    ? repositoryRoot : resolve(repositoryRoot, 'coding-harness');
  if (realpathSync(asNonEmptyString(data.dir, 'upstream diagnostic data.dir')) !== expectedRoot) {
    throw new Error('HARNESS_EFFECTIVE_CONFIG_DIAGNOSTIC_TARGET_MISMATCH');
  }
  const worstSeverity = severity(data.worst);
  const mcpEnabled = capture.tool === 'mcp-scan'
    ? boolean(data.mcpEnabled, 'mcpEnabled')
    : boolean(data.mcpInUse, 'mcpInUse');
  const reportedVerdict = capture.tool === 'threat-model'
    ? threatVerdict(data.verdict)
    : worstSeverity === 'medium' || worstSeverity === 'high' ? 'findings' : 'clean';
  // A caller-supplied capture can preserve a negative finding, but it cannot
  // attest that a clean tool result ran against this snapshot. Only the
  // trusted in-process diagnostic runner may mint a clean verdict.
  const verdict = reportedVerdict === 'clean' ? 'inconclusive' : reportedVerdict;
  const rawDigest = digest(capture.rawOutput);
  return deepFreeze({
    target: capture.target,
    tool: capture.tool,
    mcpEnabled,
    worstSeverity,
    verdict,
    toolVersion: capture.toolVersion,
    rawDigest,
    invocationId: digest({
      snapshotDigest, target: capture.target, tool: capture.tool,
      exitCode: capture.exitCode, rawDigest, toolVersion: capture.toolVersion,
    }),
    exitCode: capture.exitCode,
    degraded: false,
  });
}

function severity(value: unknown): AuditSeverity {
  if (value === 'clean' || value === 'info') return 'info';
  if (value === 'low') return 'low';
  if (value === 'medium' || value === 'warn') return 'medium';
  if (value === 'high' || value === 'error' || value === 'critical') return 'high';
  throw new Error('HARNESS_EFFECTIVE_CONFIG_DIAGNOSTIC_SEVERITY_INVALID');
}

function threatVerdict(value: unknown): UpstreamDiagnostic['verdict'] {
  if (value === 'clean') return 'clean';
  if (value === 'findings' || value === 'low' || value === 'medium' || value === 'high') {
    return 'findings';
  }
  if (value === 'inconclusive' || value === 'needs-work' || value === 'blocked') {
    return 'inconclusive';
  }
  throw new Error('HARNESS_EFFECTIVE_CONFIG_DIAGNOSTIC_VERDICT_INVALID');
}

function boolean(value: unknown, label: string): boolean {
  if (typeof value !== 'boolean') throw new Error(`HARNESS_EFFECTIVE_CONFIG_DIAGNOSTIC_${label.toUpperCase()}_INVALID`);
  return value;
}

function digest(value: unknown): string {
  const content = typeof value === 'string' ? value : JSON.stringify(value);
  return createHash('sha256').update(content, 'utf8').digest('hex');
}
