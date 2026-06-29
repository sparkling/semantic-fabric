//! Ontop-parity oracle port — batch 5 of 8 (ADR-0021 / ADR-0022).
//!
//! Assigned slice = sorted `*Test.java` indices `[25, 30)` over the combined
//! `~/source/ontop/core/optimization/src/test/java/.../iq/{executor,optimizer}/`
//! listing (tag `ontop-5.5.0`):
//!
//!   25  PushDownBooleanExpressionOptimizerTest   — BOUNDARY (see notes below)
//!   26  PushUpBooleanExpressionOptimizerTest     — BOUNDARY (see notes below)
//!   27  SelfJoinSameTermsTest                    — SUPPORTED (cascade pass 2c)
//!   28  TrueNodesRemovalOptimizerTest            — BOUNDARY (no TrueNode in sf IR)
//!   29  UniqueConstraintInferenceTest            — SUPPORTED (pass 6 proxy) / NEEDS_IMPL
//!
//! Only the SUPPORTED scenarios that are *faithfully expressible* against sf's
//! flat-join IR (`iq.rs`) + cascade (`cascade::run`) are ported here as runnable
//! oracle tests. Each builds an input `Branch`, runs the real cascade with a real
//! schema, and asserts the optimized branch matches Ontop's scenario.
//!
//! Why 25/26/28 are BOUNDARY (not ported):
//!   * 25 (PushDown) / 26 (PushUp) operate on Ontop's *tree* of InnerJoin /
//!     LeftJoin / Filter nodes with implicit-join-by-shared-variable semantics,
//!     moving boolean conditions between tree levels. sf flattens every inner
//!     join into ONE `Branch` (core scans + a flat `where_conds`) and delegates
//!     physical predicate placement to the source DB (ADR-0006: "the source DB
//!     does the set-work"). The normalized flat form IS sf's base translation by
//!     construction — there is no nested join node to push a condition into/out
//!     of, hence no before/after oracle to assert. sf's only analog (pass 5
//!     `selection_pushdown`) is a weaker flat stable-partition. The left-join /
//!     union scenarios additionally need multi-scan LEFT JOIN right sides and
//!     UNION-node construction that sf's `OptJoin` (single-scan) does not model.
//!   * 28 (TrueNodesRemoval): a `TrueNode` is an Ontop IQ-tree artifact (an
//!     arity-0 unit relation produced by substitution). sf's flat IR has no such
//!     node — the join identity is an absorbed empty `Branch`, never an explicit
//!     leaf — so there is nothing to "remove" and no oracle to assert.
#![cfg(test)]

use std::collections::BTreeMap;

use sf_core::ir::{LogicalSource, Segment, Template, TermMap, TermSpec};
use sf_sparql::cascade::{run, CascadeCtx};
use sf_sparql::iq::{Branch, CmpOp, ColRef, Scan, SqlCond, TermDef};
use sf_sql::{Column, TableSchema};

// --- shared helpers --------------------------------------------------------

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

/// A multi-column IRI template binding. `sep` is the literal placed *between*
/// the two column placeholders — `Some("/")` is injective (Ontop's
/// `URI_TEMPLATE_INJECTIVE_2`), `None`/`Some("")` is non-injective (Ontop's
/// `URI_TEMPLATE_NOT_INJECTIVE_2`, two adjacent placeholders).
fn iri_template2(alias: usize, prefix: &str, c1: &str, sep: Option<&str>, c2: &str) -> TermDef {
    let mut segs = vec![Segment::Literal(prefix.into()), Segment::Column(c1.into())];
    if let Some(s) = sep {
        if !s.is_empty() {
            segs.push(Segment::Literal(s.into()));
        }
    }
    segs.push(Segment::Column(c2.into()));
    TermDef::Derived {
        term_map: TermMap::Template(Template::from_segments(segs).unwrap(), TermSpec::iri()),
        alias,
    }
}

/// Build a fresh `Branch` over the given core scans with the given WHERE conds.
fn branch(core: Vec<Scan>, where_conds: Vec<SqlCond>, distinct: bool) -> Branch {
    Branch {
        core,
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds,
        distinct,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
    }
}

fn ctx<'a>(distinct: bool, project: &'a [String]) -> CascadeCtx<'a> {
    CascadeCtx {
        distinct,
        project: Some(project),
    }
}

fn count_nn_guards(b: &Branch, col: &str) -> usize {
    b.where_conds
        .iter()
        .filter(|c| matches!(c, SqlCond::IsNotNull(r) if r.column.as_ref() == col))
        .count()
}

// ===========================================================================
// CLASS 27 — SelfJoinSameTermsTest  (SUPPORTED — cascade pass 2c)
//
// Schema `T1_AR3`: a 3-column table, all columns nullable, NO key. Ontop's
// JOIN_LIKE_OPTIMIZER eliminates redundant same-table scans whose join columns
// cover the projection *under DISTINCT*, leaving the most-constrained scan with
// `IS NOT NULL` guards on the variable shared columns. This is exactly sf's
// `same_terms_elimination` (pass 2c, fires only when `ctx.distinct`).
// ===========================================================================

fn t1_ar3_schema() -> Vec<TableSchema> {
    let mut t = TableSchema::new("T1_AR3");
    t.columns = vec![
        Column::new("col1", "text", false),
        Column::new("col2", "text", false),
        Column::new("col3", "text", false),
    ];
    vec![t]
}

/// **GREEN** — Ontop `testSelfJoinElimination1`: two T1_AR3 scans share col2(=B)
/// and col3(=C) under DISTINCT, col1 not projected ⇒ 1 scan, IS NOT NULL on
/// col2 + col3.
#[test]
fn st_elimination1_two_scans_two_shared_cols() {
    let mut b = branch(
        vec![scan(0, "T1_AR3"), scan(1, "T1_AR3")],
        vec![
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col2")),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(1, "col3")),
        ],
        true,
    );
    b.bindings.insert("B".into(), col_binding(0, "col2"));
    b.bindings.insert("C".into(), col_binding(0, "col3"));

    let out = run(
        vec![b],
        &t1_ar3_schema(),
        &ctx(true, &["B".into(), "C".into()]),
    );
    let b = &out[0];
    assert_eq!(b.core.len(), 1, "second T1_AR3 scan eliminated");
    assert_eq!(count_nn_guards(b, "col2"), 1, "IS NOT NULL on col2");
    assert_eq!(count_nn_guards(b, "col3"), 1, "IS NOT NULL on col3");
    assert_eq!(
        count_nn_guards(b, "col1"),
        0,
        "col1 not projected ⇒ no guard"
    );
}

/// **GREEN** — Ontop `testSelfJoinElimination3`: three scans all share col2/col3
/// under DISTINCT ⇒ collapse to one scan, IS NOT NULL on col2 + col3.
#[test]
fn st_elimination3_three_scans_collapse() {
    let mut b = branch(
        vec![scan(0, "T1_AR3"), scan(1, "T1_AR3"), scan(2, "T1_AR3")],
        vec![
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col2")),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(1, "col3")),
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(2, "col2")),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(2, "col3")),
        ],
        true,
    );
    b.bindings.insert("B".into(), col_binding(0, "col2"));
    b.bindings.insert("C".into(), col_binding(0, "col3"));

    let out = run(
        vec![b],
        &t1_ar3_schema(),
        &ctx(true, &["B".into(), "C".into()]),
    );
    assert_eq!(out[0].core.len(), 1, "all three scans collapse to one");
    assert_eq!(count_nn_guards(&out[0], "col2"), 1);
    assert_eq!(count_nn_guards(&out[0], "col3"), 1);
}

/// **GREEN** — Ontop `testSelfJoinElimination4`: three scans all have a *constant*
/// col2="plop", shared variable col3(=C). Under DISTINCT ⇒ 1 scan; the constant
/// Cmp survives; IS NOT NULL only on col3 (a constant equality already implies
/// non-null, so no guard on col2).
#[test]
fn st_elimination4_constant_shared_col_no_guard() {
    let mut b = branch(
        vec![scan(0, "T1_AR3"), scan(1, "T1_AR3"), scan(2, "T1_AR3")],
        vec![
            SqlCond::Cmp(ColRef::new(0, "col2"), CmpOp::Eq, "plop".into()),
            SqlCond::Cmp(ColRef::new(1, "col2"), CmpOp::Eq, "plop".into()),
            SqlCond::Cmp(ColRef::new(2, "col2"), CmpOp::Eq, "plop".into()),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(1, "col3")),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(2, "col3")),
        ],
        true,
    );
    b.bindings.insert("C".into(), col_binding(0, "col3"));

    let out = run(vec![b], &t1_ar3_schema(), &ctx(true, &["C".into()]));
    let b = &out[0];
    assert_eq!(b.core.len(), 1, "three scans collapse to one");
    assert_eq!(count_nn_guards(b, "col3"), 1, "IS NOT NULL on col3");
    assert_eq!(count_nn_guards(b, "col2"), 0, "constant col2 ⇒ no guard");
    assert!(
        b.where_conds.iter().any(|c| matches!(
            c, SqlCond::Cmp(r, CmpOp::Eq, v) if r.column.as_ref() == "col2" && &**v == "plop")),
        "Cmp(col2='plop') must remain on the surviving scan"
    );
}

/// **GREEN** — Ontop `testSelfJoinElimination5`: two scans have constant
/// col2="plop", one has a variable col2; all share col3(=C). Under DISTINCT all
/// three collapse ⇒ 1 scan, IS NOT NULL on col3 only.
#[test]
fn st_elimination5_mixed_constant_and_variable() {
    let mut b = branch(
        vec![scan(0, "T1_AR3"), scan(1, "T1_AR3"), scan(2, "T1_AR3")],
        vec![
            SqlCond::Cmp(ColRef::new(0, "col2"), CmpOp::Eq, "plop".into()),
            SqlCond::Cmp(ColRef::new(1, "col2"), CmpOp::Eq, "plop".into()),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(1, "col3")),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(2, "col3")),
        ],
        true,
    );
    b.bindings.insert("C".into(), col_binding(0, "col3"));

    let out = run(vec![b], &t1_ar3_schema(), &ctx(true, &["C".into()]));
    assert_eq!(out[0].core.len(), 1, "all three scans collapse to one");
    assert_eq!(count_nn_guards(&out[0], "col3"), 1);
    assert_eq!(count_nn_guards(&out[0], "col2"), 0);
}

/// **GREEN** — Ontop `testSelfJoinElimination6`: two scans share col2/col3 and
/// col1(=A) is ALSO projected from the kept scan; scan1's cols are covered by
/// ColEqs ⇒ scan1 eliminated, IS NOT NULL on col2 + col3.
#[test]
fn st_elimination6_kept_col1_also_projected() {
    let mut b = branch(
        vec![scan(0, "T1_AR3"), scan(1, "T1_AR3")],
        vec![
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col2")),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(1, "col3")),
        ],
        true,
    );
    b.bindings.insert("A".into(), col_binding(0, "col1"));
    b.bindings.insert("B".into(), col_binding(0, "col2"));
    b.bindings.insert("C".into(), col_binding(0, "col3"));

    let out = run(
        vec![b],
        &t1_ar3_schema(),
        &ctx(true, &["A".into(), "B".into(), "C".into()]),
    );
    assert_eq!(
        out[0].core.len(),
        1,
        "scan1 eliminated (cols covered by ColEqs)"
    );
    assert_eq!(count_nn_guards(&out[0], "col2"), 1);
    assert_eq!(count_nn_guards(&out[0], "col3"), 1);
}

/// **GREEN** — Ontop `testSelfJoinElimination8`: the SAME scan joined with itself
/// on all three columns, all projected ⇒ collapse, IS NOT NULL on col1/col2/col3.
#[test]
fn st_elimination8_self_join_all_cols_projected() {
    let mut b = branch(
        vec![scan(0, "T1_AR3"), scan(1, "T1_AR3")],
        vec![
            SqlCond::ColEq(ColRef::new(0, "col1"), ColRef::new(1, "col1")),
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col2")),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(1, "col3")),
        ],
        true,
    );
    b.bindings.insert("A".into(), col_binding(0, "col1"));
    b.bindings.insert("B".into(), col_binding(0, "col2"));
    b.bindings.insert("C".into(), col_binding(0, "col3"));

    let out = run(
        vec![b],
        &t1_ar3_schema(),
        &ctx(true, &["A".into(), "B".into(), "C".into()]),
    );
    assert_eq!(out[0].core.len(), 1, "self-join collapses to one scan");
    assert_eq!(count_nn_guards(&out[0], "col1"), 1);
    assert_eq!(count_nn_guards(&out[0], "col2"), 1);
    assert_eq!(count_nn_guards(&out[0], "col3"), 1);
}

/// **GREEN** — Ontop `testSelfJoinNonElimination1`: same-table scans but DISTINCT
/// is NOT set ⇒ pass 2c is a no-op ⇒ both scans remain.
#[test]
fn st_non_elimination1_no_distinct() {
    let mut b = branch(
        vec![scan(0, "T1_AR3"), scan(1, "T1_AR3")],
        vec![
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col2")),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(1, "col3")),
        ],
        false,
    );
    b.bindings.insert("B".into(), col_binding(0, "col2"));
    b.bindings.insert("C".into(), col_binding(0, "col3"));

    let out = run(
        vec![b],
        &t1_ar3_schema(),
        &ctx(false, &["B".into(), "C".into()]),
    );
    assert_eq!(
        out[0].core.len(),
        2,
        "no DISTINCT ⇒ pass 2c no-op ⇒ 2 scans"
    );
}

/// **GREEN** — Ontop `testSelfJoinNonElimination2bis`: minimal scans, no
/// cross-scan ColEq (B from scan0.col2, C from scan1.col3) under DISTINCT ⇒ no
/// shared join column ⇒ no elimination ⇒ 2 scans remain.
#[test]
fn st_non_elimination2bis_no_shared_join_col() {
    let mut b = branch(vec![scan(0, "T1_AR3"), scan(1, "T1_AR3")], Vec::new(), true);
    b.bindings.insert("B".into(), col_binding(0, "col2"));
    b.bindings.insert("C".into(), col_binding(1, "col3"));

    let out = run(
        vec![b],
        &t1_ar3_schema(),
        &ctx(true, &["B".into(), "C".into()]),
    );
    assert_eq!(out[0].core.len(), 2, "no cross-scan ColEq ⇒ 2 scans remain");
}

/// **GREEN** — Ontop `testSelfJoinNonElimination3`: scans share col1(=A, not
/// projected) but the projected C(=scan1.col3) is not covered by any ColEq ⇒
/// elimination blocked ⇒ 2 scans remain.
#[test]
fn st_non_elimination3_projected_col_uncovered() {
    let mut b = branch(
        vec![scan(0, "T1_AR3"), scan(1, "T1_AR3")],
        vec![SqlCond::ColEq(
            ColRef::new(0, "col1"),
            ColRef::new(1, "col1"),
        )],
        true,
    );
    b.bindings.insert("B".into(), col_binding(0, "col2"));
    b.bindings.insert("C".into(), col_binding(1, "col3"));

    let out = run(
        vec![b],
        &t1_ar3_schema(),
        &ctx(true, &["B".into(), "C".into()]),
    );
    assert_eq!(
        out[0].core.len(),
        2,
        "scan1.col3 uncovered ⇒ blocked ⇒ 2 scans"
    );
}

/// **GREEN** — Ontop `testSelfJoinNonElimination4`: scans share col2/col3 via
/// ColEqs but col1 carries DIFFERENT constants ("cst1" vs "cst2") ⇒ the
/// local-condition equivalence check fails ⇒ NO elimination.
#[test]
fn st_non_elimination4_different_constants() {
    let mut b = branch(
        vec![scan(0, "T1_AR3"), scan(1, "T1_AR3")],
        vec![
            SqlCond::Cmp(ColRef::new(0, "col1"), CmpOp::Eq, "cst1".into()),
            SqlCond::Cmp(ColRef::new(1, "col1"), CmpOp::Eq, "cst2".into()),
            SqlCond::ColEq(ColRef::new(0, "col2"), ColRef::new(1, "col2")),
            SqlCond::ColEq(ColRef::new(0, "col3"), ColRef::new(1, "col3")),
        ],
        true,
    );
    b.bindings.insert("B".into(), col_binding(0, "col2"));
    b.bindings.insert("C".into(), col_binding(0, "col3"));

    let out = run(
        vec![b],
        &t1_ar3_schema(),
        &ctx(true, &["B".into(), "C".into()]),
    );
    assert_eq!(
        out[0].core.len(),
        2,
        "different col1 constants ⇒ no elimination"
    );
}

// ===========================================================================
// CLASS 29 — UniqueConstraintInferenceTest
//
// Ontop's `tree.inferUniqueConstraints()` returns the SET of output-variable
// sets that are unique. sf has no such variable-set API, but the *use* of that
// fact is pass 6 (distinct-removal): a single-scan DISTINCT is redundant iff a
// projected term is built from a non-null key. So the behavioural mapping is
//   Ontop {{X}}  ⟺  sf removes the DISTINCT  (out.distinct == false)
//   Ontop {}     ⟺  sf keeps   the DISTINCT  (out.distinct == true)
// which faithfully ports the *single-column-PK* construction scenarios. The
// composite-key / union / values scenarios need inference sf does not have
// (NEEDS_IMPL) — they are NOT asserted here.
//
// `pk_ar2`: arity-2, single-column PK on col1 (Ontop `PK_TABLE1_AR2`,
//           `createRelationWithPK` puts the PK on attribute 1).
// `pk_ar3`: arity-3, single-column PK on col1 (Ontop `PK_TABLE1_AR3`).
// ===========================================================================

fn pk_ar2_schema() -> Vec<TableSchema> {
    let mut t = TableSchema::new("pk_ar2");
    t.primary_key = vec!["col1".into()];
    t.columns = vec![
        Column::new("col1", "text", true), // PK ⇒ NOT NULL
        Column::new("col2", "text", true),
    ];
    vec![t]
}

fn pk_ar3_schema() -> Vec<TableSchema> {
    let mut t = TableSchema::new("pk_ar3");
    t.primary_key = vec!["col1".into()];
    t.columns = vec![
        Column::new("col1", "text", true), // PK ⇒ NOT NULL
        Column::new("col2", "text", true),
        Column::new("col3", "text", true),
    ];
    vec![t]
}

/// **GREEN** — Ontop `testConstructionInjectiveTemplate1` ⇒ `{{X}}`. An injective
/// 2-placeholder template over (col1=PK, col2); X reads the PK ⇒ X is unique ⇒
/// the DISTINCT is provably redundant ⇒ sf removes it.
#[test]
fn uc_construction_injective_template1_distinct_removed() {
    let mut b = branch(vec![scan(0, "pk_ar2")], Vec::new(), true);
    b.bindings.insert(
        "X".into(),
        iri_template2(0, "http://example.org/ds1/", "col1", Some("/"), "col2"),
    );

    let out = run(vec![b], &pk_ar2_schema(), &ctx(true, &["X".into()]));
    assert!(
        !out[0].distinct,
        "X built from the PK ⇒ unique ⇒ DISTINCT removed (Ontop {{{{X}}}})"
    );
}

/// **GREEN** — Ontop `testConstructionInjectiveTemplate2` ⇒ `{}`. The injective
/// template is over (col2, col3) of the arity-3 PK table — it does NOT include
/// the PK (col1), so X is not provably unique ⇒ sf keeps the DISTINCT.
#[test]
fn uc_construction_template_over_nonkey_cols_distinct_kept() {
    let mut b = branch(vec![scan(0, "pk_ar3")], Vec::new(), true);
    b.bindings.insert(
        "X".into(),
        iri_template2(0, "http://example.org/ds1/", "col2", Some("/"), "col3"),
    );

    let out = run(vec![b], &pk_ar3_schema(), &ctx(true, &["X".into()]));
    assert!(
        out[0].distinct,
        "X built from non-key cols ⇒ not unique ⇒ DISTINCT kept (Ontop {{}})"
    );
}

/// **GREEN** — Ontop `testDuplicateColumn1` ⇒ `{{X},{A}}`. X is a plain alias of
/// the PK column (X = A = col1). X reads the PK ⇒ unique ⇒ DISTINCT removed.
#[test]
fn uc_duplicate_column1_alias_of_pk_distinct_removed() {
    let mut b = branch(vec![scan(0, "pk_ar2")], Vec::new(), true);
    b.bindings.insert("X".into(), col_binding(0, "col1"));
    b.bindings.insert("A".into(), col_binding(0, "col1"));

    let out = run(
        vec![b],
        &pk_ar2_schema(),
        &ctx(true, &["X".into(), "A".into()]),
    );
    assert!(
        !out[0].distinct,
        "X = A = PK col ⇒ unique ⇒ DISTINCT removed (Ontop {{{{X}},{{A}}}})"
    );
}

/// **GREEN** — Ontop `testDuplicateColumn4` ⇒ `{{X},{Y},{A}}`. Two output
/// variables both alias the single PK column (X = Y = A = col1) ⇒ each is unique
/// ⇒ DISTINCT removed.
#[test]
fn uc_duplicate_column4_two_aliases_of_pk_distinct_removed() {
    let mut b = branch(vec![scan(0, "pk_ar2")], Vec::new(), true);
    b.bindings.insert("X".into(), col_binding(0, "col1"));
    b.bindings.insert("Y".into(), col_binding(0, "col1"));

    let out = run(
        vec![b],
        &pk_ar2_schema(),
        &ctx(true, &["X".into(), "Y".into()]),
    );
    assert!(
        !out[0].distinct,
        "X = Y = PK col ⇒ unique ⇒ DISTINCT removed (Ontop {{{{X}},{{Y}},{{A}}}})"
    );
}

/// **RED — real parity bug (kept `#[ignore]` to hold the suite green; run with
/// `--ignored`).** Ontop `testConstructionNonInjectiveTemplate1` ⇒ `{}`: the
/// template `ds2/{col1}{col2}` has two *adjacent* placeholders (no separator) and
/// is therefore NON-injective — distinct rows can map to the same IRI, so X is
/// NOT a unique constraint and the DISTINCT must be preserved.
///
/// sf-core's term maps carry NO injectivity flag (verified: no `injective` field
/// on `TermMap`/`TermSpec`), and pass 6 (`distinct_removal`) only checks that a
/// projected term *reads* a key column — it does not check template injectivity.
/// So sf REMOVES the DISTINCT here, diverging from Ontop and from `=_bag` vs the
/// DISTINCT semantics (two rows whose `(col1,col2)` concatenate equally would be
/// emitted twice instead of collapsed).
///
/// Expected (Ontop): DISTINCT kept (`out.distinct == true`).
/// Got (sf):         DISTINCT removed (`out.distinct == false`).
#[test]
#[ignore = "RED: sf pass-6 distinct-removal ignores IRI-template injectivity \
            (no injectivity flag in sf-core); removes a DISTINCT Ontop preserves \
            for a non-injective (separator-less) template — latent =_bag gap"]
fn uc_construction_non_injective_template1_red() {
    let mut b = branch(vec![scan(0, "pk_ar2")], Vec::new(), true);
    // Non-injective: "ds2/" + col1 + col2 with NO separator between placeholders.
    b.bindings.insert(
        "X".into(),
        iri_template2(0, "http://example.org/ds2/", "col1", None, "col2"),
    );

    let out = run(vec![b], &pk_ar2_schema(), &ctx(true, &["X".into()]));
    assert!(
        out[0].distinct,
        "Ontop keeps the DISTINCT (non-injective template ⇒ X not unique); \
         sf removes it — RED parity divergence"
    );
}
