//! Native natural decimal identity is distinct from numeric value equality.
use super::*;

const NUMBER: &str = "http://example.test/number";

#[test]
#[ignore = "requires owned pinned PostgreSQL/MySQL TLS fixtures"]
fn native_natural_decimal_identity_is_exact() {
    for postgres in [true, false] {
        let fixture = Fixture::new();
        let database = Database::start(&fixture, postgres);
        sql(
            &database,
            if postgres {
                "ALTER TABLE items ADD COLUMN id INTEGER; ALTER TABLE items ADD COLUMN src NUMERIC"
            } else {
                "ALTER TABLE items ADD COLUMN id INTEGER; ALTER TABLE items ADD COLUMN src DECIMAL(30,6)"
            },
        );
        assert_identity(&fixture, &database);
    }
}

pub(super) fn assert_identity(fixture: &Fixture, database: &Database) {
    fixture.write("first.ttl", NUMERIC_MAPPING);
    fixture.write("ontology.ttl", "<http://example.test/edge> a <http://www.w3.org/2002/07/owl#ObjectProperty> . <http://example.test/number> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .");
    sql(database, "DELETE FROM items; INSERT INTO items(id,src,value) VALUES (1,1.0,'same'),(2,1.00,'same'),(3,1.0,'same'),(4,-0.000,'same'),(5,-12.3400,'same'),(6,NULL,'same')");
    let (server, address) = start(fixture, database);
    for (lexical, expected) in [
        ("1", 1),
        ("1.0", 0),
        ("+1", 0),
        ("01", 0),
        ("0", 1),
        ("-0", 0),
        ("-12.34", 1),
        ("-12.340", 0),
    ] {
        let literal = format!("\"{lexical}\"^^<http://www.w3.org/2001/XMLSchema#decimal>");
        for pattern in [
            format!("?s <{NUMBER}> {literal}"),
            format!("?s <{NUMBER}> ?n FILTER(sameTerm(?n, {literal}))"),
            format!("?s <{NUMBER}> ?n FILTER(sameTerm({literal}, ?n))"),
        ] {
            let query = format!("SELECT ?s WHERE {{ {pattern} }}");
            assert_eq!(rows(address, fixture, &query).len(), expected, "{query}");
        }
        let query =
            format!("SELECT ?n WHERE {{ ?s <{NUMBER}> ?n FILTER(!sameTerm(?n, {literal})) }}");
        assert_eq!(
            rows(address, fixture, &query).len(),
            3 - expected,
            "{query}"
        );
    }
    for literal in [
        "\"1\"",
        "\"1\"@en",
        "\"1\"^^<http://www.w3.org/2001/XMLSchema#integer>",
        "\"1\"^^<http://example.test/type>",
    ] {
        for pattern in [
            format!("?s <{NUMBER}> {literal}"),
            format!("?s <{NUMBER}> ?n FILTER(sameTerm(?n, {literal}))"),
        ] {
            let query = format!("SELECT ?s WHERE {{ {pattern} }}");
            assert!(rows(address, fixture, &query).is_empty(), "{query}");
        }
    }
    for literal in [
        "1",
        "1.0",
        "\"1.00\"^^<http://www.w3.org/2001/XMLSchema#decimal>",
    ] {
        let query = format!("SELECT ?n WHERE {{ ?s <{NUMBER}> ?n FILTER(?n = {literal}) }}");
        assert_eq!(
            rows(address, fixture, &query).len(),
            1,
            "numeric value equality differs from identity: {query}"
        );
        for expression in [format!("?n != {literal}"), format!("!({literal} = ?n)")] {
            assert_eq!(
                rows(
                    address,
                    fixture,
                    &format!("SELECT ?n WHERE {{ ?s <{NUMBER}> ?n FILTER({expression}) }}")
                )
                .len(),
                2
            );
        }
    }
    for literal in [
        "\"1.0\"^^<http://www.w3.org/2001/XMLSchema#integer>",
        "\"NaN\"^^<http://www.w3.org/2001/XMLSchema#decimal>",
        "\"1e0\"^^<http://www.w3.org/2001/XMLSchema#decimal>",
    ] {
        for expression in [
            format!("?n = {literal}"),
            format!("?n != {literal}"),
            format!("!(?n = {literal})"),
        ] {
            let query = format!("SELECT ?n WHERE {{ ?s <{NUMBER}> ?n FILTER({expression}) }}");
            assert!(
                rows(address, fixture, &query).is_empty(),
                "invalid numeric lexical: {query}"
            );
        }
    }
    for pattern in [
        format!("{{ SELECT ?n WHERE {{ ?s <{NUMBER}> ?n }} }} FILTER(sameTerm(?n, \"1.0\"^^<http://www.w3.org/2001/XMLSchema#decimal>))"),
        format!("VALUES ?s {{ <http://example.test/item> }} OPTIONAL {{ ?s <{NUMBER}> ?n FILTER(sameTerm(?n, \"1.0\"^^<http://www.w3.org/2001/XMLSchema#decimal>)) }} FILTER(BOUND(?n))"),
    ] {
        assert!(rows(address, fixture, &format!("SELECT ?n WHERE {{ {pattern} }}")).is_empty());
    }
    database.assert_encrypted_sessions();
    drop(server);
    if database.source.starts_with("mysql:") {
        // This unsigned presentation modifier must not enter natural identity.
        sql(database, "DELETE FROM items; ALTER TABLE items MODIFY src DECIMAL(30,6) ZEROFILL; INSERT INTO items(id,src,value) VALUES (1,1.000000,'same'),(2,0.001230,'same'),(3,0,'same')");
        let (server, address) = start(fixture, database);
        for lexical in ["1", "0.00123", "0"] {
            for pattern in [format!("?s <{NUMBER}> \"{lexical}\"^^<http://www.w3.org/2001/XMLSchema#decimal>"), format!("?s <{NUMBER}> ?n FILTER(?n = \"+{lexical}\"^^<http://www.w3.org/2001/XMLSchema#decimal>)")] {
                let query = format!("SELECT ?s WHERE {{ {pattern} }}");
                assert_eq!(rows(address, fixture, &query).len(), 1, "ZEROFILL: {query}");
            }
        }
        drop(server);
    }
}

#[test]
#[ignore = "requires owned pinned PostgreSQL/MySQL TLS fixtures"]
fn explicit_natural_decimal_is_coherent() {
    for postgres in [true, false] {
        let fixture = Fixture::new();
        let database = Database::start(&fixture, postgres);
        sql(
            &database,
            if postgres {
                "ALTER TABLE items ADD COLUMN id INTEGER; ALTER TABLE items ADD COLUMN src NUMERIC"
            } else {
                "ALTER TABLE items ADD COLUMN id INTEGER; ALTER TABLE items ADD COLUMN src DECIMAL(30,6)"
            },
        );
        assert_explicit(&fixture, &database);
    }
}

pub(super) fn assert_explicit(fixture: &Fixture, database: &Database) {
    let mapping = NUMERIC_MAPPING.replace(
        "rr:column \"src\"",
        "rr:column \"src\"; rr:datatype <http://www.w3.org/2001/XMLSchema#decimal>",
    );
    fixture.write("first.ttl", &mapping);
    fixture.write("ontology.ttl", "<http://example.test/edge> a <http://www.w3.org/2002/07/owl#ObjectProperty> . <http://example.test/number> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .");
    sql(database, "DELETE FROM items; INSERT INTO items(id,src,value) VALUES (1,1.0,'same'),(2,1.00,'same'),(3,0.000,'same'),(4,NULL,'same')");
    let (server, address) = start(fixture, database);
    for modifier in ["", "DISTINCT "] {
        let query = format!("SELECT {modifier}?n WHERE {{ ?s <{NUMBER}> ?n }}");
        let actual = rows(address, fixture, &query);
        assert_eq!(actual.len(), 2, "explicit natural bag: {query}: {actual:?}");
        for lexical in ["0", "1"] {
            assert!(
                actual.iter().any(|row| row["n"]["value"] == lexical),
                "{actual:?}"
            );
        }
    }
    for (lexical, expected) in [("1", 1), ("1.0", 0), ("1.00", 0)] {
        let literal = format!("\"{lexical}\"^^<http://www.w3.org/2001/XMLSchema#decimal>");
        for pattern in [
            format!("?s <{NUMBER}> {literal}"),
            format!("?s <{NUMBER}> ?n FILTER(sameTerm(?n, {literal}))"),
        ] {
            let query = format!("SELECT ?s WHERE {{ {pattern} }}");
            assert_eq!(rows(address, fixture, &query).len(), expected, "{query}");
        }
    }
    assert_eq!(
        rows(
            address,
            fixture,
            &format!("SELECT (COUNT(*) AS ?n) WHERE {{ ?s <{NUMBER}> ?o }}")
        )[0]["n"]["value"],
        "2"
    );
    assert_eq!(
        rows(
            address,
            fixture,
            &format!("SELECT ?n WHERE {{ ?s <{NUMBER}> ?n FILTER(?n = 1.00) }}")
        )
        .len(),
        1
    );
    drop(server);
    assert_join_identity(fixture, database);
}

fn assert_join_identity(fixture: &Fixture, database: &Database) {
    fixture.write("first.ttl", r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
      <#m> rr:logicalTable [rr:tableName "items"]; rr:subject <http://example.test/item>;
        rr:predicateObjectMap [rr:predicate <http://example.test/natural>; rr:objectMap [rr:column "src"]];
        rr:predicateObjectMap [rr:predicate <http://example.test/explicit>; rr:objectMap [rr:column "src"; rr:datatype <http://www.w3.org/2001/XMLSchema#decimal>]];
        rr:predicateObjectMap [rr:predicate <http://example.test/lexical>; rr:objectMap [rr:column "value"; rr:datatype <http://www.w3.org/2001/XMLSchema#decimal>]];
        rr:predicateObjectMap [rr:predicate <http://example.test/integer>; rr:objectMap [rr:column "id"]]."#);
    fixture.write("ontology.ttl", &[
        "natural", "explicit", "lexical", "integer",
    ].map(|p| format!("<http://example.test/{p}> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .")).join("\n"));
    sql(database, "DELETE FROM items; INSERT INTO items(id,src,value) VALUES (1,1.0,'1'),(2,1.00,'1.0'),(3,0.000,'0'),(4,NULL,'0')");
    let (server, address) = start(fixture, database);
    for (other, expected) in [("explicit", 2), ("lexical", 2), ("integer", 0)] {
        for pattern in [
            format!("?s <http://example.test/natural> ?n . ?t <http://example.test/{other}> ?n"),
            format!("?s <http://example.test/natural> ?n FILTER EXISTS {{ ?t <http://example.test/{other}> ?n }}"),
            format!("?s <http://example.test/natural> ?n OPTIONAL {{ ?t <http://example.test/{other}> ?n }}"),
        ] {
            let query = format!("SELECT ?n ?t WHERE {{ {pattern} }}");
            let result = rows(address, fixture, &query);
            let actual = if pattern.contains("OPTIONAL") {
                assert_eq!(result.len(), 2, "optional preserves its left rows: {query}");
                result.iter().filter(|row| row.get("t").is_some()).count()
            } else { result.len() };
            assert_eq!(actual, expected, "{query}");
        }
    }
    database.assert_encrypted_sessions();
    drop(server);
}
