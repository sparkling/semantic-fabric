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
    let mapped = crate::pg_generation::authored::mapped_tables(&mapping)?;
    let columns = required_columns(&mapping)?;
    // Each builder calls this while the candidate's observed schema is held.
    // Fence drift first, then reject missing policy-only columns before activation.
    let mut observe_policy = |id, source: &crate::IntrospectedSource| {
        observe(id, source)?;
        if !columns_exist(&columns, source.observed_schema(), source.backend().kind())
            || !opts
                .query_admission
                .portable_columns_exist(id, &mapped, source.observed_schema())
        {
            return Err(crate::pg_generation::authored::generation_error(
                crate::pg_generation::PgGenerationError::CapabilityDrift,
            ));
        }
        Ok(())
    };
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
            &mut observe_policy,
        )
        .await?;
        return crate::RuntimeSource::admitted(source, mapping).map_err(|_| configuration());
    }
    if let PreparedSource::Mysql { options, .. } = prepared {
        let (source, mapping) = crate::mysql_generation::build(
            options,
            mapping,
            ontology,
            &budget,
            &mut observe_policy,
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
    let (source, mapping) = crate::pg_generation::authored::build(
        &pools,
        mapping,
        ontology,
        &budget,
        &mut observe_policy,
    )
    .await?;
    crate::RuntimeSource::admitted(source, mapping).map_err(|_| configuration())
}

type RequiredColumns = std::collections::BTreeMap<String, std::collections::BTreeSet<String>>;

// Explicit RDF datatypes bypass implicit-type lookup during semantic admission.
// Validate every referenced SQL column separately while the candidate is held.
fn required_columns(mapping: &sf_core::SourceMapping) -> Result<RequiredColumns, ServeError> {
    use sf_core::ir::{LogicalSource, ObjectMap, Segment, TermMap};
    let mut tables = std::collections::BTreeMap::new();
    for map in mapping.triples_maps() {
        let LogicalSource::Table(table) = &map.source else {
            return Err(configuration());
        };
        if tables.insert(map.id.as_str(), table.as_str()).is_some() {
            return Err(configuration());
        }
    }
    fn add(out: &mut RequiredColumns, table: &str, term: &TermMap) {
        let columns = out.entry(table.to_owned()).or_default();
        match term {
            TermMap::Constant(_) => {}
            TermMap::Column(column, _) => {
                columns.insert(column.to_string());
            }
            TermMap::Template(template, _) => {
                for segment in template.segments() {
                    if let Segment::Column(column) = segment {
                        columns.insert(column.to_string());
                    }
                }
            }
        }
    }
    let mut out = RequiredColumns::new();
    for map in mapping.triples_maps() {
        let table = tables[map.id.as_str()];
        add(&mut out, table, &map.subject.term);
        for graph in &map.subject.graphs {
            add(&mut out, table, graph);
        }
        for pom in &map.predicate_object_maps {
            for term in pom.predicates.iter().chain(&pom.graphs) {
                add(&mut out, table, term);
            }
            for object in &pom.objects {
                match object {
                    ObjectMap::Term(term) => add(&mut out, table, term),
                    ObjectMap::Ref(reference) => {
                        let parent = tables
                            .get(reference.parent_triples_map.as_str())
                            .ok_or_else(configuration)?;
                        for join in &reference.joins {
                            out.entry(table.to_owned())
                                .or_default()
                                .insert(join.child.clone());
                            out.entry((*parent).to_owned())
                                .or_default()
                                .insert(join.parent.clone());
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

fn columns_exist(
    required: &RequiredColumns,
    schema: &[sf_sql::TableSchema],
    backend: crate::BackendKind,
) -> bool {
    required.iter().all(|(name, columns)| {
        let mut tables = schema.iter().filter(|table| table.name == *name);
        let Some(table) = tables.next() else {
            return false;
        };
        tables.next().is_none()
            && columns.iter().all(|name| {
                let exact = table
                    .columns
                    .iter()
                    .filter(|column| column.name == *name)
                    .count();
                if exact != 0 {
                    return exact == 1;
                }
                let folded = table
                    .columns
                    .iter()
                    .filter(|column| column.name.eq_ignore_ascii_case(name))
                    .count();
                if folded != 0 {
                    return folded == 1;
                }
                // Preserve the executor's base-table physical-identity sentinel.
                name == "rowid"
                    && matches!(
                        backend,
                        crate::BackendKind::Postgres | crate::BackendKind::Sqlite
                    )
            })
    })
}

/// Keep reload work owned through actual completion and forced shutdown.
pub(crate) fn control_budget(
    config: Option<&crate::ServeConfig>,
    source: &PreparedSource,
) -> Result<crate::budget::RequestBudget, ServeError> {
    control_budget_pair(config, source, None)
}

pub(crate) fn control_budget_pair(
    config: Option<&crate::ServeConfig>,
    source: &PreparedSource,
    additional: Option<&PreparedSource>,
) -> Result<crate::budget::RequestBudget, ServeError> {
    let allowance = std::iter::once(source)
        .chain(additional)
        .try_fold(0u64, |sum, source| {
            let amount = match source {
                PreparedSource::Sqlite { .. } => crate::sqlite_generation::CONTROL_SOURCE_WORK,
                PreparedSource::Mysql { .. } => crate::mysql_generation::CONTROL_SOURCE_WORK,
                _ => crate::pg_generation::PG_DIRECT_CONTROL_SOURCE_WORK_V1,
            };
            sum.checked_add(amount).ok_or_else(configuration)
        })?;
    let mut budget = crate::budget::RequestBudget::for_control_with_shutdown(
        std::time::Duration::from_secs(30),
        sf_core::query_control::QueryLimits::new(0, allowance, 0, 0),
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
        && (matches!(opts.mapping, MappingRef::Direct { .. })
            || opts
                .additional_source
                .as_ref()
                .is_some_and(|source| matches!(source.mapping, MappingRef::Direct { .. }))
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
    Ok(())
}

fn configuration() -> ServeError {
    ServeError::new(StartupCause::Configuration {
        error: "verified authored generation requires one or two qualified PostgreSQL, MySQL or file-backed SQLite sources, authored base-table mappings, a nonzero reload interval and read-all or portable row admission".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::query_control::{QueryCharge, QueryControl};

    #[test]
    fn explicit_terms_templates_graphs_and_reference_join_columns_must_exist() {
        let maps = sf_mapping::parse_r2rml(
            r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#child> a rr:TriplesMap; rr:logicalTable [rr:tableName "child"];
rr:subjectMap [rr:template "urn:child:{id}"; rr:graphMap [rr:template "urn:graph:{scope}"]];
rr:predicateObjectMap [rr:predicateMap [rr:column "predicate"; rr:termType rr:IRI];
rr:graphMap [rr:column "graph"; rr:termType rr:IRI];
rr:objectMap [rr:column "value"; rr:datatype <http://www.w3.org/2001/XMLSchema#string>]];
rr:predicateObjectMap [rr:predicate <urn:parent>; rr:objectMap [rr:parentTriplesMap <#parent>;
rr:joinCondition [rr:child "foreign_key"; rr:parent "key"]]].
<#parent> a rr:TriplesMap; rr:logicalTable [rr:tableName "parent"];
rr:subjectMap [rr:template "urn:parent:{id}"] .
"#,
        )
        .unwrap();
        let required = required_columns(&sf_core::SourceMapping::new(
            sf_core::SourceId::new(0).unwrap(),
            maps,
        ))
        .unwrap();
        let mut schema = Vec::new();
        for (name, columns) in [
            (
                "child",
                &["id", "scope", "predicate", "graph", "value", "foreign_key"][..],
            ),
            ("parent", &["id", "key"][..]),
        ] {
            let mut table = sf_sql::TableSchema::new(name);
            table.columns = columns
                .iter()
                .map(|name| sf_core::Column::new(*name, "TEXT", false))
                .collect();
            schema.push(table);
        }
        assert!(columns_exist(
            &required,
            &schema,
            crate::BackendKind::Sqlite
        ));
        for table in 0..schema.len() {
            for column in 0..schema[table].columns.len() {
                let mut missing = schema.clone();
                missing[table].columns.remove(column);
                assert!(
                    !columns_exist(&required, &missing, crate::BackendKind::Sqlite),
                    "missing {table}:{column}"
                );
            }
        }
        let mut duplicate = schema.clone();
        duplicate[0].columns.push(schema[0].columns[0].clone());
        assert!(!columns_exist(
            &required,
            &duplicate,
            crate::BackendKind::Sqlite
        ));
        schema.push(schema[0].clone());
        assert!(!columns_exist(
            &required,
            &schema,
            crate::BackendKind::Sqlite
        ));
    }

    #[test]
    fn column_resolution_matches_executor_case_and_physical_identity_rules() {
        let required = [("items".into(), ["VALUE".into()].into())].into();
        let mut table = sf_sql::TableSchema::new("items");
        table
            .columns
            .push(sf_core::Column::new("value", "TEXT", false));
        assert!(columns_exist(
            &required,
            &[table.clone()],
            crate::BackendKind::MySql
        ));
        table
            .columns
            .push(sf_core::Column::new("Value", "TEXT", false));
        assert!(!columns_exist(
            &required,
            &[table.clone()],
            crate::BackendKind::MySql
        ));
        table
            .columns
            .push(sf_core::Column::new("VALUE", "TEXT", false));
        assert!(columns_exist(
            &required,
            &[table.clone()],
            crate::BackendKind::MySql
        ));
        let required = [("items".into(), ["rowid".into()].into())].into();
        for backend in [crate::BackendKind::Sqlite, crate::BackendKind::Postgres] {
            assert!(columns_exist(&required, &[table.clone()], backend));
        }
        assert!(!columns_exist(
            &required,
            &[table],
            crate::BackendKind::MySql
        ));
    }

    #[tokio::test]
    async fn paired_control_allowance_is_cumulative_across_clones() {
        let sources = [
            (
                "sqlite:/unused.db",
                crate::sqlite_generation::CONTROL_SOURCE_WORK,
            ),
            (
                "pg:host=127.0.0.1",
                crate::pg_generation::PG_DIRECT_CONTROL_SOURCE_WORK_V1,
            ),
            (
                "mysql://localhost/test",
                crate::mysql_generation::CONTROL_SOURCE_WORK,
            ),
        ];
        for (left, left_work) in sources {
            for (right, right_work) in sources {
                let prepare = |text| {
                    crate::SourceRef::inline(text)
                        .resolve()
                        .unwrap()
                        .prepare()
                        .unwrap()
                };
                let budget =
                    control_budget_pair(None, &prepare(left), Some(&prepare(right))).unwrap();
                let second = budget.clone();
                budget.consume(QueryCharge::SourceWork, left_work).unwrap();
                second.consume(QueryCharge::SourceWork, right_work).unwrap();
                assert!(budget.consume(QueryCharge::SourceWork, 1).is_err());
                assert!(second.checkpoint().is_err());
            }
        }
    }
}
