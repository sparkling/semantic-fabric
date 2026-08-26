//! Non-skipping W3C RDB2RDF gate over the bundled DuckDB engine.

use std::path::PathBuf;

use sf_conformance::{duckdb, Kind};

fn cases_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/rdb2rdf/cases")
}

const EXPECTED_R2RML_PASSES: usize = 61;
const EXPECTED_DIRECT_MAPPING_PASSES: usize = 22;
const EXPECTED_SKIPS: &[&str] = &["DirectGraphTC0025"];
const SHARED_EXPECTED_DEVIATIONS: &[&str] = &["R2RMLTC0002f"];
// DuckDB aliases SQL CHAR to VARCHAR and does not preserve a declared width in
// its catalog, so it cannot produce the suite's required CHAR(15) space padding.
const DUCKDB_EXPECTED_DEVIATIONS: &[&str] = &["DirectGraphTC0018", "R2RMLTC0018a"];

#[test]
fn w3c_rdb2rdf_duckdb_conformance() {
    let report = duckdb::run(&cases_dir()).expect("embedded DuckDB suite must run");
    let r2rml_pass = report.passed(Some(Kind::R2rml));
    let direct_pass = report.passed(Some(Kind::DirectMapping));

    eprintln!("\n=== W3C RDB2RDF conformance (embedded DuckDB) ===");
    eprintln!("R2RML          {}", report.split(Kind::R2rml));
    eprintln!("Direct Mapping {}", report.split(Kind::DirectMapping));
    for failure in report.unexpected_failures() {
        eprintln!("  FAIL {failure}");
    }
    for skip in report.skips() {
        eprintln!("  SKIP {skip}");
    }

    let mut unexpected: Vec<_> = report
        .cases
        .iter()
        .filter(|case| case.status == sf_conformance::Status::Failed)
        .filter(|case| sf_conformance::expected_deviation(&case.id).is_none())
        .filter(|case| !DUCKDB_EXPECTED_DEVIATIONS.contains(&case.id.as_str()))
        .map(|case| case.id.as_str())
        .collect();
    unexpected.sort_unstable();
    assert!(unexpected.is_empty(), "unexpected failures: {unexpected:?}");

    let mut duckdb_deviations: Vec<_> = report
        .cases
        .iter()
        .filter(|case| {
            case.status == sf_conformance::Status::Failed
                && DUCKDB_EXPECTED_DEVIATIONS.contains(&case.id.as_str())
        })
        .map(|case| case.id.as_str())
        .collect();
    duckdb_deviations.sort_unstable();
    assert_eq!(
        duckdb_deviations, DUCKDB_EXPECTED_DEVIATIONS,
        "DuckDB deviation allow-list changed"
    );
    let mut shared_deviations: Vec<_> = report
        .cases
        .iter()
        .filter(|case| {
            case.status == sf_conformance::Status::Failed
                && SHARED_EXPECTED_DEVIATIONS.contains(&case.id.as_str())
        })
        .map(|case| case.id.as_str())
        .collect();
    shared_deviations.sort_unstable();
    assert_eq!(
        shared_deviations, SHARED_EXPECTED_DEVIATIONS,
        "shared deviation allow-list changed"
    );
    assert_eq!(
        r2rml_pass, EXPECTED_R2RML_PASSES,
        "DuckDB R2RML adjudication changed"
    );
    assert_eq!(
        direct_pass, EXPECTED_DIRECT_MAPPING_PASSES,
        "DuckDB Direct Mapping adjudication changed"
    );

    let mut skips: Vec<_> = report
        .cases
        .iter()
        .filter(|case| case.status == sf_conformance::Status::Skipped)
        .map(|case| case.id.as_str())
        .collect();
    skips.sort_unstable();
    assert_eq!(skips, EXPECTED_SKIPS, "DuckDB skip allow-list changed");
}
