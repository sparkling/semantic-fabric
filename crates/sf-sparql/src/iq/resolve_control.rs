//! Mapping/path leaf work, kept off the recursive RESOLVE stack frame.
use super::*;
use spargebra::term::{NamedNodePattern, TermPattern};

fn term_var(term: &TermPattern) -> Option<&str> {
    match term {
        TermPattern::Variable(v) => Some(v.as_str()),
        _ => None,
    }
}

fn named_var(term: &NamedNodePattern) -> Option<&str> {
    match term {
        NamedNodePattern::Variable(v) => Some(v.as_str()),
        _ => None,
    }
}

fn leaf_vars(candidates: &[Option<&str>], work: BuildWork<'_>) -> Result<Vec<Box<str>>> {
    let mut vars = work.vector(candidates.len())?;
    for candidate in candidates {
        work.charge(1)?;
        if let Some(candidate) = candidate {
            if !work.contains(&vars, candidate)? {
                vars.push(work.variable(candidate)?);
            }
        }
    }
    Ok(vars)
}

fn pattern_key(
    vars: &[Box<str>],
    work: BuildWork<'_>,
) -> Result<std::collections::HashSet<String>> {
    let mut keep = std::collections::HashSet::<String>::new();
    work.charge(vars.len())?;
    if let crate::CompilerWorkMode::Metered(cx) = work.mode {
        cx.reserve_checked_product(&[vars.len(), std::mem::size_of::<String>()])?;
    }
    keep.try_reserve(vars.len()).map_err(|_| {
        let cause = sf_core::query_control::QueryControlError::CompilerResourceExhausted;
        match work.mode {
            crate::CompilerWorkMode::Metered(cx) => cx.reject_build_resource(cause),
            crate::CompilerWorkMode::Uncontrolled => Error::QueryControl(cause),
        }
    })?;
    for name in vars {
        work.charge(name.len())?;
        for existing in &keep {
            work.charge(1)?;
            work.charge(name.len().min(existing.len()))?;
        }
        keep.insert(work.string(name)?);
        work.checkpoint()?;
    }
    Ok(keep)
}

#[inline(never)]
pub(super) fn resolve_leaf(node: IqNode, cx: &mut ResolveCx) -> Result<IqNode> {
    match node {
        // ---- the one resolving case ---------------------------------------------
        IqNode::Intensional { pattern, graph } => {
            // ADR-0035: a `GRAPH ?v` context contributes `v` to this leaf's own scope,
            // exactly as `IqNode::output_vars` already computes for the UNRESOLVED leaf
            // (`graph_pattern_var`) — needed here too since RESOLVE builds the arms'
            // `Empty`/`Union` wrapper directly, not via a later `output_vars()` call.
            let vars = leaf_vars(
                &[
                    term_var(&pattern.subject),
                    named_var(&pattern.predicate),
                    term_var(&pattern.object),
                    graph.as_ref().and_then(named_var),
                ],
                cx.work,
            )?;
            let mut branches = cx.unfolder.resolve_pattern(&pattern, graph.as_ref())?;
            // D1 may wrap a base Table scan in a compiler-generated Query to
            // enforce duplicate safety. Capture only each branch's pre-wrap
            // alias→source authority for the later D2 pooling gate; current
            // bindings continue to come from `branches`. The two vectors stay
            // index-aligned, and the wrapper changes SQL shape, not physical
            // source identity or column type.
            let mut source_authorities = cx.work.vector(branches.len())?;
            for branch in &branches {
                source_authorities.push(crate::cascade::PoolSourceAuthority::capture_with_work(
                    branch, cx.work,
                )?);
            }
            // ADR-0034 D1/D2 are both skipped inside a FILTER EXISTS / FILTER NOT
            // EXISTS / MINUS body — see `Unfolder::in_existential`'s own doc comment
            // for the SPARQL semantics that make this sound (an existence / anti-join
            // check is unaffected by within-body duplicate rows or duplicate-map
            // triples) and for why it also sidesteps a real, unrelated SubPlan-in-
            // correlated-subquery 501 boundary this would otherwise trip.
            let resolve_schema = if !cx.unfolder.in_existential {
                crate::cascade::build_resolve_schema(cx.unfolder.schema, cx.work)?
            } else {
                Vec::new()
            };
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
                crate::cascade::force_distinct_with_schema(
                    &mut branches,
                    &resolve_schema,
                    cx.unfolder.dialect,
                    cx.work,
                )?;
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
                && !all_pairwise_disjoint_with_work(&branches, cx.work)?
            {
                let groups = crate::unfold::disjoint_groups_with_work(&branches, cx.work)?;
                let keep = pattern_key(&vars, cx.work)?;
                // Shared-term fallback can emit one child per original arm, not
                // merely one per group. Reserve that logical upper bound first.
                let mut children = cx.work.vector(branches.len())?;
                let mut slots = cx.work.vector(branches.len())?;
                slots.extend(branches.into_iter().map(Some));
                let mut branches: Vec<Option<Branch>> = slots;
                for group in groups {
                    if group.len() == 1 {
                        children.push(bridge_branch(
                            branches[group[0]].take().expect("each index visited once"),
                            cx.work,
                        )?);
                        continue;
                    }
                    let mut members = cx.work.vector(group.len())?;
                    for &i in &group {
                        members.push(branches[i].take().expect("each index visited once"));
                    }
                    // ADR-0025 (sound-pooling shape): a positional pool this group's arms
                    // would otherwise need can hit PostgreSQL's own `UNION` type-resolver
                    // (a raw SQL error) or, if aligned via a `CAST`, silently drift a
                    // floating-point column's lexical form, and mutable startup type
                    // observations cannot prove different physical columns remain
                    // compatible — see `cascade::group_pool_type_safety`.
                    let mut member_refs = cx.work.vector(members.len())?;
                    member_refs.extend(members.iter());
                    let mut source_authority_refs = cx.work.vector(group.len())?;
                    source_authority_refs
                        .extend(group.iter().map(|&index| &source_authorities[index]));
                    if crate::cascade::group_pool_type_safety_with_schema(
                        &member_refs,
                        Some(&source_authority_refs),
                        &resolve_schema,
                        cx.unfolder.dialect,
                        cx.unfolder.column_type_use,
                        cx.work,
                    )? == crate::cascade::PoolTypeSafety::Unproven
                        || crate::cascade::group_needs_resolved_iri_with_work(
                            &members, &keep, cx.work,
                        )?
                    {
                        // Type-independent correctness fallback (mirrors
                        // `unfold::pool_pattern_relation`): when every member is
                        // standalone and its projected RDF terms can be deduplicated
                        // after reconstruction, skip SQL pooling — bridge each member
                        // SEPARATELY
                        // (exactly the `group.len() == 1` arm above, just for N members),
                        // tagged via `cx.unfolder`'s `dedup_groups` so `run_branches`
                        // shares ONE Rust-side term-dedup seen-set across them instead of
                        // a `Filter{Distinct{Union}}` SQL pool — no `UNION`, so the PG
                        // float-vs-text type-alignment wall never applies.
                        if crate::cascade::group_can_fallback_with_work(&members, &keep, cx.work)? {
                            crate::cascade::narrow_group_with_work(&mut members, &keep, cx.work)?;
                            let gid = cx.unfolder.alias()?;
                            for b in &members {
                                let mut aliases = b
                                    .core
                                    .iter()
                                    .chain(b.opts.iter().map(|opt| &opt.scan))
                                    .map(|scan| scan.alias);
                                let Some(alias) = aliases.next() else {
                                    return Err(Error::Unsupported(
                                        "D2 shared term-dedup arm has no representative source → 501"
                                            .to_owned(),
                                    ));
                                };
                                if aliases.next().is_some() {
                                    return Err(Error::Unsupported(
                                        "D2 shared term-dedup arm has multiple representative sources → 501"
                                            .to_owned(),
                                    ));
                                }
                                cx.unfolder.tag_dedup_group(alias, gid, b, &keep)?;
                            }
                            for member in members {
                                children.push(bridge_branch(member, cx.work)?);
                            }
                            continue;
                        }
                        return Err(Error::Unsupported(
                            "D2 pool: PostgreSQL column-type compatibility is not proven → 501 \
                             (a positional UNION could fail or drift lexically — ADR-0025)"
                                .to_owned(),
                        ));
                    }
                    let mut arms = cx.work.vector(members.len())?;
                    for member in members {
                        arms.push(bridge_branch(member, cx.work)?);
                    }
                    children.push(IqNode::Filter {
                        child: cx.work.boxed(IqNode::Distinct {
                            child: cx.work.boxed(IqNode::Union {
                                children: arms,
                                project: cx.work.variables(vars.iter().map(|v| v.as_ref()))?,
                            })?,
                        })?,
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
            let mut arms = cx.work.vector(branches.len())?;
            for branch in branches {
                arms.push(bridge_branch(branch, cx.work)?);
            }
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
            let vars = leaf_vars(
                &[
                    term_var(&subject),
                    term_var(&object),
                    graph.as_ref().and_then(named_var),
                ],
                cx.work,
            )?;
            let branches = cx
                .unfolder
                .resolve_path(&subject, &path, &object, graph.as_ref())?;
            let mut arms = cx.work.vector(branches.len())?;
            for branch in branches {
                arms.push(bridge_branch(branch, cx.work)?);
            }
            Ok(match arms.len() {
                0 => IqNode::Empty { vars },
                1 => arms.pop().expect("len checked == 1"),
                _ => IqNode::Union {
                    children: arms,
                    project: vars,
                },
            })
        }

        _ => unreachable!("only unresolved leaf nodes enter this helper"),
    }
}
