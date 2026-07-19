//! WS-FK — Ontop 5.5.0 `RedundantJoinFKTest` oracle (ADR-0022, WAVE 1).
//!
//! Ports 9 scenarios from
//! `ontop/core/optimization/src/test/java/.../iq/executor/RedundantJoinFKTest.java`
//! re-expressed against sf's `iq.rs` IR (not a Java transliteration). All 9 are
//! GREEN (pass 4 single-col and multi-col FK/PK join elimination covers them).
//!
//! Schema fixtures mirror the Ontop test class:
//!   TABLE1: PK=col1, col2 NOT NULL
//!   TABLE2: col1 NOT NULL, col2 NOT NULL FK→TABLE1.col1
//!   TABLE3: composite PK=(col1,col2), col3 NOT NULL
//!   TABLE4: col1 NOT NULL, col2+col3 composite FK→TABLE3.(col1+col2)
#![cfg(test)]

use super::*;
use sf_sql::{Column, ForeignKey};
use std::collections::BTreeMap;

fn scan(alias: usize, table: &str) -> crate::iq::Scan {
    crate::iq::Scan {
        alias,
        source: LogicalSource::Table(table.to_owned()),
    }
}

fn col_binding(alias: usize, col: &str) -> TermDef {
    use sf_core::ir::{TermMap, TermSpec};
    TermDef::Derived {
        term_map: TermMap::Column(col.into(), TermSpec::plain_literal()),
        alias,
    }
}

/// Build the single-col FK schema (TABLE1 + TABLE2).
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

/// Build the composite FK schema (TABLE3 + TABLE4).
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
    // T4.col2 → T3.col1 AND T4.col3 → T3.col2 (composite FK)
    t4.foreign_keys = vec![ForeignKey {
        columns: vec!["col2".into(), "col3".into()],
        parent_table: "TABLE3".into(),
        parent_columns: vec!["col1".into(), "col2".into()],
    }];
    vec![t3, t4]
}

// --- Single-column FK tests -----------------------------------------------

/// **GREEN** — Ontop `testForeignKeyOptimization`: TABLE2.col2 FK→TABLE1.col1,
/// TABLE1 only needed for its PK col1 ⇒ TABLE1 eliminated.
///
/// Setup: PROJECT A; T1(col1=A,col2=B) JOIN T2(col1=D,col2=A) ON T1.col1=T2.col2
/// Expected: only T2 scan remains; A rebound to T2.col2.
#[test]
fn ontop_fk_optimization_single_col() {
    let mut b = Branch {
        core: vec![scan(0, "TABLE1"), scan(1, "TABLE2")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![SqlCond::ColEq(
            ColRef::new(0, "col1"),
            ColRef::new(1, "col2"),
        )],
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
        nps: false,
    };
    // A is the projected variable (TABLE1.col1 = TABLE2.col2 via FK join).
    b.bindings.insert("A".into(), col_binding(0, "col1"));
    // B and D are NOT projected — they reference TABLE1.col2 and TABLE2.col1.
    // We don't add their bindings to keep the Branch clean; pass 4 checks
    // `parent_referenced_only_via` against bindings, where_conds, and opts.

    let ctx = CascadeCtx {
        project: Some(&["A".to_owned()]),
        distinct: false,
    };
    let out = run(vec![b], &single_fk_schema(), &ctx);
    assert_eq!(out.len(), 1);
    let b = &out[0];
    assert_eq!(
        b.core.len(),
        1,
        "TABLE1 eliminated — T2.col2 FK to T1.col1 guarantees a match"
    );
    assert!(
        matches!(&b.core[0].source, LogicalSource::Table(t) if t == "TABLE2"),
        "T2 is the surviving scan"
    );
    assert!(b.where_conds.is_empty(), "FK ColEq dropped");
    // A must now reference TABLE2 (alias 1).
    match b.bindings.get("A").unwrap() {
        TermDef::Derived { alias, .. } => assert_eq!(*alias, 1),
        other => panic!("unexpected binding {other:?}"),
    }
}

/// **GREEN** — Ontop `testForeignKeyNonOptimization`: B (TABLE1.col2) is projected
/// ⇒ TABLE1 referenced beyond its PK ⇒ NO elimination.
#[test]
fn ontop_fk_non_opt_projected_non_pk_col() {
    let mut b = Branch {
        core: vec![scan(0, "TABLE1"), scan(1, "TABLE2")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![SqlCond::ColEq(
            ColRef::new(0, "col1"),
            ColRef::new(1, "col2"),
        )],
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
        nps: false,
    };
    b.bindings.insert("A".into(), col_binding(0, "col1"));
    // B is projected AND binds TABLE1.col2 — parent referenced via non-PK col.
    b.bindings.insert("B".into(), col_binding(0, "col2"));

    let ctx = CascadeCtx {
        project: Some(&["A".to_owned(), "B".to_owned()]),
        distinct: false,
    };
    let out = run(vec![b], &single_fk_schema(), &ctx);
    assert_eq!(
        out[0].core.len(),
        2,
        "TABLE1 must be preserved — col2 is projected"
    );
}

/// **GREEN** — Ontop `testForeignKeyNonOptimization1`: TABLE1 has an extra local
/// condition (col2 = 1) that references a non-PK column ⇒ NO elimination.
#[test]
fn ontop_fk_non_opt_extra_local_condition() {
    let mut b = Branch {
        core: vec![scan(0, "TABLE1"), scan(1, "TABLE2")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(1, "col2")),
            SqlCond::Cmp(ColRef::new(0, "col2"), CmpOp::Eq, "1".into()),
        ],
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
        nps: false,
    };
    b.bindings.insert("A".into(), col_binding(0, "col1"));

    let ctx = CascadeCtx {
        project: Some(&["A".to_owned()]),
        distinct: false,
    };
    let out = run(vec![b], &single_fk_schema(), &ctx);
    assert_eq!(
        out[0].core.len(),
        2,
        "TABLE1 must be preserved — Cmp on col2 references parent beyond its PK"
    );
}

/// **GREEN** — Ontop `testForeignKeyNonOptimization2`: a post-join filter pushes
/// a constraint onto TABLE1.col2 (non-PK) ⇒ NO elimination.
/// Modelled as a Cmp on T1.col2 already present in where_conds (representing
/// a filter that has been pushed to the parent scan).
#[test]
fn ontop_fk_non_opt_post_join_filter_on_parent() {
    let mut b = Branch {
        core: vec![scan(0, "TABLE1"), scan(1, "TABLE2")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(1, "col2")),
            // Filter "B = 1" pushed to TABLE1.col2 after join:
            SqlCond::Cmp(ColRef::new(0, "col2"), CmpOp::Eq, "1".into()),
        ],
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
        nps: false,
    };
    b.bindings.insert("A".into(), col_binding(0, "col1"));
    b.bindings.insert("D".into(), col_binding(1, "col1"));

    let ctx = CascadeCtx {
        project: Some(&["A".to_owned(), "D".to_owned()]),
        distinct: false,
    };
    let out = run(vec![b], &single_fk_schema(), &ctx);
    assert_eq!(
        out[0].core.len(),
        2,
        "TABLE1 must be preserved — Cmp on non-PK col2 blocks elimination"
    );
}

/// **GREEN** — Ontop `testForeignKeyNonOptimization3`: TABLE1.col1 and TABLE1.col2
/// both map to the same variable A (a self-equality ColEq(0.col1, 0.col2)) ⇒
/// TABLE1 is referenced via col2 in addition to the FK col1 ⇒ NO elimination.
#[test]
fn ontop_fk_non_opt_parent_self_equality() {
    let mut b = Branch {
        core: vec![scan(0, "TABLE1"), scan(1, "TABLE2")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![
            // FK join: T2.col2 = T1.col1
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(1, "col2")),
            // Self-equality on TABLE1 (col1=A AND col2=A → col1=col2)
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(0, "col2")),
        ],
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
        nps: false,
    };
    b.bindings.insert("A".into(), col_binding(0, "col1"));

    let ctx = CascadeCtx {
        project: Some(&["A".to_owned()]),
        distinct: false,
    };
    let out = run(vec![b], &single_fk_schema(), &ctx);
    assert_eq!(
        out[0].core.len(),
        2,
        "TABLE1 must be preserved — self-equality references parent via col2"
    );
}

/// **GREEN** — Ontop `testForeignKeyOptimization1` (simplified): a multi-scan join
/// where ONE T1 scan is FK-eliminable (only referenced via col1) and another is NOT
/// (referenced via col2 via a self-equality). After cascade: 3 core scans remain
/// (the eliminable T1 scan is dropped); the non-eliminable T1 scan and both T2
/// scans survive.
#[test]
fn ontop_fk_opt_multi_scan_partial_elimination() {
    // T1 alias 0: col1=A, col2=A (self-equality → NOT eliminable)
    // T1 alias 1: col1=C (only referenced via col1 → eliminable)
    // T2 alias 2: col2=A (FK to T1 col1=A → tries to eliminate alias 0, but can't)
    // T2 alias 3: col2=C (FK to T1 col1=C → eliminates alias 1)
    let mut b = Branch {
        core: vec![
            scan(0, "TABLE1"),
            scan(1, "TABLE1"),
            scan(2, "TABLE2"),
            scan(3, "TABLE2"),
        ],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![
            // T1_0 self-equality (col1=A, col2=A)
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(0, "col2")),
            // T2_2 FK join to T1_0 (T2.col2=A = T1.col1=A)
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(2, "col2")),
            // T2_3 FK join to T1_1 (T2.col2=C = T1.col1=C)
            SqlCond::ColEq(ColRef::new(1, "col1"), ColRef::new(3, "col2")),
        ],
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
        nps: false,
    };
    b.bindings.insert("A".into(), col_binding(0, "col1"));
    // C and D are intermediate/not projected but exist
    b.bindings.insert("C".into(), col_binding(1, "col1"));

    let ctx = CascadeCtx {
        project: Some(&["A".to_owned()]),
        distinct: false,
    };
    let out = run(vec![b], &single_fk_schema(), &ctx);
    assert_eq!(out.len(), 1);
    let remaining = out[0].core.len();
    assert_eq!(remaining, 3, "T1 alias 1 eliminated (only referenced via col1); T1 alias 0 preserved (self-equality blocks)");
    // T1 alias 0 must still be present.
    assert!(
        out[0].core.iter().any(|s| s.alias == 0),
        "T1 with self-equality (alias 0) must survive"
    );
    // T1 alias 1 must be gone.
    assert!(
        !out[0].core.iter().any(|s| s.alias == 1),
        "T1 without self-equality (alias 1) must be eliminated"
    );
}

// --- Composite FK tests ---------------------------------------------------

/// **GREEN** — Ontop `testForeignKeyOptimization2`: TABLE4 has a composite FK
/// (col2+col3 → TABLE3.col1+col2). T3 is only referenced via its composite PK
/// (col1, col2) ⇒ TABLE3 eliminated.
///
/// Setup: T3(col1=A,col2=B,col3=C) JOIN T4(col1=D,col2=A,col3=B) on T3.col1=T4.col2 AND T3.col2=T4.col3
/// Expected: only T4 scan remains; A rebound to T4.col2.
#[test]
fn ontop_fk_opt_composite_key() {
    let mut b = Branch {
        core: vec![scan(0, "TABLE3"), scan(1, "TABLE4")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![
            // T3.col1(=A) = T4.col2(=A)
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(1, "col2")),
            // T3.col2(=B) = T4.col3(=B)
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col3")),
        ],
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
        nps: false,
    };
    b.bindings.insert("A".into(), col_binding(0, "col1"));

    let ctx = CascadeCtx {
        project: Some(&["A".to_owned()]),
        distinct: false,
    };
    let out = run(vec![b], &composite_fk_schema(), &ctx);
    assert_eq!(out.len(), 1);
    let b = &out[0];
    assert_eq!(b.core.len(), 1, "TABLE3 eliminated by composite FK/PK join");
    assert!(
        matches!(&b.core[0].source, LogicalSource::Table(t) if t == "TABLE4"),
        "TABLE4 is the surviving scan"
    );
    // A now references TABLE4 (alias 1).
    match b.bindings.get("A").unwrap() {
        TermDef::Derived { alias, .. } => assert_eq!(*alias, 1),
        other => panic!("unexpected binding {other:?}"),
    }
}

/// **GREEN** — Ontop `testForeignKeyNonOptimization4`: TABLE4 has the composite FK
/// (col2→T3.col1 AND col3→T3.col2) but the actual join equalities are SWAPPED
/// (T4.col2=T3.col2 AND T4.col3=T3.col1). Positional alignment fails ⇒ NO elimination.
///
/// This test verified the soundness fix: a set-membership-only check would incorrectly
/// eliminate TABLE3 here (=_bag violation); sf must block it.
#[test]
fn ontop_fk_non_opt_composite_wrong_column_alignment() {
    // T3 scan: col1→A, col2→B. T4 scan: col1→D, col2→B, col3→A.
    // Join conditions: T3.col2(=B) = T4.col2(=B) AND T3.col1(=A) = T4.col3(=A)
    // FK says: T4.col2→T3.col1 AND T4.col3→T3.col2 — neither pair matches.
    let mut b = Branch {
        core: vec![scan(0, "TABLE3"), scan(1, "TABLE4")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![
            // T3.col2(=B) = T4.col2(=B) — misaligned: FK expects T4.col2 = T3.col1
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col2")),
            // T3.col1(=A) = T4.col3(=A) — misaligned: FK expects T4.col3 = T3.col2
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(1, "col3")),
        ],
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
        nps: false,
    };
    b.bindings.insert("A".into(), col_binding(0, "col1"));

    let ctx = CascadeCtx {
        project: Some(&["A".to_owned()]),
        distinct: false,
    };
    let out = run(vec![b], &composite_fk_schema(), &ctx);
    assert_eq!(
        out[0].core.len(),
        2,
        "TABLE3 must be preserved — join columns do not match the composite FK declaration"
    );
}

/// **GREEN** — Ontop `testForeignKeyNonOptimization5`: TABLE3 has a self-equality
/// (col1=col2, both bound to A), so the composite FK join only equates col2
/// of T4 to T3.col1 (not col3=B), and the full composite key condition is not
/// covered ⇒ NO elimination.
#[test]
fn ontop_fk_non_opt_composite_parent_self_equality() {
    // T3 scan: col1→A, col2→A (self-equality ColEq(0.col1, 0.col2))
    // T4 scan: col2→A (FK col for col1 of T3); col3 maps to B (not A=T3.col2)
    // The composite FK requires T4.col2=T3.col1 AND T4.col3=T3.col2.
    // But T3.col2=A and T4.col3=B (different variables) → no ColEq(0.col2, 1.col3) → incomplete.
    let mut b = Branch {
        core: vec![scan(0, "TABLE3"), scan(1, "TABLE4")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![
            // T3 self-equality (col1=A AND col2=A)
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(0, "col2")),
            // Only one composite FK condition present (the T3.col2=T4.col3 is missing)
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(1, "col2")),
            // T3.col2(=A) = T4.col1(=A) — references T3 via col2 but isn't a FK col
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col1")),
        ],
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
        nps: false,
    };
    b.bindings.insert("A".into(), col_binding(0, "col1"));

    let ctx = CascadeCtx {
        project: Some(&["A".to_owned()]),
        distinct: false,
    };
    let out = run(vec![b], &composite_fk_schema(), &ctx);
    assert_eq!(
        out[0].core.len(),
        2,
        "TABLE3 must be preserved — composite FK condition is incomplete"
    );
}
