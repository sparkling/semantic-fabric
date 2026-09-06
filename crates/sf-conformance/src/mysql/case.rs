use std::collections::HashSet;

use mysql_async::prelude::Queryable;
use mysql_async::Conn;
use sf_sparql::{exec_mysql, parse_and_translate_with, Error as SparqlError, Tbox};
use sf_sql::backend::mysql::MysqlTypeProfile;
use sf_sql::introspect::introspect_mysql;
use sf_sql::{Dialect, Error as SqlError, TableSchema};

use super::outcome::{classified_error, classify_comparison, outcome, CaseOutcome};
use crate::graph::{has_named_graph, parse_nquads, parse_turtle};
use crate::manifest::Kind;
use crate::runner::{compare, compare_quads, input_error};
use crate::sealed_suite::{
    Backend, ClassifiedCaseResult, ClassifiedReport, OutcomeCode, SealedCase, SealedSuite,
};
use crate::Status;

const DUMP: &str = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";
const BASE: &str = "http://example.com/base/";
const TYPE_PROFILE: MysqlTypeProfile = MysqlTypeProfile::W3cSql2008;

pub(super) async fn run_cases(
    sealed: &SealedSuite,
    conn: &mut Conn,
) -> Result<ClassifiedReport, String> {
    let mut cases = Vec::with_capacity(sealed.cases().len());
    for entry in sealed.cases() {
        let outcome = match entry.case.kind {
            Kind::R2rml => run_r2rml(entry, conn).await,
            Kind::DirectMapping => run_direct(entry, conn).await,
        }?;
        cases.push(ClassifiedCaseResult {
            id: entry.case.identifier.clone(),
            kind: entry.case.kind,
            status: outcome.status,
            outcome_code: outcome.code,
            reason: outcome.reason,
        });
    }
    let report = ClassifiedReport { cases };
    sealed.validate_classified_report(Backend::MySql, &report)?;
    Ok(report)
}

async fn load_fixture(conn: &mut Conn, sql: &str) -> Result<(), String> {
    reset_schema(conn).await?;
    let statements = super::sql_split::split(sql)?;
    for (index, statement) in statements.into_iter().enumerate() {
        conn.query_drop(statement)
            .await
            .map_err(|_| format!("create.sql statement {} failed", index + 1))?;
    }
    Ok(())
}

async fn reset_schema(conn: &mut Conn) -> Result<(), String> {
    conn.query_drop("SET FOREIGN_KEY_CHECKS = 0")
        .await
        .map_err(|_| "disable MySQL fixture foreign-key checks failed".to_owned())?;
    let reset = async {
        let tables: Vec<String> = conn
            .query(
                "SELECT table_name FROM information_schema.tables \
                 WHERE table_schema = DATABASE() AND table_type = 'BASE TABLE' \
                 ORDER BY table_name",
            )
            .await
            .map_err(|_| "list MySQL fixture tables failed".to_owned())?;
        for table in tables {
            conn.query_drop(format!("DROP TABLE {}", Dialect::MySql.quote_ident(&table)))
                .await
                .map_err(|_| "drop MySQL fixture table failed".to_owned())?;
        }
        Ok(())
    }
    .await;
    let enabled = conn
        .query_drop("SET FOREIGN_KEY_CHECKS = 1")
        .await
        .map_err(|_| "restore MySQL fixture foreign-key checks failed".to_owned());
    match (reset, enabled) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), _) | (Ok(()), Err(error)) => Err(error),
    }
}

async fn introspect_all(conn: &mut Conn) -> Result<Vec<TableSchema>, String> {
    let names: Vec<String> = conn
        .query(
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = DATABASE() AND table_type = 'BASE TABLE' ORDER BY table_name",
        )
        .await
        .map_err(|_| "list MySQL base tables failed".to_owned())?;
    let mut schemas = Vec::with_capacity(names.len());
    for name in names {
        schemas.push(
            introspect_mysql(conn, &name)
                .await
                .map_err(closed_introspection_reason)?,
        );
    }
    Ok(schemas)
}

fn closed_introspection_reason(_error: SqlError) -> String {
    "MySQL catalogue introspection failed".to_owned()
}

async fn validate_query_sources(
    conn: &mut Conn,
    maps: &[sf_core::ir::TriplesMap],
) -> Result<(), String> {
    use sf_core::ir::LogicalSource;
    for map in maps {
        if let LogicalSource::Query(query) = &map.source {
            let columns = sf_sql::stream::mysql_column_names(conn, query)
                .await
                .map_err(|_| "rr:sqlQuery metadata validation failed".to_owned())?;
            let mut seen = HashSet::new();
            for column in columns {
                if !seen.insert(column) {
                    return Err(
                        "rr:sqlQuery produces duplicate result-column names (R2RML §5.1)"
                            .to_owned(),
                    );
                }
            }
        }
    }
    Ok(())
}

async fn run_r2rml(entry: &SealedCase, conn: &mut Conn) -> Result<CaseOutcome, String> {
    let case = &entry.case;
    if let Err(error) = load_fixture(conn, entry.create_sql()).await {
        return Ok(outcome(
            Status::Skipped,
            OutcomeCode::FixtureLoadError,
            format!("fixture: {error}"),
        ));
    }
    let document = case
        .mapping_document
        .as_deref()
        .ok_or_else(|| input_error(case, "mappingDocument", "missing from sealed case"))?;
    let text = entry
        .mapping_text()
        .ok_or_else(|| input_error(case, document, "missing from sealed snapshot"))?;
    let mut maps = match sf_mapping::parse_r2rml(text) {
        Ok(maps) => maps,
        Err(error) => {
            return Ok(classified_error(
                case,
                OutcomeCode::MappingError,
                format!("mapping parse: {error}"),
            ))
        }
    };
    if let Err(error) = super::query::normalize_sources(&mut maps) {
        return Ok(classified_error(
            case,
            OutcomeCode::SourceValidationError,
            error,
        ));
    }
    if let Err(error) = validate_query_sources(conn, &maps).await {
        return Ok(classified_error(
            case,
            OutcomeCode::SourceValidationError,
            error,
        ));
    }
    let schemas = match introspect_all(conn).await {
        Ok(schemas) => schemas,
        Err(error) => {
            return Ok(outcome(
                Status::Skipped,
                OutcomeCode::IntrospectionError,
                format!("introspect: {error}"),
            ))
        }
    };
    let plan =
        match parse_and_translate_with(DUMP, &maps, Dialect::MySql, &Tbox::default(), &schemas) {
            Ok(plan) => plan,
            Err(SparqlError::Unsupported(message)) => {
                return Ok(outcome(
                    Status::Skipped,
                    OutcomeCode::TranslationUnsupported,
                    format!("501 translate: {message}"),
                ))
            }
            Err(error) => {
                return Ok(classified_error(
                    case,
                    OutcomeCode::TranslationError,
                    format!("translate: {error}"),
                ))
            }
        };
    let triples = match exec_mysql::construct_triples_mysql_with_type_profile(
        &plan,
        conn,
        TYPE_PROFILE,
    )
    .await
    {
        Ok(triples) => triples,
        Err(SparqlError::Unsupported(message)) => {
            return Ok(outcome(
                Status::Skipped,
                OutcomeCode::ExecutionUnsupported,
                format!("501 exec: {message}"),
            ))
        }
        Err(error) => {
            return Ok(classified_error(
                case,
                OutcomeCode::ExecutionError,
                format!("exec: {error}"),
            ))
        }
    };
    compare_r2rml_output(entry, conn, &maps, triples).await
}

async fn compare_r2rml_output(
    entry: &SealedCase,
    conn: &mut Conn,
    maps: &[sf_core::ir::TriplesMap],
    triples: Vec<sf_core::Triple>,
) -> Result<CaseOutcome, String> {
    let case = &entry.case;
    if !case.has_expected_output {
        return Ok(outcome(
            Status::Failed,
            OutcomeCode::UnexpectedOutput,
            "error case: engine produced output instead of signalling an error",
        ));
    }
    let output = case
        .output
        .as_deref()
        .ok_or_else(|| input_error(case, "output", "missing from sealed positive case"))?;
    let expected_text = entry
        .output_text()
        .ok_or_else(|| input_error(case, output, "missing from sealed snapshot"))?;
    let expected = parse_nquads(expected_text)
        .map_err(|error| input_error(case, output, &format!("invalid N-Quads: {error}")))?;
    if has_named_graph(&expected) {
        let quads = match exec_mysql::dump_quads_mysql_with_type_profile(
            maps,
            conn,
            Dialect::MySql,
            TYPE_PROFILE,
        )
        .await
        {
            Ok(quads) => quads,
            Err(SparqlError::Unsupported(message)) => {
                return Ok(outcome(
                    Status::Skipped,
                    OutcomeCode::QuadDumpUnsupported,
                    format!("501 quad dump: {message}"),
                ))
            }
            Err(error) => {
                return Ok(classified_error(
                    case,
                    OutcomeCode::QuadDumpError,
                    format!("quad dump: {error}"),
                ))
            }
        };
        return Ok(classify_comparison(
            compare_quads(&quads, &expected),
            OutcomeCode::DatasetMatched,
            OutcomeCode::DatasetMismatch,
        ));
    }
    Ok(classify_comparison(
        compare(&triples, &expected),
        OutcomeCode::GraphMatched,
        OutcomeCode::GraphMismatch,
    ))
}

async fn run_direct(entry: &SealedCase, conn: &mut Conn) -> Result<CaseOutcome, String> {
    let case = &entry.case;
    if let Err(error) = load_fixture(conn, entry.create_sql()).await {
        return Ok(outcome(
            Status::Skipped,
            OutcomeCode::FixtureLoadError,
            format!("fixture: {error}"),
        ));
    }
    let schemas = match introspect_all(conn).await {
        Ok(schemas) => schemas,
        Err(error) => {
            return Ok(outcome(
                Status::Skipped,
                OutcomeCode::IntrospectionError,
                format!("introspect: {error}"),
            ))
        }
    };
    let no_primary_key = schemas
        .iter()
        .find(|schema| schema.primary_key.is_empty())
        .map(|schema| schema.name.clone());
    if let Some(table) = no_primary_key {
        return Ok(outcome(
            Status::Skipped,
            OutcomeCode::DirectMappingUnsupported,
            format!("Direct Mapping table {table:?} has no declared primary key"),
        ));
    }
    let maps = match sf_mapping::direct_mapping_with_row_identity(
        &schemas,
        BASE,
        sf_mapping::DirectMappingRowIdentity::RequirePrimaryKey,
    ) {
        Ok(maps) => maps,
        Err(error) => {
            return Ok(outcome(
                Status::Failed,
                OutcomeCode::DirectMappingError,
                format!("direct mapping: {error}"),
            ))
        }
    };
    let plan =
        match parse_and_translate_with(DUMP, &maps, Dialect::MySql, &Tbox::default(), &schemas) {
            Ok(plan) => plan,
            Err(SparqlError::Unsupported(message)) => {
                return Ok(outcome(
                    Status::Skipped,
                    OutcomeCode::TranslationUnsupported,
                    format!("501 translate: {message}"),
                ))
            }
            Err(error) => {
                return Ok(outcome(
                    Status::Failed,
                    OutcomeCode::TranslationError,
                    format!("translate: {error}"),
                ))
            }
        };
    let triples = match exec_mysql::construct_triples_mysql_with_type_profile(
        &plan,
        conn,
        TYPE_PROFILE,
    )
    .await
    {
        Ok(triples) => triples,
        Err(SparqlError::Unsupported(message)) => {
            return Ok(outcome(
                Status::Skipped,
                OutcomeCode::ExecutionUnsupported,
                format!("501 exec: {message}"),
            ))
        }
        Err(error) => {
            return Ok(outcome(
                Status::Failed,
                OutcomeCode::ExecutionError,
                format!("exec: {error}"),
            ))
        }
    };
    if !case.has_expected_output {
        return Ok(outcome(
            Status::Failed,
            OutcomeCode::UnexpectedOutput,
            "error case: engine produced output instead of signalling an error",
        ));
    }
    let output = case
        .output
        .as_deref()
        .ok_or_else(|| input_error(case, "output", "missing from sealed positive case"))?;
    let expected_text = entry
        .output_text()
        .ok_or_else(|| input_error(case, output, "missing from sealed snapshot"))?;
    let expected = parse_turtle(expected_text, BASE)
        .map_err(|error| input_error(case, output, &format!("invalid Turtle: {error}")))?;
    Ok(classify_comparison(
        compare(&triples, &expected),
        OutcomeCode::GraphMatched,
        OutcomeCode::GraphMismatch,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn introspection_reason_does_not_reflect_typed_detail() {
        let marker = "private-table-name-must-not-appear";
        let reason = closed_introspection_reason(SqlError::Introspection(marker.to_owned()));
        assert_eq!(reason, "MySQL catalogue introspection failed");
        assert!(!reason.contains(marker));
    }
}
