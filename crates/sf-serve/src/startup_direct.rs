//! Public startup for the closed PostgreSQL Direct Mapping lifecycle.
use std::sync::Arc;
use std::time::Duration;

use crate::pg_direct_lifecycle::{
    PgDirectCoordinatorPolicy, PgDirectLifecycleSpec, PgDirectLifecycleSupervisor,
};
use crate::problem::StartupCause;
use crate::source::PreparedSource;
use crate::startup_inputs::SemanticInputs;
use crate::{MappingRef, SemanticOntology, ServeConfig, ServeError, ServeOptions};

const DEFAULT_POLL: Duration = Duration::from_secs(5);
const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);

fn validate(
    opts: &ServeOptions,
    source: &PreparedSource,
    additional: &Option<PreparedSource>,
) -> Result<(), ServeError> {
    if opts.additional_source.is_some()
        || additional.is_some()
        || !matches!(source, PreparedSource::Postgres { .. })
        || !opts.query_admission.permits_direct_mapping()
    {
        return Err(configuration(
            "Direct Mapping requires one PostgreSQL source without source row policies",
        ));
    }
    let MappingRef::Direct { base_iri } = &opts.mapping else {
        return Err(configuration("Direct Mapping input is required"));
    };
    sf_mapping::validate_direct_mapping_base(base_iri)
        .map_err(|_| configuration("Direct Mapping base IRI is invalid"))
}

pub(crate) async fn serve_async(
    opts: ServeOptions,
    source: PreparedSource,
    additional: Option<PreparedSource>,
    parser: sf_sparql::ParserRuntime,
) -> Result<(), ServeError> {
    validate(&opts, &source, &additional)?;
    let opts = Arc::new(opts);
    let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
    let (mut config, mut spec, expectation) =
        before_deadline(deadline, build_config(Arc::clone(&opts), source)).await?;
    config.set_parser_runtime(parser);
    config.control_work = Some(spec.control_permits());
    spec.observe_shutdown(config.shutdown_observer());
    let config = Arc::new(config);
    // Observation is mandatory for Direct Mapping. Zero disables only authored
    // file reload; here it selects the fixed five-second default.
    let poll = if opts.reload_interval.is_zero() {
        DEFAULT_POLL
    } else {
        opts.reload_interval
    };
    let policy = PgDirectCoordinatorPolicy::for_spec(poll, &spec).map_err(|_| unavailable())?;
    let supervisor =
        PgDirectLifecycleSupervisor::start(Arc::clone(&config), spec, expectation, policy)
            .map_err(|_| unavailable())?;
    let mut shutdown = config.shutdown_observer();
    let background = async move {
        while *shutdown.borrow_and_update() == crate::lifecycle::ShutdownPhase::Running {
            shutdown
                .changed()
                .await
                .map_err(|_| std::io::Error::other("lifecycle signal unavailable"))?;
        }
        supervisor
            .shutdown()
            .await
            .map_err(|_| std::io::Error::other("Direct lifecycle shutdown failed"))
    };
    let app = match &opts.metrics {
        Some(metrics) => crate::router_with_metrics(Arc::clone(&config), metrics.clone()),
        None => crate::router(Arc::clone(&config)),
    };
    crate::lifecycle::serve_with_background(
        &opts.bind,
        app,
        config,
        opts.shutdown_timeout,
        background,
    )
    .await
}

async fn build_config(
    opts: Arc<ServeOptions>,
    source: PreparedSource,
) -> Result<
    (
        ServeConfig,
        PgDirectLifecycleSpec,
        crate::pg_generation::PostgresDirectExpectation,
    ),
    ServeError,
> {
    let prepare_opts = Arc::clone(&opts);
    let (spec, inputs) = tokio::task::spawn_blocking(move || {
        let inputs = SemanticInputs::capture(&prepare_opts)?;
        let ontology = SemanticOntology::from_turtle(&inputs.ontology)
            .map_err(|error| ServeError::new(StartupCause::OntologyParse { error }))?;
        let (PreparedSource::Postgres { config, tls, .. }, MappingRef::Direct { base_iri }) =
            (source, &prepare_opts.mapping)
        else {
            return Err(configuration("Direct Mapping profile mismatch"));
        };
        let spec = PgDirectLifecycleSpec::with_tls(
            *config,
            *tls,
            prepare_opts.pg_pool_size,
            prepare_opts.pg_pool_wait,
            ontology,
            base_iri,
            CONTROL_TIMEOUT,
        )
        .map_err(|_| unavailable())?;
        Ok((spec, inputs))
    })
    .await
    .map_err(|_| unavailable())??;
    let initial = spec.build_initial().await.map_err(|_| unavailable())?;
    let recheck_opts = Arc::clone(&opts);
    tokio::task::spawn_blocking(move || {
        if SemanticInputs::capture(&recheck_opts)? != inputs {
            return Err(configuration("semantic files changed during startup"));
        }
        Ok(())
    })
    .await
    .map_err(|_| unavailable())??;
    let (mut config, expectation) = ServeConfig::from_initial_pg_direct(initial);
    crate::startup::configure(&opts, &mut config)?;
    Ok((config, spec, expectation))
}

fn configuration(error: &str) -> ServeError {
    ServeError::new(StartupCause::Configuration {
        error: error.into(),
    })
}

fn unavailable() -> ServeError {
    ServeError::new(StartupCause::Schema {
        spec: "PostgreSQL Direct Mapping".into(),
        error: "closed Direct Mapping profile is unavailable".into(),
    })
}

async fn before_deadline<T>(
    deadline: tokio::time::Instant,
    build: impl std::future::Future<Output = Result<T, ServeError>>,
) -> Result<T, ServeError> {
    let result = tokio::time::timeout_at(deadline, build)
        .await
        .map_err(|_| unavailable())?;
    // Timeout polls its inner future first; simultaneous completion is late too.
    if tokio::time::Instant::now() >= deadline {
        return Err(unavailable());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_source_profile_rejects_before_files_or_connector_io() {
        let mut opts = ServeOptions {
            query_shape_profile: crate::QueryShapeProfile::Ordinary,
            query_admission: crate::QueryAdmission::Deny,
            source: crate::SourceRef::inline("pg:host=database.invalid user=test"),
            mapping: MappingRef::direct("http://example.test/direct/"),
            additional_source: None,
            ontology_path: "/must/not/be/read.ttl".into(),
            bind: "127.0.0.1:0".into(),
            timeout: Duration::ZERO,
            max_query_len: 4096,
            max_concurrent_requests: 1,
            max_compiler_work: 100,
            max_source_work: 100,
            max_result_items: 100,
            max_order_rows: 100,
            max_order_bytes: 4096,
            max_serialized_bytes: 4096,
            pg_pool_size: 1,
            pg_pool_wait: Duration::from_secs(1),
            sqlite_pool_size: 1,
            shutdown_timeout: Duration::from_secs(1),
            reload_interval: Duration::ZERO,
            require_verified_generation: false,
            metrics: None,
        };
        let postgres = opts.source.resolve().unwrap().prepare().unwrap();
        assert!(validate(&opts, &postgres, &None).is_ok());
        for source in [
            "sqlite:/must/not/be/created.db",
            "mysql://test@database.invalid/db",
        ] {
            let source = crate::SourceRef::inline(source)
                .resolve()
                .unwrap()
                .prepare()
                .unwrap();
            assert_eq!(
                validate(&opts, &source, &None).unwrap_err().code(),
                "startup-configuration"
            );
        }
        assert!(validate(&opts, &postgres, &Some(postgres.clone())).is_err());
        opts.additional_source = Some(crate::AdditionalSourceOptions {
            source: crate::SourceRef::inline("pg:host=other.invalid user=test"),
            mapping: MappingRef::direct("http://example.test/other/"),
        });
        assert!(validate(&opts, &postgres, &None).is_err());
        opts.additional_source = None;
        opts.mapping = MappingRef::direct("relative");
        assert!(validate(&opts, &postgres, &None).is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn startup_rejects_simultaneous_or_late_completion() {
        for delay in [STARTUP_TIMEOUT, STARTUP_TIMEOUT + Duration::from_secs(1)] {
            let deadline = tokio::time::Instant::now() + STARTUP_TIMEOUT;
            assert!(before_deadline(deadline, async move {
                tokio::time::advance(delay).await;
                Ok(())
            })
            .await
            .is_err());
        }
        assert!(
            before_deadline(tokio::time::Instant::now() + STARTUP_TIMEOUT, async {
                Ok(())
            })
            .await
            .is_ok()
        );
    }
}
