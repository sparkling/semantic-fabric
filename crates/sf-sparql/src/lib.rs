//! `sf-sparql` — the virtualizer: SPARQL 1.2 → SQL rewriting over R2RML
//! (ADR-0003, ADR-0007). It turns a SPARQL query over the virtual graph into SQL
//! against a live relational source — instance data is **never materialised**
//! (ADR-0001/0002); rows stream back and are reconstructed into `oxrdf` terms.
//!
//! ## Pipeline (ADR-0003 / ADR-0007)
//!
//! ```text
//! parse (spargebra)                                            — caller / parse_query
//!   → [opt] algebra optimise (sparopt — opt-in, bypassable)    — ADR-0007 step 2 (deferred-on by default)
//!   → T-saturation (subclass/subproperty/inverse UNION-fold)   — saturate (ADR-0008 tier-1)
//!   → unfold each triple pattern against the mapping IR        — unfold  (ISWC-2018 base translation)
//!   → tier-0 elimination + the 6-pass optimizer cascade        — cascade (ADR-0007 order is load-bearing)
//!   → emit dialect SQL (sqlparser AST; bound params only)      — emit    (ADR-0010 §A/R1)
//!   → execute over the source + reconstruct bindings + stream  — exec    (ADR-0006 bounded memory)
//! ```
//!
//! Term construction is **lifted** to the outermost projection: joins and FILTERs
//! are over raw key columns; RDF terms are built only at reconstruction, through
//! the single `sf-core` term-gen path (ADR-0007 *Term-construction lifting*).
//!
//! ## v1 scope (ADR-0007 §v1 SPARQL coverage)
//!
//! Supported: the `?s ?p ?o` CONSTRUCT **dump** (the M2 / W3C-conformance target,
//! ADR-0005), BGP, JOIN, FILTER (comparison/`&&`/`||`/`!`/`BOUND` subset),
//! OPTIONAL (NULL-safe LEFT JOIN, single-scan right side), UNION, MINUS (§8.3
//! correlated anti-join over non-OPTIONAL shared variables, with the
//! disjoint-domain no-op), projection,
//! DISTINCT/REDUCED, LIMIT/OFFSET, ORDER BY (by a bound variable, ASC/DESC, several
//! keys — value-space order with UNBOUND first/last per direction; a single branch
//! pushes `ORDER BY … NULLS FIRST/LAST` into SQL, a bag-union sorts globally in
//! `exec`), `rr:class` / refObjectMap unfolding, and the
//! property paths with variable endpoints (`WITH RECURSIVE` / non-recursive CTE,
//! ADR-0007; `owl:TransitiveProperty` served live, ADR-0008; depth-bounded,
//! ADR-0010): `P+`/`P*` over a single predicate, the `?` (ZeroOrOne), `!` (NPS),
//! and `^`/`/`/`|` (inverse / sequence / alternative) operators, and `P+`/`P*`
//! over such composites (see [`path`] for the per-shape soundness gates — raw-key
//! equality requires matching node shapes; `P*`/`p?` reflexive requires a
//! single-predicate graph). Deferred → `501` (documented, never silent): a path
//! with a bound endpoint, a nested closure inside a composite, a predicate from a
//! multi-mapping / refObjectMap / multi-column or non-constant term map, a
//! shape-mismatched composite, `P*`/`p?` over a multi-predicate graph; for a single
//! branch pushed to SQL an ORDER BY on a non-`rr:column` term (a template IRI /
//! COALESCE); OPTIONAL with a multi-scan right side (JOIN inside OPTIONAL); plus
//! LATERAL, SERVICE, OWL 2 QL tier-2 (ADR-0008), and PostgreSQL execution (SQLite
//! is this wave's execution target; the path CTE's Postgres `CYCLE` variant is
//! the later MB-4 wave; emission is otherwise dialect-generic).
//!
//! ## Wave-E / M4 additions (2026-06-29)
//!
//! - **DESCRIBE** → Concise Bounded Description CONSTRUCT (CBD, SPARQL §10.4).
//! - **ORDER BY with arbitrary expressions** — evaluated at exec time via
//!   `exec::eval_expr` (STRLEN, arithmetic, IF, BOUND, comparisons, COALESCE, …).
//! - **OPTIONAL with UNION/multi-branch right** — sound ISWC-2018 decomposition:
//!   inner-join branches per right-branch + NOT EXISTS anti-join for unmatched lefts.
//! - **GROUP BY over UNION/multi-branch inner** — Rust-level grouping + aggregation.

use std::collections::BTreeSet;

use sf_core::ir::TriplesMap;
use sf_sql::{Dialect, TableSchema};
use spargebra::algebra::GraphPattern;
use spargebra::term::Variable;
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};
use spargebra::Query;

pub mod build;
pub mod cache;
pub mod cascade;
pub mod dump;
pub mod emit;
pub mod exec;
pub mod exec_core;
pub mod exec_mysql;
pub mod exec_pg;
pub mod iq;
pub mod leftjoin;
pub mod path;
pub mod saturate;
pub mod star;
pub mod unfold;
pub mod unify;

pub use cache::{Epoch, PlanCache, PlanKey};
pub use iq::Branch;
pub use saturate::Tbox;

/// Errors raised by the virtualizer (deferred features surface as
/// [`Error::Unsupported`] → `501`, never a silent wrong answer; ADR-0007).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// SPARQL parse failure (`spargebra`).
    #[error("sparql parse error: {0}")]
    Parse(String),
    /// A construct outside the v1 coverage surface (→ HTTP 501).
    #[error("unsupported (501): {0}")]
    Unsupported(String),
    /// A malformed / inconsistent mapping IR reference.
    #[error("mapping error: {0}")]
    Mapping(String),
    /// SQL emission/execution failure (`sqlparser` / source driver).
    #[error("sql error: {0}")]
    Sql(String),
    /// Term generation / datatype error from `sf-core`.
    #[error("core error: {0}")]
    Core(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// The result form of a compiled plan.
#[derive(Debug, Clone)]
pub enum PlanForm {
    /// `SELECT` — the projected variables, in result order.
    Select { vars: Vec<String> },
    /// `CONSTRUCT` (and the `?s ?p ?o` dump) — the construction template.
    Construct { template: Vec<TriplePattern> },
    /// `ASK`.
    Ask,
}

/// A compiled query plan: a bag-union of [`Branch`]es (each one SQL `SELECT`) plus
/// the form and the solution modifiers. Memory-bounded by `⟨T, M⟩` — independent
/// of source data (ADR-0006).
#[derive(Debug, Clone)]
pub struct Plan {
    pub branches: Vec<Branch>,
    pub form: PlanForm,
    pub distinct: bool,
    pub limit: Option<usize>,
    pub offset: usize,
    /// ORDER BY keys (SPARQL §15.1), empty when unordered. Applied to SQL for a
    /// single branch ([`Plan::prepared_branches`]) or as a global stable sort over
    /// the bag-union in [`exec`] for multiple branches.
    pub order: Vec<iq::OrderKey>,
    /// Rust-level GROUP BY descriptor for a multi-branch inner (SPARQL §11).
    /// When set, the executor buffers all branch solutions, groups by `keys`,
    /// and computes `aggs` in Rust before streaming the grouped results.
    pub rust_group: Option<iq::RustGroup>,
    pub dialect: Dialect,
}

impl Plan {
    /// Branches ready for emission: when there is exactly one unordered branch,
    /// DISTINCT / LIMIT / OFFSET are pushed into its SQL; with multiple (bag-union)
    /// branches they are applied during streaming ([`exec`]). **ORDER BY is never
    /// pushed into SQL** — it is applied in `exec` via the type-aware, collation-
    /// independent [`exec::order_cmp`] (a SQL `ORDER BY` would inherit the column's
    /// collation/affinity, which can disagree with SPARQL value order). So an ordered
    /// plan must also keep LIMIT/OFFSET out of SQL — they apply *after* the sort.
    pub fn prepared_branches(&self) -> Vec<Branch> {
        let mut branches = self.branches.clone();
        if branches.len() == 1 {
            let b = &mut branches[0];
            b.distinct = self.distinct;
            if self.order.is_empty() {
                b.limit = self.limit;
                b.offset = self.offset;
            }
        }
        branches
    }

    /// Emit every prepared branch to parameterised dialect SQL (ADR-0007 step 6).
    pub fn emitted(&self) -> Result<Vec<emit::EmittedBranch>> {
        self.prepared_branches()
            .iter()
            .map(|b| emit::emit_branch(b, self.dialect))
            .collect()
    }
}

/// Translate a parsed SPARQL query against `maps` into a [`Plan`] (no T-Box, no
/// schema — the plain R2RML conformance path). Routes through the operator-tree
/// (IQ) pipeline — the default since ADR-0023 M8. See [`translate_with`].
pub fn translate(query: &Query, maps: &[TriplesMap], dialect: Dialect) -> Result<Plan> {
    translate_with(query, maps, dialect, &Tbox::default(), &[])
}

/// Translate with a pre-classified T-Box (tier-1 saturation, ADR-0008) and source
/// schema (the constraint-driven cascade passes, ADR-0007). Routes through the
/// operator-tree (IQ) pipeline — the default since ADR-0023 M8. `sparopt` algebra
/// optimisation (pipeline step 2) is opt-in and bypassed by default (ADR-0007).
pub fn translate_with(
    query: &Query,
    maps: &[TriplesMap],
    dialect: Dialect,
    tbox: &Tbox,
    schema: &[TableSchema],
) -> Result<Plan> {
    translate_tree(query, maps, tbox, dialect, schema)
}

/// Translate through the **flat unfold path** (the proven `=_bag` oracle,
/// ADR-0023 M8 fallback). This was the production path before M8; it is now kept
/// as the oracle / regression anchor. Prefer [`translate`] / [`translate_with`]
/// (tree path) for production; call this when the flat SQL shape matters (e.g.
/// `differential_tree` oracle arm). See [`translate_with_flat`].
pub fn translate_flat(query: &Query, maps: &[TriplesMap], dialect: Dialect) -> Result<Plan> {
    translate_with_flat(query, maps, dialect, &Tbox::default(), &[])
}

/// Translate through the **flat unfold path** with a T-Box and schema. This was
/// the production path before ADR-0023 M8; it is now the `=_bag` oracle /
/// fallback. Prefer [`translate_with`] (tree path) for production; call this when
/// the flat SQL shape matters or as a regression guard.
pub fn translate_with_flat(
    query: &Query,
    maps: &[TriplesMap],
    dialect: Dialect,
    tbox: &Tbox,
    schema: &[TableSchema],
) -> Result<Plan> {
    translate_inner_flat(query, maps, dialect, tbox, schema, true)
}

/// Translate **without** the optimizer cascade — the raw ISWC-2018 base
/// translation (unfold + tier-0 done inline, but none of the order-sensitive
/// cascade rewrites). This is the NoREC differential's unoptimized arm (ADR-0012):
/// running this against the same source as [`translate_with`] must yield an
/// identical bag; any divergence pinpoints the offending cascade rule, no external
/// oracle required. Like the optimized path, an empty/contradictory branch still
/// yields no rows (the SQL `=`-constants are simply not pruned here), so `=_bag`
/// is preserved by construction.
pub fn translate_unoptimized(
    query: &Query,
    maps: &[TriplesMap],
    dialect: Dialect,
    tbox: &Tbox,
    schema: &[TableSchema],
) -> Result<Plan> {
    translate_inner_flat(query, maps, dialect, tbox, schema, false)
}

/// Shared flat-path translation core. `optimize` gates the ADR-0007 cascade:
/// `true` is the flat oracle path ([`translate_with_flat`]); `false` is the NoREC
/// unoptimized arm ([`translate_unoptimized`]).
fn translate_inner_flat(
    query: &Query,
    maps: &[TriplesMap],
    dialect: Dialect,
    tbox: &Tbox,
    schema: &[TableSchema],
    optimize: bool,
) -> Result<Plan> {
    // ADR-0031/ADR-0032: desugar quoted-triple patterns onto the ADR-0029 basic
    // encoding BEFORE anything else sees the WHERE pattern — everything below
    // this line (and everywhere else in this module) never encounters a
    // `TermPattern::Triple`. `star_env` records every variable the rewrite
    // determined to be triple-term-valued (ADR-0032 D3 item 2); consulted below
    // to pre-substitute the CONSTRUCT template and, once `branches` is
    // otherwise finalized, to install the native projection (D2).
    let (query, star_env) = star::rewrite_query(query)?;
    let query = &query;
    // M6 offline T-mapping: fold Tbox hierarchy into the maps once at startup so
    // the per-query unfold can use an empty Tbox (no runtime hash-map lookups).
    let empty_tbox = Tbox::default();
    let (saturated_maps, uf_tbox) = if tbox.is_empty() {
        (std::borrow::Cow::Borrowed(maps), tbox)
    } else {
        let expanded = saturate::saturate_maps(maps, tbox);
        (expanded, &empty_tbox)
    };
    let mut uf = unfold::Unfolder::new(&saturated_maps, uf_tbox, dialect, schema);
    let (trans, form) = match query {
        Query::Select { pattern, .. } => {
            let t = uf.translate_pattern(pattern)?;
            let vars = t
                .project
                .clone()
                .unwrap_or_else(|| visible_vars(&t.branches));
            (t, PlanForm::Select { vars })
        }
        Query::Construct {
            template, pattern, ..
        } => {
            // ADR-0032 D2: the old ADR-0031 rule-9 501 guard is superseded —
            // real instantiation now happens (`exec_core::instantiate`'s
            // recursive `TermPattern::Triple` arm) — but the template must
            // first be pre-substituted so an env-composed variable becomes an
            // explicit `TermPattern::Triple` over its component vars
            // (`star::substitute_construct_template`'s doc comment).
            let template = star::substitute_construct_template(template, &star_env);
            let t = uf.translate_pattern(pattern)?;
            (t, PlanForm::Construct { template })
        }
        Query::Ask { pattern, .. } => {
            let t = uf.translate_pattern(pattern)?;
            (t, PlanForm::Ask)
        }
        Query::Describe { pattern, .. } => {
            // Concise Bounded Description (CBD): for each described resource r,
            // CONSTRUCT { r ?__sf_p ?__sf_o } WHERE { <original WHERE> . r ?__sf_p ?__sf_o }.
            // The `pattern` field encodes the DESCRIBE targets as a Project over the
            // WHERE clause (the parser wraps literal resources in BIND expressions).
            let (describe_vars, inner_pat) = match pattern {
                GraphPattern::Project { variables, inner } => {
                    (variables.clone(), inner.as_ref().clone())
                }
                other => (Vec::new(), other.clone()),
            };
            if describe_vars.is_empty() {
                return Err(Error::Unsupported(
                    "DESCRIBE * (wildcard) is not supported → 501".to_owned(),
                ));
            }
            // Fresh synthetic variables for the CBD predicate and object
            // (double-underscore prefix avoids collision with user variables).
            let var_p = Variable::new_unchecked("__sf_describe_p");
            let var_o = Variable::new_unchecked("__sf_describe_o");
            // Build the CBD WHERE: join the original WHERE with `?v ?__sf_p ?__sf_o`
            // for each described variable. Multiple DESCRIBE targets each add their own
            // CBD triple, returning triples for all described resources in one plan.
            let mut cbd_pattern = inner_pat;
            for v in &describe_vars {
                cbd_pattern = GraphPattern::Join {
                    left: Box::new(cbd_pattern),
                    right: Box::new(GraphPattern::Bgp {
                        patterns: vec![TriplePattern {
                            subject: TermPattern::Variable(v.clone()),
                            predicate: NamedNodePattern::Variable(var_p.clone()),
                            object: TermPattern::Variable(var_o.clone()),
                        }],
                    }),
                };
            }
            let template = describe_vars
                .iter()
                .map(|v| TriplePattern {
                    subject: TermPattern::Variable(v.clone()),
                    predicate: NamedNodePattern::Variable(var_p.clone()),
                    object: TermPattern::Variable(var_o.clone()),
                })
                .collect();
            let t = uf.translate_pattern(&cbd_pattern)?;
            (t, PlanForm::Construct { template })
        }
    };
    // Pass (6) needs the projected-variable set + the requested DISTINCT to prove
    // a DISTINCT redundant; SELECT carries an explicit projection, CONSTRUCT/ASK
    // project every binding (`None`). ADR-0032 D3 item 2: expanded with any
    // env-composed variable's component names — see
    // `star::expand_projection_for_cascade`'s doc comment for why pass 7
    // (projection shrinking) needs this BEFORE it can see the `ComposedTriple`
    // binding `apply_composed_bindings` installs only at the very end.
    let project_vars: Option<Vec<String>> = match &form {
        PlanForm::Select { vars } => Some(star::expand_projection_for_cascade(vars, &star_env)),
        _ => None,
    };
    let mut branches = if optimize {
        let ctx = cascade::CascadeCtx {
            distinct: trans.distinct,
            project: project_vars.as_deref(),
        };
        cascade::run(trans.branches, schema, &ctx)
    } else {
        // `cascade::run` is skipped here by design — this is the NoREC unoptimized
        // baseline (ADR-0007: "none of the order-sensitive cascade rewrites"). D1
        // (ADR-0034) needs no extra call here: `unfold::bgp` already applies it per
        // pattern, unconditionally (both this path and the optimized one share the
        // SAME `Unfolder::translate_pattern`/`bgp` unfold), so `trans.branches`
        // already carries its correct `distinct` decisions.
        trans.branches
    };
    // ADR-0034: dedup below GROUP BY — a correctness fix applied regardless of
    // `optimize`, same as the D1 handling above; this one trusts each branch's
    // own `distinct` flag rather than needing `schema` again (see `cascade::
    // dedup_before_aggregate`'s doc comment).
    cascade::dedup_before_aggregate(&mut branches, dialect);
    // The single-branch DISTINCT decision is recorded on the branch by pass (6)
    // — computed HERE, AFTER `dedup_before_aggregate`, which can itself just have
    // CHANGED that same branch's `distinct` (clearing it once the dedup moved
    // inside a wrapped SubPlan). Reading it any earlier would capture a now-stale
    // value that `Plan::prepared_branches` (`branches.len() == 1 ⇒ b.distinct =
    // self.distinct`) would then blindly write back onto the branch at emission,
    // silently UNDOING the wrap's own fix (a real, caught bug: a GROUP BY's outer
    // SQL kept a spurious `SELECT DISTINCT` over the grouped result — the wrong
    // level — while the correctly-deduped inner SubPlan's own `DISTINCT` was
    // simultaneously overwritten away too, since both read the same stale flag).
    let distinct = if branches.len() == 1 {
        branches[0].distinct
    } else {
        trans.distinct
    };
    // ADR-0032 D3 item 2 — the projection seam, applied LAST (every real
    // join/filter unification is already done; `unify::unify` never sees a
    // `ComposedTriple` on this path in practice).
    star::apply_composed_bindings(&mut branches, &star_env);
    Ok(Plan {
        branches,
        form,
        distinct,
        limit: trans.limit,
        offset: trans.offset,
        order: trans.order,
        rust_group: trans.rust_group,
        dialect,
    })
}

/// Translate a parsed SPARQL query through the **operator-tree (IQ) path**
/// (ADR-0023 M8). This is the production default since M8: [`translate`] and
/// [`translate_with`] both route here. It drives the four-stage tree pipeline —
/// [`build::build_tree`] → [`iq::resolve::resolve`]
/// → [`iq::normalize::normalize`] → [`iq::lower::lower`] — for the same per-`Query`-form
/// wrapping the flat core uses (SELECT projection / CONSTRUCT template / ASK / DESCRIBE
/// CBD), producing a [`Plan`] the SAME `exec` runs. After lowering, the **proven flat
/// cascade** ([`cascade::run`]) is reused on the lowered branches (ADR-0023 M4 wave 1),
/// giving the tree the within-leaf-CQ rewrites (self-join / FK elimination, filter
/// pushdown, distinct removal, …) for free. The cascade is `=_bag`-preserving, so the
/// optimized tree result stays multiset-equal to the flat oracle (M3 design §7 — proved
/// by the `differential_tree` shadow test). The flat path is kept as the `=_bag` oracle
/// / fallback via [`translate_flat`] / [`translate_with_flat`].
///
/// The single [`iq::resolve::ResolveCx`] threads ONE alias counter across the whole
/// query, so sibling patterns receive disjoint scan aliases (M3 design §3.2).
pub fn translate_tree(
    query: &Query,
    maps: &[TriplesMap],
    tbox: &Tbox,
    dialect: Dialect,
    schema: &[TableSchema],
) -> Result<Plan> {
    // ADR-0031/ADR-0032: the SAME shared pre-pass `translate_inner_flat` runs, so
    // both engines see an identical, already-desugared WHERE pattern (never a
    // `TermPattern::Triple`) — `build.rs`/`iq/resolve.rs`/`iq/normalize.rs` need
    // no star-specific code. `iq/lower.rs` is the one exception (`extra_keep`,
    // below): its OWN internal projection-restrict retains run even before
    // `cascade`'s pass 7, so it needs to know which columns a LATER stage will
    // still need — see `lower`'s own doc comment for why. `star_env` — see the
    // identical note in `translate_inner_flat`.
    let (query, star_env) = star::rewrite_query(query)?;
    let query = &query;
    let mut cx = iq::resolve::ResolveCx::new(maps, tbox, dialect, schema);
    let extra_keep = star::all_component_var_names(&star_env);
    // Compile one WHERE pattern through the four-stage tree pipeline. The shared `cx`
    // (one alias counter) is threaded by `&mut`, so a query with several patterns
    // (e.g. DESCRIBE's CBD join) keeps disjoint aliases across them.
    let mut compile = |pattern: &GraphPattern| -> Result<Plan> {
        let built = build::build_tree(pattern, None)?;
        let resolved = iq::resolve::resolve(built, &mut cx)?;
        let normalized = iq::normalize::normalize(resolved)?;
        iq::lower::lower(normalized, dialect, &extra_keep, &star_env)
    };

    let mut plan = match query {
        // SELECT — `lower` already produced the projected-variable `PlanForm::Select`
        // from the tree's outermost `Construction.project` (the SELECT scope).
        Query::Select { pattern, .. } => compile(pattern)?,
        // CONSTRUCT — compile the WHERE pattern, then carry the construction template
        // (exactly the flat core's `PlanForm::Construct` wrapping).
        Query::Construct {
            template, pattern, ..
        } => {
            // ADR-0032 D2 — see the identical note in `translate_inner_flat`.
            let template = star::substitute_construct_template(template, &star_env);
            let mut plan = compile(pattern)?;
            plan.form = PlanForm::Construct { template };
            plan
        }
        // ASK — compile the WHERE pattern, carry the `Ask` form.
        Query::Ask { pattern, .. } => {
            let mut plan = compile(pattern)?;
            plan.form = PlanForm::Ask;
            plan
        }
        // DESCRIBE — Concise Bounded Description, replicated from the flat core: wrap
        // the WHERE in a CBD `?v ?__sf_p ?__sf_o` join per described resource and emit
        // a CONSTRUCT over the synthetic predicate/object (M3 design §7 "same template/
        // cbd/current_graph wrapping the flat core uses").
        Query::Describe { pattern, .. } => {
            let (describe_vars, inner_pat) = match pattern {
                GraphPattern::Project { variables, inner } => {
                    (variables.clone(), inner.as_ref().clone())
                }
                other => (Vec::new(), other.clone()),
            };
            if describe_vars.is_empty() {
                return Err(Error::Unsupported(
                    "DESCRIBE * (wildcard) is not supported → 501".to_owned(),
                ));
            }
            let var_p = Variable::new_unchecked("__sf_describe_p");
            let var_o = Variable::new_unchecked("__sf_describe_o");
            let mut cbd_pattern = inner_pat;
            for v in &describe_vars {
                cbd_pattern = GraphPattern::Join {
                    left: Box::new(cbd_pattern),
                    right: Box::new(GraphPattern::Bgp {
                        patterns: vec![TriplePattern {
                            subject: TermPattern::Variable(v.clone()),
                            predicate: NamedNodePattern::Variable(var_p.clone()),
                            object: TermPattern::Variable(var_o.clone()),
                        }],
                    }),
                };
            }
            let template = describe_vars
                .iter()
                .map(|v| TriplePattern {
                    subject: TermPattern::Variable(v.clone()),
                    predicate: NamedNodePattern::Variable(var_p.clone()),
                    object: TermPattern::Variable(var_o.clone()),
                })
                .collect();
            let mut plan = compile(&cbd_pattern)?;
            plan.form = PlanForm::Construct { template };
            plan
        }
    };

    // Reuse the PROVEN flat cascade (ADR-0007) on the tree's lowered branches — the same
    // call `translate_inner_flat` makes (ADR-0023 M4 wave 1). `project` is the SELECT scope
    // (CONSTRUCT/ASK project every binding ⇒ `None`); `distinct` is the requested DISTINCT.
    // A multi-branch aggregation (`rust_group`) still runs the cascade — its pre-group union
    // arms carry the aggregate-argument columns that `rust_group_execute` reads BY NAME, so
    // `project` is forced to `None` rather than the SELECT vars: the only pass that consults
    // `ctx.project` to *drop* bindings (pass 7, projection shrinking) becomes a no-op with
    // `None` and so can never strip an aggregate-arg column; every other pass (incl. self-join
    // elimination) is projection-agnostic and safe to run unconditionally. Cross-union
    // aggregate-specific optimization (agg-through-union rewrites) is a LATER M4 wave.
    // ADR-0032 D3 item 2 — see the identical note in `translate_inner_flat`.
    let project_vars: Option<Vec<String>> = if plan.rust_group.is_some() {
        None
    } else {
        match &plan.form {
            PlanForm::Select { vars } => Some(star::expand_projection_for_cascade(vars, &star_env)),
            _ => None,
        }
    };
    let ctx = cascade::CascadeCtx {
        distinct: plan.distinct,
        project: project_vars.as_deref(),
    };
    plan.branches = cascade::run(plan.branches, schema, &ctx);
    // A SubPlan derived table (§5.1: the M5 nested-modifier joins; ADR-0023
    // optimizer-residue's SQL agg-over-UNION pushdown) hides its own arms one level
    // down in `SubPlanJoin::plan.branches` — the `cascade::run` above never reaches
    // them (it only walks `plan.branches`), so self-join elimination and the rest of
    // the cascade would silently never fire on a pooled/nested arm otherwise. Recurse
    // into every SubPlan the SAME way: `project: None` (mirrors the `rust_group`
    // guard above — a nested arm's raw columns feed its outer union/aggregation BY
    // NAME, so they must never be shrunk away).
    for b in &mut plan.branches {
        cascade_subplans(b, schema);
    }
    // ADR-0034: dedup below GROUP BY (see the identical note in
    // `translate_inner_flat`). Ordinary D1 needs no extra call here either: like
    // the flat engine's `unfold::bgp`, `iq::resolve`'s `Intensional` arm already
    // applies it per pattern, before this tree's own aggregation lowering
    // (`iq::lower`) ever narrows a branch's bindings down to its grouping keys.
    cascade::dedup_before_aggregate(&mut plan.branches, dialect);
    // The single-branch DISTINCT decision is recorded on the branch by pass (6)
    // — read HERE, AFTER `dedup_before_aggregate`, not before it: see the
    // identical note in `translate_inner_flat` for why reading it any earlier
    // captures a stale value `Plan::prepared_branches` would blindly write back
    // onto the branch at emission, undoing the wrap's own fix.
    if plan.branches.len() == 1 {
        plan.distinct = plan.branches[0].distinct;
    }
    // ADR-0032 D3 item 2 — the projection seam, applied LAST (see the
    // identical note in `translate_inner_flat`).
    star::apply_composed_bindings(&mut plan.branches, &star_env);
    Ok(plan)
}

/// Recursively run the cascade on every [`iq::SubPlanJoin`]'s nested [`Plan`]
/// branches reachable from `b` (depth-first — a SubPlan can itself carry a
/// SubPlan). See the call site above for why this is needed.
///
/// **Multi-branch guard** (adversarial review, q9 agg-pushdown wave): a nested
/// Plan with MORE THAN ONE branch renders as `UNION ALL` ([`emit::emit_subplan_sql`])
/// — every arm's SELECT-list column COUNT must match, and the SQL agg-pushdown's
/// [`iq::lower::try_sql_group_over_union`] fixes the exact position each `c{i}`
/// column occupies BEFORE this cascade runs. A pass like self-join elimination
/// rewrites/removes `where_conds` per arm (which feeds `Branch::projection()`'s
/// trailing columns) independently per arm, so it can shrink one arm's column
/// count without touching a sibling arm the same way — silently invalidating that
/// contract (a loud `UNION ALL` column-count SQL error, or worse, a shifted
/// position). So for a multi-branch nested Plan, only ACCEPT the cascaded arms
/// when every arm still projects the SAME column count as every sibling
/// (post-cascade) — otherwise keep the pre-cascade arms for this SubPlan (still
/// correct; this SubPlan alone forgoes the optimization). A single-branch nested
/// Plan has no such cross-arm contract, so it always keeps the cascaded result.
fn cascade_subplans(b: &mut Branch, schema: &[TableSchema]) {
    for sp in &mut b.subplan_joins {
        let ctx = cascade::CascadeCtx {
            distinct: false,
            project: None,
        };
        let pre = sp.plan.branches.clone();
        let post = cascade::run(pre.clone(), schema, &ctx);
        let post_lens: Vec<usize> = post.iter().map(|br| br.projection().len()).collect();
        let safe = pre.len() == 1
            || (post.len() == pre.len() && post_lens.windows(2).all(|w| w[0] == w[1]));
        sp.plan.branches = if safe { post } else { pre };
        for inner in &mut sp.plan.branches {
            cascade_subplans(inner, schema);
        }
    }
}

/// Translate through the compiled-plan cache (ADR-0007 *Plan cache, hot path*):
/// a repeated query at the same `⟨T, M⟩` + schema `epoch` reuses its plan instead
/// of recompiling. Keying is collision-safe — the canonical algebra disambiguates
/// a hash collision, so `:a`'s plan can never serve `:b` (see [`cache`]). Bump
/// `epoch` on ontology/mapping/schema reload to invalidate.
pub fn translate_cached(
    query: &Query,
    maps: &[TriplesMap],
    dialect: Dialect,
    tbox: &Tbox,
    schema: &[TableSchema],
    cache: &PlanCache<Plan>,
    epoch: Epoch,
) -> Result<Plan> {
    let key = cache::plan_key(query, epoch);
    if let Some(plan) = cache.get(&key) {
        return Ok(plan);
    }
    let plan = translate_with(query, maps, dialect, tbox, schema)?;
    cache.put(key, plan.clone());
    Ok(plan)
}

/// Parse `sparql` and translate it with caching (ADR-0007 *Plan cache*). The
/// serve-endpoint entry point: parse first so a syntactically invalid query
/// returns a 400 without touching the cache, then hit the cache before the
/// full rewrite. Callers that already have a parsed `Query` call
/// [`translate_cached`] directly.
pub fn parse_and_translate_cached(
    sparql: &str,
    maps: &[TriplesMap],
    dialect: Dialect,
    tbox: &Tbox,
    schema: &[TableSchema],
    cache: &PlanCache<Plan>,
    epoch: Epoch,
) -> Result<Plan> {
    let query = spargebra::SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| Error::Parse(e.to_string()))?;
    translate_cached(&query, maps, dialect, tbox, schema, cache, epoch)
}

/// Parse `sparql` and translate it (convenience over [`translate`]).
pub fn parse_and_translate(sparql: &str, maps: &[TriplesMap], dialect: Dialect) -> Result<Plan> {
    let query = spargebra::SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| Error::Parse(e.to_string()))?;
    translate(&query, maps, dialect)
}

/// Parse `sparql` and translate it through the **operator-tree path** (ADR-0023)
/// with a T-Box and source `schema` (convenience over [`translate_tree`]). Drop-in
/// alternative to [`parse_and_translate_with`] for the M7 benchmark comparison.
pub fn parse_and_translate_tree_with(
    sparql: &str,
    maps: &[TriplesMap],
    dialect: Dialect,
    tbox: &Tbox,
    schema: &[TableSchema],
) -> Result<Plan> {
    let query = spargebra::SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| Error::Parse(e.to_string()))?;
    translate_tree(&query, maps, tbox, dialect, schema)
}

/// Parse `sparql` and translate it against a T-Box and source `schema`
/// (convenience over [`translate_with`]). This is the live entry point: passing a
/// real introspected `schema` is what makes the constraint-driven cascade passes
/// (self-join, FD, FK/PK join elimination, redundant-DISTINCT) actually *fire* —
/// with `&[]` they are sound no-ops (ADR-0007).
pub fn parse_and_translate_with(
    sparql: &str,
    maps: &[TriplesMap],
    dialect: Dialect,
    tbox: &Tbox,
    schema: &[TableSchema],
) -> Result<Plan> {
    let query = spargebra::SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| Error::Parse(e.to_string()))?;
    translate_with(&query, maps, dialect, tbox, schema)
}

/// Parse `sparql` and translate it through the **flat unfold path** (the ADR-0023
/// oracle / permanent fallback), with a T-Box and source `schema`. Symmetric
/// counterpart to [`parse_and_translate_tree_with`]; intended for benchmark
/// comparison (bench group `obda_select_flat_1x`) and test oracle arms.
pub fn parse_and_translate_flat_with(
    sparql: &str,
    maps: &[TriplesMap],
    dialect: Dialect,
    tbox: &Tbox,
    schema: &[TableSchema],
) -> Result<Plan> {
    let query = spargebra::SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| Error::Parse(e.to_string()))?;
    translate_with_flat(&query, maps, dialect, tbox, schema)
}

/// All variables bound anywhere in the branches (the `SELECT *` projection), in a
/// deterministic order.
fn visible_vars(branches: &[Branch]) -> Vec<String> {
    let mut set = BTreeSet::new();
    for b in branches {
        for v in b.bindings.keys() {
            set.insert(v.clone());
        }
    }
    set.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_iri_produces_construct_plan() {
        // DESCRIBE <r> translates to a CBD CONSTRUCT — should succeed, not 501.
        let q = spargebra::SparqlParser::new()
            .parse_query("DESCRIBE <http://ex/x>")
            .unwrap();
        let result = translate(&q, &[], Dialect::Sqlite);
        assert!(
            result.is_ok(),
            "DESCRIBE <iri> should translate to a CONSTRUCT plan, got: {:?}",
            result
        );
        assert!(
            matches!(result.unwrap().form, PlanForm::Construct { .. }),
            "DESCRIBE should produce a Construct form"
        );
    }

    #[test]
    fn describe_wildcard_produces_construct_plan() {
        // DESCRIBE * WHERE { P } expands `*` to all in-scope variables, each of which
        // becomes a CBD target — should succeed and produce a CONSTRUCT plan.
        let q = spargebra::SparqlParser::new()
            .parse_query("DESCRIBE * WHERE { ?s ?p ?o }")
            .unwrap();
        let result = translate(&q, &[], Dialect::Sqlite);
        assert!(
            result.is_ok(),
            "DESCRIBE * should translate successfully, got: {:?}",
            result
        );
        assert!(
            matches!(result.unwrap().form, PlanForm::Construct { .. }),
            "DESCRIBE * should produce a Construct form"
        );
    }

    #[test]
    fn translate_cached_reuses_and_invalidates() {
        // The cache is consulted on the compile path (not dead infrastructure): a
        // second call at the same epoch is a hit; an epoch bump recompiles.
        let q = spargebra::SparqlParser::new()
            .parse_query("SELECT * WHERE { ?s ?p ?o }")
            .unwrap();
        let cache: PlanCache<Plan> = PlanCache::new(8);
        let e = Epoch(1);
        assert!(cache.is_empty());
        let _ =
            translate_cached(&q, &[], Dialect::Sqlite, &Tbox::default(), &[], &cache, e).unwrap();
        assert_eq!(cache.len(), 1, "first compile populates the cache");
        let _ =
            translate_cached(&q, &[], Dialect::Sqlite, &Tbox::default(), &[], &cache, e).unwrap();
        assert_eq!(cache.len(), 1, "second call is a hit, no new entry");
        let mut e2 = e;
        e2.bump();
        let _ =
            translate_cached(&q, &[], Dialect::Sqlite, &Tbox::default(), &[], &cache, e2).unwrap();
        assert_eq!(cache.len(), 2, "a new epoch recompiles under a fresh key");
    }

    #[test]
    fn unmapped_property_path_operators_are_unsupported() {
        // With NO mappings every path predicate is unmapped → 501 regardless of the
        // operator (`?`/`!`/composite `+` are all now compiled — see
        // `crate::path` — but resolve to "predicate not mapped" here). The
        // supported-shape differentials live in
        // sf-conformance/tests/differential_paths.rs; the single-predicate P+/P*
        // engine path is exercised end-to-end in tests/e2e.rs.
        for q in [
            "SELECT * WHERE { ?s <http://ex/p>? ?o }",
            "SELECT * WHERE { ?s !<http://ex/p> ?o }",
            "SELECT * WHERE { ?s (<http://ex/p>/<http://ex/q>)+ ?o }",
            "SELECT * WHERE { ?s (^<http://ex/p>)+ ?o }",
        ] {
            let q = spargebra::SparqlParser::new().parse_query(q).unwrap();
            assert!(matches!(
                translate(&q, &[], Dialect::Sqlite),
                Err(Error::Unsupported(_))
            ));
        }
    }
}
