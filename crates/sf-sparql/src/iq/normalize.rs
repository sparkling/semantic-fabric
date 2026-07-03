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
use crate::iq::TermDef;
use crate::unify::{bind_term_def, unify, Unify};
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
        IqNode::Distinct { child } => {
            let child = normalize(*child)?;
            Ok(normalize_distinct(child))
        }
        IqNode::Slice {
            child,
            offset,
            limit,
        } => {
            let child = normalize(*child)?;
            Ok(normalize_slice(offset, limit, child))
        }
        IqNode::OrderBy { child, keys } => Ok(IqNode::OrderBy {
            child: Box::new(normalize(*child)?),
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
/// (the `Union` identity), and unwraps a one-arm `Union`. Beyond that it performs
/// **NO multiplicity-losing arm-merge / dedup** — a multiplicity-bearing arm is never
/// collapsed into a sibling — with exactly one exception: [`try_fold_constant_union`]
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
        _ => match try_fold_constant_union(&arms, &project) {
            Some(values) => values,
            None => match try_partial_fold_constant_union(&arms, &project) {
                Some(union) => union,
                None => IqNode::Union {
                    children: arms,
                    project,
                },
            },
        },
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
/// Declines (`None`, leaving the `Union` as-is) on the first arm that isn't in this
/// exact shape: a binding that isn't a compile-time constant, or a `Construction`
/// over a real pattern (a DATA arm — test15's "const arms fold, data arm kept"
/// needs a partial fold this rule does not yet attempt). Never a partial/best-effort
/// fold: any non-qualifying arm aborts the whole attempt, matching this codebase's
/// sound-no-op-on-precondition-failure convention elsewhere (e.g.
/// `try_sql_group_over_union`).
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
fn try_fold_constant_union(arms: &[IqNode], project: &[Var]) -> Option<IqNode> {
    if arms.len() < 2 {
        return None;
    }
    let mut rows = Vec::with_capacity(arms.len());
    for arm in arms {
        rows.extend(const_rows_of(arm, project)?);
    }
    Some(IqNode::Values {
        vars: project.to_vec(),
        rows,
    })
}

/// Extract the constant row(s) this ARM alone would contribute to a folded
/// `Values` leaf projecting `project` — `None` if this arm isn't one of the three
/// constant-arm shapes `try_fold_constant_union`/`try_partial_fold_constant_union`
/// recognize (a DATA arm, a real pattern underneath). Shared by both callers so
/// the fold precondition is defined in exactly one place.
fn const_rows_of(arm: &IqNode, project: &[Var]) -> Option<Vec<Vec<Option<TermDef>>>> {
    let no_vars = BTreeMap::new();
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
        IqNode::Values { vars, rows } if same_var_set(vars, project) => {
            Some(rows.iter().map(|r| reorder_row(vars, r, project)).collect())
        }
        IqNode::Construction { child, subst, .. } if matches!(**child, IqNode::True) => {
            let mut row = Vec::with_capacity(project.len());
            for var in project {
                let cell = match subst.get(var) {
                    Some(BindDef::Resolved(TermDef::Const(t))) => Some(TermDef::Const(t.clone())),
                    Some(BindDef::Expr(e)) => Some(bind_term_def(e, &no_vars).ok()?),
                    Some(_) => return None, // a resolved non-constant (e.g. a column)
                    None => None,           // unbound in this arm -- UNDEF (Values semantics)
                };
                row.push(cell);
            }
            Some(vec![row])
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

/// Partial version of `try_fold_constant_union` (Ontop
/// `ValuesNodeOptimization::test15ConstructionUnionTrueTrueDataNode`): when SOME
/// (but not all) of a `Union`'s arms are constant, fold JUST those into one
/// `Values` arm, keeping the rest (the DATA arms — real patterns underneath)
/// untouched as sibling `Union` arms. The SAME `=_bag` multiset either way (a
/// `Union` distributes row-membership independently of how its arms are grouped);
/// only the SQL shape changes — fewer arms to plan/execute.
///
/// Declines (`None`) when fewer than 2 arms are constant (nothing worth
/// combining — a single constant arm gains nothing from being wrapped in its own
/// one-arm `Values`, and folding a single ALREADY-a-`Values` arm into itself is a
/// pure no-op) or when EVERY arm is constant (`try_fold_constant_union`, tried
/// first by the caller, already produces a strictly simpler bare `Values` for
/// that case — this function only ever runs after that one has declined).
fn try_partial_fold_constant_union(arms: &[IqNode], project: &[Var]) -> Option<IqNode> {
    let mut rows = Vec::new();
    let mut kept = Vec::new();
    let mut const_arm_count = 0;
    for arm in arms {
        match const_rows_of(arm, project) {
            Some(r) => {
                rows.extend(r);
                const_arm_count += 1;
            }
            None => kept.push(arm.clone()),
        }
    }
    if const_arm_count < 2 || kept.is_empty() {
        return None;
    }
    let mut children = Vec::with_capacity(kept.len() + 1);
    children.push(IqNode::Values {
        vars: project.to_vec(),
        rows,
    });
    children.extend(kept);
    Some(IqNode::Union {
        children,
        project: project.to_vec(),
    })
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
            match try_slice_over_union(offset, limit, &children, &project) {
                Some(node) => node,
                None => IqNode::Slice {
                    child: Box::new(IqNode::Union { children, project }),
                    offset,
                    limit,
                },
            }
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
/// (the same two shapes [`try_fold_constant_union`] recognizes for a WHOLE `Union`,
/// generalized here to one arm at a time — a mixed `Union` with a genuine DATA arm
/// declines `try_fold_constant_union` entirely, so an all-constant arm can still
/// reach this function unfolded). `None` ⇒ unknown cardinality (a real pattern, or
/// a column-order mismatch this rule doesn't reconcile — cf. test26).
fn static_rows_of(arm: &IqNode, project: &[Var]) -> Option<Vec<Vec<Option<TermDef>>>> {
    if let IqNode::Values { vars, rows } = arm {
        return (vars.as_slice() == project).then(|| rows.clone());
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
            Some(rows.clone())
        }
        IqNode::True => {
            let no_vars = BTreeMap::new();
            let mut row = Vec::with_capacity(project.len());
            for var in project {
                let cell = match subst.get(var) {
                    Some(BindDef::Resolved(TermDef::Const(t))) => Some(TermDef::Const(t.clone())),
                    Some(BindDef::Expr(e)) => Some(bind_term_def(e, &no_vars).ok()?),
                    Some(_) => return None,
                    None => None,
                };
                row.push(cell);
            }
            Some(vec![row])
        }
        _ => None,
    }
}

/// Resolve a `Slice(offset, limit)` over a `Union` whose LEADING arms have a
/// statically-known row count ([`static_rows_of`]) by walking them in as-written
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
/// Returns `None` (decline, keep `Slice{Union}` as-is) when nothing at all could be
/// dropped or truncated — e.g. the very first arm is already unknown and the window
/// isn't yet satisfied — matching every other rule's sound-decline convention.
fn try_slice_over_union(
    offset: usize,
    limit: Option<usize>,
    arms: &[IqNode],
    project: &[Var],
) -> Option<IqNode> {
    let limit_n = limit.unwrap_or(usize::MAX);
    let window_end = offset.saturating_add(limit_n);
    let mut cursor = 0usize;
    let mut survivors: Vec<Vec<Option<TermDef>>> = Vec::new();
    let mut i = 0;
    let mut changed = false;

    while i < arms.len() {
        if survivors.len() >= limit_n {
            // Everything from here on (including arms[i] itself, not yet processed)
            // is unreachable -- drop the whole remaining tail, not just what's
            // already been looked at.
            changed = true;
            i = arms.len();
            break;
        }
        match static_rows_of(&arms[i], project) {
            Some(rows) => {
                let n = rows.len();
                let local_start = offset.saturating_sub(cursor).min(n);
                let local_end = window_end.saturating_sub(cursor).min(n);
                if local_start > 0 || local_end < n {
                    changed = true; // this arm got truncated (partly or fully outside the window)
                }
                if local_start < local_end {
                    survivors.extend(rows[local_start..local_end].iter().cloned());
                }
                cursor += n;
                i += 1;
            }
            None => break, // unknown cardinality -- stop, keep this arm + the rest
        }
    }

    if !changed {
        return None;
    }

    let remaining = &arms[i..];
    if remaining.is_empty() {
        Some(IqNode::Values {
            vars: project.to_vec(),
            rows: survivors,
        })
    } else {
        let mut new_arms = Vec::new();
        if !survivors.is_empty() {
            new_arms.push(IqNode::Values {
                vars: project.to_vec(),
                rows: survivors,
            });
        }
        new_arms.extend(remaining.iter().cloned());
        let child = if new_arms.len() == 1 {
            new_arms.pop().expect("len checked == 1")
        } else {
            IqNode::Union {
                children: new_arms,
                project: project.to_vec(),
            }
        };
        Some(IqNode::Slice {
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
        })
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
        IqNode::Values { vars, rows } => match dedup_rows(&rows) {
            Some(deduped) => IqNode::Values {
                vars,
                rows: deduped,
            },
            None => IqNode::Distinct {
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
    row: &[Option<TermDef>],
    to_order: &[Var],
) -> Vec<Option<TermDef>> {
    to_order
        .iter()
        .map(|v| {
            let i = from_order
                .iter()
                .position(|w| w == v)
                .expect("same_var_set checked by the caller");
            row[i].clone()
        })
        .collect()
}

/// Dedup ONE `Union` arm's own internal duplicate rows (a bare `Values` leaf, or a
/// lone `VALUES`-as-union-arm wrapped in the builder's identity-projection
/// `Construction` — the same shape `static_rows_of` recognizes, but returning the
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
            match dedup_rows(&rows) {
                Some(deduped) if deduped.len() < n => IqNode::Values {
                    vars,
                    rows: deduped,
                },
                _ => IqNode::Values { vars, rows },
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
            let deduped_child = match dedup_rows(&rows) {
                Some(deduped) if deduped.len() < n => IqNode::Values {
                    vars,
                    rows: deduped,
                },
                _ => IqNode::Values { vars, rows },
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
fn dedup_rows(rows: &[Vec<Option<TermDef>>]) -> Option<Vec<Vec<Option<TermDef>>>> {
    let keys: Option<Vec<Vec<Option<&sf_core::Term>>>> = rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| match cell {
                    None => Some(None),
                    Some(TermDef::Const(t)) => Some(Some(t)),
                    Some(_) => None,
                })
                .collect()
        })
        .collect();
    let keys = keys?; // some cell isn't a plain constant -- decline (caller keeps Distinct)
    let mut seen: Vec<&Vec<Option<&sf_core::Term>>> = Vec::new();
    let mut out = Vec::new();
    for (row, key) in rows.iter().zip(keys.iter()) {
        if !seen.contains(&key) {
            seen.push(key);
            out.push(row.clone());
        }
    }
    Some(out)
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
    /// Construction over the Extensional scan, carrying both bound variables. The
    /// column-valued object `?n` rides an R2RML §11 NULL guard (`IsNotNull`) on the
    /// scan's condition (a NULL column ⇒ no triple).
    #[test]
    fn single_pattern_is_one_leaf_cq() {
        use crate::iq::SqlCond;
        let body = strip_spine(&norm("SELECT * WHERE { ?s <http://ex/name> ?n }")).clone();
        assert_leaf_cq(&body);
        let IqNode::Construction { child, subst, .. } = &body else {
            unreachable!()
        };
        let IqNode::InnerJoin { children, cond } = &**child else {
            panic!("the scan plus its §11 NULL guard is an InnerJoin over one leaf, got {child:?}");
        };
        assert!(
            matches!(children.as_slice(), [IqNode::Extensional { .. }]),
            "{children:?}"
        );
        assert!(
            cond.iter()
                .any(|c| matches!(c, IqCond::Sql(SqlCond::IsNotNull(_)))),
            "the §11 NULL guard for the column object rides the cond: {cond:?}"
        );
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

    /// Ontop `ValuesNodeOptimization::test1/test2normalizationSlice`: `Slice` directly
    /// over a literal `Values` table (through the builder's identity-projection
    /// `Construction` wrapper, confirmed empirically to be the actual top-level shape)
    /// truncates the row list in place instead of surviving as a `Slice` node — no
    /// `Slice` reaches LOWER at all, so it can never fall back to lowering the full
    /// table + a `Plan`-level LIMIT/OFFSET.
    #[test]
    fn slice_over_values_truncates_in_place() {
        let n = norm("SELECT ?x WHERE { VALUES ?x { 1 2 3 } } LIMIT 1");
        assert!(
            !matches!(n, IqNode::Slice { .. }),
            "Slice must not survive normalize when its child is a Values leaf: {n:?}"
        );
        let IqNode::Construction { child, .. } = &n else {
            panic!("expected the identity-projection Construction wrapper to survive: {n:?}")
        };
        let IqNode::Values { vars, rows } = child.as_ref() else {
            panic!("expected a truncated Values leaf: {child:?}")
        };
        assert_eq!(vars.len(), 1, "one VALUES var");
        assert_eq!(rows.len(), 1, "LIMIT 1 keeps exactly one row: {rows:?}");
        assert_eq!(
            row_int(&rows[0]),
            1,
            "the as-written first row (1), not an arbitrary survivor"
        );
    }

    /// The OFFSET half of the same rule, and a limit exceeding the remaining rows
    /// (clamped, not an out-of-bounds panic).
    #[test]
    fn slice_over_values_offset_and_overrun() {
        let n = norm("SELECT ?x WHERE { VALUES ?x { 1 2 3 } } LIMIT 5 OFFSET 1");
        let IqNode::Construction { child, .. } = &n else {
            panic!("expected the identity-projection Construction wrapper: {n:?}")
        };
        let IqNode::Values { rows, .. } = child.as_ref() else {
            panic!("expected a truncated Values leaf: {child:?}")
        };
        assert_eq!(
            rows.iter().map(|r| row_int(r)).collect::<Vec<_>>(),
            vec![2, 3],
            "OFFSET 1 skips the first row; LIMIT 5 overruns the remainder harmlessly"
        );
    }

    /// One row's sole cell as a plain integer, for the assertions above.
    fn row_int(row: &[Option<TermDef>]) -> i64 {
        let TermDef::Const(t) = row[0].as_ref().expect("VALUES cell is bound") else {
            panic!("expected a Const cell: {row:?}")
        };
        let sf_core::Term::Literal(lit) = t else {
            panic!("expected a literal term: {t:?}")
        };
        lit.value()
            .parse()
            .unwrap_or_else(|_| panic!("expected an integer literal: {lit:?}"))
    }

    /// Ontop `ValuesNodeOptimization::test3normalizationDistinct`: `Distinct` directly
    /// over a literal `Values` table (through the identity-projection `Construction`
    /// wrapper) dedups the row list in place instead of surviving as a `Distinct` node.
    #[test]
    fn distinct_over_values_dedups_in_place() {
        let n = norm("SELECT DISTINCT ?x WHERE { VALUES ?x { 1 1 2 2 2 } }");
        assert!(
            !matches!(n, IqNode::Distinct { .. }),
            "Distinct must not survive normalize when its child is a Values leaf: {n:?}"
        );
        let IqNode::Construction { child, .. } = &n else {
            panic!("expected the identity-projection Construction wrapper: {n:?}")
        };
        let IqNode::Values { rows, .. } = child.as_ref() else {
            panic!("expected a deduped Values leaf: {child:?}")
        };
        assert_eq!(
            rows.iter().map(|r| row_int(r)).collect::<Vec<_>>(),
            vec![1, 2],
            "first-occurrence order preserved, duplicates removed: {rows:?}"
        );
    }

    /// A cell that isn't a plain `Const` (here, a `CONCAT` `TermDef::Concat` produced
    /// by the constant-Union fold) has no comparable form at this stage — the dedup
    /// declines (a safe no-op: `Distinct` still runs, correctly, at LOWER/exec).
    #[test]
    fn distinct_over_non_const_cells_declines_safely() {
        let n = norm(
            "SELECT DISTINCT ?x WHERE { { BIND(CONCAT(\"a\",\"b\") AS ?x) } \
             UNION { BIND(CONCAT(\"a\",\"b\") AS ?x) } }",
        );
        let IqNode::Construction { child, .. } = &n else {
            panic!("expected the identity-projection Construction wrapper: {n:?}")
        };
        // The constant-Union fold (test14's rule) still fires here (CONCAT of
        // constants IS foldable into a Values row) -- it's the DEDUP that must
        // decline on the resulting non-Const cell, leaving 2 (duplicate) rows for
        // Distinct/exec to handle downstream, same as before this rule existed.
        let IqNode::Distinct { child: values } = child.as_ref() else {
            panic!("expected Distinct to survive (decline) over a non-Const Values cell: {child:?}")
        };
        let IqNode::Values { rows, .. } = values.as_ref() else {
            panic!("expected the folded (but not deduped) Values leaf: {values:?}")
        };
        assert_eq!(
            rows.len(),
            2,
            "the dedup declined, leaving both rows: {rows:?}"
        );
    }

    /// Ontop `ValuesNodeOptimization::test14ConstructionUnionTrueTrue`: a `Union` of
    /// bare-constant `BIND`-only arms (each `Construction{child: True, ...}`) folds to
    /// one `Values` leaf carrying one row per arm.
    #[test]
    fn constant_union_folds_to_values() {
        let n = norm("SELECT ?x WHERE { { BIND(\"a\" AS ?x) } UNION { BIND(\"b\" AS ?x) } }");
        let IqNode::Construction { child, .. } = &n else {
            panic!("expected the identity-projection Construction wrapper: {n:?}")
        };
        let IqNode::Values { vars, rows } = child.as_ref() else {
            panic!("expected the Union to fold to a Values leaf: {child:?}")
        };
        assert_eq!(vars.len(), 1);
        assert_eq!(rows.len(), 2, "one row per constant arm: {rows:?}");
    }

    /// THREE arms: `A UNION B UNION C` parses left-associative (`(A UNION B) UNION
    /// C`), so the inner pair folds to a bare `Values` before the outer `Union` (with
    /// arm C still a `Construction`) ever runs — `try_fold_constant_union` must absorb
    /// an already-folded `Values` arm's rows directly, not just a `Construction` one.
    /// (RED before the fix: the outer fold declined on the first arm not being a
    /// `Construction`, leaving `Union[Values{[a,b]}, Construction{c}]` unfolded.)
    #[test]
    fn three_arm_constant_union_folds_to_one_values() {
        let n = norm(
            "SELECT ?x WHERE { { BIND(\"a\" AS ?x) } UNION { BIND(\"b\" AS ?x) } \
             UNION { BIND(\"c\" AS ?x) } }",
        );
        let IqNode::Construction { child, .. } = &n else {
            panic!("expected the identity-projection Construction wrapper: {n:?}")
        };
        let IqNode::Values { rows, .. } = child.as_ref() else {
            panic!("expected all three arms to fold to ONE Values leaf: {child:?}")
        };
        assert_eq!(
            rows.len(),
            3,
            "one row per constant arm, not 2+1 split: {rows:?}"
        );
    }

    /// Ontop `ValuesNodeOptimization::test26MergeableCombination`: two `VALUES`
    /// blocks binding the SAME two variables but declaring them in DIFFERENT header
    /// order still fold into one `Values` leaf, cells correctly reordered by name
    /// (not position) to the outer `project`'s canonical order — no transposition.
    #[test]
    fn constant_union_folds_reordered_columns_without_transposing() {
        let n = norm(
            "SELECT ?x ?y WHERE { { VALUES (?x ?y) { (1 2) } } \
             UNION { VALUES (?y ?x) { (3 4) } } }",
        );
        let IqNode::Construction { child, .. } = &n else {
            panic!("expected the identity-projection Construction wrapper: {n:?}")
        };
        let IqNode::Values { vars, rows } = child.as_ref() else {
            panic!("expected both arms to fold to ONE Values leaf: {child:?}")
        };
        assert_eq!(
            vars.iter().map(|v| v.as_ref()).collect::<Vec<&str>>(),
            vec!["x", "y"],
            "outer project order: {vars:?}"
        );
        // Second VALUES block declared (?y ?x) with row (3 4): y=3, x=4 -- must
        // land as (x=4, y=3) once reordered to the [x,y] project order, NOT (x=3,
        // y=4) (a transposition the naive positional copy this rule replaced would
        // produce).
        assert_eq!(
            rows.iter()
                .map(|r| (row_int(&r[0..1]), row_int(&r[1..2])))
                .collect::<Vec<_>>(),
            vec![(1, 2), (4, 3)],
            "row order preserved, but each row's OWN cells reordered by name, not \
             position: {rows:?}"
        );
    }

    /// Ontop `ValuesNodeOptimization::test4SliceUnionValuesValues`: `Slice` over a
    /// `Union` of two bare `VALUES` blocks — covered FOR FREE by composing the two
    /// existing rules above (no new production code): each bare `VALUES` arm is
    /// already an `IqNode::Values` (no `Construction` wrapper needed, unlike `BIND`),
    /// so `try_fold_constant_union`'s "absorb an already-Values arm" case (added for
    /// the left-associative 3-arm fix) folds the whole `Union` to one `Values`, which
    /// `normalize_slice` then truncates in place — verified here as its own named
    /// scenario, not assumed from the two rules' own tests.
    #[test]
    fn slice_over_union_of_values_values_folds_and_truncates() {
        let n =
            norm("SELECT ?x WHERE { { VALUES ?x { 1 2 } } UNION { VALUES ?x { 3 4 } } } LIMIT 3");
        let IqNode::Construction { child, .. } = &n else {
            panic!("expected the identity-projection Construction wrapper: {n:?}")
        };
        let IqNode::Values { rows, .. } = child.as_ref() else {
            panic!("expected the Union to fold to ONE Values leaf, then truncate: {child:?}")
        };
        assert_eq!(
            rows.iter().map(|r| row_int(r)).collect::<Vec<_>>(),
            vec![1, 2, 3],
            "as-written order across both arms, truncated to LIMIT 3: {rows:?}"
        );
    }

    /// Ontop `ValuesNodeOptimization::test5SliceUnionValuesNonValues`: a `Slice`
    /// window that falls ENTIRELY within a leading `Values` arm's known row count
    /// drops the trailing DATA (real-pattern) arm outright — no scan of it survives.
    #[test]
    fn slice_over_union_drops_unreachable_data_arm() {
        let n = norm(
            "SELECT ?n WHERE { { VALUES ?n { \"a\" \"b\" } } \
             UNION { ?s <http://ex/name> ?n } } LIMIT 2",
        );
        let IqNode::Values { vars, rows } = &n else {
            panic!(
                "expected full resolution to a bare Values leaf (no Slice/Union survives): {n:?}"
            )
        };
        assert_eq!(vars.len(), 1);
        assert_eq!(
            rows.iter().map(|r| row_str(r)).collect::<Vec<_>>(),
            vec!["a", "b"],
            "the data arm must not appear at all: {rows:?}"
        );
    }

    /// The OFFSET half: a leading `Values` arm big enough to cover BOTH the offset
    /// skip and the whole limit drops the data arm too, keeping only the surviving
    /// window from the `Values` arm.
    #[test]
    fn slice_over_union_offset_fully_satisfied_by_values_drops_data_arm() {
        let n = norm(
            "SELECT ?n WHERE { { VALUES ?n { \"a\" \"b\" \"c\" } } \
             UNION { ?s <http://ex/name> ?n } } LIMIT 2 OFFSET 1",
        );
        let IqNode::Values { rows, .. } = &n else {
            panic!("expected full resolution to a bare Values leaf: {n:?}")
        };
        assert_eq!(
            rows.iter().map(|r| row_str(r)).collect::<Vec<_>>(),
            vec!["b", "c"],
            "OFFSET 1 skips \"a\"; the data arm is still unreachable: {rows:?}"
        );
    }

    /// Ontop `ValuesNodeOptimization::test6/7SliceUnionValuesNonValues` (residual
    /// limit): the `Values` arm doesn't cover the whole window, so the data arm
    /// survives — but under a Slice reset to `offset=0` (the survivors already
    /// account for everything before them in as-written order) with the ORIGINAL
    /// limit (not reduced: a plain Slice-over-Union already reads the reconstructed
    /// sequence from position 0, so the survivor rows themselves count toward it).
    #[test]
    fn slice_over_union_residual_limit_keeps_the_data_arm() {
        let n = norm(
            "SELECT ?n WHERE { { VALUES ?n { \"a\" \"b\" \"c\" } } \
             UNION { ?s <http://ex/name> ?n } } LIMIT 5 OFFSET 1",
        );
        let IqNode::Slice {
            child,
            offset,
            limit,
        } = &n
        else {
            panic!("expected a residual Slice to survive (the data arm is still needed): {n:?}")
        };
        assert_eq!(
            *offset, 0,
            "the offset skip is already baked into the survivors"
        );
        assert_eq!(*limit, Some(5), "the ORIGINAL limit, not reduced");
        let IqNode::Union { children, .. } = child.as_ref() else {
            panic!("expected the survivor Values arm + the data arm: {child:?}")
        };
        assert_eq!(
            children.len(),
            2,
            "one survivor arm + the data arm: {children:?}"
        );
        let IqNode::Values { rows, .. } = &children[0] else {
            panic!("expected the FIRST child to be the bare survivor Values arm: {children:?}")
        };
        assert_eq!(
            rows.iter().map(|r| row_str(r)).collect::<Vec<_>>(),
            vec!["b", "c"],
            "OFFSET 1 dropped \"a\"; \"b\"/\"c\" survive as explicit rows: {rows:?}"
        );
        assert!(
            !matches!(children[1], IqNode::Values { .. }),
            "the second child must still be the untouched data arm: {:?}",
            children[1]
        );
    }

    /// Adversarial-review-caught regression: OFFSET exceeds the known (`Values`) arm's
    /// ENTIRE row count while a data arm still remains -- the whole `Values` arm is
    /// dropped (all of it falls before the window), but the LEFTOVER skip must carry
    /// forward onto the data arm itself, not vanish. A first draft hardcoded the
    /// residual `Slice`'s offset to 0 unconditionally, silently discarding that
    /// leftover and leaking extra rows the true OFFSET should have skipped.
    #[test]
    fn slice_over_union_offset_exceeding_values_carries_forward_onto_data_arm() {
        let n = norm(
            "SELECT ?n WHERE { { VALUES ?n { \"a\" \"b\" \"c\" } } \
             UNION { ?s <http://ex/name> ?n } } LIMIT 5 OFFSET 4",
        );
        let IqNode::Slice {
            child,
            offset,
            limit,
        } = &n
        else {
            panic!("expected a residual Slice to survive (the data arm is still needed): {n:?}")
        };
        assert_eq!(
            *offset, 1,
            "3 of the 4 requested skips are consumed by the (fully-dropped) Values \
             arm; exactly 1 more must land on the data arm: {n:?}"
        );
        assert_eq!(*limit, Some(5));
        assert!(
            !matches!(child.as_ref(), IqNode::Union { .. }),
            "the Values arm contributed ZERO survivor rows (all 3 before the window) \
             -- the reconstructed child is the data arm alone, no Union wrapper: {child:?}"
        );
    }

    /// Nothing to drop or truncate at all (OFFSET 0, the `Values` arm fully survives
    /// unmodified, the data arm is reached before the window is satisfied) — the
    /// rule must decline (keep `Slice{Union{..}}` byte-identical to the input),
    /// not needlessly rebuild an identical tree.
    #[test]
    fn slice_over_union_declines_when_nothing_changes() {
        let n = norm(
            "SELECT ?n WHERE { { VALUES ?n { \"a\" \"b\" } } \
             UNION { ?s <http://ex/name> ?n } } LIMIT 5",
        );
        let IqNode::Slice { child, .. } = &n else {
            panic!("expected the Slice to survive untouched: {n:?}")
        };
        let IqNode::Union { children, .. } = child.as_ref() else {
            panic!("expected the Union to survive untouched: {child:?}")
        };
        assert_eq!(children.len(), 2, "both arms untouched: {children:?}");
        assert!(
            matches!(&children[0], IqNode::Construction { child, .. } if matches!(**child, IqNode::Values { .. })),
            "the Values arm keeps its ORIGINAL Construction wrapper (not rebuilt): {:?}",
            children[0]
        );
    }

    /// A DATA arm appearing BEFORE any `Values` arm can never be dropped (its
    /// cardinality is unknown from the very first arm) — declines immediately.
    #[test]
    fn slice_over_union_declines_when_data_arm_is_first() {
        let n = norm(
            "SELECT ?n WHERE { { ?s <http://ex/name> ?n } \
             UNION { VALUES ?n { \"a\" \"b\" } } } LIMIT 1",
        );
        assert!(
            matches!(&n, IqNode::Slice { child, .. } if matches!(child.as_ref(), IqNode::Union { .. })),
            "must decline -- the data arm is first, nothing is provably unreachable: {n:?}"
        );
    }

    /// One row's sole cell as a plain string, for the arm-drop assertions above.
    fn row_str(row: &[Option<TermDef>]) -> String {
        let TermDef::Const(t) = row[0].as_ref().expect("VALUES cell is bound") else {
            panic!("expected a Const cell: {row:?}")
        };
        let sf_core::Term::Literal(lit) = t else {
            panic!("expected a literal term: {t:?}")
        };
        lit.value().to_owned()
    }

    /// Ontop `ValuesNodeOptimization::test8DistinctUnionValuesNonValues`: `Distinct`
    /// over `Union[distinct-Values, ext]` is a genuine no-op — covered FOR FREE:
    /// `normalize_distinct` only recognizes `Values`/`Construction{Values}` as its
    /// child directly, so a `Union` child (even one with an already-duplicate-free
    /// `Values` arm) correctly declines and survives untouched. Nothing to dedup
    /// here that the outer `Distinct` doesn't already have to do regardless.
    #[test]
    fn distinct_over_union_of_already_distinct_values_and_data_is_a_no_op() {
        let n = norm(
            "SELECT DISTINCT ?n WHERE { { VALUES ?n { \"a\" \"b\" } } \
             UNION { ?s <http://ex/name> ?n } }",
        );
        assert!(
            matches!(&n, IqNode::Distinct { child } if matches!(child.as_ref(), IqNode::Union { .. })),
            "no rewrite applies -- Distinct{{Union{{..}}}} survives untouched: {n:?}"
        );
    }

    /// Ontop `ValuesNodeOptimization::test9DistinctUnionValuesNonValues`: `Distinct`
    /// over `Union[Values(dups), ext]` dedups the `Values` arm's OWN internal
    /// duplicates in place; the outer `Distinct` still survives (cross-arm dedup
    /// against the data arm isn't statically provable) and the data arm itself is
    /// untouched.
    #[test]
    fn distinct_over_union_dedups_the_values_arm_keeps_distinct_and_data_arm() {
        let n = norm(
            "SELECT DISTINCT ?n WHERE { { VALUES ?n { \"a\" \"a\" \"b\" } } \
             UNION { ?s <http://ex/name> ?n } }",
        );
        let IqNode::Distinct { child } = &n else {
            panic!("the outer Distinct must survive (cross-arm dedup isn't provable): {n:?}")
        };
        let IqNode::Union { children, .. } = child.as_ref() else {
            panic!("expected the Union to survive: {child:?}")
        };
        assert_eq!(children.len(), 2, "both arms present: {children:?}");
        // The arm keeps its ORIGINAL identity-projection Construction wrapper
        // (confirmed empirically, same pattern as every other Values-as-union-arm
        // case in this file) -- `dedup_one_arm` rebuilds it around the deduped
        // Values leaf rather than unwrapping it.
        let IqNode::Construction { child: values, .. } = &children[0] else {
            panic!("expected the FIRST child to still be Construction-wrapped: {children:?}")
        };
        let IqNode::Values { rows, .. } = values.as_ref() else {
            panic!("expected the wrapped leaf to be the (deduped) Values arm: {values:?}")
        };
        assert_eq!(
            rows.iter().map(|r| row_str(r)).collect::<Vec<_>>(),
            vec!["a", "b"],
            "the Values arm's own duplicate \"a\" is gone: {rows:?}"
        );
        assert!(
            !matches!(children[1], IqNode::Values { .. }),
            "the data arm must still be the untouched real pattern: {:?}",
            children[1]
        );
    }

    /// The identical narrowing-projection hazard `same_var_set` guards against for
    /// the single-`Values`-child case (test3's adversarial-review-caught bug)
    /// applies per-arm here too: a `Values` arm whose own columns are a
    /// STRICT SUPERSET of the Union's `project` (some column is projected away
    /// above this level) must decline dedup on that arm -- collapsing on the FULL
    /// pre-projection tuple could wrongly merge two rows that remain genuinely
    /// distinct after the (elsewhere-applied) projection.
    #[test]
    fn distinct_over_union_declines_a_narrowed_values_arm() {
        // The data arm only ever binds ?n, so the Union's own project is [n] even
        // though this Values arm's OWN vars are [n, extra] -- same_var_set([n],
        // [n,extra]) is false, so dedup_one_arm must leave this arm untouched.
        let n = norm(
            "SELECT DISTINCT ?n WHERE { { VALUES (?n ?extra) { (\"a\" 1) (\"a\" 2) } } \
             UNION { ?s <http://ex/name> ?n } }",
        );
        let IqNode::Distinct { child } = &n else {
            panic!("expected Distinct to survive: {n:?}")
        };
        let IqNode::Union { children, .. } = child.as_ref() else {
            panic!("expected the Union to survive: {child:?}")
        };
        let IqNode::Construction { child: values, .. } = &children[0] else {
            panic!("expected the first child to still be Construction-wrapped: {children:?}")
        };
        let IqNode::Values { rows, .. } = values.as_ref() else {
            panic!("expected the wrapped leaf to still be the Values arm: {values:?}")
        };
        assert_eq!(
            rows.len(),
            2,
            "both rows must survive untouched -- a narrowing dedup here would be \
             the exact =_bag bug an adversarial review caught on the single-arm \
             rule: {rows:?}"
        );
    }

    /// Ontop `ValuesNodeOptimization::test25NoVariableTrueNodesAndValuesNodes`: a
    /// zero-var `Union` of bare `{}` groups (each an `IqNode::True`) folds to a
    /// "counting" `Values` leaf -- zero columns, one empty-tuple row per arm.
    #[test]
    fn zero_var_union_of_true_arms_folds_to_counting_values() {
        let n = norm("SELECT * WHERE { {} UNION {} UNION {} }");
        let IqNode::Construction { child, .. } = &n else {
            panic!("expected the identity-projection Construction wrapper: {n:?}")
        };
        let IqNode::Values { vars, rows } = child.as_ref() else {
            panic!("expected the Union to fold to a counting Values leaf: {child:?}")
        };
        assert!(vars.is_empty(), "zero columns: {vars:?}");
        assert_eq!(rows.len(), 3, "one empty-tuple row per True arm: {rows:?}");
        assert!(
            rows.iter().all(|r| r.is_empty()),
            "every row is the empty tuple: {rows:?}"
        );
    }

    /// A DATA arm (a real triple pattern, not a bare constant) blocks the FULL
    /// fold (`try_fold_constant_union`) — no `Values` leaf. With only ONE constant
    /// arm here (the class-atom `?s <rdf:type> ?x` itself expands to a 2-way
    /// per-table union during BUILD, so this is 1 constant + 2 data arms, not 1+1),
    /// the PARTIAL fold (`try_partial_fold_constant_union`, test15) declines too
    /// (nothing to combine — see that function's own doc comment); see
    /// `partial_fold_combines_multiple_constant_arms_keeps_data_arm` below for the
    /// 2-constant-arms case that DOES partially fold.
    #[test]
    fn union_with_a_data_arm_does_not_fold() {
        let n = norm(&format!(
            "SELECT ?x WHERE {{ {{ BIND(\"a\" AS ?x) }} UNION {{ ?s <{RDF_TYPE}> ?x }} }}"
        ));
        assert!(
            !matches!(n, IqNode::Values { .. }),
            "a real pattern in one arm must block the constant fold: {n:?}"
        );
    }

    /// Ontop `ValuesNodeOptimization::test15ConstructionUnionTrueTrueDataNode`:
    /// with TWO OR MORE constant arms alongside a data arm, the full fold still
    /// declines (a real pattern is present), but the PARTIAL fold now combines
    /// just the constant arms into one `Values`, keeping the data arm(s) as
    /// sibling `Union` arms — fewer arms, same `=_bag` multiset.
    ///
    /// The DATA arm is deliberately FIRST here (`A UNION B UNION C` is
    /// left-associative — `(A UNION B) UNION C`): with the data arm LAST, the
    /// inner `{BIND a} UNION {BIND b}` pair would fully fold via the PRE-EXISTING
    /// `try_fold_constant_union` (test14, both arms constant) before this rule
    /// ever runs, making the test pass regardless of whether this new function
    /// exists at all — a mistake caught empirically via this test's OWN
    /// revert-proof (bypassing `try_partial_fold_constant_union` produced no
    /// failure with the data arm last, exposing the vacuous ordering).
    #[test]
    fn partial_fold_combines_multiple_constant_arms_keeps_data_arm() {
        let n = norm(&format!(
            "SELECT ?x WHERE {{ {{ ?s <{RDF_TYPE}> ?x }} UNION {{ BIND(\"a\" AS ?x) }} \
             UNION {{ BIND(\"b\" AS ?x) }} }}"
        ));
        let IqNode::Union { children, .. } = &n else {
            panic!("expected a Union of [folded-Values, data-arm, data-arm]: {n:?}")
        };
        assert_eq!(
            children.len(),
            3,
            "2 constant arms fold to 1 Values arm + the class-atom's own 2-way \
             per-table data union = 3 total (down from 4): {children:?}"
        );
        // The folded Values arrives back here wrapped in an identity Construction
        // (`lift_construction`'s catch-all re-wraps every non-Union/Construction/
        // Empty arm when the enclosing top-level query Construction re-processes
        // this already-normalized Union -- the same double-pass mechanism found
        // during the test25 investigation), not bare -- confirmed empirically, not
        // assumed.
        let IqNode::Construction { child, subst, .. } = &children[0] else {
            panic!(
                "expected the FIRST child to be the folded constant Values leaf \
                     (kept arms are appended after it): {children:?}"
            )
        };
        assert!(
            subst.is_empty(),
            "an identity wrapper, no bindings: {subst:?}"
        );
        let IqNode::Values { rows, .. } = child.as_ref() else {
            panic!("expected an identity-Construction-wrapped Values leaf: {child:?}")
        };
        assert_eq!(rows.len(), 2, "one row per constant arm: {rows:?}");
        assert!(
            children[1..]
                .iter()
                .all(|c| !matches!(c, IqNode::Values { .. } | IqNode::Union { .. })),
            "the remaining (data) arms are untouched, no further folding: {children:?}"
        );
    }

    /// A variable-referencing `BIND` (not a compile-time constant) also blocks the
    /// fold — `bind_term_def` against an empty bindings map cannot resolve it.
    #[test]
    fn union_with_a_variable_referencing_bind_does_not_fold() {
        let n = norm(&format!(
            "SELECT ?x ?y WHERE {{ {{ ?s <{RDF_TYPE}> ?y BIND(?y AS ?x) }} \
             UNION {{ BIND(\"b\" AS ?x) }} }}"
        ));
        assert!(
            !matches!(n, IqNode::Values { .. }),
            "a variable-dependent binding must block the constant fold: {n:?}"
        );
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
