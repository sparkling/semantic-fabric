//! End-to-end virtualizer tests (ADR-0007 / ADR-0003): a hand-built mapping IR
//! over a live in-memory SQLite source, translated and executed, with the
//! produced triples/bindings asserted exactly.
//!
//! Covers the two priority targets:
//! 1. the **CONSTRUCT dump** `CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }` — the
//!    M2 / W3C-conformance path (unfold all triples-maps → bag union → term-gen);
//! 2. the **general spine** — a multi-pattern SELECT with a cross-table join,
//!    plus OPTIONAL (NULL-safe LEFT JOIN) and a pushed-down FILTER.

use std::collections::BTreeSet;

use rusqlite::Connection;
use sf_core::ir::{
    LogicalSource, ObjectMap, PredicateObjectMap, SubjectMap, Template, TermMap, TermSpec,
    TriplesMap,
};
use sf_core::NamedNode;
use sf_sql::Dialect;
use sf_sparql::{exec, parse_and_translate, translate, Tbox};

const EMP_NAME: &str = "http://ex/empName";
const EMP_DEPT: &str = "http://ex/empDept";
const DEPT_NAME: &str = "http://ex/deptName";
const EMPLOYEE: &str = "http://ex/Employee";
const DEPARTMENT: &str = "http://ex/Department";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

fn iri(s: &str) -> NamedNode {
    NamedNode::new_unchecked(s)
}

fn template_iri(t: &str) -> TermMap {
    TermMap::Template(Template::parse(t).unwrap(), TermSpec::iri())
}

fn column_literal(c: &str) -> TermMap {
    TermMap::Column(c.into(), TermSpec::plain_literal())
}

fn pom(predicate: &str, object: TermMap) -> PredicateObjectMap {
    PredicateObjectMap {
        predicates: vec![TermMap::Constant(iri(predicate).into())],
        objects: vec![ObjectMap::Term(object)],
        graphs: vec![],
    }
}

/// EMP(id,name,dept_id) and DEPT(id,dname) mapped to the `http://ex/` graph.
fn mapping() -> Vec<TriplesMap> {
    let emp = TriplesMap {
        id: "EMP".to_owned(),
        source: LogicalSource::Table("emp".to_owned()),
        subject: SubjectMap {
            term: template_iri("http://ex/emp/{id}"),
            classes: vec![iri(EMPLOYEE)],
            graphs: vec![],
        },
        predicate_object_maps: vec![
            pom(EMP_NAME, column_literal("name")),
            pom(EMP_DEPT, template_iri("http://ex/dept/{dept_id}")),
        ],
    };
    let dept = TriplesMap {
        id: "DEPT".to_owned(),
        source: LogicalSource::Table("dept".to_owned()),
        subject: SubjectMap {
            term: template_iri("http://ex/dept/{id}"),
            classes: vec![iri(DEPARTMENT)],
            graphs: vec![],
        },
        predicate_object_maps: vec![pom(DEPT_NAME, column_literal("dname"))],
    };
    vec![emp, dept]
}

fn source() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE emp(id INTEGER PRIMARY KEY, name TEXT, dept_id INTEGER);
         CREATE TABLE dept(id INTEGER PRIMARY KEY, dname TEXT);
         INSERT INTO emp VALUES (1,'Ada',10),(2,'Grace',20);
         INSERT INTO dept VALUES (10,'R&D'),(20,'Ops');",
    )
    .unwrap();
    conn
}

#[test]
fn construct_dump_unfolds_all_triples_maps() {
    let maps = mapping();
    let conn = source();
    let plan = parse_and_translate(
        "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
        &maps,
        Dialect::Sqlite,
    )
    .unwrap();

    let triples = exec::construct_triples(&plan, &conn).unwrap();
    let got: BTreeSet<String> = triples.iter().map(ToString::to_string).collect();

    let expect: BTreeSet<String> = [
        format!("<http://ex/emp/1> <{RDF_TYPE}> <{EMPLOYEE}>"),
        format!("<http://ex/emp/2> <{RDF_TYPE}> <{EMPLOYEE}>"),
        format!("<http://ex/emp/1> <{EMP_NAME}> \"Ada\""),
        format!("<http://ex/emp/2> <{EMP_NAME}> \"Grace\""),
        format!("<http://ex/emp/1> <{EMP_DEPT}> <http://ex/dept/10>"),
        format!("<http://ex/emp/2> <{EMP_DEPT}> <http://ex/dept/20>"),
        format!("<http://ex/dept/10> <{RDF_TYPE}> <{DEPARTMENT}>"),
        format!("<http://ex/dept/20> <{RDF_TYPE}> <{DEPARTMENT}>"),
        format!("<http://ex/dept/10> <{DEPT_NAME}> \"R&D\""),
        format!("<http://ex/dept/20> <{DEPT_NAME}> \"Ops\""),
    ]
    .into_iter()
    .collect();

    assert_eq!(got, expect, "dump must be the exact UNION of all per-map triples");
    // N-Triples serialisation round-trips line count (ADR-0019 G1).
    assert_eq!(exec::write_ntriples(&triples).lines().count(), 10);
}

#[test]
fn select_bgp_with_cross_table_join() {
    let maps = mapping();
    let conn = source();
    // ?e :empName ?n . ?e :empDept ?d . ?d :deptName ?dn
    // — a self-join on EMP (?e) folded into a cross-table join EMP⋈DEPT (?d).
    let q = format!(
        "SELECT ?n ?dn WHERE {{ ?e <{EMP_NAME}> ?n . ?e <{EMP_DEPT}> ?d . ?d <{DEPT_NAME}> ?dn }}"
    );
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let sol = exec::select(&plan, &conn).unwrap();

    assert_eq!(sol.vars, vec!["n".to_owned(), "dn".to_owned()]);
    let got: BTreeSet<(String, String)> = sol
        .rows
        .iter()
        .map(|r| (lit(&r[0]), lit(&r[1])))
        .collect();
    let expect: BTreeSet<(String, String)> = [
        ("Ada".to_owned(), "R&D".to_owned()),
        ("Grace".to_owned(), "Ops".to_owned()),
    ]
    .into_iter()
    .collect();
    assert_eq!(got, expect);
}

#[test]
fn select_with_pushed_down_filter() {
    let maps = mapping();
    let conn = source();
    let q = format!("SELECT ?n WHERE {{ ?e <{EMP_NAME}> ?n . FILTER(?n = \"Ada\") }}");
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    // The constant becomes a bound parameter (ADR-0010 R1), never inlined.
    let sql = &plan.emitted().unwrap()[0].sql;
    assert!(sql.contains('?'), "filter constant must be a bound placeholder: {sql}");
    assert!(!sql.contains("Ada"), "value must not be inlined: {sql}");

    let sol = exec::select(&plan, &conn).unwrap();
    let got: BTreeSet<String> = sol.rows.iter().map(|r| lit(&r[0])).collect();
    assert_eq!(got, BTreeSet::from(["Ada".to_owned()]));
}

#[test]
fn optional_is_null_safe_left_join() {
    // An employee with no department row still appears; ?dn is unbound (R3).
    let maps = mapping();
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE emp(id INTEGER PRIMARY KEY, name TEXT, dept_id INTEGER);
         CREATE TABLE dept(id INTEGER PRIMARY KEY, dname TEXT);
         INSERT INTO emp VALUES (1,'Ada',10),(3,'Lone',99);
         INSERT INTO dept VALUES (10,'R&D');",
    )
    .unwrap();

    let q = format!(
        "SELECT ?n ?dn WHERE {{ ?e <{EMP_NAME}> ?n . ?e <{EMP_DEPT}> ?d \
         OPTIONAL {{ ?d <{DEPT_NAME}> ?dn }} }}"
    );
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    // The OPTIONAL must use the NULL-safe compatibility form, never a plain a = b.
    let sql = &plan.emitted().unwrap()[0].sql;
    assert!(sql.to_uppercase().contains("LEFT JOIN"), "{sql}");
    assert!(sql.contains("IS NULL"), "R1 null-safe compatibility: {sql}");

    let sol = exec::select(&plan, &conn).unwrap();
    let got: BTreeSet<(String, Option<String>)> = sol
        .rows
        .iter()
        .map(|r| (lit(&r[0]), r[1].as_ref().map(|_| lit(&r[1]))))
        .collect();
    let expect: BTreeSet<(String, Option<String>)> = [
        ("Ada".to_owned(), Some("R&D".to_owned())),
        ("Lone".to_owned(), None), // dept 99 absent → ?dn unbound, row preserved
    ]
    .into_iter()
    .collect();
    assert_eq!(got, expect);
}

#[test]
fn nested_optional_shared_var_coalesces_to_the_matching_side() {
    // R2 (ADR-0007): `?s :p ?v . OPTIONAL { ?s :a ?x } OPTIONAL { ?s :b ?x }`
    // parses left-deep to LeftJoin(LeftJoin(BGP, A), B). ?x is shared in the OUTER
    // join with a NULLABLE left representation (A's column, behind the inner
    // OPTIONAL). When A has no value but B does, ?x must be B's value —
    // COALESCE(A.col, B.col) — not UNBOUND. Without the COALESCE the answer is
    // wrong (?x reported unbound).
    const P: &str = "http://ex/p";
    const A: &str = "http://ex/a";
    const B: &str = "http://ex/b";
    let node = TriplesMap {
        id: "NODE".to_owned(),
        source: LogicalSource::Table("node".to_owned()),
        subject: SubjectMap {
            term: template_iri("http://ex/n/{id}"),
            classes: vec![],
            graphs: vec![],
        },
        predicate_object_maps: vec![
            pom(P, column_literal("p_val")),
            pom(A, column_literal("a_val")),
            pom(B, column_literal("b_val")),
        ],
    };
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE node(id INTEGER PRIMARY KEY, p_val TEXT, a_val TEXT, b_val TEXT);
         INSERT INTO node VALUES (1,'P1',NULL,'B1');",
    )
    .unwrap();

    let q = format!(
        "SELECT ?x WHERE {{ ?s <{P}> ?v . OPTIONAL {{ ?s <{A}> ?x }} OPTIONAL {{ ?s <{B}> ?x }} }}"
    );
    let plan = parse_and_translate(&q, std::slice::from_ref(&node), Dialect::Sqlite).unwrap();
    let sol = exec::select(&plan, &conn).unwrap();

    let got: Vec<Option<String>> = sol
        .rows
        .iter()
        .map(|r| r[0].as_ref().map(|_| lit(&r[0])))
        .collect();
    assert_eq!(
        got,
        vec![Some("B1".to_owned())],
        "shared OPTIONAL ?x must COALESCE to B's value when A is unmatched"
    );
}

#[test]
fn tier1_subclass_saturation_matches_subclasses() {
    // A query for :Employee also matches a subject mapped only to :Manager ⊑ :Employee.
    let mut maps = mapping();
    // Re-tag EMP's class as :Manager (a subclass of :Employee in the T-Box).
    maps[0].subject.classes = vec![iri("http://ex/Manager")];
    let conn = source();

    let mut tbox = Tbox::new();
    tbox.add_subclass("http://ex/Manager", EMPLOYEE);

    let q = parse(&format!("SELECT ?e WHERE {{ ?e <{RDF_TYPE}> <{EMPLOYEE}> }}"));
    let plan = sf_sparql::translate_with(&q, &maps, Dialect::Sqlite, &tbox, &[]).unwrap();
    let sol = exec::select(&plan, &conn).unwrap();
    assert_eq!(sol.rows.len(), 2, "both managers match the superclass query");

    // Without the T-Box, the superclass query returns nothing.
    let plan0 = translate(&q, &maps, Dialect::Sqlite).unwrap();
    assert_eq!(exec::select(&plan0, &conn).unwrap().rows.len(), 0);
}

#[test]
fn construct_applies_r2rml_section10_natural_datatypes() {
    // A column-valued literal with no explicit rr:datatype takes its XSD type from
    // the source column's declared type (ADR-0015 §10): INTEGER → xsd:integer,
    // REAL → xsd:double (canonical E-notation), TEXT → plain literal. (The
    // computed-column storage-class fallback is covered by W3C D009d in the
    // conformance suite.)
    let tm = TriplesMap {
        id: "T".to_owned(),
        source: LogicalSource::Table("m".to_owned()),
        subject: SubjectMap {
            term: template_iri("http://ex/m/{id}"),
            classes: vec![],
            graphs: vec![],
        },
        predicate_object_maps: vec![
            pom("http://ex/age", column_literal("age")),
            pom("http://ex/score", column_literal("score")),
            pom("http://ex/name", column_literal("name")),
        ],
    };
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE m(id INTEGER PRIMARY KEY, age INTEGER, score REAL, name TEXT);
         INSERT INTO m VALUES (1, 42, 80.25, 'Ada');",
    )
    .unwrap();

    let plan = parse_and_translate(
        "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }",
        std::slice::from_ref(&tm),
        Dialect::Sqlite,
    )
    .unwrap();
    let got: BTreeSet<String> = exec::construct_triples(&plan, &conn)
        .unwrap()
        .iter()
        .map(ToString::to_string)
        .collect();

    let xsd = "http://www.w3.org/2001/XMLSchema#";
    assert!(got.contains(&format!("<http://ex/m/1> <http://ex/age> \"42\"^^<{xsd}integer>")), "{got:?}");
    assert!(got.contains(&format!("<http://ex/m/1> <http://ex/score> \"8.025E1\"^^<{xsd}double>")), "{got:?}");
    assert!(got.contains("<http://ex/m/1> <http://ex/name> \"Ada\""), "plain literal: {got:?}");
}

#[test]
fn select_with_contains_like_pushdown() {
    // ADR-0020 §2 near-free FTS: FILTER(CONTAINS(?n, "ra")) → `name LIKE ? ESCAPE`,
    // the pattern a bound parameter (never inlined). SPARQL CONTAINS is
    // case-SENSITIVE, and only PostgreSQL's LIKE is genuinely case-sensitive, so
    // the pushdown fires on PostgreSQL.
    let maps = mapping();
    let q = format!("SELECT ?n WHERE {{ ?e <{EMP_NAME}> ?n . FILTER(CONTAINS(?n, \"ra\")) }}");
    let plan = parse_and_translate(&q, &maps, Dialect::Postgres).unwrap();

    let emitted = plan.emitted().unwrap();
    let sql = &emitted[0].sql;
    assert!(sql.to_uppercase().contains("LIKE"), "substring pushdown: {sql}");
    assert!(sql.contains("$1"), "pattern must be a bound placeholder: {sql}");
    assert!(!sql.contains("%ra%"), "pattern must not be inlined: {sql}");
    assert_eq!(emitted[0].params, vec!["%ra%".to_owned()]);

    // On SQLite, LIKE is ASCII-case-INSENSITIVE by default, so it would match more
    // rows than the case-sensitive SPARQL CONTAINS (an unsound =_bag / NoREC
    // divergence). The FILTER is therefore left un-rewritten — Unsupported, never a
    // wrong case-folding LIKE (correctness over coverage).
    assert!(
        parse_and_translate(&q, &maps, Dialect::Sqlite).is_err(),
        "case-insensitive SQLite LIKE must not back a case-sensitive CONTAINS"
    );
}

const REACHES: &str = "http://ex/reaches";

/// `edge(parent, child)` mapped so `?p :reaches ?c` is the one-hop relation, with
/// both endpoints the same `http://ex/n/{…}` IRI domain — the v1 simple-predicate
/// transitive-path shape (ADR-0007 / ADR-0008).
fn edge_mapping() -> Vec<TriplesMap> {
    vec![TriplesMap {
        id: "EDGE".to_owned(),
        source: LogicalSource::Table("edge".to_owned()),
        subject: SubjectMap {
            term: template_iri("http://ex/n/{parent}"),
            classes: vec![],
            graphs: vec![],
        },
        predicate_object_maps: vec![pom(REACHES, template_iri("http://ex/n/{child}"))],
    }]
}

fn edge_source() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    // Chain 1→2→3→4 plus a side branch 1→5.
    conn.execute_batch(
        "CREATE TABLE edge(parent INTEGER, child INTEGER);
         INSERT INTO edge VALUES (1,2),(2,3),(3,4),(1,5);",
    )
    .unwrap();
    conn
}

fn n(i: u32) -> String {
    format!("<http://ex/n/{i}>")
}

#[test]
fn property_path_oneormore_is_transitive_closure() {
    // ?s :reaches+ ?o = the transitive closure (one or more hops), set-based.
    let maps = edge_mapping();
    let conn = edge_source();
    let q = format!("SELECT ?s ?o WHERE {{ ?s <{REACHES}>+ ?o }}");
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();

    // The closure compiles to a depth-bounded recursive CTE (ADR-0007 / ADR-0010).
    let sql = &plan.emitted().unwrap()[0].sql;
    let up = sql.to_uppercase();
    assert!(up.contains("WITH RECURSIVE"), "recursive CTE: {sql}");
    assert!(up.contains("UNION") && !up.contains("UNION ALL"), "set-union closure: {sql}");
    assert!(sql.contains("256"), "ADR-0010 depth bound present: {sql}");

    let sol = exec::select(&plan, &conn).unwrap();
    let got: BTreeSet<(String, String)> =
        sol.rows.iter().map(|r| (lit(&r[0]), lit(&r[1]))).collect();
    let expect: BTreeSet<(String, String)> = [
        (n(1), n(2)), (n(1), n(3)), (n(1), n(4)), (n(1), n(5)),
        (n(2), n(3)), (n(2), n(4)),
        (n(3), n(4)),
    ]
    .into_iter()
    .collect();
    assert_eq!(got, expect, "P+ must be exactly the transitive closure");
}

#[test]
fn property_path_zeroormore_adds_reflexive_pairs() {
    // ?s :reaches* ?o = the closure PLUS the reflexive (x,x) pairs over every node
    // appearing as a subject or object of the hop relation (nodes 1..5).
    let maps = edge_mapping();
    let conn = edge_source();
    let q = format!("SELECT ?s ?o WHERE {{ ?s <{REACHES}>* ?o }}");
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    assert!(
        plan.emitted().unwrap()[0].sql.to_uppercase().contains("WITH RECURSIVE"),
        "P* is also a recursive CTE"
    );

    let sol = exec::select(&plan, &conn).unwrap();
    let got: BTreeSet<(String, String)> =
        sol.rows.iter().map(|r| (lit(&r[0]), lit(&r[1]))).collect();
    let mut expect: BTreeSet<(String, String)> = [
        (n(1), n(2)), (n(1), n(3)), (n(1), n(4)), (n(1), n(5)),
        (n(2), n(3)), (n(2), n(4)),
        (n(3), n(4)),
    ]
    .into_iter()
    .collect();
    for i in 1..=5 {
        expect.insert((n(i), n(i))); // reflexive: P* but not P+
    }
    assert_eq!(got, expect, "P* = transitive closure ∪ reflexive node pairs");
}

fn parse(q: &str) -> spargebra::Query {
    spargebra::SparqlParser::new().parse_query(q).unwrap()
}

/// Extract a simple-literal lexical value from a reconstructed binding.
fn lit(t: &Option<sf_core::Term>) -> String {
    match t {
        Some(sf_core::Term::Literal(l)) => l.value().to_owned(),
        Some(other) => other.to_string(),
        None => "<unbound>".to_owned(),
    }
}
