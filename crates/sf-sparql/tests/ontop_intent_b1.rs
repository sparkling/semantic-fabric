//! Ontop-parity INTENT file — BATCH 1 (ADR-0021/0022 test-port program).
//!
//! Companion to `ontop_port_b1.rs`. Adds RED-SPEC tests (`#[ignore]`) for
//! NEEDS_IMPL scenarios and GREEN tests that the port file did not yet cover,
//! for the batch-1 assigned classes (sorted indices [5,10)):
//!
//!   5. executor/RedundantJoinFKTest        (9  tests — ALL GREEN in port; no new intent)
//!   6. executor/RedundantSelfJoinTest      (36 tests — 4 new here)
//!   7. executor/SubstitutionPropagationTest — ALL BOUNDARY (see below)
//!   8. optimizer/AggregationSplitterTest   — ALL BOUNDARY (see below)
//!   9. optimizer/BindingLiftTest           — ALL BOUNDARY (see below)
//!
//! # BOUNDARY CLASSES (no cascade-level tests possible)
//!
//! **SubstitutionPropagationTest (30 tests):** ALL BOUNDARY.
//! Ontop propagates substitutions through an IQ-tree that has explicit
//! `ConstructionNode`/`UnionNode`/`IntensionalDataNode` layers. sf resolves all
//! variable substitutions at ontology-unfolding time into `Branch::bindings`
//! (a flat map). There is no "propagate substitution upward through a
//! DistinctNode" operation in the cascade — the cascade is a purely relational
//! optimizer over base-table scans.
//!
//! **AggregationSplitterTest (15 tests):** ALL BOUNDARY.
//! This class splits aggregation expressions across sub-queries in Ontop's IQ
//! tree (GROUP BY / aggregation rewrite). sf's cascade operates on the
//! base-table-scan layer only; it emits a flat SQL query with no aggregation IR.
//! GROUP BY / COUNT / SUM rewriting belongs in a future IR layer above the
//! cascade (planned ADR-0020 "outstanding SOTA optimisations").
//!
//! **BindingLiftTest (37 tests):** ALL BOUNDARY.
//! Lifts `BIND`-equivalent substitutions upward through
//! `DistinctNode`/`SliceNode`/`UnionNode` parents in Ontop's IQ tree. sf's
//! flat `Branch` model has no such parent nodes to lift through; all variable
//! bindings are already resolved at the `Branch::bindings` level before the
//! cascade runs.
//!
//! # RedundantSelfJoinTest — NEEDS_IMPL coverage
//!
//! Pass 2 (`self_join_elimination`) merges two scans of the same table when
//! they share a NON-NULL single-column unique key equality. The following
//! scenarios require capabilities sf does not yet have:
//!
//!  1. Composite-key self-join elimination (`testSelfJoinElimination3`).
//!  2. Constant-contradiction detection after PK merge (`testNonUnification1`).
//!  3. Cross-column unsatisfiability detection (`testUnsatisfiedJoiningCondition`).
//!
//! Two additional scenarios that ARE already supported:
//!  4. Self-join on a secondary (non-PK) UNIQUE column — GREEN.
//!  5. Binding propagation from dropped scan to kept scan — GREEN.

use sf_core::ir::LogicalSource;
use sf_core::ir::{TermMap, TermSpec};
use sf_sparql::cascade::{run, CascadeCtx};
use sf_sparql::iq::{Branch, CmpOp, ColRef, Scan, SqlCond, TermDef};
use sf_sql::{Column, TableSchema};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
    }
}

fn binding_alias(b: &Branch, var: &str) -> usize {
    match b.bindings.get(var) {
        Some(TermDef::Derived { alias, .. }) => *alias,
        other => panic!("expected derived binding for {var}, got {other:?}"),
    }
}

fn has_cmp_eq(b: &Branch, col: (usize, &str), val: &str) -> bool {
    b.where_conds.iter().any(|cond| {
        matches!(cond, SqlCond::Cmp(c, CmpOp::Eq, v)
            if c == &ColRef::new(col.0, col.1) && v == val)
    })
}

// ---------------------------------------------------------------------------
// Schema — mirrors the Ontop RSJTest fixtures
// ---------------------------------------------------------------------------

/// RedundantSelfJoinTest tables 1–3 and 6.
///
/// table1: PK = col1 (single-column)
/// table2: PK = col2 (single-column, on the second column)
/// table3: PK = (col1, col2) composite
/// table6: PK = col1 AND UNIQUE = col3 (two atomic unique constraints)
fn rsj_schema() -> Vec<TableSchema> {
    let col = |n: usize| Column::new(format!("col{n}"), "integer", true);

    let mut t1 = TableSchema::new("table1");
    t1.primary_key = vec!["col1".into()];
    t1.columns = (1..=3).map(col).collect();

    let mut t2 = TableSchema::new("table2");
    t2.primary_key = vec!["col2".into()];
    t2.columns = (1..=3).map(col).collect();

    let mut t3 = TableSchema::new("table3");
    t3.primary_key = vec!["col1".into(), "col2".into()];
    t3.columns = (1..=3).map(col).collect();

    let mut t6 = TableSchema::new("table6");
    t6.primary_key = vec!["col1".into()];
    t6.unique = vec![vec!["col3".into()]];
    t6.columns = (1..=3).map(col).collect();

    vec![t1, t2, t3, t6]
}

// ===========================================================================
// NEEDS_IMPL (RED-SPEC, #[ignore])
// ===========================================================================

/// **NEEDS_IMPL** — `RSJTest.testSelfJoinElimination3`.
///
/// `table3` has a **composite** PK `(col1, col2)`. Two scans are joined on
/// BOTH `col1` AND `col2` — a full composite-key match implies the same row,
/// so the join should collapse to one scan.
///
/// sf's `find_self_join` (pass 2) only fires when the joining column is itself
/// a **single-column** unique key (`is_unique_key(col)` returns false for a
/// column that is only part of a multi-column PK). Full composite-key
/// self-join elimination requires collecting all join equalities and checking
/// them together against a composite key.
#[test]
#[ignore = "NEEDS_IMPL: composite-key self-join elimination not implemented \
            — sf pass 2 only handles single-column PKs \
            — RedundantSelfJoinTest.testSelfJoinElimination3"]
fn composite_pk_self_join_elim() {
    let mut b = branch_with(
        vec![scan(0, "table3"), scan(1, "table3")],
        vec![
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(1, "col1")), // col1 part of PK
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col2")), // col2 part of PK
            SqlCond::Cmp(ColRef::new(1, "col3"), CmpOp::Eq, "2".into()),
        ],
    );
    b.bindings.insert("Y".into(), col_binding(0, "col2"));
    b.bindings.insert("Z".into(), col_binding(1, "col3")); // reads the to-be-dropped scan

    let out = run(vec![b], &rsj_schema(), &CascadeCtx::default());
    assert_eq!(out.len(), 1);
    let b = &out[0];
    assert_eq!(
        b.core.len(),
        1,
        "composite PK (col1,col2) fully matched on both columns → must merge to one scan"
    );
    assert_eq!(
        binding_alias(b, "Z"),
        0,
        "?Z must rebind onto the kept scan after composite-key merge"
    );
    assert!(
        has_cmp_eq(b, (0, "col3"), "2"),
        "constant constraint on col3 must survive on the kept scan: {:?}",
        b.where_conds
    );
}

/// **NEEDS_IMPL** — `RSJTest.testNonUnification1` (simplified to 2-table form).
///
/// `table1` PK = `col1`. Scan0 pins `col1 = 1`; scan1 pins `col1 = 2`. A
/// ColEq on `col1` (the PK) causes sf to merge the scans, after which both
/// `Cmp(0.col1, 1)` and `Cmp(0.col1, 2)` survive — a plain-constant
/// contradiction that sf does not simplify to `FALSE` / empty result.
///
/// Ontop's optimizer detects the constant clash on the PK during unification
/// and emits an empty node immediately.
#[test]
#[ignore = "NEEDS_IMPL: plain-constant contradiction on PK not detected — \
            sf merges on col1 (PK) and retains contradictory Cmp conditions \
            rather than producing an empty result \
            — RedundantSelfJoinTest.testNonUnification1"]
fn self_join_nonunification_contradictory_constants() {
    let mut b = branch_with(
        vec![scan(0, "table1"), scan(1, "table1")],
        vec![
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(1, "col1")), // PK equality licenses merge
            SqlCond::Cmp(ColRef::new(0, "col1"), CmpOp::Eq, "1".into()),    // scan0 pins col1 = 1
            SqlCond::Cmp(ColRef::new(1, "col1"), CmpOp::Eq, "2".into()), // scan1 pins col1 = 2 — CONTRADICTION
        ],
    );
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("N".into(), col_binding(0, "col2"));
    b.bindings.insert("O".into(), col_binding(1, "col3"));

    let out = run(vec![b], &rsj_schema(), &CascadeCtx::default());
    // Ontop: constant clash on the PK (1 ≠ 2) → empty.
    // sf: merges on col1, leaves Cmp(0.col1,1) AND Cmp(0.col1,2) as residual
    //     conditions → one branch, one scan with impossible constraints.
    assert!(
        out.is_empty(),
        "contradictory PK constants (col1=1 vs col1=2) must yield an empty result: {out:?}"
    );
}

/// **NEEDS_IMPL** — `RSJTest.testUnsatisfiedJoiningCondition` (adapted).
///
/// `table2` PK = `col2`. Two scans joined on `col1(scan0) = col2(scan1)` —
/// a **cross-column** condition (different column names). sf's `find_self_join`
/// skips any ColEq where `a.column != c.column`, so no merge occurs and the
/// join is left in place with 2 surviving scans.
///
/// In Ontop, after propagating constants through the self-join, the joining
/// condition becomes unsatisfiable (the constant pinned to one column conflicts
/// with an OR-predicate on the result variable), yielding an empty node. sf
/// has no analogous unsatisfiability analysis for residual join conditions.
#[test]
#[ignore = "NEEDS_IMPL: cross-column join condition unsatisfiability not detected — \
            sf skips ColEq(col1, col2) because columns differ, leaving 2 scans; \
            Ontop detects the condition is unsatisfiable and returns empty \
            — RedundantSelfJoinTest.testUnsatisfiedJoiningCondition"]
fn unsatisfied_joining_condition_empty() {
    // table2 PK = col2. Join non-key col1 of scan0 to PK col2 of scan1.
    // sf cannot drive uniqueness-based merge on this (column names differ).
    let mut b = branch_with(
        vec![scan(0, "table2"), scan(1, "table2")],
        vec![
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(1, "col2")), // cross-column, non-key → PK
        ],
    );
    b.bindings.insert("M".into(), col_binding(0, "col2"));
    b.bindings.insert("N".into(), col_binding(0, "col1"));
    b.bindings.insert("O".into(), col_binding(1, "col3"));

    let out = run(vec![b], &rsj_schema(), &CascadeCtx::default());
    // Ontop: unsatisfied condition → empty.
    // sf: no merge (columns differ) → 1 branch, 2 scans.
    assert!(
        out.is_empty(),
        "unsatisfied cross-column join condition must yield an empty result: {out:?}"
    );
}

// ===========================================================================
// GREEN (no #[ignore]) — scenarios not yet in ontop_port_b1.rs
// ===========================================================================

/// **GREEN** — secondary-unique-key self-join (from `RSJTest.testDoubleUniqueConstraints1`).
///
/// `table6` has PK = `col1` AND a separate UNIQUE constraint on `col3`.
/// Two scans joined on `col3` (the **secondary** unique key, not the PK).
///
/// sf's `is_unique_key` returns true for any single-column UNIQUE constraint,
/// not only the primary key (`schema.rs:81`: checks `self.unique` as well as
/// `self.primary_key`). Therefore the self-join fires on `col3`, the scans
/// collapse to one, and the binding that read from the dropped scan migrates
/// onto the kept scan.
///
/// `ontop_port_b1.rs` already covers joining on `col1` (the PK in
/// `self_join_elim_double_unique_pk_collapse`); this test verifies the
/// secondary-key path.
#[test]
fn self_join_elim_secondary_unique_key_col3() {
    let mut b = branch_with(
        vec![scan(0, "table6"), scan(1, "table6")],
        vec![
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(1, "col3")), // UNIQUE col3, not the PK
        ],
    );
    b.bindings.insert("A".into(), col_binding(0, "col1"));
    b.bindings.insert("B".into(), col_binding(0, "col2"));
    b.bindings.insert("C".into(), col_binding(1, "col3")); // reads the to-be-dropped scan

    let out = run(vec![b], &rsj_schema(), &CascadeCtx::default());
    assert_eq!(out.len(), 1);
    let b = &out[0];
    assert_eq!(
        b.core.len(),
        1,
        "self-join on secondary UNIQUE key col3 must collapse to one table6 scan"
    );
    assert!(
        matches!(&b.core[0].source, LogicalSource::Table(t) if t == "table6"),
        "table6 is the surviving scan"
    );
    assert_eq!(
        binding_alias(b, "C"),
        0,
        "?C must rebind onto the kept scan (alias 0) after col3-based merge"
    );
}

/// **GREEN** — binding propagation after self-join (`RSJTest.testPropagation1`).
///
/// Table1 self-join on PK `col1` (scans 0 and 1) with a third join partner
/// (table2, scan 2). Binding `?O` reads from the **dropped** scan 1. After the
/// merge, `rewrite_alias` migrates every reference from alias 1 to alias 0,
/// including `?O`'s `TermDef`. Cross-table join conditions linking scan 0 to
/// scan 2 must survive unchanged.
#[test]
fn self_join_elim_with_propagated_binding() {
    // scan 0: table1 — binds M (col1), N (col2)
    // scan 1: table1 — binds O (col3); will be DROPPED after PK merge
    // scan 2: table2 — cross-table join partner
    let mut b = branch_with(
        vec![scan(0, "table1"), scan(1, "table1"), scan(2, "table2")],
        vec![
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(1, "col1")), // PK self-join (licenses merge)
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(2, "col1")), // table1 → table2 on col1
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(2, "col2")), // table1 → table2 on col2
        ],
    );
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("N".into(), col_binding(0, "col2"));
    b.bindings.insert("O".into(), col_binding(1, "col3")); // reads DROPPED scan1

    let out = run(vec![b], &rsj_schema(), &CascadeCtx::default());
    assert_eq!(out.len(), 1);
    let b = &out[0];

    // The table1 self-join merges → one table1 scan + one table2 scan remain.
    assert_eq!(
        b.core.len(),
        2,
        "table1 self-join collapses; table2 scan is retained: {:?}",
        b.core
    );

    // ?O's binding must migrate from the dropped scan (alias 1) to the kept scan (alias 0).
    assert_eq!(
        binding_alias(b, "O"),
        0,
        "?O must rebind onto the kept scan (alias 0) after self-join merge"
    );

    // Cross-table join conditions (table1 ↔ table2) must survive.
    let has_cross_cond = b.where_conds.iter().any(|c| {
        matches!(c, SqlCond::ColEq(a, x)
            if (a.alias == 0 && x.alias == 2) || (a.alias == 2 && x.alias == 0))
    });
    assert!(
        has_cross_cond,
        "cross-table join conditions (table1 ↔ table2) must survive the self-join merge: {:?}",
        b.where_conds
    );

    // The PK self-join equality itself must be gone (it licensed the merge).
    let pk_eq_survives = b.where_conds.iter().any(|c| {
        matches!(c, SqlCond::ColEq(a, x)
            if (a.alias == 0 && x.alias == 1) || (a.alias == 1 && x.alias == 0))
    });
    assert!(
        !pk_eq_survives,
        "the cross-scan PK equality that licensed the merge must be removed"
    );
}
