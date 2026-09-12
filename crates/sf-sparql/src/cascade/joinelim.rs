//! Pass (4) of the optimizer cascade — FK/PK join elimination (ADR-0007).
//! Split out of `cascade` to keep each file within the size budget; it consumes
//! the FD/uniqueness proof built by pass (3) in the parent module.

use sf_core::ir::{Segment, Template, TermMap};
use sf_sql::TableSchema;

use super::Fds;
use crate::build::control::BuildWork;
use crate::iq::{Branch, R2rmlGraphScope, SqlCond, TermDef};

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
    super::control_join::downgrade(
        b,
        schema,
        BuildWork::new(crate::CompilerWorkMode::Uncontrolled),
    )
    .expect("uncontrolled join promotion cannot refuse");
}

// --- 4. FK/PK join elimination --------------------------------------------

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
    with_work(
        b,
        schema,
        fds,
        BuildWork::new(crate::CompilerWorkMode::Uncontrolled),
    )
    .expect("uncontrolled FK elimination cannot refuse");
}

pub(super) fn with_work(
    b: &mut Branch,
    schema: &[TableSchema],
    fds: &Fds,
    work: BuildWork<'_>,
) -> crate::Result<()> {
    while let Some(e) = find_fk_pk_join(b, schema, fds, work)? {
        apply_multi_with_work(b, &e, work)?;
    }
    // Multi-column composite FK/PK elimination (uniqueness proven from catalog alone).
    while let Some(e) = find_multi_fk_pk_join(b, schema, work)? {
        apply_multi_with_work(b, &e, work)?;
    }
    work.checkpoint()
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
fn find_multi_fk_pk_join(
    b: &Branch,
    schema: &[TableSchema],
    work: BuildWork<'_>,
) -> crate::Result<Option<MultiFkElim>> {
    use super::control_join::{equal, schema_table, table};
    use crate::build::control::BuildVec;
    for (i, child) in b.core.iter().enumerate() {
        work.charge(1)?;
        let Some(child_name) = table(&b.core, child.alias, work)? else {
            continue;
        };
        let Some(cs) = schema_table(schema, child_name, work)? else {
            return Ok(None);
        };
        'parent: for (j, parent) in b.core.iter().enumerate() {
            work.charge(1)?;
            if i == j {
                continue;
            }
            let Some(parent_name) = table(&b.core, parent.alias, work)? else {
                continue;
            };
            let Some(ps) = schema_table(schema, parent_name, work)? else {
                return Ok(None);
            };
            let mut pairs = BuildVec::new(Vec::new());
            for (index, condition) in b.where_conds.iter().enumerate() {
                work.charge(1)?;
                if let SqlCond::ColEq(a, c) = condition {
                    if a.alias == child.alias && c.alias == parent.alias {
                        work.push(&mut pairs, (index, a.column.as_ref(), c.column.as_ref()))?;
                    } else if a.alias == parent.alias && c.alias == child.alias {
                        work.push(&mut pairs, (index, c.column.as_ref(), a.column.as_ref()))?;
                    }
                }
            }
            if pairs.values.len() < 2 {
                continue;
            }
            // Distinct observed columns plus equal cardinality and coverage below
            // prove one-to-one key/FK membership, even for raw catalog input.
            for (index, (_, cc, pc)) in pairs.values.iter().enumerate() {
                work.charge(1)?;
                for (_, other_cc, other_pc) in &pairs.values[..index] {
                    work.charge(1)?;
                    if equal(cc, other_cc, work)? || equal(pc, other_pc, work)? {
                        continue 'parent;
                    }
                }
            }
            let mut parent_cols = work.vector(pairs.values.len())?;
            for (_, _, name) in &pairs.values {
                work.charge(1)?;
                parent_cols.push(*name);
            }
            let mut unique = false;
            'key: for key in std::iter::once(&ps.primary_key).chain(&ps.unique) {
                work.charge(1)?;
                if key.len() != parent_cols.len() {
                    continue;
                }
                for name in &parent_cols {
                    work.charge(1)?;
                    let mut found = false;
                    for candidate in key {
                        work.charge(1)?;
                        if equal(name, candidate, work)? {
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        continue 'key;
                    }
                }
                unique = true;
                break;
            }
            if !unique {
                continue;
            }
            let mut declared = false;
            'fk: for fk in &cs.foreign_keys {
                work.charge(1)?;
                if fk.columns.len() != pairs.values.len()
                    || fk.parent_columns.len() != pairs.values.len()
                    || !equal(&fk.parent_table, parent_name, work)?
                {
                    continue;
                }
                for (_, cc, pc) in &pairs.values {
                    work.charge(1)?;
                    let mut found = false;
                    for (fc, fp) in fk.columns.iter().zip(&fk.parent_columns) {
                        work.charge(1)?;
                        if equal(cc, fc, work)? && equal(pc, fp, work)? {
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        continue 'fk;
                    }
                }
                for (fc, fp) in fk.columns.iter().zip(&fk.parent_columns) {
                    work.charge(1)?;
                    let mut found = false;
                    for (_, cc, pc) in &pairs.values {
                        work.charge(1)?;
                        if equal(cc, fc, work)? && equal(pc, fp, work)? {
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        continue 'fk;
                    }
                }
                declared = true;
                break;
            }
            if !declared {
                continue;
            }
            for (_, cc, _) in &pairs.values {
                work.charge(1)?;
                if !super::control_distinct::non_null(cs, cc, work)? {
                    continue 'parent;
                }
            }
            let covered = if matches!(work.mode, crate::CompilerWorkMode::Uncontrolled) {
                parent_referenced_only_via_set(b, parent.alias, &parent_cols)
            } else {
                super::control_distinct::parent_columns(b, parent.alias, &parent_cols, work)?
            };
            if !covered {
                continue;
            }
            let mut indices = work.vector(pairs.values.len())?;
            let mut rewrites = work.vector(pairs.values.len())?;
            for (index, cc, pc) in pairs.values {
                work.charge(1)?;
                indices.push(index);
                rewrites.push((work.string(pc)?.into(), work.string(cc)?.into()));
            }
            return Ok(Some(MultiFkElim {
                cond_indices: indices,
                parent_alias: parent.alias,
                child_alias: child.alias,
                rewrites,
            }));
        }
    }
    work.checkpoint()?;
    Ok(None)
}

/// Does every reference to `alias` use only the columns in `cols`?
fn parent_referenced_only_via_set(b: &Branch, alias: usize, cols: &[&str]) -> bool {
    if crate::iq::decode_valid::branch_references(b, alias) {
        return false;
    }
    super::control_distinct::parent_columns(
        b,
        alias,
        cols,
        BuildWork::new(crate::CompilerWorkMode::Uncontrolled),
    )
    .expect("uncontrolled parent coverage cannot refuse")
}

fn apply_multi_with_work(
    b: &mut Branch,
    e: &MultiFkElim,
    work: BuildWork<'_>,
) -> crate::Result<()> {
    // Discovery records indices in ascending condition order; no copy/sort needed.
    for &idx in e.cond_indices.iter().rev() {
        super::control_rewrite::remove(&mut b.where_conds, idx, work)?;
    }
    let remap = super::control_conditions::ColumnRemap {
        parent: e.parent_alias,
        child: e.child_alias,
        pairs: &e.rewrites,
    };
    for def in b.bindings.values_mut() {
        work.charge(1)?;
        rewrite_parent_def_multi(def, e, work)?;
    }
    for cond in &mut b.where_conds {
        work.charge(1)?;
        remap.condition(cond, work)?;
    }
    for opt in &mut b.opts {
        work.charge(1)?;
        for cond in opt.on.iter_mut().chain(opt.extra.iter_mut()) {
            work.charge(1)?;
            remap.condition(cond, work)?;
        }
    }
    super::control_rewrite::retain_scan(&mut b.core, e.parent_alias, work)
}

fn rewrite_parent_def_multi(
    def: &mut TermDef,
    e: &MultiFkElim,
    work: BuildWork<'_>,
) -> crate::Result<()> {
    let work = work.enter()?;
    match def {
        TermDef::Const(_) => {}
        TermDef::Derived { term_map, alias } => {
            rewrite_term_map_multi(term_map, alias, e, work)?;
        }
        TermDef::R2rmlBlank {
            term_map,
            alias,
            graph,
        } => {
            rewrite_term_map_multi(term_map, alias, e, work)?;
            if let R2rmlGraphScope::Mapped { term_map, alias } = graph {
                rewrite_term_map_multi(term_map, alias, e, work)?;
            }
        }
        TermDef::Coalesce(l, r) => {
            rewrite_parent_def_multi(l, e, work)?;
            rewrite_parent_def_multi(r, e, work)?;
        }
        TermDef::Concat(parts) => {
            for p in parts {
                work.charge(1)?;
                rewrite_parent_def_multi(p, e, work)?;
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
            rewrite_parent_def_multi(subject, e, work)?;
            rewrite_parent_def_multi(predicate, e, work)?;
            rewrite_parent_def_multi(object, e, work)?;
        }
    }
    work.checkpoint()
}

fn find_fk_pk_join(
    b: &Branch,
    schema: &[TableSchema],
    fds: &Fds,
    work: BuildWork<'_>,
) -> crate::Result<Option<MultiFkElim>> {
    use super::control_join::{equal, schema_table, table, unique};
    for (index, condition) in b.where_conds.iter().enumerate() {
        work.charge(1)?;
        let SqlCond::ColEq(x, y) = condition else {
            continue;
        };
        if x.alias == y.alias {
            continue;
        }
        for (child, parent) in [(x, y), (y, x)] {
            work.charge(1)?;
            let Some(child_name) = table(&b.core, child.alias, work)? else {
                continue;
            };
            let Some(parent_name) = table(&b.core, parent.alias, work)? else {
                continue;
            };
            let Some(cs) = schema_table(schema, child_name, work)? else {
                continue;
            };
            let Some(ps) = schema_table(schema, parent_name, work)? else {
                continue;
            };
            let mut key = false;
            for (det, alias) in &fds.deps {
                work.charge(1)?;
                if *alias == parent.alias && super::control_fd::same(det, parent, work)? {
                    key = true;
                    break;
                }
            }
            if !key || !unique(ps, &parent.column, work)? {
                continue;
            }
            let mut declared = false;
            for fk in &cs.foreign_keys {
                work.charge(1)?;
                if let ([column], [target]) = (fk.columns.as_slice(), fk.parent_columns.as_slice())
                {
                    if equal(&fk.parent_table, parent_name, work)?
                        && equal(column, &child.column, work)?
                        && equal(target, &parent.column, work)?
                    {
                        declared = true;
                        break;
                    }
                }
            }
            if !declared || !super::control_distinct::non_null(cs, &child.column, work)? {
                continue;
            }
            if !super::control_distinct::parent_columns(b, parent.alias, &[&parent.column], work)? {
                continue;
            }
            let mut indices = work.vector(1)?;
            indices.push(index);
            let mut pairs = work.vector(1)?;
            pairs.push((
                work.string(&parent.column)?.into(),
                work.string(&child.column)?.into(),
            ));
            return Ok(Some(MultiFkElim {
                cond_indices: indices,
                parent_alias: parent.alias,
                child_alias: child.alias,
                rewrites: pairs,
            }));
        }
    }
    work.checkpoint()?;
    Ok(None)
}

fn rewrite_term_map_multi(
    term_map: &mut TermMap,
    alias: &mut usize,
    e: &MultiFkElim,
    work: BuildWork<'_>,
) -> crate::Result<()> {
    work.charge(1)?;
    if *alias == e.parent_alias {
        // Look up each original column once: rename chains and swaps must not
        // feed an earlier replacement into a later parent-column lookup.
        *term_map = rename_columns_with_work(term_map, &e.rewrites, work)?;
        *alias = e.child_alias;
    }
    work.checkpoint()
}

pub(super) fn rename_columns_with_work(
    tm: &TermMap,
    pairs: &[(Box<str>, Box<str>)],
    work: crate::build::control::BuildWork<'_>,
) -> crate::Result<TermMap> {
    use crate::plan_measure::clone_root::CompilerCloneRootV1;
    work.charge(1)?;
    let rename = |name: &str| -> crate::Result<Box<str>> {
        for (parent, child) in pairs {
            work.charge(1)?;
            if super::control_join::equal(parent, name, work)? {
                return Ok(work.string(child)?.into());
            }
        }
        Ok(work.string(name)?.into())
    };
    let copy_spec = |spec: &sf_core::ir::TermSpec| -> crate::Result<sf_core::ir::TermSpec> {
        if let crate::CompilerWorkMode::Metered(context) = work.mode {
            context.reserve_ast_copy(CompilerCloneRootV1::TermSpec(spec))?;
        }
        Ok(spec.clone())
    };
    let result = match tm {
        TermMap::Constant(_) => {
            if let crate::CompilerWorkMode::Metered(context) = work.mode {
                context.reserve_ast_copy(CompilerCloneRootV1::TermMap(tm))?;
            }
            tm.clone()
        }
        TermMap::Column(c, spec) => TermMap::Column(rename(c)?, copy_spec(spec)?),
        TermMap::Template(t, spec) => {
            let mut segs = work.vector(t.segments().len())?;
            for segment in t.segments() {
                work.charge(1)?;
                segs.push(match segment {
                    Segment::Column(c) => Segment::Column(rename(c)?),
                    Segment::Literal(value) => Segment::Literal(work.string(value)?.into()),
                });
            }
            work.charge(segs.len())?;
            TermMap::Template(
                Template::from_segments(segs).expect("renamed template is non-empty"),
                copy_spec(spec)?,
            )
        }
    };
    work.checkpoint()?;
    Ok(result)
}
