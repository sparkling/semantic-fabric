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
//! [`merge`](crate::unfold) does it** (the join-substitution merge mirrors
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
//!   [`crate::iq::node::IqCond`] only for preceding arms and moving it into the final
//!   arm (never a pre-lowered `SqlCond`).
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

mod bindings;
mod conditions;
mod construction;
mod control;
mod distinct;
mod relational;
mod slice;
mod unions;
mod values;

use crate::iq::node::IqNode;
use crate::{CompilerWorkMode, Result};

/// Normalize a whole RESOLVED tree to the leaf-CQ spine (design §4). Walks `node`
/// bottom-up: each child is normalized first, then the node-local rewrite (fold /
/// lift / distribute / prune) is applied, re-normalizing the structure a distribution
/// produces. Returns a `Union`-of-(`Construction` over a `Join`/`LeftJoin`/`Filter` of
/// leaves) under the query-modifier spine, every `Union` arm carrying at most one
/// bindings map.
///
/// FILTER/BIND stay **symbolic** ([`crate::iq::node::IqCond::Expr`] /
/// [`crate::iq::node::BindDef::Expr`]) — NORMALIZE only moves and clones them; they
/// are resolved per-leaf-CQ at LOWER (M3c). The normalizer **descends into**
/// `EXISTS`/`NOT EXISTS` payloads ([`crate::iq::node::IqCond::Exists`] /
/// [`crate::iq::node::IqCond::NotExists`]) and normalizes them as first-class
/// `IqNode`s (design-lock §3 recursion clause, BINDING).
pub fn normalize(node: IqNode) -> Result<IqNode> {
    normalize_with_work_mode(node, CompilerWorkMode::Uncontrolled)
}

/// Normalize a resolved tree using the request's shared work/cancellation state.
/// This controls NORMALIZE operations, not parsing, resolution, later phases,
/// physical allocator behavior or destruction; it does not admit a profile.
pub fn normalize_with_work_control(
    node: IqNode,
    control: &dyn sf_core::query_control::QueryControl,
) -> Result<IqNode> {
    normalize_with_work_mode(
        node,
        CompilerWorkMode::Metered(crate::compiler_control::CompileContext::new(control)),
    )
}

/// Internal normalization entry point carrying the private compiler-work seam.
/// Existing raw callers use [`normalize`] and remain uncontrolled.
pub(crate) fn normalize_with_work_mode(
    node: IqNode,
    work_mode: CompilerWorkMode<'_>,
) -> Result<IqNode> {
    normalize_node(node, control::RowWork::new(work_mode))
}

fn normalize_node(node: IqNode, work: control::RowWork<'_>) -> Result<IqNode> {
    let work = work.enter()?;
    let result = normalize_dispatch(node, work);
    work.checkpoint()?;
    result
}

#[inline(never)]
fn normalize_dispatch(node: IqNode, work: control::RowWork<'_>) -> Result<IqNode> {
    match node {
        // ---- substitution-lifting carrier (a) -----------------------------------
        IqNode::Construction {
            child,
            subst,
            project,
        } => {
            let child = normalize_node(*child, work)?;
            construction::lift_construction(subst, project, child, work)
        }

        // ---- selection: distribute over Union, else push below Construction -----
        IqNode::Filter { child, cond } => {
            let child = normalize_node(*child, work)?;
            relational::normalize_filter(cond, child, work)
        }

        // ---- n-ary inner join: distribute over Union, else lift Constructions ----
        IqNode::InnerJoin { children, cond } => {
            let children = normalize_children(children, work)?;
            relational::normalize_inner_join(children, cond, work)
        }

        // ---- left join: distribute over a LEFT Union only; right stays intact ----
        IqNode::LeftJoin { left, right, cond } => {
            normalize_left_children(*left, *right, cond, work)
        }

        // ---- bag union: flatten, prune Empty arms (NO arm-merge) ----------------
        IqNode::Union { children, project } => {
            let out = normalize_children(children, work)?;
            unions::normalize_union(out, project, work.mode)
        }

        // ---- modifier spine: normalize the child, keep the node above the Union --
        IqNode::Aggregation {
            child,
            grouping,
            aggs,
        } => Ok(IqNode::Aggregation {
            child: work.boxed(normalize_node(*child, work)?)?,
            grouping,
            aggs,
        }),
        IqNode::Distinct { child } => {
            let child = normalize_node(*child, work)?;
            distinct::normalize_distinct(child, work.mode)
        }
        IqNode::Slice {
            child,
            offset,
            limit,
        } => {
            let child = normalize_node(*child, work)?;
            slice::normalize_slice(offset, limit, child, work.mode)
        }
        IqNode::OrderBy { child, keys } => Ok(IqNode::OrderBy {
            child: work.boxed(normalize_node(*child, work)?)?,
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

#[inline(never)]
fn normalize_children(children: Vec<IqNode>, work: control::RowWork<'_>) -> Result<Vec<IqNode>> {
    let mut out = work.vector(children.len())?;
    for child in children {
        work.charge(1)?;
        out.push(normalize_node(child, work)?);
    }
    Ok(out)
}

#[inline(never)]
fn normalize_left_children(
    left: IqNode,
    right: IqNode,
    cond: Vec<crate::iq::node::IqCond>,
    work: control::RowWork<'_>,
) -> Result<IqNode> {
    let left = normalize_node(left, work)?;
    let right = normalize_node(right, work)?;
    relational::normalize_left_join(left, right, cond, work)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod control_tests;

#[cfg(test)]
mod constant_control_tests;

#[cfg(test)]
mod structural_control_tests;

#[cfg(test)]
mod binding_control_tests;
#[cfg(test)]
mod condition_control_tests;
#[cfg(test)]
mod structural_test_support;
