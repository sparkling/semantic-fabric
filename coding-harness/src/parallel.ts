// SPDX-License-Identifier: MIT

export type AbortableTask<T> = (signal: AbortSignal) => Promise<T>;

/**
 * Run a bounded parallel cohort with one linked cancellation signal.
 *
 * The first rejection aborts every sibling immediately, but the cohort does
 * not return control until every task has settled. This prevents an abandoned
 * model or subprocess from outliving the gate that launched it.
 */
export async function runAbortableCohort<const T extends readonly unknown[]>(
  tasks: { readonly [K in keyof T]: AbortableTask<T[K]> },
  parentSignal?: AbortSignal,
): Promise<T> {
  const controller = new AbortController();
  let hasPrimaryFailure = false;
  let primaryFailure: unknown;

  const fail = (reason: unknown): void => {
    if (!hasPrimaryFailure) {
      hasPrimaryFailure = true;
      primaryFailure = reason;
    }
    if (!controller.signal.aborted) controller.abort(reason);
  };
  const abortFromParent = (): void => fail(abortReason(parentSignal));

  if (parentSignal?.aborted === true) throw abortReason(parentSignal);
  parentSignal?.addEventListener('abort', abortFromParent, { once: true });

  try {
    const running = tasks.map(async (task) => {
      try {
        return await task(controller.signal);
      } catch (error) {
        fail(error);
        throw error;
      }
    });
    const settled = await Promise.allSettled(running);
    if (hasPrimaryFailure) throw primaryFailure;

    return settled.map((outcome) => {
      if (outcome.status === 'rejected') {
        // Every rejection above records a primary failure; this is defensive.
        throw outcome.reason;
      }
      return outcome.value;
    }) as unknown as T;
  } finally {
    parentSignal?.removeEventListener('abort', abortFromParent);
  }
}

function abortReason(signal: AbortSignal | undefined): unknown {
  if (signal?.reason !== undefined) return signal.reason;
  const error = new Error('HARNESS_PARALLEL_OPERATION_ABORTED');
  error.name = 'AbortError';
  return error;
}
