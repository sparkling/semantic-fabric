//! Normalize-min — the operator-tree ([`IqNode`]) NORMALIZE stage (ADR-0023 M3b,
//! `docs/design/ADR-0023-M3-resolution-pipeline.md` §4; `ADR-0023-design-lock.md`
//! §3 / §4.16). It consumes a **RESOLVED** tree (the output of
//! [`crate::iq::resolve::resolve`] — ZERO [`IqNode::Intensional`] leaves, FILTER/BIND
//! still symbolic) and drives it to the **leaf-CQ spine**: a `Union`-of-
//! (`Construction` over a `Join`/`LeftJoin`/`Filter` of `Extensional`/`Values`/`Path`
//! leaves) under the query-modifier spine, where **each `Union` arm terminates in at
//! most ONE `Construction`** (one `var → BindDef` bindings map) — exactly what one
//! flat [`Branch`](crate::iq::Branch) lowers from.
//!
//! ## Status: PRODUCTION (tree default since ADR-0023 M8; banner corrected 2026-07-18)
//!
//! This NORMALIZE stage runs in the live engine: `translate`/`translate_with`
//! route through [`crate::translate_tree`] by default (`lib.rs`); the flat
//! [`crate::unfold`] remains the `=_bag` oracle / fallback.
//!
//! ## The THREE transformations (design §4 — NOTHING cost-driven)
//!
//! Exactly three structural rewrites reach the spine. The M4 optimizer (selectivity
//! push-down, self-join / FD elimination, DISTINCT-driven OPTIONAL pruning,
//! redundant-join removal, unsat-cond detection) is **explicitly OUT** — those need
//! the resolved `ColRef` form and run at/after LOWER.
//!
//! ### (a) Substitution-lifting (design §4(a))
//!
//! Fold `Construction ∘ Construction` (compose the inner substitution into the outer,
//! keep the outer projection) and push a `Construction` through `InnerJoin`/`Union`
//! so each leaf-CQ terminates in at most one `Construction`. When the per-arm
//! `Construction`s of an `InnerJoin` are merged into one, **variable equivalence from
//! shared-variable equality is materialised exactly as the flat
//! [`merge`](crate::unfold) does it** ([`merge_into`] below mirrors
//! `unfold.rs:1182-1206`): a variable bound by both operands seeds an equality via the
//! proven [`crate::unify::unify`] oracle (its `Sql` conds ride the `InnerJoin.cond`);
//! a variable bound by only one operand is *rebound* from that side (the
//! `bindings.get(var) == None ⇒ insert other side` line, `unfold.rs:1197-1199`) with
//! **no equality** — the per-arm join degenerates to a free dimension on that
//! variable (R2). We **STOP** as soon as each branch has one bindings map; we do NOT
//! chase a global optimizing fixpoint.
//!
//! **Termination measure (a):** the count of `Construction` nodes that are a child of
//! another `Construction` or of an `InnerJoin`. Each fold/lift strictly removes at
//! least one such node; the spine has none (each `Construction` sits at the leaf-CQ
//! root, over a non-`Construction` body).
//!
//! ### (b) Join-over-union distribution (design §4.16, ledger R2)
//!
//! * `InnerJoin(A, Union(B1..Bn)) ⇒ Union(InnerJoin(A,B1)..InnerJoin(A,Bn))` —
//!   **either** operand (bag-exact: `⋈` distributes over bag union).
//! * `Filter(Union(B1..Bn)) ⇒ Union(Filter(B1)..Filter(Bn))`, copying the symbolic
//!   [`IqCond`] only for preceding arms and moving it into the final arm (never a
//!   pre-lowered `SqlCond`).
//! * `LeftJoin` distributes ONLY over a `Union` on its **LEFT** (preserved) operand:
//!   `(A∪B)⟕C ⇒ (A⟕C)∪(B⟕C)`. A `LeftJoin` with a `Union`/multi-scan **RIGHT** does
//!   **not** distribute — it STAYS a `LeftJoin` node (handed to `left_join_branches`
//!   at LOWER, M3c). `A⟕(C∪D)` is NEVER split (it would fabricate a spurious
//!   null-padded row, 1 → 2 — ledger R2 / design §4.16).
//!
//! **Termination measure (b):** the count of `Union` nodes appearing as an operand of
//! an `InnerJoin`/`Filter`/`LeftJoin`-left. Each distribution removes one such `Union`
//! (lifting it above the join); the spine pushes every `Union` to the root.
//!
//! ### (c) Identity pruning (design §4(c) / §4.13)
//!
//! Drop [`IqNode::Empty`] (the `Union` identity — remove the arm, keeping the
//! `Union`'s var signature; the `InnerJoin` absorbing element — the whole join becomes
//! `Empty` over the union of its children's vars) and [`IqNode::True`] (the
//! `InnerJoin` identity — **a condition-free join only**: a `True` child is dropped,
//! but an `InnerJoin` with a non-empty `cond` never collapses to `True`).
//!
//! **Termination measure (c):** node count strictly decreases.
//!
//! ## What is forbidden (ledger R5, =_bag-CRITICAL)
//!
//! **No arm-merge / structural dedup.** A `Union` arm is NEVER collapsed into a
//! structurally identical sibling. Flattening a nested `Union` (associativity) keeps
//! every arm; pruning an `Empty` arm and unwrapping a one-arm `Union` are bag
//! identities — none deduplicate. The bag multiplicity is preserved end-to-end, which
//! is the `=_bag` (multiset-equivalence to the flat base translation) invariant.

use std::collections::BTreeMap;

use crate::iq::node::{BindDef, IqCond, IqNode, Var};
use crate::iq::TermDef;
use crate::unify::{bind_term_def, unify, Unify};
use crate::{CompilerWorkMode, Error, Result};

type ValuesRows = Vec<Vec<Option<TermDef>>>;

/// Normalize a whole RESOLVED tree to the leaf-CQ spine (design §4). Walks `node`
/// bottom-up: each child is normalized first, then the node-local rewrite (fold /
/// lift / distribute / prune) is applied, re-normalizing the structure a distribution
/// produces. Returns a `Union`-of-(`Construction` over a `Join`/`LeftJoin`/`Filter` of
/// leaves) under the query-modifier spine, every `Union` arm carrying at most one
/// bindings map.
///
/// FILTER/BIND stay **symbolic** ([`IqCond::Expr`] / [`BindDef::Expr`]) — NORMALIZE
/// only moves and clones them; they are resolved per-leaf-CQ at LOWER (M3c). The
/// normalizer **descends into** `EXISTS`/`NOT EXISTS` payloads ([`IqCond::Exists`] /
/// [`IqCond::NotExists`]) and normalizes them as first-class `IqNode`s (design-lock §3
/// recursion clause, BINDING).
pub fn normalize(node: IqNode) -> Result<IqNode> {
    normalize_with_work_mode(node, CompilerWorkMode::Uncontrolled)
}

/// Internal normalization entry point carrying the private compiler-work seam.
/// Public/raw normalization always calls [`normalize`] and remains uncontrolled.
pub(crate) fn normalize_with_work_mode(
    node: IqNode,
    work_mode: CompilerWorkMode<'_>,
) -> Result<IqNode> {
    match node {
        // ---- substitution-lifting carrier (a) -----------------------------------
        IqNode::Construction {
            child,
            subst,
            project,
        } => {
            let child = normalize_with_work_mode(*child, work_mode)?;
            lift_construction(subst, project, child)
        }

        // ---- selection: distribute over Union, else push below Construction -----
        IqNode::Filter { child, cond } => {
            let child = normalize_with_work_mode(*child, work_mode)?;
            normalize_filter(cond, child, work_mode)
        }

        // ---- n-ary inner join: distribute over Union, else lift Constructions ----
        IqNode::InnerJoin { children, cond } => {
            let children = children
                .into_iter()
                .map(|child| normalize_with_work_mode(child, work_mode))
                .collect::<Result<Vec<_>>>()?;
            normalize_inner_join(children, cond, work_mode)
        }

        // ---- left join: distribute over a LEFT Union only; right stays intact ----
        IqNode::LeftJoin { left, right, cond } => {
            let left = normalize_with_work_mode(*left, work_mode)?;
            let right = normalize_with_work_mode(*right, work_mode)?;
            normalize_left_join(left, right, cond, work_mode)
        }

        // ---- bag union: flatten, prune Empty arms (NO arm-merge) ----------------
        IqNode::Union { children, project } => {
            let children = children
                .into_iter()
                .map(|child| normalize_with_work_mode(child, work_mode))
                .collect::<Result<Vec<_>>>()?;
            normalize_union(children, project)
        }

        // ---- modifier spine: normalize the child, keep the node above the Union --
        IqNode::Aggregation {
            child,
            grouping,
            aggs,
        } => Ok(IqNode::Aggregation {
            child: Box::new(normalize_with_work_mode(*child, work_mode)?),
            grouping,
            aggs,
        }),
        IqNode::Distinct { child } => {
            let child = normalize_with_work_mode(*child, work_mode)?;
            Ok(normalize_distinct(child))
        }
        IqNode::Slice {
            child,
            offset,
            limit,
        } => {
            let child = normalize_with_work_mode(*child, work_mode)?;
            Ok(normalize_slice(offset, limit, child))
        }
        IqNode::OrderBy { child, keys } => Ok(IqNode::OrderBy {
            child: Box::new(normalize_with_work_mode(*child, work_mode)?),
            keys,
        }),

        // ---- leaves / identities pass through ------------------------------------
        // `Intensional`/`UnresolvedPath` MUST already be gone (RESOLVE invariant); they
        // are carried through verbatim rather than special-cased, so a contract violation
        // is visible downstream (a LOWER 501) instead of being silently rewritten.
        leaf @ (IqNode::Extensional { .. }
        | IqNode::Values { .. }
        | IqNode::Path { .. }
        | IqNode::Empty { .. }
        | IqNode::True
        | IqNode::Intensional { .. }
        | IqNode::UnresolvedPath { .. }) => Ok(leaf),
    }
}

// ---- (a) substitution-lifting ----------------------------------------------------

/// Lift a `Construction { subst, project }` over its already-normalized `child`
/// (design §4(a)). Folds `Construction ∘ Construction` (compose substitutions, keep
/// the outer projection) and pushes a non-trivial substitution through a `Union` so
/// each arm carries its own complete bindings map (the §4(b)/R4 precondition for
/// per-leaf-CQ FILTER/BIND resolution at LOWER). A pure projection (empty `subst`)
/// over a `Union` is pushed into the arms too (only narrowing each arm to `project`,
/// which is bag-preserving): the `Union` MUST surface to the spine top rather than
/// stay an opaque `Construction{Union}` that a parent `InnerJoin`/`Filter`
/// distribution (matching on `IqNode::Union`) cannot see through.
fn lift_construction(
    subst: BTreeMap<Var, BindDef>,
    project: Vec<Var>,
    child: IqNode,
) -> Result<IqNode> {
    match child {
        // Construction ∘ Construction: compose the inner substitution into the outer
        // (the outer overrides on a key clash — a re-bind), keep the outer projection,
        // and re-lift over the inner body (which may itself be a Union to push into).
        IqNode::Construction {
            child: gchild,
            subst: inner,
            ..
        } => {
            let mut merged = inner;
            for (k, v) in subst {
                merged.insert(k, v);
            }
            lift_construction(merged, project, *gchild)
        }

        // Construction over Union: push the substitution AND the projection into each
        // arm so every arm carries one bindings map and the `Union` surfaces to the
        // spine top (a `Union`-of-leaf-CQs). A pure projection (empty `subst`) is pushed
        // too — it only narrows each arm to `project` (bag-preserving) — so the node
        // never stays an opaque `Construction{Union}` that hides the `Union` from a
        // parent `InnerJoin`/`Filter` distribution (both match on `IqNode::Union`),
        // which would trap it un-distributed inside a join/filter body (a spine /
        // `=_bag` violation: the trapped `Union` never cross-products with the join's
        // other operands).
        IqNode::Union {
            children: mut arms, ..
        } => {
            let mut out = Vec::with_capacity(arms.len());
            let last = arms.pop();
            for a in arms {
                out.push(lift_construction(subst.clone(), project.clone(), a)?);
            }
            if let Some(a) = last {
                // Every arm owns an independent substitution. Preserve order while
                // moving the original into the final arm, so only the preceding
                // fan-out arms pay for recursive copies.
                out.push(lift_construction(subst, project.clone(), a)?);
            }
            normalize_union(out, project)
        }

        // Construction over Empty: ∅ over the projected variables.
        IqNode::Empty { .. } => Ok(IqNode::Empty { vars: project }),

        // Construction over a join / leaf / filter / left-join body: the leaf-CQ root.
        other => Ok(IqNode::Construction {
            child: Box::new(other),
            subst,
            project,
        }),
    }
}

/// Merge an `incoming` substitution into the accumulator `acc`, mirroring the flat
/// [`merge`](crate::unfold) (`unfold.rs:1182-1206`) variable-by-variable:
///
/// * a variable absent from `acc` is **rebound** from `incoming` (the
///   `bindings.get(var) == None ⇒ insert other side` line, `unfold.rs:1197-1199`) —
///   a free dimension, **no equality** (R2);
/// * a variable bound in **both** seeds an equality via the proven
///   [`crate::unify::unify`] oracle: `Sat` conds are appended to `eqs` (as
///   [`IqCond::Sql`]); `Empty` (provably disjoint) signals the whole join is empty;
///   `Unsupported` propagates as a tracked sound-501.
///
/// A shared variable whose def is a **symbolic** [`BindDef::Expr`] on either side
/// cannot be unified before LOWER (the flat oracle likewise defers a `Concat`/computed
/// binding, [`crate::unify::unify`]) → a tracked sound-501, never silently dropped.
///
/// Returns `Ok(true)` iff the merge proved the join empty (a disjointness prune).
fn merge_into(
    acc: &mut BTreeMap<Var, BindDef>,
    incoming: BTreeMap<Var, BindDef>,
    eqs: &mut Vec<IqCond>,
) -> Result<bool> {
    for (var, rdef) in incoming {
        match acc.get(&var) {
            None => {
                acc.insert(var, rdef);
            }
            Some(ldef) => match (ldef, &rdef) {
                (BindDef::Resolved(l), BindDef::Resolved(r)) => match unify(l, r) {
                    Unify::Sat(conds) => eqs.extend(conds.into_iter().map(IqCond::Sql)),
                    Unify::Empty => return Ok(true),
                    Unify::Unsupported(why) => return Err(Error::Unsupported(why)),
                },
                _ => {
                    return Err(Error::Unsupported(format!(
                        "normalize: shared join variable ?{var} has a symbolic BIND \
                         definition on one side → 501 (deferred, never silently dropped)"
                    )))
                }
            },
        }
    }
    Ok(false)
}

// ---- (b) + (a) inner join: distribute over Union, else lift to one Construction ---

/// Normalize an `InnerJoin` whose `children` are already normalized (design §4(b) +
/// §4(c) + §4(a), in that order). Empty-absorbs, drops condition-free `True` identity
/// children, distributes over a `Union` child (recursing per arm), then — once no
/// `Union` child remains — lifts the children's `Construction`s into a single
/// `Construction` over the flattened `InnerJoin` of leaves.
fn normalize_inner_join(
    children: Vec<IqNode>,
    cond: Vec<IqCond>,
    work_mode: CompilerWorkMode<'_>,
) -> Result<IqNode> {
    let join_vars = union_vars(&children);

    // (c) absorbing: any Empty child ⇒ the whole join is Empty over all its vars.
    if children.iter().any(|c| matches!(c, IqNode::Empty { .. })) {
        return Ok(IqNode::Empty { vars: join_vars });
    }

    // (c) identity: drop the condition-free `True` element (the empty tuple).
    let mut children: Vec<IqNode> = children
        .into_iter()
        .filter(|c| !matches!(c, IqNode::True))
        .collect();
    if children.is_empty() {
        // §4.13: `True` is the InnerJoin identity for a CONDITION-FREE join only. A
        // join of only `True` children with a residual `cond` is NOT vacuous (the cond
        // may be ground-false, e.g. a constant-position `IqCond::Sql`); it is preserved
        // as a `Filter` over the empty tuple, never silently dropped.
        return if cond.is_empty() {
            Ok(IqNode::True)
        } else {
            Ok(IqNode::Filter {
                child: Box::new(IqNode::True),
                cond: normalize_conds(cond, work_mode)?,
            })
        };
    }
    if children.len() == 1 && cond.is_empty() {
        return Ok(children.pop().expect("len checked == 1"));
    }

    // (b) distribute over the FIRST Union child (either operand): re-normalize each
    // resulting per-arm InnerJoin, then re-form the Union above the join.
    if let Some(i) = children
        .iter()
        .position(|c| matches!(c, IqNode::Union { .. }))
    {
        let IqNode::Union {
            children: mut arms, ..
        } = children.remove(i)
        else {
            unreachable!("position matched a Union");
        };
        let mut out_arms = Vec::with_capacity(arms.len());
        let last = arms.pop();
        for arm in arms {
            let mut nc = work_mode.clone_iq_nodes(&children)?;
            nc.insert(i, arm);
            let arm_cond = work_mode.clone_iq_conditions(&cond)?;
            out_arms.push(normalize_inner_join(nc, arm_cond, work_mode)?);
        }
        if let Some(arm) = last {
            // The final distributed arm can consume both fixed operands and the
            // condition; all earlier arms still require independent copies.
            children.insert(i, arm);
            out_arms.push(normalize_inner_join(children, cond, work_mode)?);
        }
        return normalize_union(out_arms, join_vars);
    }

    // (a) no Union child: lift the children's Constructions to one Construction.
    lift_inner_join(children, cond, join_vars, work_mode)
}

/// Lift the `Construction`s of a Union-free `InnerJoin` into a single `Construction`
/// over the flattened `InnerJoin` of leaves (design §4(a)). Each `Construction` child
/// contributes its substitution (merged via [`merge_into`], generating shared-variable
/// equalities) and its body (a nested `InnerJoin` is spliced in — associativity);
/// every other child kind (a `LeftJoin`/`Filter` carrying its own internal bindings, a
/// bare leaf) becomes an `InnerJoin` body child verbatim, so no binding is lost.
fn lift_inner_join(
    children: Vec<IqNode>,
    cond: Vec<IqCond>,
    join_vars: Vec<Var>,
    work_mode: CompilerWorkMode<'_>,
) -> Result<IqNode> {
    let mut acc: BTreeMap<Var, BindDef> = BTreeMap::new();
    let mut body_children: Vec<IqNode> = Vec::new();
    let mut body_cond: Vec<IqCond> = normalize_conds(cond, work_mode)?;
    let mut had_construction = false;

    for child in children {
        match child {
            IqNode::Construction {
                child: cbody,
                subst,
                ..
            } => {
                had_construction = true;
                if merge_into(&mut acc, subst, &mut body_cond)? {
                    return Ok(IqNode::Empty { vars: join_vars });
                }
                // Splice a nested InnerJoin body (associativity); else add it whole.
                match *cbody {
                    IqNode::InnerJoin {
                        children: gc,
                        cond: gcond,
                    } => {
                        body_children.extend(gc);
                        body_cond.extend(gcond);
                    }
                    other => body_children.push(other),
                }
            }
            // A bare (Construction-free) nested InnerJoin flattens directly.
            IqNode::InnerJoin {
                children: gc,
                cond: gcond,
            } => {
                body_children.extend(gc);
                body_cond.extend(gcond);
            }
            other => body_children.push(other),
        }
    }

    let body = match body_children.len() {
        0 => IqNode::True,
        1 if body_cond.is_empty() => body_children.pop().expect("len checked == 1"),
        _ => IqNode::InnerJoin {
            children: body_children,
            cond: body_cond,
        },
    };

    if had_construction {
        Ok(IqNode::Construction {
            child: Box::new(body),
            subst: acc,
            project: join_vars,
        })
    } else {
        Ok(body)
    }
}

// ---- (b) + spine placement: filter ------------------------------------------------

/// Normalize a `Filter { cond }` over its already-normalized `child` (design §4(b)).
/// Distributes over a `Union` (copying the symbolic `cond` for preceding arms and
/// moving it into the final arm), and over a leaf-CQ pushes the `Filter` **below the
/// `Construction`** (the spine leaf-CQ is `Construction` over a `Filter` of leaves).
/// Adjacent `Filter`s coalesce.
fn normalize_filter(
    cond: Vec<IqCond>,
    child: IqNode,
    work_mode: CompilerWorkMode<'_>,
) -> Result<IqNode> {
    let cond = normalize_conds(cond, work_mode)?;
    match child {
        IqNode::Empty { vars } => Ok(IqNode::Empty { vars }),
        IqNode::Union {
            children: mut arms,
            project,
        } => {
            let mut out = Vec::with_capacity(arms.len());
            let last = arms.pop();
            for a in arms {
                let arm_cond = work_mode.clone_iq_conditions(&cond)?;
                out.push(normalize_filter(arm_cond, a, work_mode)?);
            }
            if let Some(a) = last {
                out.push(normalize_filter(cond, a, work_mode)?);
            }
            normalize_union(out, project)
        }
        IqNode::Construction {
            child: body,
            subst,
            project,
        } => Ok(IqNode::Construction {
            child: Box::new(push_filter(cond, *body)),
            subst,
            project,
        }),
        IqNode::Filter {
            child: body,
            cond: inner,
        } => {
            let mut merged = cond;
            merged.extend(inner);
            Ok(IqNode::Filter {
                child: body,
                cond: merged,
            })
        }
        other => Ok(IqNode::Filter {
            child: Box::new(other),
            cond,
        }),
    }
}

/// Wrap `body` in a `Filter[cond]`, coalescing with an existing `Filter` (the `cond`
/// is already normalized).
fn push_filter(cond: Vec<IqCond>, body: IqNode) -> IqNode {
    match body {
        IqNode::Filter { child, cond: inner } => {
            let mut merged = cond;
            merged.extend(inner);
            IqNode::Filter {
                child,
                cond: merged,
            }
        }
        other => IqNode::Filter {
            child: Box::new(other),
            cond,
        },
    }
}

// ---- (b) left join: distribute over the LEFT union only ---------------------------

/// Normalize a `LeftJoin` over its already-normalized `left`/`right` (design §4(b),
/// ledger R1/R2). Distributes ONLY over a `Union` on the **left** (preserved) operand;
/// a `Union`/multi-scan **right** is preserved as-is (the node STAYS a `LeftJoin`,
/// lowered via `left_join_branches` at M3c — `A⟕(C∪D)` is never split). An empty left
/// ⇒ `Empty`; an empty right ⇒ the left unchanged (`OPTIONAL {}` over no match).
fn normalize_left_join(
    left: IqNode,
    right: IqNode,
    cond: Vec<IqCond>,
    work_mode: CompilerWorkMode<'_>,
) -> Result<IqNode> {
    let cond = normalize_conds(cond, work_mode)?;
    let combined_vars = {
        let mut v = left.output_vars();
        for x in right.output_vars() {
            if !v.contains(&x) {
                v.push(x);
            }
        }
        v
    };
    match left {
        IqNode::Empty { .. } => Ok(IqNode::Empty {
            vars: combined_vars,
        }),
        IqNode::Union {
            children: mut arms, ..
        } => {
            let mut out = Vec::with_capacity(arms.len());
            let last = arms.pop();
            for a in arms {
                let arm_right = work_mode.clone_iq_node(&right)?;
                let arm_cond = work_mode.clone_iq_conditions(&cond)?;
                out.push(normalize_left_join(a, arm_right, arm_cond, work_mode)?);
            }
            if let Some(a) = last {
                // LEFT JOIN is non-commutative, so retain the left-arm order and
                // transfer the original shared right/condition only to the last.
                out.push(normalize_left_join(a, right, cond, work_mode)?);
            }
            normalize_union(out, combined_vars)
        }
        _ => match right {
            IqNode::Empty { .. } => Ok(left),
            _ => Ok(IqNode::LeftJoin {
                left: Box::new(left),
                right: Box::new(right),
                cond,
            }),
        },
    }
}

// ---- (c) union: flatten + prune (NO arm-merge) -----------------------------------

/// Normalize a `Union` over already-normalized `children` (design §4(c), ledger R5).
/// Flattens nested `Union`s (associativity — keeps every arm), prunes `Empty` arms
/// (the `Union` identity), and unwraps a one-arm `Union`. Beyond that it performs
/// **NO multiplicity-losing arm-merge / dedup** — a multiplicity-bearing arm is never
/// collapsed into a sibling — with exactly one exception: [`fold_constant_union`]
/// (ADR-0023 optimizer-residue Wave C, §4.15) combines arms that are ALL bare
/// constant tuples into one `Values` leaf, which is bag-preserving (N constant arms
/// → N rows), not a lossy dedup.
fn normalize_union(children: Vec<IqNode>, project: Vec<Var>) -> Result<IqNode> {
    let mut arms: Vec<IqNode> = Vec::new();
    for c in children {
        match c {
            IqNode::Empty { .. } => {} // prune the Union identity (keep the signature)
            IqNode::Union {
                children: inner, ..
            } => arms.extend(inner), // flatten (associativity — NOT dedup)
            other => arms.push(other),
        }
    }
    Ok(match arms.len() {
        0 => IqNode::Empty { vars: project },
        1 => arms.pop().expect("len checked == 1"),
        _ if arms
            .iter()
            .all(|arm| constant_arm_is_foldable(arm, &project)) =>
        {
            fold_constant_union(arms, project)
        }
        _ => fold_partial_constant_runs(arms, project),
    })
}

/// Fold a `Union` of ALL-CONSTANT arms into one `Values` node (Ontop
/// `ValuesNodeOptimization::test14ConstructionUnionTrueTrue`): when every arm is a
/// `Construction` over `True` (i.e. genuinely data-free — a `BIND`-only row, no
/// pattern underneath) with every one of its bindings already a resolved constant,
/// the whole `Union` is exactly the literal table of those constant tuples — the
/// SAME `=_bag` multiset (one row per arm), just represented as a `Values` leaf
/// instead of an N-arm `Union` of single-row `Construction`s.
///
/// The caller preflights every arm and uses [`fold_partial_constant_runs`] instead
/// when a binding is not compile-time constant or an arm contains a real pattern.
///
/// Ontop `ValuesNodeOptimization::test25NoVariableTrueNodesAndValuesNodes`: a bare
/// `IqNode::True` arm (the zero-var identity — no `Construction` at all, since it
/// binds nothing) is ALSO foldable, contributing exactly one empty-tuple row, but
/// ONLY when `project` is itself empty (a `True` arm cannot supply a value for any
/// projected variable, so it is never a valid fold candidate otherwise). This is
/// the ONLY place `project.is_empty()` matters: every other arm shape already
/// degrades correctly on an empty `project` with no special-casing (the per-`var`
/// loop below simply doesn't execute, producing an empty row, exactly the `Values`
/// leaf's own "counting" row shape for this case).
///
/// A `BIND`-only arm's binding is still a **symbolic** `BindDef::Expr` at NORMALIZE
/// time (FILTER/BIND resolve per-leaf-CQ at LOWER, not here — confirmed empirically:
/// `{ BIND("a" AS ?x) } UNION { BIND("b" AS ?x) }` normalizes to `Union[Construction{
/// subst: {x: Expr(Literal("a"))}, child: True}, …]`, never a pre-`Resolved` constant).
/// [`bind_term_def`] is reused with an EMPTY bindings map to recognize a genuine
/// constant without resolving it against any column: it only ever succeeds on a bare
/// IRI/literal or a `CONCAT` of recursively-constant parts (`Expression::Variable`
/// always fails against an empty map), so a successful result is *provably*
/// column-free — safe to embed directly in a core-less `Values` row.
fn fold_constant_union(arms: Vec<IqNode>, project: Vec<Var>) -> IqNode {
    debug_assert!(arms.len() >= 2);
    let mut rows = Vec::with_capacity(arms.len());
    for arm in arms {
        rows.extend(
            into_const_rows(arm, &project).expect("constant_arm_is_foldable checked every arm"),
        );
    }
    IqNode::Values {
        vars: project,
        rows,
    }
}

/// Whether an arm can be consumed into constant rows without retaining the arm.
fn constant_arm_is_foldable(arm: &IqNode, project: &[Var]) -> bool {
    match arm {
        IqNode::Values { vars, .. } => same_var_set(vars, project),
        IqNode::Construction { child, subst, .. } if matches!(**child, IqNode::True) => {
            constant_subst_can_form_row(subst, project)
        }
        IqNode::True => project.is_empty(),
        _ => false,
    }
}

fn constant_subst_can_form_row(subst: &BTreeMap<Var, BindDef>, project: &[Var]) -> bool {
    let no_vars = BTreeMap::new();
    project.iter().all(|var| match subst.get(var) {
        Some(BindDef::Resolved(TermDef::Const(_))) | None => true,
        Some(BindDef::Expr(expression)) => bind_term_def(expression, &no_vars).is_ok(),
        Some(_) => false,
    })
}

fn constant_row_from_subst(
    mut subst: BTreeMap<Var, BindDef>,
    project: &[Var],
) -> Option<Vec<Option<TermDef>>> {
    let no_vars = BTreeMap::new();
    let mut row = Vec::with_capacity(project.len());
    for var in project {
        let cell = match subst.remove(var) {
            Some(BindDef::Resolved(TermDef::Const(term))) => Some(TermDef::Const(term)),
            Some(BindDef::Expr(expression)) => Some(bind_term_def(&expression, &no_vars).ok()?),
            Some(_) => return None,
            None => None,
        };
        row.push(cell);
    }
    Some(row)
}

/// Consume the constant row(s) this arm contributes to a folded `Values` leaf.
fn into_const_rows(arm: IqNode, project: &[Var]) -> Option<Vec<Vec<Option<TermDef>>>> {
    match arm {
        // `A UNION B UNION C` is left-associative (`(A UNION B) UNION C`): the
        // inner `(A UNION B)` normalizes (and, when both are constant, this SAME
        // rule folds it) *before* the outer Union ever sees it, so an
        // already-folded `Values` arm must be absorbed directly, not just a
        // `Construction`. Ontop `ValuesNodeOptimization::test26MergeableCombination`:
        // the arm's own column ORDER need not match `project`'s — `same_var_set`
        // (SAME variables, order-independent) is the acceptance test, and
        // `reorder_row` permutes each row by variable NAME to `project`'s order
        // before absorbing it (a no-op permutation when the orders already agree).
        IqNode::Values { vars, rows } if same_var_set(&vars, project) => Some(
            rows.into_iter()
                .map(|row| reorder_row(vars.as_slice(), row, project))
                .collect(),
        ),
        IqNode::Construction { child, subst, .. } if matches!(child.as_ref(), IqNode::True) => {
            Some(vec![constant_row_from_subst(subst, project)?])
        }
        // test25: a bare `True` arm binds nothing, so it can only fold when
        // there is nothing TO bind -- an empty `project` -- contributing one
        // empty-tuple row (the same "counting" shape a zero-column `Values`
        // leaf already has). Revert-proof note: removing just this arm does NOT
        // change correctness -- `lift_construction`'s Construction-over-Union
        // case (this file) individually wraps every bare `True` arm in an
        // identity `Construction` before a SECOND fold pass, which the
        // `Construction{child:True,..}` case above already handles once the
        // `project.is_empty()` guard is lifted. What this arm changes is WHICH
        // of the two fold opportunities fires first: with it, the plain
        // (non-lifted) `normalize()` Union dispatch folds immediately,
        // preserving the outer identity-Construction wrapper this whole file's
        // other tests consistently expect; without it, the fold still happens
        // (via `lift_construction`'s second pass) but that path REPLACES the
        // Construction outright, leaving a bare `Values` -- an equally
        // `=_bag`-correct but inconsistent shape. Kept for that consistency,
        // not because the fold would otherwise fail.
        IqNode::True if project.is_empty() => Some(vec![Vec::new()]),
        _ => None, // a DATA arm (real pattern), or a shape not covered above
    }
}

/// Partial version of [`fold_constant_union`] (Ontop
/// `ValuesNodeOptimization::test15ConstructionUnionTrueTrueDataNode`): when SOME
/// (but not all) of a `Union`'s arms are constant, fold JUST those into one
/// `Values` arm, keeping the rest (the DATA arms — real patterns underneath)
/// untouched as sibling `Union` arms. The SAME `=_bag` multiset either way (a
/// `Union` distributes row-membership independently of how its arms are grouped);
/// only the SQL shape changes — fewer arms to plan/execute.
///
/// Returns the original owned arms when NO maximal contiguous run of 2+ constant
/// arms exists (a single isolated constant arm gains nothing from being wrapped
/// in a one-arm `Values`). The caller handles the all-constant case first with
/// [`fold_constant_union`].
///
/// Folds each maximal contiguous run of 2+ constant arms into one `Values` arm
/// AT that run's own starting position — never combining constant arms
/// separated by a DATA arm. A LIMIT without ORDER BY is implementation-defined
/// SPARQL-wise, but flat and tree are two implementations of the SAME engine
/// that must agree with EACH OTHER on which rows survive (the `=_bag` gate this
/// whole differential proves); flat iterates every arm in its own as-written
/// order, so combining two constant arms that straddle a data arm would move
/// the SECOND one's row earlier (or the data arm's rows later) relative to
/// flat — a real bug an adversarial review caught (`SELECT ?n WHERE {{data}}
/// UNION {{c1}} UNION {{c2}} LIMIT 2` returned the data arm's rows on the flat
/// side but the folded constants on the tree side, because an earlier version
/// of this function unconditionally prepended the fold regardless of position).
fn fold_partial_constant_runs(arms: Vec<IqNode>, project: Vec<Var>) -> IqNode {
    let foldable: Vec<bool> = arms
        .iter()
        .map(|arm| constant_arm_is_foldable(arm, &project))
        .collect();
    if !foldable.windows(2).any(|pair| pair[0] && pair[1]) {
        return IqNode::Union {
            children: arms,
            project,
        };
    }

    let mut children = Vec::with_capacity(arms.len());
    let mut source = arms.into_iter();
    let mut i = 0;
    while i < foldable.len() {
        if !foldable[i] {
            children.push(source.next().expect("foldability mirrors source arms"));
            i += 1;
            continue;
        }

        let run_end = foldable[i..]
            .iter()
            .position(|foldable| !foldable)
            .map_or(foldable.len(), |offset| i + offset);
        let run_len = run_end - i;
        if run_len >= 2 {
            let mut run_rows = Vec::new();
            for _ in 0..run_len {
                let arm = source.next().expect("foldability mirrors source arms");
                run_rows.extend(
                    into_const_rows(arm, &project)
                        .expect("constant_arm_is_foldable checked the run"),
                );
            }
            children.push(IqNode::Values {
                vars: project.to_vec(),
                rows: run_rows,
            });
        } else {
            children.push(source.next().expect("foldability mirrors source arms"));
        }
        i = run_end;
    }

    if children.len() == 1 {
        children.pop().expect("len checked == 1")
    } else {
        IqNode::Union { children, project }
    }
}

// ---- (d) Slice-over-Values truncation (ADR-0023 optimizer-residue Wave C; Ontop
// ValuesNodeOptimization::test1/test2normalizationSlice) --------------------------

/// Truncate a literal `Values` table directly when a `Slice` sits over it (possibly
/// through the identity/pure-projection `Construction` the builder wraps a top-level
/// `SELECT`'s VALUES body in — the common case, confirmed empirically: `SELECT ?x
/// WHERE { VALUES ?x {…} } LIMIT n` normalizes to `Slice(Construction(Values))`, not
/// a bare `Slice(Values)`), instead of carrying the `Slice` down to LOWER (which would
/// lower every row to its own branch and apply offset/limit as a `Plan`-level SQL
/// clause over the full arm count). A `Values` block's rows have a fixed as-written
/// order and nothing else can reorder them upstream of this `Slice`, so reading the
/// same `[offset, offset+limit)` window in Rust here is `=_bag`-identical to lowering
/// the full table and slicing at emission — just without ever materializing the
/// dropped rows/branches (cosmetic SQL-shape parity with Ontop, not a correctness fix;
/// §4.15-adjacent, not a currently-numbered §4 rule). The `Construction`'s `subst`/
/// `project` apply per-row and don't reorder rows, so they commute with slicing
/// unconditionally. Any other child shape keeps the `Slice` node as-is.
fn normalize_slice(offset: usize, limit: Option<usize>, child: IqNode) -> IqNode {
    match child {
        IqNode::Values { vars, rows } => IqNode::Values {
            vars,
            rows: slice_rows(rows, offset, limit),
        },
        IqNode::Union { children, project } => {
            normalize_slice_over_union(offset, limit, children, project)
        }
        IqNode::Construction {
            child: inner,
            subst,
            project,
        } if matches!(*inner, IqNode::Values { .. } | IqNode::Union { .. }) => {
            IqNode::Construction {
                child: Box::new(normalize_slice(offset, limit, *inner)),
                subst,
                project,
            }
        }
        child => IqNode::Slice {
            child: Box::new(child),
            offset,
            limit,
        },
    }
}

/// The `[offset, offset+limit)` window of `rows` (`limit == None` ⇒ to the end).
fn slice_rows(
    rows: Vec<Vec<Option<TermDef>>>,
    offset: usize,
    limit: Option<usize>,
) -> Vec<Vec<Option<TermDef>>> {
    rows.into_iter()
        .skip(offset)
        .take(limit.unwrap_or(usize::MAX))
        .collect()
}

// ---- (d2) Slice-over-Union arm-drop / residual-limit (ADR-0023 optimizer-residue
// Wave C; Ontop ValuesNodeOptimization::test5-7SliceUnionValuesNonValues) ---------

/// An arm's statically-known row set matching `project` in EXACT column order: a
/// bare `Values` leaf, or a single-row all-constant `Construction{child: True, ...}`
/// (the same two shapes [`fold_constant_union`] recognizes for a WHOLE `Union`,
/// generalized here to one arm at a time — a mixed `Union` with a genuine DATA arm
/// declines the full constant fold entirely, so an all-constant arm can still
/// reach this function unfolded). `None` ⇒ unknown cardinality (a real pattern, or
/// a column-order mismatch this rule doesn't reconcile — cf. test26).
fn static_row_count(arm: &IqNode, project: &[Var]) -> Option<usize> {
    if let IqNode::Values { vars, rows } = arm {
        return (vars.as_slice() == project).then_some(rows.len());
    }
    // A lone `VALUES` block as a Union arm is ALSO wrapped in the builder's identity-
    // projection `Construction` (confirmed empirically — the same pattern `normalize_
    // slice`/`normalize_distinct` already handle for the top-level-body case, but here
    // for one arm among several). `child.as_ref()` decides which of the two shapes
    // this is; no ambiguity between them (`Values`/`True` are distinct variants).
    let IqNode::Construction {
        child,
        subst,
        project: arm_project,
    } = arm
    else {
        return None;
    };
    match child.as_ref() {
        IqNode::Values { vars, rows }
            if subst.is_empty()
                && arm_project.as_slice() == project
                && vars.as_slice() == project =>
        {
            Some(rows.len())
        }
        IqNode::True if constant_subst_can_form_row(subst, project) => Some(1),
        _ => None,
    }
}

fn into_static_rows(arm: IqNode, project: &[Var]) -> Option<Vec<Vec<Option<TermDef>>>> {
    match arm {
        IqNode::Values { vars, rows } if vars.as_slice() == project => Some(rows),
        IqNode::Construction {
            child,
            subst,
            project: arm_project,
        } => match *child {
            IqNode::Values { vars, rows }
                if subst.is_empty()
                    && arm_project.as_slice() == project
                    && vars.as_slice() == project =>
            {
                Some(rows)
            }
            IqNode::True => Some(vec![constant_row_from_subst(subst, project)?]),
            _ => None,
        },
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct SliceUnionPlan {
    known_arms: usize,
    drop_remaining: bool,
}

fn plan_slice_over_union(
    offset: usize,
    limit: Option<usize>,
    arms: &[IqNode],
    project: &[Var],
) -> Option<SliceUnionPlan> {
    let limit_n = limit.unwrap_or(usize::MAX);
    let window_end = offset.saturating_add(limit_n);
    let mut cursor = 0usize;
    let mut survivor_count = 0usize;
    let mut i = 0;
    let mut changed = false;

    while i < arms.len() {
        if survivor_count >= limit_n {
            return Some(SliceUnionPlan {
                known_arms: i,
                drop_remaining: true,
            });
        }
        let Some(row_count) = static_row_count(&arms[i], project) else {
            break;
        };
        let local_start = offset.saturating_sub(cursor).min(row_count);
        let local_end = window_end.saturating_sub(cursor).min(row_count);
        if local_start > 0 || local_end < row_count {
            changed = true;
        }
        survivor_count += local_end.saturating_sub(local_start);
        cursor += row_count;
        i += 1;
    }

    changed.then_some(SliceUnionPlan {
        known_arms: i,
        drop_remaining: false,
    })
}

/// Resolve a `Slice(offset, limit)` over a `Union` whose LEADING arms have a
/// statically-known row count ([`static_row_count`]) by walking them in as-written
/// order, tracking `cursor` (the true row count consumed so far) and `survivors`
/// (the rows the `[offset, offset+limit)` window actually keeps from them):
///
/// * An arm entirely BEFORE the window (`cursor + n <= offset`) contributes nothing
///   — dropped outright.
/// * An arm straddling the window contributes its own local `[offset-cursor,
///   limit_end-cursor)` slice — dropped down to just the surviving rows.
/// * Once `survivors.len()` already reaches `limit`, every remaining arm is
///   unreachable regardless of its own size or kind — dropped, INCLUDING an
///   unknown-cardinality one.
/// * An unknown-cardinality arm reached BEFORE the window is satisfied means we
///   genuinely cannot tell how many (if any) of its rows are needed — processing
///   STOPS there: that arm and everything after survives untouched, under a fresh
///   `Slice(0, limit)` over `[the truncated survivor prefix (if non-empty), the
///   unknown arm, ...the rest]` — offset 0 because the survivors already account
///   for everything before them in as-written order, so a plain Slice-over-Union
///   (whose own semantics already read arms in order from position 0) does the
///   right thing for whatever follows, at any depth, with no further bookkeeping.
///
/// Keeps the original owned `Slice{Union}` when nothing at all could be dropped or
/// truncated — e.g. the first arm is already unknown and the window is not yet
/// satisfied — matching every other rule's sound-decline convention.
fn normalize_slice_over_union(
    offset: usize,
    limit: Option<usize>,
    arms: Vec<IqNode>,
    project: Vec<Var>,
) -> IqNode {
    let Some(plan) = plan_slice_over_union(offset, limit, &arms, &project) else {
        return IqNode::Slice {
            child: Box::new(IqNode::Union {
                children: arms,
                project,
            }),
            offset,
            limit,
        };
    };

    let limit_n = limit.unwrap_or(usize::MAX);
    let window_end = offset.saturating_add(limit_n);
    let mut cursor = 0usize;
    let mut survivors: Vec<Vec<Option<TermDef>>> = Vec::new();
    let mut source = arms.into_iter();

    for _ in 0..plan.known_arms {
        let rows = into_static_rows(
            source.next().expect("slice plan mirrors source arms"),
            &project,
        )
        .expect("static_row_count admitted every planned arm");
        let row_count = rows.len();
        let local_start = offset.saturating_sub(cursor).min(row_count);
        let local_end = window_end.saturating_sub(cursor).min(row_count);
        if local_start < local_end {
            survivors.extend(
                rows.into_iter()
                    .skip(local_start)
                    .take(local_end - local_start),
            );
        }
        cursor += row_count;
    }

    let remaining: Vec<IqNode> = if plan.drop_remaining {
        Vec::new()
    } else {
        source.collect()
    };

    if remaining.is_empty() {
        IqNode::Values {
            vars: project,
            rows: survivors,
        }
    } else {
        let mut new_arms = Vec::new();
        if !survivors.is_empty() {
            new_arms.push(IqNode::Values {
                vars: project.to_vec(),
                rows: survivors,
            });
        }
        new_arms.extend(remaining);
        let child = if new_arms.len() == 1 {
            new_arms.pop().expect("len checked == 1")
        } else {
            IqNode::Union {
                children: new_arms,
                project,
            }
        };
        IqNode::Slice {
            child: Box::new(child),
            // NOT unconditionally 0: an adversarial review caught that the known
            // arms' cumulative row count (`cursor`) may still fall SHORT of the
            // original `offset` when it isn't enough to cover the whole skip on its
            // own (e.g. `offset=4` over a single known 3-row arm then a data arm --
            // all 3 rows are dropped, but 1 MORE row of skip still needs to land on
            // the data arm itself). `survivors` only ever holds what's already
            // correctly positioned at `offset` or later, so the residual is exactly
            // however much of `offset` the known prefix DIDN'T already consume.
            offset: offset.saturating_sub(cursor),
            limit,
        }
    }
}

// ---- (e) Distinct-over-Values dedup (ADR-0023 optimizer-residue Wave C; Ontop
// ValuesNodeOptimization::test3normalizationDistinct) -----------------------------

/// Dedup a literal `Values` table directly when a `Distinct` sits over it (through
/// the same identity-projection `Construction` wrapper `normalize_slice` handles),
/// instead of carrying the `Distinct` down to LOWER (where `SELECT DISTINCT` already
/// produces the right *answer*, but every duplicate row still lowers to its own
/// branch first — the cosmetic cost this rule removes). Any other child shape keeps
/// the `Distinct` node as-is.
fn normalize_distinct(child: IqNode) -> IqNode {
    match child {
        // `dedup_rows` declining (a non-Const cell) must NOT silently drop the
        // `Distinct` requirement itself — only discard the node when dedup actually
        // ran, else the duplicates it left behind would reach LOWER unguarded (a
        // wrong answer, not merely a missed optimization).
        IqNode::Values { vars, rows } => match dedup_rows(rows) {
            Ok(deduped) => IqNode::Values {
                vars,
                rows: deduped,
            },
            Err(rows) => IqNode::Distinct {
                child: Box::new(IqNode::Values { vars, rows }),
            },
        },
        // SAFETY: only when `project` is the SAME variable set as the Values leaf's
        // own `vars` (a pure identity/reorder wrapper) — never a narrowing. DISTINCT
        // in SPARQL algebra applies AFTER Project (18.2.5): if `project` drops a
        // column, rows that differ only in the dropped column must collapse into
        // ONE post-projection row, but deduping the Values leaf's FULL (pre-
        // projection) tuples here would keep them as two — an `=_bag` violation an
        // adversarial review caught (`VALUES (?x ?y) {(1 2)(1 3)(1 2)} SELECT DISTINCT
        // ?x` must yield 1 row, not 2). Declining is always sound: `Distinct` still
        // runs (correctly) at LOWER/exec, same as before this rule existed.
        IqNode::Construction {
            child: inner,
            subst,
            project,
        } if matches!(&*inner, IqNode::Values { vars, .. } if same_var_set(&project, vars)) => {
            IqNode::Construction {
                child: Box::new(normalize_distinct(*inner)),
                subst,
                project,
            }
        }
        // Ontop `ValuesNodeOptimization::test9DistinctUnionValuesNonValues`: dedup
        // each Values-shaped arm's OWN internal duplicates in place, leaving any
        // other arm (and the outer `Distinct` itself — cross-arm duplicates are NOT
        // provable statically) untouched. `dedup_one_arm` re-applies the SAME
        // narrowing-projection guard `same_var_set` enforces above (an arm's own
        // declared columns must exactly match the Union's `project`, no narrowing)
        // — the identical `=_bag` hazard that guard exists for applies per-arm here
        // too, just checked arm-by-arm instead of once at the top.
        IqNode::Union { children, project } => IqNode::Distinct {
            child: Box::new(IqNode::Union {
                children: children
                    .into_iter()
                    .map(|arm| dedup_one_arm(arm, &project))
                    .collect(),
                project,
            }),
        },
        child => IqNode::Distinct {
            child: Box::new(child),
        },
    }
}

/// Whether `a` and `b` name exactly the same set of variables (order-independent,
/// no narrowing either way). Assumes both are already duplicate-free (true of every
/// `project`/`vars` this crate builds — SPARQL rejects a repeated variable name in a
/// projection or a `VALUES` header); a caller passing a list with a repeated name
/// would get a false positive here.
fn same_var_set(a: &[Var], b: &[Var]) -> bool {
    a.len() == b.len() && a.iter().all(|v| b.contains(v))
}

/// Permute one row's cells from `from_order` to `to_order` by variable NAME (both
/// lists name the SAME set of variables — the caller checks with [`same_var_set`]
/// first; a name absent from `from_order` here would panic, which never happens
/// under that precondition). A no-op permutation when the two orders already agree.
fn reorder_row(
    from_order: &[Var],
    mut row: Vec<Option<TermDef>>,
    to_order: &[Var],
) -> Vec<Option<TermDef>> {
    if from_order == to_order {
        return row;
    }
    to_order
        .iter()
        .map(|v| {
            let i = from_order
                .iter()
                .position(|w| w == v)
                .expect("same_var_set checked by the caller");
            std::mem::take(&mut row[i])
        })
        .collect()
}

/// Dedup ONE `Union` arm's own internal duplicate rows (a bare `Values` leaf, or a
/// lone `VALUES`-as-union-arm wrapped in the builder's identity-projection
/// `Construction` — the same shape `static_row_count` recognizes, but returning the
/// (possibly unchanged) `IqNode` here rather than an extracted row list, since a
/// non-Values-shaped arm must be handed back completely untouched). Declines (the
/// arm unchanged) when its own declared columns aren't EXACTLY the Union's
/// `project` (no narrowing — the outer `IqNode::Union` dispatch in
/// `normalize_distinct` documents why), when `dedup_rows` itself declines (a
/// non-Const cell), or when nothing was actually duplicated.
fn dedup_one_arm(arm: IqNode, project: &[Var]) -> IqNode {
    match arm {
        IqNode::Values { vars, rows } if vars.as_slice() == project => {
            let n = rows.len();
            match dedup_rows(rows) {
                Ok(deduped) if deduped.len() < n => IqNode::Values {
                    vars,
                    rows: deduped,
                },
                Ok(rows) | Err(rows) => IqNode::Values { vars, rows },
            }
        }
        IqNode::Construction {
            child,
            subst,
            project: arm_project,
        } if subst.is_empty()
            && arm_project.as_slice() == project
            && matches!(&*child, IqNode::Values { vars, .. } if vars.as_slice() == project) =>
        {
            let IqNode::Values { vars, rows } = *child else {
                unreachable!("matched above")
            };
            let n = rows.len();
            let deduped_child = match dedup_rows(rows) {
                Ok(deduped) if deduped.len() < n => IqNode::Values {
                    vars,
                    rows: deduped,
                },
                Ok(rows) | Err(rows) => IqNode::Values { vars, rows },
            };
            IqNode::Construction {
                child: Box::new(deduped_child),
                subst,
                project: arm_project,
            }
        }
        other => other,
    }
}

/// Remove duplicate rows, keeping the first occurrence's order (SPARQL DISTINCT:
/// multiset → set). `oxrdf::Term` already derives structural `PartialEq`/`Hash`
/// (`TermDef` itself does not), so comparison goes by each cell's underlying `Term`
/// — a `None` (UNDEF) cell compares equal to another `None`. Declines (returns
/// `rows` unchanged — still correct, `Distinct`/`SELECT DISTINCT` still runs at
/// LOWER as before) the moment any cell isn't a plain `Const`: a `Concat`/`Coalesce`/
/// `Agg`/`Derived` `TermDef` has no reconstruction-time-only comparable form here.
fn dedup_rows(rows: ValuesRows) -> std::result::Result<ValuesRows, ValuesRows> {
    let comparable = rows
        .iter()
        .flatten()
        .all(|cell| matches!(cell, None | Some(TermDef::Const(_))));
    if !comparable {
        return Err(rows);
    }

    let mut rows = rows;
    let mut unique_prefix = 0;
    while unique_prefix < rows.len() {
        let duplicate = rows[..unique_prefix]
            .iter()
            .any(|seen| const_rows_equal(seen, &rows[unique_prefix]));
        if duplicate {
            rows.remove(unique_prefix);
        } else {
            unique_prefix += 1;
        }
    }
    Ok(rows)
}

fn const_rows_equal(left: &[Option<TermDef>], right: &[Option<TermDef>]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| match (left, right) {
                (None, None) => true,
                (Some(TermDef::Const(left)), Some(TermDef::Const(right))) => left == right,
                _ => false,
            })
}

// ---- condition normalization (descend into EXISTS / NOT EXISTS payloads) ----------

/// Normalize a conjunction of [`IqCond`]s (design-lock §3 recursion clause). The
/// symbolic `Expr`/`Sql` leaves pass through untouched (FILTER/ON stays symbolic until
/// LOWER); the `Exists`/`NotExists` subtrees are normalized as first-class `IqNode`s.
fn normalize_conds(conds: Vec<IqCond>, work_mode: CompilerWorkMode<'_>) -> Result<Vec<IqCond>> {
    conds
        .into_iter()
        .map(|cond| normalize_cond(cond, work_mode))
        .collect()
}

/// Normalize one [`IqCond`], descending into the built `IqNode` of an
/// `Exists`/`NotExists` payload (and through the boolean combinators).
fn normalize_cond(cond: IqCond, work_mode: CompilerWorkMode<'_>) -> Result<IqCond> {
    match cond {
        IqCond::Expr(e) => Ok(IqCond::Expr(e)),
        IqCond::Sql(s) => Ok(IqCond::Sql(s)),
        IqCond::And(cs) => Ok(IqCond::And(normalize_conds(cs, work_mode)?)),
        IqCond::Or(cs) => Ok(IqCond::Or(normalize_conds(cs, work_mode)?)),
        IqCond::Not(c) => Ok(IqCond::Not(Box::new(normalize_cond(*c, work_mode)?))),
        IqCond::Exists(n) => Ok(IqCond::Exists(Box::new(normalize_with_work_mode(
            *n, work_mode,
        )?))),
        IqCond::NotExists { inner, is_minus } => Ok(IqCond::NotExists {
            inner: Box::new(normalize_with_work_mode(*inner, work_mode)?),
            is_minus,
        }),
    }
}

/// The de-duplicated union of the children's output scopes (stable order) — the var
/// signature an `InnerJoin`/`Empty`/distributed-`Union` publishes.
fn union_vars(nodes: &[IqNode]) -> Vec<Var> {
    let mut out: Vec<Var> = Vec::new();
    for n in nodes {
        for v in n.output_vars() {
            if !out.contains(&v) {
                out.push(v);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
