//! Cascade unit tests — split into a sibling file to keep `cascade/mod.rs`
//! within the size budget.
#![cfg(test)]

use super::*;
use crate::iq::OptJoin;
use sf_core::ir::{LogicalSource, Segment, Template, TermMap, TermSpec};
use sf_sql::{Column, ForeignKey, FunctionalDep};
use std::collections::BTreeMap;

fn scan(alias: usize, table: &str) -> crate::iq::Scan {
    crate::iq::Scan {
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

#[test]
fn prune_drops_contradictory_equalities() {
    let mut b = Branch::single(scan(0, "emp"));
    b.where_conds
        .push(SqlCond::Cmp(ColRef::new(0, "id"), CmpOp::Eq, "1".into()));
    b.where_conds
        .push(SqlCond::Cmp(ColRef::new(0, "id"), CmpOp::Eq, "2".into()));
    let out = run(vec![b], &[], &CascadeCtx::default());
    assert!(
        out.is_empty(),
        "contradictory =1 ∧ =2 must prune the branch"
    );
}

#[test]
fn self_join_eliminated_on_unique_key() {
    // Two scans of "emp" joined on the PK "id" → one scan after the pass.
    let mut b = Branch {
        core: vec![scan(0, "emp"), scan(1, "emp")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![SqlCond::ColEq(ColRef::new(0, "id"), ColRef::new(1, "id"))],
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("n".to_owned(), col_binding(1, "name"));

    let mut ts = TableSchema::new("emp");
    ts.primary_key = vec!["id".to_owned()];
    let out = run(vec![b], std::slice::from_ref(&ts), &CascadeCtx::default());
    assert_eq!(out.len(), 1);
    let b = &out[0];
    assert_eq!(b.core.len(), 1, "self-join collapsed to a single scan");
    // The surviving binding was rewritten onto the kept alias 0.
    match b.bindings.get("n").unwrap() {
        TermDef::Derived { alias, .. } => assert_eq!(*alias, 0),
        _ => panic!("expected a derived binding"),
    }
    assert!(b.where_conds.is_empty(), "trivial id = id dropped");
}

#[test]
fn self_join_collapsed_with_is_not_null_on_nullable_unique_key() {
    // A single-column UNIQUE but NULLABLE column (e.g. `email TEXT UNIQUE`):
    // the SQL equi-join `t0.email = t1.email` already excludes NULL rows
    // (NULL = NULL ⇒ UNKNOWN). Collapsing to one scan + IS NOT NULL reproduces
    // exactly that NULL-exclusion → same bag (=_bag preserved, ADR-0007).
    let b = Branch {
        core: vec![scan(0, "emp"), scan(1, "emp")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![SqlCond::ColEq(
            ColRef::new(0, "email"),
            ColRef::new(1, "email"),
        )],
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    let mut ts = TableSchema::new("emp");
    ts.unique = vec![vec!["email".to_owned()]];
    ts.columns = vec![sf_sql::Column::new("email", "text", false)]; // NULLABLE
    let out = run(vec![b], std::slice::from_ref(&ts), &CascadeCtx::default());
    assert_eq!(
        out[0].core.len(),
        1,
        "nullable-unique self-join collapses with IS NOT NULL"
    );
    assert!(
        out[0]
            .where_conds
            .iter()
            .any(|c| matches!(c, SqlCond::IsNotNull(r) if r.column.as_ref() == "email")),
        "IS NOT NULL(email) compensates for NULL-exclusion: {:?}",
        out[0].where_conds
    );
}

#[test]
fn self_join_eliminated_keeps_unrelated_self_comparison_guard() {
    // A `?x :p ?x` pattern produces ColEq(t2.c, t2.c) — an effective IS NOT NULL
    // guard. Eliminating an *unrelated* PK self-join must not drop it as
    // collateral (the prior global `retain(!is_trivial_eq)` bug).
    let mut b = Branch {
        core: vec![scan(0, "emp"), scan(1, "emp"), scan(2, "emp")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![
            SqlCond::ColEq(ColRef::new(0, "id"), ColRef::new(1, "id")), // the self-join
            SqlCond::ColEq(ColRef::new(2, "c"), ColRef::new(2, "c")),   // ?x :p ?x guard
        ],
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("n".to_owned(), col_binding(1, "name"));
    let mut ts = TableSchema::new("emp");
    ts.primary_key = vec!["id".to_owned()];
    let out = run(vec![b], std::slice::from_ref(&ts), &CascadeCtx::default());
    let b = &out[0];
    assert_eq!(b.core.len(), 2, "the id self-join collapsed (3 → 2 scans)");
    assert!(
        b.where_conds
            .iter()
            .any(|c| matches!(c, SqlCond::ColEq(a, b) if a == b && &*a.column == "c")),
        "the unrelated self-comparison guard must survive: {:?}",
        b.where_conds
    );
}

#[test]
fn self_join_not_eliminated_without_key_proof() {
    // Same shape, but no schema → the precondition is unproven → no-op.
    let b = Branch {
        core: vec![scan(0, "emp"), scan(1, "emp")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![SqlCond::ColEq(ColRef::new(0, "id"), ColRef::new(1, "id"))],
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    let out = run(vec![b], &[], &CascadeCtx::default());
    assert_eq!(out[0].core.len(), 2, "no key proof ⇒ join is preserved");
}

// --- pass 3: FD inference (transitive closure) -------------------------

fn pk_table(name: &str, pk: &str) -> TableSchema {
    let mut t = TableSchema::new(name);
    t.primary_key = vec![pk.to_owned()];
    t.columns = vec![Column::new(pk, "text", true)];
    t
}

#[test]
fn fd_inference_seeds_keys_and_closes_transitively() {
    // a.x = b.y, b.y = c.z; a/b/c each a single-col PK. The FD set must seed
    // the three key→row FDs, propagate them across the equalities, and reach
    // `a.x → c.*` ONLY via transitivity (no direct a–c equality exists).
    let b = Branch {
        core: vec![scan(0, "a"), scan(1, "b"), scan(2, "c")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![
            SqlCond::ColEq(ColRef::new(0, "x"), ColRef::new(1, "y")),
            SqlCond::ColEq(ColRef::new(1, "y"), ColRef::new(2, "z")),
        ],
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    let schema = vec![pk_table("a", "x"), pk_table("b", "y"), pk_table("c", "z")];
    let fds = infer_functional_dependencies(&b, &schema);
    // Seeded key→row FDs.
    assert!(fds.is_key(&ColRef::new(0, "x")));
    assert!(fds.is_key(&ColRef::new(1, "y")));
    assert!(fds.is_key(&ColRef::new(2, "z")));
    // Equality closure: x determines b's row (alias 1).
    assert!(fds.has(&ColRef::new(0, "x"), 1));
    // Transitive closure: x → c.* (alias 2), only reachable through b.
    assert!(
        fds.has(&ColRef::new(0, "x"), 2),
        "transitive FD a.x → c.* must be derived"
    );
}

#[test]
fn fd_inference_empty_without_schema() {
    // No catalog ⇒ no keys can be proven ⇒ pass (4) has no uniqueness proof.
    let b = Branch {
        core: vec![scan(0, "a"), scan(1, "b")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![SqlCond::ColEq(ColRef::new(0, "x"), ColRef::new(1, "y"))],
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    let fds = infer_functional_dependencies(&b, &[]);
    assert!(!fds.is_key(&ColRef::new(0, "x")));
    assert!(!fds.is_key(&ColRef::new(1, "y")));
}

// --- pass 4: FK/PK join elimination ------------------------------------

fn iri_template_binding(alias: usize, prefix: &str, col: &str) -> TermDef {
    let segs = vec![Segment::Literal(prefix.into()), Segment::Column(col.into())];
    TermDef::Derived {
        term_map: TermMap::Template(Template::from_segments(segs).unwrap(), TermSpec::iri()),
        alias,
    }
}

/// trips(trip_id PK, rid FK→routes.route_id NOT NULL) + routes(route_id PK).
/// The FK column `rid` is deliberately *renamed* (≠ the PK name) to exercise
/// the column-rename rewrite of the parent subject template.
fn gtfs_schema() -> Vec<TableSchema> {
    let mut trips = TableSchema::new("trips");
    trips.primary_key = vec!["trip_id".into()];
    trips.columns = vec![
        Column::new("trip_id", "text", true),
        Column::new("rid", "text", true), // FK, NOT NULL
    ];
    trips.foreign_keys = vec![ForeignKey {
        columns: vec!["rid".into()],
        parent_table: "routes".into(),
        parent_columns: vec!["route_id".into()],
    }];
    let mut routes = TableSchema::new("routes");
    routes.primary_key = vec!["route_id".into()];
    routes.columns = vec![Column::new("route_id", "text", true)];
    vec![trips, routes]
}

fn fk_branch() -> Branch {
    let mut b = Branch {
        core: vec![scan(0, "trips"), scan(1, "routes")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: vec![SqlCond::ColEq(
            ColRef::new(0, "rid"),
            ColRef::new(1, "route_id"),
        )],
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    // ?route bound to the parent (routes) subject IRI, built from route_id.
    b.bindings.insert(
        "route".into(),
        iri_template_binding(1, "http://ex/route/", "route_id"),
    );
    b
}

#[test]
fn fk_pk_join_eliminated_on_notnull_fk_to_pk() {
    let out = run(vec![fk_branch()], &gtfs_schema(), &CascadeCtx::default());
    let b = &out[0];
    assert_eq!(b.core.len(), 1, "parent routes scan dropped");
    assert_eq!(b.core[0].alias, 0, "child trips scan kept");
    assert!(b.where_conds.is_empty(), "the FK=PK join equality removed");
    // =_bag spot check: the parent subject IRI now reads the equal child FK
    // column `rid` @ alias 0 — same IRI value, and NOTHING still references the
    // dropped parent alias 1.
    match b.bindings.get("route").unwrap() {
        TermDef::Derived {
            term_map: TermMap::Template(t, _),
            alias,
        } => {
            assert_eq!(*alias, 0);
            assert!(
                t.segments()
                    .iter()
                    .any(|s| matches!(s, Segment::Column(c) if &**c == "rid")),
                "PK column renamed to the child FK column"
            );
        }
        other => panic!("expected a rewritten template binding, got {other:?}"),
    }
    let mut refs_parent = false;
    for c in b.projection() {
        if c.alias == 1 {
            refs_parent = true;
        }
    }
    assert!(
        !refs_parent,
        "no surviving reference to the dropped parent alias"
    );
}

#[test]
fn fk_pk_not_eliminated_when_fk_nullable() {
    // Nullable FK: the inner join drops NULL-rid child rows; removing it would
    // re-admit them → extra rows → =_bag break. Must NOT fire.
    let mut schema = gtfs_schema();
    schema[0].columns[1].not_null = false; // rid nullable
    let out = run(vec![fk_branch()], &schema, &CascadeCtx::default());
    assert_eq!(out[0].core.len(), 2, "nullable FK ⇒ join preserved");
}

#[test]
fn fk_pk_not_eliminated_without_schema() {
    let out = run(vec![fk_branch()], &[], &CascadeCtx::default());
    assert_eq!(out[0].core.len(), 2, "no FK/PK proof ⇒ join preserved");
}

#[test]
fn fk_pk_not_eliminated_when_parent_other_column_used() {
    // The parent is reached for MORE than its PK (a non-key column is
    // projected) → it cannot be dropped without losing data.
    let mut schema = gtfs_schema();
    schema[1]
        .columns
        .push(Column::new("long_name", "text", false));
    let mut b = fk_branch();
    b.bindings
        .insert("rname".into(), col_binding(1, "long_name"));
    let out = run(vec![b], &schema, &CascadeCtx::default());
    assert_eq!(
        out[0].core.len(),
        2,
        "parent contributes a non-PK column ⇒ kept"
    );
}

// --- pass 5: selection pushdown ----------------------------------------

#[test]
fn selection_pushdown_flattens_and_hoists() {
    let mut b = Branch::single(scan(0, "emp"));
    b.where_conds = vec![SqlCond::And(vec![
        SqlCond::ColEq(ColRef::new(0, "a"), ColRef::new(1, "b")), // a join (2 aliases)
        SqlCond::Cmp(ColRef::new(0, "x"), CmpOp::Gt, "5".into()), // a single-scan selection
    ])];
    selection_pushdown(&mut b);
    assert_eq!(b.where_conds.len(), 2, "the nested AND is flattened");
    assert!(
        matches!(b.where_conds[0], SqlCond::Cmp(..)),
        "single-scan selection hoisted ahead of the join"
    );
    assert!(
        matches!(b.where_conds[1], SqlCond::ColEq(..)),
        "join equality after"
    );
}

#[test]
fn selection_pushdown_is_bag_preserving_noop_when_already_pushed() {
    // A lone selection is unchanged (reordering a 1-element conjunction is a
    // no-op); reordering never adds/removes a conjunct ⇒ =_bag preserved.
    let mut b = Branch::single(scan(0, "emp"));
    b.where_conds = vec![SqlCond::Cmp(ColRef::new(0, "x"), CmpOp::Gt, "5".into())];
    selection_pushdown(&mut b);
    assert_eq!(b.where_conds.len(), 1);
    assert!(matches!(b.where_conds[0], SqlCond::Cmp(..)));
}

// --- pass 6: distinct removal ------------------------------------------

fn emp_pk() -> TableSchema {
    let mut ts = TableSchema::new("emp");
    ts.primary_key = vec!["id".to_owned()];
    ts
}

#[test]
fn distinct_removed_when_projected_key() {
    let mut b = Branch::single(scan(0, "emp"));
    b.bindings
        .insert("s".into(), iri_template_binding(0, "http://ex/emp/", "id"));
    let ctx = CascadeCtx {
        distinct: true,
        project: None,
    };
    let out = run(vec![b], std::slice::from_ref(&emp_pk()), &ctx);
    assert!(
        !out[0].distinct,
        "DISTINCT over a projected PK-derived var is redundant → removed"
    );
}

#[test]
fn distinct_kept_when_key_not_projected() {
    // =_bag spot check: SELECT DISTINCT ?color is NOT a no-op (colors repeat),
    // so the DISTINCT must survive — removing it would change multiplicities.
    let mut b = Branch::single(scan(0, "emp"));
    b.bindings
        .insert("s".into(), iri_template_binding(0, "http://ex/emp/", "id"));
    b.bindings.insert("c".into(), col_binding(0, "color"));
    let proj = vec!["c".to_owned()];
    let ctx = CascadeCtx {
        distinct: true,
        project: Some(&proj),
    };
    let out = run(vec![b], std::slice::from_ref(&emp_pk()), &ctx);
    assert!(out[0].distinct, "DISTINCT ?color is not redundant → kept");
}

#[test]
fn distinct_not_removed_on_nullable_unique_key() {
    // Parallel to `self_join_not_eliminated_on_nullable_unique_key` for pass 6.
    // A NULLABLE single-column UNIQUE col is not a true key: SQL UNIQUE permits
    // many NULLs and build_term emits an unbound solution per NULL row, so
    // `SELECT email` keeps both NULL rows while `SELECT DISTINCT email` collapses
    // them. Removing the DISTINCT would ADD a row vs base → =_bag break. The pass
    // must NOT fire (DISTINCT retained).
    let mut b = Branch::single(scan(0, "emp"));
    b.bindings.insert("e".into(), col_binding(0, "email"));
    let mut ts = TableSchema::new("emp");
    ts.unique = vec![vec!["email".to_owned()]];
    ts.columns = vec![Column::new("email", "text", false)]; // NULLABLE
    let ctx = CascadeCtx {
        distinct: true,
        project: None,
    };
    let out = run(vec![b], std::slice::from_ref(&ts), &ctx);
    assert!(
        out[0].distinct,
        "nullable UNIQUE key ⇒ DISTINCT is NOT redundant → retained"
    );
}

#[test]
fn distinct_removed_on_notnull_unique_key() {
    // Contrast: a NOT-NULL single-column UNIQUE col IS a true key, so a DISTINCT
    // over a var derived from it is provably redundant → removed.
    let mut b = Branch::single(scan(0, "emp"));
    b.bindings.insert("e".into(), col_binding(0, "email"));
    let mut ts = TableSchema::new("emp");
    ts.unique = vec![vec!["email".to_owned()]];
    ts.columns = vec![Column::new("email", "text", true)]; // NOT NULL
    let ctx = CascadeCtx {
        distinct: true,
        project: None,
    };
    let out = run(vec![b], std::slice::from_ref(&ts), &ctx);
    assert!(
        !out[0].distinct,
        "NOT-NULL UNIQUE key ⇒ DISTINCT redundant → removed"
    );
}

#[test]
fn distinct_kept_on_join() {
    // Multi-scan: a join can duplicate the projected tuple, so DISTINCT must
    // survive even when a key is projected (no single-scan uniqueness proof).
    let mut b = Branch {
        core: vec![scan(0, "emp"), scan(1, "dept")],
        opts: Vec::new(),
        bindings: BTreeMap::new(),
        where_conds: Vec::new(),
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings
        .insert("s".into(), iri_template_binding(0, "http://ex/emp/", "id"));
    let ctx = CascadeCtx {
        distinct: true,
        project: None,
    };
    let out = run(vec![b], std::slice::from_ref(&emp_pk()), &ctx);
    assert!(
        out[0].distinct,
        "join ⇒ DISTINCT preserved (no uniqueness proof)"
    );
}

// --- pass 2e ext (wave 1b): FD-based OPTIONAL elimination under DISTINCT -----

/// A table with a non-unique FD determinant: `det_col → col3, col4`.
/// `det_notnull` controls whether `det_col` is NOT NULL or nullable.
fn fd_table(name: &str, det_notnull: bool) -> TableSchema {
    let mut t = TableSchema::new(name);
    t.columns = vec![
        Column::new("id", "integer", true),
        Column::new("det_col", "integer", det_notnull),
        Column::new("col3", "text", false),
        Column::new("col4", "text", false),
    ];
    t.primary_key = vec!["id".to_owned()];
    t.functional_dependencies = vec![FunctionalDep {
        det: vec!["det_col".to_owned()],
        dep: vec!["col3".to_owned(), "col4".to_owned()],
    }];
    t
}

#[test]
fn fd_optional_eliminated_on_notnull_fd_determinant() {
    // OPTIONAL(same table, ON NullSafeEq(det_col@0, det_col@1), no extra) where
    // det_col is NOT NULL and a declared FD determinant, and all opt bindings
    // read only from {det_col, col3, col4} — the OPTIONAL is redundant under DISTINCT.
    // Wave 1b positive case.
    let mut b = Branch {
        core: vec![scan(0, "emp")],
        opts: vec![OptJoin {
            scan: scan(1, "emp"),
            on: vec![SqlCond::NullSafeEq(
                ColRef::new(0, "det_col"),
                ColRef::new(1, "det_col"),
            )],
            extra: vec![],
        }],
        bindings: BTreeMap::new(),
        where_conds: vec![],
        distinct: false,
        limit: None,
        offset: 0,
        order: vec![],
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("x".into(), col_binding(1, "col3"));
    let ts = fd_table("emp", true); // det_col NOT NULL
    let ctx = CascadeCtx {
        distinct: true,
        project: None,
    };
    let out = run(vec![b], std::slice::from_ref(&ts), &ctx);
    let b = &out[0];
    assert!(b.opts.is_empty(), "FD-based OPTIONAL must be eliminated");
    // Binding must be rewritten to the core alias.
    match b.bindings.get("x").unwrap() {
        TermDef::Derived { alias, .. } => {
            assert_eq!(*alias, 0, "binding rewritten to core alias 0")
        }
        _ => panic!("expected a derived binding"),
    }
}

#[test]
fn fd_optional_not_eliminated_when_fd_det_nullable() {
    // With det_col NULLABLE the FD does not constrain rows with det_col IS NULL —
    // different rows could share det_col=NULL but differ on col3/col4.
    // Eliminating the OPTIONAL would merge those distinct tuples → =_bag break.
    let mut b = Branch {
        core: vec![scan(0, "emp")],
        opts: vec![OptJoin {
            scan: scan(1, "emp"),
            on: vec![SqlCond::NullSafeEq(
                ColRef::new(0, "det_col"),
                ColRef::new(1, "det_col"),
            )],
            extra: vec![],
        }],
        bindings: BTreeMap::new(),
        where_conds: vec![],
        distinct: false,
        limit: None,
        offset: 0,
        order: vec![],
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("x".into(), col_binding(1, "col3"));
    let ts = fd_table("emp", false); // det_col NULLABLE
    let ctx = CascadeCtx {
        distinct: true,
        project: None,
    };
    let out = run(vec![b], std::slice::from_ref(&ts), &ctx);
    assert_eq!(
        out[0].opts.len(),
        1,
        "nullable FD det ⇒ OPTIONAL preserved (=_bag guard)"
    );
}

#[test]
fn fd_optional_not_eliminated_without_distinct() {
    // Without DISTINCT the pass must not fire — eliminating the OPTIONAL would
    // remove rows where the opt side is absent (NULL), changing the bag.
    let mut b = Branch {
        core: vec![scan(0, "emp")],
        opts: vec![OptJoin {
            scan: scan(1, "emp"),
            on: vec![SqlCond::NullSafeEq(
                ColRef::new(0, "det_col"),
                ColRef::new(1, "det_col"),
            )],
            extra: vec![],
        }],
        bindings: BTreeMap::new(),
        where_conds: vec![],
        distinct: false,
        limit: None,
        offset: 0,
        order: vec![],
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("x".into(), col_binding(1, "col3"));
    let ts = fd_table("emp", true); // det_col NOT NULL — but no DISTINCT
    let ctx = CascadeCtx {
        distinct: false,
        project: None,
    };
    let out = run(vec![b], std::slice::from_ref(&ts), &ctx);
    assert_eq!(out[0].opts.len(), 1, "no DISTINCT ⇒ OPTIONAL preserved");
}

// --- pass 2e ext (wave 1c): nullable-det IS-NOT-NULL synthesis (inner join) --

#[test]
fn fd_inner_join_nullable_det_adds_is_not_null() {
    // Two scans of "emp" inner-joined on nullable det_col (a declared FD determinant)
    // under DISTINCT. After collapsing to one scan, IS NOT NULL(det_col@0) must be
    // synthesised — without it, rows that had det_col=NULL would re-appear (the
    // equi-join excluded them), breaking =_bag.
    // Wave 1c positive case.
    //
    // Binding design: "x" on alias 1 (drop), "y" on alias 0 (keep). This blocks
    // pass 2c (same_terms_elimination) from firing, because in BOTH orientations
    // the binding on the "drop" side uses col3 which is not covered by the
    // ColEq(det_col) — only col3 IS in the FD dep set, so pass 2e handles it.
    let mut b = Branch {
        core: vec![scan(0, "emp"), scan(1, "emp")],
        opts: vec![],
        bindings: BTreeMap::new(),
        where_conds: vec![SqlCond::ColEq(
            ColRef::new(0, "det_col"),
            ColRef::new(1, "det_col"),
        )],
        distinct: false,
        limit: None,
        offset: 0,
        order: vec![],
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("x".into(), col_binding(1, "col3")); // on drop alias
    b.bindings.insert("y".into(), col_binding(0, "col3")); // on keep alias — blocks pass 2c
    let ts = fd_table("emp", false); // det_col NULLABLE
    let ctx = CascadeCtx {
        distinct: true,
        project: None,
    };
    let out = run(vec![b], std::slice::from_ref(&ts), &ctx);
    let b = &out[0];
    assert_eq!(b.core.len(), 1, "FD self-join collapses to single scan");
    let guard = ColRef::new(0, "det_col");
    assert!(
        b.where_conds
            .iter()
            .any(|c| matches!(c, SqlCond::IsNotNull(r) if r == &guard)),
        "IS NOT NULL synthesised to preserve the equi-join's NULL exclusion: {:?}",
        b.where_conds
    );
}

#[test]
fn fd_inner_join_notnull_det_no_is_not_null_added() {
    // When det_col is NOT NULL the equi-join never excluded any rows — no IS NOT
    // NULL guard is needed. Adding one anyway would be a no-op semantically, but
    // the pass should not emit unnecessary conditions.
    // Same binding design as the nullable variant to isolate pass 2e from pass 2c.
    let mut b = Branch {
        core: vec![scan(0, "emp"), scan(1, "emp")],
        opts: vec![],
        bindings: BTreeMap::new(),
        where_conds: vec![SqlCond::ColEq(
            ColRef::new(0, "det_col"),
            ColRef::new(1, "det_col"),
        )],
        distinct: false,
        limit: None,
        offset: 0,
        order: vec![],
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("x".into(), col_binding(1, "col3")); // on drop alias
    b.bindings.insert("y".into(), col_binding(0, "col3")); // on keep alias — blocks pass 2c
    let ts = fd_table("emp", true); // det_col NOT NULL
    let ctx = CascadeCtx {
        distinct: true,
        project: None,
    };
    let out = run(vec![b], std::slice::from_ref(&ts), &ctx);
    let b = &out[0];
    assert_eq!(b.core.len(), 1, "FD self-join collapses to single scan");
    assert!(
        !b.where_conds
            .iter()
            .any(|c| matches!(c, SqlCond::IsNotNull(..))),
        "NOT-NULL det ⇒ no IS NOT NULL synthesised: {:?}",
        b.where_conds
    );
}

// --- self-join elimination WITHIN a NOT EXISTS/EXISTS correlated subquery
// (ADR-0023 optimizer-residue, Group-D-adjacent SQL-shape cosmetic wave) -------

/// A branch carrying a `NOT EXISTS` whose OWN `scans`/`conds` redundantly
/// self-join `dept` on its PK (`id`) — the exact shape the right-nested-OPTIONAL
/// decomposition (Group C, `leftjoin.rs::not_exists_cond_for`) produces before
/// this pass. `branch_has_not_exists` routes this branch past the (0)-(6)
/// constraint-driven passes entirely (they don't model a subquery's own scans),
/// so this test isolates that the NEW subquery-scoped pass still fires on the
/// early-return path.
#[test]
fn self_join_eliminated_inside_not_exists_subquery() {
    let mut b = Branch {
        core: vec![scan(0, "person")],
        opts: vec![],
        bindings: BTreeMap::new(),
        where_conds: vec![
            SqlCond::IsNotNull(ColRef::new(0, "name")),
            SqlCond::NotExists {
                scans: vec![scan(1, "person"), scan(2, "dept"), scan(3, "dept")],
                conds: vec![
                    SqlCond::ColEq(ColRef::new(1, "dept_id"), ColRef::new(2, "id")),
                    SqlCond::ColEq(ColRef::new(2, "id"), ColRef::new(3, "id")), // redundant self-join
                    SqlCond::IsNotNull(ColRef::new(3, "label")),
                    SqlCond::ColEq(ColRef::new(0, "id"), ColRef::new(1, "id")), // outer correlation
                ],
            },
        ],
        distinct: false,
        limit: None,
        offset: 0,
        order: vec![],
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("name".to_owned(), col_binding(0, "name"));
    let mut dept = TableSchema::new("dept");
    dept.primary_key = vec!["id".to_owned()];
    let out = run(vec![b], std::slice::from_ref(&dept), &CascadeCtx::default());
    assert_eq!(out.len(), 1);
    let SqlCond::NotExists { scans, conds } = out[0]
        .where_conds
        .iter()
        .find(|c| matches!(c, SqlCond::NotExists { .. }))
        .expect("NOT EXISTS survives")
    else {
        unreachable!()
    };
    assert_eq!(
        scans
            .iter()
            .filter(|s| s.alias == 2 || s.alias == 3)
            .count(),
        1,
        "the redundant dept self-join inside NOT EXISTS collapses to one scan: {scans:?}"
    );
    // The merge keeps the lower alias (2) and drops the higher (3); no cond may
    // still reference the dropped alias, and the licensing `dept.id = dept.id`
    // equality (the ONLY cond that referenced alias 3) must be gone — leaving
    // exactly the other 3 original conds (4 minus the 1 licensing equality).
    assert!(
        !conds.iter().any(|c| {
            let mut has_alias3 = false;
            collect_cond_cols(c, &mut |col| has_alias3 |= col.alias == 3);
            has_alias3
        }),
        "no remaining cond may reference the dropped dept alias 3: {conds:?}"
    );
    assert_eq!(
        conds.len(),
        3,
        "exactly the licensing dept.id = dept.id equality is removed: {conds:?}"
    );
    // The outer correlation (alias 0 -> alias 1, a DIFFERENT table/pair) must
    // survive untouched — this pass must not merge across the branch boundary.
    assert!(
        conds
            .iter()
            .any(|c| matches!(c, SqlCond::ColEq(a, b) if a.alias == 0 && b.alias == 1)),
        "the outer correlation condition must survive: {conds:?}"
    );
}

/// Negative control: two DIFFERENT (non-PK-joined) scans of the same table
/// inside a `NOT EXISTS` must NOT be merged — e.g. a genuine `?f1 :friend ?f2`
/// self-join on a non-key column. Proves the new pass reuses the same
/// soundness precondition (`find_self_join_in`) as the branch-level pass, not a
/// looser one.
#[test]
fn distinct_self_scans_inside_not_exists_not_merged_without_key_equality() {
    let mut b = Branch {
        core: vec![scan(0, "person")],
        opts: vec![],
        bindings: BTreeMap::new(),
        where_conds: vec![SqlCond::NotExists {
            scans: vec![scan(1, "person"), scan(2, "person")],
            conds: vec![
                // "friend_id" is NOT a unique key — no merge licensed.
                SqlCond::ColEq(ColRef::new(1, "friend_id"), ColRef::new(2, "id")),
                SqlCond::ColEq(ColRef::new(0, "id"), ColRef::new(1, "id")),
            ],
        }],
        distinct: false,
        limit: None,
        offset: 0,
        order: vec![],
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("x".to_owned(), col_binding(0, "id"));
    let mut person = TableSchema::new("person");
    person.primary_key = vec!["id".to_owned()];
    let out = run(
        vec![b],
        std::slice::from_ref(&person),
        &CascadeCtx::default(),
    );
    let SqlCond::NotExists { scans, .. } = out[0]
        .where_conds
        .iter()
        .find(|c| matches!(c, SqlCond::NotExists { .. }))
        .expect("NOT EXISTS survives")
    else {
        unreachable!()
    };
    assert_eq!(
        scans.len(),
        2,
        "a non-key-joined self-scan pair must NOT be merged: {scans:?}"
    );
}

/// Composite-PK sibling of [`self_join_eliminated_inside_not_exists_subquery`].
/// `org` has a 2-column composite PK (`id1`, `id2`); the `NOT EXISTS` subquery
/// redundantly self-joins `org` on BOTH PK columns. This exercises
/// `find_composite_pk_self_join_in` (mod.rs ~343, the SUBQUERY-scoped call site)
/// which — unlike the single-column pass exercised by the sibling test — had
/// ZERO coverage at this call site before this test; only the top-level call
/// site (mod.rs ~304) was covered, by `ontop_intent_b1.rs::composite_pk_self_join_elim`.
/// SPARQL `MINUS` lowers to the same `SqlCond::NotExists` IR node (see the
/// `NotExists` doc comment in `iq.rs`), so this one test also covers the MINUS
/// call site — there is no separate MINUS variant to exercise.
#[test]
fn composite_pk_self_join_eliminated_inside_not_exists_subquery() {
    let mut b = Branch {
        core: vec![scan(0, "person")],
        opts: vec![],
        bindings: BTreeMap::new(),
        where_conds: vec![
            SqlCond::IsNotNull(ColRef::new(0, "name")),
            SqlCond::NotExists {
                scans: vec![scan(1, "person"), scan(2, "org"), scan(3, "org")],
                conds: vec![
                    SqlCond::ColEq(ColRef::new(1, "org_id"), ColRef::new(2, "id1")),
                    SqlCond::ColEq(ColRef::new(2, "id1"), ColRef::new(3, "id1")), // PK col 1 — redundant self-join
                    SqlCond::ColEq(ColRef::new(2, "id2"), ColRef::new(3, "id2")), // PK col 2 — redundant self-join
                    SqlCond::IsNotNull(ColRef::new(3, "label")),
                    SqlCond::ColEq(ColRef::new(0, "id"), ColRef::new(1, "id")), // outer correlation
                ],
            },
        ],
        distinct: false,
        limit: None,
        offset: 0,
        order: vec![],
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("name".to_owned(), col_binding(0, "name"));
    let mut org = TableSchema::new("org");
    org.primary_key = vec!["id1".to_owned(), "id2".to_owned()];
    let out = run(vec![b], std::slice::from_ref(&org), &CascadeCtx::default());
    assert_eq!(out.len(), 1);
    let SqlCond::NotExists { scans, conds } = out[0]
        .where_conds
        .iter()
        .find(|c| matches!(c, SqlCond::NotExists { .. }))
        .expect("NOT EXISTS survives")
    else {
        unreachable!()
    };
    assert_eq!(
        scans
            .iter()
            .filter(|s| s.alias == 2 || s.alias == 3)
            .count(),
        1,
        "the redundant composite-PK org self-join inside NOT EXISTS collapses to one scan: {scans:?}"
    );
    // No remaining cond may reference the dropped alias 3.
    assert!(
        !conds.iter().any(|c| {
            let mut has_alias3 = false;
            collect_cond_cols(c, &mut |col| has_alias3 |= col.alias == 3);
            has_alias3
        }),
        "no remaining cond may reference the dropped org alias 3: {conds:?}"
    );
    // Both PK-licensing equalities (id1, id2) are removed; the link cond, the
    // outer correlation, and the rewritten IsNotNull survive: 5 - 2 = 3.
    assert_eq!(
        conds.len(),
        3,
        "exactly the two composite-PK licensing equalities are removed: {conds:?}"
    );
    // The surviving IsNotNull must have been rewritten from alias 3 onto the
    // KEPT alias 2 — proving rewrite_cond_alias actually ran for the composite
    // merge path, not just that scans/conds were dropped. This is the precise
    // shape where the earlier single-column-PK anti-join-FILTER bug lived.
    assert!(
        conds
            .iter()
            .any(|c| matches!(c, SqlCond::IsNotNull(r) if r == &ColRef::new(2, "label"))),
        "IsNotNull(label) must be rewritten onto the kept alias 2: {conds:?}"
    );
    // The outer correlation (alias 0 -> alias 1) must survive untouched.
    assert!(
        conds
            .iter()
            .any(|c| matches!(c, SqlCond::ColEq(a, b) if a.alias == 0 && b.alias == 1)),
        "the outer correlation condition must survive: {conds:?}"
    );
}

/// Negative control: only ONE of the two composite-PK columns is equated
/// between the two `org` scans inside the `NOT EXISTS` — a partial-key match
/// does NOT identify the same row, so no merge may fire. Proves
/// `find_composite_pk_self_join_in` requires ALL PK columns covered at the
/// subquery call site too, not just at the top-level call site.
#[test]
fn composite_pk_self_scans_inside_not_exists_not_merged_on_partial_key_equality() {
    let mut b = Branch {
        core: vec![scan(0, "person")],
        opts: vec![],
        bindings: BTreeMap::new(),
        where_conds: vec![SqlCond::NotExists {
            scans: vec![scan(1, "person"), scan(2, "org"), scan(3, "org")],
            conds: vec![
                SqlCond::ColEq(ColRef::new(1, "org_id"), ColRef::new(2, "id1")),
                SqlCond::ColEq(ColRef::new(2, "id1"), ColRef::new(3, "id1")), // only PK col 1 matched
                SqlCond::ColEq(ColRef::new(0, "id"), ColRef::new(1, "id")),
            ],
        }],
        distinct: false,
        limit: None,
        offset: 0,
        order: vec![],
        path: None,
        agg: None,
        subplan_joins: Vec::new(),
    };
    b.bindings.insert("x".to_owned(), col_binding(0, "id"));
    let mut org = TableSchema::new("org");
    org.primary_key = vec!["id1".to_owned(), "id2".to_owned()];
    let out = run(vec![b], std::slice::from_ref(&org), &CascadeCtx::default());
    let SqlCond::NotExists { scans, .. } = out[0]
        .where_conds
        .iter()
        .find(|c| matches!(c, SqlCond::NotExists { .. }))
        .expect("NOT EXISTS survives")
    else {
        unreachable!()
    };
    assert_eq!(
        scans.len(),
        3,
        "a partial composite-key match must NOT be merged: {scans:?}"
    );
}

// --- LJ→IJ FK-guaranteed downgrade (round-2 coverage) ----------------------
// A NOT NULL FK on a core scan + referential integrity to the opt scan's unique
// (PK) column guarantees a 1:1 match, so OPTIONAL degrades soundly to INNER JOIN.

fn fk_downgrade_schema() -> Vec<TableSchema> {
    // Reuses the trips/routes shape: trips.rid is a NOT NULL FK to routes.route_id (PK).
    gtfs_schema()
}

fn fk_downgrade_branch() -> Branch {
    let mut b = Branch::single(scan(0, "trips"));
    b.opts.push(OptJoin {
        scan: scan(1, "routes"),
        on: vec![SqlCond::NullSafeEq(
            ColRef::new(0, "rid"),
            ColRef::new(1, "route_id"),
        )],
        extra: Vec::new(),
    });
    b.bindings.insert(
        "route".into(),
        iri_template_binding(1, "http://ex/route/", "route_id"),
    );
    b
}

#[test]
fn lj_downgraded_to_ij_on_notnull_fk_to_unique_opt_column() {
    // Call the pass DIRECTLY (not through the full `run()` cascade): this fixture's
    // downgraded shape ALSO qualifies for `fk_pk_join_elimination` (composing
    // correctly — see the full-pipeline test below), which would eliminate the
    // routes scan entirely and mask what THIS pass, in isolation, actually does.
    let mut b = fk_downgrade_branch();
    joinelim::lj_to_ij_fk_downgrade(&mut b, &fk_downgrade_schema());
    assert!(
        b.opts.is_empty(),
        "the OptJoin must be promoted, not left as OPTIONAL"
    );
    assert_eq!(
        b.core.len(),
        2,
        "the opt scan is promoted into core (not eliminated — only downgraded)"
    );
    assert!(
        b.core.iter().any(|s| s.alias == 1),
        "the promoted routes scan keeps its alias"
    );
    // The NullSafeEq becomes a plain ColEq once both sides are proven NOT NULL.
    assert!(
        b.where_conds
            .iter()
            .any(|c| matches!(c, SqlCond::ColEq(a, o) if a.column.as_ref() == "rid" && o.column.as_ref() == "route_id")),
        "ON moved to where_conds as a plain ColEq: {:?}", b.where_conds
    );
}

#[test]
fn lj_downgrade_composes_with_fk_pk_elimination_in_the_full_pipeline() {
    // End-to-end through run(): the downgrade fires first (2b-pre), producing an
    // inner join that THEN qualifies for full FK/PK elimination (pass 4) — the
    // routes scan disappears entirely, same end-state as a query that wrote the
    // join as mandatory (non-OPTIONAL) from the start. Proves the two passes
    // compose soundly rather than leaving a redundant downgraded-but-unmerged join.
    let out = run(
        vec![fk_downgrade_branch()],
        &fk_downgrade_schema(),
        &CascadeCtx::default(),
    );
    let b = &out[0];
    assert!(b.opts.is_empty());
    assert_eq!(
        b.core.len(),
        1,
        "routes fully eliminated after downgrade+FK/PK merge"
    );
    assert!(b.where_conds.is_empty());
}

#[test]
fn lj_not_downgraded_when_fk_nullable() {
    // A nullable FK does NOT guarantee a match — LEFT JOIN must stay OPTIONAL
    // (downgrading would drop rows whose FK is NULL, since INNER JOIN excludes them
    // while LEFT JOIN keeps them with the opt side unbound).
    let mut schema = fk_downgrade_schema();
    schema[0].columns[1].not_null = false; // trips.rid nullable
    let mut b = fk_downgrade_branch();
    joinelim::lj_to_ij_fk_downgrade(&mut b, &schema);
    assert_eq!(b.opts.len(), 1, "nullable FK ⇒ OPTIONAL must be preserved");
}

#[test]
fn lj_not_downgraded_when_opt_column_not_unique() {
    // The opt-side join column must be a unique key (typically the PK) — otherwise
    // the "match" could be many-to-one and downgrading would fan out rows.
    let mut schema = fk_downgrade_schema();
    schema[1].primary_key.clear(); // routes.route_id no longer a declared key
    let mut b = fk_downgrade_branch();
    joinelim::lj_to_ij_fk_downgrade(&mut b, &schema);
    assert_eq!(
        b.opts.len(),
        1,
        "non-unique opt column ⇒ OPTIONAL must be preserved"
    );
}

// --- disjunction/IN-shaped intersection simplify (round-2 coverage) --------
// Two same-column equality-disjunctions (VALUES/IN-shaped) combined by the
// implicit AND of where_conds must simplify to their SET INTERSECTION.

#[test]
fn disjunction_intersection_simplifies_two_in_lists_to_their_overlap() {
    let mut b = Branch::single(scan(0, "emp"));
    let col = ColRef::new(0, "id");
    // (id=1 OR id=2 OR id=3) AND (id=2 OR id=3 OR id=4) => id=2 OR id=3
    b.where_conds.push(SqlCond::Or(vec![
        SqlCond::Cmp(col.clone(), CmpOp::Eq, "1".into()),
        SqlCond::Cmp(col.clone(), CmpOp::Eq, "2".into()),
        SqlCond::Cmp(col.clone(), CmpOp::Eq, "3".into()),
    ]));
    b.where_conds.push(SqlCond::Or(vec![
        SqlCond::Cmp(col.clone(), CmpOp::Eq, "2".into()),
        SqlCond::Cmp(col.clone(), CmpOp::Eq, "3".into()),
        SqlCond::Cmp(col.clone(), CmpOp::Eq, "4".into()),
    ]));
    let out = run(vec![b], &[], &CascadeCtx::default());
    assert_eq!(out.len(), 1, "intersection is non-empty; branch survives");
    assert_eq!(
        out[0].where_conds.len(),
        1,
        "the two disjunctions collapse into one: {:?}",
        out[0].where_conds
    );
    match &out[0].where_conds[0] {
        SqlCond::Or(arms) => {
            let vals: Vec<&str> = arms
                .iter()
                .map(|a| match a {
                    SqlCond::Cmp(_, CmpOp::Eq, v) => v.as_str(),
                    other => panic!("expected Cmp arm, got {other:?}"),
                })
                .collect();
            assert_eq!(vals.len(), 2);
            assert!(vals.contains(&"2") && vals.contains(&"3"));
        }
        other => panic!("expected an Or of the intersection, got {other:?}"),
    }
}

#[test]
fn disjunction_intersection_prunes_branch_when_disjoint() {
    let mut b = Branch::single(scan(0, "emp"));
    let col = ColRef::new(0, "id");
    // (id=1 OR id=2) AND (id=3 OR id=4) => empty intersection => branch unsatisfiable.
    b.where_conds.push(SqlCond::Or(vec![
        SqlCond::Cmp(col.clone(), CmpOp::Eq, "1".into()),
        SqlCond::Cmp(col.clone(), CmpOp::Eq, "2".into()),
    ]));
    b.where_conds.push(SqlCond::Or(vec![
        SqlCond::Cmp(col.clone(), CmpOp::Eq, "3".into()),
        SqlCond::Cmp(col.clone(), CmpOp::Eq, "4".into()),
    ]));
    let out = run(vec![b], &[], &CascadeCtx::default());
    assert!(
        out.is_empty(),
        "disjoint id-lists ⇒ unsatisfiable ⇒ branch pruned"
    );
}

#[test]
fn disjunction_intersection_does_not_fire_on_different_columns() {
    let mut b = Branch::single(scan(0, "emp"));
    // (id=1 OR id=2) AND (dept=3 OR dept=4) — different columns, must NOT be
    // treated as intersectable (they constrain different columns, both must hold).
    b.where_conds.push(SqlCond::Or(vec![
        SqlCond::Cmp(ColRef::new(0, "id"), CmpOp::Eq, "1".into()),
        SqlCond::Cmp(ColRef::new(0, "id"), CmpOp::Eq, "2".into()),
    ]));
    b.where_conds.push(SqlCond::Or(vec![
        SqlCond::Cmp(ColRef::new(0, "dept"), CmpOp::Eq, "3".into()),
        SqlCond::Cmp(ColRef::new(0, "dept"), CmpOp::Eq, "4".into()),
    ]));
    let out = run(vec![b], &[], &CascadeCtx::default());
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].where_conds.len(),
        2,
        "different-column disjunctions must both survive untouched: {:?}",
        out[0].where_conds
    );
}

// --- DISTINCT-driven unused-OPTIONAL pruning (round-2 coverage) ------------
// Under DISTINCT, an OPTIONAL whose bound columns are never projected cannot
// affect the (deduplicated) result — safe to drop regardless of match/no-match.

#[test]
fn unused_opt_pruned_under_distinct_when_not_projected() {
    let mut b = Branch::single(scan(0, "emp"));
    b.opts.push(OptJoin {
        scan: scan(1, "dept"),
        on: vec![SqlCond::NullSafeEq(
            ColRef::new(0, "dept_id"),
            ColRef::new(1, "id"),
        )],
        extra: Vec::new(),
    });
    b.bindings.insert("name".into(), col_binding(0, "name"));
    b.bindings
        .insert("dept_label".into(), col_binding(1, "label"));
    let project = vec!["name".to_owned()]; // dept_label NOT projected
    let ctx = CascadeCtx {
        distinct: true,
        project: Some(&project),
    };
    let out = run(vec![b], &[], &ctx);
    assert!(
        out[0].opts.is_empty(),
        "the unprojected OPTIONAL must be pruned under DISTINCT"
    );
}

#[test]
fn used_opt_kept_under_distinct_when_projected() {
    let mut b = Branch::single(scan(0, "emp"));
    b.opts.push(OptJoin {
        scan: scan(1, "dept"),
        on: vec![SqlCond::NullSafeEq(
            ColRef::new(0, "dept_id"),
            ColRef::new(1, "id"),
        )],
        extra: Vec::new(),
    });
    b.bindings.insert("name".into(), col_binding(0, "name"));
    b.bindings
        .insert("dept_label".into(), col_binding(1, "label"));
    let project = vec!["name".to_owned(), "dept_label".to_owned()]; // dept_label IS projected
    let ctx = CascadeCtx {
        distinct: true,
        project: Some(&project),
    };
    let out = run(vec![b], &[], &ctx);
    assert_eq!(
        out[0].opts.len(),
        1,
        "a projected OPTIONAL must be KEPT even under DISTINCT"
    );
}

#[test]
fn unused_opt_not_pruned_without_distinct() {
    // Without DISTINCT, an unprojected OPTIONAL can still change the projected
    // ROW COUNT (a matching opt row can fan out the core row) — must NOT be
    // pruned when DISTINCT is absent.
    let mut b = Branch::single(scan(0, "emp"));
    b.opts.push(OptJoin {
        scan: scan(1, "dept"),
        on: vec![SqlCond::NullSafeEq(
            ColRef::new(0, "dept_id"),
            ColRef::new(1, "id"),
        )],
        extra: Vec::new(),
    });
    b.bindings.insert("name".into(), col_binding(0, "name"));
    let project = vec!["name".to_owned()];
    let ctx = CascadeCtx {
        distinct: false,
        project: Some(&project),
    };
    let out = run(vec![b], &[], &ctx);
    assert_eq!(
        out[0].opts.len(),
        1,
        "no DISTINCT ⇒ the unprojected OPTIONAL must be kept (fan-out risk)"
    );
}
