//! Sealed authored admission while one qualified PostgreSQL generation is held.

use super::*;
use crate::pg_direct_lifecycle::PgDirectPools;
use crate::problem::StartupCause;
use crate::{IntrospectedSource, SemanticOntology, ServeError};

/// Pure, closed logical-source admission; no SQL or relation discovery runs here.
pub(crate) fn mapped_tables(mapping: &SourceMapping) -> Result<Arc<[String]>, ServeError> {
    crate::pg_rls::mapped_tables(mapping).ok_or_else(|| {
        configuration(
            "verified authored generation requires bounded, unqualified public base-table names",
        )
    })
}

/// Only return the admitted source after same-lease semantic validation, exact
/// final recheck and acknowledged rollback. Observation callbacks may fence an
/// old activation, but cannot take ownership of this unpublished source.
pub(crate) async fn build(
    pools: &PgDirectPools,
    mapping: SourceMapping,
    ontology: &SemanticOntology,
    budget: &RequestBudget,
    observe: &mut impl FnMut(SourceId, &IntrospectedSource) -> Result<(), ServeError>,
) -> Result<(IntrospectedSource, ValidatedMapping), ServeError> {
    let mapped = mapped_tables(&mapping)?;
    let source = discover(pools, budget).await.map_err(generation_error)?;
    let names = normalized_table_names(source.observed_schema()).map_err(generation_error)?;
    let observed = open_observed_generation(&pools.control(), mapping.source_id(), &names, budget)
        .await
        .map_err(generation_error)?;
    let tables = observed.tables();
    if !postgres_direct_table_profile_is_unambiguous(tables)
        || mapped
            .iter()
            .any(|name| !tables.iter().any(|table| &table.name == name))
    {
        return observed
            .reject(PgGenerationError::CapabilityDrift)
            .await
            .map_err(generation_error);
    }
    let current_names = match normalized_table_names(tables) {
        Ok(names) => names,
        Err(error) => return observed.reject(error).await.map_err(generation_error),
    };
    if current_names != names {
        return observed
            .reject(PgGenerationError::SchemaDrift)
            .await
            .map_err(generation_error);
    }
    // Both projections come from the SAME protected snapshot. Rich facts bind
    // the lease; authored compilation retains its established legacy schema.
    let compiler_tables = observed.legacy_tables().to_vec();
    let generation = Arc::new(PostgresGeneration {
        source_id: mapping.source_id(),
        origin: MappingOrigin::Authored,
        mapping_digest: MappingDigest::from_mapping(&mapping),
        identity: observed.identity(),
        session: observed.session().clone(),
        tables: tables.to_vec().into(),
    });
    let (lease, observation) = observed
        .promote_candidate(Arc::clone(&generation))
        .await
        .map_err(generation_error)?;
    let source = match source.bind_postgres_generation(PostgresSourceCandidate {
        tables: compiler_tables,
        observation: SourceSchemaObservationV1::postgres16_public(observation),
        generation: SourceGeneration::AuthoredPostgres(generation),
    }) {
        Ok(source) => source,
        Err(error) => {
            let _ = lease.rollback_bounded().await;
            return Err(generation_error(error));
        }
    };
    if let Err(error) = observe(mapping.source_id(), &source) {
        let _ = lease.rollback_bounded().await;
        return Err(error);
    }
    let ontology = ontology.clone();
    let built = candidate_work::run(budget, move || {
        ValidatedMapping::validate(mapping, MappingOrigin::Authored, &ontology, &source)
            .map(|mapping| (source, mapping))
    })
    .await;
    let admitted = match built {
        Ok(Ok(admitted)) => admitted,
        Ok(Err(_)) => {
            let _ = lease.rollback_bounded().await;
            return Err(configuration(
                "authored mapping does not match the protected source and ontology",
            ));
        }
        Err(error) => {
            let _ = lease.rollback_bounded().await;
            return Err(generation_error(error));
        }
    };
    lease
        .finish_bounded(budget)
        .await
        .map_err(generation_error)?;
    Ok(admitted)
}

async fn discover(
    pools: &PgDirectPools,
    budget: &RequestBudget,
) -> Result<IntrospectedSource, PgGenerationError> {
    let pool = pools.control();
    let object = budget
        .run(pool.get())
        .await?
        .map_err(|_| PgGenerationError::SourceUnavailable)?;
    let mut connection = budget
        .run(crate::backend::PgConn::checked_for_request(
            object,
            pool.tls.clone(),
            budget.clone(),
        ))
        .await?
        .map_err(|_| PgGenerationError::SourceUnavailable)?;
    let snapshot = budget
        .run(
            sf_sql::introspect::introspect_postgres_public_observed_snapshot(
                connection.discovery_client(),
            ),
        )
        .await?
        .map_err(|_| PgGenerationError::SourceUnavailable)?;
    connection.mark_recyclable();
    drop(connection);
    // A discovery observation remains explicitly unverified until build() has
    // locked, observed, admitted and closed the separate protected candidate.
    Ok(IntrospectedSource::observed_postgres(
        pools.request(),
        snapshot,
    ))
}

fn configuration(error: &'static str) -> ServeError {
    ServeError::new(StartupCause::Configuration {
        error: error.into(),
    })
}

pub(crate) fn generation_error(error: PgGenerationError) -> ServeError {
    let cause = match error {
        PgGenerationError::Control(_) | PgGenerationError::SourceUnavailable => {
            crate::ReadinessCause::SourceUnavailable
        }
        PgGenerationError::SchemaDrift => crate::ReadinessCause::SchemaDrift,
        PgGenerationError::CapabilityDrift
        | PgGenerationError::Mapping(_)
        | PgGenerationError::Internal => crate::ReadinessCause::CapabilityDrift,
    };
    ServeError::new(StartupCause::Generation { cause })
}
