//! Drive the vendored W3C RDB2RDF suite against a **live PostgreSQL** server
//! (ADR-0005 / ADR-0010 / ADR-0015): the PG execution path (forked fixtures,
//! `query_raw` cursor) end to end. The cases SQLite cannot load — the D021–D025
//! table-level-constraint DDL fixtures — run and adjudicate here, and the
//! CHAR(n)-padding cases pass against PostgreSQL's native space-padding.
//!
//! **Gating:** with no server reachable the suite SKIPS (never fails), so CI
//! stays green offline. Set `SF_PG_URL` (host/user params, no dbname) to target
//! a server; the local default is trust auth on `localhost:5432`.

use std::path::PathBuf;

use sf_conformance::{pg, Kind};

fn cases_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/rdb2rdf/cases")
}

// Measured non-regression baselines (live PostgreSQL). Bumped only upward; a drop
// is a regression to fix, never a silent lowering. As of this wave: R2RML 57/58
// adjudicated, Direct Mapping 23/23, 6 skips (the D016 SQL-datatypes fixtures
// using `VARBINARY`/`X'…'`, which PostgreSQL cannot load — a per-DBMS fork would
// be needed, ADR-0015). The 8 cases SQLite cannot clear now pass on PostgreSQL:
// the 3 CHAR(n) cases (R2RMLTC0018a, DirectGraphTC0017, DirectGraphTC0018) via
// PostgreSQL's native space-padding, and the 5 table-level-constraint DDL
// scenarios (D021–D025, both R2RML and Direct Mapping) which load fine on
// PostgreSQL. SQL identifier case-folding (resolve each column reference
// against the source's introspected columns — exact then ASCII-case-insensitive,
// and a translate-time fallback for SubPlan-embedded derived tables, which carry
// no live catalog — `emit::synthetic_subplan_catalog`) cleared 8 regular-
// identifier shortfalls (R2RMLTC0002d/0003b/0009d/0011a/0014b/c/d and the rowid→
// `ctid` Direct Mapping translation). R2RMLTC0002f stays an honest fail — but
// NOT because rejection is impossible here (corrected 2026-07-20): on PG the
// unquoted `{Name}` case-folds to `name` and genuinely does not match the
// delimited `"Name"`, and Ontop v4.1.0 passes 0002f alongside the
// structurally-identical positives R2RMLTC0002a/0018a on PG (rml.io R2RML
// implementation report). Our resolver's case-insensitive fallback erases the
// distinction; a delimited-aware PG resolver is an OPEN PARITY ITEM (README §9,
// ADR-0015). Until it exists, 0002f is a documented deviation, not a gate red. R2RMLTC0012e — its D2-pooled
// blank-node subject template positionally UNIONs `IOUs.amount` (`FLOAT`)
// against `Lives.city` (`VARCHAR`) at the same column slot, a hard PostgreSQL
// `UNION` type-resolver error if pooled as-is and unsafe to paper over with a
// `CAST` (live-verified lexical drift: PG's own `float8`-to-text formatting uses
// scientific notation outside a plain-decimal magnitude range, where
// reconstruction's native `f64::to_string()` reading never does) — is RESTORED
// (Run 5 C0e): `cascade::group_has_unsafe_float_slot_mismatch` still refuses the
// `UNION`, but the group is ALSO a STANDALONE D2 group (`cascade::group_
// eligible_for_term_dedup`), so `unfold::pool_pattern_relation` / `iq::resolve`
// route it through a cross-branch SHARED Rust-side seen-set instead of SQL
// pooling — no `UNION` ever emitted, so the type-resolver wall never applies.
const R2RML_PG_BASELINE: usize = 57;
const DM_PG_BASELINE: usize = 23;

#[test]
fn w3c_rdb2rdf_postgres_conformance() {
    let Some(report) = pg::run(&cases_dir()).expect("pg suite runs or skips") else {
        eprintln!(
            "\nSKIP: no PostgreSQL server reachable — set SF_PG_URL to run the PG conformance suite."
        );
        return;
    };

    let r2rml_pass = report.passed(Some(Kind::R2rml));
    let dm_pass = report.passed(Some(Kind::DirectMapping));

    eprintln!("\n=== W3C RDB2RDF conformance (CONSTRUCT dump, live PostgreSQL) ===");
    eprintln!("R2RML          {}", report.split(Kind::R2rml));
    eprintln!("Direct Mapping {}", report.split(Kind::DirectMapping));
    eprintln!(
        "overall passed={} adjudicated={} skipped={}",
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
    eprintln!("\n--- skips (501-deferred / unloadable-on-PG fixture) ---");
    for s in report.skips() {
        eprintln!("  SKIP {s}");
    }

    // Primary gate: no UNEXPECTED failures (R2RMLTC0002f is the one documented
    // deviation, ADR-0015 — excluded here, still earl:failed). Tighter than the
    // pass-count floor below.
    assert!(
        report.unexpected_failures().is_empty(),
        "unexpected PG conformance failure(s) — regression: {:?}",
        report.unexpected_failures()
    );

    assert!(
        r2rml_pass >= R2RML_PG_BASELINE,
        "R2RML(PG) pass regressed: {r2rml_pass} < baseline {R2RML_PG_BASELINE}"
    );
    assert!(
        dm_pass >= DM_PG_BASELINE,
        "Direct Mapping(PG) pass regressed: {dm_pass} < baseline {DM_PG_BASELINE}"
    );
}
