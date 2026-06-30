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
//! ## Status: tree path only (NOT the live engine)
//!
//! This is M3b; it is **not** wired into the live [`Plan`](crate::Plan)/exec/unfold
//! path. The flat [`crate::unfold`] stays the production engine and the proven
//! oracle. `cargo test --workspace` must stay green with the flat path byte-for-byte
//! unchanged.
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
//! * `Filter(Union(B1..Bn)) ⇒ Union(Filter(B1)..Filter(Bn))`, **cloning the symbolic
//!   [`IqCond`] into each arm** (never a pre-lowered `SqlCond`).
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
use crate::unify::{unify, Unify};
use crate::{Error, Result};

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
    match node {
        // ---- substitution-lifting carrier (a) -----------------------------------
        IqNode::Construction {
            child,
            subst,
            project,
        } => {
            let child = normalize(*child)?;
            lift_construction(subst, project, child)
        }

        // ---- selection: distribute over Union, else push below Construction -----
        IqNode::Filter { child, cond } => {
            let child = normalize(*child)?;
            normalize_filter(cond, child)
        }

        // ---- n-ary inner join: distribute over Union, else lift Constructions ----
        IqNode::InnerJoin { children, cond } => {
            let children = children
                .into_iter()
                .map(normalize)
                .collect::<Result<Vec<_>>>()?;
            normalize_inner_join(children, cond)
        }

        // ---- left join: distribute over a LEFT Union only; right stays intact ----
        IqNode::LeftJoin { left, right, cond } => {
            let left = normalize(*left)?;
            let right = normalize(*right)?;
            normalize_left_join(left, right, cond)
        }

        // ---- bag union: flatten, prune Empty arms (NO arm-merge) ----------------
        IqNode::Union { children, project } => {
            let children = children
                .into_iter()
                .map(normalize)
                .collect::<Result<Vec<_>>>()?;
            normalize_union(children, project)
        }

        // ---- modifier spine: normalize the child, keep the node above the Union --
        IqNode::Aggregation {
            child,
            grouping,
            aggs,
        } => Ok(IqNode::Aggregation {
            child: Box::new(normalize(*child)?),
            grouping,
            aggs,
        }),
        IqNode::Distinct { child } => Ok(IqNode::Distinct {
            child: Box::new(normalize(*child)?),
        }),
        IqNode::Slice {
            child,
            offset,
            limit,
        } => Ok(IqNode::Slice {
            child: Box::new(normalize(*child)?),
            offset,
            limit,
        }),
        IqNode::OrderBy { child, keys } => Ok(IqNode::OrderBy {
            child: Box::new(normalize(*child)?),
            keys,
        }),

        // ---- leaves / identities pass through ------------------------------------
        // `Intensional` MUST already be gone (RESOLVE invariant); it is carried
        // through verbatim rather than special-cased, so a contract violation is
        // visible downstream instead of silently rewritten.
        leaf @ (IqNode::Extensional { .. }
        | IqNode::Values { .. }
        | IqNode::Path { .. }
        | IqNode::Empty { .. }
        | IqNode::True
        | IqNode::Intensional { .. }) => Ok(leaf),
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
        IqNode::Union { children: arms, .. } => {
            let mut out = Vec::with_capacity(arms.len());
            for a in arms {
                out.push(lift_construction(subst.clone(), project.clone(), a)?);
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
fn normalize_inner_join(children: Vec<IqNode>, cond: Vec<IqCond>) -> Result<IqNode> {
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
                cond: normalize_conds(cond)?,
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
        let IqNode::Union { children: arms, .. } = children.remove(i) else {
            unreachable!("position matched a Union");
        };
        let mut out_arms = Vec::with_capacity(arms.len());
        for arm in arms {
            let mut nc = children.clone();
            nc.insert(i, arm);
            out_arms.push(normalize_inner_join(nc, cond.clone())?);
        }
        return normalize_union(out_arms, join_vars);
    }

    // (a) no Union child: lift the children's Constructions to one Construction.
    lift_inner_join(children, cond, join_vars)
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
) -> Result<IqNode> {
    let mut acc: BTreeMap<Var, BindDef> = BTreeMap::new();
    let mut body_children: Vec<IqNode> = Vec::new();
    let mut body_cond: Vec<IqCond> = normalize_conds(cond)?;
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
/// Distributes over a `Union` (cloning the symbolic `cond` into each arm), and over a
/// leaf-CQ pushes the `Filter` **below the `Construction`** (the spine leaf-CQ is
/// `Construction` over a `Filter` of leaves). Adjacent `Filter`s coalesce.
fn normalize_filter(cond: Vec<IqCond>, child: IqNode) -> Result<IqNode> {
    let cond = normalize_conds(cond)?;
    match child {
        IqNode::Empty { vars } => Ok(IqNode::Empty { vars }),
        IqNode::Union {
            children: arms,
            project,
        } => {
            let mut out = Vec::with_capacity(arms.len());
            for a in arms {
                out.push(normalize_filter(cond.clone(), a)?);
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
fn normalize_left_join(left: IqNode, right: IqNode, cond: Vec<IqCond>) -> Result<IqNode> {
    let cond = normalize_conds(cond)?;
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
        IqNode::Union { children: arms, .. } => {
            let mut out = Vec::with_capacity(arms.len());
            for a in arms {
                out.push(normalize_left_join(a, right.clone(), cond.clone())?);
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
/// (the `Union` identity), and unwraps a one-arm `Union`. It performs **NO arm-merge /
/// structural dedup** — a multiplicity-bearing arm is never collapsed into a sibling.
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
        _ => IqNode::Union {
            children: arms,
            project,
        },
    })
}

// ---- condition normalization (descend into EXISTS / NOT EXISTS payloads) ----------

/// Normalize a conjunction of [`IqCond`]s (design-lock §3 recursion clause). The
/// symbolic `Expr`/`Sql` leaves pass through untouched (FILTER/ON stays symbolic until
/// LOWER); the `Exists`/`NotExists` subtrees are normalized as first-class `IqNode`s.
fn normalize_conds(conds: Vec<IqCond>) -> Result<Vec<IqCond>> {
    conds.into_iter().map(normalize_cond).collect()
}

/// Normalize one [`IqCond`], descending into the built `IqNode` of an
/// `Exists`/`NotExists` payload (and through the boolean combinators).
fn normalize_cond(cond: IqCond) -> Result<IqCond> {
    match cond {
        IqCond::Expr(e) => Ok(IqCond::Expr(e)),
        IqCond::Sql(s) => Ok(IqCond::Sql(s)),
        IqCond::And(cs) => Ok(IqCond::And(normalize_conds(cs)?)),
        IqCond::Or(cs) => Ok(IqCond::Or(normalize_conds(cs)?)),
        IqCond::Not(c) => Ok(IqCond::Not(Box::new(normalize_cond(*c)?))),
        IqCond::Exists(n) => Ok(IqCond::Exists(Box::new(normalize(*n)?))),
        IqCond::NotExists(n) => Ok(IqCond::NotExists(Box::new(normalize(*n)?))),
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
mod tests {
    use super::*;
    use crate::build::build_tree;
    use crate::iq::node::IqNode;
    use crate::iq::resolve::{resolve, ResolveCx};
    use crate::iq::{Scan, TermDef};
    use crate::saturate::Tbox;
    use sf_core::ir::{
        LogicalSource, ObjectMap, PredicateObjectMap, RefObjectMap, SubjectMap, Template, TermMap,
        TermSpec, TriplesMap,
    };
    use sf_core::NamedNode;
    use spargebra::algebra::GraphPattern;

    const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

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
    /// (refObjectMap → DEPT) — mirrors the resolve.rs fixture.
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

    /// build → resolve → normalize a query against the fixture mapping.
    fn norm(q: &str) -> IqNode {
        let maps = mapping();
        let tbox = Tbox::new();
        let mut cx = ResolveCx::new(&maps, &tbox, sf_sql::Dialect::Sqlite);
        let resolved = resolve(build_tree(&pattern(q), None).unwrap(), &mut cx).unwrap();
        normalize(resolved).unwrap()
    }

    /// Strip the outermost modifier spine (Distinct/Slice/OrderBy/Aggregation) and the
    /// top *projection* Construction, returning the body the spine sits over. A
    /// Construction is stripped ONLY when it sits over a `Union`/`LeftJoin` (a pure
    /// spine projection); a Construction over a relational body IS the single leaf-CQ
    /// (the projection folded into it) and is returned as-is.
    fn strip_spine(node: &IqNode) -> &IqNode {
        match node {
            IqNode::Distinct { child }
            | IqNode::Slice { child, .. }
            | IqNode::OrderBy { child, .. }
            | IqNode::Aggregation { child, .. } => strip_spine(child),
            IqNode::Construction { child, .. }
                if matches!(**child, IqNode::Union { .. } | IqNode::LeftJoin { .. }) =>
            {
                strip_spine(child)
            }
            other => other,
        }
    }

    /// A leaf-CQ body is a Join/LeftJoin/Filter of leaves, or a bare leaf — but NEVER
    /// a nested Construction (the single-bindings-map invariant) and NEVER a Union.
    fn is_leaf_or_join_of_leaves(node: &IqNode) -> bool {
        match node {
            IqNode::Extensional { .. } | IqNode::Values { .. } | IqNode::Path { .. } => true,
            IqNode::InnerJoin { children, .. } => children.iter().all(|c| {
                // join children are leaves, or LeftJoin/Filter sub-CQs (which keep
                // their own internal Construction), never a bare nested Construction.
                matches!(
                    c,
                    IqNode::Extensional { .. }
                        | IqNode::Values { .. }
                        | IqNode::Path { .. }
                        | IqNode::LeftJoin { .. }
                        | IqNode::Filter { .. }
                )
            }),
            IqNode::Filter { child, .. } => is_leaf_or_join_of_leaves(child),
            IqNode::LeftJoin { .. } => true,
            _ => false,
        }
    }

    /// Assert a normalized arm is a leaf-CQ: exactly ONE Construction at its root over
    /// a Join/Filter of leaves (no nested Construction, no Union below).
    fn assert_leaf_cq(arm: &IqNode) {
        let IqNode::Construction { child, .. } = arm else {
            panic!("leaf-CQ must be a Construction at its root, got {arm:?}");
        };
        assert!(
            is_leaf_or_join_of_leaves(child),
            "leaf-CQ Construction must be over a Join/Filter of leaves, got {child:?}"
        );
    }

    fn arm_count(body: &IqNode) -> usize {
        match body {
            IqNode::Union { children, .. } => children.len(),
            IqNode::Empty { .. } => 0,
            _ => 1,
        }
    }

    /// A single-pattern query normalizes to ONE leaf-CQ (no Union wrapper): a
    /// Construction over the bare Extensional scan, carrying both bound variables.
    #[test]
    fn single_pattern_is_one_leaf_cq() {
        let body = strip_spine(&norm("SELECT * WHERE { ?s <http://ex/name> ?n }")).clone();
        assert_leaf_cq(&body);
        let IqNode::Construction { child, subst, .. } = &body else {
            unreachable!()
        };
        assert!(matches!(**child, IqNode::Extensional { .. }), "{child:?}");
        assert!(
            subst.contains_key("s") && subst.contains_key("n"),
            "the single bindings map carries ?s and ?n: {subst:?}"
        );
    }

    /// A 2-triple BGP normalizes to ONE Construction over an InnerJoin of Extensional
    /// leaves; the shared-variable equality is materialised as an `IqCond::Sql` on the
    /// join (the tree form of the flat `merge`).
    #[test]
    fn bgp_join_lifts_to_one_construction_with_shared_var_equality() {
        let body = strip_spine(&norm(
            "SELECT * WHERE { ?s <http://ex/name> ?n . ?s <http://ex/dept> ?d }",
        ))
        .clone();
        assert_leaf_cq(&body);
        let IqNode::Construction { child, subst, .. } = &body else {
            unreachable!()
        };
        let IqNode::InnerJoin { children, cond } = &**child else {
            panic!("expected an InnerJoin of leaves, got {child:?}");
        };
        assert!(
            children
                .iter()
                .all(|c| matches!(c, IqNode::Extensional { .. })),
            "all join children are bare Extensional leaves: {children:?}"
        );
        assert!(
            cond.iter().any(|c| matches!(c, IqCond::Sql(_))),
            "the shared ?s equality rides the InnerJoin cond as IqCond::Sql: {cond:?}"
        );
        assert!(
            subst.contains_key("s") && subst.contains_key("n") && subst.contains_key("d"),
            "one merged bindings map carries every variable: {subst:?}"
        );
    }

    /// An InnerJoin with a multi-arm Union operand distributes to a Union of joins,
    /// each arm a lifted leaf-CQ (design §4.16, either operand).
    #[test]
    fn inner_join_distributes_over_union() {
        // `?s ?p ?o` is a multi-arm Union; joined with the single-arm `?s :name ?n`.
        let body = strip_spine(&norm(
            "SELECT * WHERE { ?s ?p ?o . ?s <http://ex/name> ?n }",
        ))
        .clone();
        let IqNode::Union { children, .. } = &body else {
            panic!("a join over a Union must distribute to a Union of joins, got {body:?}");
        };
        assert!(
            children.len() >= 2,
            "expected ≥2 distributed arms: {body:?}"
        );
        for arm in children {
            assert_leaf_cq(arm);
        }
    }

    /// A LeftJoin whose RIGHT is a multi-arm Union STAYS a single LeftJoin — it is
    /// NEVER split over the non-preserved side (ledger R2 / design §4.16).
    #[test]
    fn left_join_over_right_union_stays_single_left_join() {
        let body = strip_spine(&norm(
            "SELECT * WHERE { ?s <http://ex/name> ?n OPTIONAL { ?s ?p ?o } }",
        ))
        .clone();
        let IqNode::LeftJoin { right, .. } = &body else {
            panic!("a LeftJoin with a Union right must stay a single LeftJoin, got {body:?}");
        };
        assert!(
            matches!(**right, IqNode::Union { .. }),
            "the right Union is preserved (not distributed): {right:?}"
        );
    }

    /// A LeftJoin whose LEFT is a multi-arm Union distributes over the preserved side:
    /// `(A∪B)⟕C ⇒ (A⟕C)∪(B⟕C)`.
    #[test]
    fn left_join_over_left_union_distributes() {
        let body = strip_spine(&norm(
            "SELECT * WHERE { ?s ?p ?o OPTIONAL { ?s <http://ex/name> ?n } }",
        ))
        .clone();
        let IqNode::Union { children, .. } = &body else {
            panic!("a LeftJoin over a LEFT Union must distribute to a Union, got {body:?}");
        };
        // Every distributed arm is a leaf-CQ whose relational body is a LeftJoin: the
        // outermost SELECT projection is pushed into the arms (so the Union surfaces to
        // the spine top), giving each arm a canonical `Construction` over its `LeftJoin`.
        for arm in children {
            assert_leaf_cq(arm);
            let IqNode::Construction { child, .. } = arm else {
                unreachable!("assert_leaf_cq guarantees a Construction root");
            };
            assert!(
                matches!(**child, IqNode::LeftJoin { .. }),
                "each distributed arm's body is a LeftJoin: {child:?}"
            );
        }
    }

    /// An `Empty` Union arm (an unmapped predicate) is pruned, keeping the surviving
    /// arms — never collapsed/merged and never silently dropping the others.
    #[test]
    fn empty_union_arm_is_pruned() {
        let body = strip_spine(&norm(
            "SELECT * WHERE { { ?s <http://ex/name> ?n } UNION { ?s <http://ex/nope> ?o } \
             UNION { ?s <http://ex/dname> ?d } }",
        ))
        .clone();
        assert_eq!(arm_count(&body), 2, "the unmapped arm is pruned: {body:?}");
        if let IqNode::Union { children, .. } = &body {
            for arm in children {
                assert!(
                    !matches!(arm, IqNode::Empty { .. }),
                    "no Empty arm survives: {arm:?}"
                );
            }
        }
    }

    /// `rdf:type ?c` still resolves and normalizes to a leaf-CQ spine (the two class
    /// atoms), exercising the class-atom path through NORMALIZE.
    #[test]
    fn class_atom_pattern_normalizes_to_spine() {
        let body = strip_spine(&norm(&format!("SELECT * WHERE {{ ?s <{RDF_TYPE}> ?c }}"))).clone();
        match &body {
            IqNode::Union { children, .. } => {
                for arm in children {
                    assert_leaf_cq(arm);
                }
            }
            other => assert_leaf_cq(other),
        }
    }

    // ---- synthetic R2 test: shared var UNDEF in one union arm = free dimension -----

    fn col_def(col: &str, alias: usize) -> BindDef {
        BindDef::Resolved(TermDef::Derived {
            term_map: TermMap::Column(col.into(), TermSpec::plain_literal()),
            alias,
        })
    }

    /// An arm `Construction` binding the given `(var, column, alias)` triples over a
    /// bare Extensional scan.
    fn synth_arm(binds: &[(&str, &str, usize)], scan_alias: usize) -> IqNode {
        let mut subst = BTreeMap::new();
        let mut project: Vec<Var> = Vec::new();
        for (v, col, a) in binds {
            subst.insert((*v).into(), col_def(col, *a));
            project.push((*v).into());
        }
        IqNode::Construction {
            child: Box::new(IqNode::Extensional {
                scan: Scan {
                    alias: scan_alias,
                    source: LogicalSource::Table("t".to_owned()),
                },
                bind: BTreeMap::new(),
            }),
            subst,
            project,
        }
    }

    /// R2 (=_bag-CRITICAL): when an InnerJoin distributes into a Union whose arms bind
    /// a shared variable differently, the arm that BINDS the shared var seeds an
    /// equality; the arm that leaves it UNDEF/absent degenerates to a free dimension
    /// (NO equality, rebound from the bound side) — exactly the flat `merge`
    /// (`unfold.rs:1197-1199`).
    #[test]
    fn shared_var_undef_in_union_arm_is_a_free_dimension() {
        // A binds ?x,?y on alias 0; B1 binds ?x,?z on alias 1 (?x SHARED with A);
        // B2 binds only ?z on alias 2 (?x ABSENT — UNDEF in this arm).
        let a = synth_arm(&[("x", "x", 0), ("y", "y", 0)], 0);
        let b1 = synth_arm(&[("x", "x", 1), ("z", "z", 1)], 1);
        let b2 = synth_arm(&[("z", "z", 2)], 2);
        let tree = IqNode::InnerJoin {
            children: vec![
                a,
                IqNode::Union {
                    children: vec![b1, b2],
                    project: vec!["x".into(), "z".into()],
                },
            ],
            cond: vec![],
        };
        let out = normalize(tree).unwrap();
        let IqNode::Union { children, .. } = &out else {
            panic!("the join over a Union must distribute, got {out:?}");
        };
        assert_eq!(children.len(), 2, "two distributed arms: {out:?}");

        // Count how many distributed arms carry a shared-var equality (IqCond::Sql).
        let arms_with_eq = children
            .iter()
            .filter(|arm| {
                let IqNode::Construction { child, .. } = arm else {
                    return false;
                };
                matches!(&**child, IqNode::InnerJoin { cond, .. }
                    if cond.iter().any(|c| matches!(c, IqCond::Sql(_))))
            })
            .count();
        assert_eq!(
            arms_with_eq, 1,
            "exactly the ?x-binding arm seeds an equality; the UNDEF arm is a free \
             dimension with NO equality: {out:?}"
        );

        // The free-dimension arm still binds ?x (rebound from the bound side A) and
        // its InnerJoin carries no equality condition.
        let free = children
            .iter()
            .find(|arm| {
                let IqNode::Construction { child, .. } = arm else {
                    return false;
                };
                matches!(&**child, IqNode::InnerJoin { cond, .. } if cond.is_empty())
            })
            .expect("a free-dimension arm with an empty join cond");
        let IqNode::Construction { subst, .. } = free else {
            unreachable!()
        };
        assert!(
            subst.contains_key("x"),
            "?x is rebound from the bound operand in the free-dimension arm: {subst:?}"
        );
    }

    /// §4.13 (refute-fix): an `InnerJoin` whose only data children are `True` (the empty
    /// tuple) but which carries a residual `cond` is NOT the condition-free identity —
    /// the cond MUST survive (here a ground `IqCond::Sql`), preserved as a `Filter` over
    /// `True`, never silently collapsed to `True`.
    #[test]
    fn all_true_join_with_residual_cond_keeps_the_cond() {
        use crate::iq::{ColRef, SqlCond};
        let cond = vec![IqCond::Sql(SqlCond::IsNotNull(ColRef::new(0, "c")))];
        let tree = IqNode::InnerJoin {
            children: vec![IqNode::True, IqNode::True],
            cond: cond.clone(),
        };
        let out = normalize(tree).unwrap();
        match out {
            IqNode::Filter { child, cond: kept } => {
                assert!(matches!(*child, IqNode::True), "cond preserved over True");
                assert_eq!(kept.len(), 1, "the residual cond is not dropped: {kept:?}");
            }
            other => panic!(
                "a residual cond over only-True children must NOT collapse to True; got {other:?}"
            ),
        }
    }

    /// A disjoint shared-variable unification prunes the join to `Empty` (the flat
    /// `merge` `Unify::Empty ⇒ None`), and the surviving Union arms remain — never a
    /// silent row drop.
    #[test]
    fn disjoint_shared_var_prunes_the_arm() {
        // A binds ?x to an IRI-template; B binds ?x to a literal column ⇒ disjoint.
        let a = synth_arm(&[("x", "x", 0)], 0);
        let b = IqNode::Construction {
            child: Box::new(IqNode::Extensional {
                scan: Scan {
                    alias: 1,
                    source: LogicalSource::Table("t".to_owned()),
                },
                bind: BTreeMap::new(),
            }),
            subst: {
                let mut m = BTreeMap::new();
                m.insert(
                    "x".into(),
                    BindDef::Resolved(TermDef::Derived {
                        term_map: template_iri("http://ex/{x}"),
                        alias: 1,
                    }),
                );
                m
            },
            project: vec!["x".into()],
        };
        // ?x is a plain literal column on the left and an IRI template on the right →
        // unify proves disjoint (IRI can never equal a literal).
        let tree = IqNode::InnerJoin {
            children: vec![a, b],
            cond: vec![],
        };
        let out = normalize(tree).unwrap();
        assert!(
            matches!(out, IqNode::Empty { .. }),
            "a disjoint shared variable prunes the join to Empty: {out:?}"
        );
    }

    /// A `Union` is in `under_join` position iff it is a direct operand of an
    /// `InnerJoin`/`LeftJoin` (i.e. trapped, not lifted to the spine top).
    fn union_trapped(n: &IqNode, under_join: bool) -> bool {
        match n {
            IqNode::Union { children, .. } => {
                under_join || children.iter().any(|c| union_trapped(c, false))
            }
            IqNode::InnerJoin { children, .. } => children.iter().any(|c| union_trapped(c, true)),
            IqNode::Construction { child, .. }
            | IqNode::Distinct { child }
            | IqNode::Slice { child, .. }
            | IqNode::OrderBy { child, .. }
            | IqNode::Aggregation { child, .. }
            | IqNode::Filter { child, .. } => union_trapped(child, under_join),
            IqNode::LeftJoin { left, right, .. } => {
                union_trapped(left, true) || union_trapped(right, true)
            }
            _ => false,
        }
    }

    /// =_bag-CRITICAL (R2 / spine): a pure-projection subquery over a multi-arm `Union`
    /// (`{ SELECT ?s { ?s ?p ?o } }`) joined with another pattern MUST distribute — the
    /// `Union` is lifted to the spine top, never left trapped as an opaque
    /// `Construction{empty subst, Union}` inside the `InnerJoin` body (which would never
    /// cross-product the union arms with the join's other operand).
    #[test]
    fn projection_over_union_does_not_trap_union_under_join() {
        let out =
            norm("SELECT * WHERE { { SELECT ?s WHERE { ?s ?p ?o } } ?s <http://ex/name> ?n }");
        assert!(
            !union_trapped(&out, false),
            "a projection over a Union must distribute to the spine top, not trap the \
             Union inside the join body: {out:?}"
        );
        let body = strip_spine(&out).clone();
        let IqNode::Union { children, .. } = &body else {
            panic!("the join over the subquery Union must distribute to a Union, got {body:?}");
        };
        for arm in children {
            assert_leaf_cq(arm);
        }
    }
}
