//! WS-ST — Ontop 5.5.0 `SelfJoinSameTermsTest` oracle (ADR-0022, WAVE 1).
//!
//! Ports 11 scenarios (2 @Ignore + 9 active) from
//! `ontop/core/optimization/src/test/java/.../iq/optimizer/SelfJoinSameTermsTest.java`
//! re-expressed against sf's `iq.rs` IR (not a Java transliteration). Active tests
//! exercise cascade pass 2c (`same_terms_elimination` in `sameterm.rs`).
//!
//! Schema: `T1_AR3` — a 3-column table with NO unique key (all columns nullable).
//! The optimization fires only under `DISTINCT`; its absence is tested in the
//! NonElimination cases.
#![cfg(test)]

use super::*;
use sf_sql::Column;
use std::collections::BTreeMap;

fn scan(alias: usize) -> crate::iq::Scan {
    crate::iq::Scan {
        alias,
        source: LogicalSource::Table("T1_AR3".to_owned()),
    }
}

fn col_binding(alias: usize, col: &str) -> TermDef {
    use sf_core::ir::{TermMap, TermSpec};
    TermDef::Derived {
        term_map: TermMap::Column(col.into(), TermSpec::plain_literal()),
        alias,
    }
}

/// T1_AR3 schema: 3 nullable string columns, no primary key, no unique constraint.
fn t1_schema() -> Vec<TableSchema> {
    let mut t = TableSchema::new("T1_AR3");
    t.columns = vec![
        Column::new("col1", "text", false),
        Column::new("col2", "text", false),
        Column::new("col3", "text", false),
    ];
    vec![t]
}

/// Helper: count `IS NOT NULL` conditions for a given column name.
fn count_nn_guards(b: &Branch, col: &str) -> usize {
    b.where_conds
        .iter()
        .filter(|c| matches!(c, SqlCond::IsNotNull(r) if r.column.as_ref() == col))
        .count()
}

// --- Elimination tests (pass 2c active) ------------------------------------

/// **GREEN** — Ontop `testSelfJoinElimination1`: two T1_AR3 scans sharing col2(=B)
/// and col3(=C), under DISTINCT, col1 not projected. Result: 1 scan, IS NOT NULL
/// guards on col2 and col3.
///
/// T1 scan0: col1→A (not proj), col2→B, col3→C
/// T1 scan1: col1→D (not proj), col2→B, col3→C
#[test]
fn ontop_st_elimination1_two_scans_sharing_two_cols() {
    let mut b = Branch {
        core: vec![scan(0), scan(1)],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col2")),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(1, "col3")),
        ],
        distinct: true,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("B".into(), col_binding(0, "col2"));
    b.bindings.insert("C".into(), col_binding(0, "col3"));

    let ctx = CascadeCtx {
        project: Some(&["B".to_owned(), "C".to_owned()]),
        distinct: true,
    };
    let out = run(vec![b], &t1_schema(), &ctx);
    let b = &out[0];
    assert_eq!(b.core.len(), 1, "second T1_AR3 scan eliminated");
    assert_eq!(count_nn_guards(b, "col2"), 1, "IS NOT NULL guard on col2");
    assert_eq!(count_nn_guards(b, "col3"), 1, "IS NOT NULL guard on col3");
    assert_eq!(
        count_nn_guards(b, "col1"),
        0,
        "no guard on col1 (not projected)"
    );
}

/// **@Ignore in Ontop** — `testSelfJoinElimination2`: complex 4-scan case with
/// crossed shared columns (col1/col2 intersecting across scan pairs). Ontop marks
/// this "too complex to support". sf matches: `#[ignore]`.
#[test]
#[ignore = "Ontop acknowledged: 'TODO: try to support this quite complex case'"]
fn ontop_st_elimination2_complex_four_scans_ignored() {
    // The complex 4-scan crossed-sharing case is out of scope for WAVE 1.
    // Ontop's @Ignore means sf's equivalent is also deliberately skipped.
}

/// **GREEN** — Ontop `testSelfJoinElimination3`: three T1_AR3 scans all sharing
/// col2(=B) and col3(=C), under DISTINCT. Result: 1 scan, IS NOT NULL on col2, col3.
#[test]
fn ontop_st_elimination3_three_scans_all_sharing_two_cols() {
    let mut b = Branch {
        core: vec![scan(0), scan(1), scan(2)],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col2")),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(1, "col3")),
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(2, "col2")),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(2, "col3")),
        ],
        distinct: true,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("B".into(), col_binding(0, "col2"));
    b.bindings.insert("C".into(), col_binding(0, "col3"));

    let ctx = CascadeCtx {
        project: Some(&["B".to_owned(), "C".to_owned()]),
        distinct: true,
    };
    let out = run(vec![b], &t1_schema(), &ctx);
    let b = &out[0];
    assert_eq!(b.core.len(), 1, "all three scans collapse to one");
    assert_eq!(count_nn_guards(b, "col2"), 1, "IS NOT NULL guard on col2");
    assert_eq!(count_nn_guards(b, "col3"), 1, "IS NOT NULL guard on col3");
}

/// **GREEN** — Ontop `testSelfJoinElimination4`: three scans all have col2="plop"
/// (constant), shared col3(=C). Under DISTINCT, result: 1 scan, Cmp(col2="plop")
/// on keep, IS NOT NULL on col3 only (no guard needed for the constant col2).
#[test]
fn ontop_st_elimination4_constant_shared_col_no_guard() {
    let mut b = Branch {
        core: vec![scan(0), scan(1), scan(2)],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![
            // col2 = "plop" on each scan (constant binding)
            SqlCond::Cmp(ColRef::new(0, "col2"), CmpOp::Eq, "plop".into()),
            SqlCond::Cmp(ColRef::new(1, "col2"), CmpOp::Eq, "plop".into()),
            SqlCond::Cmp(ColRef::new(2, "col2"), CmpOp::Eq, "plop".into()),
            // col3 joined across all three scans
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(1, "col3")),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(2, "col3")),
        ],
        distinct: true,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("C".into(), col_binding(0, "col3"));

    let ctx = CascadeCtx {
        project: Some(&["C".to_owned()]),
        distinct: true,
    };
    let out = run(vec![b], &t1_schema(), &ctx);
    let b = &out[0];
    assert_eq!(b.core.len(), 1, "three scans collapse to one");
    // IS NOT NULL only on col3 (C is projected, col2 has constant equality so no guard needed)
    assert_eq!(count_nn_guards(b, "col3"), 1, "IS NOT NULL on col3");
    assert_eq!(
        count_nn_guards(b, "col2"),
        0,
        "no IS NOT NULL on col2 — constant equality already implies non-null"
    );
    // The constant Cmp on the surviving scan's col2 must remain.
    let has_cmp = b.where_conds.iter().any(|c| {
        if let SqlCond::Cmp(r, CmpOp::Eq, v) = c {
            r.column.as_ref() == "col2" && &**v == "plop"
        } else {
            false
        }
    });
    assert!(
        has_cmp,
        "Cmp(col2='plop') must remain on the surviving scan"
    );
}

/// **GREEN** — Ontop `testSelfJoinElimination5`: two scans have col2="plop"
/// (constant), one has col2=B (variable). All share col3(=C). Under DISTINCT,
/// scans with constant can be merged; the variable-col2 scan is also eliminable
/// (its col3 is covered). Result: 1 scan, Cmp(col2="plop"), IS NOT NULL on col3.
#[test]
fn ontop_st_elimination5_mixed_constant_and_variable_col2() {
    let mut b = Branch {
        core: vec![scan(0), scan(1), scan(2)],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![
            SqlCond::Cmp(ColRef::new(0, "col2"), CmpOp::Eq, "plop".into()),
            SqlCond::Cmp(ColRef::new(1, "col2"), CmpOp::Eq, "plop".into()),
            // scan2 has col2→B (variable), no Cmp on col2
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(1, "col3")),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(2, "col3")),
        ],
        distinct: true,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("C".into(), col_binding(0, "col3"));
    // B on scan2.col2 is NOT projected.

    let ctx = CascadeCtx {
        project: Some(&["C".to_owned()]),
        distinct: true,
    };
    let out = run(vec![b], &t1_schema(), &ctx);
    let b = &out[0];
    assert_eq!(b.core.len(), 1, "all three scans collapse to one");
    assert_eq!(count_nn_guards(b, "col3"), 1, "IS NOT NULL on col3");
    assert_eq!(count_nn_guards(b, "col2"), 0, "no IS NOT NULL on col2");
}

/// **GREEN** — Ontop `testSelfJoinElimination6`: two scans sharing col2(=B) and
/// col3(=C) but col1 is ALSO projected (A from scan0). Under DISTINCT: scan1's
/// col2 and col3 are covered by ColEqs, so scan1 is eliminated. Result: 1 scan,
/// IS NOT NULL on col2 and col3.
#[test]
fn ontop_st_elimination6_col1_also_projected_from_keep() {
    let mut b = Branch {
        core: vec![scan(0), scan(1)],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col2")),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(1, "col3")),
        ],
        distinct: true,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    // A is from scan0.col1 (projected). B, C from scan0.col2/col3.
    b.bindings.insert("A".into(), col_binding(0, "col1"));
    b.bindings.insert("B".into(), col_binding(0, "col2"));
    b.bindings.insert("C".into(), col_binding(0, "col3"));
    // D from scan1.col1 — NOT projected.

    let ctx = CascadeCtx {
        project: Some(&["A".to_owned(), "B".to_owned(), "C".to_owned()]),
        distinct: true,
    };
    let out = run(vec![b], &t1_schema(), &ctx);
    let b = &out[0];
    assert_eq!(
        b.core.len(),
        1,
        "scan1 eliminated — its projected cols covered by ColEqs"
    );
    // IS NOT NULL on col2 and col3 (the shared cols between the two scans).
    assert_eq!(count_nn_guards(b, "col2"), 1, "IS NOT NULL on col2");
    assert_eq!(count_nn_guards(b, "col3"), 1, "IS NOT NULL on col3");
}

/// **@Ignore in Ontop** — `testSelfJoinElimination7`: 4-scan case where two pairs
/// share partially overlapping columns. Ontop marks this "too complex". sf matches.
#[test]
#[ignore = "Ontop acknowledged: 'TODO: try to support this quite complex case'"]
fn ontop_st_elimination7_complex_four_scans_partial_overlap_ignored() {}

/// **GREEN** — Ontop `testSelfJoinElimination8`: the SAME scan joined with itself
/// (all three columns shared: col1=A, col2=B, col3=C). All projected. Under
/// DISTINCT: IS NOT NULL on col1, col2, col3.
#[test]
fn ontop_st_elimination8_self_join_all_cols_projected() {
    let mut b = Branch {
        core: vec![scan(0), scan(1)],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(1, "col1")),
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col2")),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(1, "col3")),
        ],
        distinct: true,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("A".into(), col_binding(0, "col1"));
    b.bindings.insert("B".into(), col_binding(0, "col2"));
    b.bindings.insert("C".into(), col_binding(0, "col3"));

    let ctx = CascadeCtx {
        project: Some(&["A".to_owned(), "B".to_owned(), "C".to_owned()]),
        distinct: true,
    };
    let out = run(vec![b], &t1_schema(), &ctx);
    let b = &out[0];
    assert_eq!(b.core.len(), 1, "self-join collapses to single scan");
    assert_eq!(count_nn_guards(b, "col1"), 1, "IS NOT NULL on col1");
    assert_eq!(count_nn_guards(b, "col2"), 1, "IS NOT NULL on col2");
    assert_eq!(count_nn_guards(b, "col3"), 1, "IS NOT NULL on col3");
}

// --- Non-elimination tests (pass 2c is a no-op) ----------------------------

/// **GREEN** — Ontop `testSelfJoinNonElimination1`: same-table scans, DISTINCT
/// is NOT set ⇒ pass 2c does not fire ⇒ 2 scans remain.
#[test]
fn ontop_st_non_elimination1_no_distinct() {
    let mut b = Branch {
        core: vec![scan(0), scan(1)],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col2")),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(1, "col3")),
        ],
        distinct: false, // <— no DISTINCT
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("B".into(), col_binding(0, "col2"));
    b.bindings.insert("C".into(), col_binding(0, "col3"));

    let ctx = CascadeCtx {
        project: Some(&["B".to_owned(), "C".to_owned()]),
        distinct: false,
    };
    let out = run(vec![b], &t1_schema(), &ctx);
    assert_eq!(
        out[0].core.len(),
        2,
        "no DISTINCT ⇒ pass 2c is a no-op ⇒ 2 scans remain"
    );
}

/// **GREEN** — Ontop `testSelfJoinNonElimination2`: two scans with NO shared
/// projected columns (scan0 has col2=B, scan1 has col3=C, no cross-scan ColEqs)
/// ⇒ elimination cannot fire ⇒ 2 scans remain.
#[test]
fn ontop_st_non_elimination2_no_shared_projected_cols() {
    let mut b = Branch {
        core: vec![scan(0), scan(1)],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        // No cross-scan ColEqs — the projected cols (B, C) live on separate scans.
        where_conds: Vec::new(),
        distinct: true,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("B".into(), col_binding(0, "col2"));
    b.bindings.insert("C".into(), col_binding(1, "col3"));

    let ctx = CascadeCtx {
        project: Some(&["B".to_owned(), "C".to_owned()]),
        distinct: true,
    };
    let out = run(vec![b], &t1_schema(), &ctx);
    assert_eq!(
        out[0].core.len(),
        2,
        "no shared projected cols ⇒ elimination cannot fire ⇒ 2 scans remain"
    );
}

/// **GREEN** — Ontop `testSelfJoinNonElimination2bis`: the state AFTER projection-
/// shrinking (scan0 has only col2=B bound; scan1 has only col3=C), with DISTINCT
/// — no cross-scan ColEqs ⇒ 2 scans remain. Same semantics as NonElimination2
/// once the unprojected columns are removed.
#[test]
fn ontop_st_non_elimination2bis_minimal_scans_no_shared_cols() {
    let mut b = Branch {
        core: vec![scan(0), scan(1)],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: Vec::new(),
        distinct: true,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("B".into(), col_binding(0, "col2"));
    b.bindings.insert("C".into(), col_binding(1, "col3"));

    let ctx = CascadeCtx {
        project: Some(&["B".to_owned(), "C".to_owned()]),
        distinct: true,
    };
    let out = run(vec![b], &t1_schema(), &ctx);
    assert_eq!(
        out[0].core.len(),
        2,
        "no cross-scan ColEqs ⇒ 2 scans remain"
    );
}

/// **GREEN** — Ontop `testSelfJoinNonElimination3`: two scans sharing col1(=A)
/// via a cross-scan ColEq, but the PROJECTED variables B and C come from separate
/// columns of different scans (B from scan0.col2, C from scan1.col3). scan1's
/// col3(=C) binding is NOT covered by any ColEq ⇒ elimination cannot fire.
#[test]
fn ontop_st_non_elimination3_projected_col_not_covered_by_coleq() {
    let mut b = Branch {
        core: vec![scan(0), scan(1)],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        // col1 is shared (A appears in both scans), but NOT projected.
        where_conds: vec![SqlCond::ColEq(
            ColRef::new(0, "col1"),
            ColRef::new(1, "col1"),
        )],
        distinct: true,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    // B from scan0.col2 (projected) — NOT covered by the col1 ColEq.
    b.bindings.insert("B".into(), col_binding(0, "col2"));
    // C from scan1.col3 (projected) — NOT covered by any ColEq.
    b.bindings.insert("C".into(), col_binding(1, "col3"));

    let ctx = CascadeCtx {
        project: Some(&["B".to_owned(), "C".to_owned()]),
        distinct: true,
    };
    let out = run(vec![b], &t1_schema(), &ctx);
    assert_eq!(
        out[0].core.len(),
        2,
        "scan1.col3 (C) not covered by any ColEq ⇒ elimination blocked ⇒ 2 scans remain"
    );
}

/// **GREEN** — Ontop `testSelfJoinNonElimination4`: two scans sharing col2(=B) and
/// col3(=C) via ColEqs, but col1 has DIFFERENT constants on each scan ("cst1" vs
/// "cst2"). `same_cond_on_keep` fails because the constants differ ⇒ NO elimination.
#[test]
fn ontop_st_non_elimination4_different_constants_on_col1() {
    let mut b = Branch {
        core: vec![scan(0), scan(1)],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![
            SqlCond::Cmp(ColRef::new(0, "col1"), CmpOp::Eq, "cst1".into()),
            SqlCond::Cmp(ColRef::new(1, "col1"), CmpOp::Eq, "cst2".into()),
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col2")),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(1, "col3")),
        ],
        distinct: true,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("B".into(), col_binding(0, "col2"));
    b.bindings.insert("C".into(), col_binding(0, "col3"));

    let ctx = CascadeCtx {
        project: Some(&["B".to_owned(), "C".to_owned()]),
        distinct: true,
    };
    let out = run(vec![b], &t1_schema(), &ctx);
    assert_eq!(
        out[0].core.len(),
        2,
        "different constants on col1 ⇒ same_cond_on_keep fails ⇒ 2 scans remain"
    );
}
