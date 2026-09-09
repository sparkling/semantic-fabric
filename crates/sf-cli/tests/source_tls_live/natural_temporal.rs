//! Natural temporal values retain canonical lexical form and RDF datatype.
use super::*;

#[test]
#[ignore = "requires owned pinned MySQL TLS fixture"]
fn mysql_natural_temporal_identity_is_exact() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, false);
    sql(&database, "ALTER TABLE items ADD COLUMN id INTEGER PRIMARY KEY DEFAULT 0; ALTER TABLE items ADD COLUMN src DATETIME(6)");
    assert_responses(&fixture, &database);
}

pub(super) fn assert_responses(fixture: &Fixture, database: &Database) {
    fixture.write("first.ttl", r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> rr:logicalTable [rr:tableName "items"];
 rr:subject <http://example.test/item>;
 rr:predicateObjectMap [rr:predicate <http://example.test/date>; rr:objectMap [rr:column "src"]];
 rr:predicateObjectMap [rr:predicate <http://example.test/edge>; rr:objectMap [rr:template "http://example.test/n/{src}"]]."#);
    fixture.write("ontology.ttl", "<http://example.test/date> a <http://www.w3.org/2002/07/owl#DatatypeProperty> . <http://example.test/edge> a <http://www.w3.org/2002/07/owl#ObjectProperty> .");
    for (native, datatype, values, invalid) in [
        (
            "DATETIME(6)",
            "dateTime",
            vec![
                ("2024-02-29 12:34:56.100000", "2024-02-29T12:34:56.1"),
                ("2000-02-29 00:00:00.123456", "2000-02-29T00:00:00.123456"),
                ("1900-03-01 00:00:00.000000", "1900-03-01T00:00:00"),
                ("0000-01-01 00:00:00.000000", "0000-01-01T00:00:00"),
                ("9999-12-31 23:59:59.999999", "9999-12-31T23:59:59.999999"),
            ],
            "2001-00-03 01:02:03.100000",
        ),
        (
            "DATE",
            "date",
            vec![
                ("2024-02-29", "2024-02-29"),
                ("2000-02-29", "2000-02-29"),
                ("1900-03-01", "1900-03-01"),
                ("0000-01-01", "0000-01-01"),
                ("9999-12-31", "9999-12-31"),
            ],
            "1900-02-29",
        ),
    ] {
        sql(
            database,
            &format!("DELETE FROM items; ALTER TABLE items MODIFY src {native}"),
        );
        for (i, (stored, _)) in values.iter().enumerate() {
            sql(database, &format!("SET SESSION sql_mode='ALLOW_INVALID_DATES'; INSERT INTO items(id,src,value) VALUES ({},'{stored}','same'),({},'{stored}','same')", i*2+1, i*2+2));
        }
        sql(
            database,
            "INSERT INTO items(id,src,value) VALUES (99,NULL,'absent')",
        );
        let (server, address) = start(fixture, database);
        let query = "SELECT ?o WHERE { ?s <http://example.test/date> ?o }";
        let result = rows(address, fixture, query);
        assert_eq!(result.len(), values.len());
        for (_, canonical) in &values {
            let literal = serde_json::json!({"type":"literal", "value":canonical, "datatype":format!("http://www.w3.org/2001/XMLSchema#{datatype}")});
            assert_eq!(result.iter().filter(|r| r["o"] == literal).count(), 1);
            for (lexical, dt, count) in [
                (*canonical, datatype, 1),
                (*canonical, "string", 0),
                ("2024-02-29T12:34:56.100000", datatype, 0),
            ] {
                let constant = format!("\"{lexical}\"^^<http://www.w3.org/2001/XMLSchema#{dt}>");
                for filter in [
                    format!("sameTerm(?o, {constant})"),
                    format!("sameTerm({constant}, ?o)"),
                ] {
                    let query = format!(
                        "SELECT ?o WHERE {{ ?s <http://example.test/date> ?o FILTER({filter}) }}"
                    );
                    assert_eq!(rows(address, fixture, &query).len(), count, "{query}");
                }
                let query =
                    format!("SELECT ?s WHERE {{ ?s <http://example.test/date> {constant} }}");
                assert_eq!(rows(address, fixture, &query).len(), count, "{query}");
            }
        }
        assert_eq!(
            rows(
                address,
                fixture,
                "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://example.test/date> ?o }"
            )[0]["n"]["value"],
            values.len().to_string()
        );
        assert_eq!(
            rows(
                address,
                fixture,
                "SELECT ?o WHERE { { SELECT ?o WHERE { ?s <http://example.test/date> ?o } } }"
            )
            .len(),
            values.len()
        );
        assert_eq!(rows(address, fixture, "SELECT ?o WHERE { ?s <http://example.test/date> ?o . ?s <http://example.test/edge> ?iri }").len(), values.len()*values.len());
        let negation = format!("SELECT ?o WHERE {{ ?s <http://example.test/date> ?o FILTER(!sameTerm(?o, \"{}\"^^<http://www.w3.org/2001/XMLSchema#{datatype}>)) }}", values[0].1);
        assert_eq!(rows(address, fixture, &negation).len(), values.len() - 1);
        assert!(rows(address, fixture, "SELECT ?s WHERE { VALUES ?s { <http://example.test/absent> } OPTIONAL { ?s <http://example.test/date> ?o } FILTER(!sameTerm(?o, \"wrong type\")) }").is_empty());
        sql(database, &format!("SET SESSION sql_mode='ALLOW_INVALID_DATES'; DELETE FROM items; INSERT INTO items(id,src,value) VALUES (1,'{invalid}','same')"));
        for query in [
            "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://example.test/date> ?o }",
            "SELECT ?o WHERE { ?s <http://example.test/date> ?o }",
            "ASK { ?s <http://example.test/date> ?o }",
            "SELECT ?s WHERE { ?s <http://example.test/date> ?o }",
        ] {
            let response = stop_matrix::wire(cancellation::begin(address, query, &fixture.token));
            assert!(
                response.starts_with(b"HTTP/1.1 200") || response.starts_with(b"HTTP/1.1 500"),
                "unexpected error response {query}: {}",
                String::from_utf8_lossy(&response)
            );
            stop_matrix::assert_no_complete_union_success(&response);
            let until = Instant::now() + Duration::from_secs(3);
            loop {
                let (status, _) = request(address, "ASK {}", Some(&fixture.token)).unwrap();
                if status == 200 {
                    break;
                }
                assert_eq!(status, 503);
                assert!(
                    Instant::now() < until,
                    "natural error did not release cap-one admission"
                );
                thread::sleep(Duration::from_millis(10));
            }
        }
        database.assert_encrypted_sessions();
        drop(server);
        // Denied invalid rows cannot influence valid authorized natural results.
        sql(database, &format!("UPDATE items SET value='other'; SET SESSION sql_mode='ALLOW_INVALID_DATES'; INSERT INTO items(id,src,value) VALUES (2,'{}','same')", values[0].0));
        let (mut command, address) = command_with_admission(
            fixture,
            database,
            None,
            &["--auth-subjects-env", "SF_POLICY_SUBJECTS"],
        );
        command.env("SF_POLICY_VALUE", "same").env("SF_POLICY_SUBJECTS", serde_json::json!({"schemaVersion":2,"subjects":[{
        "subjectRef":"natural-reader", "credentialEnv":"SF_TLS_BEARER",
        "portableRows":[{"sourceIndex":0,"table":"items","column":"value","valueEnv":"SF_POLICY_VALUE"}]
    }]}).to_string());
        let (server, address) = start_command(fixture, command, address);
        assert_eq!(rows(address, fixture, query).len(), 1);
        for (pattern, count) in [
            ("VALUES ?s { <http://example.test/item> } FILTER EXISTS { ?s <http://example.test/date> ?o }", 1),
            ("VALUES ?s { <http://example.test/item> } FILTER NOT EXISTS { ?s <http://example.test/date> ?o }", 0),
            ("VALUES ?s { <http://example.test/item> } OPTIONAL { ?s <http://example.test/date> ?o }", 1),
        ] {
            assert_eq!(rows(address, fixture, &format!("SELECT ?s WHERE {{ {pattern} }}")).len(), count);
        }
        assert_eq!(
            rows(
                address,
                fixture,
                "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://example.test/date> ?o }"
            )[0]["n"]["value"],
            "1"
        );
        database.assert_encrypted_sessions();
        drop(server);
    }
}
