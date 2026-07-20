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
use sf_sparql::{exec, translate_with, translate_with_flat, Tbox};
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

/// Engine answer via the FLAT engine (`unfold::merge`) — used only where a
/// fixture needs BOTH engines checked (flat and tree share `path.rs`'s hop
/// resolution, so a fix there is expected to cover both, ADR-0023 M3/M5).
fn engine_bag_flat(create: &str, r2rml: &str, query: &str) -> Vec<BTreeMap<String, oxrdf::Term>> {
    let conn: Connection = sqlite::load(create).expect("fixture loads");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let schema = sqlite::introspect_all(&conn).expect("introspection");
    let q = SparqlParser::new()
        .parse_query(query)
        .expect("query parses");
    let plan = translate_with_flat(&q, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
        .expect("path translates (flat)");
    oracle::engine_bag(&exec::select(&plan, &conn).expect("exec"))
}

/// The flat-engine differential (mirrors [`assert_differential`]).
fn assert_differential_flat(create: &str, r2rml: &str, ttl: &str, query: &str) -> usize {
    let engine = engine_bag_flat(create, r2rml, query);
    let oracle = oracle_bag(ttl, query);
    assert!(
        oracle::solutions_bag_eq(&engine, &oracle),
        "flat engine vs oracle divergence on `{query}`:\n engine={engine:#?}\n oracle={oracle:#?}"
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

// --- Nullable object-column fixture — R2RML §11: a NULL-valued referenced
// column means NO triple is generated for that row at all (matching what the
// non-path `atom()` triple-pattern emission already enforces via its
// `obj_null_guard`, `unfold.rs`). Row 2's `friend_id` is NULL, so person 2
// asserts NO `ex:knows` edge; only 1->2 and 3->1 are real. A missing NULL
// guard in `hop_sql` lets the NULL leak into the hop relation as a phantom
// `(2, NULL)` one-hop pair, which a recursive closure then chains through
// transitively (`(1, NULL)` at depth 2, `(3, NULL)` at depth 3). `label` is a
// NOT NULL column joined on the path's SUBJECT (`?p`) — never NULL — so the
// joined test still forces ADR-0033's `convert_path_branches` derived-table
// wrapping without the join itself masking the phantom (a join keyed on the
// path's OBJECT would coincidentally filter NULL out via ordinary SQL
// equi-join semantics, hiding the bug rather than proving it). `label` is
// declared as a SEPARATE triples map (`NULLABLE_R2RML_JOINED`, not folded into
// `<#Knows>`'s own POMs) so the base `NULLABLE_R2RML` mapping stays
// single-predicate — `p?`'s reflexive enumeration requires that precondition
// (`graph_is_single_predicate`) and must not 501 on an unrelated added POM. ---

const NULLABLE_SQL: &str = r#"
CREATE TABLE friend (id INTEGER NOT NULL, friend_id INTEGER, label TEXT NOT NULL);
INSERT INTO friend VALUES (1, 2, 'Alice');
INSERT INTO friend VALUES (2, NULL, 'Bob');
INSERT INTO friend VALUES (3, 1, 'Carol');
"#;

const NULLABLE_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Knows>
    rr:logicalTable [ rr:tableName "friend" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:knows ; rr:objectMap [ rr:template "http://ex/n/{friend_id}" ] ] .
"#;

const NULLABLE_R2RML_JOINED: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Knows>
    rr:logicalTable [ rr:tableName "friend" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:knows ; rr:objectMap [ rr:template "http://ex/n/{friend_id}" ] ] .
<#Label>
    rr:logicalTable [ rr:tableName "friend" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:label ; rr:objectMap [ rr:column "label" ] ] .
"#;

// Row 2's NULL `friend_id` drops that whole virtual triple (R2RML §11) — only
// the two real edges exist.
const NULLABLE_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:knows <http://ex/n/2> .
<http://ex/n/3> ex:knows <http://ex/n/1> .
"#;

const NULLABLE_TTL_JOINED: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:knows <http://ex/n/2> .
<http://ex/n/3> ex:knows <http://ex/n/1> .
<http://ex/n/1> ex:label "Alice" .
<http://ex/n/2> ex:label "Bob" .
<http://ex/n/3> ex:label "Carol" .
"#;

/// `ex:knows+` transitive closure over a nullable object column, standalone.
/// Correct answer: (1,2) (3,1) (3,1->2 transitively = 3,2) — 3 pairs. A
/// missing `IS NOT NULL` guard in `hop_sql` phantom-chains through node 2's
/// NULL row, adding spurious `(_, NULL)` pairs at increasing depth.
#[test]
fn transitive_closure_over_nullable_object_column_drops_the_null_row_standalone() {
    let q = "PREFIX ex: <http://ex/> SELECT ?p ?x WHERE { ?p ex:knows+ ?x }";
    assert_eq!(
        assert_differential(NULLABLE_SQL, NULLABLE_R2RML, NULLABLE_TTL, q),
        3,
        "(1,2) (3,1) (3,2) — no phantom (_, NULL) pairs"
    );
}

/// The same nullable-hop closure JOINED with another pattern on the path's
/// SUBJECT (forcing ADR-0033's `convert_path_branches` derived-table
/// wrapping): each of the 3 correct `(p,x)` pairs joins with exactly one
/// `(p,label)` row, so a phantom `(_, NULL)` pair surviving the join
/// composition would inflate the count past 3.
#[test]
fn transitive_closure_over_nullable_object_column_drops_the_null_row_joined() {
    let q = "PREFIX ex: <http://ex/> SELECT ?p ?x ?l WHERE { ?p ex:knows+ ?x . ?p ex:label ?l }";
    assert_eq!(
        assert_differential(NULLABLE_SQL, NULLABLE_R2RML_JOINED, NULLABLE_TTL_JOINED, q),
        3,
        "(1,2,Alice) (3,1,Carol) (3,2,Carol) — no phantom rows"
    );
}

/// `p?` ZeroOrOne's reflexive `(x,x)` enumeration reads the SAME raw columns
/// directly (`reflexive_sql`, not `hop_sql`) — a separate emission path that
/// needs the identical NULL guard. Without it, node 2's NULL `friend_id`
/// still contributes a phantom `(NULL,NULL)` reflexive pair (`reflexive_sql`
/// projects `friend_id` as BOTH `sf_s`/`sf_o` in its second `UNION` half,
/// regardless of the row's OTHER column). Correct: the hop's 2 real edges (as
/// pairs) + 3 reflexive pairs (one per real node 1/2/3) = 5.
#[test]
fn zero_or_one_path_over_nullable_object_column_has_no_phantom_reflexive_pair() {
    let q = "PREFIX ex: <http://ex/> SELECT ?p ?x WHERE { ?p ex:knows? ?x }";
    assert_eq!(
        assert_differential(NULLABLE_SQL, NULLABLE_R2RML, NULLABLE_TTL, q),
        5,
        "(1,2) (3,1) + reflexive (1,1) (2,2) (3,3) — no (NULL,NULL) phantom"
    );
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

// --- D6 non-star path-endpoint join boundary pin (ADR-0032 D6 item 4): the
// GENERAL "no join onto any path branch" restriction, with NO star pattern
// anywhere, pinned precisely so a future change to `align_templates` or the
// tree/flat join machinery cannot silently shift this boundary unnoticed —
// exactly the boundary `differential_star.rs`'s own
// `star_pattern_at_property_path_endpoint_flat_501s_tree_proves_empty_a_known_divergence`
// found the TREE path escaping (via literal-prefix disjointness) for a STAR
// shape; this fixture asks the SAME structural question with an ORDINARY
// `rr:class`-derived `rdf:type` fact instead of a star pattern, to isolate
// whether that escape is star-specific or a general consequence of the W2b
// lift. `#Cls` is a SEPARATE, small triples map (not `rr:class` piggybacked
// on `#Q`'s own subjectMap) so the `rdf:type ex:C` fact genuinely
// DISCRIMINATES (only `http://ex/n/1` carries it) rather than trivially
// holding for every `?id` — proving the join actually filters, not merely
// that it is accepted. ---

const PJ_SQL: &str = r#"
CREATE TABLE pj_q (qs INTEGER NOT NULL, qo INTEGER NOT NULL);
CREATE TABLE pj_r (rs INTEGER NOT NULL, ro INTEGER NOT NULL);
CREATE TABLE pj_cls (id INTEGER NOT NULL);
INSERT INTO pj_q VALUES (1, 2);
INSERT INTO pj_q VALUES (2, 3);
INSERT INTO pj_r VALUES (2, 20);
INSERT INTO pj_r VALUES (3, 30);
INSERT INTO pj_cls VALUES (1);
"#;

const PJ_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Q>
    rr:logicalTable [ rr:tableName "pj_q" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{qs}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:q ; rr:objectMap [ rr:template "http://ex/n/{qo}" ] ] .
<#R>
    rr:logicalTable [ rr:tableName "pj_r" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{rs}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:r ; rr:objectMap [ rr:template "http://ex/n/{ro}" ] ] .
<#Cls>
    rr:logicalTable [ rr:tableName "pj_cls" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{id}" ; rr:class ex:C ] .
"#;

const PJ_SEQ_QUERY: &str =
    "PREFIX ex: <http://ex/> PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
     SELECT ?id ?x WHERE { ?id ex:q/ex:r ?x . ?id rdf:type ex:C }";
const PJ_PLUS_QUERY: &str =
    "PREFIX ex: <http://ex/> PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
     SELECT ?id ?x WHERE { ?id ex:q+ ?x . ?id rdf:type ex:C }";

/// The BARE-SEQUENCE variant (`?id ex:q/ex:r ?x . ?id rdf:type ex:C`) is
/// **not actually a path-endpoint join at all**: spargebra lowers a bare
/// top-level sequence to an ordinary BGP joined on a fresh blank-node
/// variable (`?id ex:q ?_b . ?_b ex:r ?x .`, this file's own established
/// finding, see the module doc above) — it never becomes a
/// `GraphPattern::Path` node, so the "no join onto a path branch" boundary
/// never triggers. Both engines translate and execute it as an ORDINARY
/// 3-triple-pattern BGP; verified end to end (not just "doesn't 501") —
/// `?id = n/2` also satisfies `ex:q/ex:r` (n/2→n/3→n/30) but is correctly
/// EXCLUDED because only `n/1` carries `rdf:type ex:C`, proving the extra
/// pattern genuinely filters rather than merely being accepted.
#[test]
fn bare_sequence_joined_with_class_pattern_is_an_ordinary_bgp_not_a_path_join() {
    assert_eq!(
        assert_differential(
            PJ_SQL,
            PJ_R2RML,
            "@prefix ex: <http://ex/> . \
             <http://ex/n/1> ex:q <http://ex/n/2> . <http://ex/n/2> ex:q <http://ex/n/3> . \
             <http://ex/n/2> ex:r <http://ex/n/20> . <http://ex/n/3> ex:r <http://ex/n/30> . \
             <http://ex/n/1> a ex:C .",
            PJ_SEQ_QUERY
        ),
        1,
        "only n/1 (rdf:type ex:C) survives the join; n/2's own ex:q/ex:r pair is filtered out"
    );

    // Same fixture, run directly (not through the oracle) to pin the EXACT
    // surviving row, independent of `assert_differential`'s bag-count check.
    let engine = engine_bag(PJ_SQL, PJ_R2RML, PJ_SEQ_QUERY);
    assert_eq!(engine.len(), 1, "engine={engine:#?}");
    assert_eq!(
        engine[0]["id"].to_string(),
        "<http://ex/n/1>",
        "engine={engine:#?}"
    );
    assert_eq!(
        engine[0]["x"].to_string(),
        "<http://ex/n/20>",
        "engine={engine:#?}"
    );
}

/// The CLOSURE variant (`?id ex:q+ ?x . ?id rdf:type ex:C`) IS a genuine
/// `GraphPattern::Path` node joined on its own subject endpoint — the general
/// "no join onto any path branch" boundary
/// (`unfold::merge`'s unconditional `left.path.is_some() ||
/// right.path.is_some()` check). ADR-0033 lifted this on the TREE side first:
/// at the two tree join sites (`iq/lower.rs`'s `InnerJoin`/`LeftJoin` arms) a
/// path-carrying branch is converted, BEFORE `unfold::join_branches` ever
/// sees it, into an ordinary branch whose `core` holds one `Scan` reading a
/// self-contained derived-table SQL string (the closure's own `WITH
/// [RECURSIVE] …`), with the OUTER scan alias kept IDENTICAL to the closure's
/// own alias — so the join composes exactly like any other pattern. The FLAT
/// engine's own `GraphPattern::Join` arm (`unfold.rs`) now applies the
/// IDENTICAL conversion (reusing `iq::lower::convert_path_branches` verbatim,
/// not a second copy) before its own call to `join_branches` — so `merge`'s
/// path guard is unreached from either engine's join site and BOTH now
/// return the CORRECT rows (verified against the independent `spareval`
/// oracle, not merely "doesn't 501"). This test's purpose has inverted twice:
/// "both engines 501 identically" (pre-ADR-0033), then "tree lifts, flat
/// stays pinned to 501" (ADR-0033, tree-only), now "both engines agree with
/// the oracle" (flat-engine parity).
#[test]
fn closure_joined_with_class_pattern_now_matches_oracle_on_both_engines() {
    // `ex:q+` closure over `pj_q`'s (1,2),(2,3) is {(1,2),(1,3),(2,3)}; only
    // `n/1` carries `rdf:type ex:C` (`pj_cls`), so the join keeps exactly the
    // pairs rooted at 1: (1,2) and (1,3) — 2 rows.
    const TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:q <http://ex/n/2> .
<http://ex/n/2> ex:q <http://ex/n/3> .
<http://ex/n/2> ex:r <http://ex/n/20> .
<http://ex/n/3> ex:r <http://ex/n/30> .
<http://ex/n/1> a ex:C .
"#;
    assert_eq!(
        assert_differential(PJ_SQL, PJ_R2RML, TTL, PJ_PLUS_QUERY),
        2,
        "tree: ex:q+ closure {{(1,2),(1,3),(2,3)}} filtered to id=n/1 (the only rdf:type ex:C \
         subject) leaves (1,2) and (1,3) — 2 rows"
    );
    assert_eq!(
        assert_differential_flat(PJ_SQL, PJ_R2RML, TTL, PJ_PLUS_QUERY),
        2,
        "flat: same query, same answer — the flat-engine parity this test now pins"
    );
}

// --- ADR-0033 join-composition matrix: OPTIONAL on either side, two separate
// paths joined on a shared var, and a path joined inside FILTER EXISTS. One
// shared fixture: `ex:name` on {n/1="Ann", n/11="Zed"}, `ex:next` edges
// n/1->n/10->n/11 — a chain whose CLOSURE (`ex:next+`) reaches {10,11} from 1
// and {11} from 10, so every query below exercises BOTH a match and a
// no-match branch. ---

// `oj_person`'s PRIMARY KEY is load-bearing since ADR-0034: it lets D1's
// key-coverage elision skip the dedup wrap, keeping these cells on the plain
// path-composition translation they exist to pin. (`oj_edge`'s PK is data
// faithfulness only — path closures resolve via `IqNode::Path`, which never
// reaches the D1 check; traced during the C0 follow-up.) The UNKEYED variant
// of the OPTIONAL-right shape routes through D1's SubPlan and hits the
// pre-existing ADR-0023 Item 1d boundary — pinned separately
// (`optional_right_is_path_over_unkeyed_table_is_an_adr0034_sound_501`).
const OJ_SQL: &str = r#"
CREATE TABLE oj_person (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
CREATE TABLE oj_edge (a INTEGER NOT NULL, b INTEGER NOT NULL, PRIMARY KEY (a, b));
INSERT INTO oj_person VALUES (1, 'Ann');
INSERT INTO oj_person VALUES (11, 'Zed');
INSERT INTO oj_edge VALUES (1, 10);
INSERT INTO oj_edge VALUES (10, 11);
"#;

const OJ_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Person>
    rr:logicalTable [ rr:tableName "oj_person" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] .
<#Edge>
    rr:logicalTable [ rr:tableName "oj_edge" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{a}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:next ; rr:objectMap [ rr:template "http://ex/n/{b}" ] ] .
"#;

const OJ_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:name "Ann" .
<http://ex/n/11> ex:name "Zed" .
<http://ex/n/1> ex:next <http://ex/n/10> .
<http://ex/n/10> ex:next <http://ex/n/11> .
"#;

/// OPTIONAL whose RIGHT side is a property-path closure (`build_left_join`'s
/// single-scan fast path, ADR-0033 conversion applied to the right operand
/// before the `is_single_subplan_branch` check). Ann (n/1) has a `next+`
/// closure reaching {10,11} — 2 matching rows; Zed (n/11) has none — 1
/// null-padded row. 3 rows total.
#[test]
fn optional_right_is_path_engine_matches_oracle() {
    let q = "PREFIX ex: <http://ex/> SELECT ?id ?name ?reached \
             WHERE { ?id ex:name ?name OPTIONAL { ?id ex:next+ ?reached } }";
    assert_eq!(
        assert_differential(OJ_SQL, OJ_R2RML, OJ_TTL, q),
        3,
        "Ann reaches {{10,11}} (2 rows) + Zed unbound (1 row)"
    );
}

/// The UNKEYED counterpart of `optional_right_is_path_engine_matches_oracle`.
/// UN-PINNED by Run 4 Wave C0b Item 1 (ADR-0034 D1's per-scan `SELECT DISTINCT`
/// wrap, `cascade::apply_dup_safety`): without a declared key, D1 still assumes
/// duplicate rows are possible on `oj_person`, but now dedups by rewriting
/// THAT SCAN's own source in place (`oj_person`'s bindings are both injective —
/// a single-column IRI template and a plain column — so it is wrap-eligible)
/// instead of routing the whole branch through the SubPlan mechanism. The
/// preserved side of the OPTIONAL therefore never acquires a SubPlan at all,
/// so the pre-existing ADR-0023 Item 1d correlation boundary this used to trip
/// is simply never reached — a real capability gain, not a coincidence of this
/// fixture happening to hold no duplicate rows (the wrap dedups `oj_person`
/// unconditionally, so a duplicate-carrying variant would still be correct).
#[test]
fn optional_right_is_path_over_unkeyed_table_now_answers_on_tree() {
    const UNKEYED_SQL: &str = r#"
CREATE TABLE oj_person (id INTEGER NOT NULL, name TEXT NOT NULL);
CREATE TABLE oj_edge (a INTEGER NOT NULL, b INTEGER NOT NULL);
INSERT INTO oj_person VALUES (1, 'Ann');
INSERT INTO oj_edge VALUES (1, 10);
"#;
    const UNKEYED_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:name "Ann" .
<http://ex/n/1> ex:next <http://ex/n/10> .
"#;
    let q = "PREFIX ex: <http://ex/> SELECT ?id ?name ?reached \
             WHERE { ?id ex:name ?name OPTIONAL { ?id ex:next+ ?reached } }";
    assert_eq!(
        assert_differential(UNKEYED_SQL, OJ_R2RML, UNKEYED_TTL, q),
        1,
        "Ann's only edge reaches {{10}} — 1 row"
    );
}

/// OPTIONAL whose LEFT (preceding) side is a property-path closure
/// (`build_left_join`'s `left.path.is_some()` guard, ADR-0033 conversion
/// applied to the left operand). The `next+` closure is {(1,10),(1,11),
/// (10,11)}; only reached-node 11 has an `ex:name` (Zed) — node 10 does not,
/// so that row null-pads. 3 rows, 2 matched + 1 unmatched.
#[test]
fn optional_left_is_path_engine_matches_oracle() {
    let q = "PREFIX ex: <http://ex/> SELECT ?id ?reached ?name \
             WHERE { ?id ex:next+ ?reached OPTIONAL { ?reached ex:name ?name } }";
    assert_eq!(
        assert_differential(OJ_SQL, OJ_R2RML, OJ_TTL, q),
        3,
        "(1,10,unbound) + (1,11,Zed) + (10,11,Zed)"
    );
}

/// TWO SEPARATE property-path closures joined on a shared variable — each
/// converts independently to its own derived-table `Scan` at the SAME
/// `IqNode::InnerJoin`, so `unfold::merge` sees two ordinary scan-based
/// branches and unifies them like any other join (zero special-casing).
/// `next+` = {(1,10),(1,11),(10,11)}; joining `?a next+ ?b . ?b next+ ?c`
/// keeps only `b` values that are ALSO a closure subject: b=10 has (10,11),
/// b=11 has nothing — exactly 1 row: (1,10,11).
#[test]
fn two_separate_paths_joined_on_shared_var_engine_matches_oracle() {
    let q = "PREFIX ex: <http://ex/> SELECT ?a ?b ?c \
             WHERE { ?a ex:next+ ?b . ?b ex:next+ ?c }";
    assert_eq!(
        assert_differential(OJ_SQL, OJ_R2RML, OJ_TTL, q),
        1,
        "only b=10 (reached from a=1) is itself a closure subject, reaching c=11"
    );
}

/// A property-path closure JOINED with an ordinary pattern INSIDE a FILTER
/// EXISTS body (`lower_iq_exists` reuses `lower_node`, so the SAME
/// `IqNode::InnerJoin` conversion fires inside the correlated subquery;
/// `SqlCond::Exists` CROSS-JOINs `r.core` generically, its first
/// `Query`-sourced scan). Ann's `next+` closure reaches {10,11}; node 11 has
/// an `ex:name` (Zed) — EXISTS holds for Ann. Zed's own closure (from n/11)
/// is empty — EXISTS fails for Zed. 1 row.
#[test]
fn path_joined_with_pattern_inside_filter_exists_engine_matches_oracle() {
    let q = "PREFIX ex: <http://ex/> SELECT ?id ?name \
             WHERE { ?id ex:name ?name \
             FILTER EXISTS { ?id ex:next+ ?x . ?x ex:name ?otherName } }";
    assert_eq!(
        assert_differential(OJ_SQL, OJ_R2RML, OJ_TTL, q),
        1,
        "only Ann's closure reaches a NAMED node (Zed, via n/11)"
    );
}

// --- ADR-0033 §Soundness: `!p` at PathKind::One is a BAG (UNION ALL, no outer
// DISTINCT — one solution per matching triple, §18.2.2), joined against a
// multi-row other side must reproduce that multiplicity exactly, not collapse
// it. A dedicated fixture (no existing one has both a shared-predicate-pair
// duplicate AND a multi-row join partner without changing the NPS complement
// of an already-pinned fixture — `!p`'s complement is EVERY mapped predicate
// in the WHOLE document, so a fixture reused across tests cannot safely grow
// a third predicate without also widening what `!nope` negates elsewhere). ---

const MP_SQL: &str = r#"
CREATE TABLE mp_p (a INTEGER NOT NULL, b INTEGER NOT NULL);
CREATE TABLE mp_q (a INTEGER NOT NULL, b INTEGER NOT NULL);
INSERT INTO mp_p VALUES (1, 2);
INSERT INTO mp_q VALUES (1, 2);
INSERT INTO mp_q VALUES (2, 3);
INSERT INTO mp_q VALUES (2, 4);
"#;

const MP_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#P>
    rr:logicalTable [ rr:tableName "mp_p" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{a}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rr:template "http://ex/n/{b}" ] ] .
<#Q>
    rr:logicalTable [ rr:tableName "mp_q" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{a}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:q ; rr:objectMap [ rr:template "http://ex/n/{b}" ] ] .
"#;

const MP_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:p <http://ex/n/2> .
<http://ex/n/1> ex:q <http://ex/n/2> .
<http://ex/n/2> ex:q <http://ex/n/3> .
<http://ex/n/2> ex:q <http://ex/n/4> .
"#;

/// `!ex:nope`'s complement is {p, q} (neither is negated). Bag: (1,2) via p,
/// (1,2) via q, (2,3) via q, (2,4) via q — 4 solutions, (1,2) at multiplicity
/// 2. Joined with `?y ex:q ?z`: y=2 (both (1,2) provenances) has 2 outgoing q
/// edges (z=3,4) — 2×2 = 4 rows; y=3/y=4 have none. A wrongly-deduped `!p`
/// (outer DISTINCT collapsing the two (1,2) provenances to one) would halve
/// this to 2 — `solutions_bag_eq`'s multiplicity check catches it. This is
/// ADR-0033's own mandatory soundness fixture (a duplicate-multiplicity `!p`
/// join checked against the spareval oracle's own bag counts, "not an
/// optional" test per the ADR) — checked on BOTH engines now that the flat
/// engine also composes joins onto paths: flat's `convert_path_branches`
/// pre-conversion must preserve this multiplicity exactly like tree's does,
/// not just "not 501".
#[test]
fn negated_property_set_multiplicity_joined_engine_matches_oracle_bag_counts() {
    let q = "PREFIX ex: <http://ex/> SELECT ?x ?y ?z WHERE { ?x !ex:nope ?y . ?y ex:q ?z }";
    assert_eq!(
        assert_differential(MP_SQL, MP_R2RML, MP_TTL, q),
        4,
        "tree: (1,2,3) x2 + (1,2,4) x2 — the p/q-provenance duplicate must survive the join"
    );
    assert_eq!(
        assert_differential_flat(MP_SQL, MP_R2RML, MP_TTL, q),
        4,
        "flat: same multiplicity — a wrongly-deduped !p would halve this to 2"
    );
}

// --- ADR-0033 open question: GROUP BY over a JOINED path (as opposed to
// `differential_tree.rs`'s `item7_group_by_over_property_path_now_tree_
// superset_of_flat`, which groups a STANDALONE path via the UNRELATED
// ADR-0025 Tier-2 gap 4 Rust-group routing — a standalone path never reaches
// `convert_path_branches` at all, so that mechanism is untouched by this ADR).
// This is the ordinary single-branch SQL `GROUP BY` path
// (`emit::emit_agg_branch`), which renders `core`/`subplan_joins` generically
// — the ADR flagged it as "expected to work but unverified." ---

/// COUNT over `ex:q+` joined with the `rdf:type ex:C` class pattern, grouped
/// by `?id`. `ex:q+` closure {(1,2),(1,3),(2,3)} filtered to id=n/1 (the only
/// class member) leaves x∈{2,3} — one group, count 2.
#[test]
fn group_by_over_joined_path_engine_matches_oracle() {
    let q = "PREFIX ex: <http://ex/> PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
             SELECT ?id (COUNT(?x) AS ?c) WHERE { ?id ex:q+ ?x . ?id rdf:type ex:C } GROUP BY ?id";
    const TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:q <http://ex/n/2> .
<http://ex/n/2> ex:q <http://ex/n/3> .
<http://ex/n/1> a ex:C .
"#;
    assert_eq!(
        assert_differential(PJ_SQL, PJ_R2RML, TTL, q),
        1,
        "one group (id=n/1), count 2 (x in {{2,3}})"
    );
}

// --- Wrong-graph property paths compile to an EMPTY relation, not a 501: the
// predicate IS mapped, just not in the graph being queried. R2RML §7.4 graph
// scoping is a MAPPING-level fact (a POM whose declared graph does not match
// the active one contributes NO triples, regardless of its rows' content), so
// the sound answer is 0 rows, not a refusal. `ex:reaches` is mapped via
// `rr:graphMap <http://ex/g1>` on exactly ONE triples map — a single,
// unambiguous candidate for `resolve_pred_hop`'s unscoped fallback pass to
// build the empty hop's term maps/shapes from. No `ex:reaches` triples belong
// to any OTHER graph, so both queries below see nothing — on both engines,
// which share this hop resolution (`path.rs`). ---

const WG_SQL: &str = r#"
CREATE TABLE wg_edge (parent INTEGER NOT NULL, child INTEGER NOT NULL);
INSERT INTO wg_edge VALUES (1, 2);
INSERT INTO wg_edge VALUES (2, 3);
"#;

const WG_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Edge>
    rr:logicalTable [ rr:tableName "wg_edge" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{parent}" ; rr:graphMap [ rr:constant <http://ex/g1> ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:reaches ; rr:objectMap [ rr:template "http://ex/n/{child}" ] ] .
"#;

const WG_TTL: &str = "";

/// The bug this fixes: before, `resolve_pred_hop` could not distinguish
/// "mapped, but not in THIS graph" from "not mapped at all" and refused with
/// a 501. `ex:reaches` is mapped only in `g1`, so `GRAPH <http://ex/g2>` asks
/// for a graph with zero matching edges — the sound answer is EMPTY, which
/// the oracle (no `ex:reaches` triples in scope for `g2` either) agrees with.
#[test]
fn property_path_predicate_mapped_only_in_a_different_named_graph_is_empty_not_501() {
    let q = "PREFIX ex: <http://ex/> \
             SELECT ?s ?o WHERE { GRAPH <http://ex/g2> { ?s ex:reaches+ ?o } }";
    assert_eq!(
        assert_differential(WG_SQL, WG_R2RML, WG_TTL, q),
        0,
        "ex:reaches lives only in g1 — GRAPH <g2> has no matching edges"
    );
    assert_eq!(
        assert_differential_flat(WG_SQL, WG_R2RML, WG_TTL, q),
        0,
        "same, flat engine (shares path.rs's resolve_pred_hop)"
    );
}

/// The complementary direction: `ex:reaches` is mapped only in the NAMED
/// graph `g1`, so the DEFAULT graph (no GRAPH wrapper at all) must ALSO see
/// nothing — proving the fix is symmetric, not just "the named-graph side
/// now works."
#[test]
fn property_path_predicate_mapped_only_in_a_named_graph_is_empty_in_the_default_graph() {
    let q = "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:reaches+ ?o }";
    assert_eq!(
        assert_differential(WG_SQL, WG_R2RML, WG_TTL, q),
        0,
        "ex:reaches lives only in g1 — the default graph has no matching edges"
    );
    assert_eq!(
        assert_differential_flat(WG_SQL, WG_R2RML, WG_TTL, q),
        0,
        "same, flat engine"
    );
}
