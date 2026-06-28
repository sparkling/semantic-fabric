//! The real in-memory differential oracle (ADR-0005 / ADR-0012) and the NoREC
//! internal differential (ADR-0012), end to end over a small SQLite fixture.
//!
//! * **Native-oracle differential** — the engine's live-SQL answer (SPARQL→SQL via
//!   `sf-sparql`) is diffed against the independent `spareval` oracle evaluating the
//!   *same* SPARQL over the expected RDF graph. The oracle graph here is
//!   hand-authored, so the oracle is wholly independent of the engine: agreement is
//!   real evidence of rewriter correctness (BGP / JOIN / OPTIONAL / FILTER).
//! * **Property paths** — the oracle answers a `P+` query the engine defers to
//!   `501`, proving it is a genuine evaluator (ADR-0005: the in-memory evaluator
//!   validates `P+`/`P*`).
//! * **NoREC** — the engine's optimized plan ([`translate_with`]) vs its
//!   unoptimized base translation ([`translate_unoptimized`]) over the same source
//!   must agree as bags; a divergence would pinpoint a faulty cascade rule, no
//!   oracle needed.

use rusqlite::Connection;
use sf_conformance::graph::parse_turtle;
use sf_conformance::oracle::{self, OracleAnswer};
use sf_conformance::sqlite;
use sf_sparql::{exec, translate_unoptimized, translate_with, Tbox};
use sf_sql::Dialect;
use spargebra::SparqlParser;

/// Base IRI for the hand-authored expected graph (matches the mapping templates).
const BASE: &str = "http://ex/";

/// A small relational fixture: `person`(PK `id`, NOT-NULL `name`, NOT-NULL FK
/// `dept_id`, nullable `email`) ⟕ `dept`(PK `id`, NOT-NULL `label`).
const CREATE_SQL: &str = r#"
CREATE TABLE dept (id INTEGER PRIMARY KEY, label TEXT NOT NULL);
CREATE TABLE person (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    dept_id INTEGER NOT NULL,
    email TEXT,
    FOREIGN KEY (dept_id) REFERENCES dept(id)
);
INSERT INTO dept VALUES (10, 'Sales');
INSERT INTO person VALUES (1, 'Ann', 10, 'ann@x');
INSERT INTO person VALUES (2, 'Bob', 10, NULL);
INSERT INTO person VALUES (3, 'Zed', 10, 'zed@x');
"#;

/// R2RML for the fixture: `ex:name` (literal), `ex:dept` (referencing object map →
/// dept subject IRI via the `dept_id` join), `ex:email` (nullable literal), and
/// `ex:label` on the dept map.
const R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .

<#Person>
    rr:logicalTable [ rr:tableName "person" ] ;
    rr:subjectMap [ rr:template "http://ex/person/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:name  ; rr:objectMap [ rr:column "name" ] ] ;
    rr:predicateObjectMap [ rr:predicate ex:email ; rr:objectMap [ rr:column "email" ] ] ;
    rr:predicateObjectMap [
        rr:predicate ex:dept ;
        rr:objectMap [
            rr:parentTriplesMap <#Dept> ;
            rr:joinCondition [ rr:child "dept_id" ; rr:parent "id" ]
        ]
    ] .

<#Dept>
    rr:logicalTable [ rr:tableName "dept" ] ;
    rr:subjectMap [ rr:template "http://ex/dept/{id}" ] ;
    rr:predicateObjectMap [ rr:predicate ex:label ; rr:objectMap [ rr:column "label" ] ] .
"#;

/// The expected RDF graph the mapping virtualises — hand-authored so the oracle is
/// independent of the engine. (Bob has a NULL `email`, so no `ex:email` triple.)
const EXPECTED_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/person/1> ex:name "Ann" ; ex:dept <http://ex/dept/10> ; ex:email "ann@x" .
<http://ex/person/2> ex:name "Bob" ; ex:dept <http://ex/dept/10> .
<http://ex/person/3> ex:name "Zed" ; ex:dept <http://ex/dept/10> ; ex:email "zed@x" .
<http://ex/dept/10> ex:label "Sales" .
"#;

fn fixture() -> (Connection, Vec<sf_core::ir::TriplesMap>, Vec<sf_sql::TableSchema>) {
    let conn = sqlite::load(CREATE_SQL).expect("fixture loads");
    let maps = sf_mapping::parse_r2rml(R2RML).expect("R2RML parses");
    let schema = sqlite::introspect_all(&conn).expect("introspection");
    (conn, maps, schema)
}

fn engine_plan(
    maps: &[sf_core::ir::TriplesMap],
    schema: &[sf_sql::TableSchema],
    sparql: &str,
    optimize: bool,
) -> sf_sparql::Plan {
    let query = SparqlParser::new().parse_query(sparql).expect("query parses");
    if optimize {
        translate_with(&query, maps, Dialect::Sqlite, &Tbox::default(), schema)
    } else {
        translate_unoptimized(&query, maps, Dialect::Sqlite, &Tbox::default(), schema)
    }
    .expect("translate")
}

fn engine_select(
    conn: &Connection,
    maps: &[sf_core::ir::TriplesMap],
    schema: &[sf_sql::TableSchema],
    sparql: &str,
    optimize: bool,
) -> exec::Solutions {
    let plan = engine_plan(maps, schema, sparql, optimize);
    exec::select(&plan, conn).expect("execute")
}

/// Count relation references in a plan's emitted SQL — one per base-table or
/// subquery scan and one per join keyword. Fewer ⇒ joins/scans were eliminated.
fn scan_count(plan: &sf_sparql::Plan) -> usize {
    plan.emitted()
        .expect("emit")
        .iter()
        .map(|e| {
            let s = &e.sql;
            // FROM contributes one base relation; each JOIN keyword adds another.
            s.matches(" JOIN ").count() + usize::from(s.contains(" FROM "))
        })
        .sum()
}

/// (a) The oracle INDEPENDENTLY computes a non-trivial BGP + JOIN + OPTIONAL +
/// FILTER answer that matches the engine's live-SQL answer (ADR-0005 native-oracle
/// differential).
#[test]
fn oracle_matches_engine_on_bgp_join_optional_filter() {
    let (conn, maps, schema) = fixture();
    let q = r#"
        PREFIX ex: <http://ex/>
        SELECT ?name ?label ?email WHERE {
            ?p ex:name ?name .
            ?p ex:dept ?d .
            ?d ex:label ?label .
            OPTIONAL { ?p ex:email ?email }
            FILTER (?name != "Zed")
        }"#;

    // Engine: SPARQL → SQL over the live SQLite source.
    let engine = oracle::engine_bag(&engine_select(&conn, &maps, &schema, q, true));

    // Oracle: the SAME SPARQL over the hand-authored expected graph (spareval).
    let expected = parse_turtle(EXPECTED_TTL, BASE).expect("expected graph parses");
    let OracleAnswer::Solutions(oracle_rows) = oracle::evaluate(&expected, q).expect("oracle eval")
    else {
        panic!("expected SELECT solutions from the oracle");
    };

    // The differential: the two independent evaluators agree as bags.
    assert!(
        oracle::solutions_bag_eq(&engine, &oracle_rows),
        "engine vs oracle divergence:\n engine={engine:#?}\n oracle={oracle_rows:#?}"
    );
    // Ann (with Sales + ann@x) and Bob (Sales, email unbound); Zed filtered out.
    assert_eq!(engine.len(), 2, "two rows survive the FILTER");
    assert!(
        engine.iter().any(|r| r.get("name").map(|t| t.to_string()) == Some("\"Bob\"".into())
            && !r.contains_key("email")),
        "Bob's NULL email is correctly unbound through the OPTIONAL"
    );
}

/// (b) A property-path query is answered by the oracle — proving it is a real
/// evaluator — even though the engine defers paths to `501` today (ADR-0005:
/// the in-memory evaluator validates `P+`/`P*`).
#[test]
fn oracle_evaluates_property_path_engine_defers_501() {
    let chain = r#"
        @prefix ex: <http://ex/> .
        ex:a ex:knows ex:b .
        ex:b ex:knows ex:c ."#;
    let graph = parse_turtle(chain, BASE).expect("chain parses");
    let path_q = "PREFIX ex: <http://ex/> SELECT ?x WHERE { ex:a ex:knows+ ?x }";

    // The oracle resolves the transitive closure: {b, c}.
    let OracleAnswer::Solutions(rows) = oracle::evaluate(&graph, path_q).expect("oracle eval")
    else {
        panic!("expected SELECT solutions");
    };
    let mut reached: Vec<String> = rows.iter().map(|r| r["x"].to_string()).collect();
    reached.sort();
    assert_eq!(reached, vec!["<http://ex/b>".to_owned(), "<http://ex/c>".to_owned()]);

    // The engine defers property paths to 501 (documented gap, never a wrong answer).
    let query = SparqlParser::new().parse_query(path_q).unwrap();
    let translated = translate_with(&query, &[], Dialect::Sqlite, &Tbox::default(), &[]);
    assert!(
        matches!(translated, Err(sf_sparql::Error::Unsupported(_))),
        "the engine defers P+ to 501 — the oracle is the evaluator that covers it"
    );
}

/// (b2) Property-path differential (ADR-0007 recursive CTE vs ADR-0005 oracle) —
/// the key correctness proof for the recursive path translation. A transitive
/// `edge(parent, child)` relation is mapped so `?s ex:reaches ?o` is the one-hop
/// relation. Both `P+` (transitive closure) and `P*` (closure ∪ reflexive pairs)
/// are evaluated through BOTH the engine's *live* recursive-CTE SQL and the
/// independent `spareval` oracle over the SAME triples (hand-authored, so the
/// oracle is wholly independent of the engine). The two must agree as bags: the
/// engine's SQL recursion vs an independent path evaluator, zero divergence.
#[test]
fn property_path_plus_and_star_engine_matches_oracle() {
    const EDGE_SQL: &str = r#"
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
    rr:predicateObjectMap [
        rr:predicate ex:reaches ;
        rr:objectMap [ rr:template "http://ex/n/{child}" ]
    ] .
"#;
    // The one-hop edges — hand-authored so the oracle is independent of the engine.
    const EDGES_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:reaches <http://ex/n/2> .
<http://ex/n/2> ex:reaches <http://ex/n/3> .
<http://ex/n/3> ex:reaches <http://ex/n/4> .
<http://ex/n/1> ex:reaches <http://ex/n/5> .
"#;
    let conn = sqlite::load(EDGE_SQL).expect("edge fixture loads");
    let maps = sf_mapping::parse_r2rml(EDGE_R2RML).expect("edge R2RML parses");
    let schema = sqlite::introspect_all(&conn).expect("introspection");
    let graph = parse_turtle(EDGES_TTL, BASE).expect("edges parse");

    for path in ["+", "*"] {
        let q =
            format!("PREFIX ex: <http://ex/> SELECT ?s ?o WHERE {{ ?s ex:reaches{path} ?o }}");

        // Engine: the path compiles to a depth-bounded recursive CTE (ADR-0010).
        let plan = engine_plan(&maps, &schema, &q, true);
        let sql = &plan.emitted().expect("emit")[0].sql;
        let up = sql.to_uppercase();
        assert!(up.contains("WITH RECURSIVE"), "recursive CTE for reaches{path}: {sql}");
        assert!(
            sql.contains("256"),
            "ADR-0010 recursion-depth bound present in the CTE for reaches{path}: {sql}"
        );

        // Engine answer over the live SQLite source via the recursive CTE.
        let engine = oracle::engine_bag(&exec::select(&plan, &conn).expect("exec"));

        // Oracle: spareval evaluates the SAME path over the SAME triples in-memory.
        let OracleAnswer::Solutions(oracle_rows) =
            oracle::evaluate(&graph, &q).expect("oracle eval")
        else {
            panic!("expected SELECT solutions from the oracle");
        };

        // The differential: SQL recursion vs independent path evaluator, =_bag.
        assert!(
            oracle::solutions_bag_eq(&engine, &oracle_rows),
            "engine vs oracle divergence on reaches{path}:\n engine={engine:#?}\n oracle={oracle_rows:#?}"
        );
        assert!(!engine.is_empty(), "reaches{path} must return rows");
    }
}

/// (b3) Property-path differential over a CYCLIC + unequal-length-path graph — the
/// case the acyclic single-path fixture (b2) cannot exercise. The edge relation
/// `1→2→3→1` (a 3-cycle) plus the chord `1→3` means many pairs are reachable at
/// several depths or repeatedly around the cycle. A correct `P+`/`P*` is
/// set-semantics over node *pairs*, so it must return each reachable pair exactly
/// once and terminate. This is asserted as a **BAG** (`Vec`, duplicates counted)
/// against the independent `spareval` oracle: a missing outer `SELECT DISTINCT`
/// on the pair (the depth dimension leaking into the result) would emit each pair
/// once per depth and fail the bag equality. Termination proves the depth bound
/// is a real cycle terminator.
#[test]
fn property_path_cyclic_and_diamond_engine_matches_oracle_as_bag() {
    const EDGE_SQL: &str = r#"
CREATE TABLE edge (parent INTEGER NOT NULL, child INTEGER NOT NULL);
INSERT INTO edge VALUES (1, 2);
INSERT INTO edge VALUES (2, 3);
INSERT INTO edge VALUES (3, 1);
INSERT INTO edge VALUES (1, 3);
"#;
    const EDGE_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Edge>
    rr:logicalTable [ rr:tableName "edge" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{parent}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:reaches ;
        rr:objectMap [ rr:template "http://ex/n/{child}" ]
    ] .
"#;
    // The one-hop edges — a 3-cycle 1→2→3→1 plus the chord 1→3 (so (1,3) is
    // reachable both directly and via 1→2→3, and every node reaches every node).
    const EDGES_TTL: &str = r#"
@prefix ex: <http://ex/> .
<http://ex/n/1> ex:reaches <http://ex/n/2> .
<http://ex/n/2> ex:reaches <http://ex/n/3> .
<http://ex/n/3> ex:reaches <http://ex/n/1> .
<http://ex/n/1> ex:reaches <http://ex/n/3> .
"#;
    let conn = sqlite::load(EDGE_SQL).expect("edge fixture loads");
    let maps = sf_mapping::parse_r2rml(EDGE_R2RML).expect("edge R2RML parses");
    let schema = sqlite::introspect_all(&conn).expect("introspection");
    let graph = parse_turtle(EDGES_TTL, BASE).expect("edges parse");

    for path in ["+", "*"] {
        let q =
            format!("PREFIX ex: <http://ex/> SELECT ?s ?o WHERE {{ ?s ex:reaches{path} ?o }}");
        let plan = engine_plan(&maps, &schema, &q, true);

        // Engine answer over the live SQLite source via the recursive CTE — must
        // terminate (depth bound) despite the cycle.
        let engine = oracle::engine_bag(&exec::select(&plan, &conn).expect("exec"));

        let OracleAnswer::Solutions(oracle_rows) =
            oracle::evaluate(&graph, &q).expect("oracle eval")
        else {
            panic!("expected SELECT solutions from the oracle");
        };

        // Bag equality: each reachable pair exactly once, no depth-induced dups.
        assert!(
            oracle::solutions_bag_eq(&engine, &oracle_rows),
            "engine vs oracle divergence on cyclic reaches{path} (duplicate pairs?):\n engine={engine:#?}\n oracle={oracle_rows:#?}"
        );
    }
}

/// (b4) `P*` over a MULTI-predicate virtual graph is correctly deferred to `501`.
/// The reflexive ZeroLengthPath must bind `(x,x)` for every node of the active
/// graph; with a second predicate (`ex:other`) introducing a node the path
/// predicate never touches, the single-predicate raw-key hop model cannot do this
/// provably — so the engine returns `Unsupported` rather than a wrong answer.
/// `P+` over the same multi-predicate graph stays supported (no reflexive term).
#[test]
fn property_path_star_multi_predicate_defers_501() {
    const EDGE_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Edge>
    rr:logicalTable [ rr:tableName "edge" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{parent}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:reaches ;
        rr:objectMap [ rr:template "http://ex/n/{child}" ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:other ;
        rr:objectMap [ rr:template "http://ex/m/{child}" ]
    ] .
"#;
    let maps = sf_mapping::parse_r2rml(EDGE_R2RML).expect("edge R2RML parses");

    let star = "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:reaches* ?o }";
    let q = SparqlParser::new().parse_query(star).unwrap();
    let r = translate_with(&q, &maps, Dialect::Sqlite, &Tbox::default(), &[]);
    assert!(
        matches!(r, Err(sf_sparql::Error::Unsupported(_))),
        "P* over a multi-predicate graph must defer to 501, got {r:?}"
    );

    // P+ (no reflexive node-set requirement) is still translated.
    let plus = "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:reaches+ ?o }";
    let qp = SparqlParser::new().parse_query(plus).unwrap();
    assert!(
        translate_with(&qp, &maps, Dialect::Sqlite, &Tbox::default(), &[]).is_ok(),
        "P+ over a multi-predicate graph stays supported"
    );
}

/// (c) NoREC zero-divergence (ADR-0012): for several engine-supported queries, the
/// optimized cascade and the raw base translation produce identical bags over the
/// same source. The self-join query exercises a cascade rule with teeth
/// (self-join elimination on the PK), so this is a real differential, not a no-op.
#[test]
fn norec_optimized_equals_unoptimized() {
    let (conn, maps, schema) = fixture();
    let queries = [
        // BGP + JOIN + OPTIONAL + FILTER.
        r#"PREFIX ex: <http://ex/>
           SELECT ?name ?label ?email WHERE {
               ?p ex:name ?name . ?p ex:dept ?d . ?d ex:label ?label .
               OPTIONAL { ?p ex:email ?email } FILTER (?name != "Zed") }"#,
        // Self-join on the PK subject — optimized collapses two person scans to one;
        // the base translation keeps both. Bags must still match.
        r#"PREFIX ex: <http://ex/>
           SELECT ?name ?name2 WHERE { ?p ex:name ?name . ?p ex:name ?name2 }"#,
        // Plain BGP over the dept map.
        r#"PREFIX ex: <http://ex/> SELECT ?label WHERE { ?d ex:label ?label }"#,
        // A bare triple dump as SELECT (every virtual triple).
        r#"SELECT ?s ?p ?o WHERE { ?s ?p ?o }"#,
    ];

    for q in queries {
        let optimized = oracle::engine_bag(&engine_select(&conn, &maps, &schema, q, true));
        let unoptimized = oracle::engine_bag(&engine_select(&conn, &maps, &schema, q, false));
        assert!(
            oracle::solutions_bag_eq(&optimized, &unoptimized),
            "NoREC divergence (a cascade rule is unsound) for `{q}`:\n optimized={optimized:#?}\n unoptimized={unoptimized:#?}"
        );
        assert!(!optimized.is_empty(), "query should return rows: `{q}`");
    }
}

/// (d) End-to-end FK/PK join-elimination differential (ADR-0007 pass 4, ADR-0012).
/// `?p ex:dept ?d` isolates a PK-only-parent join: `dept` is reached ONLY via its
/// PK (`id`) through `person`'s NOT-NULL FK `dept_id`, and no `dept` column other
/// than that PK is projected (the `?d` IRI rebuilds from the equal child FK). So
/// the optimizer drops the `dept` scan entirely. This asserts BOTH halves of
/// soundness the prior structural unit tests could not: (a) the join-eliminated
/// plan is `=_bag`-identical to the unoptimized two-scan plan over a REAL source
/// with schema wired, AND (b) the emitted optimized SQL has strictly fewer
/// joins/scans.
#[test]
fn fk_pk_join_elimination_end_to_end_differential() {
    let (conn, maps, schema) = fixture();
    let q = r#"PREFIX ex: <http://ex/> SELECT ?p ?d WHERE { ?p ex:dept ?d }"#;

    let opt_plan = engine_plan(&maps, &schema, q, true);
    let base_plan = engine_plan(&maps, &schema, q, false);

    // (b) the parent (`dept`) scan/join is gone from the optimized SQL.
    let opt_scans = scan_count(&opt_plan);
    let base_scans = scan_count(&base_plan);
    assert!(
        opt_scans < base_scans,
        "FK/PK elim must reduce joins/scans: optimized={opt_scans} base={base_scans}\n opt_sql={:#?}\n base_sql={:#?}",
        opt_plan.emitted().unwrap().iter().map(|e| e.sql.clone()).collect::<Vec<_>>(),
        base_plan.emitted().unwrap().iter().map(|e| e.sql.clone()).collect::<Vec<_>>(),
    );

    // (a) the two plans agree as bags over the real source.
    let optimized = oracle::engine_bag(&exec::select(&opt_plan, &conn).expect("exec"));
    let unoptimized = oracle::engine_bag(&exec::select(&base_plan, &conn).expect("exec"));
    assert!(
        oracle::solutions_bag_eq(&optimized, &unoptimized),
        "FK/PK elim changed the bag (unsound):\n optimized={optimized:#?}\n unoptimized={unoptimized:#?}"
    );
    // Three persons, all in dept 10 — both arms return the same three rows.
    assert_eq!(optimized.len(), 3, "all three person rows survive");
}
