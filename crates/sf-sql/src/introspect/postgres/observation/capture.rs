//! The single guarded rich-catalogue capture path.

use sf_core::schema_identity::{ConstraintInputV1, MAX_RELATIONS_V1};
use tokio_postgres::types::Type;
use tokio_postgres::{GenericClient, Row};

use crate::schema::TableSchema;

use super::super::evidence::{
    PostgresObservationObserverV1, PostgresObservationPhaseV1, PostgresObservationStreamV1,
};
use super::super::legacy_query::{
    query_bounded_mapped_observed, BoundedQueryObservationV1, TypedQueryParameter,
};
use super::{
    build_registered_observation, catalog_decode, catalog_sql, constraint_budget, constraints,
    relation, Postgres16PublicObservedSchemaV1, PostgresSchemaIdentityLimitCodeV1,
    PostgresSchemaIdentityUnavailableV1,
};

pub(in crate::introspect::postgres) async fn qualify_profile_guard<C, O>(
    client: &C,
    observer: &mut O,
) -> Result<(), PostgresSchemaIdentityUnavailableV1>
where
    C: GenericClient + Sync,
    O: PostgresObservationObserverV1,
{
    let phase = PostgresObservationPhaseV1::Guard;
    observer.phase_started(phase);
    observer.guard_query_started();
    let row: Row = match client
        .query_one(catalog_sql::RichCatalogueQueryV1::Guard.sql(), &[])
        .await
    {
        Ok(row) => row,
        Err(_) => {
            return phase_failure(
                observer,
                phase,
                PostgresSchemaIdentityUnavailableV1::CatalogQuery,
            )
        }
    };
    let guard = match catalog_decode::decode_guard_row_v1(&row) {
        Ok(guard) => guard,
        Err(error) => return phase_failure(observer, phase, error),
    };
    observer.guard_passed(guard.server_version_num);
    observer.phase_completed(phase);
    Ok(())
}

/// Capture and assemble the rich profile inside one caller-owned snapshot.
pub(in crate::introspect::postgres) async fn capture_registered_observation<C, O>(
    client: &C,
    schema_name: &str,
    legacy_tables: &[TableSchema],
    observer: &mut O,
) -> Result<Postgres16PublicObservedSchemaV1, PostgresSchemaIdentityUnavailableV1>
where
    C: GenericClient + Sync,
    O: PostgresObservationObserverV1,
{
    let guard = capture_guard(client, observer).await?;
    let server_version_num = guard.server_version_num;
    let text_limit = catalog_decode::MAX_CATALOG_TEXT_BYTES_V1 as i32;
    let relation_limit = MAX_RELATIONS_V1 as i64 + 1;
    observer.phase_started(PostgresObservationPhaseV1::RelationsStream);
    let relations = query_bounded_mapped_observed(
        client,
        catalog_sql::RichCatalogueQueryV1::Relations.sql(),
        &[
            TypedQueryParameter::new(&schema_name, Type::TEXT),
            TypedQueryParameter::new(&text_limit, Type::INT4),
            TypedQueryParameter::new(&relation_limit, Type::INT8),
        ],
        |_| PostgresSchemaIdentityUnavailableV1::CatalogQuery,
        || {
            PostgresSchemaIdentityUnavailableV1::LimitExceeded(
                PostgresSchemaIdentityLimitCodeV1::RichRelations,
            )
        },
        |row| catalog_decode::decode_relation_row_v1(&row)?.into_catalog_fact(),
        BoundedQueryObservationV1::new(
            MAX_RELATIONS_V1,
            PostgresObservationStreamV1::RichRelations,
            observer,
        ),
    )
    .await
    .map_err(|error| phase_error(observer, PostgresObservationPhaseV1::RelationsStream, error))?;
    observer.phase_completed(PostgresObservationPhaseV1::RelationsStream);

    let attribute_limit = relation::MAX_PHYSICAL_ATTRIBUTES_TOTAL_PG16_V1 as i64 + 1;
    observer.phase_started(PostgresObservationPhaseV1::AttributesStream);
    let attributes = query_bounded_mapped_observed(
        client,
        catalog_sql::RichCatalogueQueryV1::Attributes.sql(),
        &[
            TypedQueryParameter::new(&schema_name, Type::TEXT),
            TypedQueryParameter::new(&text_limit, Type::INT4),
            TypedQueryParameter::new(&attribute_limit, Type::INT8),
        ],
        |_| PostgresSchemaIdentityUnavailableV1::CatalogQuery,
        || {
            PostgresSchemaIdentityUnavailableV1::LimitExceeded(
                PostgresSchemaIdentityLimitCodeV1::PhysicalAttributes,
            )
        },
        |row| catalog_decode::decode_attribute_row_v1(&row)?.into_catalog_fact(&guard),
        BoundedQueryObservationV1::new(
            relation::MAX_PHYSICAL_ATTRIBUTES_TOTAL_PG16_V1,
            PostgresObservationStreamV1::RichAttributes,
            observer,
        ),
    )
    .await
    .map_err(|error| {
        phase_error(
            observer,
            PostgresObservationPhaseV1::AttributesStream,
            error,
        )
    })?;
    observer.phase_completed(PostgresObservationPhaseV1::AttributesStream);

    let normalized = observed_phase(
        observer,
        PostgresObservationPhaseV1::RelationNormalization,
        relation::normalize_postgres16_relations_v1(relations, attributes),
    )?;
    let mut raw_constraints = observed_phase(
        observer,
        PostgresObservationPhaseV1::NotNullDerivation,
        constraints::observed_not_null_constraints_v1(&normalized),
    )?;
    let not_null_count = raw_constraints.len();
    observer.constraint_counts(not_null_count, 0, not_null_count);
    let (remaining_constraints, constraint_limit) = observed_phase(
        observer,
        PostgresObservationPhaseV1::ConstraintBudget,
        constraint_budget::constraint_catalog_budget_v1(not_null_count),
    )?;
    observer.phase_started(PostgresObservationPhaseV1::CatalogConstraintsStream);
    let mut catalog_constraints = query_bounded_mapped_observed(
        client,
        catalog_sql::RichCatalogueQueryV1::Constraints.sql(),
        &[
            TypedQueryParameter::new(&schema_name, Type::TEXT),
            TypedQueryParameter::new(&constraint_limit, Type::INT8),
        ],
        |_| PostgresSchemaIdentityUnavailableV1::CatalogQuery,
        || {
            PostgresSchemaIdentityUnavailableV1::LimitExceeded(
                PostgresSchemaIdentityLimitCodeV1::RawConstraints,
            )
        },
        |row| catalog_decode::decode_constraint_row_v1(&row)?.into_raw_constraint(),
        BoundedQueryObservationV1::new(
            remaining_constraints,
            PostgresObservationStreamV1::RichCatalogConstraints,
            observer,
        ),
    )
    .await
    .map_err(|error| {
        phase_error(
            observer,
            PostgresObservationPhaseV1::CatalogConstraintsStream,
            error,
        )
    })?;
    observer.phase_completed(PostgresObservationPhaseV1::CatalogConstraintsStream);
    let catalog_count = catalog_constraints.len();
    raw_constraints.append(&mut catalog_constraints);
    observer.constraint_counts(not_null_count, catalog_count, raw_constraints.len());

    let normalized_constraints: Vec<ConstraintInputV1> = observed_phase(
        observer,
        PostgresObservationPhaseV1::ConstraintNormalization,
        constraints::normalize_postgres16_constraints_v1(&normalized, raw_constraints),
    )?;
    observed_phase(
        observer,
        PostgresObservationPhaseV1::LegacyComparison,
        relation::compare_postgres16_legacy_coordinates_v1(legacy_tables, &normalized),
    )?;
    observed_phase(
        observer,
        PostgresObservationPhaseV1::IdentityBuild,
        build_registered_observation(
            server_version_num,
            normalized.into_relations(),
            normalized_constraints,
        ),
    )
}

async fn capture_guard<C, O>(
    client: &C,
    observer: &mut O,
) -> Result<catalog_decode::CatalogGuardRowV1, PostgresSchemaIdentityUnavailableV1>
where
    C: GenericClient + Sync,
    O: PostgresObservationObserverV1,
{
    let phase = PostgresObservationPhaseV1::Guard;
    observer.phase_started(phase);
    observer.guard_query_started();
    let row = client
        .query_one(catalog_sql::RichCatalogueQueryV1::Guard.sql(), &[])
        .await
        .map_err(|_| {
            phase_error(
                observer,
                phase,
                PostgresSchemaIdentityUnavailableV1::CatalogQuery,
            )
        })?;
    let guard = catalog_decode::decode_guard_row_v1(&row)
        .map_err(|error| phase_error(observer, phase, error))?;
    observer.guard_passed(guard.server_version_num);
    observer.phase_completed(phase);
    Ok(guard)
}

fn observed_phase<T, O>(
    observer: &mut O,
    phase: PostgresObservationPhaseV1,
    result: Result<T, PostgresSchemaIdentityUnavailableV1>,
) -> Result<T, PostgresSchemaIdentityUnavailableV1>
where
    O: PostgresObservationObserverV1,
{
    observer.phase_started(phase);
    match result {
        Ok(value) => {
            observer.phase_completed(phase);
            Ok(value)
        }
        Err(error) => phase_failure(observer, phase, error),
    }
}

fn phase_error<O>(
    observer: &mut O,
    phase: PostgresObservationPhaseV1,
    error: PostgresSchemaIdentityUnavailableV1,
) -> PostgresSchemaIdentityUnavailableV1
where
    O: PostgresObservationObserverV1,
{
    observer.phase_failed(phase, error);
    error
}

fn phase_failure<T, O>(
    observer: &mut O,
    phase: PostgresObservationPhaseV1,
    error: PostgresSchemaIdentityUnavailableV1,
) -> Result<T, PostgresSchemaIdentityUnavailableV1>
where
    O: PostgresObservationObserverV1,
{
    Err(phase_error(observer, phase, error))
}
