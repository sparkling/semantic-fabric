// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';
import { describe, expect, it, vi } from 'vitest';
import { parseAcceptanceTask } from '../src/acceptance-task.js';
import { SECURE_HARNESS_CONFIG } from '../src/config.js';
import {
  candidateExpectationForTask,
  RepositoryCandidateOperations,
} from '../src/repository-operations.js';
import {
  cleanupRequiresAncestorPreservation,
  cleanupParentedResources,
  ParentedResourceCleanupError,
  runWithCleanup,
} from '../src/resource-cleanup.js';

const task = parseAcceptanceTask(JSON.parse(readFileSync(
  new URL('../config/programme-v5-acceptance.json', import.meta.url), 'utf8',
)), SECURE_HARNESS_CONFIG);

describe('parented resource cleanup', () => {
  it('waits for every child before disposing its parent', async () => {
    const events: string[] = [];
    let release!: () => void;
    const childFinished = new Promise<void>((resolve) => { release = resolve; });
    const cleanup = cleanupParentedResources({
      children: [async () => {
        events.push('child:start');
        await childFinished;
        events.push('child:end');
      }],
      parent: async () => { events.push('parent'); },
      independent: [async () => { events.push('independent'); }],
      failureMessage: 'CLEANUP_FAILED',
    });

    await vi.waitFor(() => expect(events).toContain('child:start'));
    expect(events).not.toContain('parent');
    release();
    await cleanup;
    expect(events.indexOf('child:end')).toBeLessThan(events.indexOf('parent'));
  });

  it('preserves the parent when child cleanup fails but still cleans independent resources', async () => {
    const parent = vi.fn();
    const independent = vi.fn();
    const cleanup = cleanupParentedResources({
      children: [async () => { throw new Error('CHILD_CHANGED'); }],
      parent,
      independent: [independent],
      failureMessage: 'CLEANUP_FAILED',
    });

    await expect(cleanup).rejects.toThrow('CLEANUP_FAILED');
    expect(parent).not.toHaveBeenCalled();
    expect(independent).toHaveBeenCalledOnce();
  });

  it('preserves the primary failure and propagates ancestor preservation through aggregates', async () => {
    const primary = new Error('PRIMARY_FAILURE');
    const cleanup = new ParentedResourceCleanupError(
      [new Error('CHILD_CLEANUP_FAILURE')], 'CLEANUP_FAILED',
    );
    let failure: unknown;
    try {
      await runWithCleanup(
        async () => { throw primary; },
        async () => { throw cleanup; },
        'EXECUTION_AND_CLEANUP_FAILED',
      );
    } catch (error) {
      failure = error;
    }

    expect(failure).toBeInstanceOf(AggregateError);
    expect((failure as AggregateError).errors).toEqual([primary, cleanup]);
    expect(cleanupRequiresAncestorPreservation(failure)).toBe(true);
  });

  it('does not run ancestor cleanup after a preservation-marked primary failure', async () => {
    const failure = new ParentedResourceCleanupError(
      [new Error('CHILD_CLEANUP_FAILURE')], 'CLEANUP_FAILED',
    );
    const cleanup = vi.fn();

    await expect(runWithCleanup(
      async () => { throw failure; }, cleanup, 'EXECUTION_AND_CLEANUP_FAILED',
    )).rejects.toBe(failure);
    expect(cleanup).not.toHaveBeenCalled();
  });

  it('wires repository-operation child cleanup ahead of worktree disposal', async () => {
    let release!: () => void;
    const childFinished = new Promise<void>((resolve) => { release = resolve; });
    const child = vi.fn(async () => await childFinished);
    const dispose = vi.fn(async () => {});
    const independent = vi.fn(async () => {});
    const command = task.commands.build[0]!.command;
    const operations = new RepositoryCandidateOperations({
      worktrees: { dispose } as never,
      config: SECURE_HARNESS_CONFIG,
      baselineCommit: task.baseline.commit,
      evaluatorCommit: task.baseline.commit,
      candidateExpectation: candidateExpectationForTask(task),
      taskForWorkspace: () => { throw new Error('not used'); },
      buildCommands: [command],
      verifierCommands: { public: [command], independent: [command], regression: [command] },
      artifactPaths: ['artifact'],
      model: {} as never,
      offlineIsolator: {} as never,
      offlineEnvironment: {},
      frozenLockfile: { sourcePath: '/frozen', workspacePath: 'Cargo.lock', digest: 'a'.repeat(64) },
      worktreeChildCleanupCallbacks: [child],
      cleanupCallbacks: [independent],
      agenticQeEvidence: async () => [],
      preflightEvidence: async () => ({}) as never,
      mutationEvidence: async () => ({}) as never,
    });

    const cleanup = operations.cleanup();
    await vi.waitFor(() => expect(child).toHaveBeenCalledOnce());
    expect(dispose).not.toHaveBeenCalled();
    release();
    await cleanup;
    expect(dispose).toHaveBeenCalledOnce();
    expect(independent).toHaveBeenCalledOnce();
  });
});
