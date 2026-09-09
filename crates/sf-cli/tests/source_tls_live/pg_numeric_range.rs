//! Natural decimal reconstruction uses the full finite native source range.
use super::*;

#[test]
#[ignore = "requires owned pinned PostgreSQL TLS fixture"]
fn postgres_natural_decimal_range_is_exact() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, true);
    assert_range(&fixture, &database);
}

#[test]
#[ignore = "requires owned pinned MySQL TLS fixture"]
fn mysql_natural_decimal_range_is_exact() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, false);
    sql(
        &database,
        "ALTER TABLE items ADD COLUMN id INTEGER; ALTER TABLE items ADD COLUMN src DECIMAL(65,30)",
    );
    assert_mysql(&fixture, &database);
}

pub(super) fn assert_mysql(fixture: &Fixture, database: &Database) {
    fixture.write("first.ttl", NUMERIC_MAPPING);
    fixture.write("ontology.ttl", "<http://example.test/edge> a <http://www.w3.org/2002/07/owl#ObjectProperty> . <http://example.test/number> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .");
    sql(database, "DELETE FROM items; ALTER TABLE items MODIFY src DECIMAL(65,30); INSERT INTO items(id,src,value) VALUES (1,CONCAT(REPEAT('1',35),'.',REPEAT('2',30)),'same'),(2,CONCAT(REPEAT('1',35),'.',REPEAT('2',30)),'same'),(3,-0.000,'same'),(4,NULL,'same')");
    let (server, address) = start(fixture, database);
    for modifier in ["", "DISTINCT "] {
        let result = rows(
            address,
            fixture,
            &format!("SELECT {modifier}?n WHERE {{ ?s <http://example.test/number> ?n }}"),
        );
        assert_eq!(result.len(), 2);
        let mut actual = BTreeSet::new();
        for row in result {
            assert_eq!(
                row["n"]["datatype"],
                "http://www.w3.org/2001/XMLSchema#decimal"
            );
            actual.insert(row["n"]["value"].as_str().unwrap().to_owned());
        }
        assert_eq!(
            actual,
            ["0".into(), format!("{}.{}", "1".repeat(35), "2".repeat(30))]
                .into_iter()
                .collect()
        );
    }
    database.assert_encrypted_sessions();
    drop(server);
}

pub(super) fn assert_range(fixture: &Fixture, database: &Database) {
    fixture.write("first.ttl", NUMERIC_MAPPING);
    fixture.write("ontology.ttl", "<http://example.test/edge> a <http://www.w3.org/2002/07/owl#ObjectProperty> . <http://example.test/number> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .");
    sql(database, "DELETE FROM items; ALTER TABLE items ADD COLUMN IF NOT EXISTS id INTEGER; ALTER TABLE items ADD COLUMN IF NOT EXISTS src NUMERIC; ALTER TABLE items ALTER COLUMN src TYPE NUMERIC USING src::NUMERIC; INSERT INTO items(id,src,value) VALUES (1,(repeat('1',131072) || '.00')::NUMERIC,'same'),(2,(repeat('1',131072) || '.000')::NUMERIC,'same'),(3,('0.' || repeat('0',16382) || '1')::NUMERIC,'same'),(4,-0.000,'same'),(5,170141183460469231731.687303715884105728,'same'),(6,NULL,'same')");
    let expected: BTreeSet<String> = [
        "1".repeat(131_072),
        format!("0.{}1", "0".repeat(16_382)),
        "0".into(),
        "170141183460469231731.687303715884105728".into(),
    ]
    .into_iter()
    .collect();
    let (server, address) = start(fixture, database);
    for modifier in ["", "DISTINCT "] {
        let query = format!("SELECT {modifier}?n WHERE {{ ?s <http://example.test/number> ?n }}");
        let result = bounded_rows(address, fixture, &query);
        assert_eq!(result.len(), expected.len());
        let actual: BTreeSet<String> = result
            .iter()
            .map(|row| {
                assert_eq!(row["n"]["type"], "literal");
                assert_eq!(
                    row["n"]["datatype"],
                    "http://www.w3.org/2001/XMLSchema#decimal"
                );
                row["n"]["value"].as_str().unwrap().to_owned()
            })
            .collect();
        assert_eq!(actual, expected);
    }
    assert_eq!(
        rows(
            address,
            fixture,
            "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://example.test/number> ?n }"
        )[0]["n"]["value"],
        "4"
    );
    let (status, body) = request(
        address,
        "ASK { ?s <http://example.test/number> ?n }",
        Some(&fixture.token),
    )
    .unwrap();
    assert_eq!(status, 200);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()["boolean"],
        true
    );
    drop(server);

    // The same raw cell still makes scale-distinct IRIs and canonical literals.
    fixture.write(
        "first.ttl",
        &NUMERIC_MAPPING.replace(
            "rr:subject <http://example.test/item>",
            "rr:subjectMap [rr:template \"http://example.test/n/{src}\"]",
        ),
    );
    let (server, address) = start(fixture, database);
    let result = bounded_rows(
        address,
        fixture,
        "SELECT ?s ?n WHERE { ?s <http://example.test/number> ?n }",
    );
    assert_eq!(result.len(), 5);
    for scale in ["00", "000"] {
        let iri = format!("http://example.test/n/{}.{scale}", "1".repeat(131_072));
        let matches: Vec<_> = result
            .iter()
            .filter(|row| row["s"]["value"] == iri)
            .collect();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["n"]["value"], "1".repeat(131_072));
    }
    drop(server);

    let (mut command, address) = command(fixture, database, None);
    command.args(["--max-serialized-bytes", "1024"]);
    let (server, address) = start_command(fixture, command, address);
    let response = stop_matrix::wire(cancellation::begin(
        address,
        "SELECT ?n WHERE { ?s <http://example.test/number> ?n }",
        &fixture.token,
    ));
    assert!(!response.is_empty());
    stop_matrix::assert_no_complete_union_success(&response);
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        let (status, body) = request(address, "ASK {}", Some(&fixture.token)).unwrap();
        if status == 200 {
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&body).unwrap()["boolean"],
                true
            );
            break;
        }
        assert_eq!(status, 503);
        assert!(
            Instant::now() < until,
            "large result failure did not release cap-one admission"
        );
        thread::sleep(Duration::from_millis(10));
    }
    database.assert_encrypted_sessions();
    drop(server);
}

fn bounded_rows(address: SocketAddr, fixture: &Fixture, query: &str) -> Vec<serde_json::Value> {
    let (status, body) = request_format_bounded(
        address,
        query,
        Some(&fixture.token),
        "application/sparql-results+json",
        700_000,
    )
    .unwrap();
    assert_eq!(status, 200, "natural decimal response status");
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
    result["results"]["bindings"].as_array().unwrap().clone()
}
