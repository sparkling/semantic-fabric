// SPDX-License-Identifier: MIT

import { spawnSync } from 'node:child_process';
import { chmodSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { AcceptanceRunner } from '../src/acceptance-runner.js';
import type { AcceptanceTask } from '../src/acceptance-task.js';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import { GitWorktreeSet } from '../src/git-worktrees.js';
import type { OfflineProcessIsolator } from '../src/network.js';
import { digestValue, type CommandEvidence } from '../src/receipts.js';
import { TEST_RESOURCE_SCOPE } from './helpers.js';

const roots: string[] = [];
const redTests = [
  'class_atom_repeated_graph_and_object_variable_is_pruned',
  'class_atom_repeated_predicate_and_object_variable_is_pruned',
  'incompatible_constant_subject_prunes_the_whole_atom_branch',
  'repeated_subject_and_predicate_variable_prunes_incompatible_branches',
];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

function git(cwd: string, args: string[]): string {
  const result = spawnSync('git', args, { cwd, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr);
  return result.stdout.trim();
}

const isolator: OfflineProcessIsolator = {
  assertStable() {},
  async terminateAndVerify() {},
  isolate: (command) => ({
    enforcement: 'os-network-namespace',
    mechanism: 'test-no-network',
    resourceScope: TEST_RESOURCE_SCOPE,
    command: { ...command, executable: '/usr/bin/env', args: [command.executable, ...command.args] },
  }),
};

describe('issue #8 acceptance gates', () => {
  it('records the exact red signature and restores every killed mutation', async () => {
    const repositoryRoot = mkdtempSync(join(tmpdir(), 'coding-harness-gates-repo-'));
    const parent = mkdtempSync(join(tmpdir(), 'coding-harness-gates-run-'));
    roots.push(repositoryRoot, parent);
    mkdirSync(join(repositoryRoot, 'src'));
    writeFileSync(join(repositoryRoot, 'src/fix.rs'), 'if (checked) {\n return;\n}\n');
    git(repositoryRoot, ['init', '--quiet']);
    git(repositoryRoot, ['config', 'user.email', 'harness@example.invalid']);
    git(repositoryRoot, ['config', 'user.name', 'Harness Test']);
    git(repositoryRoot, ['add', '--all']);
    git(repositoryRoot, ['commit', '--quiet', '-m', 'fixture']);
    const commit = git(repositoryRoot, ['rev-parse', 'HEAD']);
    const tree = git(repositoryRoot, ['rev-parse', 'HEAD^{tree}']);
    const cargo = join(parent, 'cargo');
    writeFileSync(cargo, fakeCargoSource(), 'utf8');
    chmodSync(cargo, 0o700);
    const command = (argv: string[]) => ({
      tool: 'cargo', executable: cargo, argv, cwd: '.', env: { CARGO_NET_OFFLINE: 'true' },
      timeoutMs: 2_000, maxOutputBytes: 100_000,
    });
    const task = {
      taskId: 'gate_task_0001',
      candidateOracle: { mode: 'exact-reference', candidate: { commit, tree } },
      tools: ['cargo'],
      redBaseline: {
        commands: [{ commandId: 'red-baseline', command: command(['--offline', 'test', '--locked']) }],
        expected: { exitCode: 101, failedTests: redTests },
      },
      commands: {
        mutation: [{
          mutationId: 'checked_prune_0001',
          path: 'src/fix.rs',
          search: 'if (checked) {\n return;\n}',
          replacement: 'return;',
          command: command(['--offline', 'test', '--locked', 'mutation_target', '--', '--exact']),
        }],
      },
    } as unknown as AcceptanceTask;
    const worktrees = new GitWorktreeSet({ repositoryRoot, runRoot: join(parent, 'worktrees') });
    const prepared = await worktrees.prepare(commit, commit);
    const runner = new AcceptanceRunner({
      task,
      worktrees,
      config: SECURE_HARNESS_CONFIG,
      offlineIsolator: isolator,
      sourceEnvironment: { PATH: process.env.PATH },
    });

    const red = await runner.redBaseline(prepared);
    await expect(runner.mutations({
      candidate: { commit, tree: '0'.repeat(40) },
      commands: [buildEvidence('0'.repeat(40), 2)],
      artifactDigests: {},
    })).rejects.toThrow('HARNESS_CANDIDATE_REFERENCE_MISMATCH');
    const mutation = await runner.mutations({
      candidate: { commit, tree },
      commands: [buildEvidence(tree, 2)],
      artifactDigests: {},
    });

    expect(red.passed).toBe(true);
    expect(red.commands[0]?.stage).toBe('red-baseline');
    expect(mutation.passed).toBe(true);
    expect(mutation.commands[0]?.stage).toBe('mutation');
    expect(mutation.commands[0]?.attempt).toBe(2);
    expect(readFileSync(join(prepared.verifierRoots.independent, 'src/fix.rs'), 'utf8'))
      .toBe('if (checked) {\n return;\n}\n');

    const killedCommand = { ...task.redBaseline.commands[0].command, argv: ['hang'], timeoutMs: 25 };
    const killedRunner = new AcceptanceRunner({
      task: {
        ...task,
        redBaseline: {
          ...task.redBaseline,
          commands: [{ commandId: 'killed-red', command: killedCommand }],
        },
      },
      worktrees,
      config: SECURE_HARNESS_CONFIG,
      offlineIsolator: isolator,
      sourceEnvironment: { PATH: process.env.PATH },
    });
    const timedOut = await killedRunner.redBaseline(prepared);
    expect(timedOut.passed).toBe(false);
    expect(timedOut.reasons).toContain('killed-red: red command did not complete normally');

    const controller = new AbortController();
    const cancelled = killedRunner.redBaseline(prepared, controller.signal);
    setTimeout(() => controller.abort(), 25);
    expect((await cancelled).passed).toBe(false);
    await worktrees.dispose();
  });
});

function fakeCargoSource(): string {
  return [
    '#!/usr/bin/env node',
    'const args = process.argv.slice(2);',
    "const split = args.indexOf('--');",
    "const names = split > 0 ? [args[split - 1]] : " + JSON.stringify(redTests) + ';',
    "for (const name of names) process.stdout.write(`test ${name} ... FAILED\\n`);",
    "if (args.includes('hang')) setInterval(() => {}, 1_000); else process.exit(101);",
    '',
  ].join('\n');
}

function buildEvidence(tree: string, attempt: number): CommandEvidence {
  return {
    stage: 'build', attempt, candidateTree: tree, tool: 'cargo', executable: '/cargo',
    argv: ['test'], cwd: '.', exitCode: 0, signal: null, durationMs: 1,
    stdoutDigest: digestValue('stdout'), stderrDigest: digestValue('stderr'),
    timedOut: false, cancelled: false, outputLimitExceeded: false, spawnErrorDigest: null,
  };
}
