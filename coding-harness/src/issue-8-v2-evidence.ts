// SPDX-License-Identifier: MIT

import type { AcceptanceTask } from './acceptance-task.js';
import { deepFreeze } from './contracts.js';
import type { LegacyTaskEvidenceBindings } from './task-evidence-plan.js';

const CONFORMANCE_REPORT_PATHS = Object.freeze([
  'tests/w3c/rdb2rdf/earl-semantic-fabric-direct.ttl',
  'tests/w3c/rdb2rdf/earl-semantic-fabric-r2rml.ttl',
]);

export const ISSUE_8_V2_EVIDENCE_BINDINGS = deepFreeze({
  taskPath: 'coding-harness/config/issue-8-acceptance.json',
  taskId: 'bprune_8_20260825',
  qeBindings: [
    {
      profile: 'lcov-gap',
      collector: 'rust-lcov',
      packageName: 'sf-conformance',
      testTarget: 'issue_8_binding_pruning',
    },
    { profile: 'sast', collector: 'agentic-qe-sast' },
  ],
  generatedOutputs: [
    {
      stage: 'regression',
      evidenceId: 'workspace-tests-earl',
      commandId: 'workspace-tests',
      workspacePaths: CONFORMANCE_REPORT_PATHS,
    },
    {
      stage: 'regression',
      evidenceId: 'w3c-conformance-earl',
      commandId: 'w3c-conformance',
      workspacePaths: CONFORMANCE_REPORT_PATHS,
    },
  ],
} satisfies LegacyTaskEvidenceBindings);

export function assertIssue8V2ExecutionTask(input: Readonly<{
  requestedTaskPath: string;
  selectedTaskPath: string;
  task: AcceptanceTask;
}>): void {
  if (input.selectedTaskPath !== input.requestedTaskPath
    || input.selectedTaskPath !== ISSUE_8_V2_EVIDENCE_BINDINGS.taskPath
    || input.task.schemaVersion !== 2) {
    throw new Error('HARNESS_ISSUE_8_REQUIRES_TASK_SCHEMA_V2');
  }
}
