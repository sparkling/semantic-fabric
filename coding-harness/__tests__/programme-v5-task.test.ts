// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { parseAcceptanceTask } from '../src/acceptance-task.js';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import { resolveTaskEvidencePlanV1 } from '../src/task-evidence-plan.js';

const TASK_PATH = 'coding-harness/config/programme-v5-acceptance.json';
const LOCK_DIGEST = '72916782d4d8fb87b613f61debe2107c160e083ef4969c89c23c7596df5b637d';
const REPORT_PATHS = [
  'tests/w3c/rdb2rdf/earl-semantic-fabric-direct.ttl',
  'tests/w3c/rdb2rdf/earl-semantic-fabric-r2rml.ttl',
] as const;

describe('programme-v5 activation task', () => {
  it('parses as a verifier-only schema-v3 fixture without changing issue-8 evidence', () => {
    const task = programmeTask();
    const legacy = issue8Task();

    expect(legacy.schemaVersion).toBe(2);
    expect(legacy.candidateOracle.mode).toBe('exact-reference');
    expect(task).toMatchObject({
      schemaVersion: 3,
      taskId: 'programme_v5_issue8_20260827',
      workItem: 'completion-programme:h0c-issue-8-activation',
      candidateOracle: { mode: 'verifier-only' },
      rust: { frozenLockSha256: LOCK_DIGEST },
      authority: 'development-only-no-promotion',
      evolutionEligible: false,
    });
    if (task.schemaVersion !== 3) throw new Error('expected schema-v3 task');
    expect(task.objective).toContain('not a general solution-neutral oracle');
    expect(task.exclusions).toContain(
      'Do not treat or reuse this first activation fixture, which intentionally retains source-shape mutation strings, as a general solution-neutral oracle.',
    );
    expect(task.qe.profiles).toEqual([
      {
        profile: 'lcov-gap', collector: 'rust-lcov',
        packageName: 'sf-conformance', testTarget: 'issue_8_binding_pruning',
      },
      { profile: 'sast', collector: 'agentic-qe-sast' },
    ]);
    expect(task.evidence.requiredAdmittedPaths).toEqual(['crates/sf-sparql/src/unfold.rs']);
    expect(task.evidence.generatedOutputs).toEqual([
      {
        stage: 'regression', evidenceId: 'workspace-tests-earl',
        commandId: 'workspace-tests', workspacePaths: REPORT_PATHS,
      },
      {
        stage: 'regression', evidenceId: 'w3c-conformance-earl',
        commandId: 'w3c-conformance', workspacePaths: REPORT_PATHS,
      },
    ]);
    expect({
      baseline: task.baseline,
      evaluatorPaths: task.evaluatorPaths,
      implementationPaths: task.implementationPaths,
      artifactPaths: task.artifactPaths,
      tools: task.tools,
      redBaseline: task.redBaseline,
      commands: task.commands,
    }).toEqual({
      baseline: legacy.baseline,
      evaluatorPaths: legacy.evaluatorPaths,
      implementationPaths: legacy.implementationPaths,
      artifactPaths: legacy.artifactPaths,
      tools: legacy.tools,
      redBaseline: legacy.redBaseline,
      commands: legacy.commands,
    });
  });

  it('derives the exact QE and generated-output evidence plan from task-local declarations', () => {
    const task = programmeTask();
    if (task.schemaVersion !== 3) throw new Error('expected schema-v3 task');
    const plan = resolveTaskEvidencePlanV1({ task, taskPath: TASK_PATH });
    const workspaceTests = task.commands.regression
      .find(({ commandId }) => commandId === 'workspace-tests');
    const w3c = task.commands.regression
      .find(({ commandId }) => commandId === 'w3c-conformance');
    if (workspaceTests === undefined || w3c === undefined) throw new Error('expected producers');

    expect(plan.qeBindings).toEqual(task.qe.profiles);
    expect(plan.requiredQeProfiles).toEqual(['lcov-gap', 'sast']);
    expect(plan.verifierGeneratedOutputs).toEqual({
      regression: [
        {
          evidenceId: 'workspace-tests-earl',
          command: workspaceTests.command,
          workspacePaths: REPORT_PATHS,
        },
        {
          evidenceId: 'w3c-conformance-earl',
          command: w3c.command,
          workspacePaths: REPORT_PATHS,
        },
      ],
    });
    expect(plan.declarationDigest)
      .toBe('11e79a1111b41e9311663b9d68d4d61365e761aea476214b4ef102f6b734339f');
  });
});

function programmeTask() {
  return parseAcceptanceTask(json('../config/programme-v5-acceptance.json'), SECURE_HARNESS_CONFIG);
}

function issue8Task() {
  return parseAcceptanceTask(json('../config/issue-8-acceptance.json'), SECURE_HARNESS_CONFIG);
}

function json(path: string): unknown {
  return JSON.parse(readFileSync(new URL(path, import.meta.url), 'utf8'));
}
