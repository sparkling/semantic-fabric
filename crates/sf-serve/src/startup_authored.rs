//! Fail-closed admission for explicitly required authored source generations.

use crate::problem::StartupCause;
use crate::source::PreparedSource;
use crate::{MappingRef, ServeError, ServeOptions};

pub(crate) async fn build_source(
    opts: &ServeOptions,
    prepared: PreparedSource,
    mapping: sf_core::SourceMapping,
    ontology: &crate::SemanticOntology,
    control: Option<&crate::budget::RequestBudget>,
    observe: &mut impl FnMut(sf_core::SourceId, &crate::IntrospectedSource) -> Result<(), ServeError>,
) -> Result<crate::RuntimeSource, ServeError> {
    validate_options(opts)?;
    validate_source(opts, &prepared)?;
    crate::pg_generation::authored::mapped_tables(&mapping)?;
    let budget = match control {
        Some(control) => control.clone(),
        None => control_budget(None, &prepared)?,
    };
    if let PreparedSource::Sqlite { path, .. } = prepared {
        let (source, mapping) = crate::sqlite_generation::build(
            path,
            opts.sqlite_pool_size,
            mapping,
            ontology,
            &budget,
            observe,
        )
        .await?;
        return crate::RuntimeSource::admitted(source, mapping).map_err(|_| configuration());
    }
    let PreparedSource::Postgres { config, tls, .. } = prepared else {
        return Err(configuration());
    };
    let pools = crate::pg_direct_lifecycle::PgDirectPools::with_tls(
        *config,
        opts.pg_pool_size,
        opts.pg_pool_wait,
        *tls,
    )
    .map_err(|_| configuration())?;
    let (source, mapping) =
        crate::pg_generation::authored::build(&pools, mapping, ontology, &budget, observe).await?;
    crate::RuntimeSource::admitted(source, mapping).map_err(|_| configuration())
}

/// Keep reload work owned through actual completion and forced shutdown.
pub(crate) fn control_budget(
    config: Option<&crate::ServeConfig>,
    source: &PreparedSource,
) -> Result<crate::budget::RequestBudget, ServeError> {
    let mut budget = crate::budget::RequestBudget::for_control_with_shutdown(
        std::time::Duration::from_secs(30),
        sf_core::query_control::QueryLimits::new(
            0,
            match source {
                PreparedSource::Sqlite { .. } => crate::sqlite_generation::CONTROL_SOURCE_WORK,
                _ => crate::pg_generation::PG_DIRECT_CONTROL_SOURCE_WORK_V1,
            },
            0,
            0,
        ),
        config.map(crate::ServeConfig::shutdown_observer),
    );
    if let Some(config) = config {
        let gate = config.control_work.as_ref().ok_or_else(configuration)?;
        let permit = std::sync::Arc::clone(gate)
            .try_acquire_owned()
            .map_err(|_| configuration())?;
        budget
            .retain_admission(permit)
            .map_err(|_| configuration())?;
    }
    Ok(budget)
}

pub(crate) fn validate_options(opts: &ServeOptions) -> Result<(), ServeError> {
    if opts.require_verified_generation
        && (opts.additional_source.is_some()
            || matches!(opts.mapping, MappingRef::Direct { .. })
            || opts.reload_interval.is_zero()
            || !opts.query_admission.permits_verified_generation())
    {
        return Err(configuration());
    }
    Ok(())
}

pub(crate) fn validate_source(
    opts: &ServeOptions,
    source: &PreparedSource,
) -> Result<(), ServeError> {
    if opts.require_verified_generation
        && matches!(source, PreparedSource::Sqlite { path, .. } if path == ":memory:" || path.starts_with("file:"))
    {
        return Err(configuration());
    }
    if opts.require_verified_generation
        && !matches!(
            source,
            PreparedSource::Postgres { .. } | PreparedSource::Sqlite { .. }
        )
    {
        return Err(configuration());
    }
    Ok(())
}

fn configuration() -> ServeError {
    ServeError::new(StartupCause::Configuration {
        error: "verified authored generation requires one PostgreSQL or file-backed SQLite source, authored base-table mappings, a nonzero reload interval and no source row policies".into(),
    })
}
