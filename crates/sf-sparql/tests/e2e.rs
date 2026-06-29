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
use sf_sparql::{
    exec, parse_and_translate, translate, translate_unoptimized, translate_with, Tbox,
};
use sf_sql::{Column, Dialect, TableSchema};

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

    assert_eq!(
        got, expect,
        "dump must be the exact UNION of all per-map triples"
    );
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
    let got: BTreeSet<(String, String)> =
        sol.rows.iter().map(|r| (lit(&r[0]), lit(&r[1]))).collect();
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
    assert!(
        sql.contains('?'),
        "filter constant must be a bound placeholder: {sql}"
    );
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
fn optional_self_left_join_elimination_q5_bag_oracle() {
    // Q5 (ADR-0022 / Ontop LeftJoinOptimizationTest.testLeftJoinElimination1):
    // `?t a :Trip . OPTIONAL { ?t :headsign ?hs }` maps both patterns to the SAME
    // `trips` table on the NOT-NULL PK `trip_id`, so the OPTIONAL reads the same row
    // — a redundant self-LEFT-join. The decisive =_bag gate: the optimized engine
    // output (cascade self_left_join_elimination ON) must be IDENTICAL to the
    // unoptimized base translation, every trip present and ?hs bound or unbound
    // exactly as the data dictates (some trips have a NULL trip_headsign).
    const TRIP: &str = "http://ex/Trip";
    const HEADSIGN: &str = "http://ex/headsign";
    let trips = TriplesMap {
        id: "TRIPS".to_owned(),
        source: LogicalSource::Table("trips".to_owned()),
        subject: SubjectMap {
            term: template_iri("http://ex/trip/{trip_id}"),
            classes: vec![iri(TRIP)],
            graphs: vec![],
        },
        predicate_object_maps: vec![pom(HEADSIGN, column_literal("trip_headsign"))],
    };
    let maps = std::slice::from_ref(&trips);
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE trips(trip_id TEXT PRIMARY KEY, trip_headsign TEXT);
         INSERT INTO trips VALUES
           ('T1','Downtown'),('T2',NULL),('T3','Airport'),('T4',NULL);",
    )
    .unwrap();

    // The introspected catalog: trip_id is a NOT-NULL PK (what licenses the
    // collapse), trip_headsign nullable. Passing it is what makes the cascade fire.
    let mut ts = TableSchema::new("trips");
    ts.primary_key = vec!["trip_id".to_owned()];
    ts.columns = vec![
        Column::new("trip_id", "text", true),
        Column::new("trip_headsign", "text", false),
    ];
    let schema = std::slice::from_ref(&ts);

    let query = parse(&format!(
        "SELECT ?t ?hs WHERE {{ ?t a <{TRIP}> OPTIONAL {{ ?t <{HEADSIGN}> ?hs }} }}"
    ));

    // Optimized: the self-LEFT-join must collapse (no surviving LEFT JOIN).
    let opt = translate_with(&query, maps, Dialect::Sqlite, &Tbox::default(), schema).unwrap();
    let opt_sql = &opt.emitted().unwrap()[0].sql;
    assert!(
        !opt_sql.to_uppercase().contains("LEFT JOIN"),
        "self-LEFT-join on the trips PK must be eliminated: {opt_sql}"
    );

    // Unoptimized oracle: the raw base translation keeps the LEFT JOIN.
    let base =
        translate_unoptimized(&query, maps, Dialect::Sqlite, &Tbox::default(), schema).unwrap();
    assert!(
        base.emitted().unwrap()[0]
            .sql
            .to_uppercase()
            .contains("LEFT JOIN"),
        "the unoptimized arm retains the self-LEFT-join (the oracle)"
    );

    let bag = |plan: &sf_sparql::Plan| -> Vec<(Option<String>, Option<String>)> {
        let sol = exec::select(plan, &conn).unwrap();
        assert_eq!(sol.vars, vec!["t".to_owned(), "hs".to_owned()]);
        let mut rows: Vec<(Option<String>, Option<String>)> = sol
            .rows
            .iter()
            .map(|r| {
                (
                    r[0].as_ref().map(|_| lit(&r[0])),
                    r[1].as_ref().map(|_| lit(&r[1])),
                )
            })
            .collect();
        rows.sort();
        rows
    };

    let got = bag(&opt);
    // Decisive =_bag gate: optimized output is identical to the unoptimized base.
    assert_eq!(
        got,
        bag(&base),
        "self-LEFT-join elimination must preserve =_bag vs the base translation"
    );
    // ...and equals the hand-computed oracle (every trip; ?hs bound iff non-NULL).
    let mut expect: Vec<(Option<String>, Option<String>)> = vec![
        (
            Some("<http://ex/trip/T1>".to_owned()),
            Some("Downtown".to_owned()),
        ),
        (Some("<http://ex/trip/T2>".to_owned()), None),
        (
            Some("<http://ex/trip/T3>".to_owned()),
            Some("Airport".to_owned()),
        ),
        (Some("<http://ex/trip/T4>".to_owned()), None),
    ];
    expect.sort();
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

    let q = parse(&format!(
        "SELECT ?e WHERE {{ ?e <{RDF_TYPE}> <{EMPLOYEE}> }}"
    ));
    let plan = sf_sparql::translate_with(&q, &maps, Dialect::Sqlite, &tbox, &[]).unwrap();
    let sol = exec::select(&plan, &conn).unwrap();
    assert_eq!(
        sol.rows.len(),
        2,
        "both managers match the superclass query"
    );

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
    assert!(
        got.contains(&format!(
            "<http://ex/m/1> <http://ex/age> \"42\"^^<{xsd}integer>"
        )),
        "{got:?}"
    );
    assert!(
        got.contains(&format!(
            "<http://ex/m/1> <http://ex/score> \"8.025E1\"^^<{xsd}double>"
        )),
        "{got:?}"
    );
    assert!(
        got.contains("<http://ex/m/1> <http://ex/name> \"Ada\""),
        "plain literal: {got:?}"
    );
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
    assert!(
        sql.to_uppercase().contains("LIKE"),
        "substring pushdown: {sql}"
    );
    assert!(
        sql.contains("$1"),
        "pattern must be a bound placeholder: {sql}"
    );
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

#[test]
fn select_with_bind_concat_computes_and_projects() {
    // BIND(CONCAT(?n, "!") AS ?greeting): an Extend adds one output column computed
    // over a row's existing binding, WITHOUT changing row multiplicity. =_bag gate:
    // exactly the two empName rows, each with its computed greeting (hand oracle).
    let maps = mapping();
    let conn = source();
    let q = format!(
        "SELECT ?n ?greeting WHERE {{ ?e <{EMP_NAME}> ?n . \
         BIND(CONCAT(?n, \"!\") AS ?greeting) }}"
    );
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let sol = exec::select(&plan, &conn).unwrap();
    assert_eq!(sol.vars, vec!["n".to_owned(), "greeting".to_owned()]);
    let got: BTreeSet<(String, String)> =
        sol.rows.iter().map(|r| (lit(&r[0]), lit(&r[1]))).collect();
    let expect: BTreeSet<(String, String)> = [
        ("Ada".to_owned(), "Ada!".to_owned()),
        ("Grace".to_owned(), "Grace!".to_owned()),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        got, expect,
        "BIND adds a computed column, multiplicity unchanged"
    );
}

#[test]
fn select_with_values_inline_table_joined_to_bgp() {
    // VALUES (?n ?tag) { ("Ada" "lead") ("Grace" UNDEF) } joined to ?e :empName ?n.
    // Each VALUES row is one core-less branch (a bag union); the shared ?n unifies
    // through join_branches. =_bag gate: the UNDEF cell leaves ?tag UNBOUND for the
    // Grace row, while Ada carries its tag — a normal bag-join, one row per pairing.
    let maps = mapping();
    let conn = source();
    let q = format!(
        "SELECT ?n ?tag WHERE {{ ?e <{EMP_NAME}> ?n . \
         VALUES (?n ?tag) {{ (\"Ada\" \"lead\") (\"Grace\" UNDEF) }} }}"
    );
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let sol = exec::select(&plan, &conn).unwrap();
    assert_eq!(sol.vars, vec!["n".to_owned(), "tag".to_owned()]);
    let got: BTreeSet<(String, Option<String>)> = sol
        .rows
        .iter()
        .map(|r| (lit(&r[0]), r[1].as_ref().map(|_| lit(&r[1]))))
        .collect();
    let expect: BTreeSet<(String, Option<String>)> = [
        ("Ada".to_owned(), Some("lead".to_owned())),
        ("Grace".to_owned(), None), // UNDEF cell ⇒ ?tag unbound
    ]
    .into_iter()
    .collect();
    assert_eq!(got, expect, "VALUES row joins the BGP; UNDEF stays unbound");
}

#[test]
fn standalone_values_emits_core_less_constant_rows() {
    // A standalone VALUES is a bag union of core-less constant branches, each
    // emitting a one-row `SELECT <const>` with NO FROM. =_bag gate: exactly the two
    // inline rows.
    let maps = mapping();
    let conn = source();
    let q = "SELECT ?x WHERE { VALUES (?x) { (1) (2) } }";
    let plan = parse_and_translate(q, &maps, Dialect::Sqlite).unwrap();
    for e in plan.emitted().unwrap() {
        assert!(
            !e.sql.to_uppercase().contains(" FROM "),
            "core-less constant branch renders without FROM: {}",
            e.sql
        );
    }
    let sol = exec::select(&plan, &conn).unwrap();
    let got: BTreeSet<String> = sol.rows.iter().map(|r| lit(&r[0])).collect();
    assert_eq!(got, BTreeSet::from(["1".to_owned(), "2".to_owned()]));
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
    assert!(
        up.contains("UNION") && !up.contains("UNION ALL"),
        "set-union closure: {sql}"
    );
    assert!(sql.contains("256"), "ADR-0010 depth bound present: {sql}");

    let sol = exec::select(&plan, &conn).unwrap();
    let got: BTreeSet<(String, String)> =
        sol.rows.iter().map(|r| (lit(&r[0]), lit(&r[1]))).collect();
    let expect: BTreeSet<(String, String)> = [
        (n(1), n(2)),
        (n(1), n(3)),
        (n(1), n(4)),
        (n(1), n(5)),
        (n(2), n(3)),
        (n(2), n(4)),
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
        plan.emitted().unwrap()[0]
            .sql
            .to_uppercase()
            .contains("WITH RECURSIVE"),
        "P* is also a recursive CTE"
    );

    let sol = exec::select(&plan, &conn).unwrap();
    let got: BTreeSet<(String, String)> =
        sol.rows.iter().map(|r| (lit(&r[0]), lit(&r[1]))).collect();
    let mut expect: BTreeSet<(String, String)> = [
        (n(1), n(2)),
        (n(1), n(3)),
        (n(1), n(4)),
        (n(1), n(5)),
        (n(2), n(3)),
        (n(2), n(4)),
        (n(3), n(4)),
    ]
    .into_iter()
    .collect();
    for i in 1..=5 {
        expect.insert((n(i), n(i))); // reflexive: P* but not P+
    }
    assert_eq!(
        got, expect,
        "P* = transitive closure ∪ reflexive node pairs"
    );
}

#[test]
fn minus_removes_left_solutions_matching_on_a_shared_var() {
    // ?e :empName ?n MINUS { ?e :empName "Ada" } — the right binds ?e to the Ada
    // employee; MINUS removes the left solution sharing that ?e, keeping Grace.
    let maps = mapping();
    let conn = source();
    let q = format!("SELECT ?n WHERE {{ ?e <{EMP_NAME}> ?n MINUS {{ ?e <{EMP_NAME}> \"Ada\" }} }}");
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let got: Vec<String> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| lit(&r[0]))
        .collect();
    assert_eq!(
        got,
        vec!["Grace"],
        "MINUS removes the left solution sharing ?e=Ada"
    );
}

#[test]
fn minus_disjoint_domains_is_a_noop() {
    // {?e :empName ?n} MINUS {?d :deptName ?dn} — NO shared variable, so MINUS is a
    // NO-OP (SPARQL §8.3): every left solution survives (it is NOT emptied). This is
    // the canonical MINUS-vs-NOT-EXISTS distinction.
    let maps = mapping();
    let conn = source();
    let q = format!("SELECT ?n WHERE {{ ?e <{EMP_NAME}> ?n MINUS {{ ?d <{DEPT_NAME}> ?dn }} }}");
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let mut got: Vec<String> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| lit(&r[0]))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec!["Ada", "Grace"],
        "disjoint-domain MINUS returns the left unchanged"
    );
}

#[test]
fn minus_respects_literal_language_not_just_lexical() {
    // Regression (WS-B wave-3 adversarial find): MINUS compatibility is `sameTerm`,
    // not raw lexical equality — "Ada"@en is NOT compatible with "Ada"@fr, so the
    // left solution must be KEPT. (Before the unify datatype/language fix this
    // wrongly equated the columns and returned [].)
    let t = TriplesMap {
        id: "T".to_owned(),
        source: LogicalSource::Table("t".to_owned()),
        subject: SubjectMap {
            term: template_iri("http://ex/t/{id}"),
            classes: vec![],
            graphs: vec![],
        },
        predicate_object_maps: vec![
            pom(
                "http://ex/enLabel",
                TermMap::Column("v".into(), TermSpec::lang_literal("en")),
            ),
            pom(
                "http://ex/frLabel",
                TermMap::Column("v".into(), TermSpec::lang_literal("fr")),
            ),
        ],
    };
    let maps = std::slice::from_ref(&t);
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT); INSERT INTO t VALUES (1,'Ada');",
    )
    .unwrap();
    let q = "SELECT ?x WHERE { ?a <http://ex/enLabel> ?x MINUS { ?b <http://ex/frLabel> ?x } }";
    let plan = parse_and_translate(q, maps, Dialect::Sqlite).unwrap();
    let got: Vec<String> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| lit(&r[0]))
        .collect();
    assert_eq!(
        got,
        vec!["Ada"],
        "\"Ada\"@en is not sameTerm \"Ada\"@fr ⇒ the left solution is kept"
    );
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

#[test]
fn values_undef_acts_as_join_wildcard() {
    // VALUES (?n) { ("Ada") (UNDEF) } joined to ?e :empName ?n. The UNDEF row is a
    // wildcard (compatible with any ?n) so it contributes every BGP solution; the
    // "Ada" row contributes only Ada. Bag = [Ada (explicit + wildcard), Grace].
    let maps = mapping();
    let conn = source();
    let q =
        format!("SELECT ?n WHERE {{ ?e <{EMP_NAME}> ?n . VALUES (?n) {{ (\"Ada\") (UNDEF) }} }}");
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let mut got: Vec<String> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| lit(&r[0]))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec!["Ada", "Ada", "Grace"],
        "UNDEF row acts as a wildcard"
    );
}

#[test]
fn values_filter_on_const_var_is_501() {
    // FILTER over a variable bound only to a VALUES constant (no source column to
    // push the predicate onto) is an honest 501 — never a silently-wrong answer.
    let maps = mapping();
    let q = "SELECT ?x WHERE { VALUES (?x) { (1) (2) } FILTER(?x = 1) }";
    assert!(
        parse_and_translate(q, &maps, Dialect::Sqlite).is_err(),
        "FILTER on a VALUES-const var is deferred to 501, not silently wrong"
    );
}

#[test]
fn distinct_over_values_dedups_across_branches() {
    // Regression: SELECT DISTINCT over a multi-branch bag union (each VALUES row is a
    // separate core-less branch) must dedup ACROSS branches — per-branch SQL can't.
    // Before the exec::for_each_solution dedup this returned [1, 1].
    let maps = mapping();
    let conn = source();
    let q = "SELECT DISTINCT ?x WHERE { VALUES (?x) { (1) (1) } }";
    let plan = parse_and_translate(q, &maps, Dialect::Sqlite).unwrap();
    let got: Vec<String> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| lit(&r[0]))
        .collect();
    assert_eq!(
        got,
        vec!["1"],
        "DISTINCT collapses the duplicate VALUES rows"
    );
}

#[test]
fn distinct_over_union_dedups_across_branches() {
    // The same multi-branch DISTINCT path on UNION (the pre-existing bug VALUES
    // exposed): `{A} UNION {A}` yields each name twice; DISTINCT must collapse them.
    let maps = mapping();
    let conn = source();
    let q = format!(
        "SELECT DISTINCT ?n WHERE {{ {{ ?e <{EMP_NAME}> ?n }} UNION {{ ?e <{EMP_NAME}> ?n }} }}"
    );
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let mut got: Vec<String> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| lit(&r[0]))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec!["Ada", "Grace"],
        "DISTINCT collapses cross-branch duplicates"
    );
}

#[test]
fn order_by_single_branch_sorts_in_exec_not_sql() {
    // ORDER BY ?n over a single-branch SELECT is applied in exec via the type-aware
    // order_cmp, NOT pushed into SQL (a SQL ORDER BY would inherit the column's
    // collation/affinity — see order_by_respects_codepoint_order_over_sql_collation).
    // =order gate: the produced rows are the exact ordered vector (not a set).
    let maps = mapping();
    let conn = source();

    let q_asc = format!("SELECT ?n WHERE {{ ?e <{EMP_NAME}> ?n }} ORDER BY ?n");
    let plan = parse_and_translate(&q_asc, &maps, Dialect::Sqlite).unwrap();
    assert!(
        !plan.emitted().unwrap()[0]
            .sql
            .to_uppercase()
            .contains("ORDER BY"),
        "ORDER BY is applied in exec (collation-independent), never pushed into SQL"
    );
    let asc: Vec<String> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| lit(&r[0]))
        .collect();
    assert_eq!(asc, vec!["Ada", "Grace"], "ASC order is exact");

    let q_desc = format!("SELECT ?n WHERE {{ ?e <{EMP_NAME}> ?n }} ORDER BY DESC(?n)");
    let plan = parse_and_translate(&q_desc, &maps, Dialect::Sqlite).unwrap();
    let desc: Vec<String> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| lit(&r[0]))
        .collect();
    assert_eq!(desc, vec!["Grace", "Ada"], "DESC order is exact");
}

#[test]
fn order_by_respects_codepoint_order_over_sql_collation() {
    // Regression (WS-B wave-2 adversarial find): a SQL `ORDER BY` on a NOCASE text
    // column sorts case-insensitively, disagreeing with SPARQL's xsd:string order
    // (Unicode codepoint: 'Z' U+005A < 'a' U+0061). Because ORDER BY is applied in
    // exec via order_cmp (codepoint), the result is correct regardless of the
    // column's declared collation. (The old single-branch SQL push returned the
    // wrong order ["apple","Zoo"] here.)
    let t = TriplesMap {
        id: "T".to_owned(),
        source: LogicalSource::Table("t".to_owned()),
        subject: SubjectMap {
            term: template_iri("http://ex/t/{id}"),
            classes: vec![],
            graphs: vec![],
        },
        predicate_object_maps: vec![pom(EMP_NAME, column_literal("name"))],
    };
    let maps = std::slice::from_ref(&t);
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE t(id INTEGER PRIMARY KEY, name TEXT COLLATE NOCASE);
         INSERT INTO t VALUES (1,'Zoo'),(2,'apple');",
    )
    .unwrap();
    let q = format!("SELECT ?n WHERE {{ ?e <{EMP_NAME}> ?n }} ORDER BY ?n");
    let plan = parse_and_translate(&q, maps, Dialect::Sqlite).unwrap();
    let got: Vec<String> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| lit(&r[0]))
        .collect();
    assert_eq!(
        got,
        vec!["Zoo", "apple"],
        "codepoint order (uppercase < lowercase), not the NOCASE column collation"
    );
}

#[test]
fn order_by_asc_sorts_unbound_optional_first() {
    // ASC pins UNBOUND (the OPTIONAL no-match) FIRST (SPARQL §15.1 + NULLS FIRST).
    // Lone has no department ⇒ ?dn unbound ⇒ sorts before the bound R&D.
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
         OPTIONAL {{ ?d <{DEPT_NAME}> ?dn }} }} ORDER BY ?dn"
    );
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let rows: Vec<(String, Option<String>)> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| (lit(&r[0]), r[1].as_ref().map(|_| lit(&r[1]))))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("Lone".to_owned(), None), // unbound ?dn sorts FIRST (ASC)
            ("Ada".to_owned(), Some("R&D".to_owned())),
        ],
        "ASC puts the unbound OPTIONAL key first"
    );
}

#[test]
fn order_by_over_union_sorts_globally_across_branches() {
    // ORDER BY over a UNION is a multi-branch bag-union: per-branch SQL cannot give
    // a global order, so exec buffers + stable-sorts ACROSS branches. The two arms
    // bind ?n from emp/dept names; the merged result must be globally ordered.
    let maps = mapping();
    let conn = source();
    let q = format!(
        "SELECT ?n WHERE {{ {{ ?e <{EMP_NAME}> ?n }} UNION {{ ?d <{DEPT_NAME}> ?n }} }} \
         ORDER BY ?n"
    );
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    assert!(plan.branches.len() > 1, "UNION is a multi-branch plan");
    let got: Vec<String> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| lit(&r[0]))
        .collect();
    // Global codepoint order across BOTH arms: Ada, Grace, Ops, R&D.
    assert_eq!(
        got,
        vec!["Ada", "Grace", "Ops", "R&D"],
        "the bag-union is sorted globally, not per-branch"
    );
}

#[test]
fn order_by_then_limit_is_global_top_n() {
    // ORDER BY ?n DESC LIMIT 2 over the UNION: the slice applies AFTER the global
    // sort (SPARQL §15: order, then LIMIT), so it is the true top-2, not a per-branch
    // head. Descending global order: R&D, Ops, Grace, Ada → top-2 = [R&D, Ops].
    let maps = mapping();
    let conn = source();
    let q = format!(
        "SELECT ?n WHERE {{ {{ ?e <{EMP_NAME}> ?n }} UNION {{ ?d <{DEPT_NAME}> ?n }} }} \
         ORDER BY DESC(?n) LIMIT 2"
    );
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let got: Vec<String> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| lit(&r[0]))
        .collect();
    assert_eq!(got, vec!["R&D", "Ops"], "top-2 after the global DESC sort");
}

#[test]
fn order_by_expression_strlen_sorts_correctly() {
    // ORDER BY over a non-variable expression (STRLEN(?n) + 1) — now supported via
    // the exec-layer expression evaluator. "Ada" (len=3) < "Grace" (len=5) ascending.
    let maps = mapping();
    let conn = source();
    let q = format!("SELECT ?n WHERE {{ ?e <{EMP_NAME}> ?n }} ORDER BY (STRLEN(?n) + 1)");
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite)
        .expect("ORDER BY expression should translate successfully");
    let sol = exec::select(&plan, &conn).unwrap();
    let names: Vec<_> = sol
        .rows
        .iter()
        .map(|r| r[0].as_ref().map(|t| t.to_string()).unwrap_or_default())
        .collect();
    // "Ada" (STRLEN = 3) comes before "Grace" (STRLEN = 5) ascending
    assert_eq!(
        names[0], "\"Ada\"",
        "shortest name should sort first: {:?}",
        names
    );
    assert_eq!(
        names[1], "\"Grace\"",
        "longer name should sort second: {:?}",
        names
    );
}

#[test]
fn bind_concat_preserves_lang_and_typechecks() {
    // CONCAT (§17.4.5.4): operands sharing one language tag keep it; a non-string
    // typed operand is a type error → the BIND variable is unbound (never wrong).
    let maps = mapping();
    let conn = source();
    let q_lang = format!(
        "SELECT ?v WHERE {{ ?e <{EMP_NAME}> ?n . BIND(CONCAT(\"x\"@en, \"y\"@en) AS ?v) }}"
    );
    let plan = parse_and_translate(&q_lang, &maps, Dialect::Sqlite).unwrap();
    let sol = exec::select(&plan, &conn).unwrap();
    assert!(!sol.rows.is_empty());
    for r in &sol.rows {
        match &r[0] {
            Some(sf_core::Term::Literal(l)) => {
                assert_eq!(l.value(), "xy");
                assert_eq!(l.language(), Some("en"), "common language tag preserved");
            }
            other => panic!("expected a lang-tagged literal, got {other:?}"),
        }
    }
    let q_type = format!("SELECT ?v WHERE {{ ?e <{EMP_NAME}> ?n . BIND(CONCAT(\"a\", 1) AS ?v) }}");
    let plan = parse_and_translate(&q_type, &maps, Dialect::Sqlite).unwrap();
    let sol = exec::select(&plan, &conn).unwrap();
    assert!(!sol.rows.is_empty());
    for r in &sol.rows {
        assert!(
            r[0].is_none(),
            "CONCAT with a non-string (xsd:integer) operand ⇒ ?v unbound"
        );
    }
}

// ---- SCRATCH ADVERSARIAL TESTS (count-variants lens) ----
fn agg_mapping_nullable() -> Vec<TriplesMap> {
    vec![TriplesMap {
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
            pom("http://ex/empSalary", column_literal("salary")),
        ],
    }]
}

fn agg_source_nullable() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    // Linus (id 3) has NULL salary -> :empSalary triple does NOT exist for him.
    conn.execute_batch(
        "CREATE TABLE emp(id INTEGER PRIMARY KEY, name TEXT, dept_id INTEGER, salary INTEGER);
         INSERT INTO emp VALUES (1,'Ada',10,100),(2,'Grace',10,300),(3,'Linus',20,NULL);",
    )
    .unwrap();
    conn
}

#[test]
fn scratch_count_var_over_optional_unbound() {
    let maps = agg_mapping_nullable();
    let conn = agg_source_nullable();
    // 3 employees; only 2 have a salary. SPARQL §11:
    //   COUNT(*)    = 3 (counts solutions)
    //   COUNT(?sal) = 2 (counts only BOUND ?sal)
    let q = format!(
        "SELECT (COUNT(*) AS ?all) (COUNT(?sal) AS ?bound) (COUNT(DISTINCT ?sal) AS ?dist) \
         WHERE {{ ?e <{EMP_NAME}> ?n OPTIONAL {{ ?e <http://ex/empSalary> ?sal }} }}"
    );
    let r = parse_and_translate(&q, &maps, Dialect::Sqlite);
    match r {
        Ok(plan) => {
            eprintln!("SQL = {}", plan.emitted().unwrap()[0].sql);
            let sol = exec::select(&plan, &conn).unwrap();
            eprintln!("vars = {:?}", sol.vars);
            for row in &sol.rows {
                eprintln!(
                    "all={} bound={} dist={}",
                    lit(&row[0]),
                    lit(&row[1]),
                    lit(&row[2])
                );
            }
        }
        Err(e) => eprintln!("DEFERRED/ERR: {e:?}"),
    }
}

// ---- SPARQL §11 aggregate correctness: empty-group SUM = 0; AVG datatype ----

const EMP_SALARY: &str = "http://ex/empSalary";
const EMP_FSALARY: &str = "http://ex/empFloatSalary";
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const XSD_DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";
const XSD_DOUBLE: &str = "http://www.w3.org/2001/XMLSchema#double";

/// The XSD datatype IRI of a reconstructed typed-literal binding.
fn dt(t: &Option<sf_core::Term>) -> String {
    match t {
        Some(sf_core::Term::Literal(l)) => l.datatype().as_str().to_owned(),
        other => panic!("expected a typed literal, got {other:?}"),
    }
}

/// `salary` is INTEGER, `fsal` is REAL (xsd:double natural type). Linus (id 3) has
/// NULL for both, so neither :empSalary nor :empFloatSalary triple exists for him.
fn agg_typed_mapping() -> Vec<TriplesMap> {
    vec![TriplesMap {
        id: "EMP".to_owned(),
        source: LogicalSource::Table("emp".to_owned()),
        subject: SubjectMap {
            term: template_iri("http://ex/emp/{id}"),
            classes: vec![iri(EMPLOYEE)],
            graphs: vec![],
        },
        predicate_object_maps: vec![
            pom(EMP_NAME, column_literal("name")),
            pom(EMP_SALARY, column_literal("salary")),
            pom(EMP_FSALARY, column_literal("fsal")),
        ],
    }]
}

fn agg_typed_source() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE emp(id INTEGER PRIMARY KEY, name TEXT, salary INTEGER, fsal REAL);
         INSERT INTO emp VALUES (1,'Ada',100,1.5),(2,'Grace',300,2.5),(3,'Linus',NULL,NULL);",
    )
    .unwrap();
    conn
}

#[test]
fn sum_over_empty_group_is_zero_integer() {
    // SPARQL §11: SUM over an empty multiset is "0"^^xsd:integer (NOT unbound). A
    // FILTER matching nobody empties the inner; implicit grouping still yields one
    // row, and SUM(∅) = 0. (SQL SUM over zero rows is NULL → must map to 0 here.)
    let maps = agg_typed_mapping();
    let conn = agg_typed_source();
    let q = format!(
        "SELECT (SUM(?sal) AS ?s) WHERE {{ ?e <{EMP_NAME}> ?n . ?e <{EMP_SALARY}> ?sal \
         FILTER(?n = \"Nobody\") }}"
    );
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let sol = exec::select(&plan, &conn).unwrap();
    assert_eq!(
        sol.rows.len(),
        1,
        "implicit grouping yields exactly one row"
    );
    assert_eq!(lit(&sol.rows[0][0]), "0", "SUM over an empty multiset is 0");
    assert_eq!(dt(&sol.rows[0][0]), XSD_INTEGER, "empty SUM is xsd:integer");
}

#[test]
fn sum_over_all_null_optional_is_zero_integer() {
    // SPARQL §11: an OPTIONAL column unbound in every solution leaves SUM an empty
    // multiset of bound values ⇒ "0"^^xsd:integer (NOT unbound). Linus is matched
    // but has no salary, so ?sal is unbound in his (only) solution.
    let maps = agg_typed_mapping();
    let conn = agg_typed_source();
    let q = format!(
        "SELECT (SUM(?sal) AS ?s) WHERE {{ ?e <{EMP_NAME}> ?n \
         OPTIONAL {{ ?e <{EMP_SALARY}> ?sal }} FILTER(?n = \"Linus\") }}"
    );
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let sol = exec::select(&plan, &conn).unwrap();
    assert_eq!(sol.rows.len(), 1);
    assert_eq!(
        lit(&sol.rows[0][0]),
        "0",
        "SUM over an all-NULL optional is 0"
    );
    assert_eq!(dt(&sol.rows[0][0]), XSD_INTEGER);
}

#[test]
fn avg_over_double_column_is_xsd_double() {
    // SPARQL §11.4: AVG result datatype follows the operand numeric type. A REAL /
    // xsd:double operand ⇒ xsd:double (NOT xsd:decimal). fsal = {1.5, 2.5} ⇒ 2.0.
    let maps = agg_typed_mapping();
    let conn = agg_typed_source();
    let q = format!("SELECT (AVG(?fs) AS ?a) WHERE {{ ?e <{EMP_FSALARY}> ?fs }}");
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let sol = exec::select(&plan, &conn).unwrap();
    assert_eq!(sol.rows.len(), 1);
    assert_eq!(
        dt(&sol.rows[0][0]),
        XSD_DOUBLE,
        "AVG over double is xsd:double"
    );
    assert_eq!(
        lit(&sol.rows[0][0]),
        "2.0E0",
        "AVG(1.5, 2.5) = 2.0 (canonical xsd:double)"
    );
}

#[test]
fn avg_over_integer_column_is_xsd_decimal() {
    // SPARQL §11.4: an xsd:integer operand promotes to xsd:decimal under SUM/COUNT
    // division — NOT xsd:double, even though SQLite's AVG yields a REAL value.
    // salary = {100, 300} ⇒ 200.
    let maps = agg_typed_mapping();
    let conn = agg_typed_source();
    let q = format!("SELECT (AVG(?sal) AS ?a) WHERE {{ ?e <{EMP_SALARY}> ?sal }}");
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let sol = exec::select(&plan, &conn).unwrap();
    assert_eq!(sol.rows.len(), 1);
    assert_eq!(
        dt(&sol.rows[0][0]),
        XSD_DECIMAL,
        "AVG over integer is xsd:decimal"
    );
    assert_eq!(lit(&sol.rows[0][0]), "200", "AVG(100, 300) = 200");
}

#[test]
fn sum_over_nonempty_integer_group_is_xsd_integer() {
    // Regression guard for SUM type inference via storage_class_code:
    // SUM(integer_col) over non-empty group must be typed xsd:integer, not plain
    // string. SQLite SUM over INTEGER values returns INTEGER storage class so the
    // per-row storage_class_code path (not decltype) must correctly produce
    // Some(XsdTypeCode::Integer). Ada=100, Grace=300 → SUM=400^^xsd:integer.
    let maps = agg_typed_mapping();
    let conn = agg_typed_source();
    let q = format!("SELECT (SUM(?sal) AS ?s) WHERE {{ ?e <{EMP_SALARY}> ?sal }}");
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let sol = exec::select(&plan, &conn).unwrap();
    assert_eq!(sol.rows.len(), 1, "implicit grouping yields one row");
    assert_eq!(lit(&sol.rows[0][0]), "400", "SUM(100, 300) = 400");
    assert_eq!(
        dt(&sol.rows[0][0]),
        XSD_INTEGER,
        "SUM over integer column must be xsd:integer"
    );
}

// --- FILTER EXISTS / NOT EXISTS (SPARQL §8.4) ---------------------------------

#[test]
fn filter_not_exists_is_correlated_anti_join() {
    // FILTER NOT EXISTS { ?e :empName "Ada" } over ?e :empName ?n — keeps only
    // the employee whose name is NOT "Ada" (i.e. Grace).  Semantically the same
    // result as MINUS here but uses the FILTER NOT EXISTS spelling.
    let maps = mapping();
    let conn = source();
    let q = format!(
        "SELECT ?n WHERE {{ ?e <{EMP_NAME}> ?n . FILTER NOT EXISTS {{ ?e <{EMP_NAME}> \"Ada\" }} }}"
    );
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    // The emitted SQL must contain NOT EXISTS for the anti-join.
    let sql = &plan.emitted().unwrap()[0].sql;
    assert!(
        sql.to_uppercase().contains("NOT EXISTS"),
        "NOT EXISTS must appear in SQL: {sql}"
    );
    let mut got: Vec<String> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| lit(&r[0]))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec!["Grace"],
        "FILTER NOT EXISTS should keep only Grace"
    );
}

#[test]
fn filter_exists_is_correlated_semi_join() {
    // FILTER EXISTS { ?e :empDept ?d . ?d :deptName "R&D" } over ?e :empName ?n —
    // keeps only employees whose department name is "R&D" (i.e. Ada in dept 10).
    let maps = mapping();
    let conn = source();
    let q = format!(
        "SELECT ?n WHERE {{ ?e <{EMP_NAME}> ?n . \
         FILTER EXISTS {{ ?e <{EMP_DEPT}> ?d . ?d <{DEPT_NAME}> \"R&D\" }} }}"
    );
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let sql = &plan.emitted().unwrap()[0].sql;
    // The emitted SQL must contain EXISTS (without NOT) for the semi-join.
    let up = sql.to_uppercase();
    assert!(
        up.contains("EXISTS") && !up.contains("NOT EXISTS"),
        "EXISTS (not NOT EXISTS) must appear in SQL: {sql}"
    );
    let got: Vec<String> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| lit(&r[0]))
        .collect();
    assert_eq!(
        got,
        vec!["Ada"],
        "FILTER EXISTS should keep only Ada (dept R&D)"
    );
}

#[test]
fn filter_not_exists_with_and_is_supported() {
    // FILTER(NOT EXISTS{P} && NOT EXISTS{Q}) — EXISTS nested inside AND.  Both
    // subpatterns constrain the result; the combined filter keeps only Grace
    // (Ada is eliminated by the first NOT EXISTS).
    let maps = mapping();
    let conn = source();
    let q = format!(
        "SELECT ?n WHERE {{ ?e <{EMP_NAME}> ?n . \
         FILTER(NOT EXISTS {{ ?e <{EMP_NAME}> \"Ada\" }} && NOT EXISTS {{ ?e <{EMP_NAME}> \"NoOne\" }}) }}"
    );
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let mut got: Vec<String> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| lit(&r[0]))
        .collect();
    got.sort();
    // "NoOne" doesn't exist so the second NOT EXISTS is vacuously true; first NOT EXISTS removes Ada.
    assert_eq!(got, vec!["Grace"]);
}

// --- GRAPH <g> { P } (SPARQL §13 / R2RML rr:graphName) -----------------------

/// Build a mapping where EMP triples are in the default graph and DEPT triples
/// are in the named graph `http://ex/namedGraph`.
fn named_graph_mapping() -> Vec<TriplesMap> {
    let graph_iri = sf_core::Term::NamedNode(iri("http://ex/namedGraph"));
    let emp = TriplesMap {
        id: "EMP".to_owned(),
        source: LogicalSource::Table("emp".to_owned()),
        subject: SubjectMap {
            term: template_iri("http://ex/emp/{id}"),
            classes: vec![],
            graphs: vec![], // default graph
        },
        predicate_object_maps: vec![pom(EMP_NAME, column_literal("name"))],
    };
    let dept = TriplesMap {
        id: "DEPT".to_owned(),
        source: LogicalSource::Table("dept".to_owned()),
        subject: SubjectMap {
            term: template_iri("http://ex/dept/{id}"),
            classes: vec![],
            graphs: vec![TermMap::Constant(graph_iri)], // named graph
        },
        predicate_object_maps: vec![pom(DEPT_NAME, column_literal("dname"))],
    };
    vec![emp, dept]
}

#[test]
fn graph_clause_filters_to_named_graph() {
    // GRAPH <http://ex/namedGraph> { ?d :deptName ?dn } — the DEPT triples are in
    // the named graph; only those rows should appear.  EMP triples (default graph)
    // must NOT leak through.
    let maps = named_graph_mapping();
    let conn = source(); // re-uses the standard emp+dept tables
    let q =
        format!("SELECT ?dn WHERE {{ GRAPH <http://ex/namedGraph> {{ ?d <{DEPT_NAME}> ?dn }} }}");
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let mut got: Vec<String> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| lit(&r[0]))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec!["Ops", "R&D"],
        "GRAPH clause should see only dept rows in the named graph"
    );
}

#[test]
fn graph_clause_unknown_graph_returns_empty() {
    // GRAPH <http://ex/unknown> { ?d :deptName ?dn } — no triples are mapped to
    // this IRI, so the result must be empty (not a 501 error).
    let maps = named_graph_mapping();
    let conn = source();
    let q = format!("SELECT ?dn WHERE {{ GRAPH <http://ex/unknown> {{ ?d <{DEPT_NAME}> ?dn }} }}");
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let got: Vec<String> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| lit(&r[0]))
        .collect();
    assert!(
        got.is_empty(),
        "unknown named graph must produce no results: {got:?}"
    );
}

#[test]
fn named_graph_triples_invisible_without_graph_clause() {
    // Regression for graph_maps_match(None, non_empty) bug: a query WITHOUT a
    // GRAPH clause over a mapping where DEPT_NAME triples sit in a named graph
    // must return EMPTY — those triples are not in the default graph.
    // Before the fix, graph_maps_match returned true unconditionally for
    // active=None, causing named-graph triples to leak.
    let maps = named_graph_mapping(); // DEPT triples in http://ex/namedGraph
    let conn = source();
    let q = format!("SELECT ?dn WHERE {{ ?d <{DEPT_NAME}> ?dn }}");
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let got: Vec<String> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| lit(&r[0]))
        .collect();
    assert!(
        got.is_empty(),
        "named-graph triples must NOT appear in default-graph query: {got:?}"
    );
}

#[test]
fn filter_exists_with_optional_derived_var_returns_501() {
    // Regression for outer_opt_aliases dead-code bug: FILTER EXISTS where a
    // shared variable (?dept) comes from the outer OPTIONAL arm must NOT emit
    // wrong SQL `col = col` correlation (NULL = value → false → wrong). Instead
    // the engine must surface a 501 Unsupported error so the caller can handle
    // or fall back gracefully.
    let maps = mapping();
    let q = format!(
        "SELECT ?n WHERE {{ ?e <{EMP_NAME}> ?n . OPTIONAL {{ ?e <{EMP_DEPT}> ?dept }} \
         FILTER EXISTS {{ ?dept <{DEPT_NAME}> ?dn }} }}"
    );
    let result = parse_and_translate(&q, &maps, Dialect::Sqlite);
    assert!(
        result.is_err(),
        "EXISTS with OPTIONAL-derived outer variable must return 501, not Ok: {result:?}"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("UNBOUND") || msg.contains("501") || msg.contains("OPTIONAL"),
        "error should mention OPTIONAL/UNBOUND/501, got: {msg}"
    );
}

/// A mapping where DEPT triples are declared in BOTH the default graph (via
/// `rr:defaultGraph`) AND a named graph simultaneously.  R2RML §7.4 allows
/// multiple graph maps on the same subject/predicate-object map; the triple
/// must appear in both graphs.
fn dual_graph_mapping() -> Vec<TriplesMap> {
    let default_g = sf_core::Term::NamedNode(iri("http://www.w3.org/ns/r2rml#defaultGraph"));
    let named_g = sf_core::Term::NamedNode(iri("http://ex/namedGraph"));
    let dept = TriplesMap {
        id: "DEPT".to_owned(),
        source: LogicalSource::Table("dept".to_owned()),
        subject: SubjectMap {
            term: template_iri("http://ex/dept/{id}"),
            classes: vec![],
            // Both rr:defaultGraph and a named graph — R2RML §7.4 multi-graph.
            graphs: vec![TermMap::Constant(default_g), TermMap::Constant(named_g)],
        },
        predicate_object_maps: vec![pom(DEPT_NAME, column_literal("dname"))],
    };
    vec![dept]
}

#[test]
fn multi_graph_default_and_named_visible_in_default_query() {
    // Regression for graph_maps_match None-branch defect: when graphs contains
    // BOTH rr:defaultGraph AND a named-graph IRI, the previous !any(n != rr:defaultGraph)
    // predicate would fire on the named-graph entry and exclude the triple from
    // default-graph queries. The fix uses (is_empty || any(n == rr:defaultGraph)).
    let maps = dual_graph_mapping();
    let conn = source();
    let q = format!("SELECT ?dn WHERE {{ ?d <{DEPT_NAME}> ?dn }}");
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let mut got: Vec<String> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| lit(&r[0]))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec!["Ops", "R&D"],
        "rr:defaultGraph + named-graph dual declaration: triple must appear in default-graph query"
    );
}

#[test]
fn multi_graph_default_and_named_visible_in_graph_clause() {
    // Complement: the same triple must also be visible via GRAPH <http://ex/namedGraph>.
    let maps = dual_graph_mapping();
    let conn = source();
    let q =
        format!("SELECT ?dn WHERE {{ GRAPH <http://ex/namedGraph> {{ ?d <{DEPT_NAME}> ?dn }} }}");
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let mut got: Vec<String> = exec::select(&plan, &conn)
        .unwrap()
        .rows
        .iter()
        .map(|r| lit(&r[0]))
        .collect();
    got.sort();
    assert_eq!(
        got,
        vec!["Ops", "R&D"],
        "rr:defaultGraph + named-graph dual declaration: triple must appear in named-graph clause"
    );
}

#[test]
fn optional_multi_scan_right_side() {
    // OPTIONAL { ?e :empName ?n . ?e :empDept ?d } has a multi-scan right side
    // (two triple patterns → two scan branches within the right side).
    // The ISWC-2018 decomposition must produce the correct left-join semantics:
    // employees with neither name nor dept appear with both vars unbound;
    // employees with both appear fully bound.
    let maps = mapping();
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE emp(id INTEGER PRIMARY KEY, name TEXT, dept_id INTEGER);
         CREATE TABLE dept(id INTEGER PRIMARY KEY, dname TEXT);
         INSERT INTO emp VALUES (1,'Ada',10),(2,'Ghost',NULL);
         INSERT INTO dept VALUES (10,'R&D');",
    )
    .unwrap();

    // ?e a :Employee . OPTIONAL { ?e :empName ?n . ?e :empDept ?d }
    // The OPTIONAL right side resolves to TWO scans (?e :empName ?n uses EMP,
    // ?e :empDept ?d also uses EMP — after self-join elimination both fold to the
    // same scan, so we test with a cross-table multi-scan by also fetching dept).
    let q = format!(
        "SELECT ?e ?n ?dn WHERE {{ ?e a <{EMPLOYEE}> . OPTIONAL {{ ?e <{EMP_NAME}> ?n . \
         ?e <{EMP_DEPT}> ?d . ?d <{DEPT_NAME}> ?dn }} }}"
    );
    let plan = parse_and_translate(&q, &maps, Dialect::Sqlite).unwrap();
    let sol = exec::select(&plan, &conn).unwrap();
    let got: BTreeSet<(String, Option<String>, Option<String>)> = sol
        .rows
        .iter()
        .map(|r| {
            (
                lit(&r[0]),
                r[1].as_ref().map(|_| lit(&r[1])),
                r[2].as_ref().map(|_| lit(&r[2])),
            )
        })
        .collect();
    let expect: BTreeSet<(String, Option<String>, Option<String>)> = [
        (
            "<http://ex/emp/1>".to_owned(),
            Some("Ada".to_owned()),
            Some("R&D".to_owned()),
        ),
        // emp/2 has NULL dept_id → the JOIN inside OPTIONAL fails → both ?n and ?dn unbound
        ("<http://ex/emp/2>".to_owned(), None, None),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        got, expect,
        "multi-scan OPTIONAL right must null-preserve non-matching rows"
    );
}
