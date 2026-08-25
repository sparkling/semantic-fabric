// SPDX-License-Identifier: MIT

import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  AGENTIC_QE_LCOV_GAPS_TOOL,
  AgenticQeLcovGapEvidenceAdapter,
  type AgenticQeLcovGapInput,
  type ProviderFreeAgenticQeMcpRequest,
} from '../src/agentic-qe-lcov.js';
import { parseAgenticQeEvidence } from '../src/evidence.js';
import { runGitCommand } from '../src/git-process.js';

const FIXED_TIME = new Date('2026-08-25T12:00:00.000Z');
const TASK_ID = 'task-aqe-0001';
const RUN_ID = 'run-aqe-0001';
const roots: string[] = [];

interface Fixture {
  readonly root: string;
  readonly lcovPath: string;
  readonly candidateTree: string;
  readonly input: AgenticQeLcovGapInput;
}

afterEach(async () => {
  await Promise.all(roots.splice(0).map(async (root) => await rm(root, {
    recursive: true,
    force: true,
  })));
});

describe('Agentic-QE real-LCOV evidence adapter', () => {
  it('invokes only the provider-free coverage-gap tool and binds every input identity', async () => {
    const fixture = await repositoryFixture();
    let request: ProviderFreeAgenticQeMcpRequest | undefined;
    const invoke = vi.fn(async (actual: ProviderFreeAgenticQeMcpRequest) => {
      request = actual;
      return successfulResponse();
    });
    const evidence = await adapter(invoke).capture(fixture.input);

    expect(invoke).toHaveBeenCalledOnce();
    expect(request).toMatchObject({
      executable: 'aqe-mcp',
      transport: 'stdio-mcp',
      method: 'tools/call',
      toolName: AGENTIC_QE_LCOV_GAPS_TOOL,
      arguments: {
        target: fixture.root,
        coverageFile: fixture.lcovPath,
        coverageFormat: 'lcov',
        language: 'rust',
        minRisk: 0,
        limit: 20,
        prioritization: 'complexity',
        includeGhost: false,
      },
      bindings: {
        taskId: TASK_ID,
        runId: RUN_ID,
        candidateTree: fixture.candidateTree,
        lcovSha256: fixture.input.lcov.sha256,
        coverageCommandDigest: 'c'.repeat(64),
        generatorVersion: 'cargo-llvm-cov 0.8.7',
      },
      runtime: {
        network: 'offline',
        environment: { inheritance: 'none' },
        filesystem: {
          inputAccess: 'read-only',
          readOnlyPaths: [fixture.root, fixture.lcovPath].sort(),
          privateHome: true,
          privateWritableTmp: true,
        },
      },
    });
    expect(JSON.stringify(request)).not.toMatch(/OPENAI|ANTHROPIC|OPENROUTER|API_KEY|testgen|sast/i);
    expect(evidence).toMatchObject({
      source: 'agentic-qe-local-profile',
      profile: 'lcov-gap',
      taskId: TASK_ID,
      runId: RUN_ID,
      candidateTree: fixture.candidateTree,
      providerVariablesStripped: true,
      authoritative: false,
      capturedAt: FIXED_TIME.toISOString(),
    });
    expect(parseAgenticQeEvidence(evidence)).toEqual(evidence);
  });

  it('normalizes volatile AQE metadata before digesting the advisory result', async () => {
    const fixture = await repositoryFixture();
    const first = await adapter(async () => successfulResponse({
      executionTime: 2,
      timestamp: '2026-08-25T12:00:01.000Z',
      requestId: 'request-one',
    })).capture(fixture.input);
    const second = await adapter(async () => successfulResponse({
      executionTime: 9_999,
      timestamp: '2026-08-25T13:45:00.000Z',
      requestId: 'request-two',
    })).capture(fixture.input);

    expect(second.outputDigest).toBe(first.outputDigest);
    expect(second.commandDigest).toBe(first.commandDigest);
  });

  it.each([
    ['nested failure', { success: false }, 'HARNESS_AGENTIC_QE_NESTED_RESULT_FAILED'],
    ['demo data', { dataSource: 'demo' }, 'HARNESS_AGENTIC_QE_REAL_TOOL_PROVENANCE_INVALID'],
    ['wrong tool', { toolName: 'qe/coverage/analyze' }, 'HARNESS_AGENTIC_QE_REAL_TOOL_PROVENANCE_INVALID'],
  ])('rejects %s instead of admitting it as real gap evidence', async (_name, response, error) => {
    const fixture = await repositoryFixture();
    await expect(adapter(async () => successfulResponse(response)).capture(fixture.input))
      .rejects.toThrow(error);
  });

  it('rejects an LCOV digest mismatch before invoking AQE', async () => {
    const fixture = await repositoryFixture();
    const invoke = vi.fn(async () => successfulResponse());
    const input = {
      ...fixture.input,
      lcov: { ...fixture.input.lcov, sha256: 'a'.repeat(64) },
    };

    await expect(adapter(invoke).capture(input)).rejects.toThrow(
      'HARNESS_AGENTIC_QE_LCOV_DIGEST_MISMATCH',
    );
    expect(invoke).not.toHaveBeenCalled();
  });

  it('fails closed when the runner changes the independently generated LCOV', async () => {
    const fixture = await repositoryFixture();
    const invoke = vi.fn(async () => {
      await writeFile(fixture.lcovPath, `${lcovFor(fixture.root)}TN:changed\n`);
      return successfulResponse();
    });

    await expect(adapter(invoke).capture(fixture.input)).rejects.toThrow(
      'HARNESS_AGENTIC_QE_LCOV_CHANGED',
    );
  });

  it('binds the output digest to the exact LCOV bytes', async () => {
    const fixture = await repositoryFixture();
    const evidence = await adapter(async () => successfulResponse()).capture(fixture.input);
    const changedLcov = lcovFor(fixture.root).replace('DA:2,0', 'DA:2,1');
    await writeFile(fixture.lcovPath, changedLcov);
    const changedInput = {
      ...fixture.input,
      lcov: {
        ...fixture.input.lcov,
        sha256: sha256(changedLcov),
      },
    };
    const changed = await adapter(async () => successfulResponse()).capture(changedInput);

    expect(changed.outputDigest).not.toBe(evidence.outputDigest);
    expect(changed.commandDigest).not.toBe(evidence.commandDigest);
  });

  it('rejects coverage without the independent-direct-coverage provenance', async () => {
    const fixture = await repositoryFixture();
    const input = {
      ...fixture.input,
      lcov: { ...fixture.input.lcov, provenance: 'aqe-generated' },
    } as unknown as AgenticQeLcovGapInput;

    await expect(adapter(async () => successfulResponse()).capture(input)).rejects.toThrow(
      'HARNESS_AGENTIC_QE_LCOV_PROVENANCE_INVALID',
    );
  });

  it.each([
    [
      { coverageCommandDigest: 'not-a-digest' },
      'HARNESS_AGENTIC_QE_COVERAGE_COMMAND_DIGEST_INVALID',
    ],
    [
      { generatorVersion: 'cargo-llvm-cov 0.8.6' },
      'HARNESS_AGENTIC_QE_COVERAGE_GENERATOR_INVALID',
    ],
  ])('rejects untrusted direct-coverage bindings', async (override, error) => {
    const fixture = await repositoryFixture();
    const input = {
      ...fixture.input,
      lcov: { ...fixture.input.lcov, ...override },
    } as AgenticQeLcovGapInput;

    await expect(adapter(async () => successfulResponse()).capture(input)).rejects.toThrow(error);
  });
});

function adapter(
  invoke: (request: ProviderFreeAgenticQeMcpRequest, signal?: AbortSignal) => Promise<unknown>,
): AgenticQeLcovGapEvidenceAdapter {
  return new AgenticQeLcovGapEvidenceAdapter({
    runner: { invoke },
    clock: () => FIXED_TIME,
  });
}

function successfulResponse(overrides: Readonly<{
  success?: boolean;
  executionTime?: number;
  timestamp?: string;
  requestId?: string;
  toolName?: string;
  dataSource?: string;
}> = {}): unknown {
  const nested = {
    success: overrides.success ?? true,
    data: {
      gaps: [{
        file: 'src/lib.rs',
        lines: [2],
        type: 'uncovered-line',
        severity: 'high',
        riskScore: 0.8,
        reason: 'Line 2 is not covered',
      }],
      totalGaps: 1,
      criticalGaps: 0,
      suggestedTests: [{
        file: 'src/lib.rs',
        description: 'Add a test for line 2',
        estimatedCoverageGain: 50,
        priority: 1,
      }],
    },
    metadata: {
      executionTime: overrides.executionTime ?? 15,
      timestamp: overrides.timestamp ?? '2026-08-25T12:00:01.000Z',
      requestId: overrides.requestId ?? 'aqe-request-0001',
      domain: 'coverage-analysis',
      toolName: overrides.toolName ?? AGENTIC_QE_LCOV_GAPS_TOOL,
      dataSource: overrides.dataSource ?? 'real',
    },
  };
  return { content: [{ type: 'text', text: JSON.stringify(nested) }] };
}

async function repositoryFixture(): Promise<Fixture> {
  const root = await mkdtemp(join(tmpdir(), 'semantic-fabric-aqe-lcov-'));
  roots.push(root);
  await git(root, ['init', '--quiet']);
  await mkdir(join(root, 'src'));
  await mkdir(join(root, 'coverage'));
  await writeFile(join(root, 'src/lib.rs'), 'pub fn covered() -> bool { true }\n');
  await git(root, ['add', '--', 'src/lib.rs']);
  await git(root, ['commit', '--quiet', '-m', 'fixture'], {
    GIT_AUTHOR_NAME: 'Harness Test',
    GIT_AUTHOR_EMAIL: 'harness@example.invalid',
    GIT_AUTHOR_DATE: '2000-01-01T00:00:00Z',
    GIT_COMMITTER_NAME: 'Harness Test',
    GIT_COMMITTER_EMAIL: 'harness@example.invalid',
    GIT_COMMITTER_DATE: '2000-01-01T00:00:00Z',
  });
  const candidateTree = (await git(root, ['write-tree'])).stdout.trim();
  const lcovPath = join(root, 'coverage/lcov.info');
  const lcov = lcovFor(root);
  await writeFile(lcovPath, lcov);
  return {
    root,
    lcovPath,
    candidateTree,
    input: {
      taskId: TASK_ID,
      runId: RUN_ID,
      candidateTree,
      candidateRoot: root,
      lcov: {
        path: lcovPath,
        sha256: sha256(lcov),
        provenance: 'independent-direct-coverage',
        coverageCommandDigest: 'c'.repeat(64),
        generatorVersion: 'cargo-llvm-cov 0.8.7',
      },
    },
  };
}

function lcovFor(root: string): string {
  return [
    'TN:',
    `SF:${join(root, 'src/lib.rs')}`,
    'FN:1,covered',
    'FNDA:1,covered',
    'FNF:1',
    'FNH:1',
    'DA:1,1',
    'DA:2,0',
    'LF:2',
    'LH:1',
    'end_of_record',
    '',
  ].join('\n');
}

function sha256(value: string): string {
  return createHash('sha256').update(value).digest('hex');
}

async function git(
  root: string,
  args: readonly string[],
  environment?: Readonly<Record<string, string>>,
) {
  const result = await runGitCommand(root, args, { environment });
  if (result.exitCode !== 0) throw new Error(`fixture Git failed: ${result.stderr}`);
  return result;
}
