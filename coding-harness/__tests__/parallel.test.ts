// SPDX-License-Identifier: MIT

import { describe, expect, it, vi } from 'vitest';
import { runAbortableCohort } from '../src/parallel.js';

describe('abortable parallel cohort', () => {
  it('aborts siblings on the first failure and drains them before rejecting', async () => {
    const failure = new Error('primary failure');
    const events: string[] = [];
    let releaseFailure!: () => void;
    const failNow = new Promise<void>((resolve) => {
      releaseFailure = resolve;
    });
    let releaseDrain!: () => void;
    const drain = new Promise<void>((resolve) => {
      releaseDrain = resolve;
    });

    const cohort = runAbortableCohort([
      async () => {
        await failNow;
        throw failure;
      },
      async (signal) => {
        await new Promise<void>((resolve) => {
          signal.addEventListener('abort', () => {
            events.push('sibling-aborted');
            resolve();
          }, { once: true });
        });
        await drain;
        events.push('sibling-drained');
        throw new Error('cancelled sibling');
      },
    ] as const);

    releaseFailure();
    await vi.waitFor(() => expect(events).toContain('sibling-aborted'));
    let rejected = false;
    void cohort.catch(() => {
      rejected = true;
    });
    await Promise.resolve();
    expect(rejected).toBe(false);

    releaseDrain();
    await expect(cohort).rejects.toBe(failure);
    expect(events).toEqual(['sibling-aborted', 'sibling-drained']);
  });

  it('links parent cancellation and does not launch an already-aborted cohort', async () => {
    const controller = new AbortController();
    const reason = new Error('caller cancelled');
    const task = vi.fn(async () => 'unreachable');
    controller.abort(reason);

    await expect(runAbortableCohort([task] as const, controller.signal)).rejects.toBe(reason);
    expect(task).not.toHaveBeenCalled();
  });

  it('preserves tuple order after every task succeeds', async () => {
    await expect(runAbortableCohort([
      async () => 7,
      async () => 'complete',
    ] as const)).resolves.toEqual([7, 'complete']);
  });
});
