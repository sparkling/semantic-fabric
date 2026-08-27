//! Issue #8 regression coverage: every intra-atom bind/constrain operation must
//! prune an incompatible mapping branch. The flat and tree translators share
//! this unfolding path, so both are compared with the mapping's materialized
//! graph evaluated by spareval.

use oxrdf::Term;
use rusqlite::Connection;
use sf_conformance::oracle::{self, OracleAnswer};
use sf_conformance::{graph, sqlite};
use sf_sparql::{exec, translate_with, translate_with_flat, Plan, Tbox};
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

fn translate_both(sql: &str, r2rml: &str, query: &str) -> (Plan, Plan) {
    let schema = schema_of(sql);
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let query = parse(query);
    let flat = translate_with_flat(&query, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
        .expect("flat translator accepts regression query");
    let tree = translate_with(&query, &maps, Dialect::Sqlite, &Tbox::default(), &schema)
        .expect("tree translator accepts regression query");
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

const SQL: &str = r#"
CREATE TABLE people (id INTEGER NOT NULL, name TEXT NOT NULL);
INSERT INTO people VALUES (1, 'Alice');
CREATE TABLE companies (id INTEGER NOT NULL, name TEXT NOT NULL);
INSERT INTO companies VALUES (1, 'Acme');
"#;

const R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .

<#People>
    rr:logicalTable [ rr:tableName "people" ] ;
    rr:subjectMap [ rr:template "http://ex/person/{id}" ; rr:class ex:Person ] ;
    rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] .

<#Companies>
    rr:logicalTable [ rr:tableName "companies" ] ;
    rr:subjectMap [ rr:template "http://ex/company/{id}" ; rr:class ex:Company ] ;
    rr:predicateObjectMap [ rr:predicate ex:name ; rr:objectMap [ rr:column "name" ] ] .
"#;

#[test]
fn incompatible_constant_subject_prunes_the_whole_atom_branch() {
    assert_both_match_oracle(
        SQL,
        R2RML,
        "PREFIX ex: <http://ex/> SELECT ?name WHERE { <http://ex/person/1> ex:name ?name }",
        1,
    );
}

#[test]
fn repeated_subject_and_predicate_variable_prunes_incompatible_branches() {
    assert_both_match_oracle(SQL, R2RML, "SELECT ?x ?value WHERE { ?x ?x ?value }", 0);
    assert_both_match_oracle(
        SQL,
        GRAPH_PREDICATE_R2RML,
        r#"SELECT ?x ?s WHERE { GRAPH ?x { ?s ?x "Alice" } }"#,
        0,
    );
}

#[test]
fn class_atom_repeated_predicate_and_object_variable_is_pruned() {
    assert_both_match_oracle(SQL, R2RML, "SELECT ?s ?x WHERE { ?s ?x ?x }", 0);
}

const GRAPH_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .

<#People>
    rr:logicalTable [ rr:tableName "people" ] ;
    rr:subjectMap [
        rr:template "http://ex/person/{id}" ;
        rr:class ex:Person ;
        rr:graphMap [ rr:constant ex:peopleGraph ]
    ] .
"#;

const GRAPH_PREDICATE_R2RML: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .

<#People>
    rr:logicalTable [ rr:tableName "people" ] ;
    rr:subjectMap [
        rr:template "http://ex/person/{id}" ;
        rr:graphMap [ rr:constant ex:peopleGraph ]
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:name ;
        rr:objectMap [ rr:column "name" ]
    ] .
"#;

#[test]
fn class_atom_repeated_graph_and_object_variable_is_pruned() {
    assert_both_match_oracle(
        SQL,
        GRAPH_R2RML,
        "SELECT ?x ?s WHERE { GRAPH ?x { ?s a ?x } }",
        0,
    );
}
