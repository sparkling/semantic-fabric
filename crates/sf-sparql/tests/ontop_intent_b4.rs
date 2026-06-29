//! Ontop-parity intent — batch 4 of 8 (ADR-0022): RED-SPEC and BOUNDARY documentation.
//!
//! Covers the NEEDS_IMPL and BOUNDARY scenarios from batch-4's five optimizer classes
//! (path-sorted indices 20–24). The two GREEN oracle ports for
//! `NullableUniqueConstraintTest.testNotSimplified1` and
//! `PullOutVariableOptimizerTest.testDataNode` live in `ontop_port_b4.rs`.
//!
//! ## BOUNDARY classes — no cascade-level tests possible
//!
//! ### index 20 — `NullabilityTest` (1 scenario) — BOUNDARY
//!
//! `testNullabilityComplexSubstitution1` asserts Ontop's
//! `IQTree.getVariableNullability().isPossiblyNullable(Y)` on a `ConstructionNode`
//! that binds `Y` to the null constant. This is a metadata query on Ontop's
//! IQ-node nullability-propagation API — no input→output tree rewrite occurs.
//! sf reasons about NOT-NULL only via `TableSchema::key_is_non_null`; there is no
//! exposed `VariableNullability` abstraction to test at cascade level (ADR-0004).
//!
//! ### index 22 — `PreventDistinctTest` (5 scenarios) — BOUNDARY
//!
//! All five scenarios (`testPreventDistinctDirect`, `testPreventDistinctMoreVariables`,
//! `testPreventDistinctIndirect`, `testPreventMultiple`, `testPreventMultipleDifferent`)
//! push a `SANITIZE`-typed sub-term below a `DistinctNode` when a DB type returns
//! `isPreventDistinctRecommended() == true`. Driven entirely by Ontop's per-type
//! `DBTermType` metadata, construction-node/DISTINCT-node splitting, and nested
//! `ImmutableFunctionalTerm` composition — dialect/type-system scaffolding
//! (ADR-0015 / ADR-0004) that sf does not model. No relational `=_bag` rewrite.
//!
//! ### index 24 — `PullOutVariableOptimizerTest` (14 of 15 scenarios) — BOUNDARY
//!
//! The `EXPLICIT_EQUALITY_TRANSFORMER` normalises implicit variable sharing across
//! IQ-tree nodes into an explicit equality + fresh variable. sf's IR is born in
//! that output form — shared-variable equi-joins are `ColEq` conditions from
//! unfold time (ADR-0007); the transform has no cascade analogue.
//!
//! Boundary scenarios: `testJoiningConditionTest1–5`, `testJoin3`, `testJoin4`,
//! `testLJUnnecessaryConstructionNode1`, `testDistinctProjection`,
//! `testUnionDistinctProjection` (operate on IQ-tree join/union/distinct nodes
//! with no sf cascade analogue); `testFlattenOutputVariable`,
//! `testFlattenOutputVariable2`, `testFlattenIndexVariable`,
//! `testFlattenIndexAndOutputVariable` (FlattenNode over nested JSON arrays —
//! out of sf's relational charter, and `@Ignore`d in Ontop too).
//!
//! ## NEEDS_IMPL scenarios — RED-SPEC tests below
//!
//! ### index 21 — `NullableUniqueConstraintTest` — 6 of 8 NEEDS_IMPL
//!
//! Optimisation sf lacks: when two scans of the same table are INNER-joined on a
//! *nullable* UNIQUE column, the SQL equi-join already excludes NULL-key rows
//! (NULL ≠ NULL in standard equality), so collapsing to one scan **plus** an
//! explicit `IS NOT NULL(key)` filter produces the same bag. Ontop's self-join
//! elimination does this; sf's `self_join_elimination` refuses because
//! `key_is_non_null(t, col)` is `false` for a nullable column — a conservative,
//! `=_bag`-sound guard that misses the optimisation.
//!
//! The five remaining NEEDS_IMPL scenarios (`testJoinOnLeft1–3`, `testFilterAbove1`,
//! `testFilterAboveSparse1`) additionally require rewriting right-side bindings via
//! `IF(key = const, col, NULL)` provenance terms — sf's `TermDef` IR has no
//! conditional-null constructor, making those output shapes inexpressible as
//! cascade oracles.
//!
//! ### index 23 — `ProjectionShrinkingOptimizerTest` — 8 of 8 NEEDS_IMPL
//!
//! Ontop removes variables not used by ancestor nodes from `UnionNode` /
//! data-node variable maps / `ConstructionNode` substitutions. sf prunes columns
//! implicitly at SQL emission; there is no cascade pass that removes a binding
//! from `Branch::bindings` when `CascadeCtx::project` excludes it. Adding
//! explicit projection-shrinking would trim `Branch::bindings` to projected and
//! join-condition variables — a `=_bag`-preserving IQ-level optimisation.
//!
//! The two Ontop no-op scenarios (`testUnionAndImplicitJoinCondition1`,
//! `testUnionAndExplicitJoinCondition1`) would be vacuous for sf (sf never
//! shrinks), so they are not written as guards.

use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_sparql::cascade::{run, CascadeCtx};
use sf_sparql::iq::{Branch, CmpOp, ColRef, Scan, SqlCond, TermDef};
use sf_sql::{Column, TableSchema};

// --- helpers ------------------------------------------------------------------

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

/// `TableSchema` for `NullableUniqueConstraintTest` tables.
/// `col1` carries a UNIQUE constraint but is NULLABLE — not a true key because
/// NULL ≠ NULL in SQL equi-join semantics.
fn nuc_table(name: &str) -> TableSchema {
    let mut t = TableSchema::new(name);
    t.unique = vec![vec!["col1".into()]]; // UNIQUE(col1) — nullable
    t.columns = vec![
        Column::new("col1", "integer", false), // nullable ⇒ key_is_non_null returns false
        Column::new("col2", "integer", false),
        Column::new("col3", "integer", false),
    ];
    t
}

// ============================================================================
// NEEDS_IMPL — NullableUniqueConstraintTest.testSimpleJoin1
// ============================================================================

/// **RED-SPEC** — `NullableUniqueConstraintTest.testSimpleJoin1`
///
/// `TABLE1(A, B, C)` INNER JOIN `TABLE1(A, 2, G)` on nullable-UNIQUE `col1 = A`.
///
/// Ontop collapses the inner self-join to a single scan plus a compensating
/// `IS NOT NULL(col1)` filter. The SQL equi-join already excludes NULL-key rows
/// (NULL ≠ NULL), so the merged scan with an explicit IS NOT NULL reproduces the
/// same bag — a valid `=_bag`-preserving optimisation.
///
/// sf's `self_join_elimination` refuses because `key_is_non_null(t, "col1")`
/// returns `false` for a nullable column — a conservative, correct guard that
/// misses this optimisation. When implemented: `core.len() == 1` and
/// `IS NOT NULL(col1)` in `where_conds`.
#[test]
fn nuc_inner_self_join_nullable_unique_collapses_with_not_null() {
    let t = nuc_table("table1");

    let mut b = Branch::single(scan(0, "table1"));
    b.core.push(scan(1, "table1"));
    b.where_conds.push(SqlCond::ColEq(
        ColRef::new(0, "col1"),
        ColRef::new(1, "col1"),
    ));
    b.where_conds
        .push(SqlCond::Cmp(ColRef::new(1, "col2"), CmpOp::Eq, "2".into()));
    b.bindings.insert("A".into(), col_binding(0, "col1"));
    b.bindings.insert("B".into(), col_binding(0, "col2"));
    b.bindings.insert("C".into(), col_binding(0, "col3"));
    b.bindings.insert("G".into(), col_binding(1, "col3"));

    let out = run(vec![b], std::slice::from_ref(&t), &CascadeCtx::default());
    assert_eq!(out.len(), 1);
    let b = &out[0];
    assert_eq!(
        b.core.len(),
        1,
        "nullable-unique inner self-join should collapse to one scan — optimisation sf lacks"
    );
    assert!(
        b.where_conds
            .iter()
            .any(|c| matches!(c, SqlCond::IsNotNull(r) if r.column.as_ref() == "col1")),
        "collapsed scan must carry IS NOT NULL(col1) to compensate for NULL-exclusion: {:?}",
        b.where_conds
    );
}

// ============================================================================
// NEEDS_IMPL — NullableUniqueConstraintTest.testSimpleJoin2
// ============================================================================

/// **RED-SPEC** — `NullableUniqueConstraintTest.testSimpleJoin2`
///
/// Three scans of `TABLE1` inner-joined on nullable-UNIQUE `col1` in a chain
/// (two `ColEq` conditions on the same key). Ontop applies self-join elimination
/// iteratively, collapsing all three to a single scan with IS NOT NULL guards.
/// sf keeps all three scans.
///
/// (Ontop's `testSimpleJoin2` covers the iterative-elimination case where both
/// duplicate scans are eliminated and IS NOT NULL constraints accumulate.)
#[test]
fn nuc_inner_self_join_nullable_unique_three_scans_collapse() {
    let t = nuc_table("table1");

    let mut b = Branch::single(scan(0, "table1"));
    b.core.push(scan(1, "table1"));
    b.core.push(scan(2, "table1"));
    b.where_conds.push(SqlCond::ColEq(
        ColRef::new(0, "col1"),
        ColRef::new(1, "col1"),
    ));
    b.where_conds.push(SqlCond::ColEq(
        ColRef::new(0, "col1"),
        ColRef::new(2, "col1"),
    ));
    b.bindings.insert("A".into(), col_binding(0, "col1"));
    b.bindings.insert("B".into(), col_binding(1, "col2"));
    b.bindings.insert("G".into(), col_binding(2, "col3"));

    let out = run(vec![b], std::slice::from_ref(&t), &CascadeCtx::default());
    assert_eq!(out.len(), 1);
    let b = &out[0];
    assert_eq!(
        b.core.len(),
        1,
        "all three nullable-unique scans should collapse to one — optimisation sf lacks"
    );
    assert!(
        b.where_conds
            .iter()
            .any(|c| matches!(c, SqlCond::IsNotNull(r) if r.column.as_ref() == "col1")),
        "IS NOT NULL(col1) must compensate for NULL-exclusion after collapse: {:?}",
        b.where_conds
    );
}

// ============================================================================
// NEEDS_IMPL — ProjectionShrinkingOptimizerTest: single branch
// ============================================================================

/// **RED-SPEC** — `ProjectionShrinkingOptimizerTest` (single-branch base case)
///
/// A branch binds `?id` and `?name` (both projected) and `?unused` (not in the
/// project list). When `CascadeCtx::project` is `Some(&["id", "name"])`, the
/// cascade should drop `?unused` from `Branch::bindings`.
///
/// sf currently passes `project` only to distinct-removal (pass 6) and never
/// consults it for binding shrinking. Optimisation sf lacks: explicit IQ-level
/// projection shrinking.
#[test]
fn projection_shrinking_removes_unused_binding_single_branch() {
    let mut t = TableSchema::new("t");
    t.columns = vec![
        Column::new("id", "integer", true),
        Column::new("name", "text", true),
        Column::new("extra", "text", true),
    ];

    let mut b = Branch::single(scan(0, "t"));
    b.bindings.insert("id".into(), col_binding(0, "id"));
    b.bindings.insert("name".into(), col_binding(0, "name"));
    b.bindings.insert("unused".into(), col_binding(0, "extra")); // not projected

    let project = vec!["id".to_owned(), "name".to_owned()];
    let ctx = CascadeCtx {
        distinct: false,
        project: Some(&project),
    };
    let out = run(vec![b], std::slice::from_ref(&t), &ctx);
    assert_eq!(out.len(), 1);
    let b = &out[0];
    assert!(
        !b.bindings.contains_key("unused"),
        "projected-out ?unused should be removed from Branch::bindings (projection shrinking): {:?}",
        b.bindings.keys().collect::<Vec<_>>()
    );
    assert!(
        b.bindings.contains_key("id"),
        "projected ?id must be retained"
    );
    assert!(
        b.bindings.contains_key("name"),
        "projected ?name must be retained"
    );
}

// ============================================================================
// NEEDS_IMPL — ProjectionShrinkingOptimizerTest.testUnion
// ============================================================================

/// **RED-SPEC** — `ProjectionShrinkingOptimizerTest.testUnion`
///
/// Two branches (UNION bag) each bind `?x` (projected) and `?y` (not projected).
/// Ontop removes `?y` from both union arms' data-node variable maps. When
/// `project = Some(&["x"])`, the cascade should remove `?y` from both branches'
/// `bindings`.
///
/// sf's cascade does not consult `project` for per-branch binding shrinking.
#[test]
fn projection_shrinking_union_removes_unused_from_all_branches() {
    let mut t1 = TableSchema::new("table1");
    t1.columns = vec![
        Column::new("c0", "integer", true),
        Column::new("c1", "integer", true),
    ];
    let mut t2 = TableSchema::new("table2");
    t2.columns = vec![
        Column::new("c0", "integer", true),
        Column::new("c1", "integer", true),
    ];

    let mut b1 = Branch::single(scan(0, "table1"));
    b1.bindings.insert("x".into(), col_binding(0, "c0"));
    b1.bindings.insert("y".into(), col_binding(0, "c1")); // not projected

    let mut b2 = Branch::single(scan(0, "table2"));
    b2.bindings.insert("x".into(), col_binding(0, "c0"));
    b2.bindings.insert("y".into(), col_binding(0, "c1")); // not projected

    let project = vec!["x".to_owned()];
    let ctx = CascadeCtx {
        distinct: false,
        project: Some(&project),
    };
    let schema = [t1, t2];
    let out = run(vec![b1, b2], &schema, &ctx);
    assert_eq!(out.len(), 2, "both union branches survive");
    for (i, br) in out.iter().enumerate() {
        assert!(
            !br.bindings.contains_key("y"),
            "branch {i}: projected-out ?y should be removed by projection shrinking"
        );
        assert!(
            br.bindings.contains_key("x"),
            "branch {i}: projected ?x must be retained"
        );
    }
}

// ============================================================================
// NEEDS_IMPL — ProjectionShrinkingOptimizerTest.testUnionAndImplicitJoinCondition2
// ============================================================================

/// **RED-SPEC** — `ProjectionShrinkingOptimizerTest.testUnionAndImplicitJoinCondition2`
///
/// Two-scan inner join: `tj` (binds `?x`, `?z`) × `tu` (binds `?x`, `?y`)
/// on `tj.x = tu.x`. Only `?x` is projected. Both `?y` and `?z` are unused
/// above the join — projection shrinking should remove them from `Branch::bindings`.
///
/// Note: sf's `ColEq` join condition uses raw `ColRef` (alias + column name),
/// not variable names. Removing `?y`/`?z` from `bindings` does not affect
/// `where_conds`; the join filter stays and is evaluated correctly. A correct
/// shrinking implementation removes all bindings that are (a) not projected and
/// (b) not needed to reconstruct any projected variable.
#[test]
fn projection_shrinking_join_removes_all_unprojected_bindings() {
    let mut tj = TableSchema::new("tj");
    tj.columns = vec![
        Column::new("x", "integer", true),
        Column::new("z", "integer", true),
    ];
    let mut tu = TableSchema::new("tu");
    tu.columns = vec![
        Column::new("x", "integer", true),
        Column::new("y", "integer", true),
    ];

    let mut b = Branch::single(scan(0, "tj"));
    b.core.push(scan(1, "tu"));
    b.where_conds
        .push(SqlCond::ColEq(ColRef::new(0, "x"), ColRef::new(1, "x")));
    b.bindings.insert("x".into(), col_binding(0, "x")); // projected
    b.bindings.insert("z".into(), col_binding(0, "z")); // not projected
    b.bindings.insert("y".into(), col_binding(1, "y")); // not projected

    let project = vec!["x".to_owned()];
    let ctx = CascadeCtx {
        distinct: false,
        project: Some(&project),
    };
    let schema = [tj, tu];
    let out = run(vec![b], &schema, &ctx);
    assert_eq!(out.len(), 1);
    let b = &out[0];
    assert!(
        !b.bindings.contains_key("y"),
        "?y is not projected — should be removed by projection shrinking"
    );
    assert!(
        !b.bindings.contains_key("z"),
        "?z is not projected — should be removed by projection shrinking"
    );
    assert!(
        b.bindings.contains_key("x"),
        "projected ?x must be retained"
    );
    // The ColEq join condition references raw columns and is independent of bindings;
    // it must survive so the SQL join filter is not lost.
    assert!(
        b.where_conds
            .iter()
            .any(|c| matches!(c, SqlCond::ColEq(a, d)
            if a.alias == 0 && a.column.as_ref() == "x"
            && d.alias == 1 && d.column.as_ref() == "x")),
        "ColEq join condition must be preserved even after binding shrinking: {:?}",
        b.where_conds
    );
}
