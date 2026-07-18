//! Pass (4) of the optimizer cascade — FK/PK join elimination (ADR-0007).
//! Split out of `cascade` to keep each file within the size budget; it consumes
//! the FD/uniqueness proof built by pass (3) in the parent module.

use sf_core::ir::{LogicalSource, Segment, Template, TermMap};
use sf_sql::TableSchema;

use super::{scan_table, Fds};
use crate::iq::{collect_cond_cols, Branch, ColRef, SqlCond, TermDef};

// --- 2b-pre. LJ→IJ FK-guaranteed downgrade --------------------------------

/// Downgrade an OptJoin to an inner join when a `NOT NULL` FK on a core scan
/// guarantees that every core row has exactly one matching optional row. Sound:
/// `NOT NULL FK` + declared referential integrity ⇒ LEFT JOIN always matches 1:1
/// ⇒ LEFT JOIN semantics = INNER JOIN semantics ⇒ `=_bag` preserved.
///
/// Promotes the opt scan to `b.core` and moves the ON + extra conditions to
/// `b.where_conds` (converting `NullSafeEq` → `ColEq` since both sides are
/// NOT NULL after the FK match-guarantee is confirmed). The demoted `OptJoin`
/// is removed from `b.opts`; subsequent cascade passes (3)/(4) see the promoted
/// scan as a normal core scan and may eliminate it further.
pub(super) fn lj_to_ij_fk_downgrade(b: &mut Branch, schema: &[TableSchema]) {
    let mut i = 0;
    while i < b.opts.len() {
        if opt_is_fk_guaranteed(b, i, schema) {
            let opt = b.opts.remove(i);
            // Move ON + extra to where_conds; null-safe equalities become plain
            // column equalities (both sides NOT NULL by FK guarantee).
            for cond in opt.on.into_iter().chain(opt.extra) {
                let inner = match cond {
                    SqlCond::NullSafeEq(a, c) => SqlCond::ColEq(a, c),
                    other => other,
                };
                b.where_conds.push(inner);
            }
            b.core.push(opt.scan);
            // Re-check the same index — a later opt might now also qualify.
        } else {
            i += 1;
        }
    }
}

/// Returns `true` when the OptJoin at `opt_idx` is guaranteed to match every
/// core row: the ON clause is a single (Null)SafeEq between a core FK column and
/// the opt scan's PK column, the FK is declared NOT NULL in the child table, and
/// referential integrity is declared in the schema.
fn opt_is_fk_guaranteed(b: &Branch, opt_idx: usize, schema: &[TableSchema]) -> bool {
    let opt = &b.opts[opt_idx];
    let LogicalSource::Table(opt_table) = &opt.scan.source else {
        return false;
    };
    let opt_alias = opt.scan.alias;
    // ON must be exactly one NullSafeEq or ColEq.
    let (core_col, opt_col) = match opt.on.as_slice() {
        [SqlCond::NullSafeEq(a, c)] | [SqlCond::ColEq(a, c)] => {
            if a.alias != opt_alias && c.alias == opt_alias {
                (a, c) // a is core, c is opt
            } else if c.alias != opt_alias && a.alias == opt_alias {
                (c, a) // c is core, a is opt
            } else {
                return false;
            }
        }
        _ => return false,
    };
    // The opt-side column must be a unique key on the opt table (typically the PK).
    let Some(opt_schema) = schema.iter().find(|t| t.name == *opt_table) else {
        return false;
    };
    if !opt_schema.is_unique_key(&opt_col.column) {
        return false;
    }
    // The core-side must be a declared NOT-NULL FK pointing at opt.
    let Some(core_table) = scan_table(b, core_col.alias) else {
        return false;
    };
    let Some(core_schema) = schema.iter().find(|t| t.name == core_table) else {
        return false;
    };
    let fk_ok = core_schema.foreign_keys.iter().any(|fk| {
        fk.parent_table == *opt_table
            && fk.columns.len() == 1
            && fk.columns[0] == *core_col.column
            && fk.parent_columns.len() == 1
            && fk.parent_columns[0] == *opt_col.column
    });
    fk_ok && column_not_null(core_schema, &core_col.column)
}

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
    // Single-column FK/PK elimination (uses FD uniqueness proof from pass 3).
    while let Some(e) = find_fk_pk_join(b, schema, fds) {
        apply_fk_pk_elim(b, &e);
    }
    // Multi-column composite FK/PK elimination (uniqueness proven from catalog alone).
    while let Some(e) = find_multi_fk_pk_join(b, schema) {
        apply_multi_fk_pk_elim(b, &e);
    }
}

// --- 4b. Multi-column composite FK/PK join elimination ----------------------

/// One eliminable multi-column FK/PK join: all FK column equalities are
/// removed together and every reference to the parent's composite-key columns
/// is rewritten onto the corresponding child FK columns.
struct MultiFkElim {
    /// Indices into `b.where_conds` of the ColEq conditions that form the FK.
    cond_indices: Vec<usize>,
    parent_alias: usize,
    child_alias: usize,
    /// `(parent_col, child_col)` column rewrites (positionally aligned with FK).
    rewrites: Vec<(Box<str>, Box<str>)>,
}

/// Find an eliminable composite FK/PK join. Collects ALL `ColEq` conditions
/// between a pair of scans and checks whether they together match a declared
/// composite FK whose parent columns are a composite key and all child FK
/// columns are NOT NULL. Sound (=_bag) iff BOTH hold, same argument as the
/// single-column variant (ADR-0007).
fn find_multi_fk_pk_join(b: &Branch, schema: &[TableSchema]) -> Option<MultiFkElim> {
    // Collect all cross-scan ColEqs grouped by (child_alias, parent_alias).
    for i in 0..b.core.len() {
        let child_alias = b.core[i].alias;
        let Some(child_table) = scan_table(b, child_alias) else {
            continue;
        };
        let cs = schema.iter().find(|t| t.name == child_table)?;

        for j in 0..b.core.len() {
            if i == j {
                continue;
            }
            let parent_alias = b.core[j].alias;
            let Some(parent_table) = scan_table(b, parent_alias) else {
                continue;
            };
            let ps = schema.iter().find(|t| t.name == parent_table)?;

            // Collect all ColEq conditions between this (child, parent) pair.
            let mut pairs: Vec<(usize, Box<str>, Box<str>)> = Vec::new(); // (cond_idx, child_col, parent_col)
            for (idx, cond) in b.where_conds.iter().enumerate() {
                let SqlCond::ColEq(a, c) = cond else { continue };
                if a.alias == child_alias && c.alias == parent_alias {
                    pairs.push((idx, a.column.clone(), c.column.clone()));
                } else if a.alias == parent_alias && c.alias == child_alias {
                    pairs.push((idx, c.column.clone(), a.column.clone()));
                }
            }
            if pairs.len() < 2 {
                continue; // single-column case handled by pass 4a
            }

            let child_cols: Vec<&str> = pairs.iter().map(|(_, c, _)| c.as_ref()).collect();
            let parent_cols: Vec<&str> = pairs.iter().map(|(_, _, p)| p.as_ref()).collect();

            // Uniqueness: parent join columns form a composite key.
            if !ps.is_composite_key(&parent_cols) {
                continue;
            }
            // Match-guarantee: declared composite FK on child and all child cols NOT NULL.
            // Sound only when each (child_col, parent_col) pair is *positionally aligned*
            // with an FK entry: both set-coverage AND per-pair FK lookup are required.
            // (Set-membership alone is unsound — T4.col2=T3.col2 plus T4.col3=T3.col1
            // looks like a composite FK but the FK actually says T4.col2→T3.col1.)
            let fk_declared = cs.foreign_keys.iter().any(|fk| {
                fk.parent_table == parent_table
                    && fk.columns.len() == child_cols.len()
                    // Every observed (child_col, parent_col) pair must match a positional FK entry.
                    && child_cols.iter().zip(parent_cols.iter()).all(|(cc, pc)| {
                        fk.columns.iter().zip(fk.parent_columns.iter())
                            .any(|(fc, fp)| fc.as_str() == *cc && fp.as_str() == *pc)
                    })
                    // Every FK entry must be covered by an observed pair (completeness).
                    && fk.columns.iter().zip(fk.parent_columns.iter()).all(|(fc, fp)| {
                        child_cols.iter().zip(parent_cols.iter())
                            .any(|(cc, pc)| cc == &fc.as_str() && pc == &fp.as_str())
                    })
            });
            if !fk_declared {
                continue;
            }
            let all_nn = child_cols.iter().all(|cc| column_not_null(cs, cc));
            if !all_nn {
                continue;
            }
            // Parent reached only for its composite key columns.
            if !parent_referenced_only_via_set(b, parent_alias, &parent_cols) {
                continue;
            }

            let cond_indices = pairs.iter().map(|(i, _, _)| *i).collect();
            let rewrites = pairs.into_iter().map(|(_, cc, pc)| (pc, cc)).collect();
            return Some(MultiFkElim {
                cond_indices,
                parent_alias,
                child_alias,
                rewrites,
            });
        }
    }
    None
}

/// Does every reference to `alias` use only the columns in `cols`?
fn parent_referenced_only_via_set(b: &Branch, alias: usize, cols: &[&str]) -> bool {
    let mut ok = true;
    let mut check = |c: &ColRef| {
        if c.alias == alias && !cols.contains(&&*c.column) {
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

fn apply_multi_fk_pk_elim(b: &mut Branch, e: &MultiFkElim) {
    // Remove FK ColEq conditions (highest indices first).
    let mut sorted_idxs = e.cond_indices.clone();
    sorted_idxs.sort_unstable_by(|a, b| b.cmp(a));
    for idx in sorted_idxs {
        b.where_conds.remove(idx);
    }
    // Rewrite parent column references → child column references.
    for def in b.bindings.values_mut() {
        rewrite_parent_def_multi(def, e);
    }
    for cond in &mut b.where_conds {
        rewrite_parent_cond_multi(cond, e);
    }
    for opt in &mut b.opts {
        for cond in opt.on.iter_mut().chain(opt.extra.iter_mut()) {
            rewrite_parent_cond_multi(cond, e);
        }
    }
    b.core.retain(|s| s.alias != e.parent_alias);
}

fn rewrite_parent_colref_multi(c: &mut ColRef, e: &MultiFkElim) {
    if c.alias != e.parent_alias {
        return;
    }
    if let Some((_, child_col)) = e
        .rewrites
        .iter()
        .find(|(parent_col, _)| *parent_col == c.column)
    {
        c.alias = e.child_alias;
        c.column = child_col.clone();
    }
}

fn rewrite_parent_cond_multi(cond: &mut SqlCond, e: &MultiFkElim) {
    match cond {
        SqlCond::ColEq(a, b) | SqlCond::NullSafeEq(a, b) => {
            rewrite_parent_colref_multi(a, e);
            rewrite_parent_colref_multi(b, e);
        }
        SqlCond::Cmp(a, _, _) | SqlCond::IsNotNull(a) | SqlCond::IsNull(a) => {
            rewrite_parent_colref_multi(a, e)
        }
        SqlCond::StrMatch { col, .. } => rewrite_parent_colref_multi(col, e),
        SqlCond::Not(c) => rewrite_parent_cond_multi(c, e),
        SqlCond::And(cs) | SqlCond::Or(cs) => {
            for c in cs {
                rewrite_parent_cond_multi(c, e);
            }
        }
        SqlCond::NotExists { conds, .. }
        | SqlCond::Exists { conds, .. }
        | SqlCond::PathExists { conds, .. } => {
            for c in conds {
                rewrite_parent_cond_multi(c, e);
            }
        }
    }
}

fn rewrite_parent_def_multi(def: &mut TermDef, e: &MultiFkElim) {
    match def {
        TermDef::Const(_) => {}
        TermDef::Derived { term_map, alias } => {
            if *alias == e.parent_alias {
                // Apply each parent→child column rename in the term map.
                let mut tm = term_map.clone();
                for (parent_col, child_col) in &e.rewrites {
                    tm = rename_col_in_term_map(&tm, parent_col, child_col);
                }
                *term_map = tm;
                *alias = e.child_alias;
            }
        }
        TermDef::Coalesce(l, r) => {
            rewrite_parent_def_multi(l, e);
            rewrite_parent_def_multi(r, e);
        }
        TermDef::Concat(parts) => {
            for p in parts {
                rewrite_parent_def_multi(p, e);
            }
        }
        TermDef::Agg { .. } => {}
        // ADR-0032 D2: forced arm (new `TermDef` variant) — recurses through the
        // three components like `Coalesce`/`Concat`. Not reachable in practice: a
        // `ComposedTriple` binding is installed only by `lib.rs`'s env-composed
        // projection override, after this cascade pass has already run.
        TermDef::ComposedTriple {
            subject,
            predicate,
            object,
        } => {
            rewrite_parent_def_multi(subject, e);
            rewrite_parent_def_multi(predicate, e);
            rewrite_parent_def_multi(object, e);
        }
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
        SqlCond::NotExists { conds, .. }
        | SqlCond::Exists { conds, .. }
        | SqlCond::PathExists { conds, .. } => {
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
        // ADR-0032 D2: forced arm (new `TermDef` variant) — see the identical note
        // on `rewrite_parent_def_multi`, above.
        TermDef::ComposedTriple {
            subject,
            predicate,
            object,
        } => {
            rewrite_parent_def(subject, e);
            rewrite_parent_def(predicate, e);
            rewrite_parent_def(object, e);
        }
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
