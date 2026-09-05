//! Startup assembly for single-source and bounded two-source serving modes.

use sf_core::query_control::QueryLimits;
use sf_core::SourceId;

use crate::problem::StartupCause;
use crate::run::ServeOptions;
use crate::source::PreparedSource;
use crate::{IntrospectedSource, RuntimeSource, ServeConfig, ServeError};

pub(crate) async fn build_config(
    opts: &ServeOptions,
    primary: PreparedSource,
    additional: Option<PreparedSource>,
) -> Result<ServeConfig, ServeError> {
    let tbox = read_tbox(opts.ontology_path.as_deref())?;
    let primary_mapping = read_mapping(&opts.mapping_path, source_id(0))?;
    let primary = open_source(opts, primary).await?;

    let mut config = match (&opts.additional_source, additional) {
        (None, None) => ServeConfig::new(primary, primary_mapping, tbox),
        (Some(additional_opts), Some(additional)) => {
            let additional_mapping = read_mapping(&additional_opts.mapping_path, source_id(1))?;
            let additional = open_source(opts, additional).await?;
            ServeConfig::new_federated(
                [
                    RuntimeSource::new(primary, primary_mapping),
                    RuntimeSource::new(additional, additional_mapping),
                ],
                tbox,
            )
            .map_err(|error| {
                ServeError::new(StartupCause::Configuration {
                    error: error.to_string(),
                })
            })?
        }
        _ => {
            return Err(ServeError::new(StartupCause::Configuration {
                error: "additional source and mapping must be configured together".to_owned(),
            }))
        }
    };

    config.timeout = opts.timeout;
    config.set_max_query_len(opts.max_query_len)?;
    config.set_max_concurrent_requests(opts.max_concurrent_requests)?;
    config.set_max_order_rows(opts.max_order_rows);
    config.query_limits = QueryLimits::new(
        config.query_limits.max_compiler_work(),
        opts.max_source_work,
        opts.max_result_items,
        opts.max_serialized_bytes,
    )
    .with_max_retained_bytes(opts.max_order_bytes);
    Ok(config)
}

fn source_id(index: usize) -> SourceId {
    SourceId::new(index).expect("the fixed two-source profile uses representable slots")
}

fn read_mapping(path: &str, source_id: SourceId) -> Result<sf_core::SourceMapping, ServeError> {
    let turtle = std::fs::read_to_string(path).map_err(|error| {
        ServeError::new(StartupCause::MappingRead {
            path: path.to_owned(),
            error: error.to_string(),
        })
    })?;
    sf_mapping::parse_r2rml_for_source(&turtle, source_id).map_err(|error| {
        ServeError::new(StartupCause::MappingParse {
            error: error.to_string(),
        })
    })
}

fn read_tbox(path: Option<&str>) -> Result<sf_sparql::Tbox, ServeError> {
    let Some(path) = path else {
        return Ok(sf_sparql::Tbox::default());
    };
    let turtle = std::fs::read_to_string(path).map_err(|error| {
        ServeError::new(StartupCause::OntologyRead {
            path: path.to_owned(),
            error: error.to_string(),
        })
    })?;
    crate::tbox_from_turtle(&turtle)
        .map_err(|error| ServeError::new(StartupCause::OntologyParse { error }))
}

async fn open_source(
    opts: &ServeOptions,
    source: PreparedSource,
) -> Result<IntrospectedSource, ServeError> {
    crate::run::open_backend(
        source,
        opts.pg_pool_size,
        opts.pg_pool_wait,
        opts.sqlite_pool_size,
    )
    .await
}
