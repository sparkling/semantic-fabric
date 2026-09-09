//! PostgreSQL NUMERIC uses its native arbitrary-precision lexical decoder.
use super::*;

#[test]
#[ignore = "requires owned pinned PostgreSQL TLS fixture"]
fn postgres_large_numeric_decoder_is_exact() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, true);
    sql(&database, "ALTER TABLE items ADD COLUMN src NUMERIC");
    assert_large_decoder(&fixture, &database);
}

pub(super) fn assert_large_decoder(fixture: &Fixture, database: &Database) {
    fixture.write(
        "first.ttl",
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> rr:logicalTable [rr:tableName "items"];
 rr:subject <http://example.test/item>;
 rr:predicateObjectMap [rr:predicate <http://example.test/edge>;
   rr:objectMap [rr:template "http://example.test/n/{src}"]]."#,
    );
    fixture.write(
        "ontology.ttl",
        "<http://example.test/edge> a <http://www.w3.org/2002/07/owl#ObjectProperty> .",
    );
    sql(database, "DELETE FROM items; ALTER TABLE items ALTER COLUMN src TYPE NUMERIC USING src::NUMERIC; INSERT INTO items(src,value) VALUES ((repeat('1',131072) || '.00')::NUMERIC,'same')");
    let (server, address) = start(fixture, database);
    let query = format!("SELECT ?o WHERE {{ ?s <{EDGE}> ?o }}");
    let (status, body) = request_format_bounded(
        address,
        &query,
        Some(&fixture.token),
        "application/sparql-results+json",
        200_000,
    )
    .unwrap();
    assert_eq!(status, 200, "{query}: {}", String::from_utf8_lossy(&body));
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let bindings = result["results"]["bindings"].as_array().unwrap();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0]["o"]["type"], "uri");
    assert_eq!(
        bindings[0]["o"]["value"],
        format!("http://example.test/n/{}.00", "1".repeat(131072))
    );
    database.assert_encrypted_sessions();
    drop(server);
}
