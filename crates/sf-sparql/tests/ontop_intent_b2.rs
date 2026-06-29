//! Ontop-parity intent file — batch 2 of 8 (ADR-0022); NEEDS_IMPL / RED-SPEC extensions.
//!
//! This file adds RED-SPEC (`#[ignore]`d) and GREEN guard tests that complement the
//! GREEN port in `ontop_port_b2.rs`.  Every test here either:
//!
//! * asserts the *desired post-implementation* outcome of a NEEDS_IMPL scenario
//!   (marked `#[ignore]`), or
//! * acts as a negative guard whose current behaviour is already correct (GREEN).
//!
//! ## Covered scenarios
//!
//! ### DistinctTest — multi-scan DISTINCT-over-join removal (NEEDS_IMPL)
//!
//! `testDistinctJoin2` and `testDistinctJoin5` extend the `testDistinctJoin1` RED-SPEC
//! already in the port file.  sf's pass 6 bails on any multi-scan core
//! (`core.len() != 1`), so it cannot prove a join's output is key-unique even when
//! every joined table contributes a projected primary key.
//!
//! * `ontop_distinct_over_join_all_keys_two_tables` — RED-SPEC.  Two PK tables; both
//!   PKs projected.  Ontop removes DISTINCT; sf keeps it.  Desired: `distinct == false`.
//!
//! * `ontop_distinct_over_join_partial_key_kept` — GREEN guard (negative direction).
//!   Two PK tables joined, but only one table's PK is projected alongside a non-key
//!   column.  Duplicates are semantically possible; DISTINCT is required.  sf preserves
//!   it (multi-scan bail coincides with the correct outcome).
//!
//! ### ConjunctionOfDisjunctionsMergingTest — boolean filter simplification (NEEDS_IMPL)
//!
//! Both specs require CNF/DNF intersection reasoning that sf's pass 5
//! (`selection_pushdown`) does not perform.  Pass 5 only flattens a top-level `AND`
//! and stable-partitions single-scan selections; it has no disjunction-intersection,
//! absorption, or unsatisfiability detection.
//!
//! * `conjunction_disjunction_intersection_simplification` — RED-SPEC.
//!   Three conjuncts, two of which constrain the same column with overlapping
//!   disjunctions.  Their intersection is the singleton {X}, collapsing to a point
//!   equality.  Desired: `where_conds.len() == 2` (point eq + the B disjunction).
//!
//! * `conjunction_disjunction_empty_intersection` — RED-SPEC.
//!   Two conjuncts constraining the same column with value-disjoint disjunctions.
//!   Their intersection is ∅ → the branch is unsatisfiable.  Desired: `out.is_empty()`.
//!
//! ## Boundary summary (no test functions — classes are out of cascade scope)
//!
//! * **ConstructionNodeCleanerTest** (12) — ALL BOUNDARY.  Each of the 12 scenarios
//!   merges or relocates consecutive `ConstructionNode`s over `IntensionalDataNode`s,
//!   lifting substitutions through `SliceNode`, `DistinctNode`, and `UnionNode`.  sf
//!   folds all term construction into `Branch::bindings` at unfold time — no stacked
//!   construction nodes exist in the cascade IR and no intensional atoms remain.  There
//!   is no cascade-level analogue.
//!
//! * **ExpressionEvaluatorTest** (14 BOUNDARY, 1 NEEDS_IMPL) — The 14 BOUNDARY are
//!   direct unit tests of Ontop's `ImmutableExpression.evaluate(...)` sub-component
//!   (`IS NOT NULL uri2(X,Y)` ⇒ `IS NOT NULL X AND IS NOT NULL Y`, `IS NOT NULL
//!   uri1("toto")` ⇒ TRUE, `IfElseNull` evaluation); sf has no standalone expression
//!   evaluator API — null-rejection over IRI templates is folded into the translation.
//!   The 1 NEEDS_IMPL (`testNonEqualOperatorDistribution`: `NEQ(uri2(A,B),uri2(C,D))
//!   ⇒ OR(A≠C,B≠D)`) requires IRI-template injectivity reasoning, a cascade-level
//!   rewrite not expressible as a `Branch`-level test without the feature.
//!
//! * **FlattenLiftTest** (16) — ALL BOUNDARY.  Every scenario lifts a `FlattenNode`
//!   (JSON-array UNNEST / lateral flatten) above a join, left-join, or construction
//!   node, or splits a join condition around a flatten.  sf has no `FlattenNode` in
//!   its IR; out of charter (ADR-0004).

use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_sparql::cascade::{run, CascadeCtx};
use sf_sparql::iq::{Branch, CmpOp, ColRef, Scan, SqlCond, TermDef};
use sf_sql::{Column, TableSchema};

fn scan(alias: usize, table: &str) -> Scan {
    Scan {
        alias,
        source: LogicalSource::Table(table.to_owned()),
    }
}

/// A plain-literal `rr:column` binding reading `col` of `alias`.
fn col_binding(alias: usize, col: &str) -> TermDef {
    TermDef::Derived {
        term_map: TermMap::Column(col.into(), TermSpec::plain_literal()),
        alias,
    }
}

/// Two 2-column PK tables, PK on `c0`, both columns NOT NULL.
/// Models Ontop's `PK_TABLE1_AR2` and `PK_TABLE2_AR2` used in
/// `DistinctTest.testDistinctJoin2` and `testDistinctJoin5`.
fn two_pk_tables() -> Vec<TableSchema> {
    let mk = |name: &str| {
        let mut t = TableSchema::new(name);
        t.primary_key = vec!["c0".into()];
        t.columns = vec![
            Column::new("c0", "text", true), // PK — NOT NULL, unique key
            Column::new("c1", "text", true), // NOT NULL, non-key
        ];
        t
    };
    vec![mk("pk_t0"), mk("pk_t1")]
}

/// One table with two nullable columns `a` and `b`.  No PK needed — used for
/// boolean-filter simplification tests where constraint passes are no-ops.
fn one_table_ab() -> Vec<TableSchema> {
    let mut t = TableSchema::new("t");
    t.columns = vec![
        Column::new("a", "text", false),
        Column::new("b", "text", false),
    ];
    vec![t]
}

// ── DistinctTest.testDistinctJoin2 ──────────────────────────────────────────

/// **NEEDS_IMPL spec (RED, `#[ignore]`d).** Ontop `DistinctTest.testDistinctJoin2`.
///
/// `DISTINCT` over the cross product of *two* PK tables, projecting both PKs:
/// `A := t0.c0` (PK of t0) and `B := t1.c0` (PK of t1).  Every output tuple is
/// uniquely identified by the pair `(t0.c0, t1.c0)` — no duplicates are possible.
/// Ontop's DISTINCT-removal pass detects this via FD-closure over the join and
/// removes the `DISTINCT`.  sf's pass 6 bails on any multi-scan core
/// (`core.len() != 1`), so it keeps the `DISTINCT`.
///
/// Asserts the DESIRED post-impl state (`distinct == false`) and is `#[ignore]`d
/// (RED) until multi-scan FD-closure DISTINCT removal lands.
/// Run with `cargo test -- --ignored`.
#[test]
fn ontop_distinct_over_join_all_keys_two_tables() {
    let mut b = Branch::single(scan(0, "pk_t0"));
    b.core.push(scan(1, "pk_t1"));
    b.bindings.insert("A".into(), col_binding(0, "c0")); // PK of t0
    b.bindings.insert("B".into(), col_binding(1, "c0")); // PK of t1

    let ctx = CascadeCtx {
        distinct: true,
        project: Some(&["A".to_owned(), "B".to_owned()]),
    };
    let out = run(vec![b], &two_pk_tables(), &ctx);
    assert_eq!(out.len(), 1);
    assert!(
        !out[0].distinct,
        "DISTINCT is redundant: both projected terms are PKs of their respective tables \
         ⇒ every output row is unique (DESIRED multi-scan removal — DistinctTest.testDistinctJoin2)"
    );
}

// ── DistinctTest.testDistinctJoin5 (negative guard) ─────────────────────────

/// **GREEN (negative guard).** Ontop `DistinctTest.testDistinctJoin5`.
///
/// `DISTINCT` over a join of two PK tables, projecting `A := t0.c0` (PK of t0)
/// and `B := t0.c1` (non-key of t0).  The non-key column `c1` can carry duplicate
/// values; `t1`'s PK is never projected.  Ontop keeps the `DISTINCT` — the join
/// does not make it redundant.  sf also keeps it: pass 6 bails on multi-scan cores.
///
/// This is the negative-direction complement of the RED-SPEC above.  It verifies
/// that sf's multi-scan bail does NOT accidentally remove a semantically necessary
/// `DISTINCT`.
#[test]
fn ontop_distinct_over_join_partial_key_kept() {
    let mut b = Branch::single(scan(0, "pk_t0"));
    b.core.push(scan(1, "pk_t1"));
    b.bindings.insert("A".into(), col_binding(0, "c0")); // PK of t0 (projected)
    b.bindings.insert("B".into(), col_binding(0, "c1")); // non-key of t0 (projected)
                                                         // t1.c0 (PK of t1) is NOT projected — its uniqueness cannot anchor the output

    let ctx = CascadeCtx {
        distinct: true,
        project: Some(&["A".to_owned(), "B".to_owned()]),
    };
    let out = run(vec![b], &two_pk_tables(), &ctx);
    assert_eq!(out.len(), 1);
    assert!(
        out[0].distinct,
        "DISTINCT must be PRESERVED: t0.c1 is a non-key column and t1's PK is not \
         projected — duplicates are possible, so DISTINCT is semantically required \
         (DistinctTest.testDistinctJoin5 negative guard)"
    );
}

// ── ConjunctionOfDisjunctionsMergingTest.mergingTest1 ───────────────────────

/// **NEEDS_IMPL spec (RED, `#[ignore]`d).**
/// Ontop `ConjunctionOfDisjunctionsMergingTest.mergingTest1`.
///
/// A conjunction of three disjunctions on two columns:
///
/// ```text
/// (a = 'X' OR a = 'Y' OR a = 'Z')
/// AND (b = 'V' OR b = 'W')
/// AND (a = 'W' OR a = 'X')
/// ```
///
/// The two a-disjunctions share the intersection {X,Y,Z} ∩ {W,X} = {X}.
/// The conjunction therefore simplifies to:
///
/// ```text
/// a = 'X'
/// AND (b = 'V' OR b = 'W')
/// ```
///
/// sf's pass 5 (`selection_pushdown`) only flattens top-level `AND` nodes and
/// stable-partitions single-scan selections; it performs no disjunction-intersection,
/// CNF/DNF merge, or absorption reasoning.  After the cascade the three conditions
/// remain in `where_conds` unchanged (len == 3, not 2).
/// The assertion `where_conds.len() == 2` fires the RED spec.
#[test]
fn conjunction_disjunction_intersection_simplification() {
    let mut b = Branch::single(scan(0, "t"));

    // (a = 'X' OR a = 'Y' OR a = 'Z')
    b.where_conds.push(SqlCond::Or(vec![
        SqlCond::Cmp(ColRef::new(0, "a"), CmpOp::Eq, "X".to_owned()),
        SqlCond::Cmp(ColRef::new(0, "a"), CmpOp::Eq, "Y".to_owned()),
        SqlCond::Cmp(ColRef::new(0, "a"), CmpOp::Eq, "Z".to_owned()),
    ]));
    // (b = 'V' OR b = 'W')
    b.where_conds.push(SqlCond::Or(vec![
        SqlCond::Cmp(ColRef::new(0, "b"), CmpOp::Eq, "V".to_owned()),
        SqlCond::Cmp(ColRef::new(0, "b"), CmpOp::Eq, "W".to_owned()),
    ]));
    // (a = 'W' OR a = 'X')  -- intersects the first a-disjunction => {X}
    b.where_conds.push(SqlCond::Or(vec![
        SqlCond::Cmp(ColRef::new(0, "a"), CmpOp::Eq, "W".to_owned()),
        SqlCond::Cmp(ColRef::new(0, "a"), CmpOp::Eq, "X".to_owned()),
    ]));

    let ctx = CascadeCtx {
        distinct: false,
        project: None,
    };
    let out = run(vec![b], &one_table_ab(), &ctx);
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].where_conds.len(),
        2,
        "DESIRED: disjunction intersection collapses three conjuncts to two \
         (a='X' plus the b-disjunction) — CoDM.mergingTest1"
    );
}

// ── ConjunctionOfDisjunctionsMergingTest.mergingTest2 ───────────────────────

/// **NEEDS_IMPL spec (RED, `#[ignore]`d).**
/// Ontop `ConjunctionOfDisjunctionsMergingTest.mergingTest2`.
///
/// A conjunction of two value-disjoint disjunctions on the same column:
///
/// ```text
/// (a = 'X' OR a = 'Y')
/// AND (a = 'Z' OR a = 'W')
/// ```
///
/// The intersection {X,Y} ∩ {Z,W} = ∅ — the conjunction is unsatisfiable.
/// Ontop collapses the branch to an `EmptyNode` (no rows produced).
///
/// sf's pass 5 detects two *top-level point equalities* on the same column that
/// disagree (`Cmp(col, Eq, "X") ∧ Cmp(col, Eq, "Y")` => prune), but the
/// constraints here are `Or(...)` nodes, not top-level equalities.  Pass 5 cannot
/// detect disjunction-level emptiness, so the branch survives unchanged
/// (`out.len() == 1`, not 0).  The assertion `out.is_empty()` fires the RED spec.
#[test]
fn conjunction_disjunction_empty_intersection() {
    let mut b = Branch::single(scan(0, "t"));

    // (a = 'X' OR a = 'Y')
    b.where_conds.push(SqlCond::Or(vec![
        SqlCond::Cmp(ColRef::new(0, "a"), CmpOp::Eq, "X".to_owned()),
        SqlCond::Cmp(ColRef::new(0, "a"), CmpOp::Eq, "Y".to_owned()),
    ]));
    // (a = 'Z' OR a = 'W')  -- value-disjoint from the first => empty intersection
    b.where_conds.push(SqlCond::Or(vec![
        SqlCond::Cmp(ColRef::new(0, "a"), CmpOp::Eq, "Z".to_owned()),
        SqlCond::Cmp(ColRef::new(0, "a"), CmpOp::Eq, "W".to_owned()),
    ]));

    let ctx = CascadeCtx {
        distinct: false,
        project: None,
    };
    let out = run(vec![b], &one_table_ab(), &ctx);
    assert!(
        out.is_empty(),
        "DESIRED: value-disjoint disjunctions on the same column are unsatisfiable \
         ⇒ branch collapses to empty result (CoDM.mergingTest2)"
    );
}
