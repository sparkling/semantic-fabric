//! Versioned metadata-probe inventory and transaction setup.

use std::time::Duration;

use sf_sql::introspect::{
    POSTGRES_GENERATION_TRANSACTION_PROBE_QUERY_COUNT_V1, POSTGRES_LEGACY_CATALOGUE_QUERY_COUNT_V1,
    POSTGRES_PROFILE_PREQUALIFICATION_QUERY_COUNT_V1,
    POSTGRES_RICH_CAPTURE_CATALOGUE_QUERY_COUNT_V1,
};

use crate::budget::RequestBudget;

use super::super::{PgGenerationError, POSTGRES_SESSION_CONTEXT_QUERY_COUNT_V1};

pub(in crate::pg_generation) const BEGIN_GENERATION_SQL: &str =
    "BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY";
const DEFAULT_UNBOUNDED_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_POSTGRES_TIMEOUT_MILLIS: u128 = i32::MAX as u128;

const TRANSACTION_POLICY_CHECKS_PER_LEASE_V1: u64 = 3;
const SESSION_CONTEXT_CAPTURES_PER_LEASE_V1: u64 = 4;
const OBSERVED_SNAPSHOT_CAPTURES_PER_LEASE_V1: u64 = 2;

/// Metadata work reserved before connection acquisition. Connection-scope
/// verification, BEGIN/SET, relation LOCK, and ROLLBACK are separately
/// time-bounded administrative operations and are not source-work probes.
pub(in crate::pg_generation) const GENERATION_METADATA_PROBE_RESERVATION: u64 =
    TRANSACTION_POLICY_CHECKS_PER_LEASE_V1 * POSTGRES_GENERATION_TRANSACTION_PROBE_QUERY_COUNT_V1
        + SESSION_CONTEXT_CAPTURES_PER_LEASE_V1 * POSTGRES_SESSION_CONTEXT_QUERY_COUNT_V1
        + OBSERVED_SNAPSHOT_CAPTURES_PER_LEASE_V1
            * (POSTGRES_PROFILE_PREQUALIFICATION_QUERY_COUNT_V1
                + POSTGRES_LEGACY_CATALOGUE_QUERY_COUNT_V1
                + POSTGRES_RICH_CAPTURE_CATALOGUE_QUERY_COUNT_V1);

pub(in crate::pg_generation) fn transaction_setup_sql(
    budget: &RequestBudget,
) -> Result<String, PgGenerationError> {
    let remaining = budget
        .remaining_duration()?
        .unwrap_or(DEFAULT_UNBOUNDED_TIMEOUT);
    let millis = remaining.as_millis().clamp(1, MAX_POSTGRES_TIMEOUT_MILLIS);
    let lock_millis = millis.min(1_000);
    Ok(format!(
        "{BEGIN_GENERATION_SQL}; SET LOCAL statement_timeout = {millis}; \
         SET LOCAL lock_timeout = {lock_millis}; \
         SET LOCAL idle_in_transaction_session_timeout = {millis}; \
         SET LOCAL search_path = pg_catalog, public, pg_temp; \
         SET LOCAL row_security = on;"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_reservation_is_derived_from_the_executable_inventory() {
        assert_eq!(POSTGRES_GENERATION_TRANSACTION_PROBE_QUERY_COUNT_V1, 2);
        assert_eq!(POSTGRES_SESSION_CONTEXT_QUERY_COUNT_V1, 1);
        assert_eq!(POSTGRES_PROFILE_PREQUALIFICATION_QUERY_COUNT_V1, 1);
        assert_eq!(POSTGRES_LEGACY_CATALOGUE_QUERY_COUNT_V1, 7);
        assert_eq!(POSTGRES_RICH_CAPTURE_CATALOGUE_QUERY_COUNT_V1, 4);
        assert_eq!(GENERATION_METADATA_PROBE_RESERVATION, 34);
    }
}
