//! Pass (4) of the optimizer cascade — FK/PK join elimination (ADR-0007).
//! Split out of `cascade` to keep each file within the size budget; it consumes
//! the FD/uniqueness proof built by pass (3) in the parent module.

use sf_core::ir::{Segment, Template, TermMap};
use sf_sql::TableSchema;

use super::{scan_table, Fds};
use crate::iq::{collect_cond_cols, Branch, ColRef, SqlCond, TermDef};

// --- 4. FK/PK join elimination --------------------------------------------

/// One eliminable FK/PK join: the child scan keeps its FK column, the parent
/// scan (reached only for its PK) is dropped, and every reference to the parent
/// PK is rewritten onto the equal child FK column.
struct FkElim {
    cond_idx: usize,
    parent_alias: usize,
    parent_col: Box<str>,
    child_alias: usize,
    child_col: Box<str>,
}

/// Drop a parent scan reached **only for its PK** via a **NOT-NULL FK** (the big
/// Q2/Q3 latency win). Fires only when BOTH integrity facts hold (ADR-0007 — the
/// hardest correctness surface): **uniqueness** — the parent join column is a key
/// (proven by pass (3)'s FD set + the catalog), so the match multiplies no rows;
/// and **match-guarantee** — the child FK column is `NOT NULL` and declared to
/// reference that key, so referential integrity guarantees exactly one parent per
/// child and the inner join drops no rows. Both ⇒ a 1:1 match ⇒ `=_bag` preserved
/// (a nullable FK would re-admit NULL rows on removal; a non-unique target would
/// multiply rows — either breaks the bag). Otherwise a sound no-op.
pub(super) fn fk_pk_join_elimination(b: &mut Branch, schema: &[TableSchema], fds: &Fds) {
    while let Some(e) = find_fk_pk_join(b, schema, fds) {
        apply_fk_pk_elim(b, &e);
    }
}

fn find_fk_pk_join(b: &Branch, schema: &[TableSchema], fds: &Fds) -> Option<FkElim> {
    for (idx, cond) in b.where_conds.iter().enumerate() {
        let SqlCond::ColEq(x, y) = cond else { continue };
        if x.alias == y.alias {
            continue;
        }
        // Try both orientations — either side could be the parent.
        for (child, parent) in [(x, y), (y, x)] {
            if let Some(e) = check_fk_pk(b, schema, fds, idx, child, parent) {
                return Some(e);
            }
        }
    }
    None
}

/// Validate the FK/PK preconditions for the orientation `child.col = parent.col`.
fn check_fk_pk(
    b: &Branch,
    schema: &[TableSchema],
    fds: &Fds,
    cond_idx: usize,
    child: &ColRef,
    parent: &ColRef,
) -> Option<FkElim> {
    // Both must be *core* base-table scans (a parent on an OPTIONAL side must not
    // be dropped — that would change LEFT JOIN semantics; `scan_table` only finds
    // core scans).
    let child_table = scan_table(b, child.alias)?;
    let parent_table = scan_table(b, parent.alias)?;
    let cs = schema.iter().find(|t| t.name == child_table)?;
    let ps = schema.iter().find(|t| t.name == parent_table)?;

    // Uniqueness: the parent join column is a key (FD proof from pass (3) **and**
    // the catalog agree).
    if !fds.is_key(parent) || !ps.is_unique_key(&parent.column) {
        return None;
    }
    // Match-guarantee: a declared FK child.col → parent_table.parent.col, and the
    // child FK column is NOT NULL (every child row has exactly one parent).
    let fk_declared = cs.foreign_keys.iter().any(|fk| {
        fk.parent_table == parent_table
            && fk.columns.len() == 1
            && fk.columns[0].as_str() == &*child.column
            && fk.parent_columns.len() == 1
            && fk.parent_columns[0].as_str() == &*parent.column
    });
    if !fk_declared || !column_not_null(cs, &child.column) {
        return None;
    }
    // The parent scan must be reached *only for its PK*: every reference to the
    // parent alias must be the join column (so it can be rewritten onto the child
    // FK and the scan dropped without losing any other parent column).
    if !parent_referenced_only_via(b, parent.alias, &parent.column) {
        return None;
    }
    Some(FkElim {
        cond_idx,
        parent_alias: parent.alias,
        parent_col: parent.column.clone(),
        child_alias: child.alias,
        child_col: child.column.clone(),
    })
}

/// A column is NOT NULL iff it is a PK column (PK ⇒ NOT NULL) or the catalog
/// declares it `NOT NULL`.
fn column_not_null(t: &TableSchema, col: &str) -> bool {
    t.primary_key.iter().any(|c| c == col) || t.column(col).is_some_and(|c| c.not_null)
}

/// Does every reference to `alias` (bindings + WHERE + OPTIONAL conditions) use
/// only column `col`? If so the parent scan contributes nothing but its PK.
fn parent_referenced_only_via(b: &Branch, alias: usize, col: &str) -> bool {
    let mut ok = true;
    let mut check = |c: &ColRef| {
        if c.alias == alias && &*c.column != col {
            ok = false;
        }
    };
    for def in b.bindings.values() {
        for c in def.columns() {
            check(&c);
        }
    }
    for cond in &b.where_conds {
        collect_cond_cols(cond, &mut check);
    }
    for opt in &b.opts {
        for cond in opt.on.iter().chain(opt.extra.iter()) {
            collect_cond_cols(cond, &mut check);
        }
    }
    ok
}

/// Fire the elimination: drop the join equality, rewrite `(parent_alias,
/// parent_col)` → `(child_alias, child_col)` everywhere (they are provably equal),
/// then remove the parent scan.
fn apply_fk_pk_elim(b: &mut Branch, e: &FkElim) {
    b.where_conds.remove(e.cond_idx);
    for def in b.bindings.values_mut() {
        rewrite_parent_def(def, e);
    }
    for cond in &mut b.where_conds {
        rewrite_parent_cond(cond, e);
    }
    for opt in &mut b.opts {
        for cond in opt.on.iter_mut().chain(opt.extra.iter_mut()) {
            rewrite_parent_cond(cond, e);
        }
    }
    b.core.retain(|s| s.alias != e.parent_alias);
}

fn rewrite_parent_colref(c: &mut ColRef, e: &FkElim) {
    if c.alias == e.parent_alias && c.column == e.parent_col {
        c.alias = e.child_alias;
        c.column = e.child_col.clone();
    }
}

fn rewrite_parent_cond(cond: &mut SqlCond, e: &FkElim) {
    match cond {
        SqlCond::ColEq(a, b) | SqlCond::NullSafeEq(a, b) => {
            rewrite_parent_colref(a, e);
            rewrite_parent_colref(b, e);
        }
        SqlCond::Cmp(a, _, _) | SqlCond::IsNotNull(a) | SqlCond::IsNull(a) => {
            rewrite_parent_colref(a, e)
        }
        SqlCond::StrMatch { col, .. } => rewrite_parent_colref(col, e),
        SqlCond::Not(c) => rewrite_parent_cond(c, e),
        SqlCond::And(cs) | SqlCond::Or(cs) => {
            for c in cs {
                rewrite_parent_cond(c, e);
            }
        }
        // A MINUS anti-join's or FILTER EXISTS semi-join's correlation may reference
        // the eliminated parent alias; recurse so it tracks the child. (Branches
        // carrying subquery conds bypass the cascade, so this is defensive.)
        SqlCond::NotExists { conds, .. } | SqlCond::Exists { conds, .. } => {
            for c in conds {
                rewrite_parent_cond(c, e);
            }
        }
    }
}

/// Rewrite a term def that reads the parent PK at the parent alias so it reads
/// the equal child FK column at the child alias (the constructed term — e.g. the
/// parent subject IRI — is byte-identical because the values are equal).
fn rewrite_parent_def(def: &mut TermDef, e: &FkElim) {
    match def {
        TermDef::Const(_) => {}
        TermDef::Derived { term_map, alias } => {
            if *alias == e.parent_alias {
                *term_map = rename_col_in_term_map(term_map, &e.parent_col, &e.child_col);
                *alias = e.child_alias;
            }
        }
        TermDef::Coalesce(l, r) => {
            rewrite_parent_def(l, e);
            rewrite_parent_def(r, e);
        }
        TermDef::Concat(parts) => {
            for p in parts {
                rewrite_parent_def(p, e);
            }
        }
        // An aggregate result reads its synthetic group alias, never a base-scan
        // parent alias — nothing to rewrite (agg branches bypass this cascade).
        TermDef::Agg { .. } => {}
    }
}

/// Rebuild a term map with column `from` renamed to `to` (the FK column carries
/// the same value as the renamed PK column, so the generated term is unchanged).
fn rename_col_in_term_map(tm: &TermMap, from: &str, to: &str) -> TermMap {
    match tm {
        TermMap::Constant(t) => TermMap::Constant(t.clone()),
        TermMap::Column(c, spec) => {
            let name: Box<str> = if &**c == from { to.into() } else { c.clone() };
            TermMap::Column(name, spec.clone())
        }
        TermMap::Template(t, spec) => {
            let segs = t
                .segments()
                .iter()
                .map(|s| match s {
                    Segment::Column(c) if &**c == from => Segment::Column(to.into()),
                    other => other.clone(),
                })
                .collect();
            // from_segments only fails on an empty list; the source template was
            // non-empty, so the renamed copy is too.
            TermMap::Template(
                Template::from_segments(segs).expect("renamed template is non-empty"),
                spec.clone(),
            )
        }
    }
}
