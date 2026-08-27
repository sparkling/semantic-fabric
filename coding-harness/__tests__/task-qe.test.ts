// SPDX-License-Identifier: MIT

import { describe, expect, it, vi } from 'vitest';
import type { TaskQeBinding } from '../src/acceptance-task-v3.js';
import {
  assertTaskQeRunnerIdentity,
  collectTaskQeBindings,
  type TaskQeCaptures,
} from '../src/task-qe.js';

const LCOV = Object.freeze({
  profile: 'lcov-gap',
  collector: 'rust-lcov',
  packageName: 'sf-conformance',
  testTarget: 'task_qe_target',
} as const);
const SAST = Object.freeze({ profile: 'sast', collector: 'agentic-qe-sast' } as const);

describe('task-bound QE orchestration', () => {
  it('preserves declared order while passing controller-safe LCOV selectors', async () => {
    let releaseLcov!: (value: string) => void;
    const captureLcov = vi.fn(async () => await new Promise<string>((resolve) => {
      releaseLcov = resolve;
    }));
    const captureSast = vi.fn(async () => 'sast-result');
    const pending = collectTaskQeBindings([LCOV, SAST], { captureLcov, captureSast });
    await vi.waitFor(() => expect(captureSast).toHaveBeenCalledOnce());
    releaseLcov('lcov-result');

    await expect(pending).resolves.toEqual(['lcov-result', 'sast-result']);
    expect(captureLcov.mock.calls[0][0]).toEqual(LCOV);
    expect(captureSast.mock.calls[0][0]).toEqual(SAST);
  });

  it('runs exactly the profiles declared by the task', async () => {
    const captures = successfulCaptures();
    await expect(collectTaskQeBindings([SAST], captures)).resolves.toEqual(['sast']);
    expect(captures.captureSast).toHaveBeenCalledOnce();
    expect(captures.captureLcov).not.toHaveBeenCalled();

    const lcovOnly = successfulCaptures();
    await expect(collectTaskQeBindings([LCOV], lcovOnly)).resolves.toEqual(['lcov-gap']);
    expect(lcovOnly.captureLcov).toHaveBeenCalledOnce();
    expect(lcovOnly.captureSast).not.toHaveBeenCalled();
  });

  it('rejects empty, duplicate, or forged bindings before invoking a collector', async () => {
    for (const bindings of [
      [],
      [SAST, SAST],
      [{ profile: 'sast', collector: 'forged-command' }],
      [{
        profile: 'lcov-gap', collector: 'rust-lcov',
        packageName: '../unsafe', testTarget: 'task_qe_target',
      }],
    ] as unknown as readonly TaskQeBinding[][]) {
      const captures = successfulCaptures();
      await expect(collectTaskQeBindings(bindings, captures)).rejects.toThrow(
        /HARNESS_TASK_QE_BINDING/,
      );
      expect(captures.captureLcov).not.toHaveBeenCalled();
      expect(captures.captureSast).not.toHaveBeenCalled();
    }
  });

  it('aborts and settles siblings when one QE collector fails', async () => {
    let siblingAborted = false;
    const primary = new Error('primary-qe-failure');
    const captures: TaskQeCaptures = {
      captureLcov: vi.fn(async () => { throw primary; }),
      captureSast: vi.fn(async (_binding, signal) => await new Promise((_, reject) => {
        const aborted = () => {
          siblingAborted = true;
          reject(signal.reason);
        };
        if (signal.aborted) aborted();
        else signal.addEventListener('abort', aborted, { once: true });
      })),
    };

    await expect(collectTaskQeBindings([LCOV, SAST], captures)).rejects.toBe(primary);
    expect(siblingAborted).toBe(true);
  });

  it('pins the exact controller-owned Agentic-QE package identity', () => {
    const expected = {
      package: {
        version: '3.13.10',
        treeSha256: '9e08c960bc1d8150d3814c2b4395762ec640afa5cdd8cbf216fafc255ed1d7a7',
      },
    };
    expect(() => assertTaskQeRunnerIdentity(expected)).not.toThrow();
    expect(() => assertTaskQeRunnerIdentity({
      package: { ...expected.package, version: '3.13.11' },
    })).toThrow('HARNESS_TASK_QE_IDENTITY_MISMATCH');
    expect(() => assertTaskQeRunnerIdentity({
      package: { ...expected.package, treeSha256: 'f'.repeat(64) },
    })).toThrow('HARNESS_TASK_QE_IDENTITY_MISMATCH');
  });
});

function successfulCaptures() {
  return {
    captureLcov: vi.fn(async () => 'lcov-gap'),
    captureSast: vi.fn(async () => 'sast'),
  } satisfies TaskQeCaptures;
}
