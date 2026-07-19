//! The whole-query composed-variable environment ([`StarEnv`]/[`ComposedInfo`])
//! and every "realization" consumer that turns an env-composed variable back
//! into something concrete on the way out: [`apply_composed_bindings`] (the
//! projection seam that installs a native `Term::Triple` reconstruction for a
//! `Branch`), [`substitute_construct_template`] (the CONSTRUCT-template
//! pre-substitution, R7), and the two projection-name-list helpers
//! ([`expand_projection_for_cascade`], [`all_component_var_names`]) `lib.rs`
//! consults so a composed variable's component columns survive cascade/tree
//! pruning long enough for realization to run. [`composed_info_for`] is the
//! env's own lookup-before-mint entry point, called by [`super::walk`] and
//! [`super::decompose`] wherever a new composed-variable binding site is
//! discovered.

use std::collections::{BTreeMap, HashSet};

use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern, Variable};

use crate::iq::{Branch, TermDef};
use crate::Plan;

use super::util::fresh_component_var;

/// ADR-0032 D3 §17.4.6 — the components of one composed (triple-term-valued)
/// SPARQL variable. Every field is a variable bound directly by a real query
/// pattern (never a `TermDef` — those don't exist yet at this AST-rewrite
/// stage): the 4 basic-encoding description patterns for the reifies-object-
/// variable case ([`super::walk::rewrite_triple`]'s new branch), the
/// decomposed columns for a VALUES ground triple ([`super::decompose::rewrite_values`]),
/// or a `TRIPLE(e1,e2,e3)` BIND's synthetic per-component `Extend`s
/// ([`super::decompose::rewrite_extend`]) — all three sites bind
/// `s_var`/`p_var`/`o_var` as ordinary variables via the ordinary unfold
/// machinery, so no new binding mechanism is needed downstream; only the
/// PROJECTION seam (`lib.rs`) needs to know they compose into a triple term.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposedInfo {
    pub s_var: Variable,
    pub p_var: Variable,
    pub o_var: Variable,
}

/// The whole-query variable → composed-info environment (ADR-0032 D3),
/// threaded mutably through the rewrite alongside the existing fresh-variable
/// counter. A `BTreeMap` (not a `HashMap`): iteration over it (the [`lib.rs`]
/// projection-override pass) must be deterministic, and `Variable` is
/// `Ord`-comparable (a thin wrapper over its name) with no reason to hash.
/// **Lookup-before-mint** everywhere a variable becomes composed: if the SAME
/// variable is independently composed from two syntactic positions in one
/// query (e.g. `?r rdf:reifies ?t` joined against a `VALUES ?t {...}` block on
/// the SAME `?t`), both sites MUST reuse the SAME `s_var`/`p_var`/`o_var` names
/// for the ordinary shared-variable join to correlate them — consult
/// [`StarEnv`] first, mint fresh component vars only on a genuine miss.
pub type StarEnv = BTreeMap<Variable, ComposedInfo>;

/// Look up `var`'s [`ComposedInfo`] in `env`, minting three fresh component
/// vars on a first sighting (lookup-before-mint, see [`StarEnv`]'s doc
/// comment for why reuse — not fresh minting per occurrence — is required
/// here).
pub(super) fn composed_info_for(var: &Variable, n: &mut usize, env: &mut StarEnv) -> ComposedInfo {
    env.entry(var.clone())
        .or_insert_with(|| ComposedInfo {
            s_var: fresh_component_var(n),
            p_var: fresh_component_var(n),
            o_var: fresh_component_var(n),
        })
        .clone()
}

/// Rule R7 (Wave 2b — ADR-0032 D2/D3 item 2): pre-substitute a CONSTRUCT
/// template so `exec_core::instantiate` never needs to know about [`StarEnv`]
/// at all. Every occurrence of an env-composed variable in subject or object
/// position (predicate structurally cannot compose — RDF 1.2 predicates are
/// always IRIs) is replaced with an explicit `TermPattern::Triple` over its
/// component vars, recursively: a component var that is ITSELF composed
/// (e.g. from a VALUES clause's recursive nested-triple decomposition, or a
/// nested `TRIPLE(...)` BIND) substitutes again, giving nested composition
/// for free — mirrors `exec_core::build_term`'s own recursion for
/// `TermDef::ComposedTriple`'s `object` field. Supersedes the old
/// `construct_template_has_quoted_triple` 501 guard (removed — see `lib.rs`).
pub fn substitute_construct_template(
    template: &[TriplePattern],
    env: &StarEnv,
) -> Vec<TriplePattern> {
    template
        .iter()
        .map(|tp| TriplePattern {
            subject: substitute_composed_term(&tp.subject, env),
            predicate: tp.predicate.clone(),
            object: substitute_composed_term(&tp.object, env),
        })
        .collect()
}

/// One CONSTRUCT-template term slot — see [`substitute_construct_template`].
fn substitute_composed_term(t: &TermPattern, env: &StarEnv) -> TermPattern {
    match t {
        TermPattern::Variable(v) => match env.get(v) {
            Some(info) => TermPattern::Triple(Box::new(TriplePattern {
                subject: substitute_composed_term(&TermPattern::Variable(info.s_var.clone()), env),
                predicate: NamedNodePattern::Variable(info.p_var.clone()),
                object: substitute_composed_term(&TermPattern::Variable(info.o_var.clone()), env),
            })),
            None => t.clone(),
        },
        other => other.clone(),
    }
}

/// ADR-0032 D3 item 2 — expand a SELECT's projected-variable NAME list with
/// every env-composed variable's component var names, recursively (nested
/// composition). `lib.rs` passes the RESULT (not the bare SELECT list) as
/// `cascade::CascadeCtx::project` wherever a query might carry composed
/// variables: `cascade`'s pass 7 (projection shrinking) runs BEFORE
/// [`apply_composed_bindings`] installs the `ComposedTriple` binding that
/// visibly "uses" a component var — a component that has NO OTHER consumer
/// (e.g. `SELECT ?t WHERE { VALUES ?t { <<(...)>> } }`, pure pass-through, no
/// join/filter references it either) would otherwise look, to pass 7, like a
/// dead column and be pruned before the override ever runs. Fixed-point
/// iteration (not a single pass) so a NESTED composed component (itself an
/// `env` key) also pulls in ITS OWN components.
pub fn expand_projection_for_cascade(vars: &[String], env: &StarEnv) -> Vec<String> {
    let mut out: Vec<String> = vars.to_vec();
    let mut changed = true;
    while changed {
        changed = false;
        for (var, info) in env {
            if out.iter().any(|v| v == var.as_str()) {
                for c in [&info.s_var, &info.p_var, &info.o_var] {
                    let name = c.as_str().to_owned();
                    if !out.contains(&name) {
                        out.push(name);
                        changed = true;
                    }
                }
            }
        }
    }
    out
}

/// ADR-0032 D3 item 2 — every component var name across the WHOLE `env`,
/// regardless of which specific SELECT variables reference them: the
/// coarser "extra keep" set [`crate::iq::lower::lower`] needs for the TREE
/// path's OWN internal `Construction` "restrict to project" retains, which
/// run even EARLIER than `cascade`'s pass 7 (before this query's actual
/// projected-variable set is known at that stage) — see that function's
/// `extra_keep` parameter doc comment. Deliberately coarser than
/// [`expand_projection_for_cascade`] (which is query-projection-aware): safe
/// because over-keeping a column here is harmless, and [`apply_composed_bindings`]
/// only ever reads a component var that a projected composed variable's
/// [`ComposedInfo`] actually names.
pub fn all_component_var_names(env: &StarEnv) -> HashSet<String> {
    let mut out = HashSet::new();
    for info in env.values() {
        out.insert(info.s_var.as_str().to_owned());
        out.insert(info.p_var.as_str().to_owned());
        out.insert(info.o_var.as_str().to_owned());
    }
    out
}

/// ADR-0032 D3 item 2 — the projection seam: for every [`StarEnv`]-composed
/// variable, install a [`TermDef::ComposedTriple`] binding for it in every
/// branch where its components are available, so `exec_core::reconstruct`
/// realizes a native `Term::Triple` (D2's "every visible surface" mandate).
/// Called from `lib.rs` once a `Plan`'s branches are otherwise finalized
/// (AFTER `unfold`/the tree `lower` pipeline AND the cascade — `unify::unify`
/// never has to know about `ComposedTriple` in practice, see its own doc
/// comment). Unconditionally OVERWRITES any pre-existing raw binding for the
/// variable (e.g. the reifies-bare-variable case's own real pf-IRI binding —
/// every real join/filter unification involving it is already done by this
/// point, so the raw binding has nothing further to participate in).
///
/// Recurses into every branch's `subplan_joins` (ADR-0023 M5 derived-table
/// pooling), SAFELY — see [`apply_composed_bindings_checked`]'s doc comment
/// for why a naive mirror of `lib.rs::cascade_subplans`' recursion shape is
/// UNSOUND here (an EMPIRICALLY CONFIRMED SQL-emission crash, not a
/// hypothetical) and what the guard does instead.
///
/// **Deeper cross-boundary gap — FIXED (F4a)**: when the composed variable
/// itself CROSSES the SubPlan boundary — i.e. it is one of the inner
/// sub-SELECT's own declared `vars` (so it participates in the outer query),
/// but its component vars are NOT (they are synthetic, never user-selected)
/// — the gap was NOT here at all. `iq::lower::lower_as_subplan` used to build
/// the outer branch's binding for that variable by remapping ONLY `vars`
/// (the SPARQL-declared projected names) through the derived table's
/// columns; the component vars, though kept alive INSIDE the inner branch by
/// `extra_keep`, were never among `vars` and so never reached the outer
/// branch AT ALL — no raw SQL column carried them across, and no recursion
/// run AFTER `lower_as_subplan` (which had already frozen the outer remap)
/// could retroactively fix that. Empirically confirmed (before the fix):
/// `SELECT ?t ?friend WHERE { ?p ex:knows ?friend . { SELECT DISTINCT ?t
/// WHERE { ?r rdf:reifies ?t } } }` against the `differential_star.rs`
/// CENSUS fixture lowered to an outer branch whose `bindings["t"]` was still
/// the RAW (pre-composition) template def — and, once the OUTER top-level
/// `apply_composed_bindings` pass here ALSO (redundantly, unguarded)
/// recomposed `?t` INSIDE the now-mutated inner SubPlan branch, the
/// EARLIER-frozen outer positional references desynced from the
/// (unrelatedly) changed derived-table shape, crashing at SQL execution with
/// "no such column" — not merely a wrong answer.
///
/// Fixed by making `lower_as_subplan` itself `StarEnv`-aware: it now builds
/// each `vars` entry's `ComposedTriple` from the ARM's own bindings (which DO
/// have the components, in the SAME inner scope) BEFORE remapping, reusing
/// `remap_termdef`'s `TermDef::ComposedTriple` arm (see its own doc comment)
/// to remap subject/predicate/object each to their own derived-table
/// position — the SAME single-pass `arm_projections` every other var already
/// uses, so there is no later, out-of-sync mutation to desync from.
/// `&StarEnv` is threaded alongside `extra_keep` through the
/// `lower`/`lower_spine`/`lower_node`/`lower_aggregation`/`lower_as_subplan`
/// chain (`iq/lower.rs`). This function's own recursion into `subplan_joins`
/// (above) and [`apply_composed_bindings_checked`]'s guard are UNCHANGED and
/// stay in place as defense — they simply become no-ops for a variable
/// `lower_as_subplan` already composed correctly.
pub fn apply_composed_bindings(branches: &mut [Branch], env: &StarEnv) {
    for branch in branches {
        apply_to_one_branch(branch, env);
        for sp in &mut branch.subplan_joins {
            propagate_single_branch_distinct(&mut sp.plan);
            for inner in &mut sp.plan.branches {
                apply_composed_bindings_checked(inner, env);
            }
        }
    }
}

/// Mirror [`Plan::prepared_branches`]'s single-branch DISTINCT propagation
/// (never persisted by that method itself — it returns a fresh clone) onto
/// `plan`'s OWN stored branch, PERMANENTLY. Needed so a `projection()` check
/// run directly on `plan.branches` (as [`apply_composed_bindings_checked`]
/// does, never going through `prepared_branches`) sees the SAME view
/// `iq::lower::lower_as_subplan` (which froze the outer positional column
/// remap) and the later, real SQL emission (`emit::emit_subplan_sql` →
/// `Plan::emitted` → `prepared_branches` again) both use. Without this, a
/// branch still showing its pre-propagation `distinct: false` could
/// UNDER-detect a real footprint change in that check: a bindings-column
/// composing away can be silently "backfilled" by a WHERE-condition
/// reference to the SAME column ([`Branch::projection`] only excludes
/// WHERE/JOIN-ON columns under `distinct: true`), which the TRUE
/// (post-propagation) view correctly excludes but a stale `distinct: false`
/// view would not — confirmed reachable by direct inspection of the SubPlan
/// this file's own doc comments cite as the empirical crash repro
/// (`apply_composed_bindings_checked`'s doc comment): its raw `distinct:
/// false` branch and its `distinct: true`-forced view disagree (10 columns
/// vs. 4) on exactly this branch. Harmless elsewhere: the final emission's
/// OWN `prepared_branches` call re-derives the identical value from
/// `plan.distinct` regardless of whether this ran first. A multi-branch
/// SubPlan needs no such propagation — `iq::lower::lower_as_subplan`'s own
/// multi-branch DISTINCT-narrowing already sets `distinct: true` on EVERY
/// arm directly (ADR-0025 Tier-2 gap 2).
pub(super) fn propagate_single_branch_distinct(plan: &mut Plan) {
    if plan.branches.len() == 1 {
        plan.branches[0].distinct = plan.distinct;
    }
}

/// Try composing every [`StarEnv`] variable in `branch.bindings` (recursing
/// into any FURTHER-nested `subplan_joins` the same way, each level guarded
/// independently), but keep the result ONLY if it does not change `branch`'s
/// [`Branch::projection`] — i.e. the exact raw-column list, same columns,
/// same order — otherwise discard the attempt and leave `branch` untouched.
///
/// **Why this guard exists (found empirically, not anticipated by the
/// original ask to "mirror `cascade_subplans`'s recursion shape")**: a
/// composed variable's OWN pre-composition binding (e.g. its raw
/// proposition-form template) may read raw columns nothing else in the
/// branch needs, while its `ComposedTriple` replacement reads its
/// components' OWN raw columns instead — columns already counted via THEIR
/// separate, `extra_keep`-kept bindings. Swapping the shape can therefore
/// shrink (or relocate) `projection()`'s deduplicated column list. That list
/// is exactly what `iq::lower::lower_as_subplan` used, EARLIER and ONCE, to
/// freeze the OUTER branch's positional column references
/// (`t{subplan_alias}.c{i}`) into that branch's OWN bindings — a frozen
/// snapshot this function has no way to reach or update (it runs strictly
/// afterward, from `lib.rs`, on the already-built `Plan`). A silent width
/// change here desyncs those already-frozen positions from the derived
/// table's ACTUAL (later re-emitted, ADR-0025 Tier-2 gap 2) SELECT list.
/// Confirmed by direct execution: `SELECT ?t ?friend WHERE { ?p ex:knows
/// ?friend . { SELECT DISTINCT ?t WHERE { ?r rdf:reifies ?t } } }` against
/// the CENSUS fixture, composing `?t` UNGUARDED inside its SubPlan branch,
/// shrank that branch's SQLite `DISTINCT` derived-table SELECT list from 4
/// columns to 2 (the ComposedTriple's `subject`/`object` columns already
/// counted via the separately-kept `__sf_star_0`/`__sf_star_2` bindings), and
/// the outer join's frozen `t6.c2`/`t6.c3` references then hit a real SQLite
/// `no such column` error at execution — a hard CRASH, not merely a wrong
/// answer. `cascade_subplans` avoids this identical hazard by running its
/// nested cascade with `project: None`, which disables the ONE pass
/// (projection shrinking) that could change a branch's column footprint;
/// this function has no equivalent lever (it always changes footprint when a
/// composed variable's own raw shape differs from its components'), so it
/// verifies safety directly instead. See [`apply_composed_bindings`]'s doc
/// comment for the closely related, NOT-closed-by-this-guard-either
/// cross-boundary gap.
pub(super) fn apply_composed_bindings_checked(branch: &mut Branch, env: &StarEnv) {
    let before = branch.projection();
    let mut candidate = branch.clone();
    apply_to_one_branch(&mut candidate, env);
    for sp in &mut candidate.subplan_joins {
        propagate_single_branch_distinct(&mut sp.plan);
        for inner in &mut sp.plan.branches {
            apply_composed_bindings_checked(inner, env);
        }
    }
    if candidate.projection() == before {
        *branch = candidate;
    }
}

/// Install every [`StarEnv`] variable's [`TermDef::ComposedTriple`] binding
/// that `branch.bindings` currently has the components for — no recursion
/// into `subplan_joins`, no safety check; see [`apply_composed_bindings`] /
/// [`apply_composed_bindings_checked`] for the two call sites that add those.
fn apply_to_one_branch(branch: &mut Branch, env: &StarEnv) {
    // Two passes (collect then insert) — inserting while iterating `env`
    // would be fine (env isn't mutated), but collecting first keeps the
    // borrow of `branch.bindings` used by `composed_term_def` read-only
    // for the whole scan, independent of the mutation that follows.
    let updates: Vec<(String, TermDef)> = env
        .keys()
        .filter_map(|var| {
            composed_term_def(var, env, &branch.bindings).map(|def| (var.as_str().to_owned(), def))
        })
        .collect();
    for (var, def) in updates {
        branch.bindings.insert(var, def);
    }
}

/// Build `var`'s [`TermDef::ComposedTriple`] by resolving its
/// [`ComposedInfo`]'s three component vars: a component that is ITSELF an
/// `env` key resolves via ANOTHER recursive call (nested composition,
/// independent of `env`'s name-sorted iteration order — this recursion does
/// not touch `env`'s iteration at all); otherwise its current binding is read
/// from `bindings` directly. `None` if a non-composed component isn't bound
/// in THIS branch — e.g. a `UNION` arm that never composed this variable (a
/// normal, branch-local absence; [`super::top_level::rewrite_union`]'s check
/// only rejects a variable BOTH arms mention but disagree on — a variable
/// only one arm binds at all is ordinary SPARQL UNION behavior).
pub(crate) fn composed_term_def(
    var: &Variable,
    env: &StarEnv,
    bindings: &BTreeMap<String, TermDef>,
) -> Option<TermDef> {
    let info = env.get(var)?;
    let component = |v: &Variable| -> Option<TermDef> {
        if env.contains_key(v) {
            composed_term_def(v, env, bindings)
        } else {
            bindings.get(v.as_str()).cloned()
        }
    };
    Some(TermDef::ComposedTriple {
        subject: Box::new(component(&info.s_var)?),
        predicate: Box::new(component(&info.p_var)?),
        object: Box::new(component(&info.o_var)?),
    })
}
