// SPDX-License-Identifier: MIT

export type ResourceCleanup = () => Promise<void> | void;

export class ParentedResourceCleanupError extends AggregateError {}

export async function cleanupParentedResources(input: Readonly<{
  children: readonly ResourceCleanup[];
  parent: ResourceCleanup;
  independent?: readonly ResourceCleanup[];
  failureMessage: string;
}>): Promise<void> {
  const independent = settle(input.independent ?? []);
  const children = await settle(input.children);
  const parent = children.every(({ status }) => status === 'fulfilled')
    ? await settle([input.parent])
    : [];
  const outcomes = [...children, ...parent, ...await independent];
  const failures = outcomes.filter((outcome) => outcome.status === 'rejected');
  if (failures.length > 0) {
    throw new ParentedResourceCleanupError(
      failures.map((outcome) => (outcome as PromiseRejectedResult).reason),
      input.failureMessage,
    );
  }
}

export function cleanupRequiresAncestorPreservation(error: unknown): boolean {
  return requiresPreservation(error, new Set());
}

export async function runWithCleanup<T>(
  run: () => Promise<T>,
  cleanup: ResourceCleanup,
  failureMessage: string,
): Promise<T> {
  let result: T | undefined;
  let failure: unknown;
  let failed = false;
  try { result = await run(); } catch (error) { failure = error; failed = true; }
  if (failed && cleanupRequiresAncestorPreservation(failure)) throw failure;
  try { await cleanup(); } catch (cleanupError) {
    failure = failed ? new AggregateError([failure, cleanupError], failureMessage) : cleanupError;
    failed = true;
  }
  if (failed) throw failure;
  return result as T;
}

async function settle(cleanups: readonly ResourceCleanup[]) {
  return await Promise.allSettled(cleanups.map(async (cleanup) => await cleanup()));
}

function requiresPreservation(error: unknown, seen: Set<unknown>): boolean {
  if (error instanceof ParentedResourceCleanupError) return true;
  if (!(error instanceof AggregateError) || seen.has(error)) return false;
  seen.add(error);
  return error.errors.some((nested) => requiresPreservation(nested, seen));
}
