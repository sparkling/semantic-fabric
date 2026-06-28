//! WS-G — Ontop-parity oracle (ADR-0022).
//!
//! Ports of Ontop 5.5.0 IQ-optimizer JUnit scenarios, re-expressed against sf's
//! `iq.rs` + cascade (a scenario/intent port, not a transliteration of Ontop's
//! Java IQ API). Two kinds of test live here:
//!
//! * **GREEN** — behaviour sf already has; the port asserts parity with Ontop's
//!   oracle and runs in the normal suite.
//! * **`#[ignore]` (RED) — the WS-A spec.** The optimization is not yet
//!   implemented, so the test asserts the *desired* optimized IQ and currently
//!   fails. It is `#[ignore]`d to keep `cargo test` green; remove the attribute as
//!   WS-A lands. Run the spec set with `cargo test -p sf-sparql -- --ignored`.
//!
//! Scenario provenance is cited per test against the Ontop source class at
//! `~/source/ontop/core/optimization/src/test/java/.../iq/{executor,optimizer}/`.
#![cfg(test)]

use super::*;
use crate::iq::{OptJoin, Scan};
use sf_core::ir::{Segment, Template, TermMap, TermSpec};
use sf_sql::Column;
use std::collections::BTreeMap;

fn scan(alias: usize, table: &str) -> Scan {
    Scan {
        alias,
        source: LogicalSource::Table(table.to_owned()),
    }
}

fn col_binding(alias: usize, col: &str) -> TermDef {
    TermDef::Derived {
        term_map: TermMap::Column(col.into(), TermSpec::plain_literal()),
        alias,
    }
}

fn iri_template_binding(alias: usize, prefix: &str, col: &str) -> TermDef {
    let segs = vec![Segment::Literal(prefix.into()), Segment::Column(col.into())];
    TermDef::Derived {
        term_map: TermMap::Template(Template::from_segments(segs).unwrap(), TermSpec::iri()),
        alias,
    }
}

/// `trips(trip_id PK NOT NULL, trip_headsign nullable)` — the GTFS table the Q5
/// pattern reads (gtfs.r2rml.ttl). `trip_headsign` is a nullable column of the row
/// keyed by the PK `trip_id`.
fn trips_schema() -> Vec<TableSchema> {
    let mut trips = TableSchema::new("trips");
    trips.primary_key = vec!["trip_id".into()];
    trips.columns = vec![
        Column::new("trip_id", "text", true),        // PK ⇒ NOT NULL
        Column::new("trip_headsign", "text", false), // nullable
    ];
    vec![trips]
}

/// **GREEN** — Ontop `RedundantSelfJoinTest.testSelfJoinElimination1`: two scans of
/// the same table joined on its PK collapse to one (equal key ⇒ same row). sf
/// supports this today (cascade pass 2); this is the inner-join parity baseline the
/// LEFT-join variant below must match.
#[test]
fn ontop_inner_self_join_elimination_on_pk() {
    let mut b = Branch {
        core: vec![scan(0, "trips"), scan(1, "trips")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![SqlCond::ColEq(
            ColRef::new(0, "trip_id"),
            ColRef::new(1, "trip_id"),
        )],
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
    };
    b.bindings
        .insert("hs".into(), col_binding(1, "trip_headsign"));

    let out = run(vec![b], &trips_schema(), &CascadeCtx::default());
    assert_eq!(
        out[0].core.len(),
        1,
        "inner self-join on the PK collapses to one scan"
    );
    match out[0].bindings.get("hs").unwrap() {
        TermDef::Derived { alias, .. } => {
            assert_eq!(*alias, 0, "binding rewritten onto the kept scan")
        }
        other => panic!("expected a derived binding, got {other:?}"),
    }
    assert!(
        out[0].where_conds.is_empty(),
        "the trivial trip_id = trip_id equality dropped"
    );
}

/// **GREEN (WS-A landed, ADR-0022).** Ontop `LeftJoinOptimizationTest.testLeftJoinElimination1`
/// / `RedundantSelfJoinTest.testOptimizationOnRightPartOfLJ1`. The Q5 query
/// `?t a gtfs:Trip . OPTIONAL { ?t gtfs:headsign ?hs }` maps both patterns to the
/// same `trips` table on the same NOT-NULL PK `trip_id`, so the OPTIONAL right side
/// reads the SAME row the core already has — `trip_headsign` is just a nullable
/// column of that row. The self-LEFT-join must collapse: `?hs` rebinds to the kept
/// scan and the `LEFT JOIN` (with its null-safe `ON`) disappears.
///
/// sf's `self_join_elimination` only inspects `where_conds`; WS-A added
/// `self_left_join_elimination` (cascade pass 2, left-join variant) which scans
/// `opts` and collapses this redundant self-LEFT-join.
#[test]
fn ontop_self_left_join_elimination_q5() {
    let mut b = Branch::single(scan(0, "trips"));
    b.bindings.insert(
        "t".into(),
        iri_template_binding(0, "http://ex/trip/", "trip_id"),
    );
    b.bindings
        .insert("hs".into(), col_binding(1, "trip_headsign"));
    // OPTIONAL { ?t gtfs:headsign ?hs } → LEFT JOIN trips t1 ON null-safe(t0.trip_id, t1.trip_id).
    b.opts.push(OptJoin {
        scan: scan(1, "trips"),
        on: vec![SqlCond::NullSafeEq(
            ColRef::new(0, "trip_id"),
            ColRef::new(1, "trip_id"),
        )],
        extra: Vec::new(),
    });

    let out = run(vec![b], &trips_schema(), &CascadeCtx::default());
    assert_eq!(out.len(), 1);
    let b = &out[0];
    assert!(
        b.opts.is_empty(),
        "self-LEFT-join on the trips PK must collapse (no surviving LEFT JOIN): {:?}",
        b.opts
    );
    assert_eq!(b.core.len(), 1, "single trips scan kept");
    match b.bindings.get("hs").unwrap() {
        TermDef::Derived { alias, .. } => {
            assert_eq!(
                *alias, 0,
                "?hs rebound onto the kept scan (reads trip_headsign of the same row)"
            )
        }
        other => panic!("expected ?hs as a derived binding on the kept scan, got {other:?}"),
    }
}

/// **Guard (safety boundary).** Ontop `LeftJoinOptimizationTest.testSelfJoinNullableUniqueConstraint`.
/// When the shared self-left-join determinant is a NULLABLE unique column (not a
/// true key), the `LEFT JOIN` must NOT collapse: the null-safe `ON` admits the
/// table's NULL-`code` rows differently on each side, so merging to a bare scan
/// would change multiplicities (=_bag break, ADR-0007) — mirroring the inner-join
/// guard `self_join_not_eliminated_on_nullable_unique_key`.
///
/// Currently passes *vacuously* (sf eliminates no self-left-join yet); it becomes
/// load-bearing the moment WS-A wires elimination — it pins the invariant the WS-A
/// pass must honour (refuse on a nullable determinant). Deliberately NOT `#[ignore]`d:
/// it must stay green before *and* after WS-A.
#[test]
fn ontop_self_left_join_not_eliminated_on_nullable_key() {
    let mut trips = TableSchema::new("trips");
    trips.unique = vec![vec!["code".into()]];
    trips.columns = vec![
        Column::new("code", "text", false), // NULLABLE unique — not a true key
        Column::new("trip_headsign", "text", false), // nullable
    ];
    let mut b = Branch::single(scan(0, "trips"));
    b.bindings
        .insert("hs".into(), col_binding(1, "trip_headsign"));
    b.opts.push(OptJoin {
        scan: scan(1, "trips"),
        on: vec![SqlCond::NullSafeEq(
            ColRef::new(0, "code"),
            ColRef::new(1, "code"),
        )],
        extra: Vec::new(),
    });

    let out = run(
        vec![b],
        std::slice::from_ref(&trips),
        &CascadeCtx::default(),
    );
    assert_eq!(
        out[0].opts.len(),
        1,
        "nullable unique determinant ⇒ self-LEFT-join MUST be preserved (=_bag safety)"
    );
}
