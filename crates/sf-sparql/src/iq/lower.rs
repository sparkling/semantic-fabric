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
    AggCol, Aggregation, Branch, ColRef, GroupKey, OrderKey, RustAgg, RustGroup, SqlCond, TermDef,
};
use crate::leftjoin::left_join_branches;
use crate::unfold::{group_key_columns, join_branches, single_column_of};
use crate::unify::{bind_term_def, filter_cond, unify, Unify};
use crate::{Error, Plan, PlanForm, Result};

/// Lower a NORMALIZED tree to a [`Plan`] (design §5). Peels the query-modifier spine
/// (`Distinct`/`Slice`/`OrderBy`) and the `Aggregation` strategy choice onto the plan,
/// then folds the relational body to a bag-union of [`Branch`]es via [`lower_node`].
/// `form` is `SELECT` over the outermost projected scope (the tree models the WHERE
/// pattern + modifiers; CONSTRUCT/ASK form is a `Query`-level concern out of M3c scope).
pub fn lower(node: IqNode, dialect: sf_sql::Dialect) -> Result<Plan> {
    let mut spine = Spine::default();
    let branches = lower_spine(node, dialect, &mut spine)?;
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
fn lower_spine(node: IqNode, dialect: sf_sql::Dialect, spine: &mut Spine) -> Result<Vec<Branch>> {
    match node {
        IqNode::Distinct { child } => {
            spine.distinct = true;
            lower_spine(*child, dialect, spine)
        }
        IqNode::Slice {
            child,
            offset,
            limit,
        } => {
            spine.offset = offset;
            spine.limit = limit;
            lower_spine(*child, dialect, spine)
        }
        IqNode::OrderBy { child, keys } => {
            spine.order = keys;
            lower_spine(*child, dialect, spine)
        }
        IqNode::Aggregation {
            child,
            grouping,
            aggs,
        } => lower_aggregation(*child, grouping, aggs, dialect, spine),
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
            let mut branches = lower_spine(*child, dialect, spine)?;
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
            lower_node(other, dialect)
        }
    }
}

/// Fold a relational subtree to a bag of [`Branch`]es (design §5), bottom-up. A node may
/// yield several branches: a `Union` arm-per-branch, a multi-branch OPTIONAL decomposition.
fn lower_node(node: IqNode, dialect: sf_sql::Dialect) -> Result<Vec<Branch>> {
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
                let cbr = lower_node(child, dialect)?;
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
            let l = lower_node(*left, dialect)?;
            let r = lower_node(*right, dialect)?;
            // The OPTIONAL ON-expression (R5 inner FILTER) is reconstructed to a single
            // `Expression` for `left_join_branches`/`build_left_join`, which lower it
            // against the COMBINED left+right bindings (we MUST NOT change that scope).
            let expr = iqconds_to_expr(&cond)?;
            left_join_branches(l, r, expr.as_ref(), dialect)
        }

        // ---- selection: resolve each cond per resulting branch (R4) ---------------
        IqNode::Filter { child, cond } => {
            let mut branches = lower_node(*child, dialect)?;
            for b in &mut branches {
                apply_conds(&cond, b, dialect)?;
            }
            Ok(branches)
        }

        // ---- bag union (§5.2, R3): one branch per arm, own bindings, absent unbound -
        IqNode::Union { children, .. } => {
            let mut out = Vec::new();
            for c in children {
                out.extend(lower_node(c, dialect)?);
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
            let branches = lower_node(body, dialect)?;
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

        // ---- §5.4 tracked sound-501s (need the §5.1 SubPlan derived table, M5/M7) --
        IqNode::Aggregation { .. } => Err(Error::Unsupported(
            "nested Aggregation as a join/filter input (HAVING / agg-subquery) → 501 \
             (needs the §5.1 SubPlan derived table, M5/M7)"
                .to_owned(),
        )),
        IqNode::Distinct { .. } => Err(Error::Unsupported(
            "nested DISTINCT as a join/filter input (subquery) → 501 \
             (needs the §5.1 SubPlan derived table, M5/M7)"
                .to_owned(),
        )),
        IqNode::Slice { .. } => Err(Error::Unsupported(
            "nested Slice (subquery LIMIT/OFFSET) as a join/filter input → 501 \
             (needs the §5.1 SubPlan derived table, M5/M7)"
                .to_owned(),
        )),
        IqNode::OrderBy { .. } => Err(Error::Unsupported(
            "nested ORDER BY as a join/filter input (subquery) → 501 \
             (needs the §5.1 SubPlan derived table, M5/M7)"
                .to_owned(),
        )),
        IqNode::Intensional { .. } => Err(Error::Unsupported(
            "Intensional survived to LOWER — the RESOLVE invariant (ZERO Intensional) \
             was violated → 501"
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
    let inner = lower_node(node.clone(), dialect)?;
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
        for (v, ldef) in &outer.bindings {
            let Some(rdef) = r.bindings.get(v) else {
                continue; // not shared
            };
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
) -> Result<Vec<Branch>> {
    let inner = lower_node(child, dialect)?;
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
        let agg_alias = next_alias(&branch);
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
    } else {
        // Multi-branch inner: buffer + group in Rust (design §5 Aggregation).
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
fn next_alias(b: &Branch) -> usize {
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
    #[test]
    fn group_by_multi_branch_uses_rust_group() {
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

    /// §5.4 tracked sound-501: a nested Aggregation as a join INPUT (an aggregate
    /// subquery joined with a pattern) → `Err(Unsupported)` AT LOWER (never silent).
    #[test]
    fn nested_aggregation_join_input_is_501() {
        let r = try_plan(
            "SELECT * WHERE { { SELECT ?s (COUNT(?n) AS ?c) WHERE { ?s <http://ex/name> ?n } \
             GROUP BY ?s } ?s <http://ex/name> ?m }",
        );
        assert!(matches!(r, Err(Error::Unsupported(_))), "{r:?}");
    }

    /// §5.4 tracked sound-501: a nested DISTINCT subquery as a join INPUT → 501 at LOWER.
    #[test]
    fn nested_distinct_join_input_is_501() {
        let r = try_plan(
            "SELECT * WHERE { { SELECT DISTINCT ?s WHERE { ?s <http://ex/name> ?n } } \
             ?s <http://ex/name> ?m }",
        );
        assert!(matches!(r, Err(Error::Unsupported(_))), "{r:?}");
    }
}
