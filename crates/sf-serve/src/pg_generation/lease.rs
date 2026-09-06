//! PostgreSQL verified-generation type-state and connection ownership.

use std::sync::Arc;
use std::time::Duration;

use deadpool_postgres::PoolError;
use sf_core::query_control::{QueryCharge, QueryControl};
use sf_core::schema_identity::ObservedSchemaIdentityV1;
use sf_core::{SourceId, TableSchema};
use sf_sql::introspect::{
    Postgres16PublicObservedSchemaV1, Postgres16PublicObservedSnapshotV1,
    PostgresGenerationObservationFailure, PostgresPublicTableLockFailure,
    PostgresSchemaIdentityAvailabilityV1,
};

use crate::backend::PgConn;
use crate::budget::RequestBudget;

use super::{
    capture_session_context, PgGenerationError, PgSessionContext, PostgresDirectGeneration,
};

pub(super) const BEGIN_GENERATION_SQL: &str =
    "BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY";
const ROLLBACK_SQL: &str = "ROLLBACK";
const DEFAULT_UNBOUNDED_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_POSTGRES_TIMEOUT_MILLIS: u128 = i32::MAX as u128;
const VERIFIED_GENERATION_ROLLBACK_ALLOWANCE: Duration = Duration::from_secs(2);
/// Initial and final checks each cover the lock/transaction guards, both
/// bounded catalogue projections, session context, and fixed subqueries.
const GENERATION_METADATA_PROBE_RESERVATION: u64 = 32;

/// A pool member detached from recycling before a transaction can begin.
struct DirtyGeneration {
    source_id: SourceId,
    conn: Option<Arc<PgConn>>,
}

/// The dirty member after an acknowledged `BEGIN` and bounded local policy.
struct OpenGeneration(DirtyGeneration);

/// Test-only view of the exact production post-`BEGIN` state before relation
/// locking. It exposes no client, so the test cannot accidentally establish
/// the snapshot it is trying to inspect.
#[cfg(test)]
pub(super) struct OpenGenerationProbe {
    owner: DirtyGeneration,
    backend_pid: i32,
}

#[cfg(test)]
impl OpenGenerationProbe {
    pub(super) const fn backend_pid(&self) -> i32 {
        self.backend_pid
    }

    pub(super) async fn finish(self) -> Result<(), PgGenerationError> {
        self.owner.rollback().await
    }
}

/// An open generation after its exact relation set is protected.
struct LockedGeneration(DirtyGeneration);

/// Rich facts observed only after the transaction and lock transitions.
pub(super) struct ObservedGeneration {
    owner: DirtyGeneration,
    snapshot: Postgres16PublicObservedSnapshotV1,
    session: PgSessionContext,
}

impl DirtyGeneration {
    async fn acquire(
        pool: &deadpool_postgres::Pool,
        source_id: SourceId,
        budget: &RequestBudget,
    ) -> Result<Self, PgGenerationError> {
        let object = budget.run(pool.get()).await?.map_err(map_pool_error)?;
        let conn = budget
            .run(PgConn::checked(object))
            .await?
            .map_err(|_| PgGenerationError::SourceUnavailable)?;
        conn.mark_generation_dirty();
        Ok(Self {
            source_id,
            conn: Some(Arc::new(conn)),
        })
    }

    fn client(&self) -> &tokio_postgres::Client {
        self.conn.as_deref().expect("active dirty generation")
    }

    async fn begin(self, budget: &RequestBudget) -> Result<OpenGeneration, PgGenerationError> {
        let setup = transaction_setup_sql(budget)?;
        budget
            .run(self.client().batch_execute(&setup))
            .await?
            .map_err(|_| PgGenerationError::SourceUnavailable)?;
        Ok(OpenGeneration(self))
    }

    async fn rollback(mut self) -> Result<(), PgGenerationError> {
        if self
            .conn
            .as_ref()
            .is_some_and(|conn| Arc::strong_count(conn) != 1)
        {
            return Err(PgGenerationError::Internal);
        }
        self.client()
            .batch_execute(ROLLBACK_SQL)
            .await
            .map_err(|_| PgGenerationError::SourceUnavailable)?;
        self.mark_recyclable();
        Ok(())
    }

    fn mark_recyclable(&mut self) {
        self.conn
            .as_ref()
            .expect("active dirty generation")
            .mark_recyclable();
        drop(self.conn.take());
    }
}

impl OpenGeneration {
    async fn lock(
        self,
        table_names: &[String],
        budget: &RequestBudget,
    ) -> Result<LockedGeneration, PgGenerationError> {
        let result = budget
            .run(
                sf_sql::introspect::lock_postgres_public_base_tables_classified(
                    self.0.client(),
                    table_names,
                ),
            )
            .await;
        match result {
            Ok(Ok(())) => Ok(LockedGeneration(self.0)),
            Ok(Err(error)) => reject(self.0, lock_error(error)).await,
            Err(error) => Err(error.into()),
        }
    }
}

impl LockedGeneration {
    async fn observe(
        self,
        budget: &RequestBudget,
    ) -> Result<ObservedGeneration, PgGenerationError> {
        let before = match budget.run(capture_session_context(self.0.client())).await {
            Ok(Ok(context)) => context,
            Ok(Err(error)) => return reject(self.0, error).await,
            Err(error) => return Err(error.into()),
        };
        let snapshot = match budget
            .run(
                sf_sql::introspect::introspect_postgres_public_observed_snapshot_in_transaction_classified(
                    self.0.client(),
                ),
            )
            .await
        {
            Ok(Ok(snapshot)) => snapshot,
            Ok(Err(error)) => return reject(self.0, observation_error(error)).await,
            Err(error) => return Err(error.into()),
        };
        let after = match budget.run(capture_session_context(self.0.client())).await {
            Ok(Ok(context)) => context,
            Ok(Err(error)) => return reject(self.0, error).await,
            Err(error) => return Err(error.into()),
        };
        if before != after || snapshot.direct_mapping_tables().is_none() {
            return reject(self.0, PgGenerationError::CapabilityDrift).await;
        }
        Ok(ObservedGeneration {
            owner: self.0,
            snapshot,
            session: after.into_context(),
        })
    }
}

impl ObservedGeneration {
    pub(super) fn tables(&self) -> &[TableSchema] {
        self.snapshot
            .direct_mapping_tables()
            .expect("observed generation carries rich tables")
    }

    pub(super) fn identity(&self) -> ObservedSchemaIdentityV1 {
        self.snapshot
            .availability()
            .identity()
            .copied()
            .expect("observed generation carries identity")
    }

    pub(super) fn session(&self) -> &PgSessionContext {
        &self.session
    }

    pub(super) async fn reject<T>(self, error: PgGenerationError) -> Result<T, PgGenerationError> {
        reject(self.owner, error).await
    }

    pub(super) async fn promote_expected(
        self,
        expected: Arc<PostgresDirectGeneration>,
    ) -> Result<VerifiedPostgresGenerationLease, PgGenerationError> {
        if let Some(error) = mismatch(&self.snapshot, &self.session, &expected) {
            return reject(self.owner, error).await;
        }
        Ok(VerifiedPostgresGenerationLease {
            owner: self.owner,
            expected,
            #[cfg(test)]
            revalidation_delay: None,
        })
    }

    pub(super) async fn promote_candidate(
        self,
        expected: Arc<PostgresDirectGeneration>,
    ) -> Result<
        (
            VerifiedPostgresGenerationLease,
            Postgres16PublicObservedSchemaV1,
        ),
        PgGenerationError,
    > {
        if let Some(error) = mismatch(&self.snapshot, &self.session, &expected) {
            return reject(self.owner, error).await;
        }
        let (_, availability) = self.snapshot.into_parts();
        let PostgresSchemaIdentityAvailabilityV1::Available(observation) = availability else {
            return reject(self.owner, PgGenerationError::Internal).await;
        };
        Ok((
            VerifiedPostgresGenerationLease {
                owner: self.owner,
                expected,
                #[cfg(test)]
                revalidation_delay: None,
            },
            observation,
        ))
    }
}

/// The only type that exposes an execution client. Construction requires every
/// preceding type-state transition and exact expected-generation comparison.
pub(crate) struct VerifiedPostgresGenerationLease {
    owner: DirtyGeneration,
    expected: Arc<PostgresDirectGeneration>,
    #[cfg(test)]
    revalidation_delay: Option<Duration>,
}

impl VerifiedPostgresGenerationLease {
    pub(crate) const fn source_id(&self) -> SourceId {
        self.owner.source_id
    }

    pub(super) fn execution_client(&self) -> PgGenerationClient {
        PgGenerationClient(Arc::clone(
            self.owner
                .conn
                .as_ref()
                .expect("active verified generation"),
        ))
    }
    /// Recheck exact facts inside the original request/startup budget, then give
    /// rollback its own bounded cleanup allowance even when that budget expired.
    pub(crate) async fn finish_bounded(
        self,
        budget: &RequestBudget,
    ) -> Result<(), PgGenerationError> {
        let validation = match budget
            .run(async {
                #[cfg(test)]
                if let Some(delay) = self.revalidation_delay {
                    delayed_catalogue_probe(self.owner.client(), delay).await?;
                }
                revalidate(&self.owner, &self.expected).await
            })
            .await
        {
            Ok(validation) => validation,
            Err(error) => Err(error.into()),
        };
        let rollback = bounded_rollback(self.owner).await;
        validation.and(rollback)
    }
    /// Cleanup-only close for a lease whose execution path was never entered.
    pub(crate) async fn rollback_bounded(self) -> Result<(), PgGenerationError> {
        bounded_rollback(self.owner).await
    }

    #[cfg(test)]
    pub(super) fn with_revalidation_delay(mut self, delay: Duration) -> Self {
        self.revalidation_delay = Some(delay);
        self
    }

    #[cfg(test)]
    pub(super) fn without_connection(
        source_id: SourceId,
        expected: Arc<PostgresDirectGeneration>,
    ) -> Self {
        Self {
            owner: DirtyGeneration {
                source_id,
                conn: None,
            },
            expected,
            revalidation_delay: None,
        }
    }
}

#[cfg(test)]
async fn delayed_catalogue_probe(
    client: &tokio_postgres::Client,
    delay: Duration,
) -> Result<(), PgGenerationError> {
    let seconds = delay.as_secs_f64();
    client
        .query_one(
            "WITH delay AS MATERIALIZED ( \
               SELECT pg_catalog.pg_sleep($1::double precision) \
             ) SELECT count(*) FROM pg_catalog.pg_class CROSS JOIN delay",
            &[&seconds],
        )
        .await
        .map_err(|_| PgGenerationError::SourceUnavailable)?;
    Ok(())
}

/// Owned executor view of the exact dirty connection held by the final state.
pub(super) struct PgGenerationClient(Arc<PgConn>);

impl std::ops::Deref for PgGenerationClient {
    type Target = tokio_postgres::Client;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub(super) async fn open_observed_generation(
    pool: &deadpool_postgres::Pool,
    source_id: SourceId,
    table_names: &[String],
    budget: &RequestBudget,
) -> Result<ObservedGeneration, PgGenerationError> {
    budget.consume(
        QueryCharge::SourceWork,
        GENERATION_METADATA_PROBE_RESERVATION,
    )?;
    DirtyGeneration::acquire(pool, source_id, budget)
        .await?
        .begin(budget)
        .await?
        .lock(table_names, budget)
        .await?
        .observe(budget)
        .await
}

#[cfg(test)]
pub(super) async fn open_generation_before_lock_for_test(
    pool: &deadpool_postgres::Pool,
    source_id: SourceId,
    budget: &RequestBudget,
) -> Result<OpenGenerationProbe, PgGenerationError> {
    let dirty = DirtyGeneration::acquire(pool, source_id, budget).await?;
    let backend_pid = budget
        .run(
            dirty
                .client()
                .query_one("SELECT pg_catalog.pg_backend_pid() AS backend_pid", &[]),
        )
        .await?
        .map_err(|_| PgGenerationError::SourceUnavailable)?
        .try_get("backend_pid")
        .map_err(|_| PgGenerationError::CapabilityDrift)?;
    let open = dirty.begin(budget).await?;
    Ok(OpenGenerationProbe {
        owner: open.0,
        backend_pid,
    })
}

pub(super) fn transaction_setup_sql(budget: &RequestBudget) -> Result<String, PgGenerationError> {
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

async fn revalidate(
    owner: &DirtyGeneration,
    expected: &PostgresDirectGeneration,
) -> Result<(), PgGenerationError> {
    let before = capture_session_context(owner.client()).await?;
    let snapshot =
        sf_sql::introspect::introspect_postgres_public_observed_snapshot_in_transaction_classified(
            owner.client(),
        )
        .await
        .map_err(observation_error)?;
    let after = capture_session_context(owner.client()).await?;
    if before != after {
        return Err(PgGenerationError::CapabilityDrift);
    }
    mismatch(&snapshot, after.context(), expected).map_or(Ok(()), Err)
}

fn mismatch(
    snapshot: &Postgres16PublicObservedSnapshotV1,
    session: &PgSessionContext,
    expected: &PostgresDirectGeneration,
) -> Option<PgGenerationError> {
    generation_facts_mismatch(
        snapshot.availability().identity(),
        snapshot.direct_mapping_tables(),
        session,
        expected,
    )
}

pub(super) fn generation_facts_mismatch(
    identity: Option<&ObservedSchemaIdentityV1>,
    tables: Option<&[TableSchema]>,
    session: &PgSessionContext,
    expected: &PostgresDirectGeneration,
) -> Option<PgGenerationError> {
    if session != &expected.session || identity.is_none() || tables.is_none() {
        return Some(PgGenerationError::CapabilityDrift);
    }
    (identity != Some(&expected.identity) || tables != Some(expected.tables.as_ref()))
        .then_some(PgGenerationError::SchemaDrift)
}

async fn reject<T>(
    owner: DirtyGeneration,
    error: PgGenerationError,
) -> Result<T, PgGenerationError> {
    let _ = bounded_rollback(owner).await;
    Err(error)
}

async fn bounded_rollback(owner: DirtyGeneration) -> Result<(), PgGenerationError> {
    tokio::time::timeout(VERIFIED_GENERATION_ROLLBACK_ALLOWANCE, owner.rollback())
        .await
        .map_err(|_| PgGenerationError::SourceUnavailable)?
}

fn lock_error(error: PostgresPublicTableLockFailure) -> PgGenerationError {
    match error {
        PostgresPublicTableLockFailure::InvalidRelationSet => PgGenerationError::Internal,
        PostgresPublicTableLockFailure::RelationSetChanged => PgGenerationError::SchemaDrift,
        PostgresPublicTableLockFailure::InsufficientPrivilege => PgGenerationError::CapabilityDrift,
        PostgresPublicTableLockFailure::Unavailable => PgGenerationError::SourceUnavailable,
    }
}

fn observation_error(error: PostgresGenerationObservationFailure) -> PgGenerationError {
    match error {
        PostgresGenerationObservationFailure::SourceUnavailable => {
            PgGenerationError::SourceUnavailable
        }
        PostgresGenerationObservationFailure::ProfileUnavailable(_) => {
            PgGenerationError::CapabilityDrift
        }
    }
}

fn map_pool_error(error: PoolError) -> PgGenerationError {
    match error {
        PoolError::Timeout(_) | PoolError::Backend(_) | PoolError::Closed => {
            PgGenerationError::SourceUnavailable
        }
        _ => PgGenerationError::Internal,
    }
}
