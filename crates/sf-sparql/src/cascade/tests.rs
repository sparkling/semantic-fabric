//! Cascade unit tests — split into a sibling file to keep `cascade/mod.rs`
//! within the size budget.
#![cfg(test)]

use super::*;
use sf_core::ir::{LogicalSource, Segment, Template, TermMap, TermSpec};
use sf_sql::{Column, ForeignKey};
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
        path: None,
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
fn self_join_not_eliminated_on_nullable_unique_key() {
    // A single-column UNIQUE but NULLABLE column (e.g. `email TEXT UNIQUE`) is
    // not a true key: the base translation's `t0.email = t1.email` already drops
    // NULL-email rows (NULL = NULL ⇒ UNKNOWN); collapsing to a bare scan would
    // re-admit them → extra rows → =_bag break (ADR-0007). Must NOT fire.
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
        path: None,
    };
    let mut ts = TableSchema::new("emp");
    ts.unique = vec![vec!["email".to_owned()]];
    ts.columns = vec![sf_sql::Column::new("email", "text", false)]; // NULLABLE
    let out = run(vec![b], std::slice::from_ref(&ts), &CascadeCtx::default());
    assert_eq!(
        out[0].core.len(),
        2,
        "nullable unique key ⇒ self-join preserved"
    );
    assert_eq!(out[0].where_conds.len(), 1, "the key equality is retained");
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
        path: None,
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
        path: None,
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
        path: None,
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
        path: None,
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
        path: None,
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
        path: None,
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
