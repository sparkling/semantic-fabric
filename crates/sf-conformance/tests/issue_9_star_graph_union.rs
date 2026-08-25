//! Issue #9 RDF-star graph inheritance. Standalone proposition-form description
//! maps must inherit the complete subject/POM graph union in both star-map
//! positions, not a POM-overrides-subject approximation.

use oxrdf::Term;
use rusqlite::Connection;
use sf_conformance::oracle::{self, OracleAnswer};
use sf_conformance::{graph, sqlite};
use sf_sparql::{exec, translate_with, translate_with_flat, Plan, Tbox};
use sf_sql::Dialect;
use spargebra::SparqlParser;
use std::collections::BTreeMap;

type Rows = Vec<BTreeMap<String, Term>>;

fn run_select(plan: &Plan, conn: &Connection) -> Rows {
    oracle::engine_bag(&exec::select(plan, conn).expect("select executes"))
}

fn assert_both_match_oracle(r2rml: &str, query: &str, expected_len: usize) {
    let conn = sqlite::load(SQL).expect("fixture loads");
    let schema = sqlite::introspect_all(&conn).expect("introspection");
    let maps = sf_mapping::parse_r2rml(r2rml).expect("R2RML parses");
    let query_ast = SparqlParser::new()
        .parse_query(query)
        .expect("query parses");
    let flat = translate_with_flat(
        &query_ast,
        &maps,
        Dialect::Sqlite,
        &Tbox::default(),
        &schema,
    )
    .expect("flat translator accepts description query");
    let tree = translate_with(
        &query_ast,
        &maps,
        Dialect::Sqlite,
        &Tbox::default(),
        &schema,
    )
    .expect("tree translator accepts description query");
    let flat_rows = run_select(&flat, &conn);
    let tree_rows = run_select(&tree, &conn);

    let quads = exec::dump_quads(&maps, &conn, Dialect::Sqlite).expect("materializes");
    let dataset = graph::quads_to_dataset(&quads);
    let oracle_rows = match oracle::evaluate(&dataset, query).expect("oracle evaluates") {
        OracleAnswer::Solutions(rows) => rows,
        other => panic!("expected SELECT solutions, got {other:?}"),
    };

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
CREATE TABLE fact (id INTEGER NOT NULL, age INTEGER NOT NULL);
INSERT INTO fact VALUES (1, 30);
"#;

const SUBJECT_STAR_R2RML: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://ex/> .

<#PersonAge>
    rr:logicalTable [ rr:tableName "fact" ] ;
    rr:subjectMap [ rr:template "http://ex/person/{id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasAge ;
        rr:objectMap [ rr:column "age" ]
    ] .

<#Assertion>
    rr:logicalTable [ rr:tableName "fact" ] ;
    rr:subjectMap [
        rml:starMap [ rml:quotedTriplesMap <#PersonAge> ] ;
        rr:graph ex:subjectGraph
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:assertedBy ;
        rr:object ex:Source ;
        rr:graph ex:pomGraph
    ] .
"#;

const OBJECT_STAR_R2RML: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix ex:  <http://ex/> .

<#PersonAge>
    rr:logicalTable [ rr:tableName "fact" ] ;
    rr:subjectMap [ rr:template "http://ex/person/{id}" ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasAge ;
        rr:objectMap [ rr:column "age" ]
    ] .

<#Quote>
    rr:logicalTable [ rr:tableName "fact" ] ;
    rr:subjectMap [
        rr:template "http://ex/quote/{id}" ;
        rr:graph ex:subjectGraph
    ] ;
    rr:predicateObjectMap [
        rr:predicate ex:hasQuote ;
        rr:objectMap [ rml:starMap [ rml:quotedTriplesMap <#PersonAge> ] ] ;
        rr:graph ex:pomGraph
    ] .
"#;

fn graph_query(graph: &str) -> String {
    format!(
        "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
         PREFIX ex: <http://ex/> \
         SELECT ?pf ?s ?o WHERE {{ GRAPH ex:{graph} {{ \
           ?pf a rdf:PropositionForm ; \
               rdf:propositionFormSubject ?s ; \
               rdf:propositionFormObject ?o \
         }} }}"
    )
}

#[test]
fn subject_position_description_inherits_subject_and_pom_graphs() {
    for graph in ["subjectGraph", "pomGraph"] {
        assert_both_match_oracle(SUBJECT_STAR_R2RML, &graph_query(graph), 1);
    }
}

#[test]
fn object_position_description_inherits_subject_and_pom_graphs() {
    for graph in ["subjectGraph", "pomGraph"] {
        assert_both_match_oracle(OBJECT_STAR_R2RML, &graph_query(graph), 1);
    }
}

#[test]
fn graph_variable_enumerates_each_inherited_description_graph_once() {
    let query = "PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
                 SELECT ?g ?pf WHERE { GRAPH ?g { ?pf a rdf:PropositionForm } }";
    assert_both_match_oracle(SUBJECT_STAR_R2RML, query, 2);
    assert_both_match_oracle(OBJECT_STAR_R2RML, query, 2);
}
