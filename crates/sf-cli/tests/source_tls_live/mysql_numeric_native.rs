//! Native scale, signedness and terminal source-validation boundaries.
use super::*;

#[test]
#[ignore = "requires owned pinned MySQL TLS fixture"]
fn mysql_integer_validation_survives_null_and_policy() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, false);
    mappings(&fixture);
    sql(&database,"CREATE TABLE sf_numeric_items(id BIGINT PRIMARY KEY, v BIGINT UNSIGNED, visible VARCHAR(32))");
    assert_source_validation(&fixture, &database);
}

pub(super) fn assert_all(fixture: &Fixture, database: &Database) {
    for (storage, values) in [
        (
            "DECIMAL(65,30)",
            vec![
                format!("0.{}", "0".repeat(30)),
                format!("1.{}", "0".repeat(30)),
                format!("-1.{}", "0".repeat(30)),
                format!("0.{}1", "0".repeat(29)),
                format!("{}.{}", "9".repeat(35), "9".repeat(30)),
            ],
        ),
        (
            "DECIMAL(8,2) UNSIGNED ZEROFILL",
            ["0.00", "1.00", "10.00", "999999.99"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        ),
        (
            "BIGINT",
            [
                "0",
                "1",
                "-1",
                "2",
                "10",
                "128",
                "9223372036854775807",
                "-9223372036854775808",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        ),
        (
            "BIGINT(20) UNSIGNED ZEROFILL",
            ["0", "1", "10", "9223372036854775807"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        ),
        (
            "YEAR",
            ["0", "1901", "2000", "2155"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        ),
    ] {
        eprintln!("MySQL exact numeric storage: {storage}");
        sql(
            database,
            &format!(
                "DELETE FROM sf_numeric_items; ALTER TABLE sf_numeric_items MODIFY v {storage}"
            ),
        );
        // Native YEAR interprets the string '0' as 2000 but numeric 0 as 0000.
        // These fixture-owned, valid numeric lexicals must seed numeric values.
        let inserts = values
            .iter()
            .enumerate()
            .map(|(id, v)| format!("({id},{v},'same')"))
            .collect::<Vec<_>>()
            .join(",");
        sql(
            database,
            &format!("INSERT INTO sf_numeric_items VALUES {inserts},(9999,NULL,'same')"),
        );
        let (server, address) = start(fixture, database);
        assert_values(address, fixture, &values);
        database.assert_encrypted_sessions();
        drop(server);
    }
    assert_source_validation(fixture, database);
}

pub(super) fn assert_source_validation(fixture: &Fixture, database: &Database) {
    sql(database,"DELETE FROM sf_numeric_items; ALTER TABLE sf_numeric_items MODIFY v BIGINT UNSIGNED; INSERT INTO sf_numeric_items VALUES(0,1,'same'),(1,18446744073709551615,'denied')");
    sql(
        database,
        "ALTER TABLE sf_numeric_items ADD COLUMN nullable_decimal TEXT NULL",
    );
    let mapping = std::fs::read_to_string(fixture.root.join("first.ttl")).unwrap();
    let nullable = [("nullable","decimal"),("nullable_double","double")].into_iter().map(|(name,kind)|format!("@prefix rr: <http://www.w3.org/ns/r2rml#> . <#{name}> rr:logicalTable [rr:tableName \"sf_numeric_items\"]; rr:subjectMap [rr:template \"http://example.test/numeric/{{id}}\"]; rr:predicateObjectMap [rr:predicate <http://example.test/{name}>; rr:objectMap [rr:column \"nullable_decimal\"; rr:datatype <{XSD}{kind}>]].")).collect::<Vec<_>>().join("\n");
    fixture.write("first.ttl", &format!("{mapping}\n{nullable}"));
    let ontology = std::fs::read_to_string(fixture.root.join("ontology.ttl")).unwrap();
    fixture.write("ontology.ttl",&format!("{ontology}\n <http://example.test/nullable> a <http://www.w3.org/2002/07/owl#DatatypeProperty>. <http://example.test/nullable_double> a <http://www.w3.org/2002/07/owl#DatatypeProperty>."));
    let (server, address) = start(fixture, database);
    // A different datatype consumes the raw u64 lexical; matching/natural
    // integer still has the Rust constructor's i64 validation obligation.
    assert_rows(address,fixture,"SELECT ?s WHERE { ?s <http://example.test/unsignedLong> ?o FILTER(?o > 9223372036854775807) }",&BTreeSet::from(["http://example.test/numeric/1".into()]));
    for expression in [
        "?o > 0",
        "?o > 0e0",
        "?o = \"NaN\"^^<http://www.w3.org/2001/XMLSchema#double>",
        "\"NaN\"^^<http://www.w3.org/2001/XMLSchema#double> = ?o",
        "?o = \"inf\"^^<http://www.w3.org/2001/XMLSchema#float>",
        "\"inf\"^^<http://www.w3.org/2001/XMLSchema#float> = ?o",
        "?o = \"inf\"^^<http://www.w3.org/2001/XMLSchema#decimal>",
        "\"inf\"^^<http://www.w3.org/2001/XMLSchema#decimal> = ?o",
        "?o = \"foo\"",
        "\"foo\" = ?o",
    ] {
        let query = format!(
            "SELECT ?s WHERE {{ ?s <http://example.test/integer> ?o FILTER({expression}) }}"
        );
        let response = stop_matrix::wire(cancellation::begin(address, &query, &fixture.token));
        assert!(
            response.starts_with(b"HTTP/1.1 200") || response.starts_with(b"HTTP/1.1 500"),
            "query rejected before source validation: {query}: {}",
            String::from_utf8_lossy(&response)
        );
        assert!(
            !response.ends_with(b"0\r\n\r\n"),
            "source validation was skipped: {query}: {}",
            String::from_utf8_lossy(&response)
        );
        stop_matrix::assert_no_complete_union_success(&response);
        recovered(address, fixture);
        assert_eq!(
            rows(
                address,
                fixture,
                "SELECT ?s WHERE { ?s <http://example.test/unsignedLong> ?o FILTER(?o = 1) }"
            )
            .len(),
            1
        );
    }
    for expression in ["?o = ?r", "?r = ?o"] {
        // A known unbound variable lowers to a constant expression error
        // before typed operands; no source value is needed to prove no match.
        let static_empty=format!("SELECT ?s WHERE {{ VALUES ?r {{ UNDEF }} ?s <http://example.test/integer> ?o FILTER({expression}) }}");
        assert!(rows(address, fixture, &static_empty).is_empty());
        for predicate in ["nullable", "nullable_double"] {
            let query=format!("SELECT ?s WHERE {{ ?s <http://example.test/integer> ?o OPTIONAL {{ ?s <http://example.test/{predicate}> ?r }} FILTER({expression}) }}");
            let response = stop_matrix::wire(cancellation::begin(address, &query, &fixture.token));
            assert!(
                response.starts_with(b"HTTP/1.1 200") || response.starts_with(b"HTTP/1.1 500"),
                "query rejected: {}",
                String::from_utf8_lossy(&response)
            );
            assert!(
                !response.ends_with(b"0\r\n\r\n"),
                "source validation was skipped: {query}: {}",
                String::from_utf8_lossy(&response)
            );
            stop_matrix::assert_no_complete_union_success(&response);
            recovered(address, fixture);
        }
    }
    database.assert_encrypted_sessions();
    drop(server);
    let (server, address) = policy_server(fixture, database);
    for pattern in [
        "?s <http://example.test/integer> ?o FILTER(?o > 0)",
        "VALUES ?s { <http://example.test/numeric/0> <http://example.test/numeric/1> } FILTER EXISTS { ?s <http://example.test/integer> ?o FILTER(?o > 0) }",
        "VALUES ?s { <http://example.test/numeric/0> <http://example.test/numeric/1> } OPTIONAL { ?s <http://example.test/integer> ?o FILTER(?o > 0) }",
    ] {
        let query=format!("SELECT ?s ?o WHERE {{ {pattern} }}");
        let rows=rows(address,fixture,&query);
        assert_eq!(rows.len(),if pattern.contains("OPTIONAL"){2}else{1},"{query}");
        assert!(rows.iter().filter(|r|r["s"]["value"]=="http://example.test/numeric/1").all(|r|r.get("o").is_none()));
    }
    database.assert_encrypted_sessions();
    drop(server);
}

fn recovered(address: SocketAddr, fixture: &Fixture) {
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        let (status, _) = request(address, "ASK {}", Some(&fixture.token)).unwrap();
        if status == 200 {
            break;
        }
        assert_eq!(
            status, 503,
            "only transient retained request capacity is retryable"
        );
        assert!(Instant::now() < until, "cap-one cleanup failed to recover");
        thread::sleep(Duration::from_millis(25));
    }
}
