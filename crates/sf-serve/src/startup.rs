//! Startup assembly for single-source and bounded two-source serving modes.

use sf_core::query_control::QueryLimits;
use sf_core::SourceId;

use crate::problem::StartupCause;
use crate::run::ServeOptions;
use crate::semantic_admission::{MappingOrigin, ValidatedMapping};
use crate::snapshot::RuntimeSnapshot;
use crate::source::PreparedSource;
use crate::{
    IntrospectedSource, MappingRef, RuntimeSource, SemanticOntology, ServeConfig, ServeError,
};

#[cfg(test)]
use crate::BackendKind;

pub(crate) async fn build_config(
    opts: &ServeOptions,
    primary: PreparedSource,
    additional: Option<PreparedSource>,
) -> Result<(ServeConfig, crate::reload::Baseline), ServeError> {
    let inputs = crate::startup_inputs::SemanticInputs::capture(opts)?;
    let mut observations = Default::default();
    let snapshot = build_snapshot(opts, &inputs, primary, additional, None, |id, source| {
        crate::reload::Baseline::record(&mut observations, id, source);
        Ok(())
    })
    .await?;
    if crate::startup_inputs::SemanticInputs::capture(opts)? != inputs {
        return Err(configuration_error("semantic files changed during startup"));
    }
    let mode = if opts.additional_source.is_some() {
        crate::config::QueryMode::SourceAffineUnion([source_id(0), source_id(1)])
    } else {
        crate::config::QueryMode::Single(source_id(0))
    };
    let mut config = ServeConfig::from_snapshot(mode, snapshot);
    configure(opts, &mut config)?;
    Ok((config, crate::reload::Baseline::new(inputs, observations)))
}

pub(crate) fn configure(opts: &ServeOptions, config: &mut ServeConfig) -> Result<(), ServeError> {
    config.timeout = opts.timeout;
    config.set_query_admission(opts.query_admission.clone());
    config.set_max_query_len(opts.max_query_len)?;
    config.set_max_concurrent_requests(opts.max_concurrent_requests)?;
    config.set_max_order_rows(opts.max_order_rows);
    config.query_limits = QueryLimits::new(
        opts.max_compiler_work,
        opts.max_source_work,
        opts.max_result_items,
        opts.max_serialized_bytes,
    )
    .with_max_retained_bytes(opts.max_order_bytes);
    Ok(())
}

pub(crate) async fn build_snapshot(
    opts: &ServeOptions,
    inputs: &crate::startup_inputs::SemanticInputs,
    primary: PreparedSource,
    additional: Option<PreparedSource>,
    generation_budget: Option<&crate::budget::RequestBudget>,
    mut observe: impl FnMut(SourceId, &IntrospectedSource) -> Result<(), ServeError>,
) -> Result<RuntimeSnapshot, ServeError> {
    let ontology = SemanticOntology::from_turtle(&inputs.ontology)
        .map_err(|error| ServeError::new(StartupCause::OntologyParse { error }))?;
    let primary_mapping = PreparedMapping::from_capture(
        &opts.mapping,
        source_id(0),
        &ontology,
        inputs.primary.as_deref(),
    )?;
    let additional_mapping = opts
        .additional_source
        .as_ref()
        .map(|options| {
            PreparedMapping::from_capture(
                &options.mapping,
                source_id(1),
                &ontology,
                inputs.additional.as_deref(),
            )
        })
        .transpose()?;
    admit_mapping_profile(
        &primary_mapping,
        &primary,
        additional_mapping.as_ref(),
        additional.as_ref(),
    )?;
    if opts.require_verified_generation {
        let PreparedMapping::Authored(mapping) = primary_mapping else {
            return Err(configuration_error("verified authored mapping is required"));
        };
        let primary = crate::startup_authored::build_source(
            opts,
            primary,
            mapping,
            &ontology,
            generation_budget,
            &mut observe,
        )
        .await?;
        return RuntimeSnapshot::single(sf_sparql::Epoch::default(), ontology, primary)
            .map_err(snapshot_error);
    }
    let primary = open_source(opts, primary).await?;
    observe(source_id(0), &primary)?;
    let primary = primary_mapping.finish(opts, primary, &ontology).await?;

    match (additional_mapping, additional) {
        (None, None) => RuntimeSnapshot::single(sf_sparql::Epoch::default(), ontology, primary),
        (Some(additional_mapping), Some(additional)) => {
            let additional = open_source(opts, additional).await?;
            observe(source_id(1), &additional)?;
            let additional = additional_mapping
                .finish(opts, additional, &ontology)
                .await?;
            RuntimeSnapshot::new(
                sf_sparql::Epoch::default(),
                ontology,
                vec![primary, additional],
            )
        }
        _ => {
            return Err(ServeError::new(StartupCause::Configuration {
                error: "additional source and mapping must be configured together".to_owned(),
            }))
        }
    }
    .map_err(snapshot_error)
}

fn source_id(index: usize) -> SourceId {
    SourceId::new(index).expect("the fixed two-source profile uses representable slots")
}

#[derive(Debug)]
enum PreparedMapping {
    Authored(sf_core::SourceMapping),
    Direct {
        #[allow(dead_code, reason = "retained for the sealed lifecycle handoff")]
        base_iri: String,
        #[allow(dead_code, reason = "retained for the sealed lifecycle handoff")]
        source_id: SourceId,
    },
}

impl PreparedMapping {
    #[cfg(test)]
    fn new(
        mapping: &MappingRef,
        source_id: SourceId,
        ontology: &SemanticOntology,
    ) -> Result<Self, ServeError> {
        Self::from_capture(mapping, source_id, ontology, None)
    }

    fn from_capture(
        mapping: &MappingRef,
        source_id: SourceId,
        ontology: &SemanticOntology,
        turtle: Option<&str>,
    ) -> Result<Self, ServeError> {
        match mapping {
            MappingRef::R2rmlFile(_) | MappingRef::R2rmlFileWithBase { .. } => {
                let turtle =
                    turtle.ok_or_else(|| configuration_error("missing captured mapping"))?;
                let mut options = sf_mapping::R2rmlOptions::default();
                if let MappingRef::R2rmlFileWithBase { base_iri, .. } = mapping {
                    options.processor_base_iri = base_iri;
                }
                sf_mapping::parse_r2rml_for_source_with_options(turtle, source_id, options)
                    .map_err(mapping_error)
                    .and_then(|mapping| {
                        ValidatedMapping::preflight(&mapping, ontology)
                            .map_err(semantic_admission_error)?;
                        Ok(Self::Authored(mapping))
                    })
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
        ontology: &SemanticOntology,
    ) -> Result<RuntimeSource, ServeError> {
        match self {
            Self::Authored(mapping) => {
                let mapping =
                    ValidatedMapping::validate(mapping, MappingOrigin::Authored, ontology, &source)
                        .map_err(semantic_admission_error)?;
                RuntimeSource::admitted(source, mapping).map_err(semantic_admission_error)
            }
            Self::Direct {
                base_iri: _,
                source_id: _,
            } => {
                let _ = (opts, source, ontology);
                Err(configuration_error(
                    "live Direct Mapping requires the sealed lifecycle builder",
                ))
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
            Self::Authored(_) => Err(configuration_error(
                "the Direct Mapping test helper requires a Direct Mapping candidate",
            )),
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

/// Keep authored assembly separate from the leased Direct lifecycle assembler.
/// SQLite's hidden row identity is shadowable,
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
    let direct_requested =
        primary_mapping.is_direct() || additional_mapping.is_some_and(PreparedMapping::is_direct);
    if !direct_requested {
        return Ok(());
    }
    if additional_mapping.is_some() {
        return Err(configuration_error(
            "live Direct Mapping admits exactly one source and no authored companion mapping",
        ));
    }
    if !matches!(primary_source, PreparedSource::Postgres { .. }) {
        return Err(configuration_error(
            "live Direct Mapping admits only the closed PostgreSQL-16 profile",
        ));
    }
    Err(configuration_error(
        "live Direct Mapping requires its dedicated leased lifecycle assembler",
    ))
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

fn semantic_admission_error(error: crate::SemanticAdmissionError) -> ServeError {
    ServeError::new(StartupCause::Configuration {
        error: error.to_string(),
    })
}

fn snapshot_error(error: crate::SnapshotError) -> ServeError {
    ServeError::new(StartupCause::Configuration {
        error: error.to_string(),
    })
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
        PreparedMapping::new(
            &MappingRef::direct(BASE),
            source_id(0),
            &crate::test_support::empty_ontology(),
        )
        .unwrap()
    }

    fn authored_empty(index: usize) -> PreparedMapping {
        PreparedMapping::Authored(sf_core::SourceMapping::new(source_id(index), Vec::new()))
    }

    #[test]
    fn direct_mapping_base_is_validated_before_source_open() {
        let error = PreparedMapping::new(
            &MappingRef::direct("not an absolute IRI"),
            source_id(0),
            &crate::test_support::empty_ontology(),
        )
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
    fn authored_assembler_rejects_direct_mapping_before_connector_io() {
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
                .expect_err("non-PostgreSQL Direct Mapping must reject without connecting");
            assert_eq!(error.code(), "startup-configuration");
        }
        let error = admit_mapping_profile(&direct(), &postgres, None, None)
            .expect_err("Direct requires its dedicated leased lifecycle assembler");
        assert_eq!(error.code(), "startup-configuration");

        let postgres = SourceRef::inline("pg:host=database.invalid user=test")
            .resolve()
            .unwrap()
            .prepare()
            .unwrap();
        let error = admit_mapping_profile(
            &direct(),
            &postgres,
            Some(&authored_empty(1)),
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

        let primary_authored = authored_empty(0);
        let additional_direct = PreparedMapping::new(
            &MappingRef::direct(BASE),
            source_id(1),
            &crate::test_support::empty_ontology(),
        )
        .unwrap();
        let error = admit_mapping_profile(
            &primary_authored,
            &postgres,
            Some(&additional_direct),
            Some(
                &SourceRef::inline("pg:host=additional.invalid user=test")
                    .resolve()
                    .unwrap()
                    .prepare()
                    .unwrap(),
            ),
        )
        .expect_err("an authored plus Direct profile must reject without connecting");
        assert_eq!(error.code(), "startup-configuration");
    }
}
