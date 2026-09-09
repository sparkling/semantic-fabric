//! Floating identity follows Rust's native decoder, not SQL numeric equality.
use super::*;

const NATURAL: &str = "http://example.test/natural";
const EXPLICIT: &str = "http://example.test/explicit";

#[test]
#[ignore = "requires owned pinned PostgreSQL TLS fixture"]
fn postgres_floating_literal_identity_is_exact() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, true);
    assert_identity(&fixture, &database);
}

pub(super) fn assert_identity(fixture: &Fixture, database: &Database) {
    sql(database, "DELETE FROM items; ALTER TABLE items ADD COLUMN float_narrow REAL; ALTER TABLE items ADD COLUMN float_wide DOUBLE PRECISION");
    // Deliberately degrade PostgreSQL text formatting. Wire-owned keys must not
    // depend on this role/session option; only this disposable fixture is changed.
    sql(database, "ALTER ROLE sf_tls SET extra_float_digits = -15");
    let narrow = [
        0.0_f32,
        -0.0,
        1.1,
        -1.1,
        33554432.0,
        f32::MIN_POSITIVE,
        f32::from_bits(1),
        f32::from_bits(0x007fffff),
        f32::MAX,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
    ]
    .map(|v| v.to_string());
    let wide = [
        0.0_f64,
        -0.0,
        1.1,
        -1.1,
        1.0 + 1.0 / 131_072.0, // Exactly 1.00000762939453125: shortest-decimal midpoint.
        1e23,
        f64::MIN_POSITIVE,
        f64::from_bits(1),
        f64::from_bits(0x000fffffffffffff),
        f64::MAX,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
    ]
    .map(|v| v.to_string());
    for (column, values) in [
        ("float_narrow", narrow.as_slice()),
        ("float_wide", wide.as_slice()),
    ] {
        fixture.write("first.ttl", &r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
        <#m> rr:logicalTable [rr:tableName "items"]; rr:subject <http://example.test/item>;
          rr:predicateObjectMap [rr:predicate <http://example.test/natural>; rr:objectMap [rr:column "FLOAT_COLUMN"]];
          rr:predicateObjectMap [rr:predicate <http://example.test/explicit>; rr:objectMap [rr:column "FLOAT_COLUMN"; rr:datatype <http://www.w3.org/2001/XMLSchema#double>]];
          rr:predicateObjectMap [rr:predicate <http://example.test/raw>; rr:objectMap [rr:column "FLOAT_COLUMN"; rr:datatype <http://www.w3.org/2001/XMLSchema#string>]];
          rr:predicateObjectMap [rr:predicate <http://example.test/edge>; rr:objectMap [rr:template "http://example.test/n/{FLOAT_COLUMN}"]];
          rr:predicateObjectMap [rr:predicate <http://example.test/lexical>; rr:objectMap [rr:column "value"; rr:datatype <http://www.w3.org/2001/XMLSchema#double>]]."#.replace("FLOAT_COLUMN", column));
        fixture.write("ontology.ttl", &(["natural", "explicit", "raw", "lexical"].map(|name| format!("<http://example.test/{name}> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .")).join("\n") + "\n<http://example.test/edge> a <http://www.w3.org/2002/07/owl#ObjectProperty> ."));
        let expected: BTreeSet<_> = values.iter().map(|v| canonical(v)).collect();
        let inserts = values
            .iter()
            .chain(&values[..1])
            .map(|v| format!("('{v}','{}')", canonical(v)))
            .collect::<Vec<_>>()
            .join(",");
        sql(
            database,
            &format!(
                "DELETE FROM items; INSERT INTO items({column},value) VALUES {inserts},(NULL,'')"
            ),
        );
        let (server, address) = start(fixture, database);
        for pattern in [
            format!("?s <{NATURAL}> ?o"),
            format!("?s <{NATURAL}> ?o . ?t <{EXPLICIT}> ?o"),
            format!("?s <{NATURAL}> ?o . ?t <http://example.test/lexical> ?o"),
            format!("?s <{NATURAL}> ?o FILTER EXISTS {{ ?t <{EXPLICIT}> ?o }}"),
            format!("?s <{NATURAL}> ?o OPTIONAL {{ ?t <http://example.test/lexical> ?o }}"),
            format!("{{ SELECT ?o WHERE {{ ?s <{NATURAL}> ?o }} }} ?t <{EXPLICIT}> ?o"),
        ] {
            for modifier in ["", "DISTINCT "] {
                let query = format!("SELECT {modifier}?o WHERE {{ {pattern} }}");
                if !modifier.is_empty() && pattern.contains("OPTIONAL") {
                    assert_eq!(
                        request(address, &query, Some(&fixture.token)).unwrap().0,
                        501,
                        "existing source-sized OPTIONAL DISTINCT profile remains closed"
                    );
                    continue;
                }
                let actual = complete_rows(address, fixture, &query);
                assert_eq!(
                    actual.len(),
                    expected.len(),
                    "{column}: {query}: {actual:?}"
                );
                assert_eq!(
                    actual
                        .iter()
                        .map(|r| r["o"]["value"].as_str().unwrap().to_owned())
                        .collect::<BTreeSet<_>>(),
                    expected,
                    "{column}: {query}"
                );
                for row in actual {
                    assert_eq!(
                        row["o"]["datatype"],
                        "http://www.w3.org/2001/XMLSchema#double"
                    );
                }
            }
        }
        assert_eq!(
            complete_rows(
                address,
                fixture,
                &format!("SELECT (COUNT(*) AS ?n) WHERE {{ ?s <{NATURAL}> ?o }}")
            )[0]["n"]["value"],
            expected.len().to_string()
        );
        let raw = complete_rows(
            address,
            fixture,
            "SELECT ?o WHERE { ?s <http://example.test/raw> ?o }",
        );
        assert_eq!(raw.len(), values.len());
        assert_eq!(
            raw.iter()
                .map(|r| r["o"]["value"].as_str().unwrap().to_owned())
                .collect::<BTreeSet<_>>(),
            values.iter().cloned().collect()
        );
        for lexical in ["0.0E0", "-0.0E0", "1.1E0", "NaN", "1.10E0"] {
            let literal = format!("\"{lexical}\"^^<http://www.w3.org/2001/XMLSchema#double>");
            for pattern in [
                format!("?s <{NATURAL}> {literal}"),
                format!("?s <{NATURAL}> ?o FILTER(sameTerm(?o, {literal}))"),
            ] {
                let query = format!("SELECT ?s WHERE {{ {pattern} }}");
                assert_eq!(
                    complete_rows(address, fixture, &query).len(),
                    usize::from(expected.contains(lexical)),
                    "{column}: {query}"
                );
            }
        }
        for value in ["0", "-0", "1.1", "inf", "NaN"] {
            let query = format!("SELECT ?s WHERE {{ ?s <http://example.test/edge> <http://example.test/n/{value}> }}");
            assert_eq!(
                complete_rows(address, fixture, &query).len(),
                1,
                "{column}: {query}"
            );
        }
        database.assert_encrypted_sessions();
        drop(server);
    }
    assert_cross_width(fixture, database);
    sql(database, "ALTER ROLE sf_tls RESET extra_float_digits");
    sql(database, "DELETE FROM items; ALTER TABLE items DROP COLUMN float_narrow; ALTER TABLE items DROP COLUMN float_wide");
}

fn assert_cross_width(fixture: &Fixture, database: &Database) {
    fixture.write("first.ttl", r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
      <#n> rr:logicalTable [rr:tableName "items"]; rr:subject <http://example.test/item>;
      rr:predicateObjectMap [rr:predicate <http://example.test/natural>; rr:objectMap [rr:column "float_narrow"]];
      rr:predicateObjectMap [rr:predicate <http://example.test/explicit>; rr:objectMap [rr:column "float_wide"; rr:datatype <http://www.w3.org/2001/XMLSchema#double>]]."#);
    sql(database, "DELETE FROM items; INSERT INTO items(float_narrow,float_wide,value) VALUES (1.1,1.1,''),('0','0',''),('-0','-0',''),(NULL,1.100000023841858,'')");
    let (server, address) = start(fixture, database);
    for pattern in [
        format!("?s <{NATURAL}> ?o . ?t <{EXPLICIT}> ?o"),
        format!("?s <{NATURAL}> ?o FILTER EXISTS {{ ?t <{EXPLICIT}> ?o }}"),
        format!("?s <{NATURAL}> ?o . ?t <{EXPLICIT}> ?v FILTER(sameTerm(?o, ?v))"),
    ] {
        let query = format!("SELECT ?o WHERE {{ {pattern} }}");
        let values = complete_rows(address, fixture, &query);
        assert_eq!(values.len(), 3, "FLOAT4/FLOAT8: {query}: {values:?}");
        assert_eq!(
            values
                .iter()
                .map(|r| r["o"]["value"].as_str().unwrap())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["1.1E0", "0.0E0", "-0.0E0"])
        );
    }
    // Same-width pooled pass-through keeps its decoder; separate public UNION
    // arms can also return unlike widths without a native SQL promotion.
    let same = format!("{{ ?s <{NATURAL}> ?o }} UNION {{ ?s <{NATURAL}> ?o }}");
    assert_eq!(
        complete_rows(
            address,
            fixture,
            &format!("SELECT ?o WHERE {{ {{ SELECT ?o WHERE {{ {same} }} }} }}")
        )
        .len(),
        6
    );
    let mixed = format!("{{ ?s <{NATURAL}> ?o }} UNION {{ ?s <{EXPLICIT}> ?o }}");
    let values = complete_rows(address, fixture, &format!("SELECT ?o WHERE {{ {mixed} }}"));
    assert_eq!(values.len(), 7);
    assert_eq!(
        values.iter().filter(|r| r["o"]["value"] == "1.1E0").count(),
        2
    );
    database.assert_encrypted_sessions();
    drop(server);
}

fn canonical(value: &str) -> String {
    let mut result = String::new();
    sf_core::datatype::canonical_lexical(
        value,
        sf_core::datatype::XsdTypeCode::Double,
        &mut result,
    )
    .unwrap();
    result
}

fn complete_rows(address: SocketAddr, fixture: &Fixture, query: &str) -> Vec<serde_json::Value> {
    let wire = stop_matrix::wire(cancellation::begin(address, query, &fixture.token));
    assert!(
        wire.starts_with(b"HTTP/1.1 200") && wire.ends_with(b"0\r\n\r\n"),
        "{query}: floating query must produce a complete successful response: {}",
        String::from_utf8_lossy(&wire)
    );
    let (_, body) = decode_response(wire);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    json["results"]["bindings"].as_array().unwrap().clone()
}
