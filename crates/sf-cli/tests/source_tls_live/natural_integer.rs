//! Natural integer identity validates the decoder without narrowing raw IRI keys.
use super::*;

const NATURAL: &str = "http://example.test/natural";
const EXPLICIT: &str = "http://example.test/explicit";

#[test]
#[ignore = "requires owned pinned PostgreSQL/MySQL TLS fixtures"]
fn native_integer_identity_and_policy_are_exact() {
    for postgres in [true, false] {
        let fixture = Fixture::new();
        let database = Database::start(&fixture, postgres);
        sql(&database, "ALTER TABLE items ADD COLUMN id INTEGER");
        assert_identity(&fixture, &database);
    }
}

pub(super) fn assert_identity(fixture: &Fixture, database: &Database) {
    let mysql = database.source.starts_with("mysql:");
    sql(
        database,
        &format!(
            "DELETE FROM items; ALTER TABLE items ADD COLUMN key_value BIGINT{}",
            if mysql { " UNSIGNED" } else { "" }
        ),
    );
    fixture.write("first.ttl", r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
        <#m> rr:logicalTable [rr:tableName "items"]; rr:subject <http://example.test/item>;
          rr:predicateObjectMap [rr:predicate <http://example.test/natural>; rr:objectMap [rr:column "key_value"]];
          rr:predicateObjectMap [rr:predicate <http://example.test/explicit>; rr:objectMap [rr:column "key_value"; rr:datatype <http://www.w3.org/2001/XMLSchema#integer>]];
          rr:predicateObjectMap [rr:predicate <http://example.test/edge>; rr:objectMap [rr:template "http://example.test/n/{key_value}"]]."#);
    fixture.write("ontology.ttl", &format!("<{NATURAL}> a <http://www.w3.org/2002/07/owl#DatatypeProperty> . <{EXPLICIT}> a <http://www.w3.org/2002/07/owl#DatatypeProperty> . <http://example.test/edge> a <http://www.w3.org/2002/07/owl#ObjectProperty> ."));
    sql(database, "INSERT INTO items(id,key_value,value) VALUES (1,0,'same'),(2,1,'same'),(3,1,'same'),(4,9223372036854775807,'same'),(5,NULL,'same')");
    let (server, address) = start(fixture, database);
    for lexical in ["0", "1", "9223372036854775807"] {
        let query = format!("SELECT ?s WHERE {{ ?s <{NATURAL}> \"{lexical}\"^^<http://www.w3.org/2001/XMLSchema#integer> }}");
        assert_eq!(rows(address, fixture, &query).len(), 1, "{query}");
    }
    for lexical in ["01", "+1", "-0"] {
        let query = format!("SELECT ?s WHERE {{ ?s <{NATURAL}> \"{lexical}\"^^<http://www.w3.org/2001/XMLSchema#integer> }}");
        assert!(rows(address, fixture, &query).is_empty(), "{query}");
    }
    for pattern in [
        format!("?s <{NATURAL}> ?n . ?t <{EXPLICIT}> ?n"),
        format!("?s <{NATURAL}> ?n FILTER EXISTS {{ ?t <{EXPLICIT}> ?n }}"),
        format!("?s <{NATURAL}> ?n OPTIONAL {{ ?t <{EXPLICIT}> ?n }}"),
    ] {
        let query = format!("SELECT ?n ?t WHERE {{ {pattern} }}");
        let actual = rows(address, fixture, &query);
        assert_eq!(actual.len(), 3, "{query}: {actual:?}");
        if pattern.contains("OPTIONAL") {
            assert!(actual.iter().all(|row| row.get("t").is_some()));
        }
    }
    database.assert_encrypted_sessions();
    drop(server);
    if mysql {
        assert_invalid_and_policy(fixture, database);
    }
    sql(
        database,
        "DELETE FROM items; ALTER TABLE items DROP COLUMN key_value",
    );
}

fn assert_invalid_and_policy(fixture: &Fixture, database: &Database) {
    sql(database, "DELETE FROM items; INSERT INTO items(id,key_value,value) VALUES (1,18446744073709551615,'same')");
    let (server, address) = start(fixture, database);
    // The raw unsigned wire lexical is a valid IRI component, not a natural
    // xsd:integer accepted by the current locked signed-i64 canonical parser.
    let raw = rows(
        address,
        fixture,
        "SELECT ?o WHERE { ?s <http://example.test/edge> ?o }",
    );
    assert_eq!(
        raw[0]["o"]["value"],
        "http://example.test/n/18446744073709551615"
    );
    for query in [
        format!("SELECT ?n WHERE {{ ?s <{NATURAL}> ?n }}"),
        format!("SELECT (COUNT(*) AS ?n) WHERE {{ ?s <{NATURAL}> ?o }}"),
        format!("ASK {{ ?s <{NATURAL}> ?o }}"),
    ] {
        let response = stop_matrix::wire(cancellation::begin(address, &query, &fixture.token));
        assert!(
            response.starts_with(b"HTTP/1.1 200") || response.starts_with(b"HTTP/1.1 500"),
            "{query}: {}",
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
                "natural integer error did not release admission"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    drop(server);
    sql(
        database,
        "UPDATE items SET value='other'; INSERT INTO items(id,key_value,value) VALUES (2,1,'same')",
    );
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
    for pattern in [
        format!("?s <{NATURAL}> ?o"),
        format!("?s <{NATURAL}> ?o . ?t <{EXPLICIT}> ?o"),
        format!("VALUES ?s {{ <http://example.test/item> }} FILTER EXISTS {{ ?s <{NATURAL}> ?o }}"),
        format!("VALUES ?s {{ <http://example.test/item> }} OPTIONAL {{ ?s <{NATURAL}> ?o }}"),
    ] {
        let query = format!("SELECT ?s WHERE {{ {pattern} }}");
        assert_eq!(
            rows(address, fixture, &query).len(),
            1,
            "denied invalid row: {query}"
        );
    }
    assert_eq!(
        rows(
            address,
            fixture,
            &format!("SELECT (COUNT(*) AS ?n) WHERE {{ ?s <{NATURAL}> ?o }}")
        )[0]["n"]["value"],
        "1"
    );
    database.assert_encrypted_sessions();
    drop(server);
}
