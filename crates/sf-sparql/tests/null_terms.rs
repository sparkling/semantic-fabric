//! R2RML term absence must remove triples before projection or correlation.
use rusqlite::Connection;
use sf_core::ir::TriplesMap;
use sf_mapping::parse_r2rml;
use sf_sparql::{exec, translate, translate_flat, translate_unoptimized, Plan, Tbox};
use sf_sql::Dialect;

const PREFIX: &str = "@prefix rr: <http://www.w3.org/ns/r2rml#> .";
const SUBJECT: &str = "rr:template \"http://ex/{s}/{tail}\"";
const POM: &str = r#"rr:predicateObjectMap [ rr:predicate <http://ex/p>;
    rr:objectMap [ rr:column "o" ] ]"#;

fn maps(subject: &str, pom: &str) -> Vec<TriplesMap> {
    parse_r2rml(&format!(
        "{PREFIX} <http://ex/map> rr:logicalTable [ rr:tableName \"facts\" ];
         rr:subjectMap [ {subject} ]; {pom} ."
    ))
    .unwrap()
}

fn source(values: &str) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!(
        "CREATE TABLE facts(s TEXT, tail TEXT, p TEXT, o TEXT, g TEXT);
         INSERT INTO facts VALUES {values};"
    ))
    .unwrap();
    conn
}

fn plans(query: &str, maps: &[TriplesMap]) -> Vec<(&'static str, Plan)> {
    let query = spargebra::SparqlParser::new().parse_query(query).unwrap();
    vec![
        ("tree", translate(&query, maps, Dialect::Sqlite).unwrap()),
        (
            "flat",
            translate_flat(&query, maps, Dialect::Sqlite).unwrap(),
        ),
        (
            "unoptimized",
            translate_unoptimized(&query, maps, Dialect::Sqlite, &Tbox::default(), &[]).unwrap(),
        ),
    ]
}

fn absent(conn: &Connection, maps: &[TriplesMap], pattern: &str) {
    // Dropping a NULL binding during term reconstruction is too late: the
    // unprojected subject still affects SELECT, ASK and aggregate cardinality.
    for (name, plan) in plans(&format!("SELECT ?o WHERE {{ {pattern} }}"), maps) {
        assert!(
            exec::select(&plan, conn).unwrap().rows.is_empty(),
            "{name}: {pattern}"
        );
    }
    for (name, plan) in plans(&format!("ASK {{ {pattern} }}"), maps) {
        assert!(!exec::ask(&plan, conn).unwrap(), "{name}: {pattern}");
    }
    for (name, plan) in plans(
        &format!("SELECT (COUNT(*) AS ?n) WHERE {{ {pattern} }}"),
        maps,
    ) {
        let rows = exec::select(&plan, conn).unwrap().rows;
        assert_eq!(rows.len(), 1, "{name}");
        assert_eq!(
            rows[0][0].as_ref().unwrap().to_string(),
            "\"0\"^^<http://www.w3.org/2001/XMLSchema#integer>",
            "{name}"
        );
    }
}

#[test]
fn null_subject_components_produce_no_solution_even_when_not_projected() {
    for subject in [
        SUBJECT,
        "rr:column \"s\"",
        "rr:template \"{s}/{tail}\"; rr:termType rr:BlankNode",
    ] {
        for values in [
            "(NULL,'end','http://ex/p','value',NULL)",
            "('http://ex/s',NULL,'http://ex/p','value',NULL)",
        ] {
            if subject == "rr:column \"s\"" && !values.starts_with("(NULL") {
                continue;
            }
            absent(&source(values), &maps(subject, POM), "?s <http://ex/p> ?o");
        }
    }
}

#[test]
fn null_predicate_and_class_subject_are_not_triples() {
    let predicate = r#"rr:predicateObjectMap [ rr:predicateMap [ rr:column "p" ]; rr:objectMap [ rr:column "o" ] ]"#;
    absent(
        &source("('s','end',NULL,'value',NULL)"),
        &maps(SUBJECT, predicate),
        "?s ?p ?o",
    );
    absent(
        &source("(NULL,'end','http://ex/p','value',NULL)"),
        &maps(&format!("{SUBJECT}; rr:class <http://ex/C>"), POM),
        "?s a ?o",
    );
}

#[test]
fn reference_join_keys_do_not_prove_parent_subject_is_present() {
    let maps = parse_r2rml(&format!(
        r#"{PREFIX}
      <http://ex/parent> rr:logicalTable [ rr:tableName "facts" ];
        rr:subjectMap [ {SUBJECT} ].
      <http://ex/child> rr:logicalTable [ rr:tableName "links" ];
        rr:subjectMap [ rr:template "http://ex/child/{{id}}" ];
        rr:predicateObjectMap [ rr:predicate <http://ex/link>; rr:objectMap [
          rr:parentTriplesMap <http://ex/parent>;
          rr:joinCondition [ rr:child "parent"; rr:parent "tail" ] ] ]."#
    ))
    .unwrap();
    let conn = source("(NULL,'joined',NULL,'value',NULL)");
    conn.execute_batch(
        "CREATE TABLE links(id TEXT,parent TEXT); INSERT INTO links VALUES('child','joined');",
    )
    .unwrap();
    absent(&conn, &maps, "?s <http://ex/link> ?o");
    absent(&conn, &maps, "?s ^<http://ex/link> ?o");
    conn.execute("UPDATE facts SET s='present'", []).unwrap();
    for (_, plan) in plans("SELECT ?s ?o WHERE { ?s <http://ex/link> ?o }", &maps) {
        let rows = exec::select(&plan, &conn).unwrap().rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0][1].as_ref().unwrap().to_string(),
            "<http://ex/present/joined>"
        );
    }
}

#[test]
fn absent_terms_stay_inside_optional_and_existence_conditions() {
    let variants = [
        (SUBJECT.to_owned(), POM.to_owned(), "(NULL,'end',NULL,'value',NULL)", "?s <http://ex/p> ?o"),
        (SUBJECT.to_owned(), "rr:predicateObjectMap [ rr:predicateMap [ rr:column \"p\" ]; rr:objectMap [ rr:column \"o\" ] ]".to_owned(), "('s','end',NULL,'value',NULL)", "?s ?p ?o"),
    ];
    for (subject, pom, values, rhs) in variants {
        let maps = maps(&subject, &pom);
        let conn = source(values);
        for (condition, expected) in [
            (format!("OPTIONAL {{ {rhs} }}"), 1),
            (format!("FILTER EXISTS {{ {rhs} }}"), 0),
            (format!("FILTER NOT EXISTS {{ {rhs} }}"), 1),
            (format!("MINUS {{ {rhs} }}"), 1),
        ] {
            let query = format!("SELECT ?o ?s WHERE {{ VALUES ?o {{ \"value\" }} {condition} }}");
            for (name, plan) in plans(&query, &maps) {
                let rows = exec::select(&plan, &conn).unwrap().rows;
                assert_eq!(rows.len(), expected, "{name}: {query}");
                for row in rows {
                    assert_eq!(row[0].as_ref().unwrap().to_string(), "\"value\"");
                    assert!(row[1].is_none(), "false right binding {name}: {query}");
                }
            }
        }
    }
}

#[test]
fn null_graph_alternative_does_not_suppress_a_valid_graph() {
    let maps = maps(&format!("{SUBJECT}; rr:graph <http://ex/graph>, rr:defaultGraph; rr:graphMap [ rr:column \"g\" ]"), POM);
    let conn = source("('s','end',NULL,'value',NULL)");
    for query in [
        "SELECT ?o WHERE { GRAPH <http://ex/graph> { ?s <http://ex/p> ?o } }",
        "SELECT ?o WHERE { ?s <http://ex/p> ?o }",
    ] {
        for (name, plan) in plans(query, &maps) {
            let rows = exec::select(&plan, &conn).unwrap().rows;
            assert_eq!(rows.len(), 1, "{name}: {query}");
            assert_eq!(rows[0][0].as_ref().unwrap().to_string(), "\"value\"");
        }
    }
}

#[test]
fn selected_null_graph_map_produces_no_named_graph_solution() {
    let maps = maps(&format!("{SUBJECT}; rr:graphMap [ rr:column \"g\" ]"), POM);
    absent(
        &source("('s','end',NULL,'value',NULL)"),
        &maps,
        "GRAPH ?g { ?s <http://ex/p> ?o }",
    );
}
