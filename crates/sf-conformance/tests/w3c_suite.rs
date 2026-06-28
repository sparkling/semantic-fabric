//! Drive the vendored W3C RDB2RDF suite (ADR-0005): run every case via the
//! CONSTRUCT dump, write the EARL reports, and gate on a non-regression baseline.
//! Honest failures (engine gaps) and 501-skips are reported, not hidden — the
//! green bar asserts the *baseline pass count* holds, so a regression turns it
//! red without demanding 100 % conformance.

use std::path::PathBuf;

use sf_conformance::{run_and_report, Kind};

fn cases_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/rdb2rdf")
}

#[test]
fn w3c_rdb2rdf_construct_conformance() {
    let root = cases_dir();
    let cases = root.join("cases");
    let report = run_and_report(&cases, &root).expect("suite runs");

    let r2rml_pass = report.passed(Some(Kind::R2rml));
    let r2rml_total = report.adjudicated(Some(Kind::R2rml));
    let dm_pass = report.passed(Some(Kind::DirectMapping));
    let dm_total = report.adjudicated(Some(Kind::DirectMapping));

    eprintln!("\n=== W3C RDB2RDF conformance (CONSTRUCT dump, SQLite) ===");
    eprintln!("R2RML          {}", report.split(Kind::R2rml));
    eprintln!("Direct Mapping {}", report.split(Kind::DirectMapping));
    eprintln!(
        "overall passed={} adjudicated={} skipped(501/fixture)={}",
        report.passed(None),
        report.adjudicated(None),
        report.skipped(None),
    );
    eprintln!("\n--- documented standards deviations (not failures; ADR-0015) ---");
    for d in report.expected_deviations() {
        eprintln!("  DEVIATION {d}");
    }
    eprintln!("\n--- UNEXPECTED failures (regressions) ---");
    for f in report.unexpected_failures() {
        eprintln!("  FAIL {f}");
    }
    eprintln!("\n--- skips (501-deferred / fixture) ---");
    for s in report.skips() {
        eprintln!("  SKIP {s}");
    }

    // EARL reports were written beside the suite.
    assert!(root.join("earl-semantic-fabric-r2rml.ttl").exists());
    assert!(root.join("earl-semantic-fabric-direct.ttl").exists());

    // Primary gate: NO unexpected failures. Tighter than a pass-count baseline — it
    // also catches a pass↔fail swap at constant count. The one documented deviation
    // (R2RMLTC0002f, ADR-0015) is excluded here but still earl:failed in the report.
    assert!(
        report.unexpected_failures().is_empty(),
        "unexpected conformance failure(s) — regression: {:?}",
        report.unexpected_failures()
    );

    // Floor baselines (raise as the engine improves; lowering them is a regression
    // to investigate, never a silent edit).
    assert!(
        r2rml_pass >= R2RML_BASELINE,
        "R2RML pass regressed: {r2rml_pass}/{r2rml_total} < baseline {R2RML_BASELINE}"
    );
    assert!(
        dm_pass >= DM_BASELINE,
        "Direct Mapping pass regressed: {dm_pass}/{dm_total} < baseline {DM_BASELINE}"
    );
}

// Measured non-regression floor (SQLite). Bumped only upward as the engine
// improves; a drop below these is a regression to fix, never a silent lowering. As
// of this wave: R2RML 62/63 adjudicated, Direct Mapping 19/19, 5 cases skipped
// (SQLite-incompatible DDL fixtures D021–D025). The 6 `rr:graphMap` named-graph
// cases (R2RMLTC0006a/0007b/0007e/0007f/0008a/0009b) PASS via the mapping-IR quad
// dump (ADR-0005). The 63rd R2RML case, R2RMLTC0002f, is a documented standards
// deviation (`sf_conformance::EXPECTED_DEVIATIONS`, ADR-0015 §Identifier
// resolution): per R2RML §5 (SQL:2008 comparison) the regular identifier {Name}
// does not match the delimited mixed-case column "Name", so a strict processor
// rejects the mapping — but the suite's own positive cases (R2RMLTC0002a,
// R2RMLTC0018a/D018) rely on lenient matching of that very pattern, and our
// virtualiser cannot recover delimited-vs-regular provenance from introspection
// (SQLite especially). We adopt lenient resolution; 0002f stays earl:failed but is
// excluded from the gate above as a documented deviation. Both 0002f and D018 are
// test:unreviewed.
const R2RML_BASELINE: usize = 62;
const DM_BASELINE: usize = 19;
