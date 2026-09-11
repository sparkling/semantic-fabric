use std::collections::BTreeMap;

use super::unions::normalize_union;
use crate::iq::node::{BindDef, IqNode, Var};
use crate::{CompilerWorkMode, Result};

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
pub(super) fn lift_construction(
    subst: BTreeMap<Var, BindDef>,
    project: Vec<Var>,
    child: IqNode,
    work_mode: CompilerWorkMode<'_>,
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
            lift_construction(merged, project, *gchild, work_mode)
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
            // Exact fan-out schedule for A arms (A >= 2): clone `subst`, then
            // `project`, for each of the first A-1 arms; clone only `project` for
            // the final arm. The original `subst` moves into that final arm and
            // the original `project` moves into the resulting outer Union.
            let mut out = Vec::with_capacity(arms.len());
            let last = arms.pop();
            for a in arms {
                let arm_subst = work_mode.clone_iq_substitution(&subst)?;
                let arm_project = work_mode.clone_variables(&project)?;
                out.push(lift_construction(arm_subst, arm_project, a, work_mode)?);
            }
            if let Some(a) = last {
                // Every arm owns an independent substitution. Preserve order while
                // moving the original into the final arm, so only the preceding
                // fan-out arms pay for recursive copies.
                let arm_project = work_mode.clone_variables(&project)?;
                out.push(lift_construction(subst, arm_project, a, work_mode)?);
            }
            normalize_union(out, project, work_mode)
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
