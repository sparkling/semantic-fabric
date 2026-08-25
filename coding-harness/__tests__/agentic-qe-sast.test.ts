// SPDX-License-Identifier: MIT

import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  AGENTIC_QE_SAST_PROFILE,
  AGENTIC_QE_SAST_TOOL,
  AgenticQeSastEvidenceAdapter,
  type AgenticQeSastRunnerIdentity,
  type ProviderFreeAgenticQeSastMcpRequest,
} from '../src/agentic-qe-sast.js';
import { parseAgenticQeEvidence } from '../src/evidence.js';
import { runGitCommand } from '../src/git-process.js';
import { digestValue } from '../src/receipts.js';

const FIXED_TIME = new Date('2026-08-25T14:00:00.000Z');
const TASK_ID = 'task-aqe-sast-0001';
const RUN_ID = 'run-aqe-sast-0001';
const roots: string[] = [];

afterEach(async () => {
  await Promise.all(roots.splice(0).map(async (root) => await rm(root, {
    recursive: true,
    force: true,
  })));
});

describe('Agentic-QE provider-free SAST evidence adapter', () => {
  it('scans an exact read-only candidate snapshot and binds task, run, tree, and runner', async () => {
    const fixture = await repositoryFixture();
    let request: ProviderFreeAgenticQeSastMcpRequest | undefined;
    let snapshotContainedPoison = true;
    const runner = fakeRunner(async (actual) => {
      request = actual;
      snapshotContainedPoison = existsSync(join(actual.arguments.target, 'target/poison.rs'));
      expect(await Bunless.read(join(actual.arguments.target, 'src/lib.rs'))).toContain('password');
      return successfulResponse();
    }, fixture.root);
    const evidence = await adapter(runner).capture(fixture.input);

    expect(request).toMatchObject({
      executable: 'aqe-mcp',
      transport: 'stdio-mcp',
      method: 'tools/call',
      toolName: AGENTIC_QE_SAST_TOOL,
      arguments: {
        sast: true,
        dast: false,
      },
      bindings: {
        taskId: TASK_ID,
        runId: RUN_ID,
        candidateTree: fixture.candidateTree,
        snapshotSha256: expect.stringMatching(/^[a-f0-9]{64}$/),
        profile: AGENTIC_QE_SAST_PROFILE,
      },
      runtime: {
        network: 'offline',
        environment: { inheritance: 'none' },
        filesystem: {
          inputAccess: 'read-only',
          privateHome: true,
          privateWritableTmp: true,
        },
      },
    });
    expect(request!.runtime.filesystem.readOnlyPaths).toEqual([request!.arguments.target]);
    expect(request!.arguments.target).not.toBe(fixture.root);
    expect(existsSync(request!.arguments.target)).toBe(false);
    expect(snapshotContainedPoison).toBe(false);
    expect(JSON.stringify(request)).not.toMatch(/OPENAI|ANTHROPIC|OPENROUTER|API_KEY|dependency/i);
    expect(evidence).toMatchObject({
      source: 'agentic-qe-local-profile',
      profile: 'sast',
      taskId: TASK_ID,
      runId: RUN_ID,
      candidateTree: fixture.candidateTree,
      commandDigest: expect.stringMatching(/^[a-f0-9]{64}$/),
      outputDigest: expect.stringMatching(/^[a-f0-9]{64}$/),
      providerVariablesStripped: true,
      authoritative: false,
      capturedAt: FIXED_TIME.toISOString(),
    });
    expect(parseAgenticQeEvidence(evidence)).toEqual(evidence);
  });

  it('normalizes volatile server metadata and finding order before digesting output', async () => {
    const fixture = await repositoryFixture();
    const first = await adapter(fakeRunner(async () => successfulResponse({
      executionTime: 1,
      taskId: 'request-one',
      reverseFindings: false,
    }), fixture.root)).capture(fixture.input);
    const second = await adapter(fakeRunner(async () => successfulResponse({
      executionTime: 9_999,
      taskId: 'request-two',
      reverseFindings: true,
    }), fixture.root)).capture(fixture.input);

    expect(second.outputDigest).toBe(first.outputDigest);
  });

  it('fails closed when a runner mutates the exported source snapshot', async () => {
    const fixture = await repositoryFixture();
    const runner = fakeRunner(async (request) => {
      await writeFile(join(request.arguments.target, 'src/lib.rs'), 'tampered\n');
      return successfulResponse();
    }, fixture.root);

    await expect(adapter(runner).capture(fixture.input)).rejects.toThrow(
      'HARNESS_AGENTIC_QE_SAST_SNAPSHOT_CHANGED',
    );
  });

  it('fails closed when runner identity changes across the invocation', async () => {
    const fixture = await repositoryFixture();
    const runner = fakeRunner(async () => successfulResponse(), fixture.root);
    let calls = 0;
    const originalIdentity = runner.identityEvidence;
    runner.identityEvidence = () => {
      const identity = originalIdentity();
      calls += 1;
      return calls === 1 ? identity : {
        ...identity,
        package: { ...identity.package, treeSha256: 'f'.repeat(64) },
      };
    };

    await expect(adapter(runner).capture(fixture.input)).rejects.toThrow(
      'HARNESS_AGENTIC_QE_SAST_RUNNER_CHANGED',
    );
  });

  it.each([
    ['reported failure', { success: false }, 'HARNESS_AGENTIC_QE_SAST_RESULT_FAILED'],
    ['bad status', { status: 'failed' }, 'HARNESS_AGENTIC_QE_SAST_STATUS_INVALID'],
    ['inconsistent summary', { vulnerabilities: 1 }, 'HARNESS_AGENTIC_QE_SAST_SUMMARY_INCONSISTENT'],
  ])('rejects %s output', async (_name, override, error) => {
    const fixture = await repositoryFixture();
    await expect(adapter(fakeRunner(
      async () => successfulResponse(override),
      fixture.root,
    )).capture(fixture.input)).rejects.toThrow(error);
  });

  it('rejects a stale candidate identity before invoking Agentic-QE', async () => {
    const fixture = await repositoryFixture();
    const runner = fakeRunner(async () => successfulResponse(), fixture.root);

    await expect(adapter(runner).capture({
      ...fixture.input,
      candidateTree: 'a'.repeat(40),
    })).rejects.toThrow('HARNESS_AGENTIC_QE_SAST_CANDIDATE_TREE_MISMATCH');
    expect(runner.invoke).not.toHaveBeenCalled();
  });
});

function adapter(runner: ReturnType<typeof fakeRunner>): AgenticQeSastEvidenceAdapter {
  return new AgenticQeSastEvidenceAdapter({ runner, clock: () => FIXED_TIME });
}

function fakeRunner(
  implementation: (request: ProviderFreeAgenticQeSastMcpRequest) => Promise<unknown>,
  root = '/tmp/semantic-fabric-aqe-sast-test',
) {
  const identity: AgenticQeSastRunnerIdentity = {
    node: { path: join(root, 'tools/node'), sha256: '1'.repeat(64) },
    bwrap: { path: join(root, 'tools/bwrap'), sha256: '2'.repeat(64) },
    package: {
      root: join(root, 'tools/agentic-qe'),
      entryPath: join(root, 'tools/agentic-qe/dist/mcp/bundle.js'),
      name: 'agentic-qe',
      version: '3.13.10-test',
      entrySha256: '3'.repeat(64),
      treeSha256: '4'.repeat(64),
      fileCount: 42,
      totalBytes: 42_000,
    },
  };
  return {
    invoke: vi.fn(implementation),
    identityEvidence: () => identity,
    commandDigest: (request: ProviderFreeAgenticQeSastMcpRequest) =>
      digestValue({ executable: identity.bwrap, request, identity }),
  };
}

function successfulResponse(overrides: Readonly<{
  success?: boolean;
  executionTime?: number;
  taskId?: string;
  status?: string;
  vulnerabilities?: number;
  reverseFindings?: boolean;
}> = {}): unknown {
  const findings = [{
    type: 'Hardcoded secret',
    severity: 'high',
    file: 'src/lib.rs',
    line: 1,
    description: 'Secret-like literal',
  }, {
    type: 'Unsafe block',
    severity: 'low',
    file: 'src/unsafe.rs',
    line: 2,
    description: 'Unsafe code requires review',
  }];
  if (overrides.reverseFindings === true) findings.reverse();
  const nested = {
    success: overrides.success ?? true,
    data: {
      taskId: overrides.taskId ?? 'scan-request-0001',
      status: overrides.status ?? 'completed',
      vulnerabilities: overrides.vulnerabilities ?? 2,
      critical: 0,
      high: 1,
      medium: 0,
      low: 1,
      topVulnerabilities: findings,
      recommendations: ['Review high-severity findings'],
      duration: overrides.executionTime ?? 15,
    },
  };
  return { content: [{ type: 'text', text: JSON.stringify(nested) }] };
}

async function repositoryFixture() {
  const root = await mkdtemp(join(tmpdir(), 'semantic-fabric-aqe-sast-fixture-'));
  roots.push(root);
  await git(root, ['init', '--quiet']);
  await mkdir(join(root, 'src'));
  await writeFile(join(root, '.gitignore'), 'target/\n');
  await writeFile(join(root, 'src/lib.rs'), 'pub fn safe() -> bool { true }\n');
  await git(root, ['add', '--', '.gitignore', 'src/lib.rs']);
  await git(root, ['commit', '--quiet', '-m', 'fixture'], {
    GIT_AUTHOR_NAME: 'Harness Test',
    GIT_AUTHOR_EMAIL: 'harness@example.invalid',
    GIT_AUTHOR_DATE: '2000-01-01T00:00:00Z',
    GIT_COMMITTER_NAME: 'Harness Test',
    GIT_COMMITTER_EMAIL: 'harness@example.invalid',
    GIT_COMMITTER_DATE: '2000-01-01T00:00:00Z',
  });
  await writeFile(join(root, 'src/lib.rs'), 'pub const password: &str = "not-a-real-secret";\n');
  await git(root, ['add', '--', 'src/lib.rs']);
  await mkdir(join(root, 'target'));
  await writeFile(join(root, 'target/poison.rs'), 'const TOKEN: &str = "ignored-poison";\n');
  const candidateTree = (await git(root, ['write-tree'])).stdout.trim();
  return {
    root,
    candidateTree,
    input: { taskId: TASK_ID, runId: RUN_ID, candidateTree, candidateRoot: root },
  };
}

const Bunless = {
  async read(path: string): Promise<string> {
    const { readFile } = await import('node:fs/promises');
    return await readFile(path, 'utf8');
  },
};

async function git(
  root: string,
  args: readonly string[],
  environment?: Readonly<Record<string, string>>,
) {
  const result = await runGitCommand(root, args, { environment });
  if (result.exitCode !== 0) throw new Error(`fixture Git failed: ${result.stderr}`);
  return result;
}
