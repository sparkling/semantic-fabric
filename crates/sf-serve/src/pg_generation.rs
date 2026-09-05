//! PostgreSQL Direct-Mapping generation leases (ADR-0003 and ADR-0050).
//!
//! A lease owns one pool object from before `BEGIN` until a clean `ROLLBACK`.
//! Any cancellation, error, or drop before that rollback permanently detaches
//! the object and aborts its driver task; a possibly open transaction therefore
//! never reaches the pool recycler.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use deadpool_postgres::PoolError;
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};
use sf_core::schema_identity::ObservedSchemaIdentityV1;
use sf_core::{SourceId, SourceMapping, TableSchema};

use crate::backend::PgConn;
use crate::budget::RequestBudget;

mod context;
mod execution;

#[cfg(test)]
use context::validate_session_context;
use context::{capture_session_context, PgSessionContext};

const BEGIN_GENERATION_SQL: &str = "BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY";
const ROLLBACK_SQL: &str = "ROLLBACK";
const DEFAULT_UNBOUNDED_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_POSTGRES_TIMEOUT_MILLIS: u128 = i32::MAX as u128;
/// Conservative reservation covering lock, transaction checks, both bounded
/// catalogue projections, session context, and their fixed subqueries.
const GENERATION_METADATA_PROBE_RESERVATION: u64 = 16;

#[derive(Clone, Default)]
pub(crate) enum SourceGeneration {
    #[default]
    Unverified,
    DirectPostgres(Arc<PostgresDirectGeneration>),
}

impl SourceGeneration {
    pub(crate) fn direct_postgres(generation: PostgresDirectGeneration) -> Self {
        Self::DirectPostgres(Arc::new(generation))
    }

    pub(crate) fn requirement(
        &self,
        backend: &crate::Backend,
    ) -> Result<Option<PgGenerationRequirement>, PgGenerationError> {
        match self {
            Self::Unverified => Ok(None),
            Self::DirectPostgres(expected) => match backend {
                crate::Backend::Pg(pool) => Ok(Some(PgGenerationRequirement {
                    pool: pool.clone(),
                    expected: Arc::clone(expected),
                })),
                crate::Backend::Sqlite(_) | crate::Backend::Mysql(_) => {
                    Err(PgGenerationError::Internal)
                }
            },
        }
    }

    pub(crate) const fn is_verified(&self) -> bool {
        matches!(self, Self::DirectPostgres(_))
    }
}

/// Immutable expectation produced by one successfully closed candidate lease.
pub(crate) struct PostgresDirectGeneration {
    source_id: SourceId,
    base_iri: Arc<str>,
    row_identity: sf_mapping::DirectMappingRowIdentity,
    identity: ObservedSchemaIdentityV1,
    session: PgSessionContext,
    table_names: Arc<[String]>,
}

impl std::fmt::Debug for PostgresDirectGeneration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PostgresDirectGeneration")
            .field("source_id", &self.source_id)
            .field("base_iri_bytes", &self.base_iri.len())
            .field("row_identity", &self.row_identity)
            .field("identity", &self.identity)
            .field("table_count", &self.table_names.len())
            .finish()
    }
}

pub(crate) struct PgGenerationRequirement {
    pool: deadpool_postgres::Pool,
    expected: Arc<PostgresDirectGeneration>,
}

impl PgGenerationRequirement {
    pub(crate) fn source_id(&self) -> SourceId {
        self.expected.source_id
    }
}

/// Request-owned verified transactions keyed by their source identity.
#[derive(Default)]
pub(crate) struct VerifiedGenerationLeases {
    leases: BTreeMap<SourceId, VerifiedPostgresGenerationLease>,
}

impl VerifiedGenerationLeases {
    pub(crate) async fn acquire(
        mut requirements: Vec<PgGenerationRequirement>,
        budget: &RequestBudget,
    ) -> Result<Self, PgGenerationError> {
        requirements.sort_by_key(PgGenerationRequirement::source_id);
        if requirements
            .windows(2)
            .any(|pair| pair[0].source_id() == pair[1].source_id())
        {
            return Err(PgGenerationError::Internal);
        }
        let mut leases = BTreeMap::new();
        for requirement in requirements {
            let source_id = requirement.source_id();
            match acquire_expected(requirement, budget).await {
                Ok(lease) => {
                    leases.insert(source_id, lease);
                }
                Err(error) => {
                    close_leases(leases).await;
                    return Err(error);
                }
            }
        }
        Ok(Self { leases })
    }

    pub(crate) fn take(&mut self, source_id: SourceId) -> Option<VerifiedPostgresGenerationLease> {
        let lease = self.leases.remove(&source_id)?;
        debug_assert_eq!(lease.source_id(), source_id);
        Some(lease)
    }

    pub(crate) fn contains(&self, source_id: SourceId) -> bool {
        self.leases.contains_key(&source_id)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.leases.is_empty()
    }

    /// Roll every still-owned transaction back, attempting all leases even if
    /// one close fails. Dirty failures detach on final drop.
    pub(crate) async fn finish(self) -> Result<(), PgGenerationError> {
        finish_leases(self.leases).await
    }
}

/// Candidate output created while the exact rich observation transaction is
/// open, then returned only after its clean rollback succeeds.
pub(crate) struct PostgresDirectCandidate {
    pub(crate) tables: Vec<TableSchema>,
    pub(crate) mapping: SourceMapping,
    pub(crate) generation: PostgresDirectGeneration,
}

pub(crate) async fn build_direct_candidate(
    pool: &deadpool_postgres::Pool,
    discovery: &[TableSchema],
    base_iri: &str,
    source_id: SourceId,
    budget: &RequestBudget,
) -> Result<PostgresDirectCandidate, PgGenerationError> {
    let discovered_names = normalized_table_names(discovery)?;
    let (lease, snapshot, session) =
        open_generation(pool, source_id, &discovered_names, budget).await?;
    let Some(direct_tables) = snapshot.direct_mapping_tables() else {
        return fail_lease(lease, PgGenerationError::CapabilityDrift).await;
    };
    let tables = direct_tables.to_vec();
    if !postgres_direct_table_profile_is_unambiguous(&tables) {
        return fail_lease(lease, PgGenerationError::CapabilityDrift).await;
    }
    if normalized_table_names(&tables)? != discovered_names {
        return fail_lease(lease, PgGenerationError::SchemaDrift).await;
    }
    let identity = snapshot
        .availability()
        .identity()
        .copied()
        .ok_or(PgGenerationError::CapabilityDrift)?;
    let row_identity = sf_mapping::DirectMappingRowIdentity::RequirePrimaryKey;
    let base_iri_owned = base_iri.to_owned();
    let generated = budget
        .run(tokio::task::spawn_blocking(move || {
            let mapping = sf_mapping::direct_mapping_for_source_with_row_identity(
                &tables,
                &base_iri_owned,
                source_id,
                row_identity,
            );
            (tables, mapping)
        }))
        .await?
        .map_err(|_| PgGenerationError::Internal)?;
    let (tables, mapping) = generated;
    let mapping = match mapping {
        Ok(mapping) => mapping,
        Err(error) => return fail_lease(lease, PgGenerationError::Mapping(error)).await,
    };
    let generation = PostgresDirectGeneration {
        source_id,
        base_iri: Arc::from(base_iri),
        row_identity,
        identity,
        session,
        table_names: discovered_names.into(),
    };
    lease.finish_with_budget(budget).await?;
    Ok(PostgresDirectCandidate {
        tables,
        mapping,
        generation,
    })
}

async fn acquire_expected(
    requirement: PgGenerationRequirement,
    budget: &RequestBudget,
) -> Result<VerifiedPostgresGenerationLease, PgGenerationError> {
    let expected = requirement.expected;
    let (lease, snapshot, session) = open_generation(
        &requirement.pool,
        expected.source_id,
        &expected.table_names,
        budget,
    )
    .await?;
    let Some(identity) = snapshot.availability().identity() else {
        return fail_lease(lease, PgGenerationError::CapabilityDrift).await;
    };
    let Some(tables) = snapshot.direct_mapping_tables() else {
        return fail_lease(lease, PgGenerationError::CapabilityDrift).await;
    };
    if identity != &expected.identity
        || session != expected.session
        || normalized_table_names(tables)? != expected.table_names.as_ref()
    {
        return fail_lease(lease, PgGenerationError::SchemaDrift).await;
    }
    Ok(lease)
}

async fn open_generation(
    pool: &deadpool_postgres::Pool,
    source_id: SourceId,
    table_names: &[String],
    budget: &RequestBudget,
) -> Result<
    (
        VerifiedPostgresGenerationLease,
        sf_sql::introspect::Postgres16PublicObservedSnapshotV1,
        PgSessionContext,
    ),
    PgGenerationError,
> {
    budget.consume(
        QueryCharge::SourceWork,
        GENERATION_METADATA_PROBE_RESERVATION,
    )?;
    let object = budget.run(pool.get()).await?.map_err(map_pool_error)?;
    let conn = budget
        .run(PgConn::checked(object))
        .await?
        .map_err(|_| PgGenerationError::SourceUnavailable)?;
    conn.mark_generation_dirty();
    let lease = VerifiedPostgresGenerationLease {
        source_id,
        conn: Some(Arc::new(conn)),
    };
    let setup = transaction_setup_sql(budget)?;
    budget
        .run(lease.client().batch_execute(&setup))
        .await?
        .map_err(|_| PgGenerationError::SourceUnavailable)?;
    budget
        .run(
            sf_sql::introspect::lock_postgres_public_base_tables_classified(
                lease.client(),
                table_names,
            ),
        )
        .await?
        .map_err(|error| match error {
            sf_sql::introspect::PostgresPublicTableLockFailure::InvalidRelationSet => {
                PgGenerationError::Internal
            }
            sf_sql::introspect::PostgresPublicTableLockFailure::RelationSetChanged => {
                PgGenerationError::SchemaDrift
            }
            sf_sql::introspect::PostgresPublicTableLockFailure::Unavailable => {
                PgGenerationError::SourceUnavailable
            }
        })?;
    let snapshot = budget
        .run(
            sf_sql::introspect::introspect_postgres_public_observed_snapshot_in_transaction_classified(
                lease.client(),
            ),
        )
        .await?
        .map_err(|error| match error {
            sf_sql::introspect::PostgresGenerationObservationFailure::SourceUnavailable => {
                PgGenerationError::SourceUnavailable
            }
            sf_sql::introspect::PostgresGenerationObservationFailure::ProfileUnavailable(_) => {
                PgGenerationError::CapabilityDrift
            }
        })?;
    let session = budget
        .run(capture_session_context(lease.client()))
        .await??;
    Ok((lease, snapshot, session))
}

fn transaction_setup_sql(budget: &RequestBudget) -> Result<String, PgGenerationError> {
    let remaining = budget
        .remaining_duration()?
        .unwrap_or(DEFAULT_UNBOUNDED_TIMEOUT);
    let millis = remaining.as_millis().clamp(1, MAX_POSTGRES_TIMEOUT_MILLIS);
    let lock_millis = millis.min(1_000);
    Ok(format!(
        "{BEGIN_GENERATION_SQL}; SET LOCAL statement_timeout = {millis}; \
         SET LOCAL lock_timeout = {lock_millis}; \
         SET LOCAL idle_in_transaction_session_timeout = {millis};"
    ))
}

fn normalized_table_names(tables: &[TableSchema]) -> Result<Vec<String>, PgGenerationError> {
    let mut names = tables
        .iter()
        .map(|table| table.name.clone())
        .collect::<Vec<_>>();
    names.sort();
    if names.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(PgGenerationError::CapabilityDrift);
    }
    Ok(names)
}

/// The current SQL IR still represents the conformance-only physical row id as
/// the string `rowid`. Until that becomes a typed marker, exclude every spelling
/// that could be confused with it from the closed live PostgreSQL profile.
fn postgres_direct_table_profile_is_unambiguous(tables: &[TableSchema]) -> bool {
    tables.iter().all(|table| {
        table
            .columns
            .iter()
            .all(|column| !column.name.eq_ignore_ascii_case("rowid"))
    })
}

fn map_pool_error(error: PoolError) -> PgGenerationError {
    match error {
        PoolError::Timeout(_) | PoolError::Backend(_) | PoolError::Closed => {
            PgGenerationError::SourceUnavailable
        }
        _ => PgGenerationError::Internal,
    }
}

async fn fail_lease<T>(
    lease: VerifiedPostgresGenerationLease,
    error: PgGenerationError,
) -> Result<T, PgGenerationError> {
    let _ = tokio::time::timeout(Duration::from_secs(1), lease.finish()).await;
    Err(error)
}

async fn close_leases(leases: BTreeMap<SourceId, VerifiedPostgresGenerationLease>) {
    let _ = finish_leases(leases).await;
}

async fn finish_leases(
    leases: BTreeMap<SourceId, VerifiedPostgresGenerationLease>,
) -> Result<(), PgGenerationError> {
    let mut first_error = None;
    for (_, lease) in leases {
        let result = tokio::time::timeout(Duration::from_secs(1), lease.finish()).await;
        if first_error.is_none() {
            first_error = match result {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(error),
                Err(_) => Some(PgGenerationError::SourceUnavailable),
            };
        }
    }
    first_error.map_or(Ok(()), Err)
}

pub(crate) struct VerifiedPostgresGenerationLease {
    source_id: SourceId,
    conn: Option<Arc<PgConn>>,
}

impl VerifiedPostgresGenerationLease {
    pub(crate) const fn source_id(&self) -> SourceId {
        self.source_id
    }

    pub(crate) fn client(&self) -> &tokio_postgres::Client {
        self.conn.as_deref().expect("active generation lease")
    }

    /// Clone the transaction-bound handle into an owned executor capability.
    /// The connection itself remains dirty until [`Self::finish`] acknowledges
    /// rollback, so cancellation of either owner can never recycle it.
    pub(crate) fn execution_client(&self) -> PgGenerationClient {
        PgGenerationClient(Arc::clone(
            self.conn.as_ref().expect("active generation lease"),
        ))
    }

    /// Return the connection to its pool only after an acknowledged rollback.
    pub(crate) async fn finish(mut self) -> Result<(), PgGenerationError> {
        self.client()
            .batch_execute(ROLLBACK_SQL)
            .await
            .map_err(|_| PgGenerationError::SourceUnavailable)?;
        self.conn
            .as_ref()
            .expect("active generation lease")
            .mark_recyclable();
        drop(self.conn.take());
        Ok(())
    }

    /// Budgeted successful close. If the absolute request/startup budget wins,
    /// dropping `self` leaves the connection dirty and final drop detaches it.
    pub(crate) async fn finish_with_budget(
        mut self,
        budget: &RequestBudget,
    ) -> Result<(), PgGenerationError> {
        budget
            .run(self.client().batch_execute(ROLLBACK_SQL))
            .await?
            .map_err(|_| PgGenerationError::SourceUnavailable)?;
        self.conn
            .as_ref()
            .expect("active generation lease")
            .mark_recyclable();
        drop(self.conn.take());
        Ok(())
    }
}

/// Owned executor view of the exact connection held by a verified lease.
pub(crate) struct PgGenerationClient(Arc<PgConn>);

impl std::ops::Deref for PgGenerationClient {
    type Target = tokio_postgres::Client;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug)]
pub(crate) enum PgGenerationError {
    Control(QueryControlError),
    SourceUnavailable,
    SchemaDrift,
    CapabilityDrift,
    Mapping(sf_core::Error),
    Internal,
}

impl From<QueryControlError> for PgGenerationError {
    fn from(error: QueryControlError) -> Self {
        Self::Control(error)
    }
}

#[cfg(test)]
#[path = "pg_generation/tests.rs"]
mod tests;
