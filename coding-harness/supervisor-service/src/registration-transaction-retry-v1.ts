// SPDX-License-Identifier: MIT

export type SupervisorRegistrationCommitResolutionV1 =
  | 'committed'
  | 'unknown';

const RETRYABLE_ABORT_V1 = Symbol('supervisor-registration-retryable-abort-v1');
const RETRY_ATTEMPT_V1 = Symbol('supervisor-registration-retry-attempt-v1');
const MAX_SERIALIZABLE_ATTEMPTS_V1 = 3;

export type SupervisorRegistrationRetryAttemptV1 = typeof RETRY_ATTEMPT_V1;

export function throwSupervisorRegistrationRetryableAbortV1(): never {
  throw RETRYABLE_ABORT_V1;
}

export function isSupervisorRegistrationRetryableAbortV1(
  error: unknown,
): boolean {
  return error === RETRYABLE_ABORT_V1;
}

export async function runBoundedSupervisorRegistrationAttemptsV1<T>(
  attempt: () => Promise<T | SupervisorRegistrationRetryAttemptV1>,
  exhausted: () => T,
): Promise<T> {
  for (let index = 0; index < MAX_SERIALIZABLE_ATTEMPTS_V1; index += 1) {
    const outcome = await attempt();
    if (outcome !== RETRY_ATTEMPT_V1) return outcome;
  }
  return exhausted();
}

export async function commitSupervisorRegistrationAttemptV1<T, R>(
  transaction: Readonly<{
    commit(): Promise<SupervisorRegistrationCommitResolutionV1>;
  }>,
  value: T,
  committed: (value: T) => R,
  indeterminate: () => R,
): Promise<R | SupervisorRegistrationRetryAttemptV1> {
  try {
    const resolution = await transaction.commit();
    if (resolution === 'committed') return committed(value);
    return indeterminate();
  } catch (error) {
    return isSupervisorRegistrationRetryableAbortV1(error)
      ? RETRY_ATTEMPT_V1 : indeterminate();
  }
}

export async function rollbackSupervisorRegistrationAttemptV1<R>(
  transaction: Readonly<{ rollback(): Promise<void> }>,
  retry: boolean,
  indeterminate: () => R,
): Promise<R | SupervisorRegistrationRetryAttemptV1> {
  try { await transaction.rollback(); }
  catch { return indeterminate(); }
  return retry ? RETRY_ATTEMPT_V1 : indeterminate();
}
