//! Characterization for the source-affinity sidecar contract.

use rusqlite::Connection;
use sf_core::SourceId;
use sf_mapping::{parse_r2rml, parse_r2rml_for_source};
use sf_sparql::{exec, parse_and_translate};
use sf_sql::Dialect;

const MAPPING: &str = r#"
    @prefix rr: <http://www.w3.org/ns/r2rml#> .
    <#items> rr:logicalTable [ rr:tableName "items" ];
        rr:subjectMap [ rr:template "http://example.com/items/{id}" ];
        rr:predicateObjectMap [
            rr:predicate <http://example.com/name>;
            rr:objectMap [ rr:column "name" ]
        ].
"#;

const QUERY: &str = r#"
    SELECT ?name WHERE {
        ?item <http://example.com/name> ?name
    }
    ORDER BY ?name
"#;

#[test]
fn source_mapping_preserves_single_source_translation_and_results() {
    let plain = parse_r2rml(MAPPING).unwrap();
    let source_mapping = parse_r2rml_for_source(MAPPING, SourceId::new(5).unwrap()).unwrap();

    let plain_plan = parse_and_translate(QUERY, &plain, Dialect::Sqlite).unwrap();
    let source_plan =
        parse_and_translate(QUERY, source_mapping.triples_maps(), Dialect::Sqlite).unwrap();
    let plain_sql = plain_plan.emitted().unwrap();
    let source_sql = source_plan.emitted().unwrap();

    assert_eq!(plain_sql.len(), source_sql.len());
    for (plain_branch, source_branch) in plain_sql.iter().zip(&source_sql) {
        assert_eq!(plain_branch.sql, source_branch.sql);
        assert_eq!(plain_branch.params, source_branch.params);
        assert_eq!(plain_branch.projection, source_branch.projection);
    }

    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE items(id INTEGER PRIMARY KEY, name TEXT);\
             INSERT INTO items VALUES (1, 'Ada'), (2, 'Grace');",
        )
        .unwrap();
    let plain_solutions = exec::select(&plain_plan, &connection).unwrap();
    let source_solutions = exec::select(&source_plan, &connection).unwrap();

    assert_eq!(plain_solutions.vars, source_solutions.vars);
    assert_eq!(plain_solutions.rows, source_solutions.rows);
    assert_eq!(source_mapping.source_id().index(), 5);
}
