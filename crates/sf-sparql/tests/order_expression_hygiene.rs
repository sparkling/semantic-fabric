//! Raw expression-ORDER hygiene. Production serving rejects expression keys
//! until its evaluator is SPARQL-error exact; these tests keep the lower-level
//! diagnostic API from corrupting a legal user binding in the meantime.

use rusqlite::Connection;
use sf_core::Term;
use sf_sparql::{exec, parse_and_translate};
use sf_sql::Dialect;

const MAPPING: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://example.test/> .
<#Items> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "items" ] ;
  rr:subjectMap [ rr:template "http://example.test/item/{id}" ] ;
  rr:predicateObjectMap [
    rr:predicate ex:label ; rr:objectMap [ rr:column "label" ]
  ] .
"#;

fn execute(query: &str) -> Vec<Vec<String>> {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, label TEXT NOT NULL);\
             INSERT INTO items VALUES (1, 'zeta'), (2, 'alpha');",
        )
        .unwrap();
    let mapping = sf_mapping::parse_r2rml(MAPPING).unwrap();
    let plan = parse_and_translate(query, &mapping, Dialect::Sqlite).unwrap();
    exec::select(&plan, &connection)
        .unwrap()
        .rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|term| match term {
                    Some(Term::Literal(literal)) => literal.value().to_owned(),
                    other => panic!("expected literal binding, got {other:?}"),
                })
                .collect()
        })
        .collect()
}

#[test]
fn successful_expression_key_cannot_overwrite_legal_user_binding() {
    let rows = execute(
        "SELECT ?__sf_ord_0 ?label WHERE { \
           ?item <http://example.test/label> ?label . \
           BIND(\"user\" AS ?__sf_ord_0) \
         } ORDER BY STRLEN(?label)",
    );
    assert_eq!(
        rows,
        vec![
            vec!["user".to_owned(), "zeta".to_owned()],
            vec!["user".to_owned(), "alpha".to_owned()],
        ]
    );
}

#[test]
fn failed_expression_key_cannot_fall_back_to_legal_user_binding() {
    let rows = execute(
        "SELECT ?__sf_ord_0 ?label WHERE { \
           ?item <http://example.test/label> ?label . \
           BIND(?label AS ?__sf_ord_0) \
         } ORDER BY (1 / 0)",
    );
    assert_eq!(
        rows,
        vec![
            vec!["zeta".to_owned(), "zeta".to_owned()],
            vec!["alpha".to_owned(), "alpha".to_owned()],
        ],
        "unbound expression keys tie and preserve source arrival order"
    );
}
