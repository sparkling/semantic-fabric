// SPDX-License-Identifier: MIT

const FREEZE = Object.freeze;
const TYPE_ERROR = TypeError;

export const POSTGRES_MIGRATION_RUNNER_RESULT_OUTCOMES_V1 = FREEZE([
  'applied',
  'exact-no-op',
  'committed-cleanup-failed',
  'rejected',
  'rejected-cleanup-failed',
  'resolution-unknown',
  'commit-resolution-unknown',
] as const);

export type PostgresMigrationRunnerOutcomeV1 =
  typeof POSTGRES_MIGRATION_RUNNER_RESULT_OUTCOMES_V1[number];

export interface PostgresMigrationRunnerTerminalResultV1 {
  readonly resultKind: 'postgresql-migration-runner-result-v1';
  readonly outcome: PostgresMigrationRunnerOutcomeV1;
  readonly authority: 'none';
  readonly readinessAuthorized: false;
  readonly databaseAccessAuthorized: false;
  readonly migrationApplyAuthorized: false;
}

const APPLIED = terminalResultV1('applied');
const EXACT_NO_OP = terminalResultV1('exact-no-op');
const COMMITTED_CLEANUP_FAILED = terminalResultV1('committed-cleanup-failed');
const REJECTED = terminalResultV1('rejected');
const REJECTED_CLEANUP_FAILED = terminalResultV1('rejected-cleanup-failed');
const RESOLUTION_UNKNOWN = terminalResultV1('resolution-unknown');
const COMMIT_RESOLUTION_UNKNOWN = terminalResultV1('commit-resolution-unknown');

/** Return one of seven exact, frozen, non-authorizing terminal singletons. */
export function postgresMigrationRunnerTerminalResultV1(
  outcome: unknown,
): PostgresMigrationRunnerTerminalResultV1 {
  if (outcome === 'applied') return APPLIED;
  if (outcome === 'exact-no-op') return EXACT_NO_OP;
  if (outcome === 'committed-cleanup-failed') return COMMITTED_CLEANUP_FAILED;
  if (outcome === 'rejected') return REJECTED;
  if (outcome === 'rejected-cleanup-failed') return REJECTED_CLEANUP_FAILED;
  if (outcome === 'resolution-unknown') return RESOLUTION_UNKNOWN;
  if (outcome === 'commit-resolution-unknown') return COMMIT_RESOLUTION_UNKNOWN;
  throw new TYPE_ERROR('PostgreSQL migration runner outcome is invalid');
}

function terminalResultV1(
  outcome: PostgresMigrationRunnerOutcomeV1,
): PostgresMigrationRunnerTerminalResultV1 {
  return FREEZE({
    resultKind: 'postgresql-migration-runner-result-v1' as const,
    outcome,
    authority: 'none' as const,
    readinessAuthorized: false as const,
    databaseAccessAuthorized: false as const,
    migrationApplyAuthorized: false as const,
  });
}
