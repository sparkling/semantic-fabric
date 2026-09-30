//! Single-default-graph generated admission executed on SQLite: exact bags for
//! each selected graph, never an RDF merge with the default or another graph.

use std::cell::Cell;
use std::sync::Arc;

use rusqlite::Connection;
use sf_core::query_control::UncontrolledQueryControl;
use sf_core::{SourceId, SourceMapping};
use sf_sparql::cache::generated::{
    ConstantCoverageError, ConstantOccurrence, DatasetGraphAllowlist, DatasetRule,
    GeneratedCompileError, GeneratedDatasetError, GeneratedQueryRefusal, ShapeRule,
};
use sf_sparql::{exec, CompilerBinding, Epoch, Plan, Tbox};
use sf_sql::Dialect;

const A: &str = "http://ex/A";
const B: &str = "http://ex/B";
/// Allowlisted, but no mapping produces it: a selected graph may be empty.
const E: &str = "http://ex/E";
const GRAPHS: [&str; 3] = [A, B, E];
const P: &str = "http://ex/p";
const Q: &str = "http://ex/q";
const FREE: &UncontrolledQueryControl = &UncontrolledQueryControl;
const PROJECT: &str = "SELECT ?o FROM <G> WHERE { ?s <http://ex/p> ?o }";

type Verdict = Result<(), ConstantCoverageError>;
type Row = Vec<Option<String>>;

fn map(id: &str, graph: Option<&str>, predicate: &str) -> String {
    let graph = graph
        .map(|g| format!("; rr:graph <{g}>"))
        .unwrap_or_default();
    format!(
        "<http://ex/map/{id}> rr:logicalTable [ rr:tableName \"{id}\" ];
           rr:subjectMap [ rr:template \"http://ex/{{s}}\"{graph} ];
           rr:predicateObjectMap [ rr:predicate <{predicate}>;
             rr:objectMap [ rr:column \"o\" ] ] .\n"
    )
}

/// Graph A: (s1 p "a") (s2 p "a") (s1 q "x"); graph B: (s1 p "b") (s1 q "y");
/// default graph: (s9 p "d"). Graph E is empty.
fn fixture() -> (CompilerBinding, Connection) {
    let text = format!(
        "@prefix rr: <http://www.w3.org/ns/r2rml#> .\n{}{}{}{}{}",
        map("ap", Some(A), P),
        map("aq", Some(A), Q),
        map("bp", Some(B), P),
        map("bq", Some(B), Q),
        map("dp", None, P),
    );
    let maps = sf_mapping::parse_r2rml(&text).unwrap();
    let binding = CompilerBinding::from_unverified_observation(
        SourceMapping::new(SourceId::new(0).unwrap(), maps),
        Dialect::Sqlite,
        Tbox::default(),
        Vec::new(),
        Epoch::default(),
        64,
    );
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE ap(s TEXT, o TEXT); INSERT INTO ap VALUES ('s1','a'),('s2','a');
         CREATE TABLE aq(s TEXT, o TEXT); INSERT INTO aq VALUES ('s1','x');
         CREATE TABLE bp(s TEXT, o TEXT); INSERT INTO bp VALUES ('s1','b');
         CREATE TABLE bq(s TEXT, o TEXT); INSERT INTO bq VALUES ('s1','y');
         CREATE TABLE dp(s TEXT, o TEXT); INSERT INTO dp VALUES ('s9','d');",
    )
    .unwrap();
    (binding, conn)
}

fn at(query: &str, graph: &str) -> String {
    query.replace("<G>", &format!("<{graph}>"))
}

fn compile_with<G>(
    binding: &CompilerBinding,
    query: &str,
    cached: bool,
    graph: G,
) -> Result<Arc<Plan>, GeneratedDatasetError>
where
    G: FnMut(&str) -> Verdict,
{
    let list = DatasetGraphAllowlist::new(GRAPHS).unwrap();
    let constant = |_: ConstantOccurrence<'_>| -> Verdict { Ok(()) };
    if cached {
        binding.compile_shared_with_single_default_dataset(query, &list, FREE, graph, constant)
    } else {
        binding.compile_uncached_shared_with_single_default_dataset(
            query, &list, FREE, graph, constant,
        )
    }
}

fn compile(binding: &CompilerBinding, query: &str, cached: bool) -> Arc<Plan> {
    compile_with(binding, query, cached, |_: &str| -> Verdict { Ok(()) }).unwrap()
}

fn bag(conn: &Connection, plan: &Plan) -> Vec<Row> {
    let solutions = exec::select(plan, conn).unwrap();
    let mut rows: Vec<Row> = solutions
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| cell.as_ref().map(|t| t.to_string()))
                .collect()
        })
        .collect();
    rows.sort();
    rows
}

fn rows(cells: &[&[Option<&str>]]) -> Vec<Row> {
    let mut out: Vec<Row> = cells
        .iter()
        .map(|row| {
            row.iter()
                .map(|cell| cell.map(|v| format!("\"{v}\"")))
                .collect()
        })
        .collect();
    out.sort();
    out
}

fn assert_bags(binding: &CompilerBinding, conn: &Connection, template: &str, want: [Vec<Row>; 3]) {
    for (graph, expected) in GRAPHS.iter().zip(want) {
        let query = at(template, graph);
        for cached in [true, false] {
            let plan = compile(binding, &query, cached);
            assert_eq!(bag(conn, &plan), expected, "{query} cached={cached}");
        }
    }
}

#[test]
fn select_bags_follow_only_the_selected_graph() {
    let (binding, conn) = fixture();
    let a = Some("a");
    let cases: Vec<(&str, [Vec<Row>; 3])> =
        vec![
        (PROJECT, [rows(&[&[a], &[a]]), rows(&[&[Some("b")]]), rows(&[])]),
        (
            "SELECT ?o ?v FROM <G> WHERE { ?s <http://ex/p> ?o . ?s <http://ex/q> ?v }",
            [rows(&[&[a, Some("x")]]), rows(&[&[Some("b"), Some("y")]]), rows(&[])],
        ),
        (
            "SELECT ?o ?v FROM <G> WHERE { { ?s <http://ex/p> ?o } { ?s <http://ex/q> ?v } }",
            [rows(&[&[a, Some("x")]]), rows(&[&[Some("b"), Some("y")]]), rows(&[])],
        ),
        (
            "SELECT ?o ?v FROM <G> WHERE { ?s <http://ex/p> ?o OPTIONAL { ?s <http://ex/q> ?v } }",
            [
                rows(&[&[a, Some("x")], &[a, None]]),
                rows(&[&[Some("b"), Some("y")]]),
                rows(&[]),
            ],
        ),
        (
            "SELECT ?o FROM <G> WHERE { ?s <http://ex/p> ?o FILTER(?o = \"a\") }",
            [rows(&[&[a], &[a]]), rows(&[]), rows(&[])],
        ),
        (
            "SELECT ?o FROM <G> WHERE { { ?s <http://ex/p> ?o } UNION { ?s <http://ex/q> ?o } }",
            [
                rows(&[&[a], &[a], &[Some("x")]]),
                rows(&[&[Some("b")], &[Some("y")]]),
                rows(&[]),
            ],
        ),
        (
            "SELECT DISTINCT ?o FROM <G> WHERE { ?s <http://ex/p> ?o }",
            [rows(&[&[a]]), rows(&[&[Some("b")]]), rows(&[])],
        ),
        (
            "SELECT REDUCED ?o FROM <G> WHERE { ?s <http://ex/p> ?o }",
            [rows(&[&[a]]), rows(&[&[Some("b")]]), rows(&[])],
        ),
        (
            "SELECT ?o FROM <G> WHERE { ?s <http://ex/p> ?o } LIMIT 1",
            [rows(&[&[a]]), rows(&[&[Some("b")]]), rows(&[])],
        ),
        (
            "SELECT ?o FROM <G> WHERE { ?s <http://ex/p> ?o } OFFSET 1",
            [rows(&[&[a]]), rows(&[]), rows(&[])],
        ),
    ];
    for (template, want) in cases {
        assert_bags(&binding, &conn, template, want);
    }
}

#[test]
fn ask_follows_only_the_selected_graph() {
    let (binding, conn) = fixture();
    let cases = [
        ("ASK FROM <G> { ?s <http://ex/p> ?o }", [true, true, false]),
        (
            "ASK FROM <G> { ?s <http://ex/p> \"d\" }",
            [false, false, false],
        ),
        ("ASK FROM <G> { }", [true, true, true]),
    ];
    for (template, want) in cases {
        for (graph, expected) in GRAPHS.iter().zip(want) {
            let query = at(template, graph);
            for cached in [true, false] {
                let plan = compile(&binding, &query, cached);
                assert_eq!(exec::ask(&plan, &conn).unwrap(), expected, "{query}");
            }
        }
    }
    // The default graph really holds "d"; the selection above never merged it.
    let ordinary = binding
        .compile_shared("ASK { ?s <http://ex/p> \"d\" }")
        .unwrap();
    assert!(exec::ask(&ordinary, &conn).unwrap());
    let ordinary = binding
        .compile_shared("SELECT ?o WHERE { ?s <http://ex/p> ?o }")
        .unwrap();
    assert_eq!(bag(&conn, &ordinary), rows(&[&[Some("d")]]));
}

#[test]
fn values_and_empty_bgp_identity_hold_for_every_graph() {
    let (binding, conn) = fixture();
    let values = rows(&[&[Some("v")], &[Some("v")], &[None]]);
    assert_bags(
        &binding,
        &conn,
        "SELECT ?x FROM <G> WHERE { VALUES ?x { \"v\" \"v\" UNDEF } }",
        [values.clone(), values.clone(), values],
    );
    let empty = rows(&[&[]]);
    assert_bags(
        &binding,
        &conn,
        "SELECT * FROM <G> WHERE { }",
        [empty.clone(), empty.clone(), empty],
    );
}

#[test]
fn duplicate_from_is_admitted_per_occurrence_then_deduplicated() {
    let (binding, conn) = fixture();
    let calls = Cell::new(0);
    let query = at(
        "SELECT ?o FROM <G> FROM <G> WHERE { ?s <http://ex/p> ?o }",
        A,
    );
    let counting = |_: &str| -> Verdict {
        calls.set(calls.get() + 1);
        Ok(())
    };
    let plan = compile_with(&binding, &query, true, counting).unwrap();
    assert_eq!(calls.get(), 2);
    assert_eq!(bag(&conn, &plan), rows(&[&[Some("a")], &[Some("a")]]));
    let single = compile(&binding, &at(PROJECT, A), true);
    assert!(Arc::ptr_eq(&plan, &single));
}

#[test]
fn graph_choices_never_share_cache_entries() {
    let (binding, conn) = fixture();
    for template in [PROJECT, "SELECT ?x FROM <G> WHERE { VALUES ?x { \"v\" } }"] {
        let cold = compile(&binding, &at(template, A), true);
        let warm = compile(&binding, &at(template, A), true);
        let other = compile(&binding, &at(template, B), true);
        let uncached = compile(&binding, &at(template, A), false);
        assert!(Arc::ptr_eq(&cold, &warm), "{template}");
        assert!(!Arc::ptr_eq(&cold, &other), "{template}");
        assert!(!Arc::ptr_eq(&cold, &uncached), "{template}");
        assert_eq!(bag(&conn, &cold), bag(&conn, &uncached), "{template}");
    }
}

#[test]
fn refusals_are_named_before_any_source_work() {
    let (binding, _conn) = fixture();
    let cases = [
        ("SELECT ?o FROM <G> FROM <http://ex/B> WHERE { ?s <http://ex/p> ?o }", DatasetRule::MultipleDefaultGraphs),
        ("SELECT ?o FROM <G> WHERE { GRAPH <G> { ?s <http://ex/p> ?o } }", DatasetRule::AuthoredGraph),
        ("SELECT ?o FROM <G> WHERE { GRAPH ?g { ?s <http://ex/p> ?o } }", DatasetRule::AuthoredGraph),
        ("SELECT ?o WHERE { ?s <http://ex/p> ?o }", DatasetRule::MissingDataset),
        ("SELECT ?o FROM NAMED <G> WHERE { ?s <http://ex/p> ?o }", DatasetRule::NamedGraphDataset),
        ("SELECT ?o FROM <http://ex/Z> WHERE { ?s <http://ex/p> ?o }", DatasetRule::GraphNotAdmitted),
        ("SELECT ?o FROM <G> WHERE { ?s <http://ex/p> ?o FILTER EXISTS { ?s <http://ex/q> ?v } }", DatasetRule::Exists),
    ];
    for (template, rule) in cases {
        for cached in [true, false] {
            let calls = Cell::new(0);
            let counting = |_: &str| -> Verdict {
                calls.set(calls.get() + 1);
                Ok(())
            };
            let error = compile_with(&binding, &at(template, A), cached, counting).unwrap_err();
            assert!(
                matches!(error, GeneratedDatasetError::Refused(found) if found == rule),
                "{template}: {error}"
            );
            assert_eq!(calls.get(), 0, "{template}");
        }
    }
}

#[test]
fn existing_generated_admission_still_refuses_dataset_clauses() {
    let (binding, _conn) = fixture();
    let refused = GeneratedQueryRefusal::Rule(ShapeRule::DatasetClause);
    let query = at(PROJECT, A);
    let allow_all = |_: ConstantOccurrence<'_>| -> Verdict { Ok(()) };
    let cached = binding.compile_shared_with_generated_admission(&query, FREE, allow_all);
    let uncached =
        binding.compile_uncached_shared_with_generated_admission(&query, FREE, allow_all);
    for result in [cached, uncached] {
        let error = result.unwrap_err();
        assert!(matches!(error, GeneratedCompileError::Refused(r) if r == refused));
    }
}
