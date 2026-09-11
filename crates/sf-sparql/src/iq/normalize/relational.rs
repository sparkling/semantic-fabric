use std::collections::BTreeMap;

use super::bindings::merge_into;
use super::conditions::normalize_conds;
use super::control::{RowVec, RowWork};
use super::unions::normalize_union;
use crate::iq::node::{BindDef, IqCond, IqNode, Var};
use crate::Result;

// ---- (b) + (a) inner join: distribute over Union, else lift to one Construction ---

/// Normalize an `InnerJoin` whose `children` are already normalized (design §4(b) +
/// §4(c) + §4(a), in that order). Empty-absorbs, drops condition-free `True` identity
/// children, distributes over a `Union` child (recursing per arm), then — once no
/// `Union` child remains — lifts the children's `Construction`s into a single
/// `Construction` over the flattened `InnerJoin` of leaves.
pub(super) fn normalize_inner_join(
    children: Vec<IqNode>,
    cond: Vec<IqCond>,
    work: RowWork<'_>,
) -> Result<IqNode> {
    work.charge(1)?;
    let join_vars = union_vars(&children, work)?;

    // (c) absorbing: any Empty child ⇒ the whole join is Empty over all its vars.
    for child in &children {
        work.charge(1)?;
        if matches!(child, IqNode::Empty { .. }) {
            return Ok(IqNode::Empty { vars: join_vars });
        }
    }

    // (c) identity: drop the condition-free `True` element (the empty tuple).
    let mut retained = work.vector(children.len())?;
    for child in children {
        work.charge(1)?;
        if !matches!(child, IqNode::True) {
            retained.push(child);
        }
    }
    let mut children = retained;
    if children.is_empty() {
        // §4.13: `True` is the InnerJoin identity for a CONDITION-FREE join only. A
        // join of only `True` children with a residual `cond` is NOT vacuous (the cond
        // may be ground-false, e.g. a constant-position `IqCond::Sql`); it is preserved
        // as a `Filter` over the empty tuple, never silently dropped.
        return if cond.is_empty() {
            Ok(IqNode::True)
        } else {
            Ok(IqNode::Filter {
                child: work.boxed(IqNode::True)?,
                cond: normalize_conds(cond, work)?,
            })
        };
    }
    if children.len() == 1 && cond.is_empty() {
        return Ok(children.pop().expect("len checked == 1"));
    }

    // (b) distribute over the FIRST Union child (either operand): re-normalize each
    // resulting per-arm InnerJoin, then re-form the Union above the join.
    let mut union_at = None;
    for (i, child) in children.iter().enumerate() {
        work.charge(1)?;
        if matches!(child, IqNode::Union { .. }) {
            union_at = Some(i);
            break;
        }
    }
    if let Some(i) = union_at {
        work.moved::<IqNode>(children.len() - i - 1)?;
        let IqNode::Union {
            children: mut arms, ..
        } = children.remove(i)
        else {
            unreachable!("position matched a Union");
        };
        let mut out_arms = work.vector(arms.len())?;
        let last = arms.pop();
        for arm in arms {
            work.charge(1)?;
            let mut nc = RowVec::new(work.mode.clone_iq_nodes(&children)?);
            work.insert(&mut nc, i, arm)?;
            let arm_cond = work.mode.clone_iq_conditions(&cond)?;
            out_arms.push(normalize_inner_join(
                nc.into_inner(),
                arm_cond,
                work.enter()?,
            )?);
        }
        if let Some(arm) = last {
            // The final distributed arm can consume both fixed operands and the
            // condition; all earlier arms still require independent copies.
            work.charge(1)?;
            let mut children = RowVec::new(children);
            work.insert(&mut children, i, arm)?;
            out_arms.push(normalize_inner_join(
                children.into_inner(),
                cond,
                work.enter()?,
            )?);
        }
        return normalize_union(out_arms, join_vars, work.mode);
    }

    // (a) no Union child: lift the children's Constructions to one Construction.
    lift_inner_join(children, cond, join_vars, work)
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
    work: RowWork<'_>,
) -> Result<IqNode> {
    work.charge(1)?;
    let mut acc: BTreeMap<Var, BindDef> = BTreeMap::new();
    let mut body_children = RowVec::new(Vec::new());
    let mut body_cond = RowVec::new(normalize_conds(cond, work)?);
    let mut had_construction = false;

    for child in children {
        work.charge(1)?;
        match child {
            IqNode::Construction {
                child: cbody,
                subst,
                ..
            } => {
                had_construction = true;
                if merge_into(&mut acc, subst, &mut body_cond, work)? {
                    return Ok(IqNode::Empty { vars: join_vars });
                }
                // Splice a nested InnerJoin body (associativity); else add it whole.
                match *cbody {
                    IqNode::InnerJoin {
                        children: gc,
                        cond: gcond,
                    } => {
                        work.append(&mut body_children, gc)?;
                        work.append(&mut body_cond, gcond)?;
                    }
                    other => work.push(&mut body_children, other)?,
                }
            }
            // A bare (Construction-free) nested InnerJoin flattens directly.
            IqNode::InnerJoin {
                children: gc,
                cond: gcond,
            } => {
                work.append(&mut body_children, gc)?;
                work.append(&mut body_cond, gcond)?;
            }
            other => work.push(&mut body_children, other)?,
        }
    }

    let mut body_children = body_children.into_inner();
    let body_cond = body_cond.into_inner();

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
            child: work.boxed(body)?,
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
    work: RowWork<'_>,
) -> Result<IqNode> {
    work.charge(1)?;
    let cond = normalize_conds(cond, work)?;
    match child {
        IqNode::Empty { vars } => Ok(IqNode::Empty { vars }),
        IqNode::Union {
            children: mut arms,
            project,
        } => {
            let mut out = work.vector(arms.len())?;
            let last = arms.pop();
            for a in arms {
                work.charge(1)?;
                let arm_cond = work.mode.clone_iq_conditions(&cond)?;
                out.push(normalize_filter(arm_cond, a, work.enter()?)?);
            }
            if let Some(a) = last {
                work.charge(1)?;
                out.push(normalize_filter(cond, a, work.enter()?)?);
            }
            normalize_union(out, project, work.mode)
        }
        IqNode::Construction {
            child: body,
            subst,
            project,
        } => Ok(IqNode::Construction {
            child: work.boxed(push_filter(cond, *body, work)?)?,
            subst,
            project,
        }),
        IqNode::Filter {
            child: body,
            cond: inner,
        } => {
            let mut merged = RowVec::new(cond);
            work.append(&mut merged, inner)?;
            Ok(IqNode::Filter {
                child: body,
                cond: merged.into_inner(),
            })
        }
        other => Ok(IqNode::Filter {
            child: work.boxed(other)?,
            cond,
        }),
    }
}

/// Wrap `body` in a `Filter[cond]`, coalescing with an existing `Filter` (the `cond`
/// is already normalized).
fn push_filter(cond: Vec<IqCond>, body: IqNode, work: RowWork<'_>) -> Result<IqNode> {
    work.charge(1)?;
    Ok(match body {
        IqNode::Filter { child, cond: inner } => {
            let mut merged = RowVec::new(cond);
            work.append(&mut merged, inner)?;
            IqNode::Filter {
                child,
                cond: merged.into_inner(),
            }
        }
        other => IqNode::Filter {
            child: work.boxed(other)?,
            cond,
        },
    })
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
    work: RowWork<'_>,
) -> Result<IqNode> {
    work.charge(1)?;
    let cond = normalize_conds(cond, work)?;
    let combined_vars = {
        let mut v = RowVec::new(crate::build::controlled_output_vars(&left, work)?);
        for x in crate::build::controlled_output_vars(&right, work)? {
            work.unique(&mut v, x)?;
        }
        v.into_inner()
    };
    match left {
        IqNode::Empty { .. } => Ok(IqNode::Empty {
            vars: combined_vars,
        }),
        IqNode::Union {
            children: mut arms, ..
        } => {
            let mut out = work.vector(arms.len())?;
            let last = arms.pop();
            for a in arms {
                work.charge(1)?;
                let arm_right = work.mode.clone_iq_node(&right)?;
                let arm_cond = work.mode.clone_iq_conditions(&cond)?;
                out.push(normalize_left_join(a, arm_right, arm_cond, work.enter()?)?);
            }
            if let Some(a) = last {
                // LEFT JOIN is non-commutative, so retain the left-arm order and
                // transfer the original shared right/condition only to the last.
                work.charge(1)?;
                out.push(normalize_left_join(a, right, cond, work.enter()?)?);
            }
            normalize_union(out, combined_vars, work.mode)
        }
        _ => match right {
            IqNode::Empty { .. } => Ok(left),
            _ => Ok(IqNode::LeftJoin {
                left: work.boxed(left)?,
                right: work.boxed(right)?,
                cond,
            }),
        },
    }
}

/// The de-duplicated union of the children's output scopes (stable order) — the var
/// signature an `InnerJoin`/`Empty`/distributed-`Union` publishes.
fn union_vars(nodes: &[IqNode], work: RowWork<'_>) -> Result<Vec<Var>> {
    work.charge(1)?;
    let mut out = RowVec::new(Vec::new());
    for n in nodes {
        work.charge(1)?;
        for v in crate::build::controlled_output_vars(n, work)? {
            work.unique(&mut out, v)?;
        }
    }
    Ok(out.into_inner())
}
