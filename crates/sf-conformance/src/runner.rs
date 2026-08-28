//! The per-case CONSTRUCT runner (ADR-0005): build the SQLite fixture, load the
//! mapping, run the virtual graph dump, and adjudicate it by graph isomorphism.
//!
//! Receipt execution consumes captured [`SealedCase`] text only. The legacy
//! path entry point remains for existing callers, but funnels into the same
//! classified execution core after loading each input once.

use std::path::Path;

use sf_sparql::{exec, parse_and_translate_with, Error as SparqlError, Tbox};
use sf_sql::Dialect;

use crate::graph::{
    has_named_graph, isomorphic, parse_nquads, parse_turtle, quads_to_dataset, triples_to_dataset,
};
use crate::manifest::{Case, Kind};
use crate::oracle;
use crate::sealed_suite::{
    Backend, ClassifiedCaseResult, ClassifiedReport, OutcomeCode, SealedCase, SealedSuite,
};
use crate::{sqlite, CaseResult, Status};

const DUMP: &str = "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }";
const BASE: &str = "http://example.com/base/";

struct CaseInputs<'a> {
    create_sql: &'a str,
    mapping: Option<&'a str>,
    output: Option<&'a str>,
}

struct OwnedCaseInputs {
    create_sql: String,
    mapping: Option<String>,
    output: Option<String>,
}

impl OwnedCaseInputs {
    fn load(dir: &Path, case: &Case) -> Result<Self, String> {
        let create_sql =
            read(dir, "create.sql").map_err(|error| input_error(case, "create.sql", &error))?;
        let mapping = case
            .mapping_document
            .as_deref()
            .map(|name| read(dir, name).map_err(|error| input_error(case, name, &error)))
            .transpose()?;
        let output = case
            .output
            .as_deref()
            .map(|name| read(dir, name).map_err(|error| input_error(case, name, &error)))
            .transpose()?;
        Ok(Self {
            create_sql,
            mapping,
            output,
        })
    }

    fn as_ref(&self) -> CaseInputs<'_> {
        CaseInputs {
            create_sql: &self.create_sql,
            mapping: self.mapping.as_deref(),
            output: self.output.as_deref(),
        }
    }
}

struct CaseOutcome {
    status: Status,
    code: OutcomeCode,
    reason: String,
}

fn outcome(status: Status, code: OutcomeCode, reason: impl Into<String>) -> CaseOutcome {
    CaseOutcome {
        status,
        code,
        reason: reason.into(),
    }
}

/// Legacy path-based entry point used by the general conformance runner.
pub fn run_case(dir: &Path, case: &Case) -> Result<CaseResult, String> {
    let inputs = OwnedCaseInputs::load(dir, case)?;
    let result = run_with_inputs(case, inputs.as_ref())?;
    Ok(CaseResult {
        id: case.identifier.clone(),
        kind: case.kind,
        status: result.status,
        reason: result.reason,
    })
}

/// Execute a previously captured sealed suite with no post-seal filesystem read.
pub fn run_sealed_suite(sealed: &SealedSuite) -> Result<ClassifiedReport, String> {
    let cases = sealed
        .cases()
        .iter()
        .map(run_sealed_case)
        .collect::<Result<Vec<_>, _>>()?;
    let report = ClassifiedReport { cases };
    sealed.validate_classified_report(Backend::Sqlite, &report)?;
    Ok(report)
}

fn run_sealed_case(entry: &SealedCase) -> Result<ClassifiedCaseResult, String> {
    let case = &entry.case;
    let result = run_with_inputs(
        case,
        CaseInputs {
            create_sql: entry.create_sql(),
            mapping: entry.mapping_text(),
            output: entry.output_text(),
        },
    )?;
    Ok(ClassifiedCaseResult {
        id: case.identifier.clone(),
        kind: case.kind,
        status: result.status,
        outcome_code: result.code,
        reason: result.reason,
    })
}

fn run_with_inputs(case: &Case, inputs: CaseInputs<'_>) -> Result<CaseOutcome, String> {
    match case.kind {
        Kind::R2rml => run_r2rml(case, inputs),
        Kind::DirectMapping => run_direct(case, inputs),
    }
}

fn run_r2rml(case: &Case, inputs: CaseInputs<'_>) -> Result<CaseOutcome, String> {
    let conn = match sqlite::load(inputs.create_sql) {
        Ok(connection) => connection,
        Err(error) => {
            return Ok(outcome(
                Status::Skipped,
                OutcomeCode::FixtureLoadError,
                format!("fixture: {error}"),
            ))
        }
    };
    let doc = case
        .mapping_document
        .as_deref()
        .ok_or_else(|| input_error(case, "mappingDocument", "missing from sealed case"))?;
    let ttl = inputs
        .mapping
        .ok_or_else(|| input_error(case, doc, "missing from sealed snapshot"))?;
    let maps = match sf_mapping::parse_r2rml(ttl) {
        Ok(maps) => maps,
        Err(error) => {
            return Ok(classified_error(
                case,
                OutcomeCode::MappingError,
                format!("mapping parse: {error}"),
            ))
        }
    };
    if let Err(error) = validate_query_sources(&conn, &maps) {
        return Ok(classified_error(
            case,
            OutcomeCode::SourceValidationError,
            error,
        ));
    }
    let schemas = match sqlite::introspect_all(&conn) {
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
        match parse_and_translate_with(DUMP, &maps, Dialect::Sqlite, &Tbox::default(), &schemas) {
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
    let triples = match exec::construct_triples(&plan, &conn) {
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
    if !case.has_expected_output {
        return Ok(outcome(
            Status::Failed,
            OutcomeCode::UnexpectedOutput,
            "error case: engine produced output instead of signalling an error",
        ));
    }

    let out = case
        .output
        .as_deref()
        .ok_or_else(|| input_error(case, "output", "missing from sealed positive case"))?;
    let expected_text = inputs
        .output
        .ok_or_else(|| input_error(case, out, "missing from sealed snapshot"))?;
    let expected = parse_nquads(expected_text)
        .map_err(|error| input_error(case, out, &format!("invalid N-Quads: {error}")))?;
    if has_named_graph(&expected) {
        let quads = match exec::dump_quads(&maps, &conn, Dialect::Sqlite) {
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

fn run_direct(case: &Case, inputs: CaseInputs<'_>) -> Result<CaseOutcome, String> {
    let conn = match sqlite::load(inputs.create_sql) {
        Ok(connection) => connection,
        Err(error) => {
            return Ok(outcome(
                Status::Skipped,
                OutcomeCode::FixtureLoadError,
                format!("fixture: {error}"),
            ))
        }
    };
    let schemas = match sqlite::introspect_all(&conn) {
        Ok(schemas) => schemas,
        Err(error) => {
            return Ok(outcome(
                Status::Skipped,
                OutcomeCode::IntrospectionError,
                format!("introspect: {error}"),
            ))
        }
    };
    let maps = match sf_mapping::direct_mapping(&schemas, BASE) {
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
        match parse_and_translate_with(DUMP, &maps, Dialect::Sqlite, &Tbox::default(), &schemas) {
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
    let triples = match exec::construct_triples(&plan, &conn) {
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
    let out = case
        .output
        .as_deref()
        .ok_or_else(|| input_error(case, "output", "missing from sealed positive case"))?;
    let expected_text = inputs
        .output
        .ok_or_else(|| input_error(case, out, "missing from sealed snapshot"))?;
    let expected = parse_turtle(expected_text, BASE)
        .map_err(|error| input_error(case, out, &format!("invalid Turtle: {error}")))?;
    Ok(classify_comparison(
        compare(&triples, &expected),
        OutcomeCode::GraphMatched,
        OutcomeCode::GraphMismatch,
    ))
}

fn classified_error(case: &Case, code: OutcomeCode, detail: String) -> CaseOutcome {
    let (status, reason) = parse_error_outcome(case, &detail);
    outcome(status, code, reason)
}

fn classify_comparison(
    (status, reason): (Status, String),
    passed: OutcomeCode,
    failed: OutcomeCode,
) -> CaseOutcome {
    outcome(
        status,
        if status == Status::Passed {
            passed
        } else {
            failed
        },
        reason,
    )
}

pub(crate) fn compare_quads(quads: &[oxrdf::Quad], expected: &oxrdf::Dataset) -> (Status, String) {
    let engine = quads_to_dataset(quads);
    if isomorphic(&engine, expected) {
        (Status::Passed, String::new())
    } else {
        (
            Status::Failed,
            format!(
                "named-graph mismatch: engine produced {} quads, expected {}",
                engine.len(),
                expected.len()
            ),
        )
    }
}

pub(crate) fn input_error(case: &Case, input: &str, detail: &str) -> String {
    format!(
        "sealed input error for {} ({input}): {detail}",
        case.identifier
    )
}

pub(crate) fn compare(triples: &[sf_core::Triple], expected: &oxrdf::Dataset) -> (Status, String) {
    let engine = triples_to_dataset(triples);
    let oracle = oracle::evaluate_dump(expected);
    if isomorphic(&engine, &oracle) {
        (Status::Passed, String::new())
    } else {
        (
            Status::Failed,
            format!(
                "graph mismatch: engine produced {} triples, expected {}",
                engine.len(),
                oracle.len()
            ),
        )
    }
}

pub(crate) fn parse_error_outcome(case: &Case, detail: &str) -> (Status, String) {
    if case.has_expected_output {
        (Status::Failed, detail.to_owned())
    } else {
        (
            Status::Passed,
            format!("error correctly surfaced — {detail}"),
        )
    }
}

fn validate_query_sources(
    conn: &rusqlite::Connection,
    maps: &[sf_core::ir::TriplesMap],
) -> Result<(), String> {
    use sf_core::ir::LogicalSource;
    for map in maps {
        if let LogicalSource::Query(query) = &map.source {
            if let Ok(names) = sf_sql::sqlite_column_names(conn, query) {
                let mut seen = std::collections::HashSet::new();
                for name in &names {
                    if !seen.insert(name.as_str()) {
                        return Err(format!(
                            "rr:sqlQuery produces duplicate column name {name:?} (R2RML §5.1)"
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn read(dir: &Path, file: &str) -> Result<String, String> {
    std::fs::read_to_string(dir.join(file)).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_sealed_input_is_an_error_not_a_skip() {
        let case = Case {
            identifier: "R2RMLTC0000".to_owned(),
            kind: Kind::R2rml,
            mapping_document: Some("r2rml.ttl".to_owned()),
            output: Some("mapped.nq".to_owned()),
            has_expected_output: true,
        };
        let error = run_case(Path::new("/definitely/not/a/w3c/scenario"), &case)
            .expect_err("missing input must fail closed");
        assert!(error.contains("sealed input error"), "{error}");
        assert!(error.contains("create.sql"), "{error}");
    }
}
