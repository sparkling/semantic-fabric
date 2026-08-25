// SPDX-License-Identifier: MIT

import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import {
  deriveUpstreamDiagnostics,
  type CapturedUpstreamDiagnostic,
} from '../src/effective-config-diagnostics.js';

const repositoryRoot = fileURLToPath(new URL('../../', import.meta.url));
const version = 'ruflo-metaharness@0.1.1/metaharness@0.3.0';

describe('captured upstream configuration diagnostics', () => {
  it('derives fields and invocation IDs from four non-degraded raw outputs', () => {
    const result = deriveUpstreamDiagnostics({
      repositoryRoot,
      snapshotDigest: 'a'.repeat(64),
      captures: captures(),
    });

    expect(result.get('repository')).toHaveLength(2);
    expect(result.get('coding-harness')).toHaveLength(2);
    expect(result.get('coding-harness')?.find(({ tool }) => tool === 'mcp-scan')).toMatchObject({
      mcpEnabled: true,
      worstSeverity: 'high',
      verdict: 'findings',
      exitCode: 1,
      degraded: false,
    });
    expect(result.get('repository')?.every(({ rawDigest, invocationId }) =>
      /^[a-f0-9]{64}$/.test(rawDigest) && /^[a-f0-9]{64}$/.test(invocationId))).toBe(true);
  });

  it('rejects degraded, target-mismatched, or caller-field diagnostic evidence', () => {
    const degraded = [...captures()];
    degraded[0] = {
      ...degraded[0]!,
      rawOutput: degraded[0]!.rawOutput.replace('"degraded":false', '"degraded":true'),
    };
    expect(() => derive(degraded)).toThrow('DIAGNOSTIC_ENVELOPE_INVALID');

    const mismatch = [...captures()];
    mismatch[0] = {
      ...mismatch[0]!,
      rawOutput: mismatch[0]!.rawOutput.replace(repositoryRoot, '/tmp/not-the-repository'),
    };
    expect(() => derive(mismatch)).toThrow();
  });
});

function derive(capturedDiagnostics: readonly CapturedUpstreamDiagnostic[]) {
  return deriveUpstreamDiagnostics({
    repositoryRoot,
    snapshotDigest: 'a'.repeat(64),
    captures: capturedDiagnostics,
  });
}

function captures(): CapturedUpstreamDiagnostic[] {
  return [
    capture('repository', 'mcp-scan', false, 'info', 0),
    capture('repository', 'threat-model', false, 'info', 0),
    capture('coding-harness', 'mcp-scan', true, 'high', 1),
    capture('coding-harness', 'threat-model', true, 'info', 0),
  ];
}

function capture(
  target: CapturedUpstreamDiagnostic['target'],
  tool: CapturedUpstreamDiagnostic['tool'],
  enabled: boolean,
  worst: string,
  exitCode: number,
): CapturedUpstreamDiagnostic {
  const dir = target === 'repository' ? repositoryRoot : resolve(repositoryRoot, 'coding-harness');
  const data = tool === 'mcp-scan'
    ? { dir, mcpEnabled: enabled, worst }
    : { dir, mcpInUse: enabled, worst, verdict: 'clean' };
  return {
    target,
    tool,
    toolVersion: version,
    exitCode,
    rawOutput: JSON.stringify({ success: exitCode === 0, data, degraded: false, exitCode }),
  };
}
