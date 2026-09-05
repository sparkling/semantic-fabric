//! Startup assembly for single-source and bounded two-source serving modes.

use sf_core::query_control::QueryLimits;
use sf_core::SourceId;

use crate::problem::StartupCause;
use crate::run::ServeOptions;
use crate::source::PreparedSource;
use crate::{BackendKind, IntrospectedSource, MappingRef, RuntimeSource, ServeConfig, ServeError};

pub(crate) async fn build_config(
    opts: &ServeOptions,
    primary: PreparedSource,
    additional: Option<PreparedSource>,
) -> Result<ServeConfig, ServeError> {
    let tbox = read_tbox(opts.ontology_path.as_deref())?;
    let primary_mapping = PreparedMapping::new(&opts.mapping, source_id(0))?;
    let additional_mapping = opts
        .additional_source
        .as_ref()
        .map(|options| PreparedMapping::new(&options.mapping, source_id(1)))
        .transpose()?;
    admit_mapping_profile(
        &primary_mapping,
        &primary,
        additional_mapping.as_ref(),
        additional.as_ref(),
    )?;
    let primary = open_source(opts, primary).await?;
    let (primary, primary_mapping) = primary_mapping.finish(opts, primary).await?;

    let mut config = match (additional_mapping, additional) {
        (None, None) => ServeConfig::new(primary, primary_mapping, tbox),
        (Some(additional_mapping), Some(additional)) => {
            let additional = open_source(opts, additional).await?;
            let (additional, additional_mapping) =
                additional_mapping.finish(opts, additional).await?;
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

#[derive(Debug)]
enum PreparedMapping {
    Authored(sf_core::SourceMapping),
    Direct {
        base_iri: String,
        source_id: SourceId,
    },
}

impl PreparedMapping {
    fn new(mapping: &MappingRef, source_id: SourceId) -> Result<Self, ServeError> {
        match mapping {
            MappingRef::R2rmlFile(path) => {
                let turtle = std::fs::read_to_string(path).map_err(|error| {
                    ServeError::new(StartupCause::MappingRead {
                        path: path.to_owned(),
                        error: error.to_string(),
                    })
                })?;
                sf_mapping::parse_r2rml_for_source(&turtle, source_id)
                    .map(Self::Authored)
                    .map_err(mapping_error)
            }
            MappingRef::Direct { base_iri } => {
                sf_mapping::validate_direct_mapping_base(base_iri).map_err(mapping_error)?;
                Ok(Self::Direct {
                    base_iri: base_iri.clone(),
                    source_id,
                })
            }
        }
    }

    async fn finish(
        self,
        opts: &ServeOptions,
        source: IntrospectedSource,
    ) -> Result<(IntrospectedSource, sf_core::SourceMapping), ServeError> {
        match self {
            Self::Authored(mapping) => Ok((source, mapping)),
            Self::Direct {
                base_iri,
                source_id,
            } => {
                let pool = match source.backend() {
                    crate::Backend::Pg(pool) => pool.clone(),
                    crate::Backend::Sqlite(_) | crate::Backend::Mysql(_) => {
                        return Err(configuration_error(
                            "live Direct Mapping requires PostgreSQL",
                        ))
                    }
                };
                let deadline = std::time::Instant::now()
                    .checked_add(opts.timeout)
                    .ok_or_else(|| configuration_error("startup timeout is not representable"))?;
                let budget = crate::budget::RequestBudget::uncontrolled(Some(deadline));
                let candidate = crate::pg_generation::build_direct_candidate(
                    &pool,
                    source.observed_schema(),
                    &base_iri,
                    source_id,
                    &budget,
                )
                .await
                .map_err(startup_generation_error)?;
                let source = source
                    .bind_postgres_direct(candidate.tables, candidate.generation)
                    .map_err(startup_generation_error)?;
                Ok((source, candidate.mapping))
            }
        }
    }

    const fn is_direct(&self) -> bool {
        matches!(self, Self::Direct { .. })
    }

    #[cfg(test)]
    fn finish_for(
        self,
        backend: BackendKind,
        schema: &[sf_core::TableSchema],
    ) -> Result<sf_core::SourceMapping, ServeError> {
        match self {
            Self::Authored(mapping) => Ok(mapping),
            Self::Direct {
                base_iri,
                source_id,
            } => {
                let row_identity = match backend {
                    BackendKind::Sqlite => sf_mapping::DirectMappingRowIdentity::SqliteRowId,
                    // No live generation lease currently spans every branch on
                    // either remote adapter. PK-backed tables remain admissible;
                    // no-PK tables fail closed during mapping generation.
                    BackendKind::Postgres | BackendKind::MySql => {
                        sf_mapping::DirectMappingRowIdentity::RequirePrimaryKey
                    }
                };
                sf_mapping::direct_mapping_for_source_with_row_identity(
                    schema,
                    &base_iri,
                    source_id,
                    row_identity,
                )
                .map_err(mapping_error)
            }
        }
    }
}

/// Admit only the closed live profile for which a source-generation lease can
/// be proven: one PostgreSQL source. SQLite's hidden row identity is shadowable,
/// MySQL has no qualified DDL-generation law, and Direct Mapping federation has
/// neither a qualified multi-source lease nor a source-scoped blank-node law.
/// This boundary runs after pure source parsing but before connector I/O.
fn admit_mapping_profile(
    primary_mapping: &PreparedMapping,
    primary_source: &PreparedSource,
    additional_mapping: Option<&PreparedMapping>,
    additional_source: Option<&PreparedSource>,
) -> Result<(), ServeError> {
    if additional_mapping.is_some() != additional_source.is_some() {
        return Err(configuration_error(
            "additional source and mapping must be configured together",
        ));
    }
    if (primary_mapping.is_direct() || additional_mapping.is_some_and(PreparedMapping::is_direct))
        && (additional_source.is_some() || primary_source.kind() != BackendKind::Postgres)
    {
        return Err(configuration_error(
            "live Direct Mapping requires the single-source PostgreSQL-16 public-base-table profile",
        ));
    }
    Ok(())
}

fn configuration_error(error: &'static str) -> ServeError {
    ServeError::new(StartupCause::Configuration {
        error: error.to_owned(),
    })
}

fn mapping_error(error: sf_core::Error) -> ServeError {
    ServeError::new(StartupCause::MappingParse {
        error: error.to_string(),
    })
}

fn startup_generation_error(error: crate::pg_generation::PgGenerationError) -> ServeError {
    if let crate::pg_generation::PgGenerationError::Mapping(error) = error {
        return mapping_error(error);
    }
    let category = match error {
        crate::pg_generation::PgGenerationError::Control(_) => "generation budget expired",
        crate::pg_generation::PgGenerationError::SourceUnavailable => {
            "generation source unavailable"
        }
        crate::pg_generation::PgGenerationError::SchemaDrift => "generation schema changed",
        crate::pg_generation::PgGenerationError::CapabilityDrift => {
            "generation profile unavailable"
        }
        crate::pg_generation::PgGenerationError::Internal => "generation assembly failed",
        crate::pg_generation::PgGenerationError::Mapping(_) => unreachable!("handled above"),
    };
    ServeError::new(StartupCause::Schema {
        spec: "<prepared-source>".to_owned(),
        error: category.to_owned(),
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

#[cfg(test)]
mod tests {
    use sf_core::{Column, TableSchema};

    use super::*;
    use crate::SourceRef;

    const BASE: &str = "http://example.com/live/";

    fn table(primary_key: bool) -> TableSchema {
        let mut table = TableSchema::new("items");
        table.columns = vec![Column::new("id", "integer", true)];
        if primary_key {
            table.primary_key = vec!["id".to_owned()];
        }
        table
    }

    fn direct() -> PreparedMapping {
        PreparedMapping::new(&MappingRef::direct(BASE), source_id(0)).unwrap()
    }

    #[test]
    fn direct_mapping_base_is_validated_before_source_open() {
        let error = PreparedMapping::new(&MappingRef::direct("not an absolute IRI"), source_id(0))
            .expect_err("invalid base must fail during mapping preparation");
        assert_eq!(error.code(), "startup-configuration");
    }

    #[test]
    fn postgres_direct_mapping_rejects_no_primary_key_tables() {
        let error = direct()
            .finish_for(BackendKind::Postgres, &[table(false)])
            .expect_err("PostgreSQL no-PK identity is not admitted for live serving");
        assert_eq!(error.code(), "startup-configuration");
    }

    #[test]
    fn postgres_direct_mapping_accepts_declared_primary_keys() {
        let mapping = direct()
            .finish_for(BackendKind::Postgres, &[table(true)])
            .expect("declared primary key does not need a physical row identity");
        assert_eq!(mapping.len(), 1);
    }

    #[test]
    fn non_postgres_and_federated_direct_mapping_reject_before_connector_io() {
        let sqlite = SourceRef::inline("sqlite:/path/that/must/not/be/created.db")
            .resolve()
            .unwrap()
            .prepare()
            .unwrap();
        let postgres = SourceRef::inline("pg:host=database.invalid user=test")
            .resolve()
            .unwrap()
            .prepare()
            .unwrap();
        let mysql = SourceRef::inline("mysql://test@database.invalid/db")
            .resolve()
            .unwrap()
            .prepare()
            .unwrap();

        for source in [sqlite, mysql] {
            let error = admit_mapping_profile(&direct(), &source, None, None)
                .expect_err("unqualified backend must reject without connecting");
            assert_eq!(error.code(), "startup-configuration");
        }
        let error = admit_mapping_profile(
            &direct(),
            &postgres,
            Some(&PreparedMapping::Authored(sf_core::SourceMapping::new(
                source_id(1),
                Vec::new(),
            ))),
            Some(
                &SourceRef::inline("pg:host=other.invalid user=test")
                    .resolve()
                    .unwrap()
                    .prepare()
                    .unwrap(),
            ),
        )
        .expect_err("Direct Mapping federation must reject without connecting");
        assert_eq!(error.code(), "startup-configuration");
    }
}
