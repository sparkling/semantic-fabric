// SPDX-License-Identifier: MIT

import { readFileSync, writeFileSync } from 'node:fs';
import type { AcceptanceTask, MutationAcceptanceCommand } from './acceptance-task.js';
import type {
  AcceptanceGateEvidence,
  CandidateBuild,
  PreparedCandidate,
} from './candidate.js';
import { deepFreeze, type HarnessConfig, type StructuredCommand } from './contracts.js';
import type { GitWorktreeSet } from './git-worktrees.js';
import type { OfflineProcessIsolator } from './network.js';
import { runStructuredProcess, type ProcessResult } from './process.js';
import { digestValue, type CommandEvidence } from './receipts.js';
import { resolveWorkspacePath } from './workspace.js';

export interface AcceptanceRunnerOptions {
  readonly task: AcceptanceTask;
  readonly worktrees: GitWorktreeSet;
  readonly config: HarnessConfig;
  readonly offlineIsolator: OfflineProcessIsolator;
  readonly sourceEnvironment: Readonly<Record<string, string | undefined>>;
}

export class AcceptanceRunner {
  readonly #options: AcceptanceRunnerOptions;

  constructor(options: AcceptanceRunnerOptions) {
    this.#options = options;
  }

  async redBaseline(
    prepared: PreparedCandidate,
    signal?: AbortSignal,
  ): Promise<AcceptanceGateEvidence> {
    const root = this.#options.worktrees.evaluatorRoot();
    const outputRoot = this.#options.worktrees.outputRoot('evaluator');
    const commands: CommandEvidence[] = [];
    const failures: string[] = [];
    for (const named of this.#options.task.redBaseline.commands) {
      const result = await this.#run(named.command, root, outputRoot, signal);
      commands.push(commandEvidence(
        'red-baseline',
        0,
        prepared.evaluator.tree,
        named.command,
        result,
      ));
      if (!normallyCompleted(result)) {
        failures.push(`${named.commandId}: red command did not complete normally`);
      }
      if (result.exitCode !== this.#options.task.redBaseline.expected.exitCode) {
        failures.push(`${named.commandId}: expected exit 101, received ${String(result.exitCode)}`);
      }
      const failedTests = parseFailedTests(`${result.stdout}\n${result.stderr}`);
      if (!sameStrings(failedTests, this.#options.task.redBaseline.expected.failedTests)) {
        failures.push(`${named.commandId}: red failure signature mismatch`);
      }
    }
    return gateEvidence(
      failures,
      commands,
      'red-baseline',
      { expected: this.#options.task.redBaseline.expected, commands },
    );
  }

  async mutations(
    build: CandidateBuild,
    signal?: AbortSignal,
  ): Promise<AcceptanceGateEvidence> {
    if (this.#options.task.candidateOracle.mode === 'exact-reference'
      && build.candidate.tree !== this.#options.task.candidateOracle.candidate.tree) {
      throw new Error('HARNESS_CANDIDATE_REFERENCE_MISMATCH');
    }
    const root = this.#options.worktrees.verifierRoot('independent');
    const outputRoot = this.#options.worktrees.outputRoot('independent');
    const commands: CommandEvidence[] = [];
    const failures: string[] = [];
    const digests: Record<string, string> = {};
    const attempts = new Set(build.commands.map(({ attempt }) => attempt));
    if (build.commands.length === 0 || attempts.size !== 1) {
      throw new Error('HARNESS_MUTATION_BUILD_ATTEMPT_INVALID');
    }
    const attempt = build.commands[0].attempt;
    for (const mutation of this.#options.task.commands.mutation) {
      const before = await this.#options.worktrees.verifierIdentity('independent', signal);
      if (before.commit !== build.candidate.commit || before.tree !== build.candidate.tree) {
        throw new Error('HARNESS_MUTATION_STALE_VERIFIER_IDENTITY');
      }
      const path = resolveWorkspacePath(root, mutation.path, {
        requireRegularFile: true,
        rejectHardlinks: true,
      });
      const original = readFileSync(path, 'utf8');
      if (occurrences(original, mutation.search) !== 1) {
        throw new Error(`HARNESS_MUTATION_SEARCH_MISMATCH:${mutation.mutationId}`);
      }
      let result: ProcessResult;
      try {
        writeFileSync(path, original.replace(mutation.search, mutation.replacement), 'utf8');
        result = await this.#run(mutation.command, root, outputRoot, signal);
      } finally {
        writeFileSync(path, original, 'utf8');
      }
      const evidence = commandEvidence(
        'mutation',
        attempt,
        build.candidate.tree,
        mutation.command,
        result,
      );
      commands.push(evidence);
      const expectedTest = exactTestName(mutation);
      const failedTests = parseFailedTests(`${result.stdout}\n${result.stderr}`);
      if (!normallyCompleted(result)
        || result.exitCode !== 101
        || !failedTests.includes(expectedTest)) {
        failures.push(`${mutation.mutationId}: mutation survived or failed outside ${expectedTest}`);
      }
      digests[`mutation:${mutation.mutationId}`] = digestValue({
        mutationId: mutation.mutationId,
        path: mutation.path,
        searchDigest: digestValue(mutation.search),
        replacementDigest: digestValue(mutation.replacement),
        command: evidence,
      });
      await this.#options.worktrees.assertVerifierSourceStable('independent', signal);
    }
    return deepFreeze({
      passed: failures.length === 0,
      reasons: failures,
      commands,
      digests,
    });
  }

  async #run(
    command: StructuredCommand,
    workspaceRoot: string,
    outputRoot: string,
    signal?: AbortSignal,
  ): Promise<ProcessResult> {
    return await runStructuredProcess({
      ...command,
      env: {
        ...command.env,
        HOME: '/home/harness',
        CARGO_HOME: '/cargo-home',
        CARGO_NET_OFFLINE: 'true',
        CARGO_INCREMENTAL: '0',
        CARGO_TARGET_DIR: outputRoot,
      },
    }, {
      workspaceRoot,
      config: this.#options.config,
      declaredTools: this.#options.task.tools,
      sourceEnvironment: { ...this.#options.sourceEnvironment },
      signal,
      boundary: {
        kind: 'offline-candidate',
        isolator: this.#options.offlineIsolator,
        writablePaths: [outputRoot],
      },
    });
  }
}

function commandEvidence(
  stage: CommandEvidence['stage'],
  attempt: number,
  tree: string,
  command: StructuredCommand,
  result: ProcessResult,
): CommandEvidence {
  return deepFreeze({
    stage,
    attempt,
    candidateTree: tree,
    tool: command.tool,
    executable: command.executable,
    argv: [...command.argv],
    cwd: command.cwd,
    exitCode: result.exitCode,
    signal: result.signal,
    durationMs: result.durationMs,
    stdoutDigest: digestValue(result.stdout),
    stderrDigest: digestValue(result.stderr),
    timedOut: result.timedOut,
    cancelled: result.cancelled,
    outputLimitExceeded: result.outputLimitExceeded,
    spawnErrorDigest: result.spawnError === null ? null : digestValue(result.spawnError),
  });
}

function normallyCompleted(result: ProcessResult): boolean {
  return !result.timedOut
    && !result.cancelled
    && !result.outputLimitExceeded
    && result.spawnError === null
    && result.signal === null;
}

function gateEvidence(
  failures: readonly string[],
  commands: readonly CommandEvidence[],
  name: string,
  value: unknown,
): AcceptanceGateEvidence {
  return deepFreeze({
    passed: failures.length === 0,
    reasons: [...failures],
    commands: [...commands],
    digests: { [name]: digestValue(value) },
  });
}

function parseFailedTests(output: string): string[] {
  const names = [...output.matchAll(/^test ([A-Za-z_][A-Za-z0-9_:]*) \.\.\. FAILED$/gm)]
    .map((match) => match[1]);
  return [...new Set(names)].sort();
}

function exactTestName(mutation: MutationAcceptanceCommand): string {
  const separator = mutation.command.argv.indexOf('--');
  if (separator < 1) throw new Error(`HARNESS_MUTATION_EXACT_TEST_MISSING:${mutation.mutationId}`);
  return mutation.command.argv[separator - 1];
}

function occurrences(value: string, search: string): number {
  return value.split(search).length - 1;
}

function sameStrings(left: readonly string[], right: readonly string[]): boolean {
  return JSON.stringify([...left].sort()) === JSON.stringify([...right].sort());
}
