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
//! OPTIONAL (NULL-safe LEFT JOIN, single-scan right side), UNION, projection,
//! DISTINCT/REDUCED, LIMIT/OFFSET, `rr:class` / refObjectMap unfolding, and the
//! recursive property paths `P+`/`P*` over a single predicate with variable
//! endpoints (`WITH RECURSIVE` CTE, ADR-0007; `owl:TransitiveProperty` served
//! live, ADR-0008; depth-bounded, ADR-0010). Deferred → `501` (documented, never
//! silent): aggregates, the `?` (ZeroOrOne) / `!` (NPS) path operators and
//! sequence/alternative/inverse path combinations, a path with a bound endpoint
//! or a multi-mapping / refObjectMap / multi-column predicate, LATERAL, MINUS,
//! GRAPH, BIND/VALUES, ORDER BY, DESCRIBE, SERVICE, OWL 2 QL tier-2 (ADR-0008),
//! and PostgreSQL execution (SQLite is this wave's execution target; the path
//! CTE's Postgres `CYCLE` variant is the later MB-4 wave; emission is otherwise
//! dialect-generic).

use std::collections::BTreeSet;

use sf_core::ir::TriplesMap;
use sf_sql::{Dialect, TableSchema};
use spargebra::term::TriplePattern;
use spargebra::Query;

pub mod cache;
pub mod cascade;
pub mod dump;
pub mod emit;
pub mod exec;
pub mod exec_pg;
pub mod iq;
pub mod leftjoin;
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
    pub dialect: Dialect,
}

impl Plan {
    /// Branches ready for emission: when there is exactly one branch, DISTINCT /
    /// LIMIT / OFFSET are pushed into its SQL; with multiple (bag-union) branches
    /// they are applied during streaming ([`exec`]), and DISTINCT-over-union is
    /// deferred (ADR-0007).
    pub fn prepared_branches(&self) -> Vec<Branch> {
        let mut branches = self.branches.clone();
        if branches.len() == 1 {
            let b = &mut branches[0];
            b.distinct = self.distinct;
            b.limit = self.limit;
            b.offset = self.offset;
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
        Query::Describe { .. } => {
            return Err(Error::Unsupported("DESCRIBE is deferred → 501 (ADR-0007)".to_owned()))
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
        let distinct = if out.len() == 1 { out[0].distinct } else { trans.distinct };
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
        dialect,
    })
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
    fn describe_is_unsupported() {
        let q = spargebra::SparqlParser::new()
            .parse_query("DESCRIBE <http://ex/x>")
            .unwrap();
        assert!(matches!(translate(&q, &[], Dialect::Sqlite), Err(Error::Unsupported(_))));
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
        let _ = translate_cached(&q, &[], Dialect::Sqlite, &Tbox::default(), &[], &cache, e).unwrap();
        assert_eq!(cache.len(), 1, "first compile populates the cache");
        let _ = translate_cached(&q, &[], Dialect::Sqlite, &Tbox::default(), &[], &cache, e).unwrap();
        assert_eq!(cache.len(), 1, "second call is a hit, no new entry");
        let mut e2 = e;
        e2.bump();
        let _ = translate_cached(&q, &[], Dialect::Sqlite, &Tbox::default(), &[], &cache, e2).unwrap();
        assert_eq!(cache.len(), 2, "a new epoch recompiles under a fresh key");
    }

    #[test]
    fn deferred_property_path_operators_are_unsupported() {
        // The `?` (ZeroOrOne) operator stays deferred → 501 (v1 = P+/P* only); the
        // P+/P* engine path is exercised end-to-end in tests/e2e.rs.
        for q in [
            "SELECT * WHERE { ?s <http://ex/p>? ?o }",
            "SELECT * WHERE { ?s !<http://ex/p> ?o }",
            "SELECT * WHERE { ?s (<http://ex/p>/<http://ex/q>)+ ?o }",
        ] {
            let q = spargebra::SparqlParser::new().parse_query(q).unwrap();
            assert!(matches!(translate(&q, &[], Dialect::Sqlite), Err(Error::Unsupported(_))));
        }
    }
}
