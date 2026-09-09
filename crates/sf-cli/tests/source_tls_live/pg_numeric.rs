//! PostgreSQL NUMERIC uses its native arbitrary-precision lexical decoder.
use super::*;
#[path = "pg_numeric_range.rs"]
mod range;

const NUMERIC_MAPPING: &str = r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#numeric> rr:logicalTable [rr:tableName "items"];
 rr:subject <http://example.test/item>;
 rr:predicateObjectMap [rr:predicate <http://example.test/edge>;
   rr:objectMap [rr:template "http://example.test/n/{src}"]];
 rr:predicateObjectMap [rr:predicate <http://example.test/number>;
   rr:objectMap [rr:column "src"]]."#;

pub(super) fn assert_all(fixture: &Fixture, database: &Database) {
    assert_large_decoder(fixture, database);
    range::assert_range(fixture, database);
    assert_identity(fixture, database);
    assert_references(fixture, database);
    assert_invalid(fixture, database);
}

pub(super) fn assert_mysql_range(fixture: &Fixture, database: &Database) {
    range::assert_mysql(fixture, database);
}

#[test]
#[ignore = "requires owned pinned PostgreSQL TLS fixture"]
fn postgres_numeric_identity_preserves_display_scale() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, true);
    assert_identity(&fixture, &database);
}

pub(super) fn assert_identity(fixture: &Fixture, database: &Database) {
    fixture.write("first.ttl", NUMERIC_MAPPING);
    fixture.write("ontology.ttl", "<http://example.test/edge> a <http://www.w3.org/2002/07/owl#ObjectProperty> . <http://example.test/number> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .");
    sql(database, "DELETE FROM items; ALTER TABLE items ADD COLUMN IF NOT EXISTS id INTEGER; ALTER TABLE items ADD COLUMN IF NOT EXISTS src NUMERIC; ALTER TABLE items ALTER COLUMN src TYPE NUMERIC USING src::NUMERIC; INSERT INTO items(id,src,value) VALUES (1,1.0,'same'),(2,1.00,'same'),(3,1.0,'same'),(4,0.000,'same'),(5,-0.000,'same'),(6,-12.3400,'same'),(7,1e-8,'same'),(8,NULL,'same')");
    let expected = ["1.0", "1.00", "0.000", "-12.3400", "0.00000001"];
    let (server, address) = start(fixture, database);
    for modifier in ["", "DISTINCT "] {
        let query = format!("SELECT {modifier}?o WHERE {{ ?s <{EDGE}> ?o }}");
        let result = rows(address, fixture, &query);
        assert_eq!(result.len(), expected.len(), "{query}: {result:?}");
        for lexical in expected {
            assert_eq!(
                result
                    .iter()
                    .filter(|row| row["o"]["value"] == format!("http://example.test/n/{lexical}"))
                    .count(),
                1,
                "{query}: {lexical}"
            );
        }
    }
    assert_eq!(
        rows(
            address,
            fixture,
            &format!("SELECT (COUNT(*) AS ?n) WHERE {{ ?s <{EDGE}> ?o }}")
        )[0]["n"]["value"],
        "5"
    );
    for lexical in expected {
        let iri = format!("<http://example.test/n/{lexical}>");
        for pattern in [
            format!("?s <{EDGE}> {iri}"),
            format!("?s <{EDGE}> ?o FILTER(?o = {iri})"),
            format!("?s <{EDGE}> ?o FILTER(sameTerm({iri}, ?o))"),
        ] {
            let query = format!("SELECT ?s WHERE {{ {pattern} }}");
            assert_eq!(rows(address, fixture, &query).len(), 1, "{query}");
        }
    }
    let natural = rows(
        address,
        fixture,
        "SELECT ?n WHERE { ?s <http://example.test/number> ?n }",
    );
    let union = format!("{{ ?s <{EDGE}> ?o FILTER(?o = <http://example.test/n/1.0>) }} UNION {{ ?s <{EDGE}> ?o FILTER(sameTerm(?o, <http://example.test/n/1.00>)) }}");
    for query in [
        format!("SELECT ?o WHERE {{ {{ SELECT DISTINCT ?o WHERE {{ {union} }} }} }}"),
        format!("SELECT ?o WHERE {{ VALUES ?anchor {{ 1 }} OPTIONAL {{ SELECT DISTINCT ?o WHERE {{ {union} }} }} }}"),
    ] {
        assert_eq!(request(address, &query, Some(&fixture.token)).unwrap().0, 501, "existing source-sized DISTINCT profile stays closed");
        let query = query.replace("SELECT DISTINCT ?o", "SELECT ?o");
        let result = rows(address, fixture, &query);
        assert_eq!(result.len(), 2, "{query}");
        for lexical in ["1.0", "1.00"] { assert_eq!(result.iter().filter(|r| r["o"]["value"] == format!("http://example.test/n/{lexical}")).count(), 1); }
    }
    assert_eq!(natural.len(), 4, "natural decimal values, not raw scale");
    for row in &natural {
        assert_eq!(
            row["n"]["datatype"],
            "http://www.w3.org/2001/XMLSchema#decimal"
        );
    }
    for modifier in ["", "DISTINCT "] {
        let query = format!(
            "SELECT {modifier}?o ?n WHERE {{ ?s <{EDGE}> ?o; <http://example.test/number> ?n }}"
        );
        assert_eq!(
            rows(address, fixture, &query).len(),
            20,
            "mixed IRI/natural bags: {query}"
        );
    }
    let query = format!("SELECT DISTINCT ?o ?n WHERE {{ ?s <{EDGE}> ?o; <http://example.test/number> ?n }} ORDER BY ?n LIMIT 6 OFFSET 1");
    let ordered = rows(address, fixture, &query);
    assert_eq!(ordered.len(), 6);
    assert_eq!(
        ordered
            .iter()
            .map(|row| row["n"]["value"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["-12.34", "-12.34", "-12.34", "-12.34", "0", "0"]
    );
    assert_eq!(rows(address, fixture, "SELECT DISTINCT ?n WHERE { ?s <http://example.test/edge> ?o; <http://example.test/number> ?n }").len(), 4, "hidden IRI is not a DISTINCT output key");
    database.assert_encrypted_sessions();
    drop(server);
    // One source column constructs BOTH positions of the same atom.
    fixture.write(
        "first.ttl",
        &NUMERIC_MAPPING.replace(
            "rr:subject <http://example.test/item>",
            "rr:subjectMap [rr:template \"http://example.test/n/{src}\"]",
        ),
    );
    let (server, address) = start(fixture, database);
    for modifier in ["", "DISTINCT "] {
        let result = rows(
            address,
            fixture,
            &format!("SELECT {modifier}?s ?n WHERE {{ ?s <http://example.test/number> ?n }}"),
        );
        assert_eq!(result.len(), 5, "single-atom decoded+natural consumer");
        for lexical in expected {
            assert_eq!(
                result
                    .iter()
                    .filter(|r| r["s"]["value"] == format!("http://example.test/n/{lexical}"))
                    .count(),
                1
            );
        }
    }
    assert_eq!(
        rows(
            address,
            fixture,
            "SELECT DISTINCT ?n WHERE { ?s <http://example.test/number> ?n }"
        )
        .len(),
        4
    );
    database.assert_encrypted_sessions();
    drop(server);
    assert_policy_scales(fixture, database);
}

fn assert_policy_scales(fixture: &Fixture, database: &Database) {
    fixture.write(
        "first.ttl",
        &NUMERIC_MAPPING.replace(
            "rr:subject <http://example.test/item>",
            "rr:subjectMap [rr:template \"http://example.test/id/{ordinal}\"]",
        ),
    );
    // Mapping id need not be a source key; preserve scale-distinct RDF triples
    // under the same integer subject even when policy disables text fallbacks.
    sql(database, "DELETE FROM items; ALTER TABLE items ADD COLUMN IF NOT EXISTS ordinal INTEGER; INSERT INTO items(id,ordinal,src,value) VALUES (1,1,1.0,'same'),(2,1,1.00,'same'),(3,1,1.0,'same'),(4,1,'NaN','denied')");
    let (mut command, address) = command_with_admission(
        fixture,
        database,
        None,
        &["--auth-subjects-env", "SF_POLICY_SUBJECTS"],
    );
    command.env("SF_POLICY_VALUE", "same").env("SF_POLICY_SUBJECTS", serde_json::json!({"schemaVersion":2,"subjects":[{
        "subjectRef":"numeric-reader", "credentialEnv":"SF_TLS_BEARER",
        "portableRows":[{"sourceIndex":0,"table":"items","column":"value","valueEnv":"SF_POLICY_VALUE"}]
    }]}).to_string());
    let (server, address) = start_command(fixture, command, address);
    let result = rows(
        address,
        fixture,
        "SELECT ?s ?o WHERE { ?s <http://example.test/edge> ?o }",
    );
    assert_eq!(result.len(), 2);
    for lexical in ["1.0", "1.00"] {
        assert_eq!(
            result
                .iter()
                .filter(|r| r["s"]["value"] == "http://example.test/id/1"
                    && r["o"]["value"] == format!("http://example.test/n/{lexical}"))
                .count(),
            1
        );
    }
    assert_eq!(
        rows(
            address,
            fixture,
            "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://example.test/edge> ?o }"
        )[0]["n"]["value"],
        "2"
    );
    database.assert_encrypted_sessions();
    drop(server);
    for (source, expected) in [
        ("SELECT ordinal, src FROM items WHERE value='same'", vec!["1.0", "1.00"]),
        ("SELECT ordinal, src::NUMERIC(30,2) AS src FROM items WHERE value='same' UNION ALL SELECT ordinal, src::NUMERIC(30,3) AS src FROM items WHERE value='same'", vec!["1.00", "1.000"]),
        ("SELECT ordinal, src + 0.000 AS src FROM items WHERE value='same'", vec!["1.000"]),
    ] {
        // Authored query literals require explicit datatype at semantic startup;
        // live NUMERIC result metadata supplies execution identity, not admission.
        fixture.write("first.ttl", &NUMERIC_MAPPING.replace("rr:tableName \"items\"", &format!("rr:sqlQuery \"{source}\"")).replace("rr:subject <http://example.test/item>", "rr:subjectMap [rr:template \"http://example.test/id/{ordinal}\"]").replace("rr:column \"src\"", "rr:column \"src\"; rr:datatype <http://www.w3.org/2001/XMLSchema#decimal>"));
        let (server, address) = start(fixture, database);
        let result = rows(address, fixture, "SELECT DISTINCT ?o WHERE { ?s <http://example.test/edge> ?o }");
        assert_eq!(result.len(), expected.len(), "{source}");
        for lexical in expected {
            let iri = format!("http://example.test/n/{lexical}");
            assert_eq!(result.iter().filter(|r| r["o"]["value"] == iri).count(), 1, "{source}: {lexical}");
            assert_eq!(rows(address, fixture, &format!("SELECT ?s WHERE {{ ?s <{EDGE}> <{iri}> }}")).len(), 1);
        }
        database.assert_encrypted_sessions();
        drop(server);
    }
}

#[test]
#[ignore = "requires owned pinned PostgreSQL TLS fixture"]
fn postgres_numeric_reference_keys_preserve_scale() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, true);
    assert_references(&fixture, &database);
}

fn assert_references(fixture: &Fixture, database: &Database) {
    sql(database, "CREATE TABLE numeric_child(src NUMERIC, fk INTEGER); CREATE TABLE numeric_parent(k INTEGER, label TEXT); GRANT SELECT ON numeric_child,numeric_parent TO sf_tls; INSERT INTO numeric_child VALUES (1.0,1),(1.00,1),(1.0,1),(NULL,1); INSERT INTO numeric_parent VALUES (1,'target'),(1,'target')");
    fixture.write(
        "ontology.ttl",
        "<http://example.test/edge> a <http://www.w3.org/2002/07/owl#ObjectProperty> .",
    );
    fixture.write("first.ttl", r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#parent> rr:logicalTable [rr:tableName "numeric_parent"]; rr:subjectMap [rr:template "http://example.test/n/{label}"].
<#child> rr:logicalTable [rr:tableName "numeric_child"]; rr:subjectMap [rr:template "http://example.test/n/{src}"];
 rr:predicateObjectMap [rr:predicate <http://example.test/edge>; rr:objectMap [rr:parentTriplesMap <#parent>; rr:joinCondition [rr:child "fk"; rr:parent "k"]]]."#);
    let (server, address) = start(fixture, database);
    for modifier in ["", "DISTINCT "] {
        let result = rows(
            address,
            fixture,
            &format!("SELECT {modifier}?s ?o WHERE {{ ?s <{EDGE}> ?o }}"),
        );
        assert_eq!(result.len(), 2, "Ref D1 scale identity");
        for lexical in ["1.0", "1.00"] {
            assert_eq!(
                result
                    .iter()
                    .filter(
                        |r| r["s"]["value"] == format!("http://example.test/n/{lexical}")
                            && r["o"]["value"] == "http://example.test/n/target"
                    )
                    .count(),
                1
            );
        }
    }
    assert_eq!(
        rows(
            address,
            fixture,
            &format!("SELECT (COUNT(*) AS ?n) WHERE {{ ?s <{EDGE}> ?o }}")
        )[0]["n"]["value"],
        "2"
    );
    for lexical in ["1.0", "1.00"] {
        assert_eq!(
            rows(
                address,
                fixture,
                &format!("SELECT ?o WHERE {{ <http://example.test/n/{lexical}> <{EDGE}> ?o }}")
            )
            .len(),
            1
        );
    }
    database.assert_encrypted_sessions();
    drop(server);
}

#[test]
#[ignore = "requires owned pinned PostgreSQL TLS fixture"]
fn postgres_numeric_hidden_invalid_terms_fail() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, true);
    sql(
        &database,
        "ALTER TABLE items ADD COLUMN id INTEGER; ALTER TABLE items ADD COLUMN src NUMERIC",
    );
    assert_invalid(&fixture, &database);
}

pub(super) fn assert_invalid(fixture: &Fixture, database: &Database) {
    fixture.write("first.ttl", NUMERIC_MAPPING);
    fixture.write("ontology.ttl", "<http://example.test/edge> a <http://www.w3.org/2002/07/owl#ObjectProperty> . <http://example.test/number> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .");
    for invalid in ["NaN", "Infinity", "-Infinity"] {
        sql(
            database,
            &format!(
                "DELETE FROM items; INSERT INTO items(id,src,value) VALUES (1,'{invalid}','same')"
            ),
        );
        let (server, address) = start(fixture, database);
        for query in [
            "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://example.test/edge> ?o }",
            "SELECT ?o WHERE { ?s <http://example.test/edge> ?o }",
            "SELECT ?s WHERE { ?s <http://example.test/edge> ?o }",
            "ASK { ?s <http://example.test/edge> ?o }",
            "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://example.test/number> ?o }",
            "SELECT DISTINCT ?s WHERE { VALUES ?s { <http://example.test/item> } OPTIONAL { ?s <http://example.test/edge> ?o } }",
        ] {
            eprintln!("invalid numeric {invalid}: {query}");
            let response = stop_matrix::wire(cancellation::begin(address, query, &fixture.token));
            stop_matrix::assert_no_complete_union_success(&response);
            let until = Instant::now() + Duration::from_secs(3);
            loop {
                let (status, _) = request(address, "ASK {}", Some(&fixture.token)).unwrap();
                if status == 200 { break; }
                assert_eq!(status, 503);
                assert!(Instant::now() < until, "numeric failure did not release cap-one admission");
                thread::sleep(Duration::from_millis(10));
            }
        }
        drop(server);
        sql(database, "UPDATE items SET value='denied'; INSERT INTO items(id,src,value) VALUES (2,1.00,'same')");
        let (mut command, address) = command_with_admission(
            fixture,
            database,
            None,
            &["--auth-subjects-env", "SF_POLICY_SUBJECTS"],
        );
        command.env("SF_POLICY_VALUE", "same").env("SF_POLICY_SUBJECTS", serde_json::json!({"schemaVersion":2,"subjects":[{
            "subjectRef":"numeric-reader", "credentialEnv":"SF_TLS_BEARER",
            "portableRows":[{"sourceIndex":0,"table":"items","column":"value","valueEnv":"SF_POLICY_VALUE"}]
        }]}).to_string());
        let (server, address) = start_command(fixture, command, address);
        for pattern in [
            "?s <http://example.test/edge> ?o",
            "?s <http://example.test/edge> <http://example.test/n/1.00>",
            "VALUES ?s { <http://example.test/item> } FILTER EXISTS { ?s <http://example.test/edge> ?o }",
            "VALUES ?s { <http://example.test/item> } OPTIONAL { ?s <http://example.test/edge> ?o }",
        ] {
            let query = format!("SELECT (COUNT(*) AS ?n) WHERE {{ {pattern} }}");
            assert_eq!(rows(address, fixture, &query)[0]["n"]["value"], "1", "{invalid}: {query}");
        }
        assert_eq!(
            rows(
                address,
                fixture,
                "SELECT ?o WHERE { ?s <http://example.test/edge> ?o }"
            )[0]["o"]["value"],
            "http://example.test/n/1.00"
        );
        database.assert_encrypted_sessions();
        drop(server);
    }
}

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
