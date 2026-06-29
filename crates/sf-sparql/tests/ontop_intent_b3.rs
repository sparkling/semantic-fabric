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
use sf_sparql::cascade::{run, CascadeCtx};
use sf_sparql::iq::{Branch, ColRef, Scan, SqlCond, TermDef};
use sf_sql::{Column, ForeignKey, TableSchema};

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
/// `FD_TABLEn_AR2`: a *non-unique* FD (`col1→col2`) that `TableSchema`
/// cannot express today.
fn fd_table(name: &str) -> TableSchema {
    let mut t = TableSchema::new(name);
    t.columns = vec![
        Column::new("col1", "text", false),
        Column::new("col2", "text", false),
    ];
    t // no primary_key, no unique, no functional_dependencies field
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
#[test]
#[ignore = "NEEDS_IMPL: non-unique functional_dependencies field absent from TableSchema — FunctionalDependencyInferenceTest.testInnerJoinFromChildren1"]
fn fd_inference_non_unique_fd_single_scan() {
    // FD_TABLE1_AR2: col1→col2, no PK, no UNIQUE — TableSchema cannot express this.
    let t = fd_table("fd_t1");

    let mut b = Branch::empty();
    b.core = vec![scan(0, "fd_t1")];
    b.bindings.insert("a".into(), col_binding(0, "col1"));
    b.bindings.insert("b".into(), col_binding(0, "col2"));

    // Ontop: inferFunctionalDependencies() returns {col1}→{col2}.
    // sf: currently run() is a no-op (no PK/UNIQUE → no seeds → no passes fire).
    // When NEEDS_IMPL resolved, the FD {col1→col2} should be seeded and
    // downstream optimizations that require this FD (e.g. extended distinctness
    // reasoning) should become available.
    let out = run(vec![b], &[t], &CascadeCtx::default());

    assert!(
        false,
        "NEEDS_IMPL: sf cannot seed non-unique FD col1→col2 from FD_TABLE1_AR2; \
         when TableSchema gains a functional_dependencies field and pass 3 reads it, \
         verify that the inferred FD enables whatever downstream optimization is added \
         (FunctionalDependencyInferenceTest.testInnerJoinFromChildren1)"
    );
    let _ = out;
}

/// **NEEDS_IMPL** — `FunctionalDependencyInferenceTest.testInnerJoinFromChildren2`.
///
/// Two FD tables joined on their non-unique determinant column with strict
/// equality (`A = D`): `FD_TABLE1_AR2(A,B) ⨝_{A=D} FD_TABLE2_AR2(C,D)`.
/// Ontop infers `{A}→{B, D}`: the non-unique `A→B` plus the join equality
/// `A = D` (propagating A's determinacy to D) plus transitivity via `D→C`
/// yields `{A}→{B, D, C}` in the full variant.
///
/// sf cannot replicate this because:
/// * Non-unique FD seeding is absent (see `fd_inference_non_unique_fd_single_scan`).
/// * Even if seeds existed, the equality rule in pass 3 only propagates already-
///   established `is_key` relationships — it cannot bootstrap from an empty seed.
#[test]
#[ignore = "NEEDS_IMPL: non-unique functional_dependencies field absent from TableSchema — FunctionalDependencyInferenceTest.testInnerJoinFromChildren2"]
fn fd_inference_non_unique_fd_through_join_equality() {
    let t1 = fd_table("fd_t1"); // col1→col2 (non-unique, no PK)
    let t2 = fd_table("fd_t2"); // col1→col2 (non-unique, no PK)

    // col1(scan 0) = col1(scan 1) — Ontop's "A = D" join equality.
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

    // Ontop: {A}→{B, D} (non-unique FD + equality propagation + transitivity).
    // sf: run() is a no-op — no seeds, no propagation, no passes fire.
    let out = run(vec![b], &[t1, t2], &CascadeCtx::default());

    assert!(
        false,
        "NEEDS_IMPL: sf cannot infer col1→{{col2, col1(t2), col2(t2)}} without non-unique FD \
         seeding; when resolved, verify that equality propagation in pass 3 correctly \
         derives the compound determinacy A→{{B, D}} \
         (FunctionalDependencyInferenceTest.testInnerJoinFromChildren2)"
    );
    let _ = out;
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
#[test]
#[ignore = "NEEDS_IMPL: cross-branch FD intersection absent; non-unique FDs also absent — FunctionalDependencyInferenceTest.testUnionNoProvenance"]
fn fd_inference_union_no_provenance() {
    let t = fd_table("fd_t");

    // Two separate branches (arms of the bag union) over the same FD table.
    let mut b0 = Branch::empty();
    b0.core = vec![scan(0, "fd_t")];
    b0.bindings.insert("a".into(), col_binding(0, "col1"));
    b0.bindings.insert("b".into(), col_binding(0, "col2"));

    let mut b1 = Branch::empty();
    b1.core = vec![scan(1, "fd_t")];
    b1.bindings.insert("a".into(), col_binding(1, "col1"));
    b1.bindings.insert("b".into(), col_binding(1, "col2"));

    // Ontop: empty FDs (non-unique FDs don't survive union without provenance).
    // sf: run() optimises each arm independently; no cross-branch FD API exists.
    let _out = run(vec![b0, b1], &[t], &CascadeCtx::default());

    assert!(
        false,
        "NEEDS_IMPL: sf has no cross-branch FD intersection; \
         when non-unique FDs and union-level FD analysis are implemented, verify that \
         a 2-arm union of FD_TABLE branches (no provenance) correctly yields empty \
         union-level FDs (FunctionalDependencyInferenceTest.testUnionNoProvenance)"
    );
}

/// **NEEDS_IMPL** — `FunctionalDependencyInferenceTest.testUnionWithProvenance`.
///
/// A 5-arm union where each arm reads `FD_TABLE1_AR2` (`col1→col2`, non-unique)
/// and binds a *different compile-time constant* to variable `X` (provenance
/// discriminator). Ontop infers `{A, X}→{B}`: the per-table non-unique FD `A→B`
/// combined with disjoint provenance constants (X is unique per arm) yields the
/// compound key `{A, X}` at the union level.
///
/// This is the most practically important union FD scenario in SPARQL-over-SQL:
/// R2RML mappings routinely produce one arm per triples-map class with a constant
/// class IRI as provenance — the pattern drives DISTINCT removal and join
/// optimizations across mapping arms.
///
/// sf lacks all three required mechanisms:
/// * Non-unique FD seeding (`FD_TABLE1_AR2` has no PK/UNIQUE).
/// * Constant bindings as FD discriminators — sf's cascade does not inspect
///   `TermDef::Const` bindings when reasoning about union-level key uniqueness.
/// * Cross-branch FD intersection with provenance awareness.
#[test]
#[ignore = "NEEDS_IMPL: union-with-provenance FD (non-unique FD + constant provenance as discriminator + cross-branch intersection) — FunctionalDependencyInferenceTest.testUnionWithProvenance"]
fn fd_inference_union_with_provenance_constant() {
    let t = fd_table("fd_t");

    // Five arms; each reads the FD table and binds ?a and ?b.
    // In Ontop: each ConstructionNode substitutes X←constant("0")…("4").
    // In sf: the cascade has no logic to treat a per-arm constant binding as
    // a union-level FD discriminator (TermDef::Const, were it used, would be
    // ignored by all existing passes).
    let branches: Vec<Branch> = (0usize..5)
        .map(|i| {
            let mut b = Branch::empty();
            b.core = vec![scan(i, "fd_t")];
            b.bindings.insert("a".into(), col_binding(i, "col1"));
            b.bindings.insert("b".into(), col_binding(i, "col2"));
            // Provenance placeholder: a col_binding to col1 stands in for the
            // per-arm constant that Ontop's ConstructionNode would inject as X.
            // The correct sf analog is TermDef::Const(Term::Literal(...)) — but
            // the cascade ignores constant bindings as FD discriminators anyway.
            b.bindings.insert("x".into(), col_binding(i, "col1"));
            b
        })
        .collect();

    let _out = run(branches, &[t], &CascadeCtx::default());

    assert!(
        false,
        "NEEDS_IMPL: sf has no union-with-provenance FD logic; \
         when implemented, verify that a 5-arm union (each arm a FD_TABLE scan + \
         different provenance constant for ?x) yields union-level FD {{A,X}}→{{B}} \
         (FunctionalDependencyInferenceTest.testUnionWithProvenance)"
    );
}
