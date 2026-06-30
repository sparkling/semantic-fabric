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
pub mod exec_mysql;
pub mod exec_pg;
pub mod iq;
pub mod leftjoin;
pub mod path;
pub mod pool;
pub mod saturate;
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
/// schema — the plain R2RML conformance path). See [`translate_with`].
pub fn translate(query: &Query, maps: &[TriplesMap], dialect: Dialect) -> Result<Plan> {
    translate_with(query, maps, dialect, &Tbox::default(), &[])
}

/// Translate with a pre-classified T-Box (tier-1 saturation, ADR-0008) and source
/// schema (the constraint-driven cascade passes, ADR-0007). `sparopt` algebra
/// optimisation (pipeline step 2) is opt-in and bypassed by default (ADR-0007).
pub fn translate_with(
    query: &Query,
    maps: &[TriplesMap],
    dialect: Dialect,
    tbox: &Tbox,
    schema: &[TableSchema],
) -> Result<Plan> {
    translate_inner(query, maps, dialect, tbox, schema, true)
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
    translate_inner(query, maps, dialect, tbox, schema, false)
}

/// Shared translation core. `optimize` gates the ADR-0007 cascade: `true` is the
/// production path ([`translate_with`]); `false` is the NoREC unoptimized arm
/// ([`translate_unoptimized`]).
fn translate_inner(
    query: &Query,
    maps: &[TriplesMap],
    dialect: Dialect,
    tbox: &Tbox,
    schema: &[TableSchema],
    optimize: bool,
) -> Result<Plan> {
    let mut uf = unfold::Unfolder::new(maps, tbox, dialect);
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
            let t = uf.translate_pattern(pattern)?;
            (
                t,
                PlanForm::Construct {
                    template: template.clone(),
                },
            )
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
    // project every binding (`None`).
    let project_vars: Option<Vec<String>> = match &form {
        PlanForm::Select { vars } => Some(vars.clone()),
        _ => None,
    };
    let (branches, distinct) = if optimize {
        let ctx = cascade::CascadeCtx {
            distinct: trans.distinct,
            project: project_vars.as_deref(),
        };
        let out = cascade::run(trans.branches, schema, &ctx);
        // The single-branch DISTINCT decision is recorded on the branch by pass (6).
        let distinct = if out.len() == 1 {
            out[0].distinct
        } else {
            trans.distinct
        };
        (out, distinct)
    } else {
        (trans.branches, trans.distinct)
    };
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

/// Translate a parsed SPARQL query through the **operator-tree (IQ) path** (ADR-0023
/// M3d shadow), the alternative to the flat [`unfold`]-based [`translate_with`]. It
/// drives the four-stage tree pipeline — [`build::build_tree`] → [`iq::resolve::resolve`]
/// → [`iq::normalize::normalize`] → [`iq::lower::lower`] — for the same per-`Query`-form
/// wrapping the flat core uses (SELECT projection / CONSTRUCT template / ASK / DESCRIBE
/// CBD), producing a [`Plan`] the SAME `exec` runs. The cascade optimizer is **not**
/// applied (the tree pipeline reaches the lowerable spine directly), so this is the
/// tree analogue of the unoptimized base translation; it must be `=_bag` to the flat
/// path (M3 design §7 — proved by the `differential_tree` shadow test).
///
/// **This is the shadow path, NOT the default.** [`translate`]/[`translate_with`] stay
/// the production engine and the proven oracle (M3 design §7: never switch before a
/// full green differential window).
///
/// The single [`iq::resolve::ResolveCx`] threads ONE alias counter across the whole
/// query, so sibling patterns receive disjoint scan aliases (M3 design §3.2).
pub fn translate_tree(
    query: &Query,
    maps: &[TriplesMap],
    tbox: &Tbox,
    dialect: Dialect,
) -> Result<Plan> {
    let mut cx = iq::resolve::ResolveCx::new(maps, tbox, dialect);
    // Compile one WHERE pattern through the four-stage tree pipeline. The shared `cx`
    // (one alias counter) is threaded by `&mut`, so a query with several patterns
    // (e.g. DESCRIBE's CBD join) keeps disjoint aliases across them.
    let mut compile = |pattern: &GraphPattern| -> Result<Plan> {
        let built = build::build_tree(pattern, None)?;
        let resolved = iq::resolve::resolve(built, &mut cx)?;
        let normalized = iq::normalize::normalize(resolved)?;
        iq::lower::lower(normalized, dialect)
    };

    match query {
        // SELECT — `lower` already produced the projected-variable `PlanForm::Select`
        // from the tree's outermost `Construction.project` (the SELECT scope).
        Query::Select { pattern, .. } => compile(pattern),
        // CONSTRUCT — compile the WHERE pattern, then carry the construction template
        // (exactly the flat core's `PlanForm::Construct` wrapping).
        Query::Construct {
            template, pattern, ..
        } => {
            let mut plan = compile(pattern)?;
            plan.form = PlanForm::Construct {
                template: template.clone(),
            };
            Ok(plan)
        }
        // ASK — compile the WHERE pattern, carry the `Ask` form.
        Query::Ask { pattern, .. } => {
            let mut plan = compile(pattern)?;
            plan.form = PlanForm::Ask;
            Ok(plan)
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
            Ok(plan)
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
