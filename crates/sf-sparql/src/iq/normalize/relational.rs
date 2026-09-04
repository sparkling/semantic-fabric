use std::collections::BTreeMap;

use super::normalize_with_work_mode;
use super::unions::normalize_union;
use crate::iq::node::{BindDef, IqCond, IqNode, Var};
use crate::unify::{unify, Unify};
use crate::{CompilerWorkMode, Error, Result};

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
pub(super) fn normalize_inner_join(
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
pub(super) fn normalize_filter(
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
pub(super) fn normalize_left_join(
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
