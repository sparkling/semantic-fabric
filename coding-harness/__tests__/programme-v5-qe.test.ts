// SPDX-License-Identifier: MIT

import { beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  createTaskQeCollectorForPackage: vi.fn(),
  assertTaskQeRunnerIdentity: vi.fn(),
}));
vi.mock('../src/task-qe.js', () => ({
  createTaskQeCollectorForPackage: mocks.createTaskQeCollectorForPackage,
  assertTaskQeRunnerIdentity: mocks.assertTaskQeRunnerIdentity,
}));

import {
  PROGRAMME_V5_AGENTIC_QE_PACKAGE_IDENTITY,
  createProgrammeV5TaskQeCollector,
} from '../src/programme-v5-qe.js';

const options = Object.freeze({
  taskId: 'programme_v5_issue8_20260827',
  runId: 'programme-v5-qe-run-0001',
  qeBindings: Object.freeze([
    {
      profile: 'lcov-gap', collector: 'rust-lcov',
      packageName: 'sf-conformance', testTarget: 'issue_8_binding_pruning',
    } as const,
    { profile: 'sast', collector: 'agentic-qe-sast' } as const,
  ]),
  snapshotParent: '/controlled/sast',
  nodeExecutable: '/usr/bin/node',
  bwrapExecutable: '/usr/bin/bwrap',
  packageRoot: '/system/agentic-qe',
  mcpExecutable: '/system/agentic-qe/dist/mcp/bundle.js',
});

describe('programme-v5 task QE factory', () => {
  beforeEach(() => mocks.createTaskQeCollectorForPackage.mockReset());

  it('uses the explicit immutable Agentic-QE 3.13.12 package identity', () => {
    const collector = vi.fn();
    mocks.createTaskQeCollectorForPackage.mockReturnValue(collector);

    expect(createProgrammeV5TaskQeCollector(options)).toBe(collector);
    expect(mocks.createTaskQeCollectorForPackage).toHaveBeenCalledWith(
      options,
      {
        version: '3.13.12',
        treeSha256: '0e7497a02997c9c43c2dbe9c200ce016c8a6c345b8fdb5d5ee99d61ff8722884',
      },
    );
    expect(Object.isFrozen(PROGRAMME_V5_AGENTIC_QE_PACKAGE_IDENTITY)).toBe(true);
  });
});
