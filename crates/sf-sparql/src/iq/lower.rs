//! Lower — the operator-tree ([`IqNode`]) LOWER stage (ADR-0023 M3c,
//! `docs/design/ADR-0023-M3-resolution-pipeline.md` §5). It consumes a **NORMALIZED**
//! tree (the output of [`crate::iq::normalize::normalize`] — a `Union`-of-(`Construction`
//! over a `Join`/`LeftJoin`/`Filter` of `Extensional`/`Values`/`Path` leaves) under the
//! query-modifier spine, every leaf-CQ carrying ONE bindings map) and folds it
//! **bottom-up** into a [`crate::Plan`]: a bag-union of [`Branch`]es plus the solution
//! modifiers. **This is the single point where FILTER/BIND resolve** — per leaf-CQ,
//! against the now-known per-branch bindings, by reusing the *identical* flat
//! [`filter_cond`]/[`bind_term_def`] the live [`crate::unfold`] Filter/Extend arms call.
//!
//! ## Status: tree path only (NOT the live engine)
//!
//! This is M3c; it is **not** wired into the live [`Plan`](crate::Plan)/exec/unfold
//! path. The flat [`crate::unfold`] stays the production engine and the proven oracle.
//! `cargo test --workspace` must stay green with the flat path byte-for-byte unchanged.
//!
//! ## The fold, per node kind (design §5)
//!
//! The core is [`lower_node`] returning `Vec<Branch>` (a node may lower to several
//! branches — a `Union` arm-per-branch, a multi-branch OPTIONAL decomposition):
//!
//! * **`Extensional`** → a core [`Scan`] ([`Branch::single`]). The bridge leaves
//!   `bind` empty; constant-position constraints arrive as [`IqCond::Sql`] in the
//!   enclosing `InnerJoin`/`Filter` and lower with the conds (NOT re-derived here).
//! * **`InnerJoin`** → lower each child, cross-product+merge via the flat
//!   [`join_branches`] (the proven `merge` — empty leaf bindings ⇒ a pure CROSS JOIN,
//!   the shared-var equalities ride `cond` as [`IqCond::Sql`]), then resolve each
//!   `cond` into `where_conds`.
//! * **`LeftJoin`** → §5.3 dispatcher: lower left/right to `Vec<Branch>` and hand BOTH
//!   to [`left_join_branches`] **verbatim** (`leftjoin.rs:27`), which routes
//!   single-scan→`build_left_join` (NullSafeEq ON, R5 inner-FILTER, R2 Coalesce) and
//!   multi-branch/multi-scan→the ISWC-2018 `(P⋈R)∪(P−R)` decomposition. NOT split here.
//! * **`Filter`** → resolve each [`IqCond`] into `where_conds` PER resulting branch
//!   (R4 loop): `Expr` via [`filter_cond`]; `Sql` passes through; `Exists`/`NotExists`
//!   via [`lower_iq_exists`] (the flat `lower_exists` correlated semi/anti-join, ported).
//! * **`Construction`** → fold `subst` entry-by-entry into each branch's `bindings`
//!   (`Resolved(td)` inserts; `Expr(e)` resolves via [`bind_term_def`] against the
//!   now-known per-branch bindings), then restrict to `project`.
//! * **`Union`** → bag union (§5.2, R3): one `Branch` per arm carrying ONLY its own
//!   bindings; an unbound projected var stays ABSENT, never padded to a `TermDef`.
//! * **`Values`** → core-less `Const` branches (`UNDEF` ⇒ absent var).
//! * **`Path`** → [`Branch::path`] (mutually exclusive with `core`).
//! * **`Empty`** → zero branches (bag-union identity); **`True`** → one empty-tuple branch.
//!
//! [`lower`] peels the query-modifier spine (`Distinct`/`Slice`/`OrderBy`) onto the
//! [`Plan`](crate::Plan), and dispatches an `Aggregation` to a single-branch
//! [`Branch::agg`] (SQL `GROUP BY`) or a multi-branch [`Plan::rust_group`] (Rust group),
//! exactly as the flat [`crate::unfold::Unfolder::group`] chooses by child branch count.
//!
//! ## §5.4 tracked sound-501s (emitted AT LOWER, never silent)
//!
//! A subquery-as-join-operand / nested `Aggregation`/`Distinct`/`Slice`/`OrderBy` as a
//! join INPUT, an `Agg`-over-`Path`, a `Path`-joined-with-a-pattern, and HAVING
//! (`Filter` over an `Aggregation`) all need the §5.1 SubPlan derived-table that is M5/M7
//! scope; each is an [`Error::Unsupported`] here (a variable graph is already a build 501).
//! (Multi-scan/Union OPTIONAL right is **NOT** a 501 — it lowers via §5.3.)

use std::collections::BTreeMap;

use spargebra::algebra::Expression;

use crate::iq::node::{AggArg, AggDef, BindDef, IqCond, IqNode, Var};
use crate::iq::{
    AggCol, Aggregation, Branch, ColRef, GroupKey, OrderKey, RustAgg, RustGroup, SqlCond,
    SubPlanJoin, TermDef,
};
use crate::leftjoin::{inner_join_one, left_join_branches, not_exists_cond_for};
use crate::unfold::{group_key_columns, join_branches, single_column_of};
use crate::unify::{bind_term_def, filter_cond, unify, Unify};
use crate::{Error, Plan, PlanForm, Result};

/// Scan the entire `IqNode` tree to find the maximum scan alias in use.
/// Used by [`lower`] to initialize a fresh alias counter that never collides
/// with any scan alias produced by the RESOLVE pass across all subtrees.
fn max_alias_in_tree(node: &IqNode) -> usize {
    match node {
        IqNode::Extensional { scan, .. } => scan.alias,
        IqNode::InnerJoin { children, .. } => {
            children.iter().map(max_alias_in_tree).max().unwrap_or(0)
        }
        IqNode::LeftJoin { left, right, .. } => {
            max_alias_in_tree(left).max(max_alias_in_tree(right))
        }
        IqNode::Filter { child, .. }
        | IqNode::Construction { child, .. }
        | IqNode::Distinct { child }
        | IqNode::Aggregation { child, .. }
        | IqNode::Slice { child, .. }
        | IqNode::OrderBy { child, .. } => max_alias_in_tree(child),
        IqNode::Union { children, .. } => children.iter().map(max_alias_in_tree).max().unwrap_or(0),
        IqNode::Path { closure } => closure.alias,
        IqNode::Values { .. }
        | IqNode::Empty { .. }
        | IqNode::True
        | IqNode::Intensional { .. }
        | IqNode::UnresolvedPath { .. } => 0,
    }
}

/// Lower a NORMALIZED tree to a [`Plan`] (design §5). Peels the query-modifier spine
/// (`Distinct`/`Slice`/`OrderBy`) and the `Aggregation` strategy choice onto the plan,
/// then folds the relational body to a bag-union of [`Branch`]es via [`lower_node`].
/// `form` is `SELECT` over the outermost projected scope (the tree models the WHERE
/// pattern + modifiers; CONSTRUCT/ASK form is a `Query`-level concern out of M3c scope).
pub fn lower(node: IqNode, dialect: sf_sql::Dialect) -> Result<Plan> {
    let mut next_alias = max_alias_in_tree(&node) + 1;
    let mut spine = Spine::default();
    let branches = lower_spine(node, dialect, &mut spine, &mut next_alias)?;
    let vars = spine
        .project
        .map(|p| p.iter().map(|v| v.to_string()).collect())
        .unwrap_or_else(|| visible_vars(&branches));
    Ok(Plan {
        branches,
        form: PlanForm::Select { vars },
        distinct: spine.distinct,
        limit: spine.limit,
        offset: spine.offset,
        order: spine.order,
        rust_group: spine.rust_group,
        dialect,
    })
}

/// The solution modifiers peeled off the spine onto the [`Plan`] (mirrors the flat
/// `TransPattern` fields). `project` is the outermost projected variable set (the
/// `SELECT` scope / the `Aggregation` output scope), used to build [`PlanForm::Select`].
#[derive(Default)]
struct Spine {
    distinct: bool,
    limit: Option<usize>,
    offset: usize,
    order: Vec<OrderKey>,
    rust_group: Option<RustGroup>,
    project: Option<Vec<Var>>,
}

/// Peel the query-modifier spine (`Distinct`/`Slice`/`OrderBy`/`Aggregation` and a pure
/// projection `Construction` over them) onto `spine`, then fold the relational body to
/// branches. The flat `prepared_branches` applies DISTINCT/LIMIT/OFFSET at emission and
/// ORDER BY in `exec`, so here we only record them (design §5 Modifiers).
fn lower_spine(
    node: IqNode,
    dialect: sf_sql::Dialect,
    spine: &mut Spine,
    next_alias: &mut usize,
) -> Result<Vec<Branch>> {
    match node {
        IqNode::Distinct { child } => {
            spine.distinct = true;
            lower_spine(*child, dialect, spine, next_alias)
        }
        IqNode::Slice {
            child,
            offset,
            limit,
        } => {
            spine.offset = offset;
            spine.limit = limit;
            lower_spine(*child, dialect, spine, next_alias)
        }
        IqNode::OrderBy { child, keys } => {
            spine.order = keys;
            lower_spine(*child, dialect, spine, next_alias)
        }
        IqNode::Aggregation {
            child,
            grouping,
            aggs,
        } => lower_aggregation(*child, grouping, aggs, dialect, spine, next_alias),
        // A `Construction` over a spine node — the SELECT projection / a post-GROUP-BY
        // `(agg AS ?v)` Extend over an `Aggregation`/`Distinct`/`Slice`/`OrderBy`. Record
        // the projected scope, lower the spine, then fold this `subst` into the (now
        // grouped/modified) branches and restrict to `project` — the post-spine binding is
        // resolved against the spine's output scope (e.g. `?c := <internal agg var>`).
        IqNode::Construction {
            child,
            subst,
            project,
        } if matches!(
            *child,
            IqNode::Aggregation { .. }
                | IqNode::Distinct { .. }
                | IqNode::Slice { .. }
                | IqNode::OrderBy { .. }
        ) =>
        {
            spine.project.get_or_insert_with(|| project.clone());
            let mut branches = lower_spine(*child, dialect, spine, next_alias)?;
            // A MULTI-branch aggregation lowers to a `rust_group`: the aggregate outputs
            // are computed in Rust AFTER grouping, so they are NOT columns of the pre-group
            // union branches. The outer `Construction`'s `(agg AS ?v)` Extend must rewrite
            // the `RustGroup` output names — NOT fold into the branches (which would fail
            // "BIND references unbound" on the internal aggregate var, the agg-over-UNION
            // bug, design §4.14). The branches feed `rust_group_execute` by variable name,
            // so they keep their full bindings (the grouped result is rebuilt from the keys
            // + renamed agg outputs).
            if let Some(rg) = spine.rust_group.as_mut() {
                rename_rust_group_outputs(&subst, rg)?;
                return Ok(branches);
            }
            for b in &mut branches {
                fold_subst(&subst, b)?;
                b.bindings
                    .retain(|k, _| project.iter().any(|p| p.as_ref() == k.as_str()));
            }
            Ok(branches)
        }
        // The relational body (a leaf-CQ `Construction`, a `Union` of leaf-CQs, or a bare
        // leaf): the projected scope is its output scope; fold it to branches.
        other => {
            spine.project.get_or_insert_with(|| other.output_vars());
            lower_node(other, dialect, false, next_alias)
        }
    }
}

/// Fold a relational subtree to a bag of [`Branch`]es (design §5), bottom-up. A node may
/// yield several branches: a `Union` arm-per-branch, a multi-branch OPTIONAL decomposition.
///
/// `decompose` forces every `LeftJoin` in this subtree to lower to its OPTS-FREE
/// `(P⋈R)∪(P−R)` form (never the efficient single-scan `OptJoin`). It is set ONLY while
/// lowering the RIGHT operand of an enclosing `LeftJoin` (§5.3 nested-right closure): the
/// right of a `LeftJoin` must be opts-free to be re-feedable into [`left_join_branches`].
/// At top level (and on a `LeftJoin`'s LEFT operand) it stays `false`, so the simple
/// non-nested OPTIONAL keeps the efficient `OptJoin` path (no perf regression).
fn lower_node(
    node: IqNode,
    dialect: sf_sql::Dialect,
    decompose: bool,
    next_alias: &mut usize,
) -> Result<Vec<Branch>> {
    match node {
        // ---- leaves --------------------------------------------------------------
        IqNode::Extensional { scan, bind } => {
            // The RESOLVE bridge leaves `bind` empty: all join/constant logic rides the
            // enclosing `IqCond::Sql` conds (design §5 Extensional). A populated `bind`
            // would need a separate lowering path we never reach in M3 → sound 501.
            if !bind.is_empty() {
                return Err(Error::Unsupported(
                    "Extensional.bind is not populated by the M3 RESOLVE bridge → 501".to_owned(),
                ));
            }
            Ok(vec![Branch::single(scan)])
        }
        IqNode::Values { vars, rows } => {
            // One core-less `Const` branch per row; an UNDEF (`None`) cell leaves the
            // variable absent (design §5 Values — mirrors the flat `Values` arm).
            let mut branches = Vec::with_capacity(rows.len());
            for row in rows {
                let mut b = Branch::empty();
                for (var, cell) in vars.iter().zip(row) {
                    if let Some(td) = cell {
                        b.bindings.insert(var.to_string(), td);
                    }
                }
                branches.push(b);
            }
            Ok(branches)
        }
        IqNode::Path { closure } => {
            let mut b = Branch::empty();
            b.path = Some(closure);
            Ok(vec![b])
        }
        IqNode::Empty { .. } => Ok(Vec::new()),
        IqNode::True => Ok(vec![Branch::empty()]),

        // ---- n-ary inner join ----------------------------------------------------
        IqNode::InnerJoin { children, cond } => {
            // Cross-product+merge the children via the proven flat `join_branches`. The
            // leaf bodies carry no bindings (those ride the outer Construction), so the
            // merge is a pure CROSS JOIN; the shared-var equalities ride `cond`.
            let mut acc = vec![Branch::empty()];
            for child in children {
                let cbr = lower_node(child, dialect, decompose, next_alias)?;
                acc = join_branches(acc, cbr)?;
                if acc.is_empty() {
                    break;
                }
            }
            for b in &mut acc {
                apply_conds(&cond, b, dialect)?;
            }
            Ok(acc)
        }

        // ---- left join: §5.3 dispatcher, reuse left_join_branches verbatim --------
        IqNode::LeftJoin { left, right, cond } => {
            // The LEFT operand inherits the enclosing `decompose` context (a left-nested
            // OPTIONAL is already opts-free-compatible: `left_join_branches` only requires
            // the RIGHT to be opts-free, so the left keeps the efficient path at top level).
            let l = lower_node(*left, dialect, decompose, next_alias)?;
            // The RIGHT operand MUST lower to OPTS-FREE branches to be re-feedable into
            // `left_join_branches` (§5.3 nested-right closure): force any OPTIONAL inside
            // the right to its `(P⋈R)∪(P−R)` decomposition rather than the OptJoin form.
            let r = lower_node(*right, dialect, true, next_alias)?;
            // The OPTIONAL ON-expression (R5 inner FILTER) is reconstructed to a single
            // `Expression` for `left_join_branches`/`build_left_join`, which lower it
            // against the COMBINED left+right bindings (we MUST NOT change that scope).
            let expr = iqconds_to_expr(&cond)?;
            if decompose {
                // This `LeftJoin` is ITSELF a right operand: its own output must be
                // opts-free, so force the decomposition (never the single-scan OptJoin).
                left_join_decomposed(l, r, expr.as_ref(), dialect)
            } else {
                // Top-level / left-nested OPTIONAL: the efficient context-dependent
                // choice (single-scan right ⇒ OptJoin; multi-branch/multi-scan ⇒ decomp).
                left_join_branches(l, r, expr.as_ref(), dialect)
            }
        }

        // ---- selection: resolve each cond per resulting branch (R4) ---------------
        IqNode::Filter { child, cond } => {
            let mut branches = lower_node(*child, dialect, decompose, next_alias)?;
            for b in &mut branches {
                apply_conds(&cond, b, dialect)?;
            }
            Ok(branches)
        }

        // ---- bag union (§5.2, R3): one branch per arm, own bindings, absent unbound -
        IqNode::Union { children, .. } => {
            let mut out = Vec::new();
            for c in children {
                out.extend(lower_node(c, dialect, decompose, next_alias)?);
            }
            Ok(out)
        }

        // ---- substitution carrier: fold subst into each branch, restrict to project -
        IqNode::Construction {
            child,
            subst,
            project,
        } => {
            // A leaf-CQ `Construction` may sit over a `Filter` of its relational body
            // (NORMALIZE pushes a FILTER below the Construction). The FILTER must resolve
            // AFTER `subst` establishes the bindings (the flat order: translate the inner
            // pattern, THEN FILTER — `unfold.rs:135-142`), so peel the leading FILTER(s)
            // and apply their conds per branch once the bindings are in place (R4).
            let (body, filters) = peel_filters(*child);
            let branches = lower_node(body, dialect, decompose, next_alias)?;
            let mut out = Vec::with_capacity(branches.len());
            for mut b in branches {
                // A `fold_subst` shared-var unify may prove the branch unsatisfiable
                // (provably disjoint constants) — drop it, mirroring the flat `merge`
                // `None` prune (§5 / R4), never a silent wrong row.
                if !fold_subst(&subst, &mut b)? {
                    continue;
                }
                for cond in &filters {
                    apply_conds(cond, &mut b, dialect)?;
                }
                b.bindings
                    .retain(|k, _| project.iter().any(|p| p.as_ref() == k.as_str()));
                out.push(b);
            }
            Ok(out)
        }

        // ---- §5.4 SubPlan derived-table lowering (ADR-0023 M5 Wave 2) --
        // A modifier node (Aggregation/Distinct/Slice/OrderBy) appearing as a JOIN or
        // FILTER operand (not the spine) is lowered to its own complete Plan and emitted
        // as `(SELECT …) AS t{alias}` — a derived table joined via INNER JOIN in the parent
        // branch. Each projected variable maps to the derived table's positional column
        // `c{i}` (the `emit_branch` naming convention), so the parent reconstruction
        // reads `t{alias}.c{i}` correctly.
        IqNode::Aggregation { .. }
        | IqNode::Distinct { .. }
        | IqNode::Slice { .. }
        | IqNode::OrderBy { .. } => lower_as_subplan(node, dialect, next_alias),
        IqNode::Intensional { .. } => Err(Error::Unsupported(
            "Intensional survived to LOWER — the RESOLVE invariant (ZERO Intensional) \
             was violated → 501"
                .to_owned(),
        )),
        IqNode::UnresolvedPath { .. } => Err(Error::Unsupported(
            "UnresolvedPath survived to LOWER — RESOLVE must compile it to an IqNode::Path \
             (the ZERO UnresolvedPath invariant was violated) → 501"
                .to_owned(),
        )),
    }
}

/// Fold a `Construction` substitution into one branch's bindings (design §5
/// Construction). `Resolved(td)` inserts straight; `Expr(e)` resolves via the flat
/// [`bind_term_def`] against the now-known per-branch bindings (R4: the same fn the live
/// `Extend` arm calls, here per resulting branch). Resolved entries are folded first so a
/// `BIND` can reference a triple-resolved variable; symbolic entries then resolve in
/// dependency order (a multi-pass fixpoint so `BIND(?y:=?x) . BIND(?z:=?y)` resolves
/// regardless of the `BTreeMap` order — a still-unresolvable entry stays a sound 501).
/// Fold a `Construction`'s `subst` into one branch's bindings (design §5 Construction).
/// Returns `Ok(false)` when the fold proved the branch **unsatisfiable** (a shared-var
/// unify yielded `Empty`) so the caller drops it — mirroring the flat `merge` `None`
/// prune (`unfold.rs:1194`).
///
/// A variable the branch ALREADY binds (e.g. it joined a `Values` leaf that bound it
/// per-row, or two leaf-CQs share a constructed var) is NOT overwritten: the incoming
/// definition is **unified** against the existing one via the proven [`unify`] oracle —
/// `Sat` conds append to `where_conds` (the natural-join equality), `Empty` drops the
/// branch, `Unsupported` is a tracked sound-501. This is the same variable-by-variable
/// rule the flat [`merge`](crate::unfold) applies (`unfold.rs:1190-1201`); without it a
/// `Join(BGP, VALUES)` (or any pre-bound shared var) degenerates to a cartesian product.
fn fold_subst(subst: &BTreeMap<Var, BindDef>, b: &mut Branch) -> Result<bool> {
    let mut pending: Vec<(&Var, &Expression)> = Vec::new();
    for (v, def) in subst {
        match def {
            BindDef::Resolved(td) => {
                if insert_or_unify(b, v, td.clone())? {
                    return Ok(false); // provably disjoint ⇒ drop the branch
                }
            }
            BindDef::Expr(e) => pending.push((v, e)),
        }
    }
    while !pending.is_empty() {
        let mut next: Vec<(&Var, &Expression)> = Vec::new();
        let mut last_err: Option<String> = None;
        let progressed_before = pending.len();
        for (v, e) in pending {
            match bind_term_def(e, &b.bindings) {
                Ok(td) => {
                    if insert_or_unify(b, v, td)? {
                        return Ok(false);
                    }
                }
                Err(why) => {
                    last_err = Some(why);
                    next.push((v, e));
                }
            }
        }
        if next.len() == progressed_before {
            // A whole pass resolved nothing — the remaining entries are genuinely
            // unsupported / unbound (never silently dropped, design §5.1 R4).
            return Err(Error::Unsupported(last_err.unwrap_or_else(|| {
                "BIND expression could not be resolved at LOWER → 501".to_owned()
            })));
        }
        pending = next;
    }
    Ok(true)
}

/// Insert `td` as the branch's binding for `v`, or — when `v` is already bound —
/// **unify** the existing and incoming definitions (the flat `merge` rule, keeping the
/// existing binding and appending the equality conds). Returns `Ok(true)` iff the two
/// are provably disjoint (`Unify::Empty`), signalling the branch is unsatisfiable.
fn insert_or_unify(b: &mut Branch, v: &Var, td: TermDef) -> Result<bool> {
    match b.bindings.get(v.as_ref()) {
        None => {
            b.bindings.insert(v.to_string(), td);
            Ok(false)
        }
        Some(existing) => match unify(existing, &td) {
            Unify::Sat(conds) => {
                b.where_conds.extend(conds);
                Ok(false)
            }
            Unify::Empty => Ok(true),
            Unify::Unsupported(why) => Err(Error::Unsupported(why)),
        },
    }
}

/// Peel the leading `Filter` node(s) directly under a `Construction`, returning the
/// relational body and the peeled condition groups (outermost first). These FILTERs
/// resolve AFTER the `Construction`'s `subst` establishes the per-branch bindings (R4 /
/// flat order). A FILTER nested inside an `InnerJoin` as a sub-CQ keeps its own
/// `Construction` and is handled when that sub-CQ lowers — not peeled here.
fn peel_filters(mut node: IqNode) -> (IqNode, Vec<Vec<IqCond>>) {
    let mut filters = Vec::new();
    loop {
        match node {
            IqNode::Filter { child, cond } => {
                filters.push(cond);
                node = *child;
            }
            other => return (other, filters),
        }
    }
}

/// Resolve a conjunction of [`IqCond`]s against `b` and push each into `b.where_conds`
/// (design §5 Filter / InnerJoin). Applied PER resulting branch (R4 loop, mirroring the
/// live `Filter` arm `unfold.rs:136-142`), so each symbolic `Expr`/`Exists` sees the
/// branch's own single bindings map.
fn apply_conds(conds: &[IqCond], b: &mut Branch, dialect: sf_sql::Dialect) -> Result<()> {
    for c in conds {
        let sql = lower_iq_cond(c, b, dialect)?;
        b.where_conds.push(sql);
    }
    Ok(())
}

/// Lower one [`IqCond`] to a [`SqlCond`] against the resolving branch `outer` (design §5
/// Filter, R4). `Sql` passes through; `Expr` resolves via the flat [`filter_cond`] (the
/// SAME fn the live FILTER path delegates leaves to — a var bound to a constructed term
/// is opaque to it and defers to a sound 501); the boolean combinators recurse;
/// `Exists`/`NotExists` build the correlated semi/anti-join via [`lower_iq_exists`].
fn lower_iq_cond(cond: &IqCond, outer: &Branch, dialect: sf_sql::Dialect) -> Result<SqlCond> {
    match cond {
        IqCond::Sql(s) => Ok(s.clone()),
        IqCond::Expr(e) => filter_cond(e, &outer.bindings, dialect).map_err(Error::Unsupported),
        IqCond::And(cs) => Ok(SqlCond::And(
            cs.iter()
                .map(|c| lower_iq_cond(c, outer, dialect))
                .collect::<Result<_>>()?,
        )),
        IqCond::Or(cs) => Ok(SqlCond::Or(
            cs.iter()
                .map(|c| lower_iq_cond(c, outer, dialect))
                .collect::<Result<_>>()?,
        )),
        IqCond::Not(c) => Ok(SqlCond::Not(Box::new(lower_iq_cond(c, outer, dialect)?))),
        IqCond::Exists(n) => lower_iq_exists(n, outer, false, dialect),
        IqCond::NotExists(n) => lower_iq_exists(n, outer, true, dialect),
    }
}

/// `EXISTS { P }` / `NOT EXISTS { P }` (and `MINUS`) → a correlated semi/anti-join
/// [`SqlCond`] (design §5 Filter; SPARQL §8.3/§8.4). A verbatim port of the flat
/// `Unfolder::lower_exists`, sourcing the inner branches from [`lower_node`] (the inner
/// `IqNode` is already RESOLVED+NORMALIZED) instead of `translate_pattern`: each inner
/// branch correlates to the outer row by raw-key equality on every shared variable
/// (term-construction lifting); a shared var that may be UNBOUND on the outer side (reads
/// an OPTIONAL alias) defers → 501 (never a wrong `NULL = value`). For NOT EXISTS every
/// branch must fail (AND of `NotExists`); for EXISTS at least one must match (OR of
/// `Exists`) — only existence is tested, so right multiplicity is irrelevant (`=_bag`).
fn lower_iq_exists(
    node: &IqNode,
    outer: &Branch,
    negated: bool,
    dialect: sf_sql::Dialect,
) -> Result<SqlCond> {
    let inner = lower_node(node.clone(), dialect, false, &mut 0)?;
    if inner.is_empty() {
        // P produces no branches (unmapped): EXISTS → always false, NOT EXISTS → true.
        return Ok(if negated {
            SqlCond::And(Vec::new()) // vacuously true
        } else {
            SqlCond::Or(Vec::new()) // vacuously false — rendered as 1=0
        });
    }
    let outer_opt_aliases: Vec<usize> = outer.opts.iter().map(|o| o.scan.alias).collect();
    let mut sub_conds = Vec::with_capacity(inner.len());
    for r in &inner {
        if r.path.is_some() {
            return Err(Error::Unsupported(
                "EXISTS with a property-path inner is deferred → 501 (v1)".to_owned(),
            ));
        }
        let mut corr = r.where_conds.clone();
        let mut never_compatible = false;
        let mut shared_var_found = false;
        for (v, ldef) in &outer.bindings {
            let Some(rdef) = r.bindings.get(v) else {
                continue; // not shared
            };
            shared_var_found = true;
            if def_reads_opt_alias(ldef, &outer_opt_aliases) {
                return Err(Error::Unsupported(format!(
                    "EXISTS shared variable ?{v} may be UNBOUND on the outer side (OPTIONAL) → 501 \
                     (v1 supports non-OPTIONAL shared variables)"
                )));
            }
            match unify(ldef, rdef) {
                Unify::Sat(conds) => corr.extend(conds),
                Unify::Empty => {
                    never_compatible = true;
                    break;
                }
                Unify::Unsupported(why) => return Err(Error::Unsupported(why)),
            }
        }
        if never_compatible {
            continue; // this branch can never match the outer row
        }
        // SPARQL §8.3: if outer and inner have disjoint variable domains, MINUS is a
        // NO-OP for this (outer, inner) pair — the inner branch can never remove the
        // left row. Skip it (mirrors flat `minus_branches` line: `if shared.is_empty() { continue }`).
        if negated && !shared_var_found {
            continue;
        }
        if negated {
            sub_conds.push(SqlCond::NotExists {
                scans: r.core.clone(),
                conds: corr,
            });
        } else {
            sub_conds.push(SqlCond::Exists {
                scans: r.core.clone(),
                conds: corr,
            });
        }
    }
    Ok(if negated {
        SqlCond::And(sub_conds) // all branches must fail to match
    } else {
        SqlCond::Or(sub_conds) // at least one branch must match
    })
}

/// Whether a term def reads any of the given OPTIONAL scan aliases — its value may be
/// UNBOUND (the trigger to defer an EXISTS shared variable → 501, flat parity).
fn def_reads_opt_alias(def: &TermDef, opt_aliases: &[usize]) -> bool {
    def.columns().iter().any(|c| opt_aliases.contains(&c.alias))
}

/// The OPTS-FREE form of `left OPT right` — the ISWC-2018 `(P⋈R)∪(P−R)` decomposition,
/// used when this `LeftJoin` is itself the RIGHT operand of an enclosing `LeftJoin` and so
/// must yield re-feedable opts-free branches (§5.3 nested-right closure). It mirrors the
/// multi-branch arm of [`left_join_branches`] but NEVER takes the single-scan `OptJoin`
/// shortcut (which would leave `opts` set), reusing the proven [`inner_join_one`] /
/// [`not_exists_cond_for`] helpers verbatim. `right` is opts-free (it was lowered with the
/// `decompose` flag); a `right` branch still carrying `opts` is a genuine SubPlan shape
/// (e.g. an OPTIONAL the decomposition could not flatten) — lowered via a derived-table
/// LEFT JOIN (ADR-0023 M5 Wave 2).
fn left_join_decomposed(
    left: Vec<Branch>,
    right: Vec<Branch>,
    expr: Option<&Expression>,
    dialect: sf_sql::Dialect,
) -> Result<Vec<Branch>> {
    if right.is_empty() {
        return Ok(left); // OPTIONAL {} = identity
    }
    // If any right branch still has opts (not fully decomposable), lower the right side
    // as a SubPlan derived-table LEFT JOIN (ADR-0023 M5 Wave 2: LeftJoinJoinLimit case).
    if right.iter().any(|r| !r.opts.is_empty()) {
        return left_join_as_subplan(left, right, expr, dialect);
    }
    // (P ⋈ Ri) for each right branch, plus one no-match branch (P − R): NOT EXISTS Ri
    // for every Ri that can possibly match. Identical to `left_join_branches`' multi
    // arm, so the opts-free output is `=_bag` to it.
    let mut out = Vec::new();
    for l in &left {
        for r in &right {
            if let Some(b) = inner_join_one(l, r, expr, dialect)? {
                out.push(b);
            }
        }
        let mut no_match = l.clone();
        for r in &right {
            if let Some(cond) = not_exists_cond_for(l, r)? {
                no_match.where_conds.push(cond);
            }
        }
        out.push(no_match);
    }
    Ok(out)
}

/// Reconstruct the single OPTIONAL ON-`Expression` from a `LeftJoin.cond`
/// (`Vec<IqCond>`) for [`left_join_branches`] (design §5.3). BUILD split the original
/// `&&` into conjuncts ([`crate::build`]); re-AND them (AND is associative, so `=_bag` is
/// preserved). An `Sql`/`Exists`/`NotExists` ON-leaf cannot be expressed as a pushable
/// `Expression` (the flat path likewise 501s an EXISTS-in-OPTIONAL-FILTER via
/// `filter_cond`) → a sound 501.
fn iqconds_to_expr(conds: &[IqCond]) -> Result<Option<Expression>> {
    let mut acc: Option<Expression> = None;
    for c in conds {
        let e = iqcond_to_expr(c)?;
        acc = Some(match acc {
            None => e,
            Some(a) => Expression::And(Box::new(a), Box::new(e)),
        });
    }
    Ok(acc)
}

/// One [`IqCond`] → a pushable [`Expression`] (the inverse of BUILD's
/// `lower_filter_to_iqconds`). `Sql`/`Exists`/`NotExists` have no `Expression` form → 501.
fn iqcond_to_expr(c: &IqCond) -> Result<Expression> {
    match c {
        IqCond::Expr(e) => Ok((**e).clone()),
        IqCond::Not(c) => Ok(Expression::Not(Box::new(iqcond_to_expr(c)?))),
        IqCond::And(cs) => fold_expr(cs, |a, b| Expression::And(Box::new(a), Box::new(b))),
        IqCond::Or(cs) => fold_expr(cs, |a, b| Expression::Or(Box::new(a), Box::new(b))),
        IqCond::Sql(_) | IqCond::Exists(_) | IqCond::NotExists(_) => Err(Error::Unsupported(
            "OPTIONAL ON-condition with a resolved/EXISTS leaf cannot be reconstructed \
             to a pushable FILTER expression → 501"
                .to_owned(),
        )),
    }
}

/// Fold a non-empty `[IqCond]` into one [`Expression`] with `combine`.
fn fold_expr(
    cs: &[IqCond],
    combine: impl Fn(Expression, Expression) -> Expression,
) -> Result<Expression> {
    let mut acc: Option<Expression> = None;
    for c in cs {
        let e = iqcond_to_expr(c)?;
        acc = Some(match acc {
            None => e,
            Some(a) => combine(a, e),
        });
    }
    acc.ok_or_else(|| Error::Unsupported("empty boolean group in OPTIONAL ON-condition".to_owned()))
}

// ── §5.4 SubPlan derived-table helpers (ADR-0023 M5 Wave 2) ─────────────────────────

/// Lower a modifier node (`Aggregation`/`Distinct`/`Slice`/`OrderBy` appearing as a JOIN
/// or FILTER operand — NOT the spine) as a SubPlan derived table. The nested `Plan` is
/// emitted as `(SELECT …) AS t{sp_alias}` and joined via `INNER JOIN … ON 1 = 1` in the
/// parent branch. Each projected SPARQL variable `v` at position `i` in the inner plan is
/// exposed as `t{sp_alias}.c{i}` — the outer branch's [`TermDef`] is the inner's remapped
/// to reference `c{i}` on `sp_alias`.
fn lower_as_subplan(
    node: IqNode,
    dialect: sf_sql::Dialect,
    next_alias: &mut usize,
) -> Result<Vec<Branch>> {
    let nested_plan = lower(node, dialect)?;
    let vars = match &nested_plan.form {
        crate::PlanForm::Select { vars } => vars.clone(),
        _ => {
            return Err(Error::Unsupported(
                "SubPlan: non-SELECT inner plan → 501".to_owned(),
            ))
        }
    };
    if vars.is_empty() {
        return Ok(Vec::new());
    }
    let prepared = nested_plan.prepared_branches();
    if prepared.len() != 1 {
        return Err(Error::Unsupported(
            "SubPlan: multi-branch inner plan not yet supported → 501 (M5 Wave 2 scope)".to_owned(),
        ));
    }
    let inner_branch = &prepared[0];
    // Use the ACTUAL emission projection (the order `emit_branch` assigns to c0, c1, …)
    // rather than `inner_branch.projection()` (BTreeMap / binding-insertion order):
    // for agg branches the emitter places GROUP BY key columns before aggregate
    // columns, which differs from the BTreeMap alphabetical order of `bindings`.
    // Mismatching these would remap `?s` to the wrong positional column.
    let inner_projection = crate::emit::emit_branch(inner_branch, dialect)
        .map_err(|e| Error::Sql(format!("SubPlan inner emit for remapping: {e}")))?
        .projection;
    let sp_alias = *next_alias;
    *next_alias += 1;
    // Build outer bindings: each projected var remapped to ColRef(sp_alias, "c{i}").
    let mut outer_bindings = std::collections::BTreeMap::new();
    for (i, v) in vars.iter().enumerate() {
        if let Some(def) = inner_branch.bindings.get(v.as_str()) {
            match remap_termdef(def, &inner_projection, sp_alias) {
                Ok(remapped) => {
                    outer_bindings.insert(v.clone(), remapped);
                }
                Err(_) => {
                    // Remap failed: fall back to a positional Column TermDef (safe for
                    // reconstruction when the inner emits a single column at c{i}).
                    outer_bindings.insert(
                        v.clone(),
                        TermDef::Derived {
                            term_map: sf_core::ir::TermMap::Column(
                                format!("c{i}").into(),
                                sf_core::ir::TermSpec::plain_literal(),
                            ),
                            alias: sp_alias,
                        },
                    );
                }
            }
        } else {
            // Variable not in inner bindings: expose positionally.
            outer_bindings.insert(
                v.clone(),
                TermDef::Derived {
                    term_map: sf_core::ir::TermMap::Column(
                        format!("c{i}").into(),
                        sf_core::ir::TermSpec::plain_literal(),
                    ),
                    alias: sp_alias,
                },
            );
        }
    }
    let mut outer = Branch::empty();
    outer.subplan_joins.push(SubPlanJoin {
        alias: sp_alias,
        plan: Box::new(nested_plan),
        on: Vec::new(),
        left: false,
    });
    outer.bindings = outer_bindings;
    Ok(vec![outer])
}

/// Lower a `LeftJoin` whose right branches still carry `opts` (the
/// `LeftJoinJoinLimit` case: an OPTIONAL whose right side cannot be fully opts-freed)
/// as a SubPlan derived-table LEFT JOIN (ADR-0023 M5 Wave 2). Re-lowering the right side
/// to a `Plan` and embedding it as a LEFT JOIN SubPlan avoids the multi-branch decomposition
/// that would require opts-free right branches (which we cannot guarantee here).
fn left_join_as_subplan(
    left: Vec<Branch>,
    right: Vec<Branch>,
    expr: Option<&spargebra::algebra::Expression>,
    dialect: sf_sql::Dialect,
) -> Result<Vec<Branch>> {
    // Sanity: right branches carrying opts means the decomposed form is unavailable.
    // We lower the right side as a SubPlan — but since we already have the lowered right
    // branches (not the original IqNode), we cannot re-lower them. For now, the right
    // branch set is single or we 501.
    if right.len() != 1 {
        return Err(Error::Unsupported(
            "LeftJoinJoinLimit: multi-branch right-side SubPlan not yet supported → 501 (M5 Wave 2 scope)"
                .to_owned(),
        ));
    }
    let r = right.into_iter().next().expect("len checked == 1");
    if !r.opts.is_empty() {
        return Err(Error::Unsupported(
            "LeftJoinJoinLimit: opts-carrying right branch → SubPlan LEFT JOIN not yet implemented → 501"
                .to_owned(),
        ));
    }
    // Treat `r` as the right side and join LEFT style; left branches remain.
    // Since `r.opts` is empty here (opts-carrying was handled above), simply return the
    // decomposed form as a fallback (inner join + NOT EXISTS) — the opts-free decomposition
    // path covers this case when `r.opts.is_empty()`.
    let mut out = Vec::new();
    for l in &left {
        if let Some(b) = crate::leftjoin::inner_join_one(l, &r, expr, dialect)? {
            out.push(b);
        }
        let mut no_match = l.clone();
        if let Some(cond) = crate::leftjoin::not_exists_cond_for(l, &r)? {
            no_match.where_conds.push(cond);
        }
        out.push(no_match);
    }
    Ok(out)
}

/// Remap a [`ColRef`] from the inner scan space to the SubPlan's positional column space.
/// Looks up `c` in `projection` (the inner branch's [`Branch::projection()`] output) and
/// returns `ColRef(sp_alias, "c{pos}")`.
fn remap_colref(c: &ColRef, projection: &[ColRef], sp_alias: usize) -> Result<ColRef> {
    let pos = projection.iter().position(|p| p == c).ok_or_else(|| {
        Error::Unsupported(format!(
            "SubPlan remap: ColRef {:?} not in inner projection → 501",
            c
        ))
    })?;
    Ok(ColRef::new(sp_alias, format!("c{pos}")))
}

/// Remap a [`TermDef`] from the inner scan space to the SubPlan's positional column space.
/// All [`ColRef`]s are replaced with `ColRef(sp_alias, "c{pos}")` via [`remap_colref`].
fn remap_termdef(def: &TermDef, projection: &[ColRef], sp_alias: usize) -> Result<TermDef> {
    match def {
        TermDef::Const(t) => Ok(TermDef::Const(t.clone())),
        TermDef::Derived {
            term_map,
            alias: inner_alias,
        } => {
            let new_tm = remap_term_map(term_map, *inner_alias, projection)?;
            Ok(TermDef::Derived {
                term_map: new_tm,
                alias: sp_alias,
            })
        }
        TermDef::Coalesce(l, r) => Ok(TermDef::Coalesce(
            Box::new(remap_termdef(l, projection, sp_alias)?),
            Box::new(remap_termdef(r, projection, sp_alias)?),
        )),
        TermDef::Concat(parts) => Ok(TermDef::Concat(
            parts
                .iter()
                .map(|p| remap_termdef(p, projection, sp_alias))
                .collect::<Result<_>>()?,
        )),
        TermDef::Agg {
            col,
            kind,
            operand,
            fixed_type,
        } => {
            let new_col = remap_colref(col, projection, sp_alias)?;
            let new_operand = operand
                .as_ref()
                .map(|o| remap_colref(o, projection, sp_alias))
                .transpose()?;
            Ok(TermDef::Agg {
                col: new_col,
                kind: *kind,
                operand: new_operand,
                fixed_type: *fixed_type,
            })
        }
    }
}

/// Remap a [`TermMap`] from the inner scan `inner_alias` to the SubPlan's column names.
/// `TermMap::Column(col_name, spec)` → find `ColRef(inner_alias, col_name)` in
/// `projection`, emit `TermMap::Column("c{pos}", spec.clone())`.
/// `TermMap::Template` segments get the same column-name substitution.
fn remap_term_map(
    term_map: &sf_core::ir::TermMap,
    inner_alias: usize,
    projection: &[ColRef],
) -> Result<sf_core::ir::TermMap> {
    use sf_core::ir::{Segment, Template, TermMap};
    match term_map {
        TermMap::Constant(t) => Ok(TermMap::Constant(t.clone())),
        TermMap::Column(col_name, spec) => {
            let pos = projection
                .iter()
                .position(|c| c.alias == inner_alias && c.column.as_ref() == col_name.as_ref())
                .ok_or_else(|| {
                    Error::Unsupported(format!(
                        "SubPlan remap: column '{}' on alias {} not in inner projection → 501",
                        col_name, inner_alias
                    ))
                })?;
            Ok(TermMap::Column(format!("c{pos}").into(), spec.clone()))
        }
        TermMap::Template(tmpl, spec) => {
            let new_segments = tmpl
                .segments()
                .iter()
                .map(|seg| {
                    Ok(match seg {
                        Segment::Literal(s) => Segment::Literal(s.clone()),
                        Segment::Column(col_name) => {
                            let pos = projection
                                .iter()
                                .position(|c| {
                                    c.alias == inner_alias
                                        && c.column.as_ref() == col_name.as_ref()
                                })
                                .ok_or_else(|| {
                                    Error::Unsupported(format!(
                                        "SubPlan remap: template column '{}' on alias {} not in projection → 501",
                                        col_name, inner_alias
                                    ))
                                })?;
                            Segment::Column(format!("c{pos}").into())
                        }
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let new_tmpl =
                Template::from_segments(new_segments).map_err(|e| Error::Sql(e.to_string()))?;
            Ok(TermMap::Template(new_tmpl, spec.clone()))
        }
    }
}

/// Lower an `Aggregation` (SPARQL §11) by child branch count (design §5 Aggregation):
/// a single-branch inner ⇒ a SQL `GROUP BY` on one [`Branch::agg`]; a multi-branch
/// (UNION/VALUES) inner ⇒ a Rust-level [`Plan::rust_group`]. Same IR scope; only the
/// strategy differs — exactly the flat [`crate::unfold::Unfolder::group`] dispatch.
fn lower_aggregation(
    child: IqNode,
    grouping: Vec<Var>,
    aggs: Vec<AggDef>,
    dialect: sf_sql::Dialect,
    spine: &mut Spine,
    next_alias: &mut usize,
) -> Result<Vec<Branch>> {
    let inner = lower_node(child, dialect, false, next_alias)?;
    spine.project.get_or_insert_with(|| {
        let mut out = grouping.clone();
        for a in &aggs {
            if !out.iter().any(|v| v.as_ref() == a.var.as_ref()) {
                out.push(a.var.clone());
            }
        }
        out
    });

    if inner.len() == 1 {
        let mut branch = inner.into_iter().next().expect("len checked == 1");
        if branch.path.is_some() {
            return Err(Error::Unsupported(
                "GROUP BY over a property-path closure is deferred → 501".to_owned(),
            ));
        }
        if branch.agg.is_some() {
            return Err(Error::Unsupported(
                "nested GROUP BY (aggregate over an aggregate) is deferred → 501".to_owned(),
            ));
        }
        // The grouping keys, lowered to their raw key columns (reusing the flat
        // `group_key_columns`); each stays in scope, rebuilt from its grouped columns.
        let mut keys = Vec::with_capacity(grouping.len());
        let mut out_bindings: BTreeMap<String, TermDef> = BTreeMap::new();
        for v in &grouping {
            let def = branch.bindings.get(v.as_ref()).ok_or_else(|| {
                Error::Unsupported(format!(
                    "GROUP BY ?{v} is not a bound variable in the group's inner → 501"
                ))
            })?;
            let cols = group_key_columns(def, v)?;
            out_bindings.insert(v.to_string(), def.clone());
            keys.push(GroupKey {
                var: v.to_string(),
                cols,
            });
        }
        // The aggregate result columns share one reserved synthetic alias (computed in
        // SQL, never read from a base scan).
        let agg_alias = branch_next_alias(&branch);
        let mut agg_cols = Vec::with_capacity(aggs.len());
        for def in &aggs {
            let (kind, arg, distinct, fixed_type) = lower_agg_col(def, &branch.bindings)?;
            let out = ColRef::new(agg_alias, &*def.var);
            out_bindings.insert(
                def.var.to_string(),
                TermDef::Agg {
                    col: out.clone(),
                    kind,
                    operand: arg.clone(),
                    fixed_type,
                },
            );
            agg_cols.push(AggCol {
                var: def.var.to_string(),
                kind,
                arg,
                distinct,
                out,
                fixed_type,
            });
        }
        branch.bindings = out_bindings;
        branch.agg = Some(Aggregation {
            keys,
            aggs: agg_cols,
        });
        Ok(vec![branch])
    } else if let Some(branch) =
        try_sql_group_over_union(&inner, &grouping, &aggs, dialect, next_alias)
    {
        // SQL pushdown (ADR-0023 optimizer-residue, q9 agg-pushdown wave): the union
        // arms pool into ONE derived-table `UNION ALL` and the DB does the GROUP BY —
        // no `RustGroup` buffer-and-group. Falls through to the Rust path below when
        // `try_sql_group_over_union` returns `None` (not provably `=_bag`-safe to pool).
        Ok(vec![branch])
    } else {
        // Multi-branch inner: buffer + group in Rust (design §5 Aggregation). This
        // stays the correctness oracle/fallback for every shape the SQL pushdown
        // above declines (cross-arm type mismatch, multi-column keys/args, etc.).
        let keys: Vec<String> = grouping.iter().map(|v| v.to_string()).collect();
        let mut rust_aggs = Vec::with_capacity(aggs.len());
        for def in &aggs {
            rust_aggs.push(lower_rust_agg(def)?);
        }
        spine.rust_group = Some(RustGroup {
            keys,
            aggs: rust_aggs,
        });
        Ok(inner)
    }
}

/// Attempt to push a multi-branch (UNION) GROUP BY + aggregates down to ONE SQL
/// statement — `SELECT <keys>, <aggs> FROM (<arm1> UNION ALL <arm2> ...) sub GROUP BY
/// <keys>` — so the database aggregates instead of `exec_core::rust_group_execute`
/// buffering every inner solution in Rust (ADR-0023 optimizer-residue horizon, the
/// q9 headline case). Reuses the SubPlan derived-table machinery (`SubPlanJoin`,
/// already rendering a multi-branch nested [`Plan`] as `(arm1) UNION ALL (arm2)` via
/// [`crate::emit::emit_subplan_sql`]) plus the existing single-branch [`Aggregation`]
/// SQL emission verbatim — pooling the arms under one `Aggregation` branch needs no
/// new emission code, only this construction.
///
/// Returns `None` (never `Err`) when NOT provably `=_bag`-safe to pool — the caller
/// falls back to [`RustGroup`] (the correctness oracle), so an inapplicable shape is
/// silently as correct as before, never silently wrong.
///
/// **Applicability (the four correctness concerns, each resolved conservatively):**
/// 1. *Cross-arm type unification.* Every grouping-key / aggregate-argument variable
///    must resolve, in EVERY arm, to `TermDef::Derived { term_map: TermMap::Column(_,
///    spec), .. }` — a single raw column (the same shape the single-branch SQL path
///    already requires via `group_key_columns`/`single_column_of`) — AND `spec`
///    (term_type + datatype + language: the R2RML-declared XSD type) must be
///    IDENTICAL across every arm at that position. A `Template`/`Coalesce`/`Concat`/
///    `Const` def, a column-count mismatch, or a declared-type mismatch bails to the
///    Rust path — no live DB schema introspection needed, since the mapping's own
///    declared type is a sound, deterministic proxy the mapping author controls.
/// 2. *Empty-group semantics.* Unchanged from the single-branch path: when `grouping`
///    is empty, `Aggregation.keys` is empty too, so `emit_agg_branch` omits `GROUP BY`
///    entirely — a bare aggregate SELECT over zero rows is ALREADY one row in SQL
///    (COUNT⇒0, SUM/AVG/MIN/MAX⇒NULL), matching SPARQL §11's implicit-group rule with
///    zero new code. (An EXPLICIT grouping key legitimately yields zero result rows
///    over zero input rows in both SPARQL and SQL — no special-casing needed there.)
/// 3. *`COUNT(DISTINCT ?v)` over the union.* Falls out of pooling into one SQL scope:
///    `COUNT(DISTINCT col)` dedupes per GROUP BY group under standard SQL semantics,
///    the same per-group scope `RustAgg.distinct`'s manual dedup targets.
/// 4. *Scope.* This function is additive (`iq/lower.rs` gains a helper, no rewrite);
///    `RustGroup`/`rust_group_execute` are untouched and remain the oracle.
fn try_sql_group_over_union(
    inner: &[Branch],
    grouping: &[Var],
    aggs: &[AggDef],
    dialect: sf_sql::Dialect,
    next_alias: &mut usize,
) -> Option<Branch> {
    use sf_core::ir::{TermMap, TermSpec};
    use std::collections::BTreeSet;

    if inner.len() < 2 || inner.iter().any(|b| b.path.is_some() || b.agg.is_some()) {
        return None;
    }

    fn column_spec(def: &TermDef) -> Option<&TermSpec> {
        match def {
            TermDef::Derived {
                term_map: TermMap::Column(_, spec),
                ..
            } => Some(spec),
            _ => None,
        }
    }
    fn spec_eq(a: &TermSpec, b: &TermSpec) -> bool {
        // `base` (relative-IRI resolution context) is irrelevant to SQL storage-type
        // compatibility — intentionally excluded.
        a.term_type == b.term_type && a.datatype == b.datatype && a.language == b.language
    }

    // The needed variable set: every grouping key + every var-backed aggregate
    // argument (a non-variable aggregate argument, or `COUNT(DISTINCT *)`, bails —
    // the flat/tree Rust-group path 501s on those too, so this is no new deferral).
    let mut needed: BTreeSet<String> = grouping.iter().map(|v| v.to_string()).collect();
    for def in aggs {
        match &def.arg {
            Some(AggArg::Var(v)) => {
                needed.insert(v.to_string());
            }
            Some(AggArg::Expr(_)) => return None,
            None if def.distinct => return None,
            None => {}
        }
    }
    // BTreeSet iteration order (alphabetical by var name) is EXACTLY the order
    // `Branch::projection()`/`emit_branch_with` assign positional `c{i}` aliases in,
    // since a pooled arm's `bindings` (below) contains precisely this var set keyed
    // by the same `BTreeMap<String, _>` ordering — so `sorted_vars[i]` ↔ `c{i}`.
    let sorted_vars: Vec<String> = needed.into_iter().collect();

    // Validate shape + cross-arm type compatibility for every needed var.
    let mut specs: Vec<TermSpec> = Vec::with_capacity(sorted_vars.len());
    for v in &sorted_vars {
        let mut common: Option<&TermSpec> = None;
        for arm in inner {
            let def = arm.bindings.get(v.as_str())?;
            let spec = column_spec(def)?;
            match common {
                None => common = Some(spec),
                Some(c) if spec_eq(c, spec) => {}
                Some(_) => return None, // cross-arm type mismatch — bail to Rust path
            }
        }
        specs.push(common.expect("inner.len() >= 2 checked above").clone());
    }

    let pos = |v: &str| {
        sorted_vars
            .iter()
            .position(|x| x == v)
            .expect("validated above")
    };

    // Pool the arms: each retains its own FROM/WHERE, reduced to ONLY the needed
    // bindings — but `Branch::projection()` ALSO appends every raw column its
    // (non-DISTINCT) `where_conds`/`opts`/`subplan_joins` reference (join-key
    // equalities etc., deduped against the bindings columns) — the SAME mechanism
    // a normal multi-branch bag-union relies on. Those trailing columns are dead
    // weight here (the outer `Aggregation` only ever reads positions `0..sorted_vars
    // .len()`, resolved by `pos()` below) but they DO have to line up 1:1 across
    // arms for `UNION ALL` to be syntactically valid — checked after the fact
    // (equal-length gate) rather than suppressed, since suppressing them would mean
    // `distinct: true`, which would corrupt the pre-aggregation multiset.
    let pooled_arms: Vec<Branch> = inner
        .iter()
        .map(|arm| {
            let bindings = sorted_vars
                .iter()
                .map(|v| (v.clone(), arm.bindings[v.as_str()].clone()))
                .collect();
            Branch {
                bindings,
                distinct: false,
                limit: None,
                offset: 0,
                order: Vec::new(),
                ..arm.clone()
            }
        })
        .collect();
    // Cross-arm column-count parity (the `UNION ALL` syntactic requirement): every
    // arm's `where_conds`/`opts`/`subplan_joins` may contribute a different number of
    // trailing dead columns via `projection()` when the arms' shapes are NOT
    // symmetric (e.g. one arm joins 2 tables, another 3) — bail to the Rust path
    // rather than emit a `UNION ALL` the database rejects (or, worse, one it
    // silently accepts with misaligned trailing columns nothing ever reads).
    let proj_len = pooled_arms.first().map(|b| b.projection().len())?;
    if pooled_arms.iter().any(|b| b.projection().len() != proj_len) {
        return None;
    }

    let sp_alias = *next_alias;
    *next_alias += 1;
    let nested_plan = Plan {
        branches: pooled_arms,
        form: PlanForm::Select {
            vars: sorted_vars.clone(),
        },
        distinct: false,
        limit: None,
        offset: 0,
        order: Vec::new(),
        rust_group: None,
        dialect,
    };

    let mut outer = Branch::empty();
    outer.subplan_joins.push(SubPlanJoin {
        alias: sp_alias,
        plan: Box::new(nested_plan),
        on: Vec::new(),
        left: false,
    });

    let mut out_bindings: BTreeMap<String, TermDef> = BTreeMap::new();
    let mut keys = Vec::with_capacity(grouping.len());
    for v in grouping {
        let p = pos(v.as_ref());
        let col = ColRef::new(sp_alias, format!("c{p}"));
        out_bindings.insert(
            v.to_string(),
            TermDef::Derived {
                term_map: TermMap::Column(format!("c{p}").into(), specs[p].clone()),
                alias: sp_alias,
            },
        );
        keys.push(GroupKey {
            var: v.to_string(),
            cols: vec![col],
        });
    }

    let agg_alias = *next_alias;
    *next_alias += 1;
    let mut agg_cols = Vec::with_capacity(aggs.len());
    for def in aggs {
        let arg = match &def.arg {
            None => None,
            Some(AggArg::Var(v)) => Some(ColRef::new(sp_alias, format!("c{}", pos(v.as_ref())))),
            Some(AggArg::Expr(_)) => unreachable!("filtered above"),
        };
        let out = ColRef::new(agg_alias, &*def.var);
        out_bindings.insert(
            def.var.to_string(),
            TermDef::Agg {
                col: out.clone(),
                kind: def.kind,
                operand: arg.clone(),
                fixed_type: def.fixed_type,
            },
        );
        agg_cols.push(AggCol {
            var: def.var.to_string(),
            kind: def.kind,
            arg,
            distinct: def.distinct,
            out,
            fixed_type: def.fixed_type,
        });
    }

    outer.bindings = out_bindings;
    outer.agg = Some(Aggregation {
        keys,
        aggs: agg_cols,
    });
    Some(outer)
}

/// Map one [`AggDef`] to a single-branch SQL [`AggCol`] tuple `(kind, arg col, distinct,
/// fixed type)` (reusing the flat `single_column_of` for the argument column). Mirrors the
/// flat `lower_aggregate` deferrals: `COUNT(DISTINCT *)` and a non-variable aggregate
/// argument are sound 501s.
fn lower_agg_col(
    def: &AggDef,
    bindings: &BTreeMap<String, TermDef>,
) -> Result<(
    crate::iq::AggKind,
    Option<ColRef>,
    bool,
    Option<sf_core::datatype::XsdTypeCode>,
)> {
    match &def.arg {
        None => {
            if def.distinct {
                return Err(Error::Unsupported(
                    "COUNT(DISTINCT *) is deferred → 501 (v1 supports COUNT(*))".to_owned(),
                ));
            }
            Ok((def.kind, None, false, def.fixed_type))
        }
        Some(AggArg::Var(v)) => {
            let bdef = bindings.get(v.as_ref()).ok_or_else(|| {
                Error::Unsupported(format!(
                    "aggregate variable ?{v} is not bound in the group's inner → 501"
                ))
            })?;
            let col = single_column_of(bdef, v)?;
            Ok((def.kind, Some(col), def.distinct, def.fixed_type))
        }
        Some(AggArg::Expr(_)) => Err(Error::Unsupported(
            "aggregate over a non-variable expression is deferred → 501 \
             (v1 aggregates a single column-backed variable)"
                .to_owned(),
        )),
    }
}

/// Map one [`AggDef`] to a multi-branch Rust-group [`RustAgg`] (mirrors the flat
/// `parse_rust_agg`): the argument stays a variable name; `COUNT(DISTINCT *)` and a
/// non-variable argument are sound 501s.
fn lower_rust_agg(def: &AggDef) -> Result<RustAgg> {
    let arg_var = match &def.arg {
        None => {
            if def.distinct {
                return Err(Error::Unsupported(
                    "COUNT(DISTINCT *) is deferred → 501 (v1 supports COUNT(*))".to_owned(),
                ));
            }
            None
        }
        Some(AggArg::Var(v)) => Some(v.to_string()),
        Some(AggArg::Expr(_)) => {
            return Err(Error::Unsupported(
                "aggregate over a non-variable expression is deferred → 501 \
                 (v1 aggregates a single column-backed variable)"
                    .to_owned(),
            ))
        }
    };
    Ok(RustAgg {
        out_var: def.var.to_string(),
        kind: def.kind,
        arg_var,
        distinct: def.distinct,
        fixed_type: def.fixed_type,
    })
}

/// Rewrite a multi-branch [`RustGroup`]'s output variable names per the outer
/// `Construction`'s substitution — the post-GROUP-BY `(agg AS ?v)` Extend (design §4.14).
/// In SPARQL algebra `SELECT (COUNT(?x) AS ?c)` is a `Group` producing an INTERNAL
/// aggregate variable, then an `Extend` `?c := ?internal`. For the SQL `GROUP BY`
/// (single-branch) path that Extend folds into the branch bindings; for the Rust-group
/// (UNION/VALUES) path the aggregate has no branch column, so the Extend instead RENAMES
/// the `RustAgg::out_var` from the internal var to the SELECT var. Each subst entry must
/// be such a bare-variable rename of an aggregate output; anything else (arithmetic over
/// an aggregate, a group-key rename) is a tracked sound-501 (never silently dropped).
fn rename_rust_group_outputs(subst: &BTreeMap<Var, BindDef>, rg: &mut RustGroup) -> Result<()> {
    for (out_var, def) in subst {
        let BindDef::Expr(e) = def else {
            return Err(Error::Unsupported(
                "post-GROUP-BY substitution over a UNION aggregate must be a bare-variable \
                 rename → 501"
                    .to_owned(),
            ));
        };
        let Expression::Variable(inner) = e.as_ref() else {
            return Err(Error::Unsupported(format!(
                "post-GROUP-BY expression over a UNION aggregate is deferred → 501: {e:?}"
            )));
        };
        if let Some(agg) = rg.aggs.iter_mut().find(|a| a.out_var == inner.as_str()) {
            agg.out_var = out_var.to_string();
        } else {
            return Err(Error::Unsupported(format!(
                "post-GROUP-BY substitution references ?{inner}, not a UNION aggregate output \
                 → 501"
            )));
        }
    }
    Ok(())
}

/// A fresh scan alias for the aggregate result columns: one past the max alias used
/// anywhere in `b` (the flat `group` draws this from the `Unfolder` counter; here we
/// derive it from the branch so the synthetic alias never collides with a base scan).
fn branch_next_alias(b: &Branch) -> usize {
    let mut aliases: Vec<usize> = Vec::new();
    for (a, _) in b.alias_sources() {
        aliases.push(a);
    }
    for def in b.bindings.values() {
        for c in def.columns() {
            aliases.push(c.alias);
        }
    }
    for cond in &b.where_conds {
        crate::iq::collect_cond_cols(cond, &mut |c| aliases.push(c.alias));
    }
    for opt in &b.opts {
        for cond in opt.on.iter().chain(opt.extra.iter()) {
            crate::iq::collect_cond_cols(cond, &mut |c| aliases.push(c.alias));
        }
    }
    aliases.into_iter().max().map_or(0, |m| m + 1)
}

/// All variables bound anywhere in `branches` (the `SELECT *` fallback projection), in a
/// deterministic order — mirrors the flat `crate::visible_vars`.
fn visible_vars(branches: &[Branch]) -> Vec<String> {
    let mut set = std::collections::BTreeSet::new();
    for b in branches {
        for v in b.bindings.keys() {
            set.insert(v.clone());
        }
    }
    set.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::build_tree;
    use crate::iq::node::IqNode;
    use crate::iq::resolve::{resolve, ResolveCx};
    use crate::saturate::Tbox;
    use sf_core::ir::{
        LogicalSource, ObjectMap, PredicateObjectMap, RefObjectMap, SubjectMap, Template, TermMap,
        TermSpec, TriplesMap,
    };
    use sf_core::NamedNode;
    use spargebra::algebra::GraphPattern;

    fn iri(s: &str) -> NamedNode {
        NamedNode::new(s).unwrap()
    }

    fn template_iri(t: &str) -> TermMap {
        TermMap::Template(Template::parse(t).unwrap(), TermSpec::iri())
    }

    fn column_literal(c: &str) -> TermMap {
        TermMap::Column(c.into(), TermSpec::plain_literal())
    }

    fn pom(predicate: &str, object: ObjectMap) -> PredicateObjectMap {
        PredicateObjectMap {
            predicates: vec![TermMap::Constant(iri(predicate).into())],
            objects: vec![object],
            graphs: vec![],
        }
    }

    /// EMP(id,name,dept_id) + DEPT(id,dname); EMP :name (column) and EMP :dept
    /// (refObjectMap → DEPT) — the same fixture as resolve.rs / normalize.rs.
    fn mapping() -> Vec<TriplesMap> {
        let emp = TriplesMap {
            id: "EMP".to_owned(),
            source: LogicalSource::Table("emp".to_owned()),
            subject: SubjectMap {
                term: template_iri("http://ex/emp/{id}"),
                classes: vec![iri("http://ex/Employee")],
                graphs: vec![],
            },
            predicate_object_maps: vec![
                pom("http://ex/name", ObjectMap::Term(column_literal("name"))),
                pom(
                    "http://ex/dept",
                    ObjectMap::Ref(RefObjectMap {
                        parent_triples_map: "DEPT".to_owned(),
                        joins: vec![sf_core::ir::Join {
                            child: "dept_id".to_owned(),
                            parent: "id".to_owned(),
                        }],
                    }),
                ),
            ],
        };
        let dept = TriplesMap {
            id: "DEPT".to_owned(),
            source: LogicalSource::Table("dept".to_owned()),
            subject: SubjectMap {
                term: template_iri("http://ex/dept/{id}"),
                classes: vec![iri("http://ex/Department")],
                graphs: vec![],
            },
            predicate_object_maps: vec![pom(
                "http://ex/dname",
                ObjectMap::Term(column_literal("dname")),
            )],
        };
        vec![emp, dept]
    }

    fn pattern(q: &str) -> GraphPattern {
        match spargebra::SparqlParser::new().parse_query(q).unwrap() {
            spargebra::Query::Select { pattern, .. } => pattern,
            other => panic!("expected SELECT, got {other:?}"),
        }
    }

    /// build → resolve → normalize → lower a query against the fixture mapping.
    fn plan(q: &str) -> Plan {
        let maps = mapping();
        let tbox = Tbox::new();
        let mut cx = ResolveCx::new(&maps, &tbox, sf_sql::Dialect::Sqlite);
        let resolved = resolve(build_tree(&pattern(q), None).unwrap(), &mut cx).unwrap();
        let normalized = crate::iq::normalize::normalize(resolved).unwrap();
        lower(normalized, sf_sql::Dialect::Sqlite).unwrap()
    }

    /// build → resolve → normalize → lower, returning the lower `Result` (for 501 cases).
    fn try_plan(q: &str) -> Result<Plan> {
        let maps = mapping();
        let tbox = Tbox::new();
        let mut cx = ResolveCx::new(&maps, &tbox, sf_sql::Dialect::Sqlite);
        let resolved = resolve(build_tree(&pattern(q), None).unwrap(), &mut cx).unwrap();
        let normalized = crate::iq::normalize::normalize(resolved).unwrap();
        lower(normalized, sf_sql::Dialect::Sqlite)
    }

    fn has_col_eq(conds: &[SqlCond]) -> bool {
        conds.iter().any(|c| matches!(c, SqlCond::ColEq(..)))
    }

    fn has_not_exists(conds: &[SqlCond]) -> bool {
        conds.iter().any(|c| matches!(c, SqlCond::NotExists { .. }))
    }

    /// A single triple pattern → ONE [`Branch`] over a single scan, carrying both vars.
    #[test]
    fn single_triple_is_one_scan_branch() {
        let p = plan("SELECT * WHERE { ?s <http://ex/name> ?n }");
        assert_eq!(p.branches.len(), 1, "{:?}", p.branches);
        let b = &p.branches[0];
        assert_eq!(b.core.len(), 1, "one base scan: {:?}", b.core);
        assert!(b.opts.is_empty());
        assert!(
            b.bindings.contains_key("s") && b.bindings.contains_key("n"),
            "carries ?s and ?n: {:?}",
            b.bindings
        );
    }

    /// A 2-triple BGP join → ONE branch whose shared-var equality is a `ColEq` in
    /// `where_conds` (the flat `merge`/`unify` equality, carried via the InnerJoin cond).
    #[test]
    fn bgp_join_has_col_eq_in_where_conds() {
        let p = plan("SELECT * WHERE { ?s <http://ex/name> ?n . ?s <http://ex/name> ?m }");
        assert_eq!(p.branches.len(), 1, "{:?}", p.branches);
        let b = &p.branches[0];
        assert_eq!(b.core.len(), 2, "two EMP scans: {:?}", b.core);
        assert!(
            has_col_eq(&b.where_conds),
            "shared ?s ⇒ ColEq: {:?}",
            b.where_conds
        );
    }

    /// A refObjectMap triple → a 2-scan branch with the join `ColEq` (design §3.4 / §5).
    #[test]
    fn ref_object_map_is_two_scan_branch() {
        let p = plan("SELECT * WHERE { ?s <http://ex/dept> ?d }");
        assert_eq!(p.branches.len(), 1);
        let b = &p.branches[0];
        assert_eq!(b.core.len(), 2, "child ⋈ parent scan: {:?}", b.core);
        assert!(has_col_eq(&b.where_conds), "{:?}", b.where_conds);
    }

    /// OPTIONAL with a single-scan right → ONE branch with an `OptJoin` (the SQL LEFT
    /// JOIN via `build_left_join`), NOT a decomposition (§5.3).
    #[test]
    fn optional_single_scan_is_opt_join() {
        let p =
            plan("SELECT * WHERE { ?s <http://ex/name> ?n OPTIONAL { ?s <http://ex/name> ?m } }");
        assert_eq!(
            p.branches.len(),
            1,
            "single OptJoin branch: {:?}",
            p.branches
        );
        let b = &p.branches[0];
        assert_eq!(b.opts.len(), 1, "one OptJoin: {:?}", b.opts);
    }

    /// OPTIONAL with a multi-arm Union right → the ISWC-2018 `(P⋈R)∪(P−R)` decomposition:
    /// several branches plus exactly ONE no-match branch carrying `NOT EXISTS` (§5.3).
    #[test]
    fn optional_multi_branch_right_decomposes() {
        let p = plan("SELECT * WHERE { ?s <http://ex/name> ?n OPTIONAL { ?s ?p ?o } }");
        assert!(
            p.branches.len() >= 2,
            "(P⋈R)∪(P−R) ⇒ ≥2 branches: {:?}",
            p.branches
        );
        let no_match = p
            .branches
            .iter()
            .filter(|b| has_not_exists(&b.where_conds))
            .count();
        assert_eq!(no_match, 1, "exactly one no-match (NOT EXISTS) branch");
    }

    /// ADR-0023 M4 wave 3 (§5.3 nested-right closure) — a RIGHT-nested OPTIONAL
    /// `P OPT (Q OPT R)` lowers WITHOUT a 501: the inner OPTIONAL (the outer's right
    /// operand) is forced to its OPTS-FREE `(P⋈R)∪(P−R)` decomposition so it is
    /// re-feedable into the outer `left_join_branches`. Every resulting branch is
    /// opts-free (no `OptJoin` survives in the right) and the bag carries a no-match
    /// (`NOT EXISTS`) branch — the multi-branch decomposition shape.
    #[test]
    fn right_nested_optional_lowers_opt_free() {
        let r = try_plan(
            "SELECT * WHERE { ?s <http://ex/name> ?n \
             OPTIONAL { ?s <http://ex/dept> ?d . ?d <http://ex/dname> ?dn \
             OPTIONAL { ?s <http://ex/name> ?m } } }",
        );
        let p = r.expect("right-nested OPTIONAL must lower (no 501)");
        assert!(
            p.branches.len() >= 2,
            "(P⋈R)∪(P−R) ⇒ ≥2 branches: {:?}",
            p.branches
        );
        assert!(
            p.branches.iter().all(|b| b.opts.is_empty()),
            "every branch is OPTS-FREE (the right was decomposed, not OptJoin): {:?}",
            p.branches
        );
        assert!(
            p.branches.iter().any(|b| has_not_exists(&b.where_conds)),
            "a no-match (NOT EXISTS) branch is present: {:?}",
            p.branches
        );
    }

    /// UNION → a `Vec<Branch>` bag union; each arm carries ONLY its own bindings, and a
    /// variable an arm does not bind stays ABSENT — never padded (§5.2, R3).
    #[test]
    fn union_arms_keep_own_bindings_unbound_absent() {
        let p =
            plan("SELECT * WHERE { { ?s <http://ex/name> ?n } UNION { ?s <http://ex/dname> ?d } }");
        assert_eq!(p.branches.len(), 2, "two arms: {:?}", p.branches);
        let name_arm = p
            .branches
            .iter()
            .find(|b| b.bindings.contains_key("n"))
            .expect("an arm binding ?n");
        assert!(
            !name_arm.bindings.contains_key("d"),
            "?d is ABSENT (not padded) in the ?n arm: {:?}",
            name_arm.bindings
        );
        let dname_arm = p
            .branches
            .iter()
            .find(|b| b.bindings.contains_key("d"))
            .expect("an arm binding ?d");
        assert!(
            !dname_arm.bindings.contains_key("n"),
            "?n is ABSENT in the ?d arm: {:?}",
            dname_arm.bindings
        );
    }

    /// FILTER → resolved per-branch via the flat `filter_cond` into `where_conds` (R4).
    #[test]
    fn filter_lowers_to_where_cond() {
        let p = plan("SELECT * WHERE { ?s <http://ex/name> ?n FILTER(?n > \"5\") }");
        assert_eq!(p.branches.len(), 1);
        let b = &p.branches[0];
        assert!(
            b.where_conds.iter().any(|c| matches!(c, SqlCond::Cmp(..))),
            "?n > 5 ⇒ a Cmp WHERE cond: {:?}",
            b.where_conds
        );
    }

    /// BIND → resolved per-branch via the flat `bind_term_def` into `bindings` (a CONCAT
    /// term-construction-lifted value, design §5 Construction).
    #[test]
    fn bind_lowers_to_binding() {
        let p = plan("SELECT * WHERE { ?s <http://ex/name> ?n BIND(CONCAT(?n, ?n) AS ?b) }");
        assert_eq!(p.branches.len(), 1);
        let b = &p.branches[0];
        assert!(
            matches!(b.bindings.get("b"), Some(TermDef::Concat(_))),
            "?b := CONCAT(?n,?n) ⇒ TermDef::Concat: {:?}",
            b.bindings.get("b")
        );
    }

    /// GROUP BY over a single-branch inner → a SQL GROUP BY on `Branch.agg` (no rust_group).
    #[test]
    fn group_by_single_branch_uses_branch_agg() {
        let p = plan("SELECT ?s (COUNT(?n) AS ?c) WHERE { ?s <http://ex/name> ?n } GROUP BY ?s");
        assert_eq!(p.branches.len(), 1, "{:?}", p.branches);
        assert!(p.branches[0].agg.is_some(), "Branch.agg set (SQL GROUP BY)");
        assert!(
            p.rust_group.is_none(),
            "no Rust group for a single-branch inner"
        );
        assert!(
            p.branches[0].bindings.contains_key("c"),
            "the aggregate result var is in scope"
        );
    }

    /// GROUP BY over a multi-branch (UNION) inner → a Rust-level `Plan.rust_group`
    /// (design §5 Aggregation). Tested on a hand-built `Aggregation` over a two-arm
    /// `Union` so the agg output var is the user var directly: a query-level *aliased*
    /// aggregate over a multi-branch inner injects a spargebra-internal var + `Extend`
    /// the tree now closes by renaming the `RustGroup` output — see
    /// [`aliased_aggregate_over_multi_branch_closes_via_rust_group`].
    /// The arms here declare INCOMPATIBLE `TermSpec`s for `?o` (plain literal vs a
    /// typed `xsd:integer` literal) — `try_sql_group_over_union`'s concern-#1 gate
    /// (cross-arm type unification) must decline the SQL pushdown for exactly this
    /// reason, so the plan still falls back to `RustGroup`. See
    /// [`group_by_multi_branch_pushes_down_to_sql_when_compatible`] for the
    /// compatible-arms case (the SQL pushdown fires instead).
    #[test]
    fn group_by_multi_branch_uses_rust_group() {
        use crate::iq::node::{AggArg, AggDef, BindDef};
        use crate::iq::{AggKind, Scan};
        use sf_core::ir::{LogicalSource, TermMap, TermSpec};

        let arm = |alias: usize, o_spec: TermSpec| -> IqNode {
            let col = |c: &str, spec: TermSpec| {
                BindDef::Resolved(TermDef::Derived {
                    term_map: TermMap::Column(c.into(), spec),
                    alias,
                })
            };
            let mut subst = BTreeMap::new();
            subst.insert("s".into(), col("id", TermSpec::plain_literal()));
            subst.insert("o".into(), col("v", o_spec));
            IqNode::Construction {
                child: Box::new(IqNode::Extensional {
                    scan: Scan {
                        alias,
                        source: LogicalSource::Table("t".to_owned()),
                    },
                    bind: BTreeMap::new(),
                }),
                subst,
                project: vec!["s".into(), "o".into()],
            }
        };
        let tree = IqNode::Aggregation {
            grouping: vec!["s".into()],
            aggs: vec![AggDef {
                var: "c".into(),
                kind: AggKind::Sum,
                arg: Some(AggArg::Var("o".into())),
                distinct: false,
                fixed_type: None,
            }],
            child: Box::new(IqNode::Union {
                children: vec![
                    arm(0, TermSpec::plain_literal()),
                    arm(
                        1,
                        TermSpec::typed_literal(sf_core::NamedNode::new_unchecked(
                            "http://www.w3.org/2001/XMLSchema#integer",
                        )),
                    ),
                ],
                project: vec!["s".into(), "o".into()],
            }),
        };
        let p = lower(tree, sf_sql::Dialect::Sqlite).unwrap();
        let rg = p.rust_group.expect("multi-branch inner ⇒ Plan.rust_group");
        assert_eq!(rg.keys, vec!["s".to_owned()]);
        assert_eq!(rg.aggs.len(), 1);
        assert_eq!(rg.aggs[0].out_var, "c");
        assert_eq!(rg.aggs[0].arg_var.as_deref(), Some("o"));
        assert_eq!(
            p.branches.len(),
            2,
            "the two inner arms are kept for grouping"
        );
        assert!(
            p.branches.iter().all(|b| b.agg.is_none()),
            "no per-branch SQL GROUP BY"
        );
    }

    /// The pushdown counterpart of [`group_by_multi_branch_uses_rust_group`]: the SAME
    /// shape, but both arms declare the IDENTICAL `?o` `TermSpec` — concern #1's gate
    /// passes, so `try_sql_group_over_union` fires: ONE branch, no `RustGroup`, an
    /// `Aggregation` carrying a `SubPlanJoin` that pools the two arms.
    #[test]
    fn group_by_multi_branch_pushes_down_to_sql_when_compatible() {
        use crate::iq::node::{AggArg, AggDef, BindDef};
        use crate::iq::{AggKind, Scan};
        use sf_core::ir::{LogicalSource, TermMap, TermSpec};

        let arm = |alias: usize| -> IqNode {
            let col = |c: &str| {
                BindDef::Resolved(TermDef::Derived {
                    term_map: TermMap::Column(c.into(), TermSpec::plain_literal()),
                    alias,
                })
            };
            let mut subst = BTreeMap::new();
            subst.insert("s".into(), col("id"));
            subst.insert("o".into(), col("v"));
            IqNode::Construction {
                child: Box::new(IqNode::Extensional {
                    scan: Scan {
                        alias,
                        source: LogicalSource::Table("t".to_owned()),
                    },
                    bind: BTreeMap::new(),
                }),
                subst,
                project: vec!["s".into(), "o".into()],
            }
        };
        let tree = IqNode::Aggregation {
            grouping: vec!["s".into()],
            aggs: vec![AggDef {
                var: "c".into(),
                kind: AggKind::Sum,
                arg: Some(AggArg::Var("o".into())),
                distinct: false,
                fixed_type: None,
            }],
            child: Box::new(IqNode::Union {
                children: vec![arm(0), arm(1)],
                project: vec!["s".into(), "o".into()],
            }),
        };
        let p = lower(tree, sf_sql::Dialect::Sqlite).unwrap();
        assert!(
            p.rust_group.is_none(),
            "compatible arms must push down, not fall back to RustGroup"
        );
        assert_eq!(p.branches.len(), 1, "pooled into one Aggregation branch");
        let b = &p.branches[0];
        let agg = b.agg.as_ref().expect("SQL GROUP BY over the pooled union");
        assert_eq!(agg.keys.len(), 1);
        assert_eq!(agg.keys[0].var, "s");
        assert_eq!(agg.aggs.len(), 1);
        assert_eq!(agg.aggs[0].var, "c");
        assert_eq!(agg.aggs[0].kind, AggKind::Sum);
        assert_eq!(
            b.subplan_joins.len(),
            1,
            "the two arms pool into one SubPlan derived table"
        );
        assert_eq!(b.subplan_joins[0].plan.branches.len(), 2);
    }

    /// ADR-0023 M4 wave 1 — the agg-over-UNION HEADLINE (design §4.14): an *aliased*
    /// aggregate over a multi-branch inner, where SPARQL injects an internal agg var + a
    /// post-Group `Extend(?c := <internal var>)`, now CLOSES in the tree path (the FLAT
    /// oracle still 501s — "BIND references unbound"). The Construction renames the
    /// `RustGroup` output `<internal var>` → `?c` instead of folding into the pre-group
    /// branches, so the plan is a correct multi-branch `rust_group`. (Multiset correctness
    /// vs the independent spareval oracle is gated by `differential_tree::agg_over_union_*`.)
    #[test]
    fn aliased_aggregate_over_multi_branch_closes_via_rust_group() {
        let p = plan("SELECT ?s (COUNT(?o) AS ?c) WHERE { ?s ?p ?o } GROUP BY ?s");
        let rg = p
            .rust_group
            .expect("multi-branch aggregate ⇒ Plan.rust_group");
        assert_eq!(rg.keys, vec!["s".to_owned()]);
        assert_eq!(rg.aggs.len(), 1);
        assert_eq!(rg.aggs[0].out_var, "c", "internal agg var renamed to ?c");
        assert_eq!(rg.aggs[0].arg_var.as_deref(), Some("o"));
        assert!(
            p.branches.iter().all(|b| b.agg.is_none()),
            "no per-branch SQL GROUP BY for a multi-branch inner"
        );
        let PlanForm::Select { vars } = &p.form else {
            panic!("SELECT form");
        };
        assert_eq!(vars, &vec!["s".to_owned(), "c".to_owned()]);
    }

    /// VALUES → core-less `Const` branches, one per row (UNDEF ⇒ absent var).
    #[test]
    fn values_lowers_to_const_branches() {
        let p = plan("SELECT * WHERE { VALUES ?x { \"a\" \"b\" } }");
        assert_eq!(p.branches.len(), 2, "one branch per row: {:?}", p.branches);
        for b in &p.branches {
            assert!(b.core.is_empty(), "VALUES is core-less");
            assert!(matches!(b.bindings.get("x"), Some(TermDef::Const(_))));
        }
    }

    /// LIMIT / OFFSET → `Plan.limit`/`offset` modifiers.
    #[test]
    fn slice_sets_plan_limit_offset() {
        let p = plan("SELECT * WHERE { ?s <http://ex/name> ?n } LIMIT 5 OFFSET 2");
        assert_eq!(p.limit, Some(5));
        assert_eq!(p.offset, 2);
    }

    /// DISTINCT → `Plan.distinct`.
    #[test]
    fn distinct_sets_plan_distinct() {
        let p = plan("SELECT DISTINCT ?s WHERE { ?s <http://ex/name> ?n }");
        assert!(p.distinct);
    }

    /// ORDER BY → `Plan.order`.
    #[test]
    fn order_by_sets_plan_order() {
        let p = plan("SELECT * WHERE { ?s <http://ex/name> ?n } ORDER BY ?n");
        assert_eq!(p.order.len(), 1);
        assert_eq!(p.order[0].var, "n");
    }

    /// §5.4 SubPlan lowering (ADR-0023 M5 Wave 2): a nested Aggregation as a join INPUT
    /// (an aggregate subquery joined with a pattern) → SubPlan derived-table branch.
    #[test]
    fn nested_aggregation_join_input_lowers_to_subplan() {
        let p = plan(
            "SELECT * WHERE { { SELECT ?s (COUNT(?n) AS ?c) WHERE { ?s <http://ex/name> ?n } \
             GROUP BY ?s } ?s <http://ex/name> ?m }",
        );
        // The tree lower must now succeed: at least one branch has a subplan_join entry.
        assert!(
            p.branches.iter().any(|b| !b.subplan_joins.is_empty()),
            "nested aggregation subquery must produce a subplan_join branch: {:?}",
            p.branches
        );
    }

    /// §5.4 SubPlan lowering (ADR-0023 M5 Wave 2): a nested DISTINCT subquery as a join
    /// INPUT → SubPlan derived-table branch.
    #[test]
    fn nested_distinct_join_input_lowers_to_subplan() {
        let p = plan(
            "SELECT * WHERE { { SELECT DISTINCT ?s WHERE { ?s <http://ex/name> ?n } } \
             ?s <http://ex/name> ?m }",
        );
        assert!(
            p.branches.iter().any(|b| !b.subplan_joins.is_empty()),
            "nested DISTINCT subquery must produce a subplan_join branch: {:?}",
            p.branches
        );
    }
}
