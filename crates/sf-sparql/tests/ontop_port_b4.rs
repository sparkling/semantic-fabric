//! Ontop-parity oracle port — batch 4 of 8 (ADR-0022).
//!
//! Assigned slice `[20, 25)` over the combined, path-sorted `*Test.java` listing
//! of `~/source/ontop/core/optimization/src/test/java/it/unibz/inf/ontop/iq/{executor,optimizer}/`
//! (33 files, sorted indices 0..=32). The slice resolves to five `optimizer`
//! classes:
//!
//!   * index 20 — `optimizer/NullabilityTest.java`                 ( 1 scenario)
//!   * index 21 — `optimizer/NullableUniqueConstraintTest.java`    ( 8 scenarios)
//!   * index 22 — `optimizer/PreventDistinctTest.java`             ( 5 scenarios)
//!   * index 23 — `optimizer/ProjectionShrinkingOptimizerTest.java`( 8 scenarios)
//!   * index 24 — `optimizer/PullOutVariableOptimizerTest.java`    (15 scenarios)
//!
//! scenarios_total = 1 + 8 + 5 + 8 + 15 = 37.
//!
//! **This slice lands almost entirely on Ontop machinery sf does not model.** Two
//! scenarios port faithfully as GREEN guards (below); the rest split NEEDS_IMPL /
//! BOUNDARY. The honest summary: converted_green = 2, converted_red = 0 (no
//! SUPPORTED-claimed scenario diverges — where sf and Ontop differ, sf is the
//! *more conservative*, still-`=_bag`-sound party, i.e. a missing optimization,
//! never a wrong answer).
//!
//! Port pattern mirrors `src/cascade/ws_g.rs` / `ws_st.rs`: build the input IQ as
//! a [`Branch`], run the real `cascade::run` with a real `TableSchema`, assert the
//! optimized branch structurally.
//!
//! ## Per-scenario classification (provenance: Ontop 5.5.0 source)
//!
//! ### index 20 — `NullabilityTest` — 1 scenario — BOUNDARY
//!
//! * `testNullabilityComplexSubstitution1` — **BOUNDARY.** Asserts
//!   `iqTree.getVariableNullability().isPossiblyNullable(Y)` over a
//!   `ConstructionNode` binding `Y` to the null constant. This is a *metadata
//!   query* on Ontop's IQ-node variable-nullability propagation — no input→output
//!   tree rewrite at all. sf has no exposed `VariableNullability` abstraction (it
//!   reasons about NOT-NULL only via `TableSchema`/`key_is_non_null`); pure Java IQ
//!   scaffolding (ADR-0004), nothing for the cascade to do.
//!
//! ### index 21 — `NullableUniqueConstraintTest` — 8 scenarios
//!
//! Table1/Table2 each have a **NULLABLE** single-column UNIQUE constraint (the
//! class name is literal). Every transforming scenario relies on Ontop's
//! if-else-null provenance machinery (`getProvenanceSpecialConstant`,
//! `getIfElseNull`) which sf's [`TermDef`] cannot represent (it has
//! Const/Derived/Coalesce/Concat/Agg — no conditional `IF(cond, v, NULL)`).
//!
//! * `testNotSimplified1` — **SUPPORTED → GREEN** (`ontop_nuc_self_left_join_*`
//!   below). The one no-op scenario: a self-LEFT-join of Table1 with itself on the
//!   nullable UNIQUE column is *not* simplified (no `IS NOT NULL` guarantees the
//!   determinant is a true key). sf's `self_left_join_elimination` refuses on a
//!   nullable determinant (and on the inner FILTER) — same end state, same reason.
//! * `testSimpleJoin1`, `testSimpleJoin2` — **NEEDS_IMPL.** INNER self-join over a
//!   nullable UNIQUE column collapses to one scan **+ a compensating `IS NOT NULL`
//!   filter** (`testSimpleJoin2` adds two). sf refuses self-join elimination on a
//!   nullable key (`key_is_non_null`), keeping both scans — `=_bag`-equivalent
//!   (the join already drops NULL-key rows) but unoptimized. Optimization sf lacks:
//!   nullable-unique self-join elimination with a compensating NOT-NULL filter.
//! * `testJoinOnLeft1`, `testJoinOnLeft2`, `testJoinOnLeft3`, `testFilterAbove1`,
//!   `testFilterAboveSparse1` — **NEEDS_IMPL.** Self-LEFT-join over a nullable
//!   UNIQUE column lifted into the left scan, the right-only columns rebound via
//!   `IF(left.uniqueCol = const, leftCol, NULL)`. Requires the if-else-null
//!   provenance term sf's IR lacks → cannot be faithfully expressed as a cascade
//!   oracle (output tree is inexpressible).
//!
//! ### index 22 — `PreventDistinctTest` — 5 scenarios — BOUNDARY (all)
//!
//! `testPreventDistinctDirect`, `testPreventDistinctMoreVariables`,
//! `testPreventDistinctIndirect`, `testPreventMultiple`,
//! `testPreventMultipleDifferent` — push a `SANITIZE`-typed sub-term **below** a
//! `DistinctNode` (binding a fresh `f0`/`f1`) when the inner DB type returns
//! `isPreventDistinctRecommended() == true`. Driven entirely by Ontop's
//! `DBTermType` type-system metadata + construction-node/DISTINCT splitting over
//! nested `ImmutableFunctionalTerm`s. sf models neither per-type prevent-distinct
//! recommendation nor nested function-application terms nor DISTINCT pushing —
//! dialect/type-system scaffolding (ADR-0015 / ADR-0004), not a relational
//! `=_bag` cascade rewrite.
//!
//! ### index 23 — `ProjectionShrinkingOptimizerTest` — 8 scenarios — NEEDS_IMPL (all)
//!
//! `testUnion`, `testUnionAndImplicitJoinCondition1`,
//! `testUnionAndImplicitJoinCondition2`, `testUnionAndExplicitJoinCondition1`,
//! `testUnionAndExplicitJoinCondition2`, `testUnionAndFilter`,
//! `testConstructionNode`, `testConstructionNodeAndImplicitJoinCondition2` —
//! *projection/column shrinking*: drop variables from `UnionNode` / data-node
//! variable-maps / `ConstructionNode` substitutions when they are not projected
//! upward and not used by a join/filter. A real `=_bag`-preserving optimization,
//! but sf prunes unread columns **implicitly at SQL emission** (a `Branch`'s
//! `Scan` carries no exposed column-set to shrink) — there is no cascade pass that
//! transforms the IQ this way, so it cannot be expressed as a `cascade::run`
//! oracle. (The two no-op scenarios, `...ImplicitJoinCondition1` /
//! `...ExplicitJoinCondition1`, would be *vacuous* for sf, which never shrinks —
//! so they are not written as guards: coverage theater, refused.) Optimization sf
//! lacks at the IQ level: explicit projection-shrinking.
//!
//! ### index 24 — `PullOutVariableOptimizerTest` — 15 scenarios
//!
//! The `EXPLICIT_EQUALITY_TRANSFORMER`: rename a variable that repeats across data
//! nodes / positions to a fresh variable and add an explicit strict-equality
//! (`X = Xf0`) on the enclosing join/filter. **sf's IR is born in exactly this
//! output form** — a shared-variable equi-join is a `SqlCond::ColEq` between two
//! scan columns, and a variable repeated within one scan is a same-alias `ColEq`
//! self-comparison (term-construction lifting, ADR-0007). There is no implicit
//! form for a cascade pass to normalize.
//!
//! * `testDataNode` — **SUPPORTED → GREEN** (`ontop_pullout_*` below). A data node
//!   reusing one variable in two columns (`TABLE7(Z,X,Z,Y)`) is, in sf, a
//!   same-scan self-equality `col0 = col2`. The faithful, non-vacuous invariant:
//!   the cascade **preserves** that self-equality and does **not** mistake it for
//!   an eliminable self-join (`find_self_join` skips `a.alias == c.alias`). This is
//!   the `=_bag`-safety counterpart of Ontop's pulled-out `Z = Zf0`.
//! * `testJoiningConditionTest1`, `testJoiningConditionTest2`,
//!   `testJoiningConditionTest3`, `testJoiningConditionTest4`,
//!   `testJoiningConditionTest5`, `testJoin3`, `testJoin4`,
//!   `testLJUnnecessaryConstructionNode1`, `testDistinctProjection`,
//!   `testUnionDistinctProjection` — **BOUNDARY.** The implicit→explicit equality
//!   normalization over `InnerJoinNode`/`LeftJoinNode`/`UnionNode`/`DistinctNode`
//!   has no sf cascade analogue (sf is born explicit; the transform is an
//!   unfold-time/representation concern, not an `=_bag` rewrite).
//! * `testFlattenOutputVariable`, `testFlattenOutputVariable2`,
//!   `testFlattenIndexVariable`, `testFlattenIndexAndOutputVariable` —
//!   **BOUNDARY** (`@Ignore`d in Ontop too): `FlattenNode` over nested JSON arrays
//!   — out of the relational charter (no `Flatten` IR in sf).

use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_sparql::cascade::{run, CascadeCtx};
use sf_sparql::iq::{Branch, CmpOp, ColRef, OptJoin, Scan, SqlCond, TermDef};
use sf_sql::{Column, TableSchema};

// --- port helpers (mirror `src/cascade/ws_g.rs`) ---------------------------

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

/// Whether `where_conds` contains a `ColEq` between the two given columns
/// (in either order).
fn has_col_eq(b: &Branch, a: (usize, &str), c: (usize, &str)) -> bool {
    b.where_conds.iter().any(|cond| {
        if let SqlCond::ColEq(x, y) = cond {
            let xa = (x.alias, x.column.as_ref());
            let ya = (y.alias, y.column.as_ref());
            (xa == a && ya == c) || (xa == c && ya == a)
        } else {
            false
        }
    })
}

// ===========================================================================
// GREEN — NullableUniqueConstraintTest.testNotSimplified1
// ===========================================================================

/// **GREEN** — Ontop `NullableUniqueConstraintTest.testNotSimplified1`
/// (`optimizeAndCompare(initialIQ, initialIQ)` — a no-op).
///
/// `Table1` has a **nullable** UNIQUE constraint on `col1`. The query is a
/// self-LEFT-join of `Table1` with itself on that nullable unique column, the
/// right side adding an implicit `col2 = 2` restriction:
///
/// ```text
/// LEFT JOIN( Table1{col1:A},  Table1(col1:A, col2:2, col3:G) )   project [A, G]
/// ```
///
/// Ontop does **not** simplify it: without an `IS NOT NULL` guarantee the nullable
/// UNIQUE column is not a true key (NULL ≠ NULL), so the left-join right side is
/// not provably a 1:1 match. sf's `self_left_join_elimination` refuses for the
/// matching `=_bag`-safety reason — its determinant must be a NON-NULL single
/// unique key, and an OPTIONAL carrying an inner FILTER (`extra`) is never
/// collapsed. Same input, same preserved LEFT JOIN.
#[test]
fn ontop_nuc_self_left_join_nullable_unique_not_simplified() {
    // Table1(col1 nullable-UNIQUE, col2, col3) — the literal "NullableUniqueConstraint" table.
    let mut t1 = TableSchema::new("table1");
    t1.unique = vec![vec!["col1".into()]]; // UNIQUE(col1)…
    t1.columns = vec![
        Column::new("col1", "integer", false), // …but NULLABLE ⇒ not a true key
        Column::new("col2", "integer", false),
        Column::new("col3", "integer", false),
    ];

    let mut b = Branch::single(scan(0, "table1"));
    b.bindings.insert("A".into(), col_binding(0, "col1"));
    b.bindings.insert("G".into(), col_binding(1, "col3"));
    // OPTIONAL right side: Table1 again, shared nullable-unique col1 + inner col2 = 2.
    b.opts.push(OptJoin {
        scan: scan(1, "table1"),
        on: vec![SqlCond::NullSafeEq(
            ColRef::new(0, "col1"),
            ColRef::new(1, "col1"),
        )],
        extra: vec![SqlCond::Cmp(ColRef::new(1, "col2"), CmpOp::Eq, "2".into())],
    });

    let out = run(vec![b], std::slice::from_ref(&t1), &CascadeCtx::default());
    assert_eq!(out.len(), 1);
    let b = &out[0];
    assert_eq!(
        b.opts.len(),
        1,
        "self-LEFT-join on a NULLABLE unique constraint must be preserved (no IS NOT NULL guarantee) — Ontop no-op parity"
    );
    assert_eq!(b.core.len(), 1, "the single left scan is unchanged");
    // ?G still reads the optional (right) scan — nothing was rebound/merged.
    match b.bindings.get("G").unwrap() {
        TermDef::Derived { alias, .. } => assert_eq!(
            *alias, 1,
            "?G stays bound to the un-eliminated OPTIONAL right scan"
        ),
        other => panic!("expected ?G as a derived binding, got {other:?}"),
    }
    // The inner FILTER stays inside the OPTIONAL (R5), never hoisted to the outer WHERE.
    assert!(
        b.where_conds.is_empty(),
        "outer WHERE stays empty — the col2=2 restriction remains inside the OPTIONAL"
    );
}

// ===========================================================================
// GREEN — PullOutVariableOptimizerTest.testDataNode
// ===========================================================================

/// **GREEN** — Ontop `PullOutVariableOptimizerTest.testDataNode`.
///
/// Ontop's `EXPLICIT_EQUALITY_TRANSFORMER` turns a data node that reuses one
/// variable in two columns into a fresh-variable rename plus an explicit equality:
///
/// ```text
/// TABLE7(Z, X, Z, Y)   ──▶   FILTER(Z = Zf0) / TABLE7(Z, X, Zf0, Y)
/// ```
///
/// sf's IR is **born in that pulled-out form**: the repeated variable is a
/// same-scan self-equality `col0 = col2` (term-construction lifting, ADR-0007).
/// The faithful, non-vacuous invariant this pins: the cascade **preserves** that
/// self-comparison and does **not** mistake it for an eliminable self-join
/// (`find_self_join` deliberately skips `a.alias == c.alias` — a `?x :p ?x` guard
/// is an effective `IS NOT NULL`, not a redundant join; ADR-0007 R3/`=_bag`). A
/// regression that dropped or mis-eliminated the self-equality would fail here.
#[test]
fn ontop_pullout_duplicate_variable_self_equality_preserved() {
    // TABLE7 with four columns (no key); col0 and col2 both hold variable Z.
    let mut t7 = TableSchema::new("table7");
    t7.columns = vec![
        Column::new("col0", "integer", false),
        Column::new("col1", "integer", false),
        Column::new("col2", "integer", false),
        Column::new("col3", "integer", false),
    ];

    let mut b = Branch::single(scan(0, "table7"));
    // The duplicate Z (positions col0 and col2) ⇒ an explicit same-scan equality.
    b.where_conds = vec![SqlCond::ColEq(
        ColRef::new(0, "col0"),
        ColRef::new(0, "col2"),
    )];
    b.bindings.insert("X".into(), col_binding(0, "col1"));
    b.bindings.insert("Y".into(), col_binding(0, "col3"));
    b.bindings.insert("Z".into(), col_binding(0, "col0"));

    let out = run(vec![b], std::slice::from_ref(&t7), &CascadeCtx::default());
    assert_eq!(out.len(), 1);
    let b = &out[0];
    assert_eq!(
        b.core.len(),
        1,
        "single TABLE7 scan kept — a same-scan self-equality is NOT a self-join"
    );
    assert!(
        has_col_eq(b, (0, "col0"), (0, "col2")),
        "the duplicate-variable self-equality (col0 = col2) must be preserved, not eliminated: {:?}",
        b.where_conds
    );
    // ?Z still reads col0 of the kept scan (nothing rebound onto a dropped alias).
    match b.bindings.get("Z").unwrap() {
        TermDef::Derived { alias, .. } => assert_eq!(*alias, 0, "?Z still reads the kept scan"),
        other => panic!("expected ?Z as a derived binding, got {other:?}"),
    }
}
