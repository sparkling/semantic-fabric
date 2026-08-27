// SPDX-License-Identifier: MIT

import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({ createTaskQeCollector: vi.fn() }));
vi.mock('../src/task-qe.js', () => ({
  createTaskQeCollector: mocks.createTaskQeCollector,
}));

import {
  createIssue8TaskQeCollector,
  rethrowIssue8QeFactoryError,
} from '../src/issue-8-qe.js';

const options = Object.freeze({
  taskId: 'bprune_8_20260825',
  runId: 'run-issue8-qe-0001',
  qeBindings: Object.freeze([
    { profile: 'sast', collector: 'agentic-qe-sast' } as const,
  ]),
  snapshotParent: '/controlled/sast',
  nodeExecutable: '/system/node',
  bwrapExecutable: '/system/bwrap',
  packageRoot: '/system/agentic-qe',
  mcpExecutable: '/system/aqe-mcp',
});

describe('issue-8 task QE factory', () => {
  beforeEach(() => mocks.createTaskQeCollector.mockReset());

  it('passes the attested task bindings to the generic collector', () => {
    const collector = vi.fn();
    mocks.createTaskQeCollector.mockReturnValue(collector);

    expect(createIssue8TaskQeCollector(options)).toBe(collector);
    expect(mocks.createTaskQeCollector).toHaveBeenCalledWith(options);
  });

  it('preserves the legacy issue-8 package-identity failure code', () => {
    expect(() => rethrowIssue8QeFactoryError(
      new Error('HARNESS_TASK_QE_IDENTITY_MISMATCH'),
    ))
      .toThrow('HARNESS_ISSUE_8_AGENTIC_QE_IDENTITY_MISMATCH');
  });

  it('does not relabel unrelated factory failures', () => {
    const failure = new Error('HARNESS_TASK_QE_BINDING_INVALID');
    expect(() => rethrowIssue8QeFactoryError(failure))
      .toThrow('HARNESS_TASK_QE_BINDING_INVALID');
  });
});
