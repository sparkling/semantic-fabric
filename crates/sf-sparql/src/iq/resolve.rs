//! Resolve — the operator-tree ([`IqNode`]) RESOLVE stage (ADR-0023 M3a,
//! `docs/design/ADR-0023-M3-resolution-pipeline.md` §3). It consumes the
//! context-free tree produced by [`crate::build::build_tree`] and returns a tree
//! with **ZERO** [`IqNode::Intensional`] leaves: every unresolved triple pattern is
//! replaced by the resolved relational subtree it unfolds to against the
//! T-mappings. Everything else — `Construction`/`Filter`/`InnerJoin`/`LeftJoin`/
//! `Union`/`Aggregation`/`Distinct`/`Slice`/`OrderBy`/`Values`/`Empty`/`True`/`Path`
//! — passes through, only recursing into children. **FILTER/BIND stay symbolic:**
//! an [`IqCond::Expr`] and a [`BindDef::Expr`] are carried through untouched and
//! resolved per-leaf-CQ only at LOWER (M3 design §3, §5).
//!
//! ## Status: PRODUCTION (tree default since ADR-0023 M8; banner corrected 2026-07-18)
//!
//! This RESOLVE stage runs in the live engine: `translate`/`translate_with` route
//! through [`crate::translate_tree`] by default (`lib.rs`); the flat
//! [`crate::unfold`] remains the `=_bag` oracle / fallback (and this stage still
//! reuses the flat `Unfolder` verbatim as its resolution bridge — byte-identical
//! arm sets by construction).
//!
//! ## Smallest change: the bridge over the flat oracle (M3 design §3.3 fallback)
//!
//! Rather than fork the per-`(triples-map × POM)` atom logic into a new shared
//! primitive (which risks perturbing the byte-identical flat `atom`/`pattern_branches`
//! oracle this milestone must preserve), RESOLVE **calls the flat
//! [`Unfolder::pattern_branches`] verbatim** (via the graph-scoped
//! [`Unfolder::resolve_pattern`]) and **bridges** each resulting [`Branch`] to an
//! [`IqNode`] arm. The arm set, conds, fresh aliases, and `predicate_can_match`
//! pruning are therefore *identical* to the flat translation by construction — that is
//! the `=_bag` argument (M3 design §3.3, §6, ledger R1). The flat `Vec<Branch>` bag
//! union becomes:
//!
//! * **0 arms** ⇒ [`IqNode::Empty`] over the pattern's variables;
//! * **1 arm** ⇒ that arm's subtree directly (no `Union` wrapper);
//! * **N ≥ 2 arms** ⇒ [`IqNode::Union`] over the arms, `project` = the pattern's
//!   variables.
//!
//! Each arm bridges one [`Branch`] (which, for a single triple pattern, only ever uses
//! `core` + `where_conds` + `bindings` — never `opts`/`path`/`agg`) to:
//!
//! ```text
//! Construction {
//!   subst:   branch.bindings  → BindDef::Resolved(TermDef)   (the var → term scope)
//!   project: branch.bindings.keys()                          (the arm's bound vars)
//!   child:   InnerJoin {                                     (CROSS JOIN + WHERE-eq,
//!     children: branch.core → Extensional { scan }            exactly the flat core)
//!     cond:     branch.where_conds → IqCond::Sql(SqlCond)     (constant-position &
//!   }                                                          shared-var / refObjectMap
//! }                                                            equalities)
//! ```
//!
//! A single-scan arm with no conds collapses to a bare [`IqNode::Extensional`] (no
//! degenerate one-child `InnerJoin`). A `rr:refObjectMap` arm needs no special case:
//! the flat `atom` already pushed the parent scan into `core` and the
//! [`SqlCond::ColEq`] join into `where_conds`, so it bridges to the contract's 2-scan
//! `InnerJoin` automatically (M3 design §3.1, §3.4).
//!
//! All join logic lives in the `IqCond::Sql` conds (not in `Extensional.bind`, which
//! the bridge leaves empty): the `InnerJoin` is a cross-join driven by explicit
//! equalities, mirroring the flat `core` + `where_conds` "CROSS JOIN + WHERE-eq ≡ inner
//! join" lowering (`iq.rs` [`Branch`] doc).

use std::collections::BTreeMap;

use sf_core::ir::TriplesMap;

use crate::iq::node::{
    graph_pattern_var, path_pattern_vars, triple_pattern_vars, BindDef, IqCond, IqNode,
};
use crate::iq::Branch;
use crate::saturate::Tbox;
use crate::unfold::{all_pairwise_disjoint, Unfolder};
use crate::{Error, Result};

/// The resolution context (M3 design §3): the T-mappings, the T-Box, the SQL
/// dialect, and a **monotone alias counter** shared across the whole tree.
///
/// It wraps a single [`Unfolder`] so that every [`IqNode::Intensional`] in the tree
/// draws from the *same* alias counter — sibling intensionals therefore get disjoint
/// scan aliases (the precondition for a parent `InnerJoin`/`Union` to compose their
/// arms without alias collisions, design §3.2). The wrapped `Unfolder` is the proven
/// flat oracle, used read-only here except for its alias counter and the transient
/// `current_graph` that [`Unfolder::resolve_pattern`] saves/restores per leaf.
pub struct ResolveCx<'a> {
    unfolder: Unfolder<'a>,
}

impl<'a> ResolveCx<'a> {
    /// A fresh resolution context over the given mappings, T-Box, dialect, and
    /// source schema (the same `(maps, tbox, dialect, schema)` the flat
    /// [`Unfolder::new`] takes — `schema` feeds ADR-0034 D1, see its doc comment
    /// on [`Unfolder`]). The alias counter starts at zero and advances
    /// monotonically across the whole tree.
    pub fn new(
        maps: &'a [TriplesMap],
        tbox: &'a Tbox,
        dialect: sf_sql::Dialect,
        schema: &'a [sf_sql::TableSchema],
    ) -> Self {
        Self {
            unfolder: Unfolder::new(maps, tbox, dialect, schema),
        }
    }

    /// The accumulated D2 shared-dedup-group map (ADR-0034 C0e restoration) —
    /// pulled into the final [`crate::Plan::dedup_groups`] by `translate_tree`
    /// once resolution finishes. Delegates to [`Unfolder::dedup_groups`] (the
    /// field itself is private to `unfold`).
    pub(crate) fn dedup_groups(&self) -> &std::collections::HashMap<usize, usize> {
        self.unfolder.dedup_groups()
    }
}

/// Resolve a whole tree (M3 design §3): walk `node` and replace every
/// [`IqNode::Intensional`] leaf with its resolved subtree, returning a tree with
/// **ZERO** `Intensional` leaves. Every other node recurses into its children
/// unchanged; FILTER/BIND symbolic leaves ([`IqCond::Expr`] / [`BindDef::Expr`]) are
/// carried through untouched (resolved per-leaf-CQ at LOWER, design §5).
///
/// `EXISTS`/`NOT EXISTS` carry a built [`IqNode`] subtree (in [`IqCond::Exists`] /
/// [`IqCond::NotExists`]) that may itself contain `Intensional` leaves, so RESOLVE
/// descends into those subtrees too — the "ZERO Intensional" invariant is over the
/// *entire* tree, including condition-embedded patterns.
pub fn resolve(node: IqNode, cx: &mut ResolveCx) -> Result<IqNode> {
    match node {
        // ---- the one resolving case ---------------------------------------------
        IqNode::Intensional { pattern, graph } => {
            // ADR-0035: a `GRAPH ?v` context contributes `v` to this leaf's own scope,
            // exactly as `IqNode::output_vars` already computes for the UNRESOLVED leaf
            // (`graph_pattern_var`) — needed here too since RESOLVE builds the arms'
            // `Empty`/`Union` wrapper directly, not via a later `output_vars()` call.
            let mut vars = triple_pattern_vars(&pattern);
            if let Some(v) = graph_pattern_var(graph.as_ref()) {
                if !vars.contains(&v) {
                    vars.push(v);
                }
            }
            let mut branches = cx.unfolder.resolve_pattern(&pattern, graph.as_ref())?;
            // ADR-0034 D1/D2 are both skipped inside a FILTER EXISTS / FILTER NOT
            // EXISTS / MINUS body — see `Unfolder::in_existential`'s own doc comment
            // for the SPARQL semantics that make this sound (an existence / anti-join
            // check is unaffected by within-body duplicate rows or duplicate-map
            // triples) and for why it also sidesteps a real, unrelated SubPlan-in-
            // correlated-subquery 501 boundary this would otherwise trip.
            if !cx.unfolder.in_existential {
                // ADR-0034 D1: checked HERE, on this pattern's own just-resolved arms —
                // their bindings are still the pattern's own complete variable set, not
                // yet narrowed by anything downstream. `iq::lower`'s own Construction-arm
                // `project` restriction (which runs during LOWER, well before
                // `cascade::run`'s later, defensive D1 pass ever sees the branch) can
                // strip a key-covering variable the outer query does not project — see
                // `unfold::bgp`'s identical note (the flat engine's mirror of this same
                // per-pattern timing fix) for the `r5_i_duplicate_union_arms` /
                // `r5_iii_non_unique_self_join` regression this closes. `bridge_branch`
                // below reads each branch's (possibly now `true`) `distinct` flag and
                // wraps accordingly — never silently dropped.
                crate::cascade::force_distinct_for_dup_safety(
                    &mut branches,
                    cx.unfolder.schema,
                    cx.unfolder.dialect,
                );
            }
            // ADR-0034 D2: when this pattern's own candidate-map arms are not ALL
            // provably disjoint (the SAME elision check the flat engine's
            // `unfold::pool_pattern_relation` applies), `unfold::disjoint_groups`
            // partitions them into maximal not-provably-disjoint groups — only a
            // group whose members can't all be told apart needs deduping together;
            // an arm disjoint from every other arm stays a plain bag-union
            // alternative even when SOME OTHER pair in this pattern is not disjoint
            // (see that function's own doc comment, and W3C R2RMLTC0004a). `IqNode`
            // has no representation for a pre-pooled `Branch` (RESOLVE/NORMALIZE
            // never see anything but algebra nodes), so instead of pooling here
            // directly, each group of size ≥2 gets its arms wrapped in
            // `IqNode::Distinct` — the tree's established "must become its own
            // derived table" modifier boundary (`iq::lower::lower_node`'s
            // `Aggregation|Distinct|Slice|OrderBy => lower_as_subplan` arm) — which
            // routes them through THAT function's existing multi-branch pooling
            // (ADR-0025 Tier-2 gap 2: narrow-to-vars, injectivity gate, cross-arm
            // reconstruction-agreement gate, `UNION`-vs-`UNION ALL` via
            // `emit_subplan_sql`) unmodified. This reuses the identical pooling
            // algorithm the flat engine's own `pool_group` runs (via the shared
            // `remap_termdef`/`remap_colref` helpers), so the two engines stay
            // `=_bag`-identical by construction — the elision check and grouping are
            // duplicated (`unfold::all_pairwise_disjoint`/`unfold::disjoint_groups`,
            // pure boolean/partition tests), the pooling mechanism is not.
            //
            // Each pooled group is wrapped in a condition-free `IqNode::Filter`
            // around its `Distinct`: `iq::lower::lower_spine` special-cases a
            // `Distinct`/`Aggregation`/`Slice`/`OrderBy` node it reaches DIRECTLY (or
            // immediately under a `Construction` whose OWN child is exactly one of
            // those four shapes) as the outer query-modifier SPINE — peeling it
            // straight into `Plan::distinct` instead of routing it through
            // `lower_node`'s `lower_as_subplan` arm. That peeling is correct for a
            // REAL top-level SPARQL DISTINCT, but this `Distinct` is an internal D2
            // pooling marker, not a user modifier — when a SINGLE group spans the
            // pattern's entire arm set (so there is no enclosing `Union` over
            // multiple groups) and this triple pattern happens to BE the entire
            // WHERE clause (no sibling pattern to force an enclosing `InnerJoin`
            // either, the only other thing that routes a child through `lower_node`
            // instead of `lower_spine`), the bare `Distinct` reaches the spine
            // directly and gets silently un-pooled (found via `differential_tree.rs`'s
            // `r5_ii_overlapping_maps_same_predicate`'s CONSTRUCT-form assertion —
            // flat/tree diverged: flat deduped via `unfold::pool_pattern_relation`,
            // tree did not). Two earlier attempts at this fix failed: a plain
            // `Construction` wrapper still has the `Distinct` as its DIRECT child, so
            // the SAME guard matches; a one-child condition-free `InnerJoin` wrapper
            // is torn back down to its bare child by `iq::normalize`'s OWN
            // InnerJoin-identity pruning (`children.len() == 1 && cond.is_empty()`)
            // before LOWER ever sees it. `Filter` has no such identity-unwrap for an
            // unrecognized child shape (`normalize_filter`'s `other => Filter{child,
            // cond}` arm keeps the wrapper), and `lower_spine` never special-cases
            // `Filter` at all — it always falls to the "other" arm, which delegates
            // the whole subtree to `lower_node`, whose own `Filter` arm then calls
            // `lower_node` on `child` again, reaching the `Distinct =>
            // lower_as_subplan` arm correctly regardless of tree position (including
            // as one of several children under an enclosing `Union` over multiple
            // groups, the ordinary "nested" case this mechanism already handles). An
            // empty `cond` is a true no-op (`apply_conds` over zero conditions), so
            // this adds no semantic content.
            if !cx.unfolder.in_existential
                && branches.len() >= 2
                && !all_pairwise_disjoint(&branches)
            {
                let groups = crate::unfold::disjoint_groups(&branches);
                let mut branches: Vec<Option<Branch>> = branches.into_iter().map(Some).collect();
                let mut children: Vec<IqNode> = Vec::with_capacity(groups.len());
                for group in groups {
                    if group.len() == 1 {
                        children.push(bridge_branch(
                            branches[group[0]].take().expect("each index visited once"),
                        ));
                        continue;
                    }
                    let mut members: Vec<Branch> = group
                        .iter()
                        .map(|&i| branches[i].take().expect("each index visited once"))
                        .collect();
                    // ADR-0025 (sound-pooling shape): a positional pool this group's arms
                    // would otherwise need can hit PostgreSQL's own `UNION` type-resolver
                    // (a raw SQL error) or, if aligned via a `CAST`, silently drift a
                    // floating-point column's lexical form — see `cascade::group_has_
                    // unsafe_float_slot_mismatch`'s doc comment for the live-verified
                    // evidence (W3C R2RMLTC0012e).
                    let member_refs: Vec<&Branch> = members.iter().collect();
                    if crate::cascade::group_has_unsafe_float_slot_mismatch(
                        &member_refs,
                        cx.unfolder.schema,
                        cx.unfolder.dialect,
                    ) {
                        // Run 5 C0e restoration (mirrors `unfold::pool_pattern_relation`'s
                        // identical gate verbatim): when this group is ALSO
                        // `group_eligible_for_term_dedup` (every member standalone, an
                        // offending non-injective binding present), skip SQL pooling
                        // entirely instead of refusing — bridge each member SEPARATELY
                        // (exactly the `group.len() == 1` arm above, just for N members),
                        // tagged via `cx.unfolder`'s `dedup_groups` so `run_branches`
                        // shares ONE Rust-side term-dedup seen-set across them instead of
                        // a `Filter{Distinct{Union}}` SQL pool — no `UNION`, so the PG
                        // float-vs-text type-alignment wall never applies.
                        let keep: std::collections::HashSet<String> =
                            vars.iter().map(|v| v.to_string()).collect();
                        if crate::cascade::group_eligible_for_term_dedup(&members, &keep) {
                            crate::cascade::narrow_group_for_shared_term_dedup(&mut members, &keep);
                            let gid = cx.unfolder.alias();
                            for b in &members {
                                if let Some((alias, _)) = b.alias_sources().into_iter().next() {
                                    cx.unfolder.tag_dedup_group(alias, gid);
                                }
                            }
                            children.extend(members.into_iter().map(bridge_branch));
                            continue;
                        }
                        return Err(Error::Unsupported(
                            "D2 pool: a floating-point column would positionally UNION \
                             against a differently-typed sibling column on PostgreSQL → 501 \
                             (cannot be aligned soundly in SQL without risking lexical drift \
                             — ADR-0025)"
                                .to_owned(),
                        ));
                    }
                    let arms: Vec<IqNode> = members.into_iter().map(bridge_branch).collect();
                    children.push(IqNode::Filter {
                        child: Box::new(IqNode::Distinct {
                            child: Box::new(IqNode::Union {
                                children: arms,
                                project: vars.clone(),
                            }),
                        }),
                        cond: Vec::new(),
                    });
                }
                return Ok(match children.len() {
                    1 => children.pop().expect("checked len == 1"),
                    _ => IqNode::Union {
                        children,
                        project: vars,
                    },
                });
            }
            let mut arms: Vec<IqNode> = branches.into_iter().map(bridge_branch).collect();
            Ok(match arms.len() {
                0 => IqNode::Empty { vars },
                1 => arms.pop().expect("len checked == 1"),
                _ => IqNode::Union {
                    children: arms,
                    project: vars,
                },
            })
        }

        // ---- the property-path resolving case (M5 Wave 1; ADR-0035 for GRAPH ?v) --
        // Reuse the flat `path_branch` VERBATIM via `resolve_path` (pinning the
        // constant active graph exactly as the flat `GRAPH <g> { ?s PATH ?o }` path
        // does; a *variable* graph instead unions over the mapping's declared constant
        // named graphs — `Unfolder::path_branches_for_graph_var`, `path.rs`), then
        // bridge each resulting `Branch` (carrying `path = Some(PathClosure)`) to an
        // `IqNode::Path` UNDER its `Construction` bindings via the SAME `bridge_branch`
        // the triple case uses — the identical 0/1/N arm-count bridging `Intensional`
        // above already applies (`resolve_path` returns `Vec<Branch>` uniformly now: a
        // constant/no-GRAPH path is always exactly one arm, matching the old
        // single-`Branch` contract; `GRAPH ?v` may be several, or none). ZERO
        // `UnresolvedPath` survives — `bridge_branch` produces no `UnresolvedPath`.
        IqNode::UnresolvedPath {
            subject,
            path,
            object,
            graph,
        } => {
            let vars = path_pattern_vars(&subject, &object, graph.as_ref());
            let branches = cx
                .unfolder
                .resolve_path(&subject, &path, &object, graph.as_ref())?;
            let mut arms: Vec<IqNode> = branches.into_iter().map(bridge_branch).collect();
            Ok(match arms.len() {
                0 => IqNode::Empty { vars },
                1 => arms.pop().expect("len checked == 1"),
                _ => IqNode::Union {
                    children: arms,
                    project: vars,
                },
            })
        }

        // ---- recurse into children, structure unchanged -------------------------
        IqNode::Construction {
            child,
            subst,
            project,
        } => Ok(IqNode::Construction {
            child: Box::new(resolve(*child, cx)?),
            subst,
            project,
        }),
        IqNode::Filter { child, cond } => Ok(IqNode::Filter {
            child: Box::new(resolve(*child, cx)?),
            cond: resolve_conds(cond, cx)?,
        }),
        IqNode::InnerJoin { children, cond } => Ok(IqNode::InnerJoin {
            children: resolve_children(children, cx)?,
            cond: resolve_conds(cond, cx)?,
        }),
        IqNode::LeftJoin { left, right, cond } => Ok(IqNode::LeftJoin {
            left: Box::new(resolve(*left, cx)?),
            right: Box::new(resolve(*right, cx)?),
            cond: resolve_conds(cond, cx)?,
        }),
        IqNode::Union { children, project } => Ok(IqNode::Union {
            children: resolve_children(children, cx)?,
            project,
        }),
        IqNode::Aggregation {
            child,
            grouping,
            aggs,
        } => Ok(IqNode::Aggregation {
            child: Box::new(resolve(*child, cx)?),
            grouping,
            aggs,
        }),
        IqNode::Distinct { child } => Ok(IqNode::Distinct {
            child: Box::new(resolve(*child, cx)?),
        }),
        IqNode::Slice {
            child,
            offset,
            limit,
        } => Ok(IqNode::Slice {
            child: Box::new(resolve(*child, cx)?),
            offset,
            limit,
        }),
        IqNode::OrderBy { child, keys } => Ok(IqNode::OrderBy {
            child: Box::new(resolve(*child, cx)?),
            keys,
        }),

        // ---- already-resolved leaves / identities pass through ------------------
        leaf @ (IqNode::Values { .. }
        | IqNode::Extensional { .. }
        | IqNode::Empty { .. }
        | IqNode::True
        | IqNode::Path { .. }) => Ok(leaf),
    }
}

/// Resolve each child of an n-ary node (every `Intensional` inside is replaced).
fn resolve_children(children: Vec<IqNode>, cx: &mut ResolveCx) -> Result<Vec<IqNode>> {
    children.into_iter().map(|c| resolve(c, cx)).collect()
}

/// Resolve a conjunction of [`IqCond`]s: the symbolic `Expr`/`Sql` leaves are left
/// untouched (FILTER/BIND are NOT resolved by RESOLVE); only the `EXISTS`/`NOT EXISTS`
/// subtrees recurse, so an `Intensional` embedded in a FILTER is resolved like any
/// other.
fn resolve_conds(conds: Vec<IqCond>, cx: &mut ResolveCx) -> Result<Vec<IqCond>> {
    conds.into_iter().map(|c| resolve_cond(c, cx)).collect()
}

/// Resolve one [`IqCond`] (design §3, recursion clause). `Expr`/`Sql` are symbolic
/// FILTER/ON leaves — passed through verbatim; `Exists`/`NotExists` recurse into their
/// built subtrees.
fn resolve_cond(cond: IqCond, cx: &mut ResolveCx) -> Result<IqCond> {
    match cond {
        IqCond::Expr(e) => Ok(IqCond::Expr(e)),
        IqCond::Sql(s) => Ok(IqCond::Sql(s)),
        IqCond::And(cs) => Ok(IqCond::And(resolve_conds(cs, cx)?)),
        IqCond::Or(cs) => Ok(IqCond::Or(resolve_conds(cs, cx)?)),
        IqCond::Not(c) => Ok(IqCond::Not(Box::new(resolve_cond(*c, cx)?))),
        IqCond::Exists(n) => Ok(IqCond::Exists(Box::new(resolve_existential(*n, cx)?))),
        IqCond::NotExists { inner, is_minus } => Ok(IqCond::NotExists {
            inner: Box::new(resolve_existential(*inner, cx)?),
            is_minus,
        }),
    }
}

/// [`resolve`] a `FILTER EXISTS` / `FILTER NOT EXISTS` / `MINUS` body: sets
/// [`Unfolder::in_existential`] for the duration of the recursion (restoring the
/// PREVIOUS value afterward — an EXISTS nested inside another EXISTS/MINUS body
/// stays `in_existential`, never wrongly reset to `false` on the way back out),
/// so the `Intensional` arm skips ADR-0034 D1/D2 for every pattern inside — see
/// the field's own doc comment on [`Unfolder`] for why that is sound here.
fn resolve_existential(node: IqNode, cx: &mut ResolveCx) -> Result<IqNode> {
    let saved = std::mem::replace(&mut cx.unfolder.in_existential, true);
    let out = resolve(node, cx);
    cx.unfolder.in_existential = saved;
    out
}

/// Bridge one resolved flat [`Branch`] to an [`IqNode`] arm (module docs; M3 design
/// §3.1, §3.3 fallback; M5 Wave 1 path extension). A **triple-pattern** branch only ever
/// populates `core` + `where_conds` + `bindings` (`atom`/`class_atoms` never set
/// `opts`/`path`/`agg`): the scans become [`IqNode::Extensional`] leaves, the WHERE conds
/// become [`IqCond::Sql`] join/constant conditions, and the bindings become the
/// [`IqNode::Construction`] substitution (the var → [`TermDef`] scope, design §3.2). A
/// **property-path** branch (M5 Wave 1) instead carries `path = Some(PathClosure)` with an
/// empty `core`: the bridge must NOT drop it (the old `..` rest pattern silently did), so
/// the relational body becomes the [`IqNode::Path`] closure leaf, with any `where_cond`
/// (the `?s PATH ?s` self-unify [`SqlCond::ColEq`] from `bind`) wrapping it in a `Filter`.
/// Either way the body sits under the same `Construction(bindings)`, so an outer
/// `InnerJoin`/`LeftJoin`/`Filter` composes over the path arm exactly as over a triple arm.
///
/// ADR-0034 D1: a `true` `branch.distinct` (set by `cascade::force_distinct_for_dup_
/// safety`, called per pattern in the `Intensional` arm below, BEFORE this bridge runs)
/// wraps the resulting `Construction` in `IqNode::Distinct` — the flag would otherwise
/// be silently dropped here (the old `..` rest pattern discarded it, same class of bug
/// M3's own doc comment above already fixed once for `path`). A single-arm `Distinct`
/// reaching the query's own spine directly is fine (`Plan::prepared_branches` pushes it
/// into that one branch's SQL); as one arm of several inside a `Union`, it always
/// reaches `iq::lower::lower_node`'s `Distinct => lower_as_subplan` arm (a `Union`'s
/// children are never processed via `lower_spine`'s peeling) — either way the SAME
/// mechanism ADR-0034 D2 already relies on for pooling.
fn bridge_branch(branch: Branch) -> IqNode {
    let Branch {
        core,
        bindings,
        where_conds,
        path,
        distinct,
        ..
    } = branch;

    let conds: Vec<IqCond> = where_conds.into_iter().map(IqCond::Sql).collect();

    let child = match path {
        // ---- property-path closure arm (M5 Wave 1) ------------------------------
        // The relational body is the recursive `PathClosure` leaf (empty `core` by
        // construction). A self-path `where_cond` (`?s PATH ?s` ⇒ `ColEq sf_s,sf_o`)
        // wraps the leaf in a `Filter` so LOWER pushes it via `apply_conds` after the
        // `Branch::path` leaf lowers (it never rides an `InnerJoin`, which has no scans).
        Some(closure) => {
            let leaf = IqNode::Path { closure };
            if conds.is_empty() {
                leaf
            } else {
                IqNode::Filter {
                    child: Box::new(leaf),
                    cond: conds,
                }
            }
        }
        // ---- triple-pattern arm (M3a) -------------------------------------------
        // The flat `core` + `where_conds`, as a CROSS JOIN + WHERE-eq InnerJoin
        // (`Extensional.bind` left empty — all join logic is carried by the explicit
        // `IqCond::Sql` conds, exactly mirroring the flat lowering).
        None => {
            let scans: Vec<IqNode> = core
                .into_iter()
                .map(|scan| IqNode::Extensional {
                    scan,
                    bind: BTreeMap::new(),
                })
                .collect();
            if scans.len() == 1 && conds.is_empty() {
                scans.into_iter().next().expect("len checked == 1")
            } else {
                IqNode::InnerJoin {
                    children: scans,
                    cond: conds,
                }
            }
        }
    };

    // The var → resolved-term scope (design §3.2): each flat binding becomes a
    // `BindDef::Resolved(TermDef)`; the arm projects exactly its bound variables.
    let project = bindings.keys().map(|v| v.as_str().into()).collect();
    let subst = bindings
        .into_iter()
        .map(|(v, td)| (v.into(), BindDef::Resolved(td)))
        .collect();

    let construction = IqNode::Construction {
        child: Box::new(child),
        subst,
        project,
    };
    if distinct {
        IqNode::Distinct {
            child: Box::new(construction),
        }
    } else {
        construction
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::build_tree;
    use crate::iq::node::IqNode;
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
    /// (refObjectMap → DEPT) — a representative single-/multi-map mapping.
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

    /// `emp`/`dept`'s schema, PK-keyed on `id` (matching `mapping()`'s own
    /// `{id}`-templated subjects) — ADR-0034 D1 forces `SELECT DISTINCT` on any
    /// unkeyed scan, so these structural, shape-focused tests must supply a
    /// schema proving `mapping()`'s tables ARE keyed, or every arm here would
    /// grow an extra `Distinct` wrapper unrelated to what each test examines.
    fn keyed_schema() -> Vec<sf_sql::TableSchema> {
        let mut emp = sf_sql::TableSchema::new("emp");
        emp.primary_key = vec!["id".to_owned()];
        let mut dept = sf_sql::TableSchema::new("dept");
        dept.primary_key = vec!["id".to_owned()];
        vec![emp, dept]
    }

    fn pattern(q: &str) -> GraphPattern {
        match spargebra::SparqlParser::new().parse_query(q).unwrap() {
            spargebra::Query::Select { pattern, .. } => pattern,
            other => panic!("expected SELECT, got {other:?}"),
        }
    }

    /// `true` iff any node in the tree is an `Intensional` (including inside
    /// FILTER EXISTS / NOT EXISTS subtrees).
    fn has_intensional(node: &IqNode) -> bool {
        match node {
            IqNode::Intensional { .. } => true,
            IqNode::Construction { child, .. }
            | IqNode::Distinct { child }
            | IqNode::Slice { child, .. }
            | IqNode::OrderBy { child, .. }
            | IqNode::Aggregation { child, .. } => has_intensional(child),
            IqNode::Filter { child, cond } => {
                has_intensional(child) || cond.iter().any(cond_has_intensional)
            }
            IqNode::InnerJoin { children, cond } => {
                children.iter().any(has_intensional) || cond.iter().any(cond_has_intensional)
            }
            IqNode::LeftJoin { left, right, cond } => {
                has_intensional(left)
                    || has_intensional(right)
                    || cond.iter().any(cond_has_intensional)
            }
            IqNode::Union { children, .. } => children.iter().any(has_intensional),
            // `UnresolvedPath` is the M5 path companion of `Intensional` — also a
            // transient leaf that must not survive RESOLVE; treated as a violation here so
            // the `resolve_leaves_zero_intensional` invariant covers paths too.
            IqNode::UnresolvedPath { .. } => true,
            IqNode::Values { .. }
            | IqNode::Extensional { .. }
            | IqNode::Empty { .. }
            | IqNode::True
            | IqNode::Path { .. } => false,
        }
    }

    fn cond_has_intensional(c: &IqCond) -> bool {
        match c {
            IqCond::Expr(_) | IqCond::Sql(_) => false,
            IqCond::And(cs) | IqCond::Or(cs) => cs.iter().any(cond_has_intensional),
            IqCond::Not(c) => cond_has_intensional(c),
            IqCond::Exists(n) | IqCond::NotExists { inner: n, .. } => has_intensional(n),
        }
    }

    /// The flat per-pattern arm count (the oracle this milestone reproduces).
    fn flat_arm_count(q: &str, maps: &[TriplesMap]) -> usize {
        let tp = match pattern(q) {
            GraphPattern::Project { inner, .. } => match *inner {
                GraphPattern::Bgp { mut patterns } => patterns.pop().unwrap(),
                other => panic!("expected single-triple BGP, got {other:?}"),
            },
            other => panic!("expected Project, got {other:?}"),
        };
        let tbox = Tbox::new();
        let schema = keyed_schema();
        let mut u = Unfolder::new(maps, &tbox, sf_sql::Dialect::Sqlite, &schema);
        u.resolve_pattern(&tp, None).unwrap().len()
    }

    /// The resolved-tree arm count for the same single-triple pattern: count the
    /// arms of the `Union` (≥2), or 1 for a bare arm, or 0 for `Empty`.
    fn resolved_arm_count(q: &str, maps: &[TriplesMap]) -> usize {
        let tbox = Tbox::new();
        let schema = keyed_schema();
        let mut cx = ResolveCx::new(maps, &tbox, sf_sql::Dialect::Sqlite, &schema);
        let tree = resolve(build_tree(&pattern(q), None).unwrap(), &mut cx).unwrap();
        // Strip the outer Project Construction the parser wraps `SELECT *` in.
        let inner = match tree {
            IqNode::Construction { child, .. } => *child,
            other => other,
        };
        match inner {
            IqNode::Empty { .. } => 0,
            IqNode::Union { children, .. } => children.len(),
            _ => 1,
        }
    }

    /// An `Intensional` resolves to the SAME arm count as the flat
    /// `pattern_branches` oracle — for a constant-predicate single-map pattern, a
    /// variable-predicate multi-arm pattern, an `rdf:type` class atom, and a pattern
    /// no map can serve (0 arms).
    #[test]
    fn resolve_arm_count_matches_flat_oracle() {
        let maps = mapping();
        for q in [
            // EMP :name ?n — one constant-predicate arm.
            "SELECT * WHERE { ?s <http://ex/name> ?n }",
            // ?s ?p ?o — every (class atom + POM) arm across both maps.
            "SELECT * WHERE { ?s ?p ?o }",
            // rdf:type ?c — the two rr:class atoms.
            &format!("SELECT * WHERE {{ ?s <{RDF_TYPE}> ?c }}"),
            // an unmapped predicate — zero arms.
            "SELECT * WHERE { ?s <http://ex/nope> ?o }",
        ] {
            assert_eq!(
                resolved_arm_count(q, &maps),
                flat_arm_count(q, &maps),
                "arm-count parity broken for {q}"
            );
        }
    }

    /// A `rr:refObjectMap` pattern bridges to a 2-scan `InnerJoin` (child scan ⋈
    /// parent scan) under the arm's `Construction` (design §3.1, §3.4).
    #[test]
    fn ref_object_map_resolves_to_two_scan_inner_join() {
        let maps = mapping();
        let tbox = Tbox::new();
        let schema = keyed_schema();
        let mut cx = ResolveCx::new(&maps, &tbox, sf_sql::Dialect::Sqlite, &schema);
        let q = "SELECT * WHERE { ?s <http://ex/dept> ?d }";
        let tree = resolve(build_tree(&pattern(q), None).unwrap(), &mut cx).unwrap();
        // Project Construction → the single arm Construction → InnerJoin of 2 scans.
        let inner = match tree {
            IqNode::Construction { child, .. } => *child,
            other => panic!("expected Project Construction, got {other:?}"),
        };
        let IqNode::Construction { child, .. } = inner else {
            panic!("expected arm Construction, got {inner:?}");
        };
        let IqNode::InnerJoin { children, cond } = *child else {
            panic!("expected 2-scan InnerJoin, got {child:?}");
        };
        assert_eq!(children.len(), 2, "refObjectMap → child ⋈ parent scan");
        assert!(
            children
                .iter()
                .all(|c| matches!(c, IqNode::Extensional { .. })),
            "both children are Extensional scans"
        );
        // The join condition is the rr:joinCondition ColEq, carried as IqCond::Sql.
        assert!(
            cond.iter().any(|c| matches!(c, IqCond::Sql(_))),
            "the refObjectMap join equality is carried as IqCond::Sql: {cond:?}"
        );
    }

    /// `resolve` leaves ZERO `Intensional` leaves anywhere in the tree, including
    /// inside a FILTER EXISTS subtree.
    #[test]
    fn resolve_leaves_zero_intensional() {
        let maps = mapping();
        let tbox = Tbox::new();
        for q in [
            "SELECT * WHERE { ?s <http://ex/name> ?n . ?s <http://ex/dept> ?d }",
            "SELECT * WHERE { ?s ?p ?o OPTIONAL { ?s <http://ex/name> ?n } }",
            "SELECT * WHERE { { ?s <http://ex/name> ?n } UNION { ?s <http://ex/dname> ?n } }",
            "SELECT * WHERE { ?s <http://ex/name> ?n FILTER EXISTS { ?s <http://ex/dept> ?d } }",
            "SELECT * WHERE { ?s <http://ex/name> ?n MINUS { ?s <http://ex/dept> ?d } }",
        ] {
            let schema = keyed_schema();
            let mut cx = ResolveCx::new(&maps, &tbox, sf_sql::Dialect::Sqlite, &schema);
            let tree = resolve(build_tree(&pattern(q), None).unwrap(), &mut cx).unwrap();
            assert!(
                !has_intensional(&tree),
                "Intensional survived resolve for {q}: {tree:?}"
            );
        }
    }

    /// FILTER (`IqCond::Expr`) and BIND (`BindDef::Expr`) survive RESOLVE untouched —
    /// they are resolved per-leaf-CQ at LOWER, never by RESOLVE (design §3, §5).
    #[test]
    fn filter_and_bind_survive_resolve_untouched() {
        let maps = mapping();
        let tbox = Tbox::new();
        let schema = keyed_schema();
        let mut cx = ResolveCx::new(&maps, &tbox, sf_sql::Dialect::Sqlite, &schema);
        // FILTER(?n > "5") stays an IqCond::Expr; BIND(?b := ?n) stays a BindDef::Expr.
        let q = "SELECT * WHERE { ?s <http://ex/name> ?n . BIND(?n AS ?b) FILTER(?n > \"5\") }";
        let tree = resolve(build_tree(&pattern(q), None).unwrap(), &mut cx).unwrap();
        assert!(
            symbolic_filter_present(&tree),
            "IqCond::Expr must survive: {tree:?}"
        );
        assert!(
            symbolic_bind_present(&tree),
            "BindDef::Expr must survive: {tree:?}"
        );
    }

    fn symbolic_filter_present(node: &IqNode) -> bool {
        match node {
            IqNode::Filter { child, cond } => {
                cond.iter().any(|c| matches!(c, IqCond::Expr(_))) || symbolic_filter_present(child)
            }
            IqNode::Construction { child, .. }
            | IqNode::Distinct { child }
            | IqNode::Slice { child, .. }
            | IqNode::OrderBy { child, .. }
            | IqNode::Aggregation { child, .. } => symbolic_filter_present(child),
            IqNode::InnerJoin { children, .. } | IqNode::Union { children, .. } => {
                children.iter().any(symbolic_filter_present)
            }
            IqNode::LeftJoin { left, right, .. } => {
                symbolic_filter_present(left) || symbolic_filter_present(right)
            }
            _ => false,
        }
    }

    fn symbolic_bind_present(node: &IqNode) -> bool {
        match node {
            IqNode::Construction { child, subst, .. } => {
                subst.values().any(|d| matches!(d, BindDef::Expr(_)))
                    || symbolic_bind_present(child)
            }
            IqNode::Filter { child, .. }
            | IqNode::Distinct { child }
            | IqNode::Slice { child, .. }
            | IqNode::OrderBy { child, .. }
            | IqNode::Aggregation { child, .. } => symbolic_bind_present(child),
            IqNode::InnerJoin { children, .. } | IqNode::Union { children, .. } => {
                children.iter().any(symbolic_bind_present)
            }
            IqNode::LeftJoin { left, right, .. } => {
                symbolic_bind_present(left) || symbolic_bind_present(right)
            }
            _ => false,
        }
    }
}
