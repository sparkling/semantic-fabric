//! Issue #9 regression coverage for R2RML target-graph semantics. A generated
//! triple belongs to the distinct union of its subject-map and POM graph maps;
//! `rr:defaultGraph` is a default-graph destination, never a `GRAPH ?g` name.

use oxrdf::Term;
use rusqlite::Connection;
use sf_conformance::oracle::{self, OracleAnswer};
use sf_conformance::{graph, sqlite};
use sf_sparql::{exec, translate_with, translate_with_flat, Error, Plan, Tbox};
use sf_sql::{Dialect, TableSchema};
use spargebra::{Query, SparqlParser};
use std::collections::BTreeMap;

type Rows = Vec<BTreeMap<String, Term>>;

fn parse(query: &str) -> Query {
    SparqlParser::new()
        .parse_query(query)
        .expect("query parses")
}

fn schema_of(sql: &str) -> Vec<TableSchema> {
    let conn = sqlite::load(sql).expect("fixture loads");
    sqlite::introspect_all(&conn).expect("introspection")
}

fn translate_both(
    sql: &str,
    r2rml: &str,
    query: &str,
) -> (sf_sparql::Result<Plan>, sf_sparql::Result<Plan>) {
    let schema = schema_of(sql);
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let query = parse(query);
    let flat = translate_with_flat(&query, &maps, Dialect::Sqlite, &Tbox::default(), &schema);
    let tree = translate_with(&query, &maps, Dialect::Sqlite, &Tbox::default(), &schema);
    (flat, tree)
}

fn run_select(plan: &Plan, conn: &Connection) -> Rows {
    oracle::engine_bag(&exec::select(plan, conn).expect("select executes"))
}

fn oracle_bag(sql: &str, r2rml: &str, query: &str) -> Rows {
    let conn = sqlite::load(sql).expect("fixture loads");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let quads = exec::dump_quads(&maps, &conn, Dialect::Sqlite).expect("materializes");
    let dataset = graph::quads_to_dataset(&quads);
    match oracle::evaluate(&dataset, query).expect("oracle evaluates") {
        OracleAnswer::Solutions(rows) => rows,
        other => panic!("expected SELECT solutions, got {other:?}"),
    }
}

fn assert_both_match_oracle(sql: &str, r2rml: &str, query: &str, expected_len: usize) {
    let (flat, tree) = translate_both(sql, r2rml, query);
    let flat = flat.unwrap_or_else(|e| panic!("flat rejected `{query}`: {e:?}"));
    let tree = tree.unwrap_or_else(|e| panic!("tree rejected `{query}`: {e:?}"));
    let conn = sqlite::load(sql).expect("fixture loads");
    let flat_rows = run_select(&flat, &conn);
    let tree_rows = run_select(&tree, &conn);
    let oracle_rows = oracle_bag(sql, r2rml, query);

    assert!(
        oracle::solutions_bag_eq(&flat_rows, &oracle_rows),
        "flat/oracle mismatch for `{query}`:\nflat={flat_rows:#?}\noracle={oracle_rows:#?}"
    );
    assert!(
        oracle::solutions_bag_eq(&tree_rows, &oracle_rows),
        "tree/oracle mismatch for `{query}`:\ntree={tree_rows:#?}\noracle={oracle_rows:#?}"
    );
    assert_eq!(tree_rows.len(), expected_len, "unexpected tree row bag");
}

fn assert_both_501(sql: &str, r2rml: &str, query: &str) {
    let (flat, tree) = translate_both(sql, r2rml, query);
    assert!(
        matches!(flat, Err(Error::Unsupported(_))),
        "flat must reject an unrepresentable graph constraint: {flat:?}"
    );
    assert!(
        matches!(tree, Err(Error::Unsupported(_))),
        "tree must reject an unrepresentable graph constraint: {tree:?}"
    );
}

const EDGE_SQL: &str = r#"
CREATE TABLE edge (s INTEGER NOT NULL, o INTEGER NOT NULL);
INSERT INTO edge VALUES (1, 2);
INSERT INTO edge VALUES (2, 3);
"#;

const UNION_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Edge>
    rr:logicalTable [ rr:tableName "edge" ] ;
    rr:subjectMap [
        rr:template "http://ex/n/{s}" ;
        rr:graph ex:subjectGraph
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:reaches ;
        rr:objectMap [ rr:template "http://ex/n/{o}" ] ;
        rr:graph ex:pomGraph
    ] .
"#;

#[test]
fn subject_and_pom_named_graphs_are_both_queryable() {
    for graph in ["subjectGraph", "pomGraph"] {
        let query = format!(
            "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE {{ GRAPH ex:{graph} {{ ?s ex:reaches ?o }} }}"
        );
        assert_both_match_oracle(EDGE_SQL, UNION_R2RML, &query, 2);
    }
    assert_both_match_oracle(
        EDGE_SQL,
        UNION_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s ex:reaches ?o } }",
        4,
    );
}

#[test]
fn paths_use_both_halves_of_the_graph_union() {
    for graph in ["subjectGraph", "pomGraph"] {
        let query = format!(
            "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE {{ GRAPH ex:{graph} {{ ?s ex:reaches+ ?o }} }}"
        );
        assert_both_match_oracle(EDGE_SQL, UNION_R2RML, &query, 3);
    }
    assert_both_match_oracle(
        EDGE_SQL,
        UNION_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s ex:reaches+ ?o } }",
        6,
    );
}

const DUPLICATE_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Edge>
    rr:logicalTable [ rr:tableName "edge" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{s}" ; rr:graph ex:g ] ;
    rr:predicateObjectMap [
        rr:predicate ex:reaches ;
        rr:objectMap [ rr:template "http://ex/n/{o}" ] ;
        rr:graph ex:g
    ] .
"#;

#[test]
fn identical_graph_at_both_levels_emits_one_solution_per_triple() {
    assert_both_match_oracle(
        EDGE_SQL,
        DUPLICATE_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s ex:reaches ?o } }",
        2,
    );
}

const DEFAULT_AND_NAMED_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Edge>
    rr:logicalTable [ rr:tableName "edge" ] ;
    rr:subjectMap [ rr:template "http://ex/n/{s}" ; rr:graph rr:defaultGraph ] ;
    rr:predicateObjectMap [
        rr:predicate ex:reaches ;
        rr:objectMap [ rr:template "http://ex/n/{o}" ] ;
        rr:graph ex:pomGraph
    ] .
"#;

#[test]
fn explicit_default_and_named_destinations_are_both_visible() {
    assert_both_match_oracle(
        EDGE_SQL,
        DEFAULT_AND_NAMED_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:reaches ?o }",
        2,
    );
    assert_both_match_oracle(
        EDGE_SQL,
        DEFAULT_AND_NAMED_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s ex:reaches ?o } }",
        2,
    );
}

const DYNAMIC_SQL: &str = r#"
CREATE TABLE dynamic_edge (s INTEGER NOT NULL, o INTEGER NOT NULL, g TEXT NOT NULL);
INSERT INTO dynamic_edge VALUES (1, 2, 'http://ex/g1');
INSERT INTO dynamic_edge VALUES (3, 4, 'http://www.w3.org/ns/r2rml#defaultGraph');
"#;

const DYNAMIC_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Edge>
    rr:logicalTable [ rr:tableName "dynamic_edge" ] ;
    rr:subjectMap [
        rr:template "http://ex/n/{s}" ;
        rr:graphMap [ rr:column "g" ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:reaches ;
        rr:objectMap [ rr:template "http://ex/n/{o}" ]
    ] .
"#;

#[test]
fn dynamic_graphs_constrain_fixed_and_default_queries_and_hide_default_name() {
    assert_both_match_oracle(
        DYNAMIC_SQL,
        DYNAMIC_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { GRAPH ex:g1 { ?s ex:reaches ?o } }",
        1,
    );
    assert_both_match_oracle(
        DYNAMIC_SQL,
        DYNAMIC_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { ?s ex:reaches ?o }",
        1,
    );
    assert_both_match_oracle(
        DYNAMIC_SQL,
        DYNAMIC_R2RML,
        "SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s <http://ex/reaches> ?o } }",
        1,
    );
}

#[test]
fn path_over_a_row_dependent_fixed_graph_is_an_honest_501() {
    assert_both_501(
        DYNAMIC_SQL,
        DYNAMIC_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { GRAPH ex:g1 { ?s ex:reaches+ ?o } }",
    );
}

const MULTISLOT_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Edge>
    rr:logicalTable [ rr:tableName "dynamic_edge" ] ;
    rr:subjectMap [
        rr:template "http://ex/n/{s}" ;
        rr:graphMap [ rr:template "http://ex/{s}/{o}" ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:reaches ;
        rr:objectMap [ rr:template "http://ex/n/{o}" ]
    ] .
"#;

#[test]
fn unrepresentable_dynamic_graph_constraint_is_an_honest_501() {
    assert_both_501(
        DYNAMIC_SQL,
        MULTISLOT_R2RML,
        "SELECT ?s ?o WHERE { GRAPH <http://ex/1/2> { ?s <http://ex/reaches> ?o } }",
    );
}

const CORRELATED_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
<#Edge>
    rr:logicalTable [ rr:tableName "edge" ] ;
    rr:subjectMap [
        rr:template "http://ex/n/{s}" ;
        rr:graphMap [ rr:template "http://ex/n/{s}" ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:reaches ;
        rr:objectMap [ rr:template "http://ex/n/{o}" ]
    ] .
"#;

#[test]
fn graph_variable_reused_as_subject_is_correlated_normally() {
    assert_both_match_oracle(
        EDGE_SQL,
        CORRELATED_R2RML,
        "PREFIX ex: <http://ex/> SELECT ?s ?o WHERE { GRAPH ?s { ?s ex:reaches ?o } }",
        2,
    );
}
