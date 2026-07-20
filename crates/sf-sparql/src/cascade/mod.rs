//! The optimizer cascade (ADR-0007 §Decision Outcome) — semantics-preserving
//! IQ→IQ rewrites over the unfolded base translation. **The order is
//! load-bearing** and runs after tier-0 elimination:
//!
//! 0. **tier-0 elimination** (up front) — a `refObjectMap` with no join inlines
//!    the parent IRI; a parent==child self-join on a PK collapses to a scan.
//! 1. **IRI-template-mismatch pruning** — drop branches whose key/constant
//!    equalities are provably unsatisfiable. *Must precede (2)* so empty branches
//!    are gone and the IRI-term equalities that license a self-join merge are
//!    established first.
//! 2. **self-join / self-left-join elimination** — merge two scans of the same
//!    table joined on a unique key (the same row); the left-join variant
//!    preserves the right-bound provenance. Also applied, independently of
//!    (0)-(6) below, WITHIN any `NOT EXISTS`/`EXISTS` correlated subquery's own
//!    locally-scoped scans (ADR-0023 optimizer-residue: the right-nested-OPTIONAL
//!    decomposition's anti-join branch re-derives its right side from scratch and
//!    can leave a redundant same-table self-join there even when the branch's own
//!    `core` is already clean).
//! 3. **functional-dependency inference** (transitive closure, through unions) —
//!    *must precede (4)*: eliminating a join is sound only once uniqueness **and**
//!    match-guarantee hold.
//! 4. **FK/PK join elimination** — drop a parent scan reached only for its PK via
//!    a NOT-NULL FK (referential integrity guarantees the match).
//! 5. **selection pushdown** — push FILTERs toward the scans.
//! 6. **distinct removal** — drop a DISTINCT already implied by a projected key.
//!
//! Every rule preserves `=_bag` w.r.t. the base translation and **fires only when
//! its integrity-constraint precondition is already established**. Where a
//! precondition cannot be proven (e.g. no schema is supplied), the pass is a
//! sound no-op — never an unsound transform (ADR-0007 *cascade invariants*).
//! All six passes now **fire when their precondition holds** (and are sound
//! no-ops otherwise): (3) derives a transitive-closure FD set, (4) drops a
//! parent scan reached only for its PK via a NOT-NULL FK, (5) hoists single-scan
//! selections toward their scans, (6) drops a DISTINCT implied by a projected
//! unique key. This is the Path-B engine-perf work (ADR-0013) under the hard
//! invariant that cost may choose only among `=_bag`-equivalent plans.

use sf_core::ir::{LogicalSource, Segment, TermMap, TermType};
use sf_sql::TableSchema;

use crate::iq::{collect_cond_cols, Branch, CmpOp, ColRef, Scan, SqlCond, TermDef};

/// `table name -> TableSchema` — built ONCE per [`run`] call ([`build_schema_map`])
/// and threaded to every constraint-driven pass below, so a pass's schema lookup is
/// an O(log n) binary search instead of an O(n) `schema.iter().find(...)` linear
/// scan repeated at every one of its call sites (some inside per-branch or
/// per-round loops — ADR-0024/M4 perf). A SORTED `Vec`, not a `HashMap`: a
/// mapping's schema is typically a handful of tables, small enough that a
/// `HashMap`'s constant-factor overhead (table allocation, `SipHash`) measurably
/// LOST to the plain linear scan in a criterion bench (the same finding as
/// `exec_core::ColIndex`) — a sorted `Vec` avoids both the allocation and the
/// hashing while still beating an O(n) scan at the largest mappings.
type SchemaMap<'a> = Vec<(&'a str, &'a TableSchema)>;

/// Build a [`SchemaMap`] over `schema` (see its doc comment).
fn build_schema_map(schema: &[TableSchema]) -> SchemaMap<'_> {
    let mut map: SchemaMap<'_> = schema.iter().map(|t| (t.name.as_str(), t)).collect();
    map.sort_unstable_by_key(|&(name, _)| name);
    map
}

/// Look up a table by name in a [`SchemaMap`] via binary search.
fn schema_map_get<'a>(map: &SchemaMap<'a>, name: &str) -> Option<&'a TableSchema> {
    map.binary_search_by_key(&name, |&(n, _)| n)
        .ok()
        .map(|pos| map[pos].1)
}

mod fd;
mod joinelim;
mod sameterm;
#[cfg(test)]
mod tests;
/// WS-FK — RedundantJoinFKTest oracle scenarios (ADR-0022, WAVE 1).
#[cfg(test)]
mod ws_fk;
/// WS-G — Ontop-parity oracle (ADR-0022): GREEN parity ports + `#[ignore]` WS-A specs.
#[cfg(test)]
mod ws_g;
/// WS-ST — SelfJoinSameTermsTest oracle scenarios (ADR-0022, WAVE 1).
#[cfg(test)]
mod ws_st;

/// Re-export `Fds` so `joinelim` and tests can use `super::Fds`.
pub use fd::Fds;
/// Re-export for tests that use `use super::*`.
pub use fd::{infer_functional_dependencies, single_col_keys};

/// Context the constraint-driven passes need beyond the per-branch shape:
/// whether a `DISTINCT` is requested and which variables the query projects
/// (resolved at the plan layer — pass (6) proves a projected key makes the
/// DISTINCT redundant). `project == None` ⇒ `SELECT *` / CONSTRUCT: every
/// binding is projected.
#[derive(Debug, Default, Clone, Copy)]
pub struct CascadeCtx<'a> {
    pub distinct: bool,
    pub project: Option<&'a [String]>,
}

/// Run the cascade over a bag-union of branches in the load-bearing order.
/// `schema` (by table name) is the catalog read that gates the constraint-driven
/// passes; an empty slice makes (2)–(4)/(6) sound no-ops. Pass (6) inspects
/// `ctx` and, for the single-branch case, records its DISTINCT decision on the
/// returned branch's [`Branch::distinct`] flag.
pub fn run(branches: Vec<Branch>, schema: &[TableSchema], ctx: &CascadeCtx) -> Vec<Branch> {
    let schema_map: SchemaMap = build_schema_map(schema);
    let mut out: Vec<Branch> = branches
        .into_iter()
        .filter_map(|mut b| {
            // A recursive property-path closure has no base scans to rewrite — the
            // constraint-driven passes are inapplicable; pass it through untouched.
            // A MINUS anti-join branch likewise carries a correlated `NotExists`
            // whose subquery scans the constraint passes do not model; pass it
            // through so a self-join/FK rewrite never silently corrupts the
            // correlation (the anti-join is already a sound base translation).
            // A GROUP BY + aggregates branch (SPARQL §11) carries an `Aggregation`
            // over its inner FROM/WHERE. The constraint-driven passes that touch
            // grouping/aggregate semantics stay skipped — FK/PK join elimination could
            // drop a table the GROUP BY key needs (it inspects bindings/conds, not
            // `b.agg`); `prune_iri_template_mismatch` would wrongly delete an empty
            // *implicit*-group branch that must still yield one row (COUNT ⇒ 0);
            // selection pushdown / DISTINCT-driven prunes are likewise not modelled.
            // But self-join elimination IS `=_bag`-safe on an aggregate branch: it
            // merges two scans of the SAME table on a unique key — a 1:1 self-join that
            // changes neither the group multiplicity nor the COUNT — and `rewrite_alias`
            // now follows the merge into `b.agg`'s key/argument `ColRef`s. This collapses
            // the redundant self-joins q6/q13 emit (routes×2, agency×2) to one scan of
            // each table, the same win Wave 1 brought to the multi-branch `rust_group`
            // path (which the single-branch aggregate path had been missing).
            if b.path.is_some() {
                return Some(b);
            }
            if b.agg.is_some() {
                self_join_elimination(&mut b, &schema_map);
                nullable_unique_self_join_elimination(&mut b, &schema_map);
                return Some(b);
            }
            // A branch carrying LEFT-JOINed / INNER-JOINed SubPlan derived tables
            // (`subplan_joins`, ADR-0023 M5 Wave 2 / Item 1d — e.g. a modifier
            // sub-SELECT as an OPTIONAL's right operand) bypasses the constraint-driven
            // passes, the same way `path` / `NotExists` branches above do. A SubPlan's
            // `on` correlation references outer scan/opt aliases, but NONE of the
            // scan-mutating passes below know about `subplan_joins`: `rewrite_alias`
            // (self-join / self-LEFT-join elimination) rewrites bindings / where_conds /
            // opts / agg but NOT `subplan_joins[_].on`, and `distinct_prune_unused_opts`
            // / `fd_self_join_elimination` / `joinelim` DROP scans a SubPlan's ON still
            // references — either dangles the correlation at a vanished alias (a "no such
            // column" crash at exec, ADR-0007). Rather than teach every pass about
            // `subplan_joins` (a broad, fragile surface — the exact composition class the
            // Item 1d rounds keep re-finding), skip them wholesale for these rare
            // branches: the passes are `=_bag`-preserving OPTIMIZATIONS, so forgoing them
            // only leaves a (correct) less-collapsed plan. The agg-over-UNION pushdown's
            // own SubPlan branch is core-empty (handled by the `b.agg` arm above, whose
            // self-join elimination is a no-op with no core scans), so this never blocks
            // that optimization. (`cascade_subplans` in `lib.rs` still recurses INTO each
            // SubPlan's own nested `Plan`, so the inside of the derived table is optimized
            // normally — only the OUTER branch's scan-mutating passes are skipped.)
            if !b.subplan_joins.is_empty() {
                return Some(b);
            }
            if branch_has_not_exists(&b) {
                // The constraint-driven passes below don't model a correlated
                // subquery's own scans, so they're skipped for this branch (see the
                // comment above) — but self-join elimination WITHIN the subquery's
                // own locally-scoped `scans`/`conds` (ADR-0023 optimizer-residue,
                // the Group-D-adjacent SQL-shape cosmetic wave) never touches
                // `b.core`/`b.opts`/bindings, so it's always safe to run here too
                // (e.g. the right-nested-OPTIONAL decomposition's `NOT EXISTS`
                // re-derives its right side from scratch and can leave a redundant
                // same-table self-join the outer branch's own self-join elimination
                // already collapsed).
                self_join_elimination_in_subqueries(&mut b.where_conds, &schema_map);
                return Some(b);
            }
            tier0_eliminate(&mut b, &schema_map); // 0
            if !prune_iri_template_mismatch(&b) {
                return None; // 1 — unsatisfiable branch
            }
            self_join_elimination(&mut b, &schema_map); // 2a (inner-join variant — unique key)
            nullable_unique_self_join_elimination(&mut b, &schema_map); // 2a-ext (nullable unique + IS NOT NULL)
            if !prune_iri_template_mismatch(&b) {
                return None; // 1b — contradiction exposed after merge
            }
            joinelim::lj_to_ij_fk_downgrade(&mut b, schema); // 2b-pre — LJ→IJ FK guarantee
            self_left_join_elimination(&mut b, &schema_map); // 2b (left-join variant — Q5)
            sameterm::same_terms_elimination(&mut b, ctx); // 2c (same terms under DISTINCT — ADR-0022)
            distinct_prune_unused_opts(&mut b, ctx); // 2d — DISTINCT-driven prune of unused OPTIONAL right
            fd_self_join_elimination(&mut b, &schema_map, ctx); // 2e — FD-driven self-join elim under DISTINCT
            let fds = fd::infer_functional_dependencies(&b, schema); // 3
            joinelim::fk_pk_join_elimination(&mut b, schema, &fds); // 4
            selection_pushdown(&mut b); // 5
            if !disjunction_intersection_simplify(&mut b) {
                return None; // 5b — disjunction empty intersection → unsatisfiable branch
            }
            Some(b)
        })
        .collect();
    // 6 — distinct removal. For a multi-branch bag union, DISTINCT cannot be proved
    // redundant per-branch, so it is *enforced* by deduplicating projected solutions
    // in `exec::for_each_solution` (the streaming core); this pass only *removes* a
    // DISTINCT a single branch's projected key already makes redundant (pushed into
    // the SQL). So the proof here applies only to the single-branch case.
    if ctx.distinct && out.len() == 1 && out[0].path.is_none() && out[0].agg.is_none() {
        out[0].distinct = true;
        // Only PROVE the DISTINCT redundant (and drop it) when the single branch is
        // free of LEFT-JOINed SubPlan derived tables. `distinct_removal` proves
        // redundancy from the core PK key alone (a projected injective binding reads a
        // non-null unique key ⇒ distinct output terms) — but a `left == true`
        // `SubPlanJoin` (a modifier sub-SELECT attached as an OPTIONAL's right operand,
        // ADR-0023 Item 1d) can MULTIPLY a core PK row into several output rows (its
        // solution multiset LEFT-JOINed on the correlation), so that proof is invalid:
        // dropping the DISTINCT would leave those duplicates in the bag (=_bag broken,
        // ADR-0007 — a silent wrong answer). The `!b.opts.is_empty()` early-return
        // inside `distinct_removal` guards the ordinary-OPTIONAL analogue but NOT
        // `subplan_joins` (the OPTIONAL-over-a-modifier-subplan right side lands in
        // `subplan_joins`, never `opts`), so gate the call here. We still set
        // `distinct = true` above so the single-branch DISTINCT is pushed into the SQL;
        // skipping only the *removal* keeps that DISTINCT in place (a correct, merely
        // un-optimized plan).
        if out[0].subplan_joins.is_empty() {
            distinct_removal(&mut out[0], &schema_map, ctx.project);
        }
    }
    // D1 (ADR-0034) does NOT run here. It runs much earlier, per pattern —
    // `unfold::bgp` and `iq::resolve`'s `Intensional` arm both call
    // `force_distinct_for_dup_safety` on a pattern's own just-resolved arms,
    // BEFORE either engine's own later projection-narrowing has any chance to
    // strip a key-covering variable a branch's bindings still needed to prove
    // safety. Running it AGAIN here, on the fully-assembled `out`, would be not
    // merely redundant but actively WRONG for the tree engine specifically:
    // `iq::lower`'s Construction-arm `project` restriction has, by this point,
    // already narrowed every branch down to the OUTER query's projected
    // variables — so a check here would see only `?o` for `SELECT ?o WHERE
    // {?p :name ?o}` and wrongly conclude "not covered" (found via
    // `differential_tree.rs`'s `r5_i_duplicate_union_arms` / `r5_iii_non_
    // unique_self_join`: this pass, run here, collapsed legitimately-different
    // `(?p,?o)` solutions sharing an `?o` value down to one before they ever
    // reached the outer UNION/self-join). `join_branches`/`merge` (flat) and
    // `bridge_branch` (tree) both carry a pattern's own `distinct` decision
    // forward through the rest of translation, so by the time `out` reaches
    // here every branch's flag is already final.
    // Projection shrinking: drop bindings not in the project list (pass 7).
    if let Some(project) = ctx.project {
        for b in &mut out {
            b.bindings.retain(|var, _| project.iter().any(|p| p == var));
        }
    }
    out
}

/// Whether a branch carries a MINUS anti-join (`NotExists`) or FILTER EXISTS
/// semi-join (`Exists`) anywhere in its `where_conds` — such branches bypass
/// the constraint-driven cascade passes.
fn branch_has_not_exists(b: &Branch) -> bool {
    fn has(c: &SqlCond) -> bool {
        match c {
            SqlCond::NotExists { .. } | SqlCond::Exists { .. } | SqlCond::PathExists { .. } => true,
            SqlCond::Not(c) => has(c),
            SqlCond::And(cs) | SqlCond::Or(cs) => cs.iter().any(has),
            _ => false,
        }
    }
    b.where_conds.iter().any(has)
}

// --- 0. tier-0 elimination -------------------------------------------------

/// Tier-0 (ADR-0007 step 4). The no-join refObjectMap inlining is already done
/// during unfold (the parent subject is built from the child row); the
/// parent==child PK self-join collapse is handled by pass (2). Kept as an
/// explicit, documented stage so the pipeline shape matches the ADR.
fn tier0_eliminate(_b: &mut Branch, _schema: &SchemaMap) {}

// --- 1. IRI-template-mismatch pruning -------------------------------------

/// Returns `false` if the branch is provably empty: a column is constrained by
/// two `=` constants with different values (the algebra-level disjointness the
/// ADR calls IRI-template-mismatch). Also propagates constants through
/// `ColEq` join equalities to fixpoint, detecting cross-alias contradictions
/// (e.g. `col2=1 AND col2=2 AND ColEq(t0.col2, t1.col2)` → unsatisfiable).
/// Sound: such a branch yields no rows.
fn prune_iri_template_mismatch(b: &Branch) -> bool {
    // Map column reference → constant it equals (first-seen wins).
    let mut known: Vec<(ColRef, String)> = Vec::new();

    // Seed from direct Cmp(col, Eq, val) conditions.
    for cond in &b.where_conds {
        if let SqlCond::Cmp(col, CmpOp::Eq, val) = cond {
            if let Some((_, prev)) = known.iter().find(|(c, _)| c == col) {
                if prev != val {
                    return false;
                }
            } else {
                known.push((col.clone(), val.clone()));
            }
        }
    }

    // Propagate through ColEq to fixpoint: if a=X and ColEq(a, b) then b=X.
    loop {
        let mut changed = false;
        for cond in &b.where_conds {
            let (lhs, rhs) = match cond {
                SqlCond::ColEq(a, c) => (a, c),
                _ => continue,
            };
            for (src, tgt) in [(lhs, rhs), (rhs, lhs)] {
                if let Some((_, val)) = known.iter().find(|(c, _)| c == src) {
                    let val = val.clone();
                    if let Some((_, prev)) = known.iter().find(|(c, _)| c == tgt) {
                        if *prev != val {
                            return false; // contradiction via join equality
                        }
                    } else {
                        known.push((tgt.clone(), val));
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    true
}

// --- 2. self-join / self-left-join elimination ----------------------------

/// Merge two core scans of the **same table** joined on a **non-nullable unique
/// key** (equal key value ⇒ same row, a 1:1 match → `=_bag` preserved). Fires only
/// when the schema proves the equated column is a single-column unique key **that
/// is NOT NULL** (ADR-0007: a nullable unique column is *not* a true key — the
/// `NULL = NULL ⇒ UNKNOWN` join already excludes its NULL rows, so collapsing to a
/// bare scan would re-admit them and break `=_bag`). Otherwise a no-op.
fn self_join_elimination(b: &mut Branch, schema: &SchemaMap) {
    // Single-column unique-key self-join elimination.
    while let Some((keep, drop, cond_idx)) = find_self_join_in(&b.core, &b.where_conds, schema) {
        // Remove exactly the key equality that licenses *this* merge (by index,
        // before the rewrite). Removing every trivial `x = x` would also drop a
        // genuine `?x :p ?x` self-comparison, which is an effective `IS NOT NULL`
        // guard and must survive (ADR-0007 R3/=_bag).
        b.where_conds.remove(cond_idx);
        rewrite_alias(b, drop, keep);
        b.core.retain(|s| s.alias != drop);
    }
    // Composite-key self-join elimination (all PK cols covered by cross-scan ColEqs).
    while let Some((keep, drop, mut idxs)) =
        find_composite_pk_self_join_in(&b.core, &b.where_conds, schema)
    {
        // Remove all licensing ColEq conditions, highest index first.
        idxs.sort_unstable_by(|a, c| c.cmp(a));
        for idx in idxs {
            b.where_conds.remove(idx);
        }
        rewrite_alias(b, drop, keep);
        b.core.retain(|s| s.alias != drop);
    }
}

/// Self-join elimination WITHIN a `NOT EXISTS`/`EXISTS` correlated subquery's own
/// locally-scoped `scans`/`conds` — the SAME two merge rules [`self_join_elimination`]
/// applies to a [`Branch`]'s `core`/`where_conds`, but scoped to the subquery. A
/// subquery's inner scans feed no outer `bindings`/`opts` (a `NOT EXISTS`/`EXISTS`
/// is a boolean condition, not a value source), so only `conds` needs rewriting —
/// no [`rewrite_alias`] (which also walks `bindings`/`opts`) is needed or correct
/// here (those belong to the OUTER branch, not this subquery).
fn self_join_elimination_in_subquery(
    scans: &mut Vec<Scan>,
    conds: &mut Vec<SqlCond>,
    schema: &SchemaMap,
) {
    let fix_alias = |from: usize, to: usize| {
        move |c: &mut ColRef| {
            if c.alias == from {
                c.alias = to;
            }
        }
    };
    while let Some((keep, drop, cond_idx)) = find_self_join_in(scans, conds, schema) {
        conds.remove(cond_idx);
        let fix = fix_alias(drop, keep);
        for cond in conds.iter_mut() {
            rewrite_cond_alias(cond, &fix);
        }
        scans.retain(|s| s.alias != drop);
    }
    while let Some((keep, drop, mut idxs)) = find_composite_pk_self_join_in(scans, conds, schema) {
        idxs.sort_unstable_by(|a, c| c.cmp(a));
        for idx in idxs {
            conds.remove(idx);
        }
        let fix = fix_alias(drop, keep);
        for cond in conds.iter_mut() {
            rewrite_cond_alias(cond, &fix);
        }
        scans.retain(|s| s.alias != drop);
    }
}

/// Recurse `conds` looking for a `NotExists`/`Exists` correlated subquery (through
/// `Not`/`And`/`Or` wrappers, mirroring [`branch_has_not_exists`]'s traversal) and
/// apply [`self_join_elimination_in_subquery`] to each one found — including,
/// defensively, any subquery NESTED inside another (the current lowering never
/// nests `NOT EXISTS`, but this must not silently skip one if it ever does).
fn self_join_elimination_in_subqueries(conds: &mut [SqlCond], schema: &SchemaMap) {
    for cond in conds.iter_mut() {
        match cond {
            SqlCond::NotExists { scans, conds } | SqlCond::Exists { scans, conds } => {
                self_join_elimination_in_subquery(scans, conds, schema);
                self_join_elimination_in_subqueries(conds, schema);
            }
            SqlCond::Not(c) => {
                self_join_elimination_in_subqueries(std::slice::from_mut(&mut **c), schema)
            }
            SqlCond::And(cs) | SqlCond::Or(cs) => self_join_elimination_in_subqueries(cs, schema),
            _ => {}
        }
    }
}

/// Returns `(keep_alias, drop_alias, where_cond_index)` of an eliminable
/// self-join within `scans`/`conds`, or `None`. A `ColEq` whose alias resolves
/// OUTSIDE `scans` (a correlated subquery's reference to an outer alias) is
/// skipped, not treated as a search-aborting failure — `scan_table_in` returning
/// `None` for such a `ColEq` must not stop the search from finding a LATER,
/// legitimately-in-scope self-join.
fn find_self_join_in(
    scans: &[Scan],
    conds: &[SqlCond],
    schema: &SchemaMap,
) -> Option<(usize, usize, usize)> {
    for (idx, cond) in conds.iter().enumerate() {
        let SqlCond::ColEq(a, c) = cond else { continue };
        if a.alias == c.alias || a.column != c.column {
            continue; // a same-alias `?x :p ?x` guard is not a self-join
        }
        let Some(ta) = scan_table_in(scans, a.alias) else {
            continue;
        };
        let Some(tc) = scan_table_in(scans, c.alias) else {
            continue;
        };
        if ta != tc {
            continue;
        }
        if let Some(t) = schema_map_get(schema, ta.as_str()) {
            if t.is_unique_key(&a.column) && key_is_non_null(t, &a.column) {
                // Keep the lower alias, drop the higher.
                let (keep, drop) = if a.alias < c.alias {
                    (a.alias, c.alias)
                } else {
                    (c.alias, a.alias)
                };
                return Some((keep, drop, idx));
            }
        }
    }
    None
}

/// Find `(keep, drop, cond_indices)` for a composite-PK self-join within
/// `scans`/`conds`: two scans of the same table where the set of cross-scan
/// `ColEq` conditions (same column name on both sides) covers every column of the
/// composite primary key. Such a join identifies the same row on both sides, so
/// the merge is `=_bag`-safe. All PK columns are `NOT NULL` by SQL semantics — no
/// nullable-key hazard.
fn find_composite_pk_self_join_in(
    scans: &[Scan],
    conds: &[SqlCond],
    schema: &SchemaMap,
) -> Option<(usize, usize, Vec<usize>)> {
    for i in 0..scans.len() {
        let LogicalSource::Table(ti) = &scans[i].source else {
            continue;
        };
        let ai = scans[i].alias;
        let Some(ts) = schema_map_get(schema, ti.as_str()) else {
            continue;
        };
        if ts.primary_key.len() < 2 {
            continue; // single-column handled by find_self_join_in
        }
        for scan_j in scans.iter().skip(i + 1) {
            let LogicalSource::Table(tj) = &scan_j.source else {
                continue;
            };
            if ti != tj {
                continue;
            }
            let aj = scan_j.alias;
            let (keep, drop) = if ai < aj { (ai, aj) } else { (aj, ai) };

            // Collect cross-scan ColEq conditions whose column is part of the composite PK.
            let mut pk_cols_covered: Vec<String> = Vec::new();
            let mut cond_idxs: Vec<usize> = Vec::new();
            for (idx, cond) in conds.iter().enumerate() {
                let SqlCond::ColEq(a, c) = cond else { continue };
                if a.column != c.column {
                    continue; // different column names — not a direct PK equality
                }
                let spans_pair =
                    (a.alias == ai && c.alias == aj) || (a.alias == aj && c.alias == ai);
                if !spans_pair {
                    continue;
                }
                let col = a.column.to_string();
                if ts.primary_key.contains(&col) && !pk_cols_covered.contains(&col) {
                    pk_cols_covered.push(col);
                    cond_idxs.push(idx);
                }
            }
            // All PK columns must be covered.
            if ts.primary_key.iter().all(|pk| pk_cols_covered.contains(pk)) {
                return Some((keep, drop, cond_idxs));
            }
        }
    }
    None
}

/// Collapse a self-**LEFT**-join — an `OPTIONAL` right side that is a provable 1:1
/// match of a kept core scan — by rebinding it onto that scan and dropping the
/// `LEFT JOIN` (the Q5 `?t a :Trip . OPTIONAL { ?t :headsign ?hs }` fix). Unlike
/// pass (2)'s inner-join variant the redundant side lives in [`Branch::opts`], not
/// `where_conds`, so this scans `opts`. Loops to a fixpoint to handle several
/// eliminable OPTIONALs; a no-op when no precondition holds.
///
/// `=_bag`-safe: the OPTIONAL reads the SAME base table as a kept core scan, joined
/// on a SINGLE shared NON-NULL unique key. Equal NON-NULL key value ⇒ the optional
/// row *is* the core row (exactly one, by uniqueness; the null-safe `ON`'s `IS NULL`
/// disjuncts are dead for a NOT-NULL key). The LEFT JOIN therefore always matches
/// 1:1 and only contributes columns of the already-read row — eliminating it
/// preserves multiplicities and every binding value. A nullable unique determinant
/// is *not* a true key (the null-safe `ON` would admit NULL rows the bare scan
/// re-reads differently → `=_bag` break), so the pass refuses (ADR-0007).
fn self_left_join_elimination(b: &mut Branch, schema: &SchemaMap) {
    while let Some((keep, opt_alias, opt_idx)) = find_self_left_join(b, schema) {
        // The ON lived on the OptJoin, never in `where_conds`; remove the whole
        // OptJoin, then rebind every reference to the optional scan onto the kept
        // scan (rewrite_alias recurses through Coalesce bindings).
        b.opts.remove(opt_idx);
        rewrite_alias(b, opt_alias, keep);
    }
    // Detect self-LJ where the right-side extra conditions contradict the core
    // WHERE conditions: since PK equality ⇒ same row, any column constant on the
    // right that conflicts with one on the left means the OPTIONAL never matches.
    lj_contradiction_elim(b, schema);
}

/// Drop OptJoins whose `extra` conditions contradict the core `where_conds` on
/// same-table, PK-joined (self-LJ) branches. On a self-LEFT-JOIN the ON key
/// equality guarantees that left and right sides read the SAME physical row; a
/// constant on the right (col = X) that disagrees with one on the left
/// (col = Y, X ≠ Y) is therefore impossible — the OPTIONAL never matches and
/// all right-side variables are always NULL. Sound to drop the OptJoin and
/// remove bindings that exclusively reference the vanished alias.
fn lj_contradiction_elim(b: &mut Branch, schema: &SchemaMap) {
    let mut i = 0;
    while i < b.opts.len() {
        if opt_has_pk_contradiction(b, i, schema) {
            let drop_alias = b.opts[i].scan.alias;
            b.opts.remove(i);
            // NULL-pad: drop bindings that reference only the vanished alias.
            b.bindings.retain(|_, def| {
                let cols = def.columns();
                !cols.iter().all(|c| c.alias == drop_alias)
            });
            // Do not advance i — recheck at the same slot.
        } else {
            i += 1;
        }
    }
}

/// Returns `true` when the OptJoin at `opt_idx` is a PK-keyed self-LEFT-JOIN
/// whose extra conditions contain a constant that contradicts a core WHERE
/// constant on the same column (same physical cell, impossible value).
fn opt_has_pk_contradiction(b: &Branch, opt_idx: usize, schema: &SchemaMap) -> bool {
    let opt = &b.opts[opt_idx];
    let LogicalSource::Table(opt_table) = &opt.scan.source else {
        return false;
    };
    let opt_alias = opt.scan.alias;
    // ON must be a single NullSafeEq/ColEq on the same column.
    let [cond] = opt.on.as_slice() else {
        return false;
    };
    let (SqlCond::NullSafeEq(a, c) | SqlCond::ColEq(a, c)) = cond else {
        return false;
    };
    if a.column != c.column {
        return false;
    }
    let keep = if a.alias == opt_alias && c.alias != opt_alias {
        c
    } else if c.alias == opt_alias && a.alias != opt_alias {
        a
    } else {
        return false;
    };
    // Kept side must be a core scan of the same table.
    if scan_table(b, keep.alias).as_deref() != Some(opt_table.as_str()) {
        return false;
    }
    // The shared key must be a NON-NULL unique key (ensures same-row identity).
    let Some(ts) = schema_map_get(schema, opt_table.as_str()) else {
        return false;
    };
    if !ts.is_unique_key(&keep.column) || !key_is_non_null(ts, &keep.column) {
        return false;
    }
    // Contradiction: same column name, different constants on kept vs opt sides.
    for extra in &opt.extra {
        let SqlCond::Cmp(ec, CmpOp::Eq, eval) = extra else {
            continue;
        };
        if ec.alias != opt_alias {
            continue;
        }
        let contradicts = b.where_conds.iter().any(|wc| {
            matches!(wc, SqlCond::Cmp(wc_col, CmpOp::Eq, wc_val)
                if wc_col.alias == keep.alias && wc_col.column == ec.column && wc_val != eval)
        });
        if contradicts {
            return true;
        }
    }
    false
}

/// Returns `(keep_alias, opt_alias, opt_index)` of an eliminable self-left-join, or
/// `None`. Fires only when ALL hold (else a sound no-op): the OptJoin right side is
/// a single base-table scan whose table a core scan also reads; `on` is EXACTLY one
/// shared-key compatibility condition (`NullSafeEq`/`ColEq`, same column on both
/// sides, one side the kept scan and the other the optional scan); that column is a
/// NON-NULL single-column unique key; and `extra` is empty (a FILTER inside the
/// OPTIONAL makes the match conditional → not always-matching → not eliminable).
fn find_self_left_join(b: &Branch, schema: &SchemaMap) -> Option<(usize, usize, usize)> {
    for (idx, opt) in b.opts.iter().enumerate() {
        let opt_alias = opt.scan.alias;
        // A FILTER inside the OPTIONAL can make the match conditional → keep it.
        // Exception: a lone `IS NOT NULL(col)` on the opt scan is not conditional in
        // the PK self-join case — because the same-row identity means the column has
        // the same value on the kept scan, and NULL propagates naturally after merge.
        let extra_ok = opt.extra.is_empty()
            || matches!(
                opt.extra.as_slice(),
                [SqlCond::IsNotNull(c)] if c.alias == opt_alias
            );
        if !extra_ok {
            continue;
        }
        // The right side must be a single base-table scan.
        let LogicalSource::Table(opt_table) = &opt.scan.source else {
            continue;
        };
        // Exactly one shared-key compatibility condition, same column on both sides.
        let [cond] = opt.on.as_slice() else { continue };
        let (SqlCond::NullSafeEq(a, c) | SqlCond::ColEq(a, c)) = cond else {
            continue;
        };
        if a.column != c.column {
            continue;
        }
        // One side on the optional scan, the other on a kept core scan.
        let keep = if a.alias == opt_alias && c.alias != opt_alias {
            c
        } else if c.alias == opt_alias && a.alias != opt_alias {
            a
        } else {
            continue;
        };
        // The kept side must be a core scan reading the SAME table.
        if scan_table(b, keep.alias).as_deref() != Some(opt_table.as_str()) {
            continue;
        }
        // The shared column must be a NON-NULL single-column unique key.
        if let Some(t) = schema_map_get(schema, opt_table.as_str()) {
            if t.is_unique_key(&keep.column) && key_is_non_null(t, &keep.column) {
                return Some((keep.alias, opt_alias, idx));
            }
        }
    }
    None
}

/// A key column is non-NULL iff it is a primary-key column (PK ⇒ NOT NULL by SQL
/// semantics) or the catalog declares the column `NOT NULL`. A nullable UNIQUE
/// column is therefore *not* treated as a safe self-join determinant (ADR-0007).
fn key_is_non_null(t: &TableSchema, col: &str) -> bool {
    let is_pk = t.primary_key.iter().any(|c| c == col);
    is_pk || t.column(col).is_some_and(|c| c.not_null)
}

/// The table name a scan reads, if it is a base table (`rr:tableName`).
fn scan_table(b: &Branch, alias: usize) -> Option<String> {
    scan_table_in(&b.core, alias)
}

/// [`scan_table`], generalized to any scan list (a [`Branch`]'s `core`, or a
/// `NOT EXISTS`/`EXISTS` correlated subquery's own `scans`).
fn scan_table_in(scans: &[Scan], alias: usize) -> Option<String> {
    scans
        .iter()
        .find(|s| s.alias == alias)
        .and_then(|s| match &s.source {
            LogicalSource::Table(t) => Some(t.clone()),
            LogicalSource::Query(_) => None,
        })
}

/// Rewrite every reference to alias `from` → `to` (bindings, conditions, opts).
fn rewrite_alias(b: &mut Branch, from: usize, to: usize) {
    let fix = |c: &mut ColRef| {
        if c.alias == from {
            c.alias = to;
        }
    };
    for def in b.bindings.values_mut() {
        rewrite_def_alias(def, from, to);
    }
    for cond in &mut b.where_conds {
        rewrite_cond_alias(cond, &fix);
    }
    for opt in &mut b.opts {
        for cond in opt.on.iter_mut().chain(opt.extra.iter_mut()) {
            rewrite_cond_alias(cond, &fix);
        }
    }
    // A GROUP BY + aggregates branch (§11) holds its grouping-key columns and
    // aggregate-argument columns as `ColRef`s on `b.agg`, OUTSIDE `bindings`/conds.
    // Self-join elimination now runs on aggregate branches (see `run`), so the merged
    // alias must be followed here too, or the GROUP BY key / COUNT argument would
    // dangle at the dropped scan. (`out` is the synthetic aggregate-result alias, never
    // a base scan being merged; rewriting it is a harmless no-op kept for uniformity.)
    if let Some(agg) = &mut b.agg {
        for key in &mut agg.keys {
            for c in &mut key.cols {
                fix(c);
            }
        }
        for a in &mut agg.aggs {
            if let Some(arg) = &mut a.arg {
                fix(arg);
            }
            fix(&mut a.out);
        }
    }
}

/// Rewrite the scan alias inside a term def (recursing through a `Coalesce`).
pub(super) fn rewrite_def_alias(def: &mut TermDef, from: usize, to: usize) {
    match def {
        TermDef::Const(_) => {}
        TermDef::Derived { alias, .. } => {
            if *alias == from {
                *alias = to;
            }
        }
        TermDef::Coalesce(l, r) => {
            rewrite_def_alias(l, from, to);
            rewrite_def_alias(r, from, to);
        }
        TermDef::Concat(parts) => {
            for p in parts {
                rewrite_def_alias(p, from, to);
            }
        }
        TermDef::Agg { col, .. } => {
            if col.alias == from {
                col.alias = to;
            }
        }
        // ADR-0032 D2: forced arm (new `TermDef` variant) — recurses like
        // `Coalesce`/`Concat`. Not reachable in practice (see `unify::unify`'s
        // identical note): a `ComposedTriple` binding is installed only by
        // `lib.rs`'s env-composed projection override, after this cascade pass runs.
        TermDef::ComposedTriple {
            subject,
            predicate,
            object,
        } => {
            rewrite_def_alias(subject, from, to);
            rewrite_def_alias(predicate, from, to);
            rewrite_def_alias(object, from, to);
        }
    }
}

fn rewrite_cond_alias(cond: &mut SqlCond, fix: &impl Fn(&mut ColRef)) {
    match cond {
        SqlCond::ColEq(a, b) | SqlCond::NullSafeEq(a, b) => {
            fix(a);
            fix(b);
        }
        SqlCond::Cmp(a, _, _) | SqlCond::IsNotNull(a) | SqlCond::IsNull(a) => fix(a),
        SqlCond::StrMatch { col, .. } => fix(col),
        SqlCond::Not(c) => rewrite_cond_alias(c, fix),
        SqlCond::And(cs) | SqlCond::Or(cs) => {
            for c in cs {
                rewrite_cond_alias(c, fix);
            }
        }
        // `NotExists` and `Exists` correlate on outer (left) aliases, which a
        // self-join merge may rename; recurse so those references track the kept
        // alias. (Inner scan aliases are globally unique and never a merge target.)
        SqlCond::NotExists { conds, .. }
        | SqlCond::Exists { conds, .. }
        | SqlCond::PathExists { conds, .. } => {
            for c in conds {
                rewrite_cond_alias(c, fix);
            }
        }
        // Run 4 Wave B3: apply `fix` to each side's alias via a throwaway `ColRef`
        // per `Segment::Column` (there is no single owned `ColRef` to hand `fix`
        // directly, unlike every other arm — the alias is stored once per side,
        // shared by every column in that side's template) and write the
        // (possibly rewritten) alias back; a self-join merge renames the WHOLE
        // alias uniformly, so every column agrees on the same new value.
        SqlCond::TemplateEq(sx, a1, sy, a2, _) => {
            for seg in sx {
                if let Segment::Column(c) = seg {
                    let mut cr = ColRef::new(*a1, c.clone());
                    fix(&mut cr);
                    *a1 = cr.alias;
                }
            }
            for seg in sy {
                if let Segment::Column(c) = seg {
                    let mut cr = ColRef::new(*a2, c.clone());
                    fix(&mut cr);
                    *a2 = cr.alias;
                }
            }
        }
    }
}

// --- 2c. same terms elimination — see `sameterm.rs` ----------------------

// --- 2d. DISTINCT-driven pruning of unused OPTIONAL right sides -----------

/// Pass 2d — under DISTINCT, drop any OPTIONAL (LEFT JOIN) right side whose
/// scan alias is not read by any *projected* binding.
///
/// Soundness (=_bag): under DISTINCT, if no projected binding reads the opt
/// scan, then for every core row:
///   * matches k opt rows  → k identical projected tuples → DISTINCT ⇒ 1
///   * matches 0 opt rows  → 1 NULL-extended projected tuple → same 1 row
///
/// So DISTINCT ∘ (core ⊕ opt) ≡ DISTINCT ∘ core on the projected columns.
///
/// The `extra` conditions are part of the LEFT JOIN ON clause — they cannot
/// filter core rows (the core always appears in a LEFT JOIN regardless of
/// whether the optional side matches), so dropping the OptJoin is safe even
/// when `extra` references core aliases.
fn distinct_prune_unused_opts(b: &mut Branch, ctx: &CascadeCtx) {
    if !ctx.distinct {
        return;
    }
    let Some(project) = ctx.project else {
        return;
    };
    b.opts.retain(|oj| {
        let opt_alias = oj.scan.alias;
        // Retain if any projected binding reads a column from the optional scan.
        // TermDef::columns() recurses through Coalesce / Concat for correctness.
        b.bindings.iter().any(|(var, def)| {
            project.iter().any(|p| p == var) && def.columns().iter().any(|c| c.alias == opt_alias)
        })
    });
}

// --- 2e. FD-driven self-join elimination under DISTINCT -------------------

/// Pass 2e — when two core scans of the same table are inner-joined on a single
/// column `C` that is a non-unique FD determinant (`C → dep1, dep2, …`) and every
/// binding that reads from the second scan reads only columns within `{C} ∪ {dep}`,
/// the second scan is redundant under DISTINCT.
///
/// Soundness (`=_bag`): under DISTINCT, n² identical projected tuples from the
/// self-join and n tuples from the single scan both deduplicate to the same set.
/// Without DISTINCT the counts differ (n² ≠ n), so the guard is required.
///
/// **1b — FD-based self-LEFT-JOIN (OPTIONAL) elimination.** When an OPTIONAL right
/// side is a scan of the SAME table as a core scan, joined on a NOT-NULL FD determinant
/// column `C`, and all opt-scan bindings use only `{C} ∪ {dep}`, the OPTIONAL is
/// redundant under DISTINCT. The NOT-NULL guard ensures the FD applies to every row
/// (FDs are vacuously true for NULL determinants in SQL, so a NULL `C` would not
/// constrain the dep columns → the opt could produce different dep values from
/// different rows → elimination would be unsound). The same-table guarantee means
/// every opt row IS a row of the core table; under DISTINCT the FD forces all
/// opt-produced projected values to equal the core-produced values.
///
/// **1c — nullable-determinant IS-NOT-NULL synthesis.** For the inner-join case: an
/// equi-join on a nullable `C` excludes NULL rows (`NULL = NULL ⇒ UNKNOWN`). After
/// merging to a single scan, NULL rows would be re-admitted, breaking `=_bag`. A
/// synthetic `IS NOT NULL(C)` guard on the kept alias restores the exclusion.
fn fd_self_join_elimination(b: &mut Branch, schema: &SchemaMap, ctx: &CascadeCtx) {
    if !ctx.distinct {
        return;
    }
    // 2e inner-join case (with 1c nullable-det IS-NOT-NULL synthesis).
    while let Some((keep, drop, cond_idx)) = find_fd_self_join(b, schema) {
        // Extract det_col name before removing the condition.
        let det_col: Box<str> = {
            let SqlCond::ColEq(a, _) = &b.where_conds[cond_idx] else {
                unreachable!("find_fd_self_join returns a ColEq index");
            };
            a.column.clone()
        };
        // 1c: if the determinant is nullable, the equi-join excluded NULL rows.
        // Synthesise IS NOT NULL to preserve that exclusion after the merge.
        let is_nullable = scan_table(b, keep)
            .and_then(|tbl| schema_map_get(schema, tbl.as_str()))
            .is_some_and(|ts| !key_is_non_null(ts, &det_col));

        b.where_conds.remove(cond_idx);
        rewrite_alias(b, drop, keep);
        b.core.retain(|s| s.alias != drop);

        if is_nullable {
            let col = ColRef::new(keep, det_col);
            if !b
                .where_conds
                .iter()
                .any(|c| matches!(c, SqlCond::IsNotNull(r) if r == &col))
            {
                b.where_conds.push(SqlCond::IsNotNull(col));
            }
        }
    }
    // 1b: FD-based self-LEFT-JOIN (OPTIONAL) elimination under DISTINCT.
    let mut i = 0;
    while i < b.opts.len() {
        if let Some(keep) = find_fd_self_left_join(b, schema, i) {
            let opt_alias = b.opts[i].scan.alias;
            b.opts.remove(i);
            rewrite_alias(b, opt_alias, keep);
            // Restart from the beginning — removing shifts all subsequent indices.
            i = 0;
        } else {
            i += 1;
        }
    }
}

/// Returns the core alias to keep when the OPTIONAL at `opt_idx` qualifies for
/// FD-based self-left-join elimination under DISTINCT (wave 1b).
///
/// All five preconditions must hold:
/// 1. `opt.extra` is empty (a non-empty extra makes the match conditional).
/// 2. `opt.on` is a single `(Null)SafeEq` or `ColEq` with the SAME column name on
///    both sides (the FD determinant `det_col`).
/// 3. One side of the ON condition is the opt scan; the other is a core scan of the
///    SAME table (self-join identity).
/// 4. `det_col` is NOT NULL on that table — necessary because FDs are defined on
///    non-NULL values; a NULL `det_col` would not constrain the dep columns, so
///    different rows could have the same `NULL` det but different dep values.
/// 5. The schema declares `det_col` as a non-unique FD determinant (`det_col → dep`),
///    and ALL bindings that reference the opt scan use only columns in `{det_col} ∪ dep`.
fn find_fd_self_left_join(b: &Branch, schema: &SchemaMap, opt_idx: usize) -> Option<usize> {
    let opt = &b.opts[opt_idx];
    let opt_alias = opt.scan.alias;

    // Precondition 1: no extra conditions (a non-empty extra makes the match conditional).
    if !opt.extra.is_empty() {
        return None;
    }

    // Precondition 2: exactly one ON condition, (Null)SafeEq or ColEq, same column.
    let [cond] = opt.on.as_slice() else {
        return None;
    };
    let (a, c) = match cond {
        SqlCond::NullSafeEq(a, c) | SqlCond::ColEq(a, c) => (a, c),
        _ => return None,
    };
    if a.column != c.column {
        return None;
    }

    // Precondition 3: one side on the opt scan, the other on a core scan of the same table.
    let (det_col, core_alias) = if a.alias == opt_alias && c.alias != opt_alias {
        (&a.column, c.alias)
    } else if c.alias == opt_alias && a.alias != opt_alias {
        (&c.column, a.alias)
    } else {
        return None;
    };
    let LogicalSource::Table(opt_table) = &opt.scan.source else {
        return None;
    };
    if scan_table(b, core_alias).as_deref() != Some(opt_table.as_str()) {
        return None;
    }

    // Precondition 4: det_col must be NOT NULL (FDs only constrain non-NULL rows).
    let ts = schema_map_get(schema, opt_table.as_str())?;
    if !key_is_non_null(ts, det_col) {
        return None;
    }

    // Precondition 5: det_col is a declared FD determinant, and all opt bindings
    // are confined to {det_col} ∪ fd.dep.
    let fd = ts
        .functional_dependencies
        .iter()
        .find(|fd| fd.det.len() == 1 && fd.det[0].as_str() == &**det_col)?;
    let allowed: Vec<&str> = fd
        .dep
        .iter()
        .map(|s| s.as_str())
        .chain(fd.det.iter().map(|s| s.as_str()))
        .collect();
    let all_ok = b.bindings.values().all(|def| {
        def.columns()
            .iter()
            .filter(|c| c.alias == opt_alias)
            .all(|c| allowed.contains(&&*c.column))
    });
    if all_ok {
        Some(core_alias)
    } else {
        None
    }
}

fn find_fd_self_join(b: &Branch, schema: &SchemaMap) -> Option<(usize, usize, usize)> {
    for i in 0..b.core.len() {
        for j in (i + 1)..b.core.len() {
            let (alias_i, alias_j) = (b.core[i].alias, b.core[j].alias);
            let (LogicalSource::Table(tbl_i), LogicalSource::Table(tbl_j)) =
                (&b.core[i].source, &b.core[j].source)
            else {
                continue;
            };
            if tbl_i != tbl_j {
                continue;
            }
            let ts = schema_map_get(schema, tbl_i.as_str())?;
            // Require exactly one ColEq joining the two scans on the SAME column name.
            let Some((cond_idx, det_col)) =
                b.where_conds.iter().enumerate().find_map(|(idx, c)| {
                    if let SqlCond::ColEq(a, cv) = c {
                        if a.column == cv.column
                            && ((a.alias == alias_i && cv.alias == alias_j)
                                || (a.alias == alias_j && cv.alias == alias_i))
                        {
                            Some((idx, a.column.clone()))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
            else {
                continue;
            };
            // Must be a declared non-unique FD determinant (not just a PK/unique key,
            // which is handled by the =_bag-safe pass 2a without the DISTINCT guard).
            let Some(fd) = ts
                .functional_dependencies
                .iter()
                .find(|fd| fd.det.len() == 1 && fd.det[0].as_str() == &*det_col)
            else {
                continue;
            };
            // The allowed column set for the scan to be dropped: {det_col} ∪ fd.dep.
            let allowed: Vec<&str> = fd
                .dep
                .iter()
                .map(|s| s.as_str())
                .chain(fd.det.iter().map(|s| s.as_str()))
                .collect();
            // Try both orientations: drop scan j (keep i), then drop scan i (keep j).
            for &(keep, drop) in &[(alias_i, alias_j), (alias_j, alias_i)] {
                let all_ok = b.bindings.values().all(|def| {
                    def.columns()
                        .iter()
                        .filter(|c| c.alias == drop)
                        .all(|c| allowed.contains(&&*c.column))
                });
                if all_ok {
                    return Some((keep, drop, cond_idx));
                }
            }
        }
    }
    None
}

// --- 3. functional-dependency inference — see `fd.rs` ---------------------

// --- 5/6 + helpers live below; pass 4 is in `joinelim`. ------------------

// --- 5. selection pushdown -------------------------------------------------

/// Push FILTERs toward their scans at the algebra level (the source optimizer
/// then does the *physical* pushdown — ADR-0006: the source DB does the
/// set-work). Two semantics-preserving steps: flatten nested `AND`s so every
/// conjunct is an independent, pushable predicate; then **stable-hoist**
/// single-scan selections (`col <op> ?`, `IS [NOT] NULL`, a `?x :p ?x` self
/// guard) ahead of the multi-scan join equalities, so each scan's restriction
/// sits next to it in the emitted conjunction. A conjunction is commutative, so
/// reordering preserves `=_bag`; R5 is respected because OPTIONAL `ON`/`extra`
/// conditions live on [`crate::iq::OptJoin`], never in `where_conds` — an outer
/// FILTER is never pushed onto the preserved side.
fn selection_pushdown(b: &mut Branch) {
    let mut flat = Vec::new();
    for cond in std::mem::take(&mut b.where_conds) {
        flatten_and(cond, &mut flat);
    }
    // Stable partition: single-scan selections first, joins after.
    let (mut sels, joins): (Vec<SqlCond>, Vec<SqlCond>) =
        flat.into_iter().partition(is_single_scan_selection);
    sels.extend(joins);
    b.where_conds = sels;
}

fn flatten_and(cond: SqlCond, out: &mut Vec<SqlCond>) {
    match cond {
        SqlCond::And(cs) => {
            for c in cs {
                flatten_and(c, out);
            }
        }
        other => out.push(other),
    }
}

/// A predicate over the columns of a single scan (a restriction the source can
/// push to that scan), as opposed to a cross-scan join equality.
fn is_single_scan_selection(cond: &SqlCond) -> bool {
    let mut aliases = Vec::new();
    collect_cond_cols(cond, &mut |c| {
        if !aliases.contains(&c.alias) {
            aliases.push(c.alias);
        }
    });
    aliases.len() <= 1
}

// --- 6. distinct removal ---------------------------------------------------

/// Returns `true` when `def`'s term construction is injective — distinct
/// source-column tuples always produce distinct output terms — so that a key
/// column in the binding implies no two solution rows share the same term.
///
/// `TermMap::Column` is trivially injective (column value → term, bijection).
/// `TermMap::Template` with adjacent column slots is **not** injective (see
/// [`Template::is_injective`]); for non-IRI templates only a single column
/// slot is safe because the lack of percent-encoding means a separator
/// character can appear in a column value, breaking uniqueness.
///
/// `pub(crate)`: also the gate `unfold::group_key_columns` and
/// `iq::lower::try_sql_group_over_union` use before treating a `GROUP BY` key's
/// raw columns as equivalent to grouping by the constructed term (a distinct
/// injectivity concern from the one this fn was written for — DISTINCT-removal
/// — but the same underlying soundness condition).
pub(crate) fn binding_is_injective(def: &TermDef) -> bool {
    let TermDef::Derived {
        term_map: TermMap::Template(t, spec),
        ..
    } = def
    else {
        return true; // Column / Constant / Coalesce / Concat / Agg — not gated
    };
    if spec.term_type == TermType::Iri {
        t.is_injective()
    } else {
        // Literal/BlankNode: no percent-encoding, so only a single-column
        // template is unambiguously injective.
        t.segments()
            .iter()
            .filter(|s| matches!(s, Segment::Column(_)))
            .count()
            <= 1
    }
}

/// Drop a `DISTINCT` already implied by a **projected unique key** (DISTINCT over
/// a key is a no-op — R4: never *add* DISTINCT, only remove a provably redundant
/// one). Sound proof: the branch is a single base-table scan with no OPTIONAL, so
/// output rows are a subset of source rows (FILTERs only remove); the scan has a
/// single-column unique key `K`; and a **projected** variable's term is built
/// from `K`, so distinct source rows (distinct `K`) yield distinct solution
/// tuples — the DISTINCT removes nothing. `project == None` ⇒ every binding is
/// projected (`SELECT *` / CONSTRUCT). Any join/OPTIONAL ⇒ a non-key projection
/// could hide duplicates ⇒ no-op.
fn distinct_removal(b: &mut Branch, schema: &SchemaMap, project: Option<&[String]>) {
    if !b.distinct || b.core.is_empty() || !b.opts.is_empty() {
        return;
    }
    let projected = |var: &str| project.is_none_or(|p| p.iter().any(|v| v == var));
    if b.core.len() == 1 {
        let scan = &b.core[0];
        let LogicalSource::Table(table) = &scan.source else {
            return;
        };
        let Some(ts) = schema_map_get(schema, table.as_str()) else {
            return;
        };
        // Only a NOT-NULL single-column key proves DISTINCT redundant: a nullable
        // UNIQUE column permits multiple NULL rows (SQL UNIQUE allows many NULLs), and
        // build_term emits an unbound solution per NULL row — so `SELECT K` keeps both
        // NULL rows while `SELECT DISTINCT K` collapses them. Dropping the DISTINCT
        // would then ADD a row vs the base (=_bag broken). Mirror pass (2)'s
        // `key_is_non_null` guard (ADR-0007 cascade invariants).
        let keys: Vec<String> = single_col_keys(ts)
            .into_iter()
            .filter(|k| key_is_non_null(ts, k))
            .collect();
        // Single-column key: any projected injective binding reads a unique key col.
        let redundant_single = b.bindings.iter().any(|(var, def)| {
            projected(var)
                && binding_is_injective(def)
                && keys
                    .iter()
                    .any(|k| def.columns().contains(&ColRef::new(scan.alias, k.clone())))
        });
        // Composite PK: a projected injective Template binding whose Column slots
        // cover ALL PK columns — distinct PK tuples ⇒ distinct output terms.
        // PK columns are always NOT NULL (PK ⇒ NOT NULL), so no nullable-key hazard.
        let redundant_composite = !redundant_single
            && ts.primary_key.len() > 1
            && b.bindings.iter().any(|(var, def)| {
                if !projected(var) || !binding_is_injective(def) {
                    return false;
                }
                let TermDef::Derived {
                    term_map: TermMap::Template(t, _),
                    alias,
                } = def
                else {
                    return false;
                };
                if *alias != scan.alias {
                    return false;
                }
                // Every PK column must appear as a Column slot in the template.
                let template_cols: Vec<&str> = t
                    .segments()
                    .iter()
                    .filter_map(|s| match s {
                        Segment::Column(c) => Some(c.as_ref()),
                        _ => None,
                    })
                    .collect();
                ts.primary_key
                    .iter()
                    .all(|pk| template_cols.contains(&pk.as_str()))
            });
        if redundant_single || redundant_composite {
            b.distinct = false;
        }
    } else {
        // Multi-scan proof: every scan's non-null PK must be covered by a projected
        // injective binding. Distinct PKs on each side ⇒ distinct combined output tuples
        // (any two rows that agree on all projected variables must share the same PK on
        // every scan ⇒ they ARE the same row combination ⇒ no duplicates).
        let redundant_multi = b.core.iter().all(|scan| {
            let LogicalSource::Table(table) = &scan.source else {
                return false;
            };
            let Some(ts) = schema_map_get(schema, table.as_str()) else {
                return false;
            };
            !ts.primary_key.is_empty()
                && ts.primary_key.iter().all(|pk_col| {
                    b.bindings.iter().any(|(var, def)| {
                        projected(var)
                            && binding_is_injective(def)
                            && def.columns().iter().any(|c| {
                                c.alias == scan.alias && c.column.as_ref() == pk_col.as_str()
                            })
                    })
                })
        });
        if redundant_multi {
            b.distinct = false;
        }
    }
}

// --- D1 (ADR-0034) — per-scan DISTINCT wrap when duplicates are not provably
// impossible ----------------------------------------------------------------

/// D1 (ADR-0034; Run 4 Wave C0b Item 1): make every branch whose joined tables are
/// not provably duplicate-free ([`scan_key_covered`]) duplicate-safe — the SPARQL
/// §18.3 BGP set-semantics requirement (card[μ] = 1), independent of any
/// user-requested DISTINCT and of how many branches the query has (a branch's own
/// duplicate-safety does not depend on its siblings; D2, the CROSS-branch case, is
/// handled separately at the pattern-relation boundary — `unfold::pool_pattern_
/// relation` / `iq::resolve`'s `Intensional` arm — before branches ever reach here).
/// MUST run before projection shrinking (pass 7 in [`run`]) so the check sees each
/// branch's FULL binding set, not the outer SELECT's — dropping `?p` from the
/// projection of `SELECT ?age WHERE { ?p :hasAge ?age }` must never retroactively
/// make the BGP itself look duplicate-laden (that apparent duplication is a
/// legitimate consequence of projection, ADR-0034: "never the final result").
/// Never touches a branch already `distinct` (whatever set it stays); skips `path`
/// (a closure already self-dedups via its own `SELECT DISTINCT sf_s, sf_o`) and
/// `agg` (dedup happens BELOW the GROUP BY — see [`dedup_before_aggregate`]).
/// Does NOT skip a branch merely for carrying `subplan_joins`: a D2-pooled arm's
/// OWN `core`/`opts` are empty (only its `bindings` reference the pooled derived
/// table), so [`apply_dup_safety`] already reads that as "no scans to duplicate"
/// and leaves it alone — but a BGP can mix a pooled position (wrapped into
/// `subplan_joins`, D2-elided because it was NOT provably disjoint) with an
/// ORDINARY, D2-elided-because-disjoint sibling position that still reads its own
/// unkeyed `core` table directly (e.g. two candidate maps whose reifier templates
/// carry different literal prefixes — provably disjoint, so D2 leaves them as
/// separate branches — while their SHARED description-pattern variable is NOT
/// disjoint and DOES get pooled): once `unfold::join_branches`/`merge` folds both
/// positions into one final branch, it carries BOTH an unkeyed `core` scan AND a
/// `subplan_joins` entry, and D1 must still fire on the `core` half — an
/// unconditional `!b.subplan_joins.is_empty()` skip here previously missed
/// exactly that shape (`cross_source_with_duplicate_bag_multiplicity_diverges_
/// from_oracle`: `?r ex:assertedBy ?src`'s per-source arms are disjoint on `?r`'s
/// own template and stay unpooled, but each still reads its own `core` scan of an
/// unkeyed source table).
pub(crate) fn force_distinct_for_dup_safety(
    branches: &mut [Branch],
    schema: &[TableSchema],
    dialect: sf_sql::Dialect,
) {
    let schema_map = build_schema_map(schema);
    for b in branches {
        if b.distinct || b.path.is_some() || b.agg.is_some() {
            continue;
        }
        apply_dup_safety(b, &schema_map, dialect);
    }
}

/// Apply D1 to one branch: find every `core`/`opts` scan whose table is NOT
/// covered by a declared PK/UNIQUE key over `b`'s OWN bindings
/// ([`scan_key_covered`] — deliberately NOT `distinct_removal`'s
/// `project`-narrowed proof: D1 asks "can THIS BGP block itself produce
/// duplicate rows", answered over its own full binding set regardless of what
/// the OUTER query later projects away), then make each uncovered scan
/// duplicate-safe by one of two mechanisms:
///
/// * **Per-scan wrap (the common case, Item 1).** Rewrite the scan's
///   `LogicalSource` in place to `SELECT DISTINCT <used cols> FROM <source>`
///   ([`wrap_scan_distinct`]) — `<used cols>` is EVERY column
///   [`alias_used_columns`] finds this branch reading from that alias, so the
///   wrap can never orphan a JOIN/NULL-guard condition that still names it.
///   `scan.alias` is unchanged (the ADR-0033 alias-preserving precedent), so
///   every pre-existing binding/condition resolves against the wrapped derived
///   table's identically-named output columns with zero rewriting elsewhere.
///   Because the dedup is now baked into the OFFENDING scan's own FROM
///   fragment rather than a branch-wide flag, it survives everything that used
///   to defeat the flag: `unfold::merge`'s OR-fold (nothing to fold — the wrap
///   travels with the scan through every later merge), the NPS guard (an NPS
///   sibling's own protected bag is untouched — only the unkeyed scan's own
///   source changed), and `cascade::run`'s later projection shrinking (the
///   wrap dedups the scan's OWN full column set, never whatever the outer
///   query later happens to project). This closes the two refutations a
///   branch-wide flag could not express: `s3b_join_projection_duplicates_
///   survive` (projection narrowing turned the flag into an illegal
///   final-result dedup) and `s5_nps_bag_multiplicity_joined_with_unkeyed_
///   dup_table` (the NPS guard dropped the flag rather than let it apply to
///   just the sibling table).
///
/// * **Branch-level flag (the pre-Item-1 mechanism, kept as a fallback).**
///   Used only when [`alias_used_columns`] is empty (nothing to select —
///   pathological) or some binding reading the alias is NOT
///   [`binding_is_injective`]: `SELECT DISTINCT <raw cols>` dedups RAW column
///   tuples, which equals SPARQL term dedup only when every projected term's
///   construction is injective (ADR-0025 C.3) — a non-injective template can
///   render two DIFFERENT raw tuples to the SAME term, so a per-scan raw
///   DISTINCT would UNDER-dedup exactly the case that needs it most. Falling
///   back to `b.distinct = true` keeps the existing, already-sound behavior:
///   `unfold::merge` still OR-folds it across the rest of the BGP (respecting
///   the NPS guard), and `emit::emit_branch_with`'s own C.3 check refuses
///   (sound 501) rather than emit a silently-wrong DISTINCT — exactly the
///   outcome `s2a_noninjective_binding_does_not_cover_key_sound_501` and the
///   W3C TC0005b flat-Ok/tree-C.3 asymmetry both pin today. A branch can wrap
///   SOME scans and still fall back for others in the same BGP — the
///   fallback's blanket "any non-injective binding ⇒ 501" is sound regardless
///   of which alias tripped it, so wrapping the eligible scans first never
///   costs correctness even when a sibling scan still needs the fallback.
fn apply_dup_safety(b: &mut Branch, schema: &SchemaMap, dialect: sf_sql::Dialect) {
    let uncovered: Vec<usize> = b
        .core
        .iter()
        .chain(b.opts.iter().map(|o| &o.scan))
        .filter(|scan| !scan_key_covered(scan, schema, &b.bindings))
        .map(|scan| scan.alias)
        .collect();
    let mut need_flag = false;
    for alias in uncovered {
        let cols = alias_used_columns(b, alias);
        if cols.is_empty() || !alias_bindings_injective(b, alias) {
            need_flag = true;
            continue;
        }
        wrap_scan_distinct(b, alias, &cols, dialect);
    }
    if need_flag {
        b.distinct = true;
    }
}

/// Run 4 Wave C0d — ADR-0034 D1's THIRD path, alongside the per-scan `DISTINCT`
/// wrap ([`wrap_scan_distinct`]) and the branch-level flag's sound-501 fallback
/// ([`apply_dup_safety`]'s doc comment): when `b.distinct` is set (by D1's own
/// `need_flag` fallback, OR by an explicit SPARQL `DISTINCT` — the mechanism
/// below is equally sound either way, since BOTH need "dedup by RECONSTRUCTED
/// TERM", not raw columns) over a NON-INJECTIVE binding, AND `b`'s relation
/// stands ALONE in its plan slot, term-level Rust-side dedup answers instead of
/// refusing.
///
/// **The sound-scope rule** (state it here, once, for every caller):
/// `b.core.len() + b.opts.len() + b.subplan_joins.len() <= 1` — AT MOST ONE
/// thing contributes rows to this branch (one base/view scan, XOR one SubPlan
/// derived table, XOR nothing at all) and nothing else joins against it. A
/// branch with a join partner (a second `core` scan, an `OptJoin`, an
/// INNER/LEFT-JOINed `SubPlanJoin` alongside another source) is excluded: the
/// join would consume/multiply this relation's raw-row multiplicity BEFORE a
/// dedup pass downstream ever sees the pre-join picture, so post-execution
/// dedup on the FINAL joined row would be dedup applied too late — those shapes
/// keep the ADR-0025 C.3 sound 501. The `<= 1` form (not `== 1` per field)
/// deliberately admits BOTH the plain "one unkeyed scan" shape ([`emit_
/// branch_with`](crate::emit::emit_branch_with)'s direct C.3 site, and
/// `iq::lower::lower_as_subplan`'s OWN inner `arm_projections` pre-check on a
/// single un-pooled arm) and the "this branch IS one SubPlan, wrapping nothing
/// else" pass-through shape a bridged tree arm's `Distinct` produces when it is
/// NOT the query spine (`iq::resolve::bridge_branch`'s doc comment) — the
/// SAME rule, checked at the SAME call, covers both without a second flag.
///
/// **Mechanism.** When eligible, [`crate::emit::emit_branch_with`]'s C.3 gate
/// answers instead of refusing: it emits the branch's SQL WITHOUT `DISTINCT`
/// (raw rows, duplicates and all — pushing `SELECT DISTINCT` on non-injective
/// raw columns is exactly the unsound operation C.3 exists to prevent), and
/// [`crate::exec_core`]'s `run_branches` — seeing this same eligibility on the
/// SAME branch — reconstructs every row as usual, THEN drops a row whose full
/// reconstructed solution tuple (every one of `b.bindings`' bound variables,
/// compared by `Term`'s own `Eq`: full lexical form + kind + datatype/lang) was
/// already emitted, via a streaming `HashSet` scoped to this one branch. This
/// deliberately relaxes the ADR-0006 constant-memory invariant for EXACTLY this
/// branch class (bounded by DISTINCT OUTPUT rows, not total input rows) —
/// answering correctly beats refusing; the invariant holds everywhere else
/// untouched. `iq::lower::lower_as_subplan` propagates eligibility outward: when
/// a SubPlan wraps exactly one eligible inner arm (the pass-through shape
/// above), it sets `distinct = true` on the OUTER wrapper branch too, so this
/// same check — re-run once more at the point the outer branch is finally
/// executed — fires there and the term-level dedup runs over the FULLY
/// reconstructed (positionally-remapped) row, not the never-independently-
/// executed inner one.
///
/// **Literal/BlankNode only, never IRI.** A non-injective offending binding must
/// be a `TermType::Literal` or `TermType::BlankNode` multi-column template — R2RML
/// gives EITHER no way to become provably injective (no percent-encoding exists to
/// protect a separator character between column slots, unlike IRI's RFC-3987
/// escaping), so a mapping author has no alternative design that avoids this class
/// entirely; a `{FirstName} {LastName}`-shaped literal or a no-PK default-mapping
/// blank-node template is a common, unavoidable R2RML pattern (the W3C target
/// shape this wave restores: Student/IOUs/Lives, every one Literal- or BlankNode-
/// typed). An IRI's non-injectivity is narrower and avoidable — ADJACENT column
/// slots with NO literal separator at all (`Template::is_injective`'s own doc
/// comment); adding so much as one separator character fixes it, since IRI
/// percent-encoding then guarantees that character can never appear verbatim
/// inside a column's own value. `adversarial_adr0034_refute.rs`'s `s2a_
/// noninjective_binding_does_not_cover_key_sound_501` deliberately probes exactly
/// that IRI edge case (`http://ex/{a}{b}`, over a table that DOES declare a
/// composite key `(a,b)` the binding merely can't use for elision) and pins the
/// sound-501 outcome — term-dedup answering it instead would regress that lock,
/// which is testing a DIFFERENT property (the elision proof's injectivity filter)
/// than this mechanism addresses.
pub(crate) fn eligible_for_term_dedup(b: &Branch) -> bool {
    b.distinct
        && b.path.is_none()
        && b.agg.is_none()
        && b.core.len() + b.opts.len() + b.subplan_joins.len() <= 1
        && b.bindings.values().any(|def| !binding_is_injective(def))
        && b.bindings.values().all(binding_is_term_dedup_safe)
}

/// Whether `def` is safe for term-level dedup: already injective (nothing to
/// dedup), or a Literal/BlankNode multi-column template — the ONLY non-injective
/// shape [`eligible_for_term_dedup`] / [`group_eligible_for_term_dedup`] cover
/// (see the former's "Literal/BlankNode only, never IRI" doc section for why). A
/// non-`Derived` non-injective shape never arises in practice (`Coalesce`/
/// `Concat`/`Agg`/`ComposedTriple` are not `binding_is_injective`'s `Template`
/// match arm to begin with) but is excluded defensively rather than assumed.
fn binding_is_term_dedup_safe(def: &TermDef) -> bool {
    if binding_is_injective(def) {
        return true;
    }
    let TermDef::Derived { term_map, .. } = def else {
        return false;
    };
    crate::iq::term_map_type(term_map) != Some(TermType::Iri)
}

/// Run 4 Wave C0d's GROUP extension — [`eligible_for_term_dedup`]'s own sound
/// scope rule, applied to a whole D2 pooling GROUP of `branches.len() >= 2` arms
/// (`iq::lower::lower_as_subplan`'s / `unfold::pool_group`'s own multi-arm
/// injectivity gate) instead of one standalone branch. A group arm need not
/// itself carry `distinct` — an arm with an already-injective binding has
/// nothing of its own to dedup; the group-wide term-dedup this licenses covers
/// it regardless once SOME sibling arm is an offender. Requires: EVERY arm is
/// standalone (the SAME rule, checked per arm — a joined arm's multiplicity
/// would be consumed before the group-wide dedup could ever see it, even when
/// that one arm's own binding happens to be injective); and every offending
/// (non-injective) `keep`-projected binding, across every arm, is
/// [`binding_is_term_dedup_safe`]. `keep` is the pattern's own projected
/// variable set (`iq::lower::lower_as_subplan`'s `vars`) — an arm's own
/// internal-only binding (never read by the outer query) is not a dedup key and
/// must not gate this.
pub(crate) fn group_eligible_for_term_dedup(
    branches: &[Branch],
    keep: &std::collections::HashSet<String>,
) -> bool {
    if branches.len() < 2 {
        return false;
    }
    let mut any_offender = false;
    for b in branches {
        if b.path.is_some()
            || b.agg.is_some()
            || b.core.len() + b.opts.len() + b.subplan_joins.len() > 1
        {
            return false;
        }
        for (k, def) in &b.bindings {
            if !keep.contains(k) || binding_is_injective(def) {
                continue;
            }
            if !binding_is_term_dedup_safe(def) {
                return false;
            }
            any_offender = true;
        }
    }
    any_offender
}

/// Run 5 C0e restoration — prepare a [`group_eligible_for_term_dedup`] group's
/// members for [`crate::exec_core::run_branches`]'s cross-branch SHARED seen-set
/// path instead of [`crate::unfold::pool_group`]'s SQL `UNION` pooling: narrow
/// each member's bindings down to the pattern's own projected vars (`keep`) — an
/// arm may bind extra internal-only vars that must not widen the dedup key,
/// mirroring `pool_group`'s identical narrowing — and force `distinct` so
/// [`eligible_for_term_dedup`] recognizes each member once it executes
/// standalone (every member needs `distinct = true` regardless of whether ITS
/// OWN binding is the offender — `group_eligible_for_term_dedup`'s own doc
/// comment: "the group-wide term-dedup this licenses covers it regardless once
/// SOME sibling arm is an offender", and the caller shares one seen-set across
/// every member by [`crate::iq::Scan::alias`], not by which member offends).
/// Pure mutation — callers still owe `group_eligible_for_term_dedup`'s own
/// eligibility check.
pub(crate) fn narrow_group_for_shared_term_dedup(
    members: &mut [Branch],
    keep: &std::collections::HashSet<String>,
) {
    for b in members {
        b.bindings.retain(|k, _| keep.contains(k.as_str()));
        b.distinct = true;
    }
}

/// Whether pooling `members` (a D2 group of ≥2 not-provably-disjoint arms,
/// [`disjoint_groups`](crate::unfold::disjoint_groups)) positionally on
/// PostgreSQL risks a `UNION` the engine cannot honor soundly: either a hard SQL
/// type-resolver error, or — if papered over with a `CAST` to align the types — a
/// silent lexical-drift wrong answer. Live-verified (PostgreSQL 17): a bare
/// `float8::text` cast switches to scientific notation outside a plain-decimal
/// magnitude range (`1e+20`, `1.7976931348623157e+308`) and drops the sign of
/// negative zero, where Rust's `f64::to_string()` — what reconstruction actually
/// reads for a NATIVELY-typed `float8` column, `sf_sql::backend::pg::pg_value`'s
/// `Type::FLOAT8` arm — never does; casting through `numeric` first does not fix
/// it either (loses trailing significant digits at extreme magnitudes,
/// live-verified). No PostgreSQL expression was found that exactly reproduces
/// Rust's shortest-round-trip plain-decimal formatting, so a floating-point slot
/// mismatch cannot be aligned soundly in SQL — sound refuse (ADR-0025's own
/// established "cannot pool soundly ⇒ 501" shape) rather than risk either
/// failure mode (W3C R2RMLTC0012e: `IOUs.amount FLOAT` pools against
/// `Lives.city VARCHAR` at the shared blank-node subject template's 3rd column
/// slot). Integer/text/boolean mismatches are NOT flagged — their to-text
/// conversions are exact and dialect-agnostic, so positional pooling already
/// renders them correctly with no cast needed.
///
/// Only checks pairs of arms that project the SAME variable with the SAME
/// column-count (`TermDef::columns().len()`) — a differing count is a width
/// mismatch, a completely different code path (Mechanism B / `pool_rendered`),
/// not this one. SQLite is dynamically typed (no such `UNION` error exists
/// there), so this is a no-op for every other dialect.
pub(crate) fn group_has_unsafe_float_slot_mismatch(
    members: &[&Branch],
    schema: &[TableSchema],
    dialect: sf_sql::Dialect,
) -> bool {
    if dialect != sf_sql::Dialect::Postgres || members.len() < 2 {
        return false;
    }
    let schema_map = build_schema_map(schema);
    let col_type = |b: &Branch, c: &ColRef| -> Option<&str> {
        let scan = b.core.iter().find(|s| s.alias == c.alias)?;
        let LogicalSource::Table(t) = &scan.source else {
            return None;
        };
        schema_map_get(&schema_map, t)?
            .column(&c.column)
            .map(|col| col.sql_type.as_str())
    };
    let is_float = |ty: &str| {
        let ty = ty.to_ascii_lowercase();
        ty == "real" || ty.contains("float") || ty.contains("double")
    };
    for (i, bi) in members.iter().enumerate() {
        for bj in &members[i + 1..] {
            for (var, def_i) in &bi.bindings {
                let Some(def_j) = bj.bindings.get(var) else {
                    continue;
                };
                let (cols_i, cols_j) = (def_i.columns(), def_j.columns());
                if cols_i.len() != cols_j.len() {
                    continue;
                }
                for (ci, cj) in cols_i.iter().zip(&cols_j) {
                    let (Some(ti), Some(tj)) = (col_type(bi, ci), col_type(bj, cj)) else {
                        continue;
                    };
                    if !ti.eq_ignore_ascii_case(tj) && (is_float(ti) || is_float(tj)) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Whether `scan`'s table is covered by a declared PK/UNIQUE key over
/// `bindings` ([`table_key_covered_by_bindings`]). An `rr:sqlQuery` view scan
/// (no `TableSchema` entry to prove anything from) is conservatively treated as
/// never covered — the same conservatism `distinct_removal`'s own scans apply.
fn scan_key_covered(
    scan: &Scan,
    schema: &SchemaMap,
    bindings: &std::collections::BTreeMap<String, TermDef>,
) -> bool {
    let LogicalSource::Table(table) = &scan.source else {
        return false;
    };
    let Some(ts) = schema_map_get(schema, table.as_str()) else {
        return false;
    };
    table_key_covered_by_bindings(ts, scan.alias, bindings)
}

/// Every raw column `alias` contributes to `b`'s CURRENT shape: each binding's
/// own columns ([`TermDef::columns`] — subject/predicate/object term-map
/// slots, every column of a multi-column template) PLUS every column a
/// `where_conds`/`opts`/`subplan_joins` condition mentions on `alias`
/// ([`collect_cond_cols`] — a referencing-object-map's own `rr:joinCondition`
/// equality, an R2RML §11 NULL guard, or a later-attached FILTER/OPTIONAL/
/// SubPlan correlation). Mirrors [`Branch::projection`]'s own "not distinct"
/// column set, scoped to one alias. Missing a column class here would break
/// the wrap: the wrapped derived table would stop exposing a column a
/// JOIN/NULL-guard still references outside it.
///
/// Deliberately does NOT special-case `NotExists`/`Exists`/`PathExists`
/// (`collect_cond_cols` skips their own nested `conds` by design, per its own
/// doc comment) — a MINUS/EXISTS correlation always names a variable that is
/// ALSO one of THIS branch's own bindings (correlation requires a pre-existing
/// shared variable), so the bindings loop below already covers whatever column
/// it would have added. True regardless of WHEN a caller runs this: D1 decides
/// at per-pattern time (`unfold::bgp` / `iq::resolve`'s `Intensional` arm),
/// before EXISTS/MINUS/FILTER are ever attached to `where_conds` — a column a
/// later-added correlation needs was necessarily already read by a binding
/// that exists right now, so the wrap this builds from today's bindings stays
/// valid however `where_conds` grows afterward.
fn alias_used_columns(b: &Branch, alias: usize) -> Vec<Box<str>> {
    let mut cols: Vec<Box<str>> = Vec::new();
    let push = |c: &ColRef, cols: &mut Vec<Box<str>>| {
        if c.alias == alias && !cols.contains(&c.column) {
            cols.push(c.column.clone());
        }
    };
    for def in b.bindings.values() {
        for c in def.columns() {
            push(&c, &mut cols);
        }
    }
    for cond in &b.where_conds {
        collect_cond_cols(cond, &mut |c| push(c, &mut cols));
    }
    for opt in &b.opts {
        for cond in opt.on.iter().chain(opt.extra.iter()) {
            collect_cond_cols(cond, &mut |c| push(c, &mut cols));
        }
    }
    for sp in &b.subplan_joins {
        for cond in &sp.on {
            collect_cond_cols(cond, &mut |c| push(c, &mut cols));
        }
    }
    cols
}

/// Whether every binding reading `alias`'s columns is [`binding_is_injective`]
/// — the per-scan wrap's soundness precondition (see [`apply_dup_safety`]'s
/// doc comment for why a non-injective binding disqualifies the wrap).
fn alias_bindings_injective(b: &Branch, alias: usize) -> bool {
    b.bindings.values().all(|def| match def {
        TermDef::Derived { alias: a, .. } if *a == alias => binding_is_injective(def),
        _ => true,
    })
}

/// Rewrite `alias`'s own scan (in `b.core` or `b.opts`) from its base source to
/// `SELECT DISTINCT <cols> FROM <source>` — the ADR-0034 Item 1 per-scan D1
/// wrap. `cols` must be [`alias_used_columns`]`(b, alias)`; `alias` is left
/// unchanged (ADR-0033's alias-preserving precedent) so no other reference to
/// `t{alias}.*` needs rewriting.
///
/// Every projected column is qualified against a fresh local alias
/// (`sfs{alias}.<col>`), never emitted bare (`<col>`) — qualification here is
/// load-bearing, not cosmetic. SQLite's historical "double-quoted string"
/// fallback silently reinterprets a BARE double-quoted identifier that fails
/// to resolve to a real column as a STRING LITERAL rather than raising "no
/// such column", so `SELECT DISTINCT "IDs" FROM "Student"` over a table with
/// no `IDs` column would silently succeed with the literal text `IDs` as the
/// row's value instead of erroring. A qualified `alias."col"` reference has
/// no such fallback (`table.string-literal` is not valid SQL in any dialect,
/// so it can only parse as a column reference) — found via the W3C
/// R2RMLTC0002c regression (a deliberately undefined `rr:column`, which the
/// engine must reject, silently produced a bogus triple instead).
fn wrap_scan_distinct(b: &mut Branch, alias: usize, cols: &[Box<str>], dialect: sf_sql::Dialect) {
    let scan = b
        .core
        .iter_mut()
        .chain(b.opts.iter_mut().map(|o| &mut o.scan))
        .find(|s| s.alias == alias)
        .expect("alias came from this branch's own core/opts scan");
    let src_alias = format!("sfs{alias}");
    // The immediate SQL text this wrap is about to nest — an `rr:sqlQuery` view
    // scans its OWN text for [`col_is_unquoted_alias`] below; a `Table` scan has
    // no such text (there is nothing to be an unquoted ALIAS of — a base-table
    // column reference is never itself an alias declaration).
    let inner_sql = match &scan.source {
        LogicalSource::Table(_) => None,
        LogicalSource::Query(q) => Some(q.clone()),
    };
    let select_list = cols
        .iter()
        .map(|c| wrap_col_ref(&src_alias, c, inner_sql.as_deref(), dialect))
        .collect::<Vec<_>>()
        .join(", ");
    let from = match &scan.source {
        LogicalSource::Table(t) => format!("{} {src_alias}", dialect.quote_ident(t)),
        // A view (`rr:sqlQuery`) source is already a derived table; nest it
        // under the same local alias — PostgreSQL/MySQL require every
        // FROM-position subquery to be named (only SQLite tolerates a bare
        // one).
        LogicalSource::Query(q) => format!("({q}) {src_alias}"),
    };
    scan.source = LogicalSource::Query(format!("SELECT DISTINCT {select_list} FROM {from}"));
}

/// Render one `<expr> AS <alias>` SELECT-list item for [`wrap_scan_distinct`].
/// Mirrors `emit::colref`'s two dialect special-cases — that function isn't
/// reachable here (no live `ColumnCatalog` exists yet at D1's translate-time
/// call site, which is exactly the seam that broke on PostgreSQL: this wrap
/// bakes `<src_alias>.<col>` directly into a NEW SQL string rather than
/// routing through the catalog-aware emission path every OTHER column
/// reference in the branch still gets) — so both fixes have to be re-applied
/// here specifically, PostgreSQL-only, C0e repair. **Always emits an explicit
/// `AS <alias>`, in the SAME quoted-or-unquoted form as the expression's own
/// reference**, even where SQL's own "no explicit alias inherits the
/// reference's name" rule would have produced the identical output column
/// name anyway: a wrap can nest inside ANOTHER wrap (`iq::lower::
/// pool_rendered`'s own D2 rendered-projection fallback, when a D1-wrapped
/// arm also needs width-mismatch pooling — W3C R2RMLTC0011a) or be re-probed
/// as a `LogicalSource::Query` by a live `ColumnCatalog` — either consumer
/// re-derives this SAME quoting decision from THIS layer's own SQL text via
/// [`col_is_unquoted_alias`], which only ever looks for an EXPLICIT `AS`
/// clause; without one here, a nested consumer has no signal to detect that
/// D1 already folded this column and would default back to the mapping's
/// original, unfolded text.
///
/// * **`rowid` → `ctid`** (identical to `emit::colref`'s own special case):
///   Direct Mapping's synthetic no-PK blank-node identifier reads the
///   physical row id (`sf-mapping`'s `rowid` column). SQLite exposes that as
///   the `rowid` pseudo-column; PostgreSQL has none — render the equivalent
///   system tuple id `(sfsN.ctid)::text` (existential blank-node seed; only
///   per-row uniqueness matters, ADR-0005). Confirmed live: every
///   DirectMapping no-PK W3C case (DirectGraphTC0000/1/2/3/4/5/12/14/17/18/
///   22/25) failed with "column sfsN.rowid does not exist" before this.
/// * **A column that is itself an UNQUOTED alias in the immediate inner SQL
///   stays unquoted.** An `rr:sqlQuery` view scan's OWN output column names
///   come from whatever the mapping author wrote in the view's `SELECT … AS
///   <alias>` — PostgreSQL case-folds an UNQUOTED alias DECLARATION to
///   lowercase at view-definition time (`AS StudentId` → output column
///   `studentid`); quoting the REFERENCE (`"StudentId"`, the literal
///   mapping-authored text, `dialect.quote_ident`'s unconditional behavior)
///   pins it to the UNFOLDED text, which the folded column can no longer
///   match — confirmed live: every one of W3C R2RMLTC0002d/0003b/0009d/
///   0011a/0014b/0014c/0014d is an unquoted view alias hitting exactly this.
///   Deliberately narrower than "any regular-identifier-shaped name over a
///   view source": an EARLIER version of this fix unquoted every such name
///   unconditionally and REGRESSED four different, previously-passing cases
///   (R2RMLTC0002d's OWN sibling columns `"ID"`/`"Name"` in the SAME view,
///   each a bare, un-aliased, quoted PASS-THROUGH of a delimited base-table
///   column — R2RML §5's `"ID"`/`"Name"` stay exact-case, so unquoting
///   THOSE broke them). [`col_is_unquoted_alias`] checks for the SPECIFIC
///   `AS <col>` (unquoted) text, not just `col`'s own shape, so a
///   bare/quoted-alias reference correctly stays on the quoted path.
/// * **A `Table` source's own columns, or a view-sourced column that is not
///   itself an unquoted alias, keep the unconditional quoted rendering**:
///   the W3C fixtures' base-table DDL is delimited (quoted, exact-case
///   preserved, e.g. `CREATE TABLE "Student" ("ID" INTEGER, …)`), so quoting
///   is the correct, matching reference for a `Table` scan; a bare or
///   quoted-alias view column likewise preserves exact case in the view's
///   own output, so quoting the reference is correct there too.
fn wrap_col_ref(
    src_alias: &str,
    col: &str,
    inner_sql: Option<&str>,
    dialect: sf_sql::Dialect,
) -> String {
    if dialect == sf_sql::Dialect::Postgres && col == "rowid" {
        // Aliased AS `ctid`, NOT `rowid`: `emit::colref`'s own rowid special
        // case always emits a bare `t{alias}.ctid` for ANY `ColRef.column ==
        // "rowid"`, regardless of what this wrap's own output is named — so
        // the wrapped derived table must expose a column literally called
        // `ctid` for that outer reference to resolve, or this wrap would
        // silently orphan the very reference it exists to serve.
        return format!("({src_alias}.ctid)::text AS ctid");
    }
    if inner_sql.is_some_and(|sql| col_is_unquoted_alias(sql, col)) {
        return format!("{src_alias}.{col} AS {col}");
    }
    let quoted = dialect.quote_ident(col);
    format!("{src_alias}.{quoted} AS {quoted}")
}

/// Whether `col` appears as an UNQUOTED SQL alias (`AS col` / `as col`,
/// case-insensitive `AS` keyword, exact-case `col`, word-bounded on both
/// sides — no partial match inside a longer identifier, e.g. `col = "ID"`
/// must not match inside `AS Sport_ID`) anywhere in `sql`. [`wrap_col_ref`]'s
/// signal that `col`'s output column was already case-folded at the point
/// this `AS` clause was written, so a reference to it should fold the same
/// way rather than pin to the exact-case source text. A bare, un-aliased
/// reference or a QUOTED alias (`AS "col"`) does not match — both preserve
/// `col`'s exact case in the source's own output, so quoting the reference
/// stays correct for those. No `regex` dependency: a plain byte scan for the
/// literal keyword is sufficient here (`sql` is a bounded mapping-authored
/// string, never a hot loop).
///
/// `pub(crate)`: also reused by `iq::lower::pool_rendered` (Run 4 Wave C0d
/// Mechanism B, W3C R2RMLTC0011a) against an arm's OWN `scan.source` text —
/// which, when D1 already wrapped that scan, IS `wrap_col_ref`'s own output
/// (`<expr> AS <alias>`, in the SAME quoted-or-unquoted form this function
/// detects), so the identical check composes correctly whether the
/// immediate source is the ORIGINAL mapping-authored `rr:sqlQuery` or an
/// already-D1-wrapped derived table one layer in.
pub(crate) fn col_is_unquoted_alias(sql: &str, col: &str) -> bool {
    fn is_ident_byte(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }
    let bytes = sql.as_bytes();
    let lower = sql.to_ascii_lowercase();
    let mut from = 0;
    while let Some(rel) = lower[from..].find("as") {
        let as_start = from + rel;
        let as_end = as_start + 2;
        from = as_end;
        let word_before_ok = as_start == 0 || !is_ident_byte(bytes[as_start - 1]);
        let word_after_ok = as_end >= bytes.len() || !is_ident_byte(bytes[as_end]);
        if !word_before_ok || !word_after_ok {
            continue;
        }
        let mut i = as_end;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b'"' {
            continue; // a QUOTED alias — exact case preserved, not this rule's target
        }
        if sql[i..].starts_with(col) {
            let after = i + col.len();
            if after >= bytes.len() || !is_ident_byte(bytes[after]) {
                return true;
            }
        }
    }
    false
}

/// Whether some declared key of `ts` (the PK, or a `UNIQUE` constraint — ADR-0034
/// D1 says "PK/UNIQUE"; `distinct_removal` above only ever needs the PK, a
/// narrower question since a mapping's subject template is conventionally
/// PK-templated) is covered by the bindings on `alias`: every column of the key
/// must be NOT NULL (a nullable key permits several NULL rows a UNIQUE
/// constraint does not distinguish — mirrors `key_is_non_null`'s use in passes
/// 2/2a/6) and appear as a `Column`/template `Segment::Column` slot read by SOME
/// individually-injective binding on this alias (a superset is fine, whether
/// within one binding's own template or spread across several — the same
/// permissiveness `distinct_removal`'s own composite-PK proof already allows
/// for the single-binding case).
///
/// The union is taken across ALL of `alias`'s injective bindings, not just one:
/// a composite key can be split across separate output variables — e.g.
/// `<#Mid>`'s subject reads `person_id` (binds `?id`), its object reads `mid`
/// (binds `?m`), and `om_mid`'s own PK is the PAIR `(person_id, mid)`; neither
/// binding alone covers it, but together they do. This is sound by the same
/// argument as the single-binding case, composed: if two rows agree on every
/// bound variable's output, then for EACH injective binding they must agree on
/// that binding's own read columns (injectivity's contrapositive), hence on the
/// union of every injective binding's columns — and if that union covers a
/// declared key, the table's own PK/UNIQUE constraint forces the two rows to be
/// the same physical row (`table_key_covered_by_bindings`'s test coverage:
/// `om_mid`-shaped composite-key-split-across-variables fixture, ADR-0034/C0
/// follow-up).
fn table_key_covered_by_bindings(
    ts: &TableSchema,
    alias: usize,
    bindings: &std::collections::BTreeMap<String, TermDef>,
) -> bool {
    let keys: Vec<&[String]> = std::iter::once(ts.primary_key.as_slice())
        .chain(ts.unique.iter().map(Vec::as_slice))
        .filter(|k| !k.is_empty() && k.iter().all(|c| key_is_non_null(ts, c)))
        .collect();
    if keys.is_empty() {
        return false;
    }
    let covered: Vec<&str> = bindings
        .values()
        .filter(|def| binding_is_injective(def))
        .filter_map(|def| match def {
            TermDef::Derived { term_map, alias: a } if *a == alias => Some(term_map),
            _ => None,
        })
        .flat_map(|term_map| -> Vec<&str> {
            match term_map {
                TermMap::Column(c, _) => vec![c.as_ref()],
                TermMap::Template(t, _) => t
                    .segments()
                    .iter()
                    .filter_map(|s| match s {
                        Segment::Column(c) => Some(c.as_ref()),
                        Segment::Literal(_) => None,
                    })
                    .collect(),
                TermMap::Constant(_) => Vec::new(),
            }
        })
        .collect();
    keys.iter()
        .any(|key| key.iter().all(|k| covered.contains(&k.as_str())))
}

/// D1 (ADR-0034) for a GROUP BY + aggregates branch — "dedup lands below GROUP
/// BY" (the ADR's own Interactions commitment): unlike an ordinary branch (which
/// just gets `Branch::distinct = true` rendered as a flat `SELECT DISTINCT`), a
/// `SELECT DISTINCT <agg-exprs> ... GROUP BY` would dedupe the GROUPED RESULT —
/// the wrong level, since COUNT/SUM/etc must see already-deduped pre-aggregation
/// rows. Called SEPARATELY from `run` (by `lib.rs`, once after each translation
/// path's own `run` call) because it needs `dialect` to emit the wrapped inner
/// SELECT, which `run`/[`CascadeCtx`] do not carry — extending their signature
/// was not worth the blast radius (`CascadeCtx` has ~60 existing construction
/// sites, mostly in unit tests with no `Dialect` to hand, and `Dialect` has no
/// `Default` impl to fall back on).
pub(crate) fn dedup_before_aggregate(branches: &mut [Branch], dialect: sf_sql::Dialect) {
    for b in branches {
        wrap_aggregate_input_if_needed(b, dialect);
    }
}

/// Wrap `b`'s `core`/`opts`/`where_conds` into a `SELECT DISTINCT` [`crate::iq::
/// SubPlanJoin`] BEFORE its `Aggregation` groups over them, when `b.distinct` is
/// already `true`. Trusts the flag rather than re-deriving it from `b`'s CURRENT
/// bindings/schema: `b.distinct` was decided per pattern, BEFORE `unfold`'s
/// `group`/`iq::lower`'s aggregation lowering replaced this branch's bindings
/// with just the grouping keys + aggregate result names (`Aggregation`'s own doc
/// comment: "the inner pattern's other variables are not projected by the
/// group") — by the time THIS function runs, the very variable (e.g. a PK-
/// templated `?s`, grouped away in favor of `?g`) that proved the branch
/// duplicate-free is long gone from `b.bindings`, so re-checking here (as an
/// earlier version of this function did, via `branch_needs_distinct_for_dup_
/// safety`) would wrongly conclude "not covered" and wrap even a PK-clean inner
/// pattern (`differential_tree.rs`'s `single_branch_group_by_self_join_
/// collapses_to_one_scan` caught this — the SAME "outer restriction strips the
/// key-covering variable before D1 can see it" class of bug the per-pattern
/// `unfold::bgp` / `iq::resolve` timing fix already closed for the ordinary
/// case, recurring here for GROUP BY's OWN narrowing). Clears `b.distinct` after
/// wrapping — the dedup now happens INSIDE the derived table, so the outer
/// `GROUP BY` must not ALSO render a (wrong-level) `DISTINCT`. Rewrites
/// `agg.keys[].cols` / `agg.aggs[].arg` to the derived table's positional
/// columns — reuses the SAME `SubPlanJoin` + `emit_sp` machinery `emit_agg_
/// branch`'s existing "SQL agg-over-UNION pushdown" FROM-clause rendering
/// already handles (`crate::emit`), so no `emit.rs` change is needed here. A
/// no-op when `b` carries no `Aggregation`, is not `distinct`, or has nothing to
/// dedup on (`COUNT(*)` with no GROUP BY key and no aggregate argument — no
/// columns to distinguish rows by, so no wrapping is possible or needed; the
/// flag is left set in that last case since nothing removed the duplication it
/// flagged).
fn wrap_aggregate_input_if_needed(b: &mut Branch, dialect: sf_sql::Dialect) {
    if b.agg.is_none() || !b.distinct {
        return;
    }
    let agg = b.agg.as_ref().expect("checked Some above");
    let mut cols: Vec<ColRef> = Vec::new();
    for key in &agg.keys {
        for c in &key.cols {
            if !cols.contains(c) {
                cols.push(c.clone());
            }
        }
    }
    for a in &agg.aggs {
        if let Some(arg) = &a.arg {
            if !cols.contains(arg) {
                cols.push(arg.clone());
            }
        }
    }
    if cols.is_empty() {
        return;
    }
    // The dedup moves INSIDE the wrapped SubPlan below — the outer branch's own
    // GROUP BY must not also carry a (wrong-level) DISTINCT now.
    b.distinct = false;
    // A fresh alias unique within THIS branch — each branch emits its own
    // independent SQL statement, so alias uniqueness need not span branches.
    let sp_alias = 1 + b
        .core
        .iter()
        .map(|s| s.alias)
        .chain(b.opts.iter().map(|o| o.scan.alias))
        .chain(b.subplan_joins.iter().map(|sp| sp.alias))
        .max()
        .unwrap_or(0);
    let mut inner = Branch::empty();
    inner.core = std::mem::take(&mut b.core);
    inner.opts = std::mem::take(&mut b.opts);
    inner.where_conds = std::mem::take(&mut b.where_conds);
    inner.distinct = true;
    for (i, c) in cols.iter().enumerate() {
        inner.bindings.insert(
            format!("k{i:04}"),
            TermDef::Derived {
                term_map: TermMap::Column(c.column.clone(), sf_core::ir::TermSpec::plain_literal()),
                alias: c.alias,
            },
        );
    }
    let nested_plan = crate::Plan {
        branches: vec![inner],
        form: crate::PlanForm::Select {
            vars: (0..cols.len()).map(|i| format!("k{i:04}")).collect(),
        },
        // A single arm — nothing to POOL, just its own SELECT DISTINCT — but this
        // Plan-level flag must still be `true`: `Plan::emitted`/`prepared_branches`
        // ALWAYS overwrites a sole branch's `distinct` with the PLAN's own flag
        // (`branches.len() == 1 ⇒ b.distinct = self.distinct`), so setting only
        // `inner.distinct` above and leaving this `false` silently clobbered it
        // back to `false` at emission — a real, caught bug (the inner SubPlan's
        // own SQL never got its DISTINCT at all: `differential_tree.rs`'s
        // `adr0034_d1_count_aggregate_dedups_below_group_by`).
        distinct: true,
        limit: None,
        offset: 0,
        order: Vec::new(),
        rust_group: None,
        dialect,
        // This nested SubPlan executes wholly in SQL — the Run 5 C0e shared-
        // seen-set mechanism never applies to it (see `Plan::dedup_groups`'s
        // doc comment).
        dedup_groups: std::collections::HashMap::new(),
        construct_drops_some_branch_var: false,
    };
    let rewrite = |c: &mut ColRef| {
        if let Some(pos) = cols.iter().position(|x| x == c) {
            *c = ColRef::new(sp_alias, format!("c{pos}"));
        }
    };
    if let Some(agg) = &mut b.agg {
        for key in &mut agg.keys {
            for c in &mut key.cols {
                rewrite(c);
            }
        }
        for a in &mut agg.aggs {
            if let Some(arg) = &mut a.arg {
                rewrite(arg);
            }
        }
    }
    // `agg.keys`/`agg.aggs` are the GROUP BY/aggregate-expression SQL's own raw
    // column refs (rewritten above) — but `b.bindings` separately carries the
    // RECONSTRUCTION `TermDef` for each grouping-key variable (e.g. `?x`), built
    // from the SAME raw columns, that `exec::reconstruct` reads to rebuild the
    // term. Missing this left `?x` pointing at an alias `core`/`opts` no longer
    // have (now moved into the wrapped SubPlan) — `?x` silently vanished from
    // every result row (a real, caught bug: `differential_tree.rs`'s own
    // `adr0034_d1_count_aggregate_dedups_below_group_by`). An aggregate RESULT
    // binding (`TermDef::Agg`) is left untouched — it reads `b.agg`'s own output
    // column, unrelated to the wrapped inner SELECT.
    for def in b.bindings.values_mut() {
        if matches!(def, TermDef::Derived { .. }) {
            if let Ok(remapped) = crate::iq::lower::remap_termdef(def, &cols, sp_alias) {
                *def = remapped;
            }
        }
    }
    b.subplan_joins.push(crate::iq::SubPlanJoin {
        alias: sp_alias,
        plan: Box::new(nested_plan),
        on: Vec::new(),
        left: false,
    });
}

// --- 2a-ext. nullable-unique inner self-join elimination ------------------

/// Collapse two core scans of the same table joined on a **nullable unique key**.
/// An SQL equi-join excludes NULL rows (`NULL = NULL ⇒ UNKNOWN`), so the join
/// already produces a 1:1 match. After merge, an explicit `IS NOT NULL(col)` filter
/// replicates the NULL-exclusion the equi-join enforced implicitly. Loops to fixpoint
/// to handle chains of same-table scans. `=_bag`-safe by the same argument.
fn nullable_unique_self_join_elimination(b: &mut Branch, schema: &SchemaMap) {
    while let Some((keep, drop, cond_idx, not_null_col)) = find_nullable_unique_self_join(b, schema)
    {
        b.where_conds.remove(cond_idx);
        rewrite_alias(b, drop, keep);
        b.core.retain(|s| s.alias != drop);
        if !b
            .where_conds
            .iter()
            .any(|c| matches!(c, SqlCond::IsNotNull(r) if r == &not_null_col))
        {
            b.where_conds.push(SqlCond::IsNotNull(not_null_col));
        }
    }
}

/// Returns `(keep, drop, cond_idx, not_null_col)` for a nullable-unique self-join:
/// two core scans of the same table with a `ColEq` on a UNIQUE but nullable column.
fn find_nullable_unique_self_join(
    b: &Branch,
    schema: &SchemaMap,
) -> Option<(usize, usize, usize, ColRef)> {
    for (idx, cond) in b.where_conds.iter().enumerate() {
        let SqlCond::ColEq(a, c) = cond else { continue };
        if a.alias == c.alias || a.column != c.column {
            continue;
        }
        let Some(ta) = scan_table(b, a.alias) else {
            continue;
        };
        let Some(tc) = scan_table(b, c.alias) else {
            continue;
        };
        if ta != tc {
            continue;
        }
        if let Some(t) = schema_map_get(schema, ta.as_str()) {
            // Unique but NOT non-null: pass (2) already handles the non-null case.
            if t.is_unique_key(&a.column) && !key_is_non_null(t, &a.column) {
                let (keep, drop) = if a.alias < c.alias {
                    (a.alias, c.alias)
                } else {
                    (c.alias, a.alias)
                };
                return Some((keep, drop, idx, ColRef::new(keep, a.column.to_string())));
            }
        }
    }
    None
}

// --- 5b. disjunction-intersection simplification -------------------------

/// Simplify conjunctions of same-column equality disjunctions by computing their
/// set intersection. If the intersection is ∅ the branch is unsatisfiable (returns
/// `false`). Otherwise replaces the two conjuncts with their intersection and loops
/// to fixpoint. `=_bag`-safe: `(a ∈ S) ∧ (a ∈ T) ≡ (a ∈ S∩T)`.
fn disjunction_intersection_simplify(b: &mut Branch) -> bool {
    loop {
        let len = b.where_conds.len();
        let mut changed = false;
        'outer: for i in 0..len {
            let Some((col_i, vals_i)) = extract_eq_disjunction(&b.where_conds[i]) else {
                continue;
            };
            for j in (i + 1)..len {
                let Some((col_j, vals_j)) = extract_eq_disjunction(&b.where_conds[j]) else {
                    continue;
                };
                if col_i != col_j {
                    continue;
                }
                let intersection: Vec<String> = vals_i
                    .iter()
                    .filter(|v| vals_j.contains(*v))
                    .cloned()
                    .collect();
                if intersection.is_empty() {
                    return false; // unsatisfiable branch
                }
                if intersection.len() < vals_i.len().max(vals_j.len()) {
                    // Replace the two conjuncts with the intersection (j first — higher index).
                    let new_cond = if intersection.len() == 1 {
                        SqlCond::Cmp(col_i.clone(), CmpOp::Eq, intersection[0].clone())
                    } else {
                        SqlCond::Or(
                            intersection
                                .iter()
                                .map(|v| SqlCond::Cmp(col_i.clone(), CmpOp::Eq, v.clone()))
                                .collect(),
                        )
                    };
                    b.where_conds.remove(j);
                    b.where_conds[i] = new_cond;
                    changed = true;
                    break 'outer;
                }
            }
        }
        if !changed {
            break;
        }
    }
    true
}

/// Extract a single-column equality disjunction: `Or([Cmp(col, Eq, v1), ...])` where
/// all arms share the same column reference. Returns `None` for non-Or conditions or
/// mixed-column disjunctions.
fn extract_eq_disjunction(cond: &SqlCond) -> Option<(ColRef, Vec<String>)> {
    let SqlCond::Or(arms) = cond else {
        return None;
    };
    let mut col: Option<ColRef> = None;
    let mut vals = Vec::new();
    for arm in arms {
        let SqlCond::Cmp(c, CmpOp::Eq, v) = arm else {
            return None;
        };
        match &col {
            None => col = Some(c.clone()),
            Some(existing) if existing == c => {}
            _ => return None,
        }
        vals.push(v.clone());
    }
    col.map(|c| (c, vals))
}

/// Columns referenced by a branch's conditions (test/diagnostic helper).
pub fn condition_columns(b: &Branch) -> Vec<ColRef> {
    let mut cols = Vec::new();
    for cond in &b.where_conds {
        collect_cond_cols(cond, &mut |c| {
            if !cols.contains(c) {
                cols.push(c.clone());
            }
        });
    }
    cols
}
