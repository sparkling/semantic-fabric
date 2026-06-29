//! Ontop-parity oracle port — batch 3 of 8 (ADR-0022).
//!
//! Assigned slice = sorted `*Test.java` indices `[15, 20)` over the combined
//! `~/source/ontop/core/optimization/src/test/java/.../iq/{executor,optimizer}/`
//! listing (33 files, indices 0..=32):
//!
//! * 15 `optimizer/FlattenUnionOptimizerTest`        — BOUNDARY (see notes)
//! * 16 `optimizer/FunctionalDependencyInferenceTest`— NEEDS_IMPL (see notes)
//! * 17 `optimizer/NodeDeletionTest`                 — 4 GREEN ports below
//! * 18 `optimizer/NoQueryContextTest`               — BOUNDARY (see notes)
//! * 19 `optimizer/NRAJoinOptimizerTest`             — BOUNDARY (file commented out)
//!
//! Only `NodeDeletionTest` ports to a faithful `cascade::run` oracle. Its
//! union / inner-join scenarios use Ontop's `InnerJoinNode(FALSE_CONDITION)` to
//! make a conjunctive branch unsatisfiable; the optimizer then **deletes** that
//! branch (and, when it is the whole query, declares the tree empty). sf realises
//! exactly this in cascade **pass 1** (`prune_iri_template_mismatch`: a column
//! pinned to two different `=` constants is unsatisfiable) composed with the
//! `filter_map`/`return None` branch drop in [`sf_sparql::cascade::run`]: an
//! empty arm is removed from the bag union, and a query of only empty arms
//! returns the empty bag (Ontop's `isDeclaredAsEmpty`).
//!
//! The FALSE join condition is modelled the sf way — `x = "1" AND x = "2"` on a
//! core column (the documented "IRI-template-mismatch" disjointness pass 1
//! detects). This is the faithful sf analog of an unsatisfiable conjunctive
//! branch; it is the same mechanism the engine uses to prune disjoint
//! triples-map alternatives.
//!
//! Provenance: `it.unibz.inf.ontop.iq.optimizer.NodeDeletionTest`
//! (`testSimpleJoin`, `testUnion1`, `testUnion2`, `testInvalidLeftPartOfLeftJoin`).

use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_sparql::cascade::{run, CascadeCtx};
use sf_sparql::iq::{Branch, CmpOp, ColRef, OptJoin, Scan, SqlCond, TermDef};
use sf_sql::{Column, TableSchema};

fn scan(alias: usize, table: &str) -> Scan {
    Scan {
        alias,
        source: LogicalSource::Table(table.to_owned()),
    }
}

/// Bind `?x` to column `x` of `alias` — makes each arm a realistic, union-
/// compatible query arm (all arms project the same variable, as in Ontop).
fn col_binding(alias: usize, col: &str) -> TermDef {
    TermDef::Derived {
        term_map: TermMap::Column(col.into(), TermSpec::plain_literal()),
        alias,
    }
}

/// `table1..table5`, each two plain text columns `x`, `y` — no keys, no FKs (so
/// no self-join / FK-PK elimination fires; only the empty-branch pass is under
/// test). Mirrors Ontop's `TABLEn_AR2` key-free `NoDependencyTestDBMetadata`.
fn schema() -> Vec<TableSchema> {
    (1..=5)
        .map(|i| {
            let mut t = TableSchema::new(format!("table{i}"));
            t.columns = vec![
                Column::new("x", "text", false),
                Column::new("y", "text", false),
            ];
            t
        })
        .collect()
}

/// `x = "1" AND x = "2"` on `alias` — sf's unsatisfiable-branch encoding (pass 1).
/// The faithful analog of Ontop's `InnerJoinNode(FALSE_CONDITION)`.
fn false_condition(alias: usize) -> Vec<SqlCond> {
    vec![
        SqlCond::Cmp(ColRef::new(alias, "x"), CmpOp::Eq, "1".to_owned()),
        SqlCond::Cmp(ColRef::new(alias, "x"), CmpOp::Eq, "2".to_owned()),
    ]
}

/// Build one bag-union arm: scans `tables` (aliased positionally from `base`),
/// binds `?x` to the first scan, and applies `conds`.
fn arm(base: usize, tables: &[&str], conds: Vec<SqlCond>) -> Branch {
    let mut b = Branch::empty();
    for (i, t) in tables.iter().enumerate() {
        b.core.push(scan(base + i, t));
    }
    b.bindings.insert("x".into(), col_binding(base, "x"));
    b.where_conds = conds;
    b
}

/// The sorted table names a branch's core scans read.
fn core_tables(b: &Branch) -> Vec<String> {
    let mut ts: Vec<String> = b
        .core
        .iter()
        .filter_map(|s| match &s.source {
            LogicalSource::Table(t) => Some(t.clone()),
            LogicalSource::Query(_) => None,
        })
        .collect();
    ts.sort();
    ts
}

/// **GREEN** — `NodeDeletionTest.testSimpleJoin`. A single inner join whose
/// condition is FALSE is unsatisfiable; the whole query is empty
/// (`optimizedQuery.getTree().isDeclaredAsEmpty()`). sf drops the only arm, so
/// the bag union is empty.
#[test]
fn node_deletion_simple_join_false_condition_empties_query() {
    let mut conds = vec![SqlCond::ColEq(ColRef::new(0, "x"), ColRef::new(1, "x"))];
    conds.extend(false_condition(0));
    let b = arm(0, &["table1", "table2"], conds);

    let out = run(vec![b], &schema(), &CascadeCtx::default());

    assert!(
        out.is_empty(),
        "an inner join with an unsatisfiable (FALSE) condition yields no rows → \
         the whole query is empty (isDeclaredAsEmpty), got {} surviving arm(s)",
        out.len()
    );
}

/// **GREEN** — `NodeDeletionTest.testUnion1`. A 3-arm union where two arms are
/// inner joins with a FALSE condition: both empty arms are deleted, and the
/// union collapses to its single surviving arm (`expectedIQ == dataNode1`).
#[test]
fn node_deletion_union_drops_all_empty_arms_collapsing_to_single() {
    let arms = vec![
        arm(0, &["table1"], Vec::new()), // satisfiable
        arm(
            10,
            &["table2", "table3"],
            with_join(10, false_condition(10)),
        ), // FALSE → empty
        arm(
            20,
            &["table4", "table5"],
            with_join(20, false_condition(20)),
        ), // FALSE → empty
    ];

    let out = run(arms, &schema(), &CascadeCtx::default());

    assert_eq!(
        out.len(),
        1,
        "both FALSE-condition arms deleted; union collapses to the one survivor"
    );
    assert_eq!(
        core_tables(&out[0]),
        vec!["table1"],
        "the surviving arm is the satisfiable `table1` scan"
    );
}

/// **GREEN** — `NodeDeletionTest.testUnion2`. A 3-arm union where only the
/// middle arm has a FALSE join: that one arm is deleted, the other two
/// (`table1`, and the satisfiable `table4 ⨝ table5`) survive.
#[test]
fn node_deletion_union_drops_only_the_empty_arm() {
    let arms = vec![
        arm(0, &["table1"], Vec::new()), // satisfiable
        arm(
            10,
            &["table2", "table3"],
            with_join(10, false_condition(10)),
        ), // FALSE → empty
        arm(20, &["table4", "table5"], with_join(20, Vec::new())), // satisfiable join
    ];

    let out = run(arms, &schema(), &CascadeCtx::default());

    assert_eq!(
        out.len(),
        2,
        "only the single FALSE-condition arm is deleted"
    );
    assert_eq!(
        core_tables(&out[0]),
        vec!["table1"],
        "first survivor: the `table1` scan"
    );
    assert_eq!(
        core_tables(&out[1]),
        vec!["table4", "table5"],
        "second survivor: the satisfiable `table4 ⨝ table5` join is preserved intact"
    );
}

/// **GREEN** — `NodeDeletionTest.testInvalidLeftPartOfLeftJoin`. A LEFT JOIN
/// whose **preserved (left) side** is an unsatisfiable inner join is empty
/// (an empty preserved side ⇒ no rows, regardless of the optional). sf drops
/// the branch even though an `OptJoin` is attached (pass 1 inspects the core
/// `where_conds`).
#[test]
fn node_deletion_invalid_left_part_of_left_join_empties_query() {
    // Preserved side: table2 ⨝ table3 with a FALSE condition.
    let mut b = arm(0, &["table2", "table3"], with_join(0, false_condition(0)));
    // OPTIONAL right side: table4, null-safe joined on the (dead) core column.
    b.opts.push(OptJoin {
        scan: scan(2, "table4"),
        on: vec![SqlCond::NullSafeEq(
            ColRef::new(0, "x"),
            ColRef::new(2, "x"),
        )],
        extra: Vec::new(),
    });

    let out = run(vec![b], &schema(), &CascadeCtx::default());

    assert!(
        out.is_empty(),
        "empty preserved (left) side of a LEFT JOIN ⇒ empty result, got {} arm(s)",
        out.len()
    );
}

/// `ColEq(base.x, (base+1).x)` join equality prepended to `tail` — the inner
/// join over the two scans an arm reads.
fn with_join(base: usize, tail: Vec<SqlCond>) -> Vec<SqlCond> {
    let mut conds = vec![SqlCond::ColEq(
        ColRef::new(base, "x"),
        ColRef::new(base + 1, "x"),
    )];
    conds.extend(tail);
    conds
}
