// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import {
  bindAcceptanceTaskToRustProfile,
  parseAcceptanceTask,
  type AcceptanceTask,
} from '../src/acceptance-task.js';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import {
  ISSUE_8_V2_EVIDENCE_BINDINGS,
  assertIssue8V2ExecutionTask,
} from '../src/issue-8-v2-evidence.js';
import type { RustOfflineProfile } from '../src/rust-sandbox.js';
import {
  resolveTaskEvidencePlan,
  type LegacyTaskEvidenceBindings,
} from '../src/task-evidence-plan.js';

const TASK_PATH = 'coding-harness/config/issue-8-acceptance.json';
const REPORT_PATHS = Object.freeze([
  'tests/w3c/rdb2rdf/earl-semantic-fabric-direct.ttl',
  'tests/w3c/rdb2rdf/earl-semantic-fabric-r2rml.ttl',
]);

const rustProfile: RustOfflineProfile = Object.freeze({
  cargoExecutable: '/trusted/toolchain/bin/cargo',
  environment: Object.freeze({
    PATH: '/trusted/toolchain/bin:/usr/bin',
    HOME: '/home/harness',
    CARGO_HOME: '/cargo-home',
    CARGO_NET_OFFLINE: 'true',
    CARGO_INCREMENTAL: '0',
  }),
  readOnlyMounts: Object.freeze([]),
  isolator: Object.freeze({ isolate() {}, assertStable() {} }),
});

describe('task evidence plan', () => {
  it('preserves the exact legacy issue-8 QE and generated-output contract', () => {
    const task = bindAcceptanceTaskToRustProfile(v2Task(), rustProfile);
    const plan = resolveTaskEvidencePlan({
      task,
      taskPath: TASK_PATH,
      legacyV2: ISSUE_8_V2_EVIDENCE_BINDINGS,
    });

    const workspaceTests = task.commands.regression
      .find(({ commandId }) => commandId === 'workspace-tests');
    const w3cConformance = task.commands.regression
      .find(({ commandId }) => commandId === 'w3c-conformance');
    if (workspaceTests === undefined || w3cConformance === undefined) {
      throw new Error('expected legacy producers');
    }
    expect(plan.qeBindings).toEqual([
      {
        profile: 'lcov-gap', collector: 'rust-lcov',
        packageName: 'sf-conformance', testTarget: 'issue_8_binding_pruning',
      },
      { profile: 'sast', collector: 'agentic-qe-sast' },
    ]);
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
          command: w3cConformance.command,
          workspacePaths: REPORT_PATHS,
        },
      ],
    });
    expect(plan.declarationDigest)
      .toBe('8b228fbdc207804b7457c5dc64e5852086707e9b69f2acafe9ad5c64a401ab12');
    visitObjects(plan, (value) => expect(Object.isFrozen(value)).toBe(true));
  });

  it('derives schema-v3 QE selectors and producers only from the task', () => {
    const task = bindAcceptanceTaskToRustProfile(v3Task(), rustProfile);
    const plan = resolveTaskEvidencePlan({ task, taskPath: TASK_PATH });

    expect(plan.qeBindings).toEqual([
      {
        profile: 'lcov-gap', collector: 'rust-lcov',
        packageName: 'sf-conformance', testTarget: 'issue_8_binding_pruning',
      },
      { profile: 'sast', collector: 'agentic-qe-sast' },
    ]);
    expect(plan.verifierGeneratedOutputs.regression).toEqual([
      expect.objectContaining({
        evidenceId: 'task-declared-earl',
        workspacePaths: [REPORT_PATHS[0]],
      }),
    ]);
    expect(plan.verifierGeneratedOutputs.regression?.[0].command)
      .toEqual(task.commands.regression.find(({ commandId }) => commandId === 'workspace-tests')?.command);
    expect(() => resolveTaskEvidencePlan({
      task,
      taskPath: TASK_PATH,
      legacyV2: ISSUE_8_V2_EVIDENCE_BINDINGS,
    })).toThrow('HARNESS_TASK_EVIDENCE_V3_LEGACY_BINDINGS_FORBIDDEN');
  });

  it('fails closed on legacy path, task, or QE drift', () => {
    const task = bindAcceptanceTaskToRustProfile(v2Task(), rustProfile);
    expect(() => resolveTaskEvidencePlan({
      task,
      taskPath: 'coding-harness/config/not-issue-8.json',
      legacyV2: ISSUE_8_V2_EVIDENCE_BINDINGS,
    })).toThrow('HARNESS_TASK_EVIDENCE_V2_COMPATIBILITY_MISMATCH');

    const changedId = structuredClone(ISSUE_8_V2_EVIDENCE_BINDINGS);
    changedId.taskId = 'different_task_0001';
    expect(() => resolveTaskEvidencePlan({
      task,
      taskPath: TASK_PATH,
      legacyV2: changedId,
    })).toThrow('HARNESS_TASK_EVIDENCE_V2_COMPATIBILITY_MISMATCH');

    const changedProfiles = taskInput();
    changedProfiles.qeProfiles = ['sast'];
    expect(() => resolveTaskEvidencePlan({
      task: bindAcceptanceTaskToRustProfile(
        parseAcceptanceTask(changedProfiles, SECURE_HARNESS_CONFIG),
        rustProfile,
      ),
      taskPath: TASK_PATH,
      legacyV2: ISSUE_8_V2_EVIDENCE_BINDINGS,
    })).toThrow('HARNESS_TASK_EVIDENCE_V2_QE_MISMATCH');

    const changedProducer = structuredClone(task);
    const workspaceTests = changedProducer.commands.regression
      .find(({ commandId }) => commandId === 'workspace-tests');
    if (workspaceTests === undefined) throw new Error('expected legacy producer');
    workspaceTests.commandId = 'workspace-tests-drift';
    expect(() => resolveTaskEvidencePlan({
      task: changedProducer,
      taskPath: TASK_PATH,
      legacyV2: ISSUE_8_V2_EVIDENCE_BINDINGS,
    })).toThrow('HARNESS_TASK_EVIDENCE_COMMAND_BINDING_INVALID:workspace-tests');
  });

  it('requires every generated output to resolve to one exact bound producer', () => {
    const task = structuredClone(v3Task());
    if (task.schemaVersion !== 3) throw new Error('expected schema-v3 task');
    task.evidence.generatedOutputs[0] = {
      ...task.evidence.generatedOutputs[0],
      commandId: 'missing-command',
    };
    expect(() => resolveTaskEvidencePlan({ task, taskPath: TASK_PATH }))
      .toThrow('HARNESS_TASK_EVIDENCE_COMMAND_BINDING_INVALID:missing-command');

    const legacy = structuredClone(ISSUE_8_V2_EVIDENCE_BINDINGS) as LegacyTaskEvidenceBindings;
    (legacy.generatedOutputs as Array<{ evidenceId: string }>)[1].evidenceId =
      legacy.generatedOutputs[0].evidenceId;
    expect(() => resolveTaskEvidencePlan({
      task: v2Task(), taskPath: TASK_PATH, legacyV2: legacy,
    })).toThrow('HARNESS_TASK_EVIDENCE_ID_DUPLICATE');
  });

  it('rejects forged QE declarations before they can be normalized', () => {
    const task = structuredClone(v3Task());
    if (task.schemaVersion !== 3) throw new Error('expected schema-v3 task');
    (task.qe.profiles as Array<Record<string, unknown>>)[0] = {
      profile: 'lcov-gap',
      collector: 'task-selected-command',
      packageName: 'sf-conformance',
      testTarget: 'issue_8_binding_pruning',
    };
    expect(() => resolveTaskEvidencePlan({ task, taskPath: TASK_PATH }))
      .toThrow('HARNESS_TASK_QE_BINDING_INVALID');

    (task.qe.profiles as Array<Record<string, unknown>>)[0] = {
      profile: 'forged-profile',
      collector: 'rust-lcov',
    };
    expect(() => resolveTaskEvidencePlan({ task, taskPath: TASK_PATH }))
      .toThrow('HARNESS_TASK_QE_BINDING_INVALID');
  });

  it('keeps schema-v3 execution behind the explicit issue-8 compatibility guard', () => {
    expect(() => assertIssue8V2ExecutionTask({
      requestedTaskPath: TASK_PATH,
      selectedTaskPath: TASK_PATH,
      task: v2Task(),
    })).not.toThrow();
    expect(() => assertIssue8V2ExecutionTask({
      requestedTaskPath: TASK_PATH,
      selectedTaskPath: TASK_PATH,
      task: v3Task(),
    })).toThrow('HARNESS_ISSUE_8_REQUIRES_TASK_SCHEMA_V2');
    expect(() => assertIssue8V2ExecutionTask({
      requestedTaskPath: 'coding-harness/config/other-task.json',
      selectedTaskPath: TASK_PATH,
      task: v2Task(),
    })).toThrow('HARNESS_ISSUE_8_REQUIRES_TASK_SCHEMA_V2');
  });
});

function taskInput(): Record<string, any> {
  return JSON.parse(readFileSync(
    new URL('../config/issue-8-acceptance.json', import.meta.url),
    'utf8',
  )) as Record<string, any>;
}

function v2Task(): AcceptanceTask {
  return parseAcceptanceTask(taskInput(), SECURE_HARNESS_CONFIG);
}

function v3Task(): AcceptanceTask {
  const input = taskInput();
  input.schemaVersion = 3;
  input.taskId = 'task_evidence_v3_0001';
  input.candidateOracle = { mode: 'verifier-only' };
  delete input.qeProfiles;
  input.rust = { frozenLockSha256: 'a'.repeat(64) };
  input.qe = { profiles: [
    { profile: 'sast', collector: 'agentic-qe-sast' },
    {
      profile: 'lcov-gap', collector: 'rust-lcov',
      packageName: 'sf-conformance', testTarget: 'issue_8_binding_pruning',
    },
  ] };
  input.evidence = {
    requiredAdmittedPaths: ['crates/sf-sparql/src/unfold.rs'],
    generatedOutputs: [{
      stage: 'regression',
      evidenceId: 'task-declared-earl',
      commandId: 'workspace-tests',
      workspacePaths: [REPORT_PATHS[0]],
    }],
  };
  return parseAcceptanceTask(input, SECURE_HARNESS_CONFIG);
}

function visitObjects(value: unknown, visit: (value: object) => void): void {
  if (value === null || typeof value !== 'object') return;
  visit(value);
  for (const child of Object.values(value)) visitObjects(child, visit);
}
