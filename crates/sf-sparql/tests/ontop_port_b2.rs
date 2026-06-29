//! Ontop-parity oracle port — batch 2 of 8 (ADR-0022).
//!
//! Assigned slice `[10, 15)` over the combined, path-sorted `*Test.java` listing
//! of `~/source/ontop/core/optimization/src/test/java/it/unibz/inf/ontop/iq/{executor,optimizer}/`
//! (33 files, indices 0..=32). The slice resolves to five `iq/optimizer` classes:
//!
//!   * index 10 — `ConjunctionOfDisjunctionsMergingTest.java` ( 5 scenarios)
//!   * index 11 — `ConstructionNodeCleanerTest.java`          (12 scenarios)
//!   * index 12 — `DistinctTest.java`                         (21 scenarios)
//!   * index 13 — `ExpressionEvaluatorTest.java`              (15 scenarios)
//!   * index 14 — `FlattenLiftTest.java`                      (16 scenarios)
//!
//! scenarios_total = 5 + 12 + 21 + 15 + 16 = **69**.
//!
//! Of the 69, exactly **one** lands on a `=_bag` rewrite sf's cascade already
//! performs — the *no-op direction* of pass 6 (`distinct_removal`,
//! `crates/sf-sparql/src/cascade/mod.rs`). It is ported GREEN below
//! (`ontop_distinct_preserved_when_projected_term_is_not_a_key`). The other 68 are
//! NEEDS_IMPL (25) or BOUNDARY (43); none is a SUPPORTED scenario that *diverges*,
//! so **converted_red = 0**. The mismatch is structural: sf's post-unfold
//! [`Branch`](sf_sparql::iq::Branch) is a *flat* SQL `SELECT` (core base-table
//! `Scan`s + `opts` LEFT JOINs + one `bindings` map + `where_conds`), with **no**
//! `UnionNode` (a UNION is a `Vec<Branch>`), `ValuesNode`, `FlattenNode`, nested
//! `ConstructionNode`, `SliceNode`, or `IntensionalDataNode` for an IQ-tree pass to
//! rewrite. The six cascade passes are: 1 IRI-template prune · 2 self-/self-left-
//! join elim · 2c same-terms elim · 3 FD inference · 4 FK/PK join elim · 5
//! selection pushdown · 6 single-scan DISTINCT removal.
//!
//! Per-class classification (provenance cited against Ontop 5.5.0 source):
//!
//! ## index 10 — `ConjunctionOfDisjunctionsMergingTest` — 5 — all NEEDS_IMPL
//!
//! Boolean-filter simplification over a conjunction/disjunction of strict
//! equalities. sf's pass 5 only *flattens* a top-level `AND` and stable-partitions
//! single-scan selections (`cascade::selection_pushdown`); it has no
//! CNF/DNF merge, absorption, `IN`-coalescing, or disjoint-disjunction
//! unsatisfiability reasoning. All `=_bag`-safe, all absent:
//!
//! * `mergingTest1` — `(A∈{X,Y,Z}) ∧ (B∈{V,W}) ∧ (A∈{W,X})` ⇒ `A="X" ∧ B IN(V,W)`
//!   (intersect the A-disjunctions to the singleton {X}, coalesce B to an `IN`).
//! * `mergingTest2` — `(A∈{%,[%],perc}) ∧ (A∈{W/m²,W/m2,W/mq})` ⇒ **EmptyNode**
//!   (the two A-disjunctions are value-disjoint ⇒ unsatisfiable). sf's pass-1 prune
//!   only fires on two *top-level* `=`-constants on one column, never on a pair of
//!   disjunctions, so it cannot detect this emptiness.
//! * `mergingTest3` — distribute/merge `OR(AND(..),AND(..),AND(..))` into
//!   `AND(OR(..), IN(A,..), OR(..), IN(B,..))`.
//! * `mergingTest4` — merge to `OR(AND(IN(B..),A=X), AND(B=W,IN(A..)))`.
//! * `mergingTest5` — the same CNF simplification underneath a 4-way join +
//!   `ConstructionNode(lower(..))`; also needs constant propagation into scans.
//!
//! ## index 11 — `ConstructionNodeCleanerTest` — 12 — all BOUNDARY
//!
//! `removeConstructionNodeTest1`..`12` each merge/relocate *consecutive*
//! `ConstructionNode`s (projection + substitution) over `IntensionalDataNode`s,
//! lifting substitutions through `SliceNode` (LIMIT), `DistinctNode`, and
//! `UnionNode`. sf resolves all term construction at *unfold* time into a single
//! flat `Branch::bindings` map — there are no stacked construction nodes to clean
//! and no intensional (unresolved) triple atoms in the cascade. Pure Ontop
//! IQ-tree scaffolding (ADR-0004); no base-scan-cascade analogue exists.
//!
//! ## index 12 — `DistinctTest` — 21 — 1 SUPPORTED · 19 NEEDS_IMPL · 1 BOUNDARY
//!
//! * **SUPPORTED → GREEN** — `testDistinctConstructionConstant2`: DISTINCT over a
//!   single `PK_TABLE1_AR2` scan projecting `A:=<const>` and `B:=col1` (a NON-key
//!   column — the PK is col0). The DISTINCT is **not** redundant (duplicate `B`
//!   possible) so Ontop keeps it; sf's pass 6 likewise refuses to drop it. Ported
//!   below.
//! * NEEDS_IMPL (19):
//!   * `testDistinctConstructionConstant1` — DISTINCT of an all-constant
//!     projection over a non-empty table ⇒ `SLICE(0,1)` (LIMIT 1). sf has no
//!     constant-projection ⇒ limit-1 rule.
//!   * `testDistinctJoin1,2,3,5,6,7` — **multi-scan DISTINCT removal**: a DISTINCT
//!     over a join is redundant once every joined relation contributes a projected
//!     key (cross-table uniqueness/FD closure). sf's pass 6 bails on a multi-scan
//!     core (`core.len()==1` guard) — the central gap, encoded as the `#[ignore]`d
//!     RED spec below.
//!   * `testDistinctJoin4` — as above but DISTINCT is *kept* while an unused data
//!     node is projection-shrunk (`PK_TABLE3_AR2` loses its bound col).
//!   * `testDistinctUnion1,3,4,5` — DISTINCT over a UNION-with-`ValuesNode` ⇒ per-
//!     arm `SLICE(0,1)` + dedup of the inline `VALUES` rows (`[2,2,3]`→`[2,3]`).
//!   * `testDistinctUnion2,12` — same, but a non-deterministic term
//!     (`getDBRowUniqueStr`/`getDBRand`) blocks the per-arm slice; the `VALUES`
//!     arm is still deduped / shrunk.
//!   * `testDistinctUnion6,8,9` — DISTINCT preserved over a UNION while the arms'
//!     data nodes are projection-shrunk via FD inference *through the union*.
//!   * `testDistinctUnion10,11` — FD inference from the union (`@Ignore`d in Ontop:
//!     "TODO: support FD inference from the union for this case").
//! * BOUNDARY (1):
//!   * `testDistinctUnion7` — DISTINCT over a UNION with no projected key is a pure
//!     no-op (Ontop returns it unchanged). sf *also* preserves it — but a
//!     multi-branch DISTINCT is enforced by exec-layer dedup
//!     (`exec::for_each_solution`), not a per-`Branch` cascade flag, so there is no
//!     single-`Branch` cascade assertion to make. Correct-in-spirit, not
//!     cascade-expressible.
//!
//! ## index 13 — `ExpressionEvaluatorTest` — 15 — 1 NEEDS_IMPL · 14 BOUNDARY
//!
//! * NEEDS_IMPL (1) — `testNonEqualOperatorDistribution`: IRI-template
//!   injectivity — `NEQ(uri2(A,B), uri2(C,D))` ⇒ `OR(A≠C, B≠D)` (the bug it pins:
//!   it must be `OR`, not `AND`). An `=_bag`-safe expression rewrite sf's cascade
//!   does not perform (it also presumes binding-lift through the join).
//! * BOUNDARY (14):
//!   * `testLangLeftNodeFunction`, `testLangRightNode` — `UNION_AND_BINDING_LIFT` +
//!     `JOIN_LIKE` lift `ConstructionNode` substitutions up through a join and
//!     evaluate `LANGMATCHES(LANG(?w), "en-us")`. Ontop binding-lift framework +
//!     SPARQL function evaluation; sf folds construction into unfold (no lift pass).
//!   * `testIsNotNullUri1`..`4`, `testIsNotNullUriTrickyCase`, `testIsNullUri1`..
//!     `4`, `testIfElseNull1`..`3` (12) — direct unit tests of
//!     `ImmutableExpression.evaluate(...)` (e.g. `IS NOT NULL uri2(X,Y)` ⇒
//!     `IS NOT NULL X ∧ IS NOT NULL Y`; `IS NOT NULL uri1("toto")` ⇒ TRUE). These
//!     test Ontop's term evaluator as a sub-component, not an IQ→IQ transform; sf
//!     has no standalone evaluator API (null-rejection over IRI templates is folded
//!     into translation), so there is no cascade run to assert against.
//!
//! ## index 14 — `FlattenLiftTest` — 16 — all BOUNDARY
//!
//! Every scenario lifts a `FlattenNode` (JSON-array UNNEST / lateral flatten) above
//! a join / left-join / construction, or splits a join condition around a flatten
//! (`testLiftFlatten1`..`3`, `testLiftFlattenAndJoinCondition1`/`2`,
//! `testFlattenAndJoinCondition3`, `testLiftDoubleFlatten`,
//! `testLift{Left,Right,LeftAndRight}FlattenWithLeftJoin`, `testNoLiftLeftJoin`,
//! `testConsecutiveFlatten1`/`2`(@Ignore), `testLiftAboveConstruct`,
//! `testNonLiftAboveConstructDueTo{Projection,SubstitutionRange}`). sf's IQ model
//! has **no** `FlattenNode` — no nested/array data and no lateral UNNEST — so none
//! is expressible. Out of charter (ADR-0004).
//!
//! ---
//! The single GREEN port plus its load-bearing positive baseline and the central
//! NEEDS_IMPL spec follow. See `src/cascade/ws_g.rs` / `ws_st.rs` / `ws_fk.rs` for
//! the broader port pattern.

use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_core::{NamedNode, Term};
use sf_sparql::cascade::{run, CascadeCtx};
use sf_sparql::iq::{Branch, Scan, TermDef};
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

/// A constant binding (`rr:constant` / a bound query constant). Its value and type
/// are immaterial to pass 6 — a constant reads **no** column, so it can never be a
/// key. Stands in for Ontop's `A := ONE`.
fn const_binding() -> TermDef {
    TermDef::Const(Term::NamedNode(NamedNode::new_unchecked("http://ex/one")))
}

/// `PK_TABLE1_AR2` analogue: a 2-column table with the PK on `c0`. Ontop's `PK_`
/// tables are created with `canBeNull = false`, so **both** columns are NOT NULL —
/// but only `c0` is a unique key, so `single_col_keys` returns just `c0` and the
/// non-key `c1` cannot make a DISTINCT redundant.
fn pk_table1() -> Vec<TableSchema> {
    let mut t = TableSchema::new("pk_table1");
    t.primary_key = vec!["c0".into()];
    t.columns = vec![
        Column::new("c0", "text", true), // PK ⇒ NOT NULL, the only unique key
        Column::new("c1", "text", true), // NOT NULL, but NOT a key
    ];
    vec![t]
}

/// **GREEN** — Ontop `DistinctTest.testDistinctConstructionConstant2`.
///
/// `DISTINCT` over a single `PK_TABLE1_AR2` scan, projecting `A := <const>` and
/// `B := col1` (a NON-key column — the PK is col0). Ontop keeps the `DISTINCT`
/// (relocating it below the `ConstructionNode`): duplicate `B` rows are possible,
/// so the `DISTINCT` is **not** redundant. sf's cascade pass 6 (`distinct_removal`)
/// must likewise REFUSE to drop it — no projected term is built from the unique key
/// `c0`. This is the no-op direction of the redundant-DISTINCT pass (mirrors the
/// guard style of `ws_g::ontop_self_left_join_not_eliminated_on_nullable_key`).
#[test]
fn ontop_distinct_preserved_when_projected_term_is_not_a_key() {
    let mut b = Branch::single(scan(0, "pk_table1"));
    b.bindings.insert("A".into(), const_binding()); // A := constant (reads no column)
    b.bindings.insert("B".into(), col_binding(0, "c1")); // B := non-key column

    let ctx = CascadeCtx {
        distinct: true,
        project: Some(&["A".to_owned(), "B".to_owned()]),
    };
    let out = run(vec![b], &pk_table1(), &ctx);
    assert_eq!(out.len(), 1);
    assert!(
        out[0].distinct,
        "DISTINCT must be PRESERVED — neither projected term (A const, B=c1 non-key) \
         is built from the unique key c0, so duplicates are possible (=_bag)"
    );
}

/// **GREEN (positive baseline).** The load-bearing contrast that makes the guard
/// above bite: the SAME single scan, but now `B := c0` (the NOT-NULL unique key).
/// `DISTINCT` over a projected key is redundant, so pass 6 *fires* and drops it
/// (`b.distinct ← false`). This is the single-scan analogue of
/// `DistinctTest.testDistinctJoin1`'s key-driven removal — which over a *join* sf
/// does not yet perform (see the `#[ignore]`d spec below). Not a 1:1 Ontop port;
/// included so the "preserved" assertion is provably non-vacuous.
#[test]
fn sf_distinct_removed_when_a_projected_term_is_the_unique_key_baseline() {
    let mut b = Branch::single(scan(0, "pk_table1"));
    b.bindings.insert("A".into(), const_binding());
    b.bindings.insert("B".into(), col_binding(0, "c0")); // B := the unique key

    let ctx = CascadeCtx {
        distinct: true,
        project: Some(&["A".to_owned(), "B".to_owned()]),
    };
    let out = run(vec![b], &pk_table1(), &ctx);
    assert!(
        !out[0].distinct,
        "DISTINCT over a projected NOT-NULL unique key (c0) is redundant ⇒ pass 6 removes it"
    );
}

/// Three PK tables (`PK_TABLE1/2/3_AR2`), PK on `c0`, all columns NOT NULL.
fn three_pk_tables() -> Vec<TableSchema> {
    let mk = |name: &str| {
        let mut t = TableSchema::new(name);
        t.primary_key = vec!["c0".into()];
        t.columns = vec![
            Column::new("c0", "text", true),
            Column::new("c1", "text", true),
        ];
        t
    };
    vec![mk("pk_t0"), mk("pk_t1"), mk("pk_t2")]
}

/// **NEEDS_IMPL spec (RED, `#[ignore]`d).** Ontop `DistinctTest.testDistinctJoin1`.
///
/// `DISTINCT` over the cross product of three PK tables, projecting `A:=t0.c0(PK)`,
/// `B:=t0.c1`, `C:=t1.c0(PK)`, `D:=t2.c0(PK)`. Each relation contributes its PK to
/// the projection ⇒ every output tuple is unique ⇒ Ontop removes the `DISTINCT`.
/// sf's pass 6 bails on a multi-scan core (it only proves redundancy for a single
/// base-table scan), so it currently KEEPS the `DISTINCT`. This asserts the DESIRED
/// post-impl state (`distinct == false`) and is `#[ignore]`d (RED) until multi-scan
/// FD-closure DISTINCT removal lands. Run with `cargo test -- --ignored`.
#[test]
#[ignore = "NEEDS_IMPL: pass 6 distinct_removal is single-scan only; multi-scan \
            DISTINCT-over-join removal (every joined relation contributes a projected \
            key) is not yet implemented — DistinctTest.testDistinctJoin1"]
fn ontop_distinct_over_join_removed_when_all_relations_contribute_a_key_spec() {
    let mut b = Branch::single(scan(0, "pk_t0"));
    b.core.push(scan(1, "pk_t1"));
    b.core.push(scan(2, "pk_t2"));
    b.bindings.insert("A".into(), col_binding(0, "c0")); // PK of t0
    b.bindings.insert("B".into(), col_binding(0, "c1")); // non-key of t0
    b.bindings.insert("C".into(), col_binding(1, "c0")); // PK of t1
    b.bindings.insert("D".into(), col_binding(2, "c0")); // PK of t2

    let ctx = CascadeCtx {
        distinct: true,
        project: Some(&[
            "A".to_owned(),
            "B".to_owned(),
            "C".to_owned(),
            "D".to_owned(),
        ]),
    };
    let out = run(vec![b], &three_pk_tables(), &ctx);
    assert_eq!(out.len(), 1);
    assert!(
        !out[0].distinct,
        "DISTINCT is redundant: t0.c0, t1.c0, t2.c0 are projected keys of a cross product \
         ⇒ every output tuple is unique (DESIRED multi-scan removal)"
    );
}
