//! The in-memory differential oracle (ADR-0005) — the **independent second
//! evaluator** the engine's live-SQL answer is diffed against. Two layers live
//! here, by design kept distinct so a bug in one cannot mask the other:
//!
//! * [`evaluate_dump`] — the W3C conformance gate's identity-dump oracle. The
//!   suite query is `CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }`, whose answer over
//!   a graph is exactly that graph's default-graph triples. This is computed by an
//!   independent pass over the parsed expected store — *not* by the spareval path
//!   below and *not* by reusing the comparison's canonicalization — so the W3C
//!   gate stands on its own. Kept unchanged.
//!
//! * [`evaluate`] — the **real** `spareval`-backed SPARQL evaluator. It loads an
//!   RDF graph into an in-memory `oxrdf` dataset and evaluates the *same* SPARQL
//!   the engine ran — general BGP / JOIN / OPTIONAL / FILTER **and property paths
//!   `P+`/`P*`** — returning the oracle's bindings (SELECT), triples (CONSTRUCT),
//!   or boolean (ASK). This is the ADR-0012 native-oracle differential used where
//!   there is no W3C gold file: a divergence from the engine pinpoints a
//!   base-translation bug. ADR-0004 sanctions `spareval` for this in-memory oracle
//!   **only** — it never touches the OBDA hot path.

use std::collections::BTreeMap;

use oxrdf::{Dataset, GraphName, GraphNameRef, Quad, Term};
use spareval::{QueryEvaluator, QueryResults};
use spargebra::SparqlParser;

/// Evaluate `CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }` over `expected` (the
/// in-memory store), returning the produced triples as a default-graph dataset.
/// The BGP `?s ?p ?o` binds every default-graph triple; CONSTRUCT re-emits it.
/// Independent of [`evaluate`] (the spareval path), so the W3C gate cannot be
/// masked by a shared evaluator bug (ADR-0005).
pub fn evaluate_dump(expected: &Dataset) -> Dataset {
    let solutions = expected
        .iter()
        .filter(|q| q.graph_name == GraphNameRef::DefaultGraph)
        .map(|q| {
            Quad::new(
                q.subject.into_owned(),
                q.predicate.into_owned(),
                q.object.into_owned(),
                GraphName::DefaultGraph,
            )
        });
    Dataset::from_iter(solutions)
}

/// The oracle's answer to a query — the independent ground truth the engine's
/// live-SQL answer is diffed against (ADR-0005).
#[derive(Debug, Clone)]
pub enum OracleAnswer {
    /// SELECT — a **bag** of solutions; each maps a projected variable to its
    /// bound term (an unbound variable is simply absent, matching the engine's
    /// [`sf_sparql::exec::Solutions`] form once normalised by [`engine_bag`]).
    Solutions(Vec<BTreeMap<String, Term>>),
    /// CONSTRUCT / DESCRIBE — the produced triples as a default-graph dataset
    /// (boxed: a `Dataset` dwarfs the other variants).
    Graph(Box<Dataset>),
    /// ASK.
    Boolean(bool),
}

/// Evaluate `sparql` over `graph` with `spareval` (ADR-0004's in-memory evaluator)
/// — the independent second evaluator of ADR-0005. Handles general graph patterns
/// and property paths `P+`/`P*`, so it validates query shapes the SQL-rewriting
/// path defers to `501`. Parse / evaluation failures surface as `Err`.
pub fn evaluate(graph: &Dataset, sparql: &str) -> Result<OracleAnswer, String> {
    let query = SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| format!("oracle parse: {e}"))?;
    let results = QueryEvaluator::new()
        .prepare(&query)
        .execute(graph)
        .map_err(|e| format!("oracle eval: {e}"))?;
    match results {
        QueryResults::Solutions(iter) => {
            let mut rows = Vec::new();
            for sol in iter {
                let sol = sol.map_err(|e| format!("oracle solution: {e}"))?;
                let row = sol
                    .iter()
                    .map(|(var, term)| (var.as_str().to_owned(), term.clone()))
                    .collect();
                rows.push(row);
            }
            Ok(OracleAnswer::Solutions(rows))
        }
        QueryResults::Graph(iter) => {
            let mut quads = Vec::new();
            for t in iter {
                let t = t.map_err(|e| format!("oracle triple: {e}"))?;
                quads.push(Quad::new(t.subject, t.predicate, t.object, GraphName::DefaultGraph));
            }
            Ok(OracleAnswer::Graph(Box::new(Dataset::from_iter(quads))))
        }
        QueryResults::Boolean(b) => Ok(OracleAnswer::Boolean(b)),
    }
}

/// Normalise an engine `SELECT` result ([`sf_sparql::exec::Solutions`]) into the
/// oracle's bag form — dropping unbound (`None`) variables so the two evaluators'
/// answers are directly bag-comparable via [`solutions_bag_eq`].
pub fn engine_bag(sols: &sf_sparql::exec::Solutions) -> Vec<BTreeMap<String, Term>> {
    sols.rows
        .iter()
        .map(|row| {
            sols.vars
                .iter()
                .zip(row)
                .filter_map(|(v, t)| t.clone().map(|t| (v.clone(), t)))
                .collect()
        })
        .collect()
}

/// Multiset (bag) equality of two SELECT solution sets. SPARQL SELECT results are
/// **bags**: order is irrelevant but multiplicity is significant (the `=_bag`
/// invariant, ADR-0007). Terms are compared structurally; answers that project
/// blank nodes must be canonicalized first (the conformance gate uses graph
/// isomorphism for that — these differential queries project IRIs/literals).
pub fn solutions_bag_eq(a: &[BTreeMap<String, Term>], b: &[BTreeMap<String, Term>]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut remaining: Vec<&BTreeMap<String, Term>> = b.iter().collect();
    for row in a {
        match remaining.iter().position(|r| *r == row) {
            Some(pos) => {
                remaining.swap_remove(pos);
            }
            None => return false,
        }
    }
    remaining.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{isomorphic, parse_turtle};

    #[test]
    fn dump_oracle_returns_default_graph_triples() {
        let g = parse_turtle("@prefix : <http://e/> . :s :p :o . :s :q :r .", "http://e/").unwrap();
        let out = evaluate_dump(&g);
        assert_eq!(out.len(), 2);
        assert!(isomorphic(&out, &g));
    }

    #[test]
    fn evaluate_runs_a_real_bgp_filter_query() {
        // The spareval oracle is a *real* evaluator (not a gold-file clone): it
        // computes a SELECT answer with a BGP + FILTER over an in-memory graph.
        let g = parse_turtle(
            "@prefix ex: <http://ex/> . ex:a ex:age 30 . ex:b ex:age 20 .",
            "http://ex/",
        )
        .unwrap();
        let ans = evaluate(
            &g,
            "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?n FILTER(?n > 25) }",
        )
        .unwrap();
        match ans {
            OracleAnswer::Solutions(rows) => {
                assert_eq!(rows.len(), 1, "only ex:a has age > 25");
                assert_eq!(rows[0]["s"].to_string(), "<http://ex/a>");
            }
            other => panic!("expected Solutions, got {other:?}"),
        }
    }

    #[test]
    fn bag_equality_is_multiplicity_sensitive() {
        let g = parse_turtle("@prefix ex: <http://ex/> . ex:a ex:p ex:x .", "http://ex/").unwrap();
        let one = match evaluate(&g, "SELECT ?s WHERE { ?s ?p ?o }").unwrap() {
            OracleAnswer::Solutions(r) => r,
            _ => unreachable!(),
        };
        assert!(solutions_bag_eq(&one, &one.clone()));
        let mut dup = one.clone();
        dup.extend(one.clone());
        assert!(!solutions_bag_eq(&one, &dup), "differing multiplicity ⇒ unequal bags");
    }
}
