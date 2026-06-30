//! Ontop-parity oracle — BATCH 1 (ADR-0021/0022 test-port program).
//!
//! Public-API (integration-test) ports of Ontop 5.5.0 IQ-optimizer JUnit
//! scenarios for the classes assigned to batch 1 (sorted indices [5,10)):
//!
//!   5. executor/RedundantJoinFKTest        (FK/PK join elim — cascade pass 4)
//!   6. executor/RedundantSelfJoinTest      (self-join elim    — cascade pass 2)
//!   7. executor/SubstitutionPropagationTest
//!   8. optimizer/AggregationSplitterTest
//!   9. optimizer/BindingLiftTest
//!
//! Only the scenarios that sf's `cascade::run` already performs (pass 2 single
//! column self-join elimination, pass 4 single/composite FK/PK elimination) are
//! converted here, asserted against the SAME optimized shape Ontop's oracle
//! expects (scan collapses, surviving table, binding rebinds onto the kept scan,
//! surviving residual constraints). Scenarios requiring optimizations sf does not
//! have (composite-key self-join elim, substitution propagation, aggregation
//! splitting, binding/substitution lift, provenance LEFT-JOIN rewrites) are
//! classified in the batch report — NOT faked green here.
//!
//! These are scenario/intent ports against sf's `iq.rs` IR, not transliterations
//! of Ontop's Java IQ API. Driven through the crate's PUBLIC API only
//! (`sf_sparql::cascade::run`), so they double as a public-surface smoke test.
//!
//! NOTE: `RedundantJoinFKTest` is also ported crate-internally in
//! `src/cascade/ws_fk.rs`; the 3 FK ports below re-express representative
//! scenarios at the public-API level (a different test surface) — not a
//! substitute for that file.

use sf_core::ir::LogicalSource;
use sf_core::ir::{TermMap, TermSpec};
use sf_sparql::cascade::{run, CascadeCtx};
use sf_sparql::iq::{Branch, CmpOp, ColRef, Scan, SqlCond, TermDef};
use sf_sql::{Column, ForeignKey, TableSchema};
use std::collections::BTreeMap;

// --- helpers (mirror src/cascade/ws_g.rs PORT PATTERN) ---------------------

fn scan(alias: usize, table: &str) -> Scan {
    Scan {
        alias,
        source: LogicalSource::Table(table.to_owned()),
    }
}

/// A plain column term map at `alias` — the binding environment entry for a
/// projected variable that reads `col` of that scan.
fn col_binding(alias: usize, col: &str) -> TermDef {
    TermDef::Derived {
        term_map: TermMap::Column(col.into(), TermSpec::plain_literal()),
        alias,
    }
}

fn branch_with(core: Vec<Scan>, where_conds: Vec<SqlCond>) -> Branch {
    Branch {
        core,
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds,
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    }
}

/// The alias a derived binding reads from (panics if not a plain derived def).
fn binding_alias(b: &Branch, var: &str) -> usize {
    match b.bindings.get(var) {
        Some(TermDef::Derived { alias, .. }) => *alias,
        other => panic!("expected derived binding for {var}, got {other:?}"),
    }
}

fn has_col_eq(b: &Branch, a: (usize, &str), c: (usize, &str)) -> bool {
    b.where_conds.iter().any(|cond| match cond {
        SqlCond::ColEq(x, y) => {
            (x == &ColRef::new(a.0, a.1) && y == &ColRef::new(c.0, c.1))
                || (x == &ColRef::new(c.0, c.1) && y == &ColRef::new(a.0, a.1))
        }
        _ => false,
    })
}

fn has_cmp_eq(b: &Branch, col: (usize, &str), val: &str) -> bool {
    b.where_conds.iter().any(|cond| {
        matches!(cond, SqlCond::Cmp(c, CmpOp::Eq, v) if c == &ColRef::new(col.0, col.1) && v == val)
    })
}

// === Schemas (mirror the Ontop test fixtures) ==============================

/// RedundantSelfJoinTest TABLE1: PK = col1 (single column), col2/col3 plain.
/// All columns NOT NULL (Ontop `createDatabaseRelation(..., false)` ⇒ not nullable).
fn self_join_schema() -> Vec<TableSchema> {
    let mk = |name: &str, pk: &str, ncols: usize| {
        let mut t = TableSchema::new(name);
        t.primary_key = vec![pk.into()];
        t.columns = (1..=ncols)
            .map(|i| Column::new(format!("col{i}"), "integer", true))
            .collect();
        t
    };
    let t1 = mk("table1", "col1", 3);
    let t2 = mk("table2", "col2", 3); // PK on the SECOND column
    let mut t3 = TableSchema::new("table3"); // composite PK (col1,col2)
    t3.primary_key = vec!["col1".into(), "col2".into()];
    t3.columns = vec![
        Column::new("col1", "integer", true),
        Column::new("col2", "integer", true),
        Column::new("col3", "integer", true),
    ];
    // TABLE6: two atomic unique keys — PK col1 AND a UNIQUE on col3.
    let mut t6 = TableSchema::new("table6");
    t6.primary_key = vec!["col1".into()];
    t6.unique = vec![vec!["col3".into()]];
    t6.columns = vec![
        Column::new("col1", "integer", true),
        Column::new("col2", "integer", true),
        Column::new("col3", "integer", true),
    ];
    vec![t1, t2, t3, t6]
}

/// RedundantJoinFKTest single-column FK fixture: TABLE1 PK=col1; TABLE2.col2 → TABLE1.col1.
fn single_fk_schema() -> Vec<TableSchema> {
    let mut t1 = TableSchema::new("TABLE1");
    t1.primary_key = vec!["col1".into()];
    t1.columns = vec![
        Column::new("col1", "integer", true),
        Column::new("col2", "integer", true),
    ];
    let mut t2 = TableSchema::new("TABLE2");
    t2.columns = vec![
        Column::new("col1", "integer", true),
        Column::new("col2", "integer", true),
    ];
    t2.foreign_keys = vec![ForeignKey {
        columns: vec!["col2".into()],
        parent_table: "TABLE1".into(),
        parent_columns: vec!["col1".into()],
    }];
    vec![t1, t2]
}

/// RedundantJoinFKTest composite FK fixture: TABLE3 PK=(col1,col2);
/// TABLE4.(col2,col3) → TABLE3.(col1,col2).
fn composite_fk_schema() -> Vec<TableSchema> {
    let mut t3 = TableSchema::new("TABLE3");
    t3.primary_key = vec!["col1".into(), "col2".into()];
    t3.columns = vec![
        Column::new("col1", "integer", true),
        Column::new("col2", "integer", true),
        Column::new("col3", "integer", true),
    ];
    let mut t4 = TableSchema::new("TABLE4");
    t4.columns = vec![
        Column::new("col1", "integer", true),
        Column::new("col2", "integer", true),
        Column::new("col3", "integer", true),
    ];
    t4.foreign_keys = vec![ForeignKey {
        columns: vec!["col2".into(), "col3".into()],
        parent_table: "TABLE3".into(),
        parent_columns: vec!["col1".into(), "col2".into()],
    }];
    vec![t3, t4]
}

// === RedundantSelfJoinTest — cascade pass 2 (self-join elimination) =========

/// **GREEN** — Ontop `RedundantSelfJoinTest.testSelfJoinElimination2`.
/// `table2(X,Y,Z) JOIN table2(X,Y,2)` on the PK `col2` (=Y) collapses to one
/// scan; `col3` is pinned to the constant 2. Ontop's expected:
/// `table2(col2=Y, col3=2)` projecting Y. sf merges on the NON-NULL PK col2.
#[test]
fn self_join_elim2_pk_collapse_with_constant() {
    let mut b = branch_with(
        vec![scan(0, "table2"), scan(1, "table2")],
        vec![
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(1, "col1")), // X shared (non-key)
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col2")), // Y shared = PK col2
            SqlCond::Cmp(ColRef::new(1, "col3"), CmpOp::Eq, "2".into()),    // col3 = TWO
        ],
    );
    b.bindings.insert("Y".into(), col_binding(0, "col2")); // the only projected var

    let out = run(vec![b], &self_join_schema(), &CascadeCtx::default());
    assert_eq!(out.len(), 1);
    let b = &out[0];
    assert_eq!(
        b.core.len(),
        1,
        "self-join on PK col2 collapses to one table2 scan"
    );
    assert!(
        matches!(&b.core[0].source, LogicalSource::Table(t) if t == "table2"),
        "table2 is the surviving scan"
    );
    assert_eq!(binding_alias(b, "Y"), 0, "?Y rebinds onto the kept scan");
    // The constant pin on col3 survives (rewritten onto the kept alias 0).
    assert!(
        has_cmp_eq(b, (0, "col3"), "2"),
        "col3 = 2 constant constraint must survive on the kept scan: {:?}",
        b.where_conds
    );
}

/// **GREEN** — Ontop `RedundantSelfJoinTest.testDoubleUniqueConstraints1`.
/// `table6(M,N,O) JOIN table6(M,N1,2)` shares the PK `col1` (=M) ⇒ collapse;
/// `col3` pinned to 2 (Ontop lifts O→2 as a substitution; sf keeps it as a
/// residual `col3 = 2` filter — `=_bag`-equivalent since the row is the same).
#[test]
fn self_join_elim_double_unique_pk_collapse() {
    let mut b = branch_with(
        vec![scan(0, "table6"), scan(1, "table6")],
        vec![
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(1, "col1")), // M = PK col1
            SqlCond::Cmp(ColRef::new(1, "col3"), CmpOp::Eq, "2".into()),    // col3 = TWO
        ],
    );
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("N".into(), col_binding(0, "col2"));
    b.bindings.insert("O".into(), col_binding(1, "col3")); // reads the dropped scan → must rebind

    let out = run(vec![b], &self_join_schema(), &CascadeCtx::default());
    assert_eq!(out.len(), 1);
    let b = &out[0];
    assert_eq!(
        b.core.len(),
        1,
        "self-join on PK col1 collapses to one table6 scan"
    );
    assert_eq!(
        binding_alias(b, "O"),
        0,
        "?O rebinds onto the kept scan (alias 0)"
    );
    assert!(
        has_cmp_eq(b, (0, "col3"), "2"),
        "col3 = 2 constraint survives on the kept scan: {:?}",
        b.where_conds
    );
}

/// **GREEN** — Ontop `RedundantSelfJoinTest.testJoiningConditionTest`.
/// `table1(M,N,O1) JOIN table1(M,N1,O)` on the PK `col1` (=M), with an extra
/// joining condition `M = N` (= `col1 = col2`). The self-join collapses; the
/// same-scan `col1 = col2` guard is NOT a self-join and must survive (Ontop
/// lifts it as the substitution N→M; `=_bag`-equivalent).
#[test]
fn self_join_elim_with_surviving_join_guard() {
    let mut b = branch_with(
        vec![scan(0, "table1"), scan(1, "table1")],
        vec![
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(1, "col1")), // M = PK col1
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(0, "col2")), // M = N joining cond
        ],
    );
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("N".into(), col_binding(0, "col2"));
    b.bindings.insert("O".into(), col_binding(1, "col3")); // on the dropped scan

    let out = run(vec![b], &self_join_schema(), &CascadeCtx::default());
    assert_eq!(out.len(), 1);
    let b = &out[0];
    assert_eq!(
        b.core.len(),
        1,
        "self-join on PK col1 collapses to one table1 scan"
    );
    assert_eq!(binding_alias(b, "O"), 0, "?O rebinds onto the kept scan");
    assert!(
        has_col_eq(b, (0, "col1"), (0, "col2")),
        "the same-scan M=N guard (col1=col2) must survive the merge: {:?}",
        b.where_conds
    );
    // The PK self-join equality itself is gone (it licensed the merge).
    assert!(
        !has_col_eq(b, (0, "col1"), (1, "col1")),
        "the cross-scan PK equality that licensed the merge is removed"
    );
}

/// **GREEN (no-op)** — Ontop `RedundantSelfJoinTest.testNonEliminationTable1`.
/// `table1(X,Y,Z) JOIN table1(Z,Y,2)` shares only `col2` (=Y), which is NOT the
/// PK (col1 is). No PK match ⇒ NO self-join elimination. Both scans survive
/// (parity with Ontop, which leaves the join in place).
#[test]
fn self_join_non_elim_shared_non_key_column() {
    let mut b = branch_with(
        vec![scan(0, "table1"), scan(1, "table1")],
        vec![
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col2")), // Y shared (col2, non-key)
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(1, "col1")), // Z shared across positions
            SqlCond::Cmp(ColRef::new(1, "col3"), CmpOp::Eq, "2".into()),
        ],
    );
    b.bindings.insert("Y".into(), col_binding(0, "col2"));

    let out = run(vec![b], &self_join_schema(), &CascadeCtx::default());
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].core.len(),
        2,
        "no PK column is shared ⇒ the self-join MUST be preserved (no over-eager merge)"
    );
}

/// **GREEN (no-op)** — Ontop `RedundantSelfJoinTest.testNonEliminationTable3`.
/// `table3` has a COMPOSITE PK (col1,col2); the two scans share only `col1`
/// (=X). A partial composite key is not unique ⇒ NO elimination. (sf only does
/// single-column self-join elimination anyway — this pins the safe boundary.)
#[test]
fn self_join_non_elim_partial_composite_key() {
    let mut b = branch_with(
        vec![scan(0, "table3"), scan(1, "table3")],
        vec![
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(1, "col1")), // X shared = only col1 of (col1,col2)
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(0, "col3")), // Z self-guard on scan0
            SqlCond::Cmp(ColRef::new(1, "col3"), CmpOp::Eq, "2".into()),
        ],
    );
    b.bindings.insert("Y".into(), col_binding(1, "col2"));

    let out = run(vec![b], &self_join_schema(), &CascadeCtx::default());
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].core.len(),
        2,
        "only col1 of the composite PK (col1,col2) is shared ⇒ NOT a unique match ⇒ preserved"
    );
}

// === RedundantJoinFKTest — cascade pass 4 (FK/PK join elimination) ==========

/// **GREEN** — Ontop `RedundantJoinFKTest.testForeignKeyOptimization`.
/// `TABLE1(A,B) JOIN TABLE2(D,A)` on `TABLE2.col2 = TABLE1.col1` (a NOT-NULL FK
/// to the PK). TABLE1 is reached only for its PK ⇒ eliminated; A rebinds to
/// TABLE2.col2.
#[test]
fn fk_pk_elim_single_column() {
    let mut b = branch_with(
        vec![scan(0, "TABLE1"), scan(1, "TABLE2")],
        vec![SqlCond::ColEq(
            ColRef::new(0, "col1"),
            ColRef::new(1, "col2"),
        )],
    );
    b.bindings.insert("A".into(), col_binding(0, "col1"));

    let proj = vec!["A".to_string()];
    let ctx = CascadeCtx {
        distinct: false,
        project: Some(&proj),
    };
    let out = run(vec![b], &single_fk_schema(), &ctx);
    assert_eq!(out.len(), 1);
    let b = &out[0];
    assert_eq!(
        b.core.len(),
        1,
        "TABLE1 (parent, reached only for its PK) is eliminated"
    );
    assert!(
        matches!(&b.core[0].source, LogicalSource::Table(t) if t == "TABLE2"),
        "TABLE2 (child, carrying the FK) is the surviving scan"
    );
    assert!(b.where_conds.is_empty(), "the FK join equality is dropped");
    assert_eq!(
        binding_alias(b, "A"),
        1,
        "?A rebinds onto the child FK column (TABLE2.col2)"
    );
}

/// **GREEN (no-op)** — Ontop `RedundantJoinFKTest.testForeignKeyNonOptimization`.
/// Same join, but `B` (= TABLE1.col2, a NON-PK column) is also projected ⇒ the
/// parent is referenced beyond its PK ⇒ NO elimination.
#[test]
fn fk_pk_non_elim_projected_non_pk_column() {
    let mut b = branch_with(
        vec![scan(0, "TABLE1"), scan(1, "TABLE2")],
        vec![SqlCond::ColEq(
            ColRef::new(0, "col1"),
            ColRef::new(1, "col2"),
        )],
    );
    b.bindings.insert("A".into(), col_binding(0, "col1"));
    b.bindings.insert("B".into(), col_binding(0, "col2")); // non-PK column of the parent

    let proj = vec!["A".to_string(), "B".to_string()];
    let ctx = CascadeCtx {
        distinct: false,
        project: Some(&proj),
    };
    let out = run(vec![b], &single_fk_schema(), &ctx);
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].core.len(),
        2,
        "TABLE1 must be preserved — col2 (non-PK) is projected, so the parent is needed beyond its PK"
    );
}

/// **GREEN** — Ontop `RedundantJoinFKTest.testForeignKeyOptimization2`.
/// Composite FK: `TABLE4.(col2,col3) → TABLE3.(col1,col2)`; TABLE3 reached only
/// via its composite PK ⇒ eliminated; A rebinds to TABLE4.col2.
#[test]
fn fk_pk_elim_composite_key() {
    let mut b = branch_with(
        vec![scan(0, "TABLE3"), scan(1, "TABLE4")],
        vec![
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(1, "col2")), // T3.col1 = T4.col2
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col3")), // T3.col2 = T4.col3
        ],
    );
    b.bindings.insert("A".into(), col_binding(0, "col1"));

    let proj = vec!["A".to_string()];
    let ctx = CascadeCtx {
        distinct: false,
        project: Some(&proj),
    };
    let out = run(vec![b], &composite_fk_schema(), &ctx);
    assert_eq!(out.len(), 1);
    let b = &out[0];
    assert_eq!(
        b.core.len(),
        1,
        "TABLE3 eliminated via the composite FK/PK join"
    );
    assert!(
        matches!(&b.core[0].source, LogicalSource::Table(t) if t == "TABLE4"),
        "TABLE4 is the surviving scan"
    );
    assert_eq!(binding_alias(b, "A"), 1, "?A rebinds onto TABLE4.col2");
}
