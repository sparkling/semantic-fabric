//! Ontop-parity oracle port — batch 0 of 8 (ADR-0022).
//!
//! Assigned slice `[0, 5)` over the combined, path-sorted `*Test.java` listing of
//! `~/source/ontop/core/optimization/src/test/java/it/unibz/inf/ontop/iq/{executor,optimizer}/`
//! (33 files). Both a locale sort and a bytewise (`LC_ALL=C`) sort agree on the
//! batch-0 membership (they differ only in the internal order of
//! `LJJoinLiftTest`/`LeftJoinOptimizationTest`, both inside the slice), so the five
//! assigned classes are unambiguous:
//!
//!   * `iq/executor/EmptyNodeRemovalTest.java`     (21 scenarios)
//!   * `iq/executor/FunctionalDependencyTest.java` (35 scenarios)
//!   * `iq/executor/LeftJoinOptimizationTest.java` (180 scenarios)
//!   * `iq/executor/LJJoinLiftTest.java`           ( 2 scenarios)
//!   * `iq/executor/QueryMergingTest.java`         (21 scenarios)
//!
//! scenarios_total = 259.
//!
//! This file holds ONLY batch-0 tests (no edits to `src/` or other test files, so
//! batches never conflict). It is an *external* integration test, so it reaches the
//! cascade exclusively through the crate's public surface
//! (`sf_sparql::cascade::run` over `sf_sparql::iq::Branch`), mirroring the in-crate
//! port pattern in `src/cascade/ws_g.rs` / `ws_st.rs` / `ws_fk.rs`.
//!
//! # Classification summary (honest — coverage over theater is refused)
//!
//! **SUPPORTED → GREEN (7 tests below).** The relational base-scan optimizations
//! sf's cascade already performs:
//!
//!   * self-LEFT-join elimination (cascade pass 2b, `self_left_join_elimination`):
//!     `LeftJoinOptimizationTest.testSelfJoinElimination1`,
//!     `…testSelfLeftJoinWithJoinOnLeft1`;
//!   * its NON-NULL-key safety guards:
//!     `…testSelfJoinNullableUniqueConstraint` (nullable unique determinant ⇒ keep),
//!     `…testNoSelfLeftJoin3` (non-unique determinant ⇒ keep);
//!   * same-terms self-join elimination under DISTINCT (cascade pass 2c,
//!     `same_terms_elimination`): `FunctionalDependencyTest.testRedundantSelfJoin8`,
//!     `…testRedundantSelfJoin9`, and its guard `…testNonRedundantSelfJoin1`.
//!
//! **NEEDS_IMPL** — relational, mostly `=_bag`-class optimizations sf does not have:
//!
//!   * *Non-unique functional-dependency–driven self-join elimination / FD-closure
//!     inference.* `FunctionalDependencyTest` is built around a NON-unique
//!     `col2 → col3,col4` constraint (`FunctionalDependency.defaultBuilder`); sf's
//!     pass-3 FD set seeds ONLY single-column unique keys (PK/UNIQUE) — there is no
//!     `functional_dependencies` field on `sf_sql::TableSchema` at all — so a merge
//!     that reads a *dependent* column from the other scan cannot be proven:
//!     `testRedundantSelfJoin1..7,7T11,7_1,7_3,10..18`, the `_T3`/`_T4` variants,
//!     `testRejectedJoin1..3` (FD-closure contradiction ⇒ empty),
//!     `testNonRequiredVariableDistinctProjection1,2`,
//!     `testLJRedundantSelfLeftJoin1,2`. Same root cause in
//!     `LeftJoinOptimizationTest`: `testJoinTransferFD1..7`,
//!     `testFDOnNullableDeterminant1..10`, `testNonJoinTransferFD1..4`,
//!     `testFDOnRight1..7`, `testFDSimplification`.
//!   * *LEFT-JOIN → INNER-JOIN downgrade via FK match-guarantee* (the right side is
//!     guaranteed to match through a NOT-NULL FK to the parent PK, so the LJ is
//!     bag-equal to an inner join; sf keeps the LEFT JOIN — its `fk_pk` pass only
//!     DROPS a parent, never downgrades a join): `testLeftJoinElimination1,2,4`,
//!     `testLeftJoinEliminationWithFilterCondition2`, `…WithImplicitFilterCondition`,
//!     `testLeftJoinNonElimination1` (its negative guard — vacuous in sf).
//!   * *Self-LEFT-join elimination with a right-side condition lifted to an
//!     `IfElseNull` construction* (sf's pass 2b conservatively refuses when the
//!     OPTIONAL carries an `extra`/`ON` filter): `testSelfJoinElimination2,3`,
//!     `testSelfJoinWithCondition`, `testSelfLeftJoinIfElseNull1,2`,
//!     `testSelfLeftJoinWithJoinOnLeft2`, `testLeftJoinEliminationWithFilterCondition4`.
//!   * *Self-LEFT-join non-unification ⇒ NULL-padding* (contradictory constants on a
//!     PK-determined row make the right never match): `testSelfLeftJoinNonUnification1`,
//!     `…1NotSimplifiedExpression`, `…EmptyResult`.
//!   * *DISTINCT-driven pruning of an unused OPTIONAL right side*:
//!     `testDistinctPruneUnusedRight1..7` (and the `testDistinctNoPrune*` guards).
//!   * *Selection / requirement transfer across a LEFT JOIN*: `testJoinTransfer1..14`,
//!     `testNonJoinTransfer6..9`, `testKeepConstraint1`, `testRequirement1,2` /
//!     `testNonRequirement1..3`.
//!   * *NULL-padding for an unsatisfiable LJ right side*: `testPaddingForUnsatisfiableRight1..3`.
//!
//! **BOUNDARY** — out of the base-scan cascade charter (Ontop IQ-tree normalization
//! scaffolding, ADR-0004; or handled at a different sf stage):
//!
//!   * **`EmptyNodeRemovalTest` (all 21).** The whole class exercises Ontop's
//!     `normalizeForOptimization` over an IQ tree of explicit `EmptyNode` /
//!     `ConstructionNode` / `UnionNode` / `LeftJoinNode` nodes (empty-relation
//!     propagation, substitution lifting, UNION-arm removal). sf has none of those
//!     node types: an unsatisfiable / empty sub-pattern yields *zero branches* at
//!     `unfold` time (not an `EmptyNode` a later pass removes), and there is no
//!     `ConstructionNode` to lift substitutions through. No faithful expression
//!     against sf's `Branch` cascade exists. (`testJoin1..3`, `testJoinLJ1..4`,
//!     `testLJ1..4`, `testFilter1,2`, `testUnionRemoval1..3`,
//!     `testUnionNoNullPropagation`, `testComplexTreeWithJoinCondition`,
//!     `testLJRemovalDueToNull1..3`. The LJ-with-provably-empty-right → NULL-pad
//!     subcase is the one genuine optimization in the bunch — noted under NEEDS_IMPL
//!     "NULL-padding" above — but it is unreachable as a cascade-level test.)
//!   * **`LJJoinLiftTest` (both).** `reduceLJTest1`, `testLjLift1` exercise the
//!     `UNION_AND_BINDING_LIFT_OPTIMIZER`: lifting a `UnionNode` above an
//!     INNER/LEFT join and lifting bindings into a `ConstructionNode`. sf compiles a
//!     UNION to a *bag of independent `Branch`es* (no union tree node to lift), so
//!     there is no analogous rewrite.
//!   * **`QueryMergingTest` (all 21).** `testPruning1`, `testEx1..14`,
//!     `testConflictingVariables`, `testUnionSameVariable`,
//!     `testDescendingSubstitutionOnRenamedNode`, `testTrueNodeCreation` exercise the
//!     `AbstractQueryMergingTransformer` (substituting a mapping/sub-query definition
//!     into an `IntensionalDataNode`, composing substitutions, renaming, distributing
//!     unions, pruning template-incompatible arms). sf inlines mappings during
//!     `unfold` (BGP → per-triples-map branches) rather than via an IQ-tree merging
//!     pass; there is no `IntensionalDataNode` to merge into. (`testPruning1`'s
//!     template-mismatch pruning overlaps sf's pass-1 *conceptually*, but is keyed on
//!     two-template incompatibility during merging — not the two-conflicting-`=`-
//!     constants shape pass-1 acts on — so porting it would assert a *different*
//!     scenario: refused.)
//!   * **`LeftJoinOptimizationTest` structural families.** `testMergeLJs1..27` /
//!     `testNonMergeLJs1..3` (nested-LEFT-JOIN associativity/merge — sf's flat `opts`
//!     model + `leftjoin.rs` handle nesting at unfold, not a cascade merge);
//!     `testProjectionAway1..12` / `testPartialProjectionAway1,2` /
//!     `testNonProjectionAway1..4` / `testImplicitVariableNonRemoval` (projection
//!     shrinking — sf projects only needed columns at emission, no `ConstructionNode`
//!     IR); `testLJReductionWithLJOnTheRight1..12` / `testNonLJReductionWithLJOnTheRight1,2`
//!     (LJ-on-the-right reduction over the IQ tree);
//!     `testSelfLeftJoinWithProvenanceBlockedByDistinct1..10` /
//!     `…NoOpt1,2` / `testSelfLeftJoinSameVarsDistinct1` (provenance/DISTINCT
//!     interaction over construction nodes); `testLeftJoinUnionConstants`,
//!     `testLeftJoinValues`, `testLeftJoinJoinLimit` (UNION/VALUES/SLICE inside an
//!     LJ — no VALUES/SLICE IR, cf. batch 6);
//!     `testLeftJoinEliminationConstructionNode1,2_1,2_2`,
//!     `testLeftJoinEliminationUnnecessaryConstructionNode1`,
//!     `testLeftJoinOrder1`, `testLeftJoinDenormalized1,2`,
//!     `testJoinTransferSameTerms1,2` / `testNonJoinTransferSameTerms1` /
//!     `testLJSameTerms1` (construction-node/term-lift normalization).
//!
//! **converted_red = 0**: every scenario asserted GREEN below actually passes — no
//! SUPPORTED-claimed scenario diverged from Ontop's oracle.

use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_sparql::cascade::{run, CascadeCtx};
use sf_sparql::iq::{Branch, ColRef, OptJoin, Scan, SqlCond, TermDef};
use sf_sql::{Column, TableSchema};
use std::collections::BTreeMap;

// --- shared helpers (mirror ws_g.rs / ws_st.rs) ----------------------------

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

fn binding_alias(def: &TermDef) -> usize {
    match def {
        TermDef::Derived { alias, .. } => *alias,
        other => panic!("expected a derived column binding, got {other:?}"),
    }
}

fn count_nn_guards(b: &Branch, col: &str) -> usize {
    b.where_conds
        .iter()
        .filter(|c| matches!(c, SqlCond::IsNotNull(r) if r.column.as_ref() == col))
        .count()
}

/// `LeftJoinOptimizationTest` TABLE1: PK `col1` (NOT NULL), `col2` NOT NULL,
/// `col3` nullable — the table the self-(LEFT-)join scenarios read.
fn lj_table1() -> TableSchema {
    let mut t = TableSchema::new("TABLE1");
    t.primary_key = vec!["col1".into()];
    t.columns = vec![
        Column::new("col1", "integer", true),  // PK ⇒ NOT NULL
        Column::new("col2", "integer", true),  // NOT NULL
        Column::new("col3", "integer", false), // nullable
    ];
    t
}

/// `LeftJoinOptimizationTest` TABLE3: composite PK `(col1, col2)`. Inert in these
/// ports (no FK, never self-joined) — present only so the extra core scan in
/// `testSelfLeftJoinWithJoinOnLeft1` reads a real table.
fn lj_table3() -> TableSchema {
    let mut t = TableSchema::new("TABLE3");
    t.primary_key = vec!["col1".into(), "col2".into()];
    t.columns = vec![
        Column::new("col1", "integer", true),
        Column::new("col2", "integer", true),
        Column::new("col3", "integer", true),
    ];
    t
}

/// `LeftJoinOptimizationTest` TABLE5: a NULLABLE unique constraint on `col1`
/// (`UniqueConstraint.builder` over a nullable column), `col2` NOT NULL. The
/// nullable determinant is the whole point of `testSelfJoinNullableUniqueConstraint`.
fn lj_table5() -> TableSchema {
    let mut t = TableSchema::new("TABLE5");
    t.unique = vec![vec!["col1".into()]];
    t.columns = vec![
        Column::new("col1", "integer", false), // NULLABLE unique — not a true key
        Column::new("col2", "integer", true),  // NOT NULL
    ];
    t
}

/// `FunctionalDependencyTest` TABLE1: PK `col1` plus a non-unique FD `col2 → col3,
/// col4` and an independent `col5`; all columns NOT NULL. sf models only the PK
/// (the FD is invisible to the cascade), which is exactly why the same-terms ports
/// below fire via pass 2c (DISTINCT + projection coverage on `col2`), NOT via the FD.
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
// SUPPORTED — cascade pass 2b (self_left_join_elimination)
// ===========================================================================

/// **GREEN** — Ontop `LeftJoinOptimizationTest.testSelfJoinElimination1`.
///
/// `LeftJoin(TABLE1(col1=M,col2=N,col3=O1), TABLE1(col1=M,col2=N1,col3=O))` joined
/// on the PK `col1=M` (NOT NULL). The OPTIONAL right side reads the SAME row the
/// core scan already has, so the self-LEFT-join collapses to one scan and the
/// right-side `?O` (its `col3`) rebinds onto the kept scan. Ontop's expected is the
/// single `TABLE1(col1=M, col2=N, col3=O)`.
#[test]
fn ontop_self_left_join_elimination_on_pk() {
    let mut b = Branch::single(scan(0, "TABLE1"));
    b.opts.push(OptJoin {
        scan: scan(1, "TABLE1"),
        on: vec![SqlCond::NullSafeEq(
            ColRef::new(0, "col1"),
            ColRef::new(1, "col1"),
        )],
        extra: Vec::new(),
    });
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("N".into(), col_binding(0, "col2"));
    b.bindings.insert("O".into(), col_binding(1, "col3")); // from the OPTIONAL right side

    let out = run(vec![b], &[lj_table1()], &CascadeCtx::default());
    assert_eq!(out.len(), 1);
    let b = &out[0];
    assert!(
        b.opts.is_empty(),
        "self-LEFT-join on the NOT-NULL PK col1 must collapse: {:?}",
        b.opts
    );
    assert_eq!(b.core.len(), 1, "single TABLE1 scan kept");
    assert_eq!(
        binding_alias(b.bindings.get("O").unwrap()),
        0,
        "?O (col3 of the OPTIONAL) rebinds onto the kept scan — same row"
    );
    assert_eq!(binding_alias(b.bindings.get("N").unwrap()), 0);
    assert_eq!(binding_alias(b.bindings.get("M").unwrap()), 0);
}

/// **GREEN** — Ontop `LeftJoinOptimizationTest.testSelfLeftJoinWithJoinOnLeft1`.
///
/// `LeftJoin( Join(TABLE1(col1=M,col2=N,col3=O1), TABLE3()), TABLE1(col1=M,col2=N1,col3=O) )`.
/// The self-LEFT-join on the PK `col1=M` collapses even though the left side also
/// inner-joins an unrelated `TABLE3` scan: pass 2b finds the kept TABLE1 core scan
/// reading the same table as the OPTIONAL and drops the OPTIONAL, leaving
/// `Join(TABLE1(M,N,O), TABLE3())`.
#[test]
fn ontop_self_left_join_collapses_with_extra_core_scan() {
    let mut b = Branch {
        core: vec![scan(0, "TABLE1"), scan(2, "TABLE3")],
        opts: vec![OptJoin {
            scan: scan(1, "TABLE1"),
            on: vec![SqlCond::NullSafeEq(
                ColRef::new(0, "col1"),
                ColRef::new(1, "col1"),
            )],
            extra: Vec::new(),
        }],
        bindings: BTreeMap::new(),
        where_conds: Vec::new(),
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
        nps: false,
    };
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("N".into(), col_binding(0, "col2"));
    b.bindings.insert("O".into(), col_binding(1, "col3"));

    let out = run(vec![b], &[lj_table1(), lj_table3()], &CascadeCtx::default());
    assert_eq!(out.len(), 1);
    let b = &out[0];
    assert!(b.opts.is_empty(), "self-LEFT-join collapses: {:?}", b.opts);
    assert_eq!(b.core.len(), 2, "kept TABLE1 scan + the inert TABLE3 scan");
    assert!(
        b.core.iter().any(|s| s.alias == 0)
            && b.core
                .iter()
                .any(|s| matches!(&s.source, LogicalSource::Table(t) if t == "TABLE3")),
        "both the kept TABLE1 (alias 0) and TABLE3 survive"
    );
    assert!(
        !b.core.iter().any(|s| s.alias == 1),
        "the redundant OPTIONAL TABLE1 scan (alias 1) is gone"
    );
    assert_eq!(
        binding_alias(b.bindings.get("O").unwrap()),
        0,
        "?O rebinds onto the kept TABLE1 scan"
    );
}

/// **GREEN (safety guard)** — Ontop `LeftJoinOptimizationTest.testSelfJoinNullableUniqueConstraint`.
///
/// `LeftJoin(TABLE5(col1=M), TABLE5(col1=M,col2=N))` where TABLE5.col1 is a
/// NULLABLE unique constraint. A nullable unique column is not a true key (the
/// null-safe `ON` admits its NULL rows differently on each side), so collapsing to
/// a bare scan would change multiplicities — Ontop keeps the LEFT JOIN and so must
/// sf (`key_is_non_null` refuses). Not vacuous: col1 *is* unique, so a naive impl
/// would wrongly eliminate.
#[test]
fn ontop_self_left_join_not_eliminated_on_nullable_unique() {
    let mut b = Branch::single(scan(0, "TABLE5"));
    b.opts.push(OptJoin {
        scan: scan(1, "TABLE5"),
        on: vec![SqlCond::NullSafeEq(
            ColRef::new(0, "col1"),
            ColRef::new(1, "col1"),
        )],
        extra: Vec::new(),
    });
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("N".into(), col_binding(1, "col2"));

    let out = run(vec![b], &[lj_table5()], &CascadeCtx::default());
    assert_eq!(
        out[0].opts.len(),
        1,
        "nullable unique determinant ⇒ self-LEFT-join MUST be preserved (=_bag safety)"
    );
    assert_eq!(out[0].core.len(), 1);
}

/// **GREEN (safety guard)** — Ontop `LeftJoinOptimizationTest.testNoSelfLeftJoin3`.
///
/// `LeftJoin(TABLE1(col1=M,col2=N), TABLE1(col2=N,col3=O))` joined on `col2` — a
/// NON-unique column. The shared determinant is not a key, so the OPTIONAL right
/// side can match many rows; eliminating it would change multiplicities. Ontop
/// leaves the query unchanged; sf's `is_unique_key(col2)` is false ⇒ no-op.
#[test]
fn ontop_self_left_join_not_eliminated_on_non_unique_col() {
    let mut b = Branch::single(scan(0, "TABLE1"));
    b.opts.push(OptJoin {
        scan: scan(1, "TABLE1"),
        on: vec![SqlCond::NullSafeEq(
            ColRef::new(0, "col2"),
            ColRef::new(1, "col2"),
        )],
        extra: Vec::new(),
    });
    b.bindings.insert("M".into(), col_binding(0, "col1"));
    b.bindings.insert("N".into(), col_binding(0, "col2"));
    b.bindings.insert("O".into(), col_binding(1, "col3"));

    let out = run(vec![b], &[lj_table1()], &CascadeCtx::default());
    assert_eq!(
        out[0].opts.len(),
        1,
        "non-unique join column col2 ⇒ self-LEFT-join MUST be preserved"
    );
    assert_eq!(out[0].core.len(), 1);
}

// ===========================================================================
// SUPPORTED — cascade pass 2c (same_terms_elimination under DISTINCT)
// ===========================================================================

/// **GREEN** — Ontop `FunctionalDependencyTest.testRedundantSelfJoin8`.
///
/// `SELECT DISTINCT ?X` over `Join(TABLE1(col2=X,col3=B), TABLE1(col2=X,col4=F))`
/// joined on `col2`. Only `?X` (= the shared `col2`) is projected, so under DISTINCT
/// the second scan is redundant — a single scan with an `IS NOT NULL(col2)` guard
/// yields the same distinct tuples. Ontop's expected is the single `TABLE1(col2=X)`.
/// NB: in sf this fires via same-terms (DISTINCT + projection coverage), *not* via
/// Ontop's `col2 → col3` FD — sf does not model non-unique FDs.
#[test]
fn ontop_same_terms_only_shared_col_projected() {
    let mut b = Branch {
        core: vec![scan(0, "table1"), scan(1, "table1")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![SqlCond::ColEq(
            ColRef::new(0, "col2"),
            ColRef::new(1, "col2"),
        )],
        distinct: true,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
        nps: false,
    };
    b.bindings.insert("X".into(), col_binding(0, "col2"));

    let ctx = CascadeCtx {
        project: Some(&["X".to_owned()]),
        distinct: true,
    };
    let out = run(vec![b], &[fd_table1()], &ctx);
    let b = &out[0];
    assert_eq!(b.core.len(), 1, "redundant second TABLE1 scan eliminated");
    assert_eq!(binding_alias(b.bindings.get("X").unwrap()), 0);
    assert_eq!(
        count_nn_guards(b, "col2"),
        1,
        "IS NOT NULL(col2) guard added for the shared projected column"
    );
}

/// **GREEN** — Ontop `FunctionalDependencyTest.testRedundantSelfJoin9`.
///
/// `SELECT DISTINCT ?X ?Y` over `Join(TABLE1(col2=X), TABLE1(col2=X,col3=Y))` on
/// `col2`. The first scan binds only `col2` (= `?X`), which the second scan already
/// provides; under DISTINCT it is subsumed, so the richer scan survives and both
/// `?X` and `?Y` rebind onto it. Ontop's expected is the single
/// `TABLE1(col2=X,col3=Y)`.
#[test]
fn ontop_same_terms_subsumed_scan_dropped() {
    let mut b = Branch {
        core: vec![scan(0, "table1"), scan(1, "table1")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![SqlCond::ColEq(
            ColRef::new(0, "col2"),
            ColRef::new(1, "col2"),
        )],
        distinct: true,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
        nps: false,
    };
    b.bindings.insert("X".into(), col_binding(0, "col2")); // only col2 on scan 0
    b.bindings.insert("Y".into(), col_binding(1, "col3")); // col3 only on scan 1

    let ctx = CascadeCtx {
        project: Some(&["X".to_owned(), "Y".to_owned()]),
        distinct: true,
    };
    let out = run(vec![b], &[fd_table1()], &ctx);
    let b = &out[0];
    assert_eq!(b.core.len(), 1, "the subsumed (col2-only) scan is dropped");
    // The surviving scan is the one carrying col3 (alias 1); both ?X and ?Y bind to it.
    assert_eq!(
        binding_alias(b.bindings.get("Y").unwrap()),
        1,
        "?Y stays on the richer surviving scan"
    );
    assert_eq!(
        binding_alias(b.bindings.get("X").unwrap()),
        1,
        "?X rebinds onto the surviving scan"
    );
    assert_eq!(
        count_nn_guards(b, "col2"),
        1,
        "IS NOT NULL(col2) guard added"
    );
}

/// **GREEN (guard)** — Ontop `FunctionalDependencyTest.testNonRedundantSelfJoin1`.
///
/// `SELECT DISTINCT ?X ?Y` over `Join(TABLE1(col1=X,col2=A,…), TABLE1(col2=A,…,col5=Y))`
/// joined on `col2=A`. `?X` is `col1` of one scan and `?Y` is the *independent*
/// `col5` of the other; neither projected variable is covered by the single shared
/// `col2` equality, so the self-join is NOT redundant. Both Ontop and sf keep the
/// two scans (pass 2c refuses — coverage check fails). Not vacuous: a naive
/// shared-column merge would wrongly collapse it.
#[test]
fn ontop_same_terms_independent_attribute_not_eliminated() {
    let mut b = Branch {
        core: vec![scan(0, "table1"), scan(1, "table1")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![SqlCond::ColEq(
            ColRef::new(0, "col2"),
            ColRef::new(1, "col2"),
        )],
        distinct: true,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
        nps: false,
    };
    b.bindings.insert("X".into(), col_binding(0, "col1")); // col1 of scan 0
    b.bindings.insert("Y".into(), col_binding(1, "col5")); // independent col5 of scan 1

    let ctx = CascadeCtx {
        project: Some(&["X".to_owned(), "Y".to_owned()]),
        distinct: true,
    };
    let out = run(vec![b], &[fd_table1()], &ctx);
    assert_eq!(
        out[0].core.len(),
        2,
        "projected ?X (col1) and ?Y (col5) are not covered by the shared col2 ⇒ both scans kept"
    );
}
