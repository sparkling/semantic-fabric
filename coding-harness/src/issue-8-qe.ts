// SPDX-License-Identifier: MIT

import { ISSUE_8_V2_EVIDENCE_BINDINGS } from './issue-8-v2-evidence.js';
import type { TaskQeBinding } from './acceptance-task-v3.js';
import {
  createTaskQeCollector,
  type TaskQeContext,
} from './task-qe.js';

export type Issue8QeContext = TaskQeContext;

interface Issue8QeOptions {
  taskId: string;
  runId: string;
  snapshotParent: string;
  nodeExecutable: string;
  bwrapExecutable: string;
  packageRoot: string;
  mcpExecutable: string;
}

export function createIssue8TaskQeCollector(options: Readonly<
  Issue8QeOptions & { qeBindings: readonly TaskQeBinding[] }
>) {
  try {
    return createTaskQeCollector(options);
  } catch (error) {
    rethrowIssue8QeFactoryError(error);
  }
}

export function rethrowIssue8QeFactoryError(error: unknown): never {
  if (error !== null && typeof error === 'object'
    && 'message' in error
    && error.message === 'HARNESS_TASK_QE_IDENTITY_MISMATCH') {
    throw new Error('HARNESS_ISSUE_8_AGENTIC_QE_IDENTITY_MISMATCH');
  }
  throw error;
}

export function createIssue8QeCollector(options: Readonly<Issue8QeOptions>) {
  return createIssue8TaskQeCollector({
    ...options,
    qeBindings: ISSUE_8_V2_EVIDENCE_BINDINGS.qeBindings,
  });
}
