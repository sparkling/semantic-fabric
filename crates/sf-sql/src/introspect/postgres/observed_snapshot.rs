//! Committed public observation entry points.

use crate::error::{Error, Result};
use tokio_postgres::types::Type;

#[cfg(feature = "postgres-observation-evidence")]
use super::evidence::PostgresObservationEvidenceV1;
use super::evidence::{NoopPostgresObservationObserverV1, PostgresObservationObserverV1};
use super::generation::collect_postgres_public_observed_snapshot_allow_unavailable_observed;
use super::legacy_inventory::LegacyCatalogueQueryV1;
use super::legacy_query::{
    query_bounded, TypedQueryParameter, LEGACY_RELATION_QUERY_LIMIT_PG16_V1,
    MAX_LEGACY_RELATIONS_PG16_V1,
};
use super::legacy_row::{LegacyRow, LEGACY_TEXT_QUERY_LIMIT_PG16_V1};
use super::{
    introspect_in_schema, observation, Postgres16PublicObservedSnapshotV1, RUNTIME_SCHEMA,
    SNAPSHOT_TIMEOUTS_SQL,
};

/// Capture the legacy public snapshot only after the frozen PostgreSQL-16
/// profile guards pass.
pub async fn introspect_postgres_public_snapshot_guarded(
    client: &mut tokio_postgres::Client,
) -> Result<Vec<crate::schema::TableSchema>> {
    let transaction = client
        .build_transaction()
        .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await?;
    transaction.batch_execute(SNAPSHOT_TIMEOUTS_SQL).await?;
    let mut observer = NoopPostgresObservationObserverV1;
    observation::qualify_profile_guard(&transaction, &mut observer)
        .await
        .map_err(|error| Error::Introspection(error.to_string()))?;
    let rows = query_bounded(
        &transaction,
        LegacyCatalogueQueryV1::Tables.sql(),
        &[
            TypedQueryParameter::new(&RUNTIME_SCHEMA, Type::TEXT),
            TypedQueryParameter::new(&LEGACY_TEXT_QUERY_LIMIT_PG16_V1, Type::INT4),
            TypedQueryParameter::new(&LEGACY_RELATION_QUERY_LIMIT_PG16_V1, Type::INT8),
        ],
        MAX_LEGACY_RELATIONS_PG16_V1,
        "table rows",
    )
    .await?;
    let tables: Vec<String> = rows
        .into_iter()
        .map(|row| LegacyRow::try_new(&row)?.text("bounded_text_0", "table name"))
        .collect::<Result<_>>()?;
    let schemas = introspect_in_schema(&transaction, RUNTIME_SCHEMA, &tables).await?;
    transaction.commit().await?;
    Ok(schemas)
}

/// Capture one committed legacy snapshot with a non-authorizing rich-identity
/// availability result.
pub async fn introspect_postgres_public_observed_snapshot(
    client: &mut tokio_postgres::Client,
) -> Result<Postgres16PublicObservedSnapshotV1> {
    let mut observer = NoopPostgresObservationObserverV1;
    capture_committed_observation(client, &mut observer).await
}

#[cfg(feature = "postgres-observation-evidence")]
/// Execute the production observer while recording bounded qualification data.
///
/// Fatal transaction/setup/commit failures remain ordinary `sf-sql` errors;
/// the accompanying evidence records every lifecycle step reached before the
/// failure. The feature is disabled in ordinary product builds.
pub async fn introspect_postgres_public_observed_snapshot_with_evidence(
    client: &mut tokio_postgres::Client,
) -> (
    Result<Postgres16PublicObservedSnapshotV1>,
    PostgresObservationEvidenceV1,
) {
    let mut observer = PostgresObservationEvidenceV1::default();
    let result = capture_committed_observation(client, &mut observer).await;
    (result, observer)
}

async fn capture_committed_observation<O>(
    client: &mut tokio_postgres::Client,
    observer: &mut O,
) -> Result<Postgres16PublicObservedSnapshotV1>
where
    O: PostgresObservationObserverV1,
{
    let transaction = client
        .build_transaction()
        .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await?;
    transaction
        .batch_execute(SNAPSHOT_TIMEOUTS_SQL)
        .await
        .map_err(|_| Error::Introspection("PostgreSQL observed snapshot setup failed".into()))?;
    let snapshot = collect_postgres_public_observed_snapshot_allow_unavailable_observed(
        &transaction,
        observer,
    )
    .await?;
    observer.commit_started();
    match transaction.commit().await {
        Ok(()) => {
            observer.commit_completed();
            Ok(snapshot)
        }
        Err(_) => {
            observer.commit_failed();
            Err(Error::Introspection(
                "PostgreSQL observed snapshot commit failed".into(),
            ))
        }
    }
}
