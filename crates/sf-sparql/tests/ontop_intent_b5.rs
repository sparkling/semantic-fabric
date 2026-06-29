//! Ontop-parity INTENT file — batch 5 supplementary tests (ADR-0021 / ADR-0022).
//!
//! Extends `ontop_port_b5.rs` with additional UniqueConstraintInference scenarios
//! not yet covered there, and documents the accounting for all five assigned classes.
//!
//! ── Class accounting ──────────────────────────────────────────────────────────
//!
//! PushDownBooleanExpressionOptimizerTest (9) — BOUNDARY
//!   All scenarios require Ontop's tree-of-InnerJoin/LeftJoin nodes; see port
//!   file for detailed rationale. No sf-representable oracle exists.
//!
//! PushUpBooleanExpressionOptimizerTest (13) — BOUNDARY
//!   Inverse direction of push-down; same structural constraints apply.
//!
//! SelfJoinSameTermsTest (13) — BOUNDARY (all 13 accounted for):
//!   - Elimination1,3,4,5,6,8 (6) → ported GREEN (port file).
//!   - Elimination2, Elimination7 (2) → @Ignore in Ontop ("TODO: complex case");
//!     both involve 4-scan joins with split projected columns — BOUNDARY for sf
//!     as well (sf's pass 2c is a superset in the flat-branch model but cannot
//!     represent the 4-scan split-projection pattern these tests require).
//!   - NonElimination1,2bis,3,4 (4) → ported GREEN (port file).
//!   - NonElimination2 (1) → maps to NonElimination2bis in sf's flat IR: Ontop
//!     first strips unbound variables from data-node ImmutableMaps (yielding
//!     NonElimination2bis's initial state), then finds no elimination. In sf the
//!     two scans start without cross-scan ColEqs so `st_non_elimination2bis`
//!     already covers this case. No new test needed.
//!
//!   Zero NEEDS_IMPL gaps; no additional SelfJoin tests in this file.
//!
//! TrueNodesRemovalOptimizerTest (11) — BOUNDARY
//!   sf has no TrueNode artifact; see port file for detailed rationale.
//!
//! UniqueConstraintInferenceTest (28) — SUPPORTED (pass 6 proxy) / NEEDS_IMPL
//!   Port file has 5 UC tests (4 GREEN + 1 RED P0 spec). This file adds 4 more:
//!
//!   [GREEN]      uc_single_col_template_pk_distinct_removed
//!                Analog of Ontop URI_TEMPLATE_INJECTIVE_1 (single placeholder).
//!                Template "http://example.org/ds4/{col1}" over pk_ar2 (col1=PK).
//!                Single-placeholder ⇒ trivially injective ⇒ X unique ⇒ DISTINCT
//!                removed. sf correctly handles this. ✓
//!
//!   [GREEN]      uc_non_injective_reversed_cols_distinct_kept
//!                P0 companion — FIXED by Template::is_injective() + pass-6 gate.
//!                Template "ds2/{col2}{col1}": reversed-column adjacent placeholders
//!                ⇒ non-injective ⇒ DISTINCT kept. sf now correctly matches Ontop.
//!
//!   [RED NEEDS_IMPL] uc_composite_pk_injective_distinct_removed_needs_impl
//!                Analog of Ontop testConstructionInjectiveTemplate3. Table with
//!                COMPOSITE PK (col1,col2). Injective "/" separator template ⇒
//!                X covers the full composite PK ⇒ Ontop {{X}} (unique, DISTINCT
//!                removed). sf: `single_col_keys` returns [] for composite PKs
//!                (primary_key.len() > 1) ⇒ DISTINCT kept. Gap: pass 6 needs
//!                composite-key inference to match Ontop.
//!
//!   [GREEN/coincidence] uc_composite_pk_non_injective_distinct_kept
//!                Analog of Ontop testConstructionNonInjectiveTemplate2. Same
//!                composite PK table, non-injective adjacent template ⇒ Ontop {}
//!                (keeps DISTINCT). sf also keeps DISTINCT — but because
//!                single_col_keys returns [], NOT because it detects
//!                non-injectivity. Regression guard: if composite-PK support is
//!                added without an injectivity check, this test catches the bug.

#![cfg(test)]

use sf_core::ir::{LogicalSource, Segment, Template, TermMap, TermSpec};
use sf_sparql::cascade::{run, CascadeCtx};
use sf_sparql::iq::{Branch, Scan, SqlCond, TermDef};
use sf_sql::{Column, TableSchema};

// ── helpers ──────────────────────────────────────────────────────────────────

fn scan(alias: usize, table: &str) -> Scan {
    Scan {
        alias,
        source: LogicalSource::Table(table.to_owned()),
    }
}

fn branch(core: Vec<Scan>, where_conds: Vec<SqlCond>, distinct: bool) -> Branch {
    let mut b = Branch::empty();
    b.core = core;
    b.where_conds = where_conds;
    b.distinct = distinct;
    b
}

fn ctx<'a>(distinct: bool, project: &'a [String]) -> CascadeCtx<'a> {
    CascadeCtx {
        distinct,
        project: Some(project),
    }
}

/// Single-placeholder IRI template: `{prefix}{col}`.
fn iri_template1(alias: usize, prefix: &str, col: &str) -> TermDef {
    let segs = vec![Segment::Literal(prefix.into()), Segment::Column(col.into())];
    TermDef::Derived {
        term_map: TermMap::Template(Template::from_segments(segs).unwrap(), TermSpec::iri()),
        alias,
    }
}

/// Two-placeholder IRI template: `{prefix}{c1}{sep?}{c2}`.
/// `sep = Some("/")` ⇒ injective. `sep = None` or `Some("")` ⇒ adjacent
/// placeholders — non-injective (collisions possible by string splitting).
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

/// pk_ar2: 2-column table, single-column PK on col1.
fn pk_ar2_schema() -> Vec<TableSchema> {
    let mut t = TableSchema::new("pk_ar2");
    t.primary_key = vec!["col1".into()];
    t.columns = vec![
        Column::new("col1", "text", true), // PK ⇒ NOT NULL
        Column::new("col2", "text", true),
    ];
    vec![t]
}

/// composite_pk: 2-column table, COMPOSITE PK on (col1, col2).
fn composite_pk_schema() -> Vec<TableSchema> {
    let mut t = TableSchema::new("composite_pk");
    t.primary_key = vec!["col1".into(), "col2".into()];
    t.columns = vec![
        Column::new("col1", "text", true),
        Column::new("col2", "text", true),
    ];
    vec![t]
}

// ── UniqueConstraintInferenceTest — supplementary tests ───────────────────────
//
// Behavioural mapping (same as port file):
//   Ontop {{X}}  ⟺  sf removes DISTINCT  (out.distinct == false)
//   Ontop {}     ⟺  sf keeps   DISTINCT  (out.distinct == true)

/// **GREEN** — URI_TEMPLATE_INJECTIVE_1 analog (single placeholder, not yet in
/// port file). Template `http://example.org/ds4/{col1}` over pk_ar2 (col1=PK).
///
/// A single-placeholder IRI template is trivially injective (one column → one
/// string). X reads the PK column ⇒ X is unique ⇒ DISTINCT is redundant.
///
/// sf: `def.columns()` = `[col1]`; `single_col_keys(pk_ar2)` = `["col1"]`;
/// `contains(col1_ref)` = true ⇒ `redundant = true` ⇒ DISTINCT removed. ✓
#[test]
fn uc_single_col_template_pk_distinct_removed() {
    let mut b = branch(vec![scan(0, "pk_ar2")], Vec::new(), true);
    b.bindings.insert(
        "X".into(),
        iri_template1(0, "http://example.org/ds4/", "col1"),
    );

    let out = run(vec![b], &pk_ar2_schema(), &ctx(true, &["X".into()]));
    assert!(
        !out[0].distinct,
        "single-placeholder template over PK col ⇒ X unique ⇒ DISTINCT removed"
    );
}

/// **GREEN — P0 soundness fix shipped.** Template
/// `http://example.org/ds2/{col2}{col1}` reverses the column order vs the port
/// test but is equally non-injective: for col2="2", col1="13" vs col2="21",
/// col1="3" both yield the IRI ending in "213". DISTINCT must be preserved.
///
/// Fixed by `Template::is_injective()` + `binding_is_injective()` in pass 6:
/// adjacent `Column` slots ⇒ `is_injective() = false` ⇒ DISTINCT kept.
///
/// Expected (Ontop): DISTINCT kept (`out.distinct == true`).
/// sf (after fix):   DISTINCT kept (`out.distinct == true`). ✓
#[test]
fn uc_non_injective_reversed_cols_distinct_kept() {
    // col2 first, then col1, no separator ⇒ non-injective.
    // ("2","13") and ("21","3") both produce IRI suffix "213".
    let mut b = branch(vec![scan(0, "pk_ar2")], Vec::new(), true);
    b.bindings.insert(
        "X".into(),
        iri_template2(0, "http://example.org/ds2/", "col2", None, "col1"),
    );

    let out = run(vec![b], &pk_ar2_schema(), &ctx(true, &["X".into()]));
    assert!(
        out[0].distinct,
        "Ontop keeps DISTINCT (reversed non-injective template ⇒ X not unique); \
         sf removes it — RED P0 parity divergence"
    );
}

/// **RED — NEEDS_IMPL.** Analog of Ontop `testConstructionInjectiveTemplate3`.
///
/// Table `composite_pk` has a COMPOSITE primary key (col1, col2). An injective
/// two-placeholder template `http://example.org/ds1/{col1}/{col2}` (separator "/"
/// between the two placeholders) maps the full composite PK to X. Because every
/// distinct (col1, col2) pair produces a distinct IRI, X uniquely identifies every
/// row ⇒ Ontop infers `{{X}}` and the DISTINCT is redundant.
///
/// sf gap: `cascade::fd::single_col_keys` returns `[]` when
/// `ts.primary_key.len() > 1` (composite PK). Consequently `keys` is empty,
/// `redundant = false`, and DISTINCT is kept — even though the full composite key
/// is covered by the injective template.
///
/// Expected (Ontop): DISTINCT removed (`out.distinct == false`).
/// Got (sf):         DISTINCT kept (`out.distinct == true`).
/// Fix required:     pass 6 must recognise composite PKs and verify that an
///                   injective template covers all PK columns before removing
///                   DISTINCT.
#[test]
fn uc_composite_pk_injective_distinct_removed_needs_impl() {
    // Injective template: "/" separator between col1 and col2.
    // (col1, col2) is the composite PK ⇒ X is unique ⇒ DISTINCT redundant.
    let mut b = branch(vec![scan(0, "composite_pk")], Vec::new(), true);
    b.bindings.insert(
        "X".into(),
        iri_template2(0, "http://example.org/ds1/", "col1", Some("/"), "col2"),
    );

    let out = run(vec![b], &composite_pk_schema(), &ctx(true, &["X".into()]));
    assert!(
        !out[0].distinct,
        "Ontop removes DISTINCT (injective template covers full composite PK ⇒ X unique); \
         sf keeps it — NEEDS_IMPL: composite-PK inference missing in pass 6"
    );
}

/// **GREEN (correct outcome, wrong reason) — regression guard.** Analog of Ontop
/// `testConstructionNonInjectiveTemplate2`.
///
/// Same `composite_pk` table; non-injective template
/// `http://example.org/ds2/{col1}{col2}` (adjacent placeholders, no separator).
/// X is NOT uniquely determined — collisions are possible (e.g. col1="1", col2="23"
/// vs col1="12", col2="3" both yield "123"). Ontop infers `{}` ⇒ DISTINCT kept.
///
/// sf: `single_col_keys(composite_pk)` = `[]` (composite PK) ⇒ `redundant = false`
/// ⇒ DISTINCT kept. Outcomes agree, but sf's reason is "can't prove redundant"
/// rather than "non-injective template".
///
/// **Regression intent**: if composite-PK support is later added to pass 6 WITHOUT
/// a corresponding injectivity check, the P0 bug would resurface for composite-PK
/// tables. This test will catch that regression.
#[test]
fn uc_composite_pk_non_injective_distinct_kept() {
    // Non-injective: col1 + col2 with no separator.
    let mut b = branch(vec![scan(0, "composite_pk")], Vec::new(), true);
    b.bindings.insert(
        "X".into(),
        iri_template2(0, "http://example.org/ds2/", "col1", None, "col2"),
    );

    let out = run(vec![b], &composite_pk_schema(), &ctx(true, &["X".into()]));
    assert!(
        out[0].distinct,
        "non-injective template on composite PK ⇒ X not provably unique ⇒ \
         DISTINCT kept (Ontop and sf agree; regression guard against P0 regression)"
    );
}
