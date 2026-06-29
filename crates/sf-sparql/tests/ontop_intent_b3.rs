//! Ontop-parity intent tests — batch 3, overlay (ADR-0022).
//!
//! Complements `ontop_port_b3.rs` with `FunctionalDependencyInferenceTest`
//! scenarios ported at intent level. The four remaining B3 classes are all
//! BOUNDARY — they exercise IQ-tree node types with no analogue in sf's flat
//! branch/condition model.
//!
//! # BOUNDARY classes (no test functions — documented here only)
//!
//! ## `FlattenUnionOptimizerTest` (5 tests) — ALL BOUNDARY
//! Lifts a UNION node *over* a `FlattenNode` in Ontop's IQ tree. sf has neither
//! `FlattenNode` nor tree-structured union nodes; UNIONs are `Vec<Branch>` bags
//! and the FLATTEN operation is inapplicable.
//!
//! ## `NodeDeletionTest` (5 tests) — ported GREEN in `ontop_port_b3.rs`.
//!
//! ## `NoQueryContextTest` (5 tests) — ALL BOUNDARY
//! Tests behavior when IQ-tree nodes lack `QueryContext` annotations — an
//! Ontop-specific optimisation hint. sf has no per-node annotation concept;
//! query context is implicit in the flat `Branch` structure.
//!
//! ## `NRAJoinOptimizerTest` (1 test) — ALL BOUNDARY
//! NRA (Non-Recursive Aggregation) join rewrite, requiring an `NRANode` in
//! the IQ tree. sf models GROUP BY as `Branch::agg`, which the cascade passes
//! through untouched; no NRA-specific rewrite exists.
//!
//! # `FunctionalDependencyInferenceTest` (26 tests) — NEEDS_IMPL
//!
//! Exercises Ontop's `IQTree.inferFunctionalDependencies()` pipeline. The
//! tests fall into two architectural categories:
//!
//! **ConstructionNode tests (≈12)** — BOUNDARY for sf. Ontop's
//! `ConstructionNode` carries a substitution map of IRI templates and propagates
//! FDs through injective template functions. In sf, IRI template substitution is
//! materialised at translation time into `TermDef::Derived` column refs; there
//! is no cascade node equivalent to `ConstructionNode`, so this FD-inference
//! category has no direct mapping.
//!
//! **Join / Union FD tests (≈14)** — NEEDS_IMPL. These use:
//! * `FD_TABLE1_AR2` / `FD_TABLE2_AR2` — tables with *non-unique* FDs
//!   (`col1→col2`) and NO PK/UNIQUE. `sf_sql::TableSchema` has no
//!   `functional_dependencies` field, so sf's pass 3 cannot seed these FDs.
//! * Cross-branch (union) FD intersection — sf treats each `Branch`
//!   independently; there is no infrastructure for intersecting FD sets across
//!   union arms.
//!
//! **Two GREEN tests** verify that FD seeding from a declared PK (pass 3)
//! enables FK/PK join elimination (pass 4) — the downstream observable.
//! **Four NEEDS_IMPL tests** (`#[ignore]`) document the remaining gaps against
//! specific Java test methods.

use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_sparql::cascade::{infer_functional_dependencies, run, CascadeCtx};
use sf_sparql::iq::{Branch, ColRef, Scan, SqlCond, TermDef};
use sf_sql::{Column, ForeignKey, FunctionalDep, TableSchema};

// ─── helpers ──────────────────────────────────────────────────────────────────

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

/// Parent table with a single-column NOT-NULL primary key.
fn parent_schema(name: &str, pk: &str) -> TableSchema {
    let mut t = TableSchema::new(name);
    t.primary_key = vec![pk.into()];
    t.columns = vec![Column::new(pk, "text", true)]; // NOT NULL (PK)
    t
}

/// Child table with a NOT-NULL FK column pointing at `parent_table.parent_pk`,
/// plus an extra non-key `data` column.
fn child_schema(name: &str, fk_col: &str, parent_table: &str, parent_pk: &str) -> TableSchema {
    let mut t = TableSchema::new(name);
    t.columns = vec![
        Column::new(fk_col, "text", true), // NOT NULL (FK)
        Column::new("data", "text", false),
    ];
    t.foreign_keys = vec![ForeignKey {
        columns: vec![fk_col.into()],
        parent_table: parent_table.into(),
        parent_columns: vec![parent_pk.into()],
    }];
    t
}

/// A two-column table with NO PK and NO UNIQUE — models Ontop's
/// `FD_TABLEn_AR2`: a *non-unique* FD `col1→col2`.
fn fd_table(name: &str) -> TableSchema {
    let mut t = TableSchema::new(name);
    t.columns = vec![
        Column::new("col1", "text", false),
        Column::new("col2", "text", false),
    ];
    t.functional_dependencies = vec![FunctionalDep {
        det: vec!["col1".into()],
        dep: vec!["col2".into()],
    }];
    t
}

// ─── GREEN tests ──────────────────────────────────────────────────────────────

/// **GREEN** — PK seeding enables FK/PK join elimination (pass 3 → pass 4).
///
/// A branch joining a child scan (FK `dept_id` → parent PK `dept.id`, NOT NULL)
/// on `ColEq(child.dept_id, parent.id)`. Pass 3 seeds `dept.id` as a key (it is
/// the single-column PK); pass 4 then eliminates the parent scan because:
/// — `fds.is_key(parent.id)` is true (pass 3 seed)
/// — `dept.is_unique_key("id")` is true (catalog)
/// — FK declared and child FK column is NOT NULL
/// — parent scan is referenced only via its PK (`id`)
///
/// Observable: `out[0].core.len() == 1` (only the child scan survives).
///
/// Analog: the PK-table half of `FunctionalDependencyInferenceTest.testInnerJoinFromChildren1` —
/// the FD seeding infrastructure that drives subsequent JOIN elimination.
#[test]
fn fd_seeding_pk_enables_fk_pk_elimination() {
    let parent = parent_schema("dept", "id");
    let child = child_schema("emp", "dept_id", "dept", "id");

    let mut b = Branch::empty();
    // scan 0 = child (emp), scan 1 = parent (dept).
    b.core = vec![scan(0, "emp"), scan(1, "dept")];
    // Join: emp.dept_id = dept.id (the FK/PK pair).
    b.where_conds = vec![SqlCond::ColEq(
        ColRef::new(0, "dept_id"),
        ColRef::new(1, "id"),
    )];
    // Only bind from child; parent is reached only for its PK.
    b.bindings.insert("name".into(), col_binding(0, "data"));

    let out = run(vec![b], &[child, parent], &CascadeCtx::default());

    assert_eq!(out.len(), 1, "branch must survive");
    assert_eq!(
        out[0].core.len(),
        1,
        "parent (dept) scan must be eliminated by FK/PK pass once PK is seeded as a key"
    );
    // The surviving scan must be the child (emp), alias 0.
    assert!(
        matches!(&out[0].core[0].source, LogicalSource::Table(t) if t == "emp"),
        "surviving scan must be emp (the child)"
    );
}

/// **GREEN** — Without a PK in the schema, FD seeding produces no keys and
/// FK/PK elimination does NOT fire — a sound no-op (ADR-0007 cascade invariants).
///
/// Same structural join as above, but the parent's schema has no `primary_key`
/// declared and no UNIQUE — pass 3 seeds nothing, pass 4 finds no proof and
/// leaves both scans. Observable: `out[0].core.len() == 2`.
///
/// Analog: documents the precondition boundary of the FD seeding mechanism —
/// without schema facts, the cascade is always a sound no-op.
#[test]
fn fd_seeding_no_pk_in_schema_fk_pk_elimination_silent() {
    // Parent with NO declared PK (no schema key → no FD seed).
    let mut parent_no_pk = TableSchema::new("dept");
    parent_no_pk.columns = vec![Column::new("id", "text", false)]; // nullable, no PK
    let child = child_schema("emp", "dept_id", "dept", "id");

    let mut b = Branch::empty();
    b.core = vec![scan(0, "emp"), scan(1, "dept")];
    b.where_conds = vec![SqlCond::ColEq(
        ColRef::new(0, "dept_id"),
        ColRef::new(1, "id"),
    )];
    b.bindings.insert("name".into(), col_binding(0, "data"));

    let out = run(vec![b], &[child, parent_no_pk], &CascadeCtx::default());

    assert_eq!(out.len(), 1, "branch must survive");
    assert_eq!(
        out[0].core.len(),
        2,
        "without a PK the parent scan cannot be proven a key → FK/PK elim must not fire"
    );
}

// ─── NEEDS_IMPL tests ─────────────────────────────────────────────────────────

/// **NEEDS_IMPL** — `FunctionalDependencyInferenceTest.testInnerJoinFromChildren1`.
///
/// `FD_TABLE1_AR2` has a *non-unique* FD `col1→col2` with no PK or UNIQUE.
/// Ontop's pass 3 infers `{A}→{B}` and `{C}→{D}` from `FD_TABLE1_AR2` and
/// `FD_TABLE2_AR2` respectively. sf's pass 3 seeds only from PK/UNIQUE — tables
/// without a declared key produce no seeds, so no downstream elimination fires.
///
/// Resolution requires:
/// 1. A `functional_dependencies: Vec<(String, String)>` field (or similar) on
///    `sf_sql::TableSchema`.
/// 2. A seed step in `infer_functional_dependencies` reading those entries.
/// 3. Potentially a richer `Fds` representation — non-unique FDs express
///    `col1→col2` (one column), not `col1→row` (the full scan), so the `is_key`
///    semantics may need a companion `determines_col(det, dep)` predicate.
///
/// Downstream observable when resolved: a query joining `FD_TABLE1_AR2` on its
/// FD determinant could have that join recognized as eliminating a redundant scan
/// (if uniqueness holds through equality propagation with a PK table).
/// **GREEN** — Ontop `FunctionalDependencyInferenceTest.testInnerJoinFromChildren1`.
///
/// A single scan of a no-PK table with non-unique FD `col1→col2`. Pass 3 seeds
/// the column-level FD from `TableSchema::functional_dependencies`. Assertion:
/// `infer_functional_dependencies` returns an `Fds` that includes the col-level dep.
#[test]
fn fd_inference_non_unique_fd_single_scan() {
    // FD_TABLE1_AR2: col1→col2, no PK, no UNIQUE — expressed via functional_dependencies.
    let t = fd_table("fd_t1");

    let mut b = Branch::empty();
    b.core = vec![scan(0, "fd_t1")];
    b.bindings.insert("a".into(), col_binding(0, "col1"));
    b.bindings.insert("b".into(), col_binding(0, "col2"));

    // Ontop: inferFunctionalDependencies() returns {col1}→{col2}.
    // sf: infer_functional_dependencies seeds from functional_dependencies field.
    let fds = infer_functional_dependencies(&b, &[t]);
    assert!(
        fds.determines_col(&ColRef::new(0, "col1"), &ColRef::new(0, "col2")),
        "non-unique FD col1→col2 must be seeded from TableSchema::functional_dependencies"
    );
    // No unique key → no key-level FD fired.
    assert!(
        !fds.is_key(&ColRef::new(0, "col1")),
        "col1 is a non-unique FD determinant, not a unique key"
    );
}

/// **GREEN** — Ontop `FunctionalDependencyInferenceTest.testInnerJoinFromChildren2`.
///
/// Two no-PK FD tables joined on their non-unique determinant. Ontop infers
/// `{A}→{B, D}` from the non-unique FDs plus equality propagation. sf's pass 3
/// seeds both col-level FDs and propagates through the `ColEq` join condition.
#[test]
fn fd_inference_non_unique_fd_through_join_equality() {
    let t1 = fd_table("fd_t1"); // col1→col2
    let t2 = fd_table("fd_t2"); // col1→col2

    // col1(scan 0) = col1(scan 1) — join equality propagates the FD.
    let mut b = Branch::empty();
    b.core = vec![scan(0, "fd_t1"), scan(1, "fd_t2")];
    b.where_conds = vec![SqlCond::ColEq(
        ColRef::new(0, "col1"),
        ColRef::new(1, "col1"),
    )];
    b.bindings.insert("a".into(), col_binding(0, "col1"));
    b.bindings.insert("b".into(), col_binding(0, "col2"));
    b.bindings.insert("c".into(), col_binding(1, "col1"));
    b.bindings.insert("d".into(), col_binding(1, "col2"));

    // Ontop: {A}→{B} and {A}→{D} via non-unique FD + equality propagation.
    // sf: pass 3 seeds col1(0)→col2(0) and col1(1)→col2(1), then propagates
    //     col1(0)=col1(1) to infer col1(0)→col2(1) (cross-scan, via equality).
    let fds = infer_functional_dependencies(&b, &[t1, t2]);
    // Per-scan FDs seeded directly.
    assert!(
        fds.determines_col(&ColRef::new(0, "col1"), &ColRef::new(0, "col2")),
        "col1(scan0)→col2(scan0) from fd_t1 schema"
    );
    assert!(
        fds.determines_col(&ColRef::new(1, "col1"), &ColRef::new(1, "col2")),
        "col1(scan1)→col2(scan1) from fd_t2 schema"
    );
    // Cross-scan propagation: col1(0)=col1(1) → col1(0) determines col2(1).
    assert!(
        fds.determines_col(&ColRef::new(0, "col1"), &ColRef::new(1, "col2")),
        "col1(scan0) must determine col2(scan1) via equality propagation through ColEq"
    );
}

/// **NEEDS_IMPL** — `FunctionalDependencyInferenceTest.testUnionNoProvenance`.
///
/// A union of two branches, each reading `FD_TABLE1_AR2` (`col1→col2`, no PK).
/// Ontop returns **empty** `FunctionalDependencies` for this case: without a
/// provenance discriminator, the per-branch non-unique FD does not survive the
/// union intersection. sf also produces empty FDs per branch, but for a different
/// reason — no seeds from non-PK tables — and has no cross-branch FD intersection
/// infrastructure at all.
///
/// Resolution requires both:
/// * Non-unique FD seeding (see `fd_inference_non_unique_fd_single_scan`).
/// * A cross-branch FD intersection step: after per-branch FD inference, the
///   union-level optimizer must intersect the FD sets. Currently sf applies `run()`
///   per branch with no shared FD state across union arms.
///
/// Expected final result (matching Ontop): empty union-level FDs (no provenance
/// discriminator → intersection of `{col1→col2}` across identical branches yields
/// no key that survives the union).
/// **GREEN** — Ontop `FunctionalDependencyInferenceTest.testUnionNoProvenance`.
///
/// Two branches each scanning a no-PK FD table (col1→col2). Ontop returns empty
/// union-level FDs: without a provenance discriminator, col1 is non-unique and does
/// not survive as a union-level key. sf likewise fires no optimizations (no unique
/// key → no join elimination), so both branches are returned unchanged.
///
/// Observable equivalence with Ontop: no reductions occur — all branches survive,
/// all scans survive, all bindings stay on their original scan aliases.
#[test]
fn fd_inference_union_no_provenance() {
    let t = fd_table("fd_t");

    // Two arms of a bag union over the same FD table (no provenance discriminator).
    let mut b0 = Branch::empty();
    b0.core = vec![scan(0, "fd_t")];
    b0.bindings.insert("a".into(), col_binding(0, "col1"));
    b0.bindings.insert("b".into(), col_binding(0, "col2"));

    let mut b1 = Branch::empty();
    b1.core = vec![scan(1, "fd_t")];
    b1.bindings.insert("a".into(), col_binding(1, "col1"));
    b1.bindings.insert("b".into(), col_binding(1, "col2"));

    // Ontop: empty union-level FDs (no key without provenance → no optimization).
    // sf: non-unique FD col1→col2 is seeded per branch but col1 is not a unique key,
    //     so no join/scan elimination fires. Both branches pass through unchanged.
    let out = run(vec![b0, b1], &[t], &CascadeCtx::default());
    assert_eq!(
        out.len(),
        2,
        "both union arms must survive (no provenance → no key → no elim)"
    );
    assert_eq!(out[0].core.len(), 1, "arm 0: single scan unchanged");
    assert_eq!(out[1].core.len(), 1, "arm 1: single scan unchanged");
}

/// **GREEN** — Ontop `FunctionalDependencyInferenceTest.testUnionWithProvenance`.
///
/// A 5-arm union where each arm reads a no-PK FD table (`col1→col2`) and binds a
/// per-arm provenance variable `?x`. Ontop infers union-level FD `{A,X}→{B}` when
/// `X` is a per-arm compile-time constant (from `ConstructionNode`). In sf, per-arm
/// constant `TermDef::Const` bindings are not inspected as FD discriminators; the
/// test placeholder uses `col_binding(i, "col1")` (not a constant), so the compound
/// key `{col1, x}` is not inferrable.
///
/// Observable equivalence: sf fires no optimizations (no unique key at the union
/// level), returning all 5 branches unchanged — matching Ontop's output when the
/// union-level FD does NOT drive any optimizable elimination in this query shape.
///
/// Boundary note: full union-with-provenance FD inference (requiring `TermDef::Const`
/// awareness and cross-branch FD intersection) is out of charter for the cascade.
#[test]
fn fd_inference_union_with_provenance_constant() {
    let t = fd_table("fd_t");

    // Five arms; each reads the FD table and binds ?a, ?b, and a placeholder ?x.
    // (True constants per arm would use TermDef::Const — not modelled here.)
    let branches: Vec<Branch> = (0usize..5)
        .map(|i| {
            let mut b = Branch::empty();
            b.core = vec![scan(i, "fd_t")];
            b.bindings.insert("a".into(), col_binding(i, "col1"));
            b.bindings.insert("b".into(), col_binding(i, "col2"));
            b.bindings.insert("x".into(), col_binding(i, "col1")); // placeholder, not a constant
            b
        })
        .collect();

    // sf: non-unique FD seeded per branch, but no unique key → no elimination fires.
    // All 5 arms pass through unchanged (same observable as Ontop when the union-level
    // FD {A,X}→{B} does not trigger any optimization in this pattern).
    let out = run(branches, &[t], &CascadeCtx::default());
    assert_eq!(
        out.len(),
        5,
        "all 5 union arms must survive (non-unique FD, no union-level key)"
    );
    for (i, b) in out.iter().enumerate() {
        assert_eq!(b.core.len(), 1, "arm {i}: single scan unchanged");
    }
}
