//! Property-path differential (ADR-0007 recursive/CTE path translation vs the
//! ADR-0005 `spareval` oracle) for the operators beyond single-predicate `P+`/`P*`:
//! `^p` (inverse), `p/q` (sequence), `p|q` (alternative), `p?` (zero-or-one),
//! `!p` (negated property set), and composite `(…)+` closures. For each supported
//! shape the engine's **live SQL** answer (SPARQL→SQL over a real SQLite source via
//! the recursive/non-recursive CTE) is diffed against the independent `spareval`
//! evaluator over the SAME hand-authored triples; the two must agree as **bags**
//! (`=_bag` — distinct node pairs, but multiplicity-checked so a duplicate-pair or
//! depth-leak bug fails). The deferred shapes are asserted to stay an explicit 501.

use rusqlite::Connection;
use sf_conformance::graph::parse_turtle;
use sf_conformance::oracle::{self, OracleAnswer};
use sf_conformance::sqlite;
use sf_sparql::{exec, translate_with, Tbox};
use sf_sql::Dialect;
use spargebra::SparqlParser;
use std::collections::BTreeMap;

const BASE: &str = "http://ex/";

/// Engine answer: SPARQL → SQL over a live SQLite source, normalised to the bag.
fn engine_bag(create: &str, r2rml: &str, query: &str) -> Vec<BTreeMap<String, oxrdf::Term>> {
    let conn: Connection = sqlite::load(create).expect("fixture loads");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let schema = sqlite::introspect_all(&conn).expect("introspection");
    let q = SparqlParser::new()
        .parse_query(query)
        .expect("query parses");
    let plan = translate_with(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
        .expect("path translates");
    oracle::engine_bag(&exec::select(&plan, &conn).expect("exec"))
}

/// Oracle answer: the SAME SPARQL over the hand-authored expected graph (spareval).
fn oracle_bag(ttl: &str, query: &str) -> Vec<BTreeMap<String, oxrdf::Term>> {
    let g = parse_turtle(ttl, BASE).expect("expected graph parses");
    match oracle::evaluate(&g, query).expect("oracle eval") {
        OracleAnswer::Solutions(rows) => rows,
        other => panic!("expected SELECT solutions, got {other:?}"),
    }
}

/// The differential: the two independent evaluators agree as bags. Returns the row
/// count for an additional sanity assertion at the call site.
fn assert_differential(create: &str, r2rml: &str, ttl: &str, query: &str) -> usize {
    let engine = engine_bag(create, r2rml, query);
    let oracle = oracle_bag(ttl, query);
    assert!(
        oracle::solutions_bag_eq(&engine, &oracle),
        "engine vs oracle divergence on `{query}`:\n engine={engine:#?}\n oracle={oracle:#?}"
    );
    engine.len()
}

/// A still-deferred shape must surface an explicit `Unsupported` (501), never a
/// wrong/silent answer (translation only — no source needed).
fn assert_deferred(r2rml: &str, query: &str) {
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let q = SparqlParser::new()
        .parse_query(query)
        .expect("query parses");
    let r = translate_with(&q, &maps, Dialect::Sqlite, &Tbox::default(), &[]);
    assert!(
        matches!(r, Err(sf_sparql::Error::Unsupported(_))),
        "expected 501 (Unsupported) for `{query}`, got {r:?}"
    );
}

// --- Fixture A: two predicates ex:p / ex:q, all nodes in the ex:n/ domain (one
// node shape), so sequence/alternative/inverse/NPS are all soundly composable. ---

const A_SQL: &str = r#"
CREATE TABLE pe (ps INTEGER NOT NULL, pm INTEGER NOT NULL);
CREATE TABLE qe (qm INTEGER NOT NULL, qo INTEGER NOT NULL);
INSERT INTO pe VALUES (1, 2);
INSERT INTO pe VALUES (2, 3);
INSERT INTO qe VALUES (2, 20);
INSERT INTO qe VALUES (3, 30);
"#;

const A_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#P>
    rr:logicalTable [ rr:tableName "pe" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{ps}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:template "http://ex/n/{pm}" ] ] .
<#Q>
    rr:logicalTable [ rr:tableName "qe" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{qm}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:q ; rr:objectMap [ rr:template "http://ex/n/{qo}" ] ] .
"#;

const A_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:p <http://ex/n/2> .
<http://ex/n/2> ex:p <http://ex/n/3> .
<http://ex/n/2> ex:q <http://ex/n/20> .
<http://ex/n/3> ex:q <http://ex/n/30> .
"#;

/// `^p` INVERSE — swap subject/object. (spargebra lowers a *bare* top-level `^p`
/// to the plain reversed triple `?y p ?x`, so this validates inverse semantics
/// end to end; the `HopExpr::Inverse` node itself is exercised by the composite
/// `(^p)+` closure test below, where it stays a path.)
#[test]
fn inverse_path_engine_matches_oracle() {
    let q = "PREFIX ex: <http://ex/> SELECT ?x ?y WHERE { ?x ^ex:p ?y }";
    assert_eq!(
        assert_differential(A_SQL, A_R2RML, A_TTL, q),
        2,
        "(2,1) (3,2)"
    );
}

// `p/q` SEQUENCE: spargebra lowers a *bare* top-level sequence to a BGP joined on
// a fresh blank-node variable (it never reaches the path layer), so the
// `HopExpr::Seq` node is exercised through the composite `(p/q)+` closure test
// below (which stays a path), not a bare-sequence query here.

/// `p|q` ALTERNATIVE — set union of the two hop relations.
#[test]
fn alternative_path_engine_matches_oracle() {
    let q = "PREFIX ex: <http://ex/> SELECT ?x ?y WHERE { ?x ex:p|ex:q ?y }";
    assert_eq!(
        assert_differential(A_SQL, A_R2RML, A_TTL, q),
        4,
        "2 p + 2 q pairs"
    );
}

/// `!p` NEGATED PROPERTY SET — every mapped predicate except the negated set
/// (here the complement of `ex:p` is `{ex:q}`).
#[test]
fn negated_property_set_engine_matches_oracle() {
    let q = "PREFIX ex: <http://ex/> SELECT ?x ?y WHERE { ?x !ex:p ?y }";
    assert_eq!(
        assert_differential(A_SQL, A_R2RML, A_TTL, q),
        2,
        "the ex:q pairs"
    );
}

// --- Shared-pair fixture S: ex:p and ex:q BOTH connect n/1→n/2 (one pair, two
// predicates). This separates NPS bag semantics from alternative set semantics —
// the disjoint fixture A above cannot, because no pair is reached twice. ---

const S_SQL: &str = r#"
CREATE TABLE pe (ps INTEGER NOT NULL, po INTEGER NOT NULL);
CREATE TABLE qe (qs INTEGER NOT NULL, qo INTEGER NOT NULL);
INSERT INTO pe VALUES (1, 2);
INSERT INTO qe VALUES (1, 2);
INSERT INTO qe VALUES (3, 4);
"#;

const S_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#P>
    rr:logicalTable [ rr:tableName "pe" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{ps}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:template "http://ex/n/{po}" ] ] .
<#Q>
    rr:logicalTable [ rr:tableName "qe" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{qs}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:q ; rr:objectMap [ rr:template "http://ex/n/{qo}" ] ] .
"#;

const S_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:p <http://ex/n/2> .
<http://ex/n/1> ex:q <http://ex/n/2> .
<http://ex/n/3> ex:q <http://ex/n/4> .
"#;

/// `!p` NPS as a **BAG** over a shared pair (the regression this fixes). The
/// complement of `ex:nope` is `{ex:p, ex:q}`; (n/1, n/2) is connected by BOTH, so
/// the oracle returns it twice (one solution per matching triple, §18.2.2). An
/// `Alt`+`UNION`+outer-`DISTINCT` emission would collapse it to one and silently
/// undercount — `solutions_bag_eq` (multiplicity-checked) catches that.
#[test]
fn negated_property_set_bag_shared_pair_matches_oracle() {
    let q = "PREFIX ex: <http://ex/> SELECT ?x ?y WHERE { ?x !ex:nope ?y }";
    assert_eq!(
        assert_differential(S_SQL, S_R2RML, S_TTL, q),
        3,
        "(1,2) via p + (1,2) via q + (3,4) via q — bag of 3"
    );
}

/// `p|q` ALTERNATIVE over the SAME shared pair is **SET**-valued in the oracle:
/// (n/1, n/2) is reached via both branches but counted once. This pins the bag
/// (NPS) vs set (alternative) split so a fix to one cannot regress the other.
#[test]
fn alternative_set_shared_pair_matches_oracle() {
    let q = "PREFIX ex: <http://ex/> SELECT ?x ?y WHERE { ?x ex:p|ex:q ?y }";
    assert_eq!(
        assert_differential(S_SQL, S_R2RML, S_TTL, q),
        2,
        "(1,2) once (set) + (3,4)"
    );
}

// --- Single-predicate acyclic edge fixture: the reflexive (P*/p?) shapes are
// sound here because the hop's node set equals the active graph's node set. ---

const E_SQL: &str = r#"
CREATE TABLE edge (parent INTEGER NOT NULL, child INTEGER NOT NULL);
INSERT INTO edge VALUES (1, 2);
INSERT INTO edge VALUES (2, 3);
INSERT INTO edge VALUES (3, 4);
INSERT INTO edge VALUES (1, 5);
"#;

const EDGE_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Edge>
    rr:logicalTable [ rr:tableName "edge" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{parent}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:reaches ; rr:objectMap [ rr:template "http://ex/n/{child}" ] ] .
"#;

const E_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:reaches <http://ex/n/2> .
<http://ex/n/2> ex:reaches <http://ex/n/3> .
<http://ex/n/3> ex:reaches <http://ex/n/4> .
<http://ex/n/1> ex:reaches <http://ex/n/5> .
"#;

/// `p?` ZeroOrOne — the hop ∪ the reflexive `(x,x)` pairs over the graph's nodes
/// (SPARQL §9.3). Five nodes ⇒ 5 reflexive pairs + 4 edges = 9 pairs.
#[test]
fn zero_or_one_path_engine_matches_oracle() {
    let q = "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:reaches? ?o }";
    assert_eq!(assert_differential(E_SQL, EDGE_R2RML, E_TTL, q), 9);
}

// --- Cyclic edge fixture (1→2→3→1 + chord 1→3): a composite closure must
// terminate (depth bound) and return each reachable pair exactly once. ---

const C_SQL: &str = r#"
CREATE TABLE edge (parent INTEGER NOT NULL, child INTEGER NOT NULL);
INSERT INTO edge VALUES (1, 2);
INSERT INTO edge VALUES (2, 3);
INSERT INTO edge VALUES (3, 1);
INSERT INTO edge VALUES (1, 3);
"#;

const C_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:reaches <http://ex/n/2> .
<http://ex/n/2> ex:reaches <http://ex/n/3> .
<http://ex/n/3> ex:reaches <http://ex/n/1> .
<http://ex/n/1> ex:reaches <http://ex/n/3> .
"#;

/// COMPOSITE `(^p)+` over a CYCLIC graph — the generalised recursive CTE closes
/// over an inverse one-hop relation, must terminate, and returns each reachable
/// (reversed) pair exactly once. Asserted as a BAG against the oracle: a depth-leak
/// (missing the outer DISTINCT-pairs) or a non-terminating cycle would diverge.
#[test]
fn composite_inverse_plus_cyclic_engine_matches_oracle_as_bag() {
    let q = "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s (^ex:reaches)+ ?o }";
    let n = assert_differential(C_SQL, EDGE_R2RML, C_TTL, q);
    assert!(n > 0, "the reversed closure must return pairs");
}

/// COMPOSITE `(p/q)+` over a cyclic-by-construction sequence — the one-hop relation
/// is itself a sequence, closed transitively, bounded + cycle-safe.
#[test]
fn composite_sequence_plus_engine_matches_oracle_as_bag() {
    // edges chain so p/q forms a cycle: 1-p->1, 1-q->2, 2-p->2, 2-q->1.
    const SQL: &str = r#"
CREATE TABLE pe (ps INTEGER NOT NULL, pm INTEGER NOT NULL);
CREATE TABLE qe (qm INTEGER NOT NULL, qo INTEGER NOT NULL);
INSERT INTO pe VALUES (1, 1);
INSERT INTO pe VALUES (2, 2);
INSERT INTO qe VALUES (1, 2);
INSERT INTO qe VALUES (2, 1);
"#;
    const TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:p <http://ex/n/1> .
<http://ex/n/2> ex:p <http://ex/n/2> .
<http://ex/n/1> ex:q <http://ex/n/2> .
<http://ex/n/2> ex:q <http://ex/n/1> .
"#;
    let q = "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s (ex:p/ex:q)+ ?o }";
    let n = assert_differential(SQL, A_R2RML, TTL, q);
    assert!(n > 0, "the composite closure must return pairs");
}

// --- Deferred shapes stay an explicit 501 (documented, never silently wrong). ---

/// A shape-mismatch fixture: `ex:r`'s subject is in a DIFFERENT node domain
/// (`ex:m/`), so a raw-key join on the `ex:p`→`ex:r` middle node is unsound.
const MISMATCH_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#P>
    rr:logicalTable [ rr:tableName "pe" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{ps}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:template "http://ex/n/{pm}" ] ] .
<#R>
    rr:logicalTable [ rr:tableName "re" ] ;
    rr:subjectMap [ rr:template "http://ex/m/{rm}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:r ; rr:objectMap [ rr:template "http://ex/n/{ro}" ] ] .
"#;

#[test]
fn deferred_path_shapes_return_501() {
    // Bound endpoint — outside `?s PATH ?o`.
    assert_deferred(
        EDGE_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?o WHERE { <http://ex/n/1> ex:reaches+ ?o }",
    );
    // Nested closure operator inside a composite hop relation.
    assert_deferred(
        A_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s (ex:p+)/ex:q ?o }",
    );
    // Sequence whose middle node crosses two node shapes (ex:n/ vs ex:m/) — tested
    // inside a closure so it stays a path (a bare `p/q` is lowered to a BGP).
    assert_deferred(
        MISMATCH_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s (ex:p/ex:r)+ ?o }",
    );
    // p? reflexive over a MULTI-predicate graph — the ZeroLengthPath node set is
    // not the single-predicate hop node set.
    assert_deferred(
        A_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:p? ?o }",
    );
    // A negated property set nested inside a set-valued alternative — its bag
    // semantics cannot be preserved through the surrounding `DISTINCT` → 501
    // (rather than silently undercount, as a naive Alt-flatten would).
    assert_deferred(
        A_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s (!ex:p)|ex:q ?o }",
    );
}
