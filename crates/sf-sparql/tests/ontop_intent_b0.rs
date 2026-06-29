//! Ontop-parity intent — batch 0 of 8 (ADR-0022): RED-SPEC (#[ignore]d) tests.
//!
//! Companion to `ontop_port_b0.rs` which holds the GREEN (passing) tests.
//! This file adds `#[ignore]`-d RED-SPEC tests for NEEDS_IMPL scenarios that
//! represent genuine cascade gaps, confirmed against the Ontop 5.5.0 oracle.
//! Every function here currently fails (or would fail) — that is intentional.
//! The `#[ignore]` attribute keeps CI green; lift it once the gap is closed.
//!
//! Assigned classes (batch 0):
//!
//!   * `iq/executor/EmptyNodeRemovalTest.java`     (21 scenarios — ALL BOUNDARY)
//!   * `iq/executor/FunctionalDependencyTest.java` (35 scenarios — 2 GREEN in port,
//!     3 RED-SPEC here, remainder NEEDS_IMPL)
//!   * `iq/executor/LeftJoinOptimizationTest.java` (180 scenarios — 4 GREEN in port,
//!     3 RED-SPEC here, many BOUNDARY)
//!   * `iq/executor/LJJoinLiftTest.java`           ( 2 scenarios — ALL BOUNDARY)
//!   * `iq/executor/QueryMergingTest.java`         (21 scenarios — ALL BOUNDARY)
//!
//! # Classification summary
//!
//! **BOUNDARY** (out of cascade charter — no faithful sf expression exists)
//!
//!   * `EmptyNodeRemovalTest` (all 21): exercises Ontop IQ-tree `EmptyNode` /
//!     `ConstructionNode` / `UnionNode` / `LeftJoinNode` propagation.  sf has no
//!     such node types; unsatisfiable sub-patterns yield zero branches at unfold
//!     time, not a later-pass removal.
//!   * `LJJoinLiftTest` (both): `UNION_AND_BINDING_LIFT_OPTIMIZER` — lifts a
//!     `UnionNode` above an INNER/LEFT join.  sf compiles UNION to a flat bag of
//!     independent `Branch`es; there is no union tree node to lift.
//!   * `QueryMergingTest` (all 21): `AbstractQueryMergingTransformer` —
//!     substitutes mapping sub-queries into `IntensionalDataNode`s.  sf inlines
//!     mappings at unfold; no analogous merge pass exists.
//!   * `LeftJoinOptimizationTest` structural families: `testMergeLJs*`,
//!     `testProjectionAway*`, `testLJReductionWithLJOnTheRight*`, construction-node
//!     projection shrinking, provenance/DISTINCT interaction, VALUES/SLICE inside a
//!     LEFT JOIN.
//!
//! **NEEDS_IMPL** (cascade gap — expressible at the `Branch` level, not yet done)
//!
//!   * Non-unique FD self-join elimination (`FunctionalDependencyTest`):
//!     `TableSchema` has no `functional_dependencies` field; the cascade cannot
//!     reason over `col2 → col3,col4` to prove the second scan redundant.
//!     Covers `testRedundantSelfJoin1–7`, `testRedundantSelfJoin10–18`,
//!     `testLJRedundantSelfLeftJoin1,2`, and more.
//!   * FD-closure contradiction → empty result (`testRejectedJoin1–3`):
//!     contradictory constants on the FD determinant across join legs.
//!   * LJ → IJ downgrade via NOT-NULL FK match guarantee
//!     (`testLeftJoinElimination1,2,4`, `…WithFilterCondition2`,
//!     `…WithImplicitFilterCondition`): sf's `fk_pk` pass only *drops* a redundant
//!     parent scan; it never *downgrades* a LEFT JOIN on FK match-guarantee.
//!   * Self-LJ elimination with right-side filter lifted to `IfElseNull`
//!     (`testSelfJoinElimination2,3`, `testSelfJoinWithCondition`,
//!     `testSelfLeftJoinIfElseNull1,2`, `testSelfLeftJoinWithJoinOnLeft2`,
//!     `testLeftJoinEliminationWithFilterCondition4`): pass 2b refuses when
//!     `opts[i].extra` is non-empty (cannot synthesise the conditional term).
//!   * Self-LJ non-unification via contradictory constants → NULL-padding
//!     (`testSelfLeftJoinNonUnification1`, `…1NotSimplifiedExpression`,
//!     `…EmptyResult`): no cross-alias column-identity reasoning.

use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_sparql::cascade::{run, CascadeCtx};
use sf_sparql::iq::{Branch, CmpOp, ColRef, OptJoin, Scan, SqlCond, TermDef};
use sf_sql::{Column, ForeignKey, TableSchema};

// --- shared helpers -----------------------------------------------------------

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

/// `FunctionalDependencyTest` TABLE1: PK `col1`, non-unique FD `col2 → col3,col4`,
/// independent `col5`; all NOT NULL.  sf models only the PK — no
/// `functional_dependencies` field on `TableSchema` — which is why the FD tests
/// below are NEEDS_IMPL.
fn fd_table1() -> TableSchema {
    let mut t = TableSchema::new("table1");
    t.primary_key = vec!["col1".into()];
    t.columns = vec![
        Column::new("col1", "integer", true),
        Column::new("col2", "integer", true),
        Column::new("col3", "integer", true),
        Column::new("col4", "integer", true),
        Column::new("col5", "integer", true),
    ];
    t
}

// ===========================================================================
// RED-SPEC — FunctionalDependencyTest: non-unique FD self-join elimination
// ===========================================================================

/// **RED-SPEC** — Ontop `FunctionalDependencyTest.testRedundantSelfJoin1`.
///
/// Two TABLE1 scans inner-joined on `col2` (the FD determinant).  Bindings for
/// `?y` (col3) and `?z` (col4) are read from the *second* scan.  Ontop's FD
/// engine recognises that `col2 → col3,col4` makes the second scan redundant
/// and collapses to a single scan.  sf has no `functional_dependencies` field on
/// `TableSchema` and therefore cannot perform this inference.
#[test]
#[ignore = "NEEDS_IMPL: TableSchema has no functional_dependencies field; \
            non-unique FD col2→{col3,col4} self-join elimination not implemented \
            — FunctionalDependencyTest.testRedundantSelfJoin1"]
fn fd_self_join_elim_basic() {
    // TABLE1: PK=col1, non-unique FD: col2→{col3,col4}, col5 independent.
    // Two scans join on col2 (the FD determinant). bindings read col3,col4 from scan1.
    // Expected: cascade merges to one scan; all bindings from scan0.
    let mut t = TableSchema::new("table1");
    t.primary_key = vec!["col1".into()];
    t.columns = vec![
        Column::new("col1", "integer", true),
        Column::new("col2", "integer", true),
        Column::new("col3", "integer", true),
        Column::new("col4", "integer", true),
        Column::new("col5", "integer", true),
    ];
    // Would set: t.functional_dependencies = vec![FD { det: vec!["col2"], dep: vec!["col3","col4"] }];
    // but field does not exist yet.

    let mut b = Branch::empty();
    b.core = vec![scan(0, "table1"), scan(1, "table1")];
    b.where_conds = vec![SqlCond::ColEq(
        ColRef::new(0, "col2"),
        ColRef::new(1, "col2"),
    )];
    b.bindings.insert("x".into(), col_binding(0, "col2")); // from kept scan
    b.bindings.insert("y".into(), col_binding(1, "col3")); // from scan to eliminate
    b.bindings.insert("z".into(), col_binding(1, "col4")); // from scan to eliminate

    let out = run(vec![b], &[t], &CascadeCtx::default());
    let b = &out[0];
    assert_eq!(b.core.len(), 1, "FD self-join should collapse to one scan");
    assert!(
        b.bindings
            .values()
            .all(|d| matches!(d, TermDef::Derived { alias: 0, .. })),
        "all bindings must migrate to the kept scan"
    );
}

/// **RED-SPEC** — Ontop `FunctionalDependencyTest.testRejectedJoin1`.
///
/// Two TABLE1 scans inner-joined on `col2`, but with contradictory constant
/// equalities: `col2 = 1` on the first scan and `col2 = 2` on the second.
/// Ontop's FD-closure reasoning detects that the join is unsatisfiable
/// (the shared determinant cannot simultaneously equal 1 and 2) and yields an
/// empty result.  sf neither models non-unique FDs nor detects constant
/// contradictions across equi-join legs.
#[test]
fn fd_self_join_rejected_join_contradiction() {
    // scan0 requires col2=1, scan1 requires col2=2, inner-joined on col2.
    // col2=1 AND col2=2 is unsatisfiable → empty result.
    let mut b = Branch::empty();
    b.core = vec![scan(0, "table1"), scan(1, "table1")];
    b.where_conds = vec![
        SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col2")),
        SqlCond::Cmp(ColRef::new(0, "col2"), CmpOp::Eq, "1".to_owned()),
        SqlCond::Cmp(ColRef::new(1, "col2"), CmpOp::Eq, "2".to_owned()),
    ];
    b.bindings.insert("x".into(), col_binding(0, "col2"));
    b.bindings.insert("y".into(), col_binding(1, "col3"));

    let out = run(vec![b], &[fd_table1()], &CascadeCtx::default());
    assert!(
        out.is_empty() || out[0].core.is_empty(),
        "contradictory col2 constants across join legs must produce empty result"
    );
}

/// **RED-SPEC** — Ontop `FunctionalDependencyTest.testRedundantSelfJoin2`.
///
/// Like `fd_self_join_elim_basic` but the FD determinant `col2` is also
/// projected (`?x` bound from scan0's col2).  The determinant being projected
/// does not prevent elimination: scan1 still adds no information beyond what
/// scan0 provides via `col2 → col3,col4`.  Ontop collapses to a single scan;
/// sf cannot without FD support.
#[test]
#[ignore = "NEEDS_IMPL: TableSchema has no functional_dependencies field; \
            non-unique FD col2→{col3,col4} self-join elimination not implemented \
            — FunctionalDependencyTest.testRedundantSelfJoin2"]
fn fd_self_join_elim_with_determinant_projected() {
    // TABLE1: PK=col1, non-unique FD: col2→{col3,col4}.
    // scan0: col2=?x (determinant). scan1: col2=?x, col3=?y, col4=?w (dependents).
    // Expected: collapse to one scan; all bindings from the surviving scan.
    let mut t = TableSchema::new("table1");
    t.primary_key = vec!["col1".into()];
    t.columns = vec![
        Column::new("col1", "integer", true),
        Column::new("col2", "integer", true),
        Column::new("col3", "integer", true),
        Column::new("col4", "integer", true),
        Column::new("col5", "integer", true),
    ];
    // Would set: t.functional_dependencies = vec![FD { det: vec!["col2"], dep: vec!["col3","col4"] }];

    let mut b = Branch::empty();
    b.core = vec![scan(0, "table1"), scan(1, "table1")];
    b.where_conds = vec![SqlCond::ColEq(
        ColRef::new(0, "col2"),
        ColRef::new(1, "col2"),
    )];
    b.bindings.insert("x".into(), col_binding(0, "col2")); // determinant on kept scan
    b.bindings.insert("y".into(), col_binding(1, "col3")); // dependent from scan to eliminate
    b.bindings.insert("w".into(), col_binding(1, "col4")); // dependent from scan to eliminate

    let out = run(vec![b], &[t], &CascadeCtx::default());
    let b = &out[0];
    assert_eq!(
        b.core.len(),
        1,
        "FD self-join with projected determinant should collapse to one scan"
    );
    assert!(
        b.bindings
            .values()
            .all(|d| matches!(d, TermDef::Derived { .. })),
        "all bindings must be on the single surviving scan"
    );
}

// ===========================================================================
// RED-SPEC — LeftJoinOptimizationTest: LJ → IJ via FK match guarantee
// ===========================================================================

/// **RED-SPEC** — Ontop `LeftJoinOptimizationTest.testLeftJoinElimination1`.
///
/// `LeftJoin(TABLE2(col1=M,col2=M1,col3=O), TABLE1(col1=M1,col2=N1,col3=_))`
/// joined on `TABLE2.col2 = TABLE1.col1`.  `TABLE2.col2` is a NOT-NULL FK
/// referencing `TABLE1.col1` (the PK), so every TABLE2 row is guaranteed a
/// matching TABLE1 row → the LEFT JOIN is semantically equal to an INNER JOIN.
/// Ontop downgrades to `InnerJoin(TABLE2, TABLE1)`.  sf's `fk_pk` cascade pass
/// only *drops* a redundant parent scan; it never *downgrades* a LEFT JOIN on a
/// FK match-guarantee basis.
#[test]
fn lj_to_ij_fk_basic() {
    // TABLE1: PK col1. TABLE2: FK col2 NOT NULL → TABLE1.col1.
    // OPTIONAL {TABLE1 t1} ON t2.col2 = t1.col1.
    // FK is NOT NULL ⇒ every t2 row has a matching t1 ⇒ LJ = IJ.
    let mut parent = TableSchema::new("TABLE1");
    parent.primary_key = vec!["col1".into()];
    parent.columns = vec![
        Column::new("col1", "integer", true),
        Column::new("col2", "integer", true),
        Column::new("col3", "integer", true),
    ];

    let mut child = TableSchema::new("TABLE2");
    child.primary_key = vec!["col1".into()];
    child.columns = vec![
        Column::new("col1", "integer", true),
        Column::new("col2", "integer", true), // NOT NULL FK → TABLE1.col1
        Column::new("col3", "integer", true),
    ];
    child.foreign_keys = vec![ForeignKey {
        columns: vec!["col2".into()],
        parent_table: "TABLE1".into(),
        parent_columns: vec!["col1".into()],
    }];

    let mut b = Branch::empty();
    b.core = vec![scan(0, "TABLE2")];
    b.opts = vec![OptJoin {
        scan: scan(1, "TABLE1"),
        on: vec![SqlCond::NullSafeEq(
            ColRef::new(0, "col2"),
            ColRef::new(1, "col1"),
        )],
        extra: vec![],
    }];
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("M1".into(), col_binding(0, "col2")); // FK column on child
    b.bindings.insert("O".into(), col_binding(0, "col3"));
    b.bindings.insert("N1".into(), col_binding(1, "col2")); // from parent (non-PK)

    let out = run(vec![b], &[parent, child], &CascadeCtx::default());
    let b = &out[0];
    assert!(
        b.opts.is_empty(),
        "FK-guaranteed OPTIONAL should downgrade to inner join (no opts remaining)"
    );
}

// ===========================================================================
// RED-SPEC — LeftJoinOptimizationTest: self-LJ with extra filter (IfElseNull)
// ===========================================================================

/// **RED-SPEC** — Ontop `LeftJoinOptimizationTest.testSelfJoinElimination2`.
///
/// `LeftJoin(TABLE1(col1=M,col2=N,col3=_), TABLE1(col1=M,col2=_,col3=O))
///  ON IS_NOT_NULL(O)`.
/// Ontop's PK-driven self-LJ elimination merges to a single scan and lifts the
/// extra condition into an `IfElseNull(IS_NOT_NULL(col3), col3)` construction for
/// `?O`.  sf's pass 2b conservatively refuses elimination whenever
/// `opts[i].extra` is non-empty — it cannot synthesise the conditional term —
/// so the OPTIONAL is kept as-is.
#[test]
fn self_lj_ifelsenull_with_filter() {
    // TABLE1: PK col1 (NOT NULL), col2 NOT NULL, col3 nullable.
    // Left scan: col1=M, col2=N. Right scan: col1=M, col3=O.
    // OPTIONAL extra condition: IS_NOT_NULL(col3 of right scan).
    let mut t = TableSchema::new("TABLE1");
    t.primary_key = vec!["col1".into()];
    t.columns = vec![
        Column::new("col1", "integer", true),
        Column::new("col2", "integer", true),
        Column::new("col3", "integer", false), // nullable
    ];

    let mut b = Branch::empty();
    b.core = vec![scan(0, "TABLE1")];
    b.opts = vec![OptJoin {
        scan: scan(1, "TABLE1"),
        on: vec![SqlCond::NullSafeEq(
            ColRef::new(0, "col1"),
            ColRef::new(1, "col1"),
        )],
        // The IS_NOT_NULL extra condition is what blocks sf's pass 2b today.
        extra: vec![SqlCond::IsNotNull(ColRef::new(1, "col3"))],
    }];
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("N".into(), col_binding(0, "col2"));
    b.bindings.insert("O".into(), col_binding(1, "col3")); // from right scan

    let out = run(vec![b], &[t], &CascadeCtx::default());
    let b = &out[0];
    // After impl: right scan merged via IfElseNull; OPTIONAL removed.
    assert!(
        b.opts.is_empty(),
        "self-LJ with IS_NOT_NULL extra should merge to single scan with conditional ?O"
    );
    // TODO: once TermDef gains a Conditional/IfElseNull variant, also assert that
    //       b.bindings["O"] carries the conditional expression rather than a plain column.
}

// ===========================================================================
// RED-SPEC — LeftJoinOptimizationTest: self-LJ non-unification → NULL-pad
// ===========================================================================

/// **RED-SPEC** — Ontop `LeftJoinOptimizationTest.testSelfLeftJoinNonUnification1`.
///
/// `LeftJoin(TABLE1(col1=M,col2=_,col3=1), TABLE1(col1=M,col2=N,col3=2))`.
/// Both scans read the same PK-identified row (col1=M).  The left side
/// constrains `col3=1`; the right side constrains `col3=2`.  Since `col3` is a
/// deterministic function of `col1` (PK → all columns), the same physical cell
/// cannot equal both 1 and 2 → the OPTIONAL right side can NEVER match → Ontop
/// NULL-pads `?N` and drops the right scan.  sf has no cross-alias
/// column-identity reasoning and does not detect the contradiction.
#[test]
fn self_lj_non_unification_constants() {
    // TABLE1: PK col1. Left scan: WHERE col3=1. Right scan: WHERE col3=2.
    // Same col1=M (PK) → same physical row → col3 cannot be both 1 and 2.
    // Ontop: ?N always NULL; drop right scan.
    let mut t = TableSchema::new("TABLE1");
    t.primary_key = vec!["col1".into()];
    t.columns = vec![
        Column::new("col1", "integer", true),
        Column::new("col2", "integer", true),
        Column::new("col3", "integer", true),
    ];

    let mut b = Branch::empty();
    b.core = vec![scan(0, "TABLE1")];
    b.where_conds = vec![SqlCond::Cmp(
        ColRef::new(0, "col3"),
        CmpOp::Eq,
        "1".to_owned(),
    )];
    b.opts = vec![OptJoin {
        scan: scan(1, "TABLE1"),
        on: vec![SqlCond::NullSafeEq(
            ColRef::new(0, "col1"),
            ColRef::new(1, "col1"),
        )],
        extra: vec![SqlCond::Cmp(
            ColRef::new(1, "col3"),
            CmpOp::Eq,
            "2".to_owned(),
        )],
    }];
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("N".into(), col_binding(1, "col2")); // from right scan → should become NULL

    let out = run(vec![b], &[t], &CascadeCtx::default());
    let b = &out[0];
    // After impl: right scan dropped; ?N becomes NULL (TermDef::Const or similar).
    assert!(
        b.opts.is_empty(),
        "contradictory col3 constants on same PK-identified row: right can never match; \
         OPTIONAL must be dropped and right-bound vars NULL-padded"
    );
}

// ===========================================================================
// GREEN — pass 2d: DISTINCT-driven pruning of unused OPTIONAL right sides
// Covers Ontop LeftJoinOptimizationTest.testDistinctPruneUnusedRight1-7
// and the testDistinctNoPrune* guards.
// ===========================================================================

fn prune_schema() -> Vec<TableSchema> {
    let mut t1 = TableSchema::new("TABLE1");
    t1.primary_key = vec!["col1".into()];
    t1.columns = vec![
        Column::new("col1", "integer", true),
        Column::new("col2", "integer", true),
    ];
    let mut t2 = TableSchema::new("TABLE2");
    t2.primary_key = vec!["col1".into()];
    t2.columns = vec![
        Column::new("col1", "integer", true),
        Column::new("col2", "integer", true),
    ];
    vec![t1, t2]
}

fn ctx_d<'a>(project: &'a [String]) -> CascadeCtx<'a> {
    CascadeCtx {
        distinct: true,
        project: Some(project),
    }
}

/// **GREEN** — `testDistinctPruneUnusedRight1`: basic case — one OPTIONAL
/// whose only binding is not projected; under DISTINCT it is dropped.
#[test]
fn distinct_prune_unused_right_basic() {
    let mut b = Branch::single(scan(0, "TABLE1"));
    b.opts.push(OptJoin {
        scan: scan(1, "TABLE2"),
        on: vec![],
        extra: vec![],
    });
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("N".into(), col_binding(1, "col2")); // NOT projected
    let out = run(vec![b], &prune_schema(), &ctx_d(&["M".into()]));
    assert!(
        out[0].opts.is_empty(),
        "unused OPTIONAL pruned under DISTINCT"
    );
}

/// **GREEN** — `testDistinctPruneUnusedRight2`: OPTIONAL with an `extra`
/// condition on the opt scan itself — still pruned (ON-clause, no core filter).
#[test]
fn distinct_prune_unused_right_extra_on_opt_side() {
    let mut b = Branch::single(scan(0, "TABLE1"));
    b.opts.push(OptJoin {
        scan: scan(1, "TABLE2"),
        on: vec![SqlCond::NullSafeEq(
            ColRef::new(0, "col1"),
            ColRef::new(1, "col1"),
        )],
        extra: vec![SqlCond::IsNotNull(ColRef::new(1, "col2"))],
    });
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("N".into(), col_binding(1, "col2")); // NOT projected
    let out = run(vec![b], &prune_schema(), &ctx_d(&["M".into()]));
    assert!(
        out[0].opts.is_empty(),
        "extra on opt side only → still pruned"
    );
}

/// **GREEN** — `testDistinctPruneUnusedRight3`: two OPTIONALs, both unused
/// under DISTINCT → both dropped.
#[test]
fn distinct_prune_unused_right_two_both_unused() {
    let mut b = Branch::single(scan(0, "TABLE1"));
    b.opts.push(OptJoin {
        scan: scan(1, "TABLE2"),
        on: vec![],
        extra: vec![],
    });
    b.opts.push(OptJoin {
        scan: scan(2, "TABLE2"),
        on: vec![],
        extra: vec![],
    });
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("N".into(), col_binding(1, "col2")); // NOT projected
    b.bindings.insert("P".into(), col_binding(2, "col2")); // NOT projected
    let out = run(vec![b], &prune_schema(), &ctx_d(&["M".into()]));
    assert!(out[0].opts.is_empty(), "both unused OPTIONALs pruned");
}

/// **GREEN** — `testDistinctPruneUnusedRight4`: two OPTIONALs — alias 1
/// unused (pruned), alias 2 used (kept).
#[test]
fn distinct_prune_unused_right_two_opts_one_used() {
    let mut b = Branch::single(scan(0, "TABLE1"));
    b.opts.push(OptJoin {
        scan: scan(1, "TABLE2"),
        on: vec![],
        extra: vec![],
    });
    b.opts.push(OptJoin {
        scan: scan(2, "TABLE2"),
        on: vec![],
        extra: vec![],
    });
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("N".into(), col_binding(1, "col2")); // NOT projected
    b.bindings.insert("P".into(), col_binding(2, "col2")); // IS projected
    let out = run(vec![b], &prune_schema(), &ctx_d(&["M".into(), "P".into()]));
    assert_eq!(out[0].opts.len(), 1, "only unused OPTIONAL pruned");
    assert_eq!(out[0].opts[0].scan.alias, 2, "alias 2 (used) kept");
}

/// **GREEN** — `testDistinctPruneUnusedRight5`: `extra` condition references
/// the CORE alias — still safe to prune (LEFT JOIN ON cannot filter core rows).
#[test]
fn distinct_prune_unused_right_extra_touches_core() {
    let mut b = Branch::single(scan(0, "TABLE1"));
    b.opts.push(OptJoin {
        scan: scan(1, "TABLE2"),
        on: vec![SqlCond::NullSafeEq(
            ColRef::new(0, "col1"),
            ColRef::new(1, "col1"),
        )],
        // Condition references core alias 0 — but it's a LEFT JOIN ON condition,
        // not a WHERE filter: core rows are never filtered by the ON clause.
        extra: vec![SqlCond::ColEq(
            ColRef::new(0, "col2"),
            ColRef::new(1, "col2"),
        )],
    });
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("N".into(), col_binding(1, "col2")); // NOT projected
    let out = run(vec![b], &prune_schema(), &ctx_d(&["M".into()]));
    assert!(
        out[0].opts.is_empty(),
        "extra references core alias but is ON-clause only → still pruned"
    );
}

/// **GREEN** — `testDistinctPruneUnusedRight6`: binding in `b.bindings` whose
/// variable is absent from the project list — treated as unused → pruned.
#[test]
fn distinct_prune_unused_right_binding_not_in_project() {
    let mut b = Branch::single(scan(0, "TABLE1"));
    b.opts.push(OptJoin {
        scan: scan(1, "TABLE2"),
        on: vec![],
        extra: vec![],
    });
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("N".into(), col_binding(1, "col2"));
    b.bindings.insert("O".into(), col_binding(1, "col1")); // also from alias 1, not projected
                                                           // project contains only "M" — none of the alias-1 bindings are projected
    let out = run(vec![b], &prune_schema(), &ctx_d(&["M".into()]));
    assert!(
        out[0].opts.is_empty(),
        "no projected var reads alias 1 → pruned"
    );
}

/// **GREEN** — `testDistinctPruneUnusedRight7`: a `Coalesce` binding that
/// reads alias 1 on the left → the OPTIONAL is retained (the binding IS used).
#[test]
fn distinct_prune_no_prune_coalesce_reads_opt() {
    let mut b = Branch::single(scan(0, "TABLE1"));
    b.opts.push(OptJoin {
        scan: scan(1, "TABLE2"),
        on: vec![],
        extra: vec![],
    });
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    // "N" is a Coalesce that reads alias 1 (opt) first, alias 0 (core) as fallback.
    b.bindings.insert(
        "N".into(),
        TermDef::Coalesce(
            Box::new(col_binding(1, "col2")),
            Box::new(col_binding(0, "col2")),
        ),
    );
    let out = run(vec![b], &prune_schema(), &ctx_d(&["M".into(), "N".into()]));
    assert_eq!(
        out[0].opts.len(),
        1,
        "Coalesce reads opt alias → OPTIONAL kept"
    );
}

/// **GREEN** — `testDistinctNoPrune1`: projected variable IS sourced from the
/// OPTIONAL scan → opt must be retained.
#[test]
fn distinct_no_prune_used_var_projected() {
    let mut b = Branch::single(scan(0, "TABLE1"));
    b.opts.push(OptJoin {
        scan: scan(1, "TABLE2"),
        on: vec![],
        extra: vec![],
    });
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("N".into(), col_binding(1, "col2")); // IS projected
    let out = run(vec![b], &prune_schema(), &ctx_d(&["M".into(), "N".into()]));
    assert_eq!(
        out[0].opts.len(),
        1,
        "projected var from opt → OPTIONAL kept"
    );
}

/// **GREEN** — `testDistinctNoPrune2`: same branch but WITHOUT DISTINCT context
/// → pass 2d does not fire; OPTIONAL retained.
#[test]
fn distinct_no_prune_without_distinct() {
    let mut b = Branch::single(scan(0, "TABLE1"));
    b.opts.push(OptJoin {
        scan: scan(1, "TABLE2"),
        on: vec![],
        extra: vec![],
    });
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("N".into(), col_binding(1, "col2")); // NOT projected but no DISTINCT
                                                           // Use a project list but no DISTINCT — pass 2d must not fire.
    let ctx = CascadeCtx {
        distinct: false,
        project: Some(&["M".into()]),
    };
    let out = run(vec![b], &prune_schema(), &ctx);
    assert_eq!(out[0].opts.len(), 1, "no DISTINCT → pass 2d must not fire");
}

/// **GREEN** — `testDistinctNoPrune3`: SELECT * (no project list) — pass 2d
/// requires a known project list and must not fire when `project = None`.
#[test]
fn distinct_no_prune_no_project_list() {
    let mut b = Branch::single(scan(0, "TABLE1"));
    b.opts.push(OptJoin {
        scan: scan(1, "TABLE2"),
        on: vec![],
        extra: vec![],
    });
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("N".into(), col_binding(1, "col2"));
    let ctx = CascadeCtx {
        distinct: true,
        project: None,
    }; // SELECT DISTINCT * analog
    let out = run(vec![b], &prune_schema(), &ctx);
    assert_eq!(out[0].opts.len(), 1, "project=None → pass 2d must not fire");
}
