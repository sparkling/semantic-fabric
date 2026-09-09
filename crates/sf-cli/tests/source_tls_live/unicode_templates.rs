//! Authenticated public equality renders both different-shape templates in SQL.
use super::*;

pub(super) fn assert_native_encoding(fixture: &Fixture, database: &Database, postgres: bool) {
    fixture.write(
        "first.ttl",
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> rr:logicalTable [rr:tableName "items"];
 rr:subjectMap [rr:template "http://example.test/id/{id}"];
 rr:predicateObjectMap [rr:predicate <http://example.test/edge>;
   rr:objectMap [rr:template "http://example.test/n/{src}-suffix"]];
 rr:predicateObjectMap [rr:predicate <http://example.test/other>;
   rr:objectMap [rr:template "http://example.test/n/{dst}"]]."#,
    );
    fixture.write("ontology.ttl", "<http://example.test/edge> a <http://www.w3.org/2002/07/owl#ObjectProperty> . <http://example.test/other> a <http://www.w3.org/2002/07/owl#ObjectProperty> .");
    sql(database, "DELETE FROM items");
    // Earlier profile slices deliberately narrow these columns to CHAR(4).
    // This separate fixture slice needs space for a scalar plus its suffix.
    sql(
        database,
        if postgres {
            "ALTER TABLE items ALTER COLUMN src TYPE VARCHAR(64); ALTER TABLE items ALTER COLUMN dst TYPE VARCHAR(64)"
        } else {
            "ALTER TABLE items MODIFY src VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci, MODIFY dst VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"
        },
    );
    let cases = [
        ("C280", "%C2%80"),
        ("EE8080", "%EE%80%80"),
        ("EFB790", "%EF%B7%90"),
        ("F09FBFBE", "%F0%9F%BF%BE"),
        ("F3A08080", "%F3%A0%80%80"),
        ("F3B08080", "%F3%B0%80%80"),
        ("E4BDA0E5A5BD", "你好"),
        ("F3A18080", "\u{e1000}"),
    ];
    let text = |hex: &str| {
        if postgres {
            format!("convert_from(decode('{hex}', 'hex'), 'UTF8')")
        } else {
            format!("CONVERT(UNHEX('{hex}') USING utf8mb4)")
        }
    };
    for (id, (hex, _)) in cases.iter().enumerate() {
        sql(database, &format!("INSERT INTO items(id,src,dst,value) VALUES ({id}, {}, CONCAT({},'-suffix'),'same')", text(hex), text(hex)));
    }
    let (server, address) = start(fixture, database);
    // Raw term reconstruction and SQL TemplateEq must agree independently.
    for filter in ["", "FILTER(?a = ?b)", "FILTER(sameTerm(?a, ?b))"] {
        let query = format!(
            "SELECT ?a WHERE {{ ?s <{EDGE}> ?a . ?s <http://example.test/other> ?b {filter} }}"
        );
        let result = rows(address, fixture, &query);
        let actual: BTreeSet<_> = result
            .iter()
            .map(|row| row["a"]["value"].as_str().unwrap().to_owned())
            .collect();
        let expected: BTreeSet<_> = cases
            .iter()
            .map(|(_, encoded)| format!("http://example.test/n/{encoded}-suffix"))
            .collect();
        assert_eq!(actual, expected, "{query}");
        assert_eq!(result.len(), cases.len());
    }
    let query = format!("SELECT (COUNT(*) AS ?n) WHERE {{ ?s <{EDGE}> ?a . ?s <http://example.test/other> ?b FILTER(?a != ?b) }}");
    assert_eq!(rows(address, fixture, &query)[0]["n"]["value"], "0");
    database.assert_encrypted_sessions();
    drop(server);
}

pub(super) fn assert_static_constants(fixture: &Fixture, database: &Database, postgres: bool) {
    fixture.write("first.ttl", r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> rr:logicalTable [rr:tableName "items"];
 rr:subjectMap [rr:template "http://example.test/id/{id}"];
 rr:predicateObjectMap [rr:predicate <http://example.test/value>; rr:objectMap [rr:column "value"]];
 rr:predicateObjectMap [rr:predicate <http://example.test/edge>; rr:objectMap [rr:template "http://example.test/n/{src}"]]."#);
    fixture.write("ontology.ttl", "<http://example.test/value> a <http://www.w3.org/2002/07/owl#DatatypeProperty> . <http://example.test/edge> a <http://www.w3.org/2002/07/owl#ObjectProperty> .");
    sql(database, "DELETE FROM items; INSERT INTO items(id,src,value) VALUES (1,'a/b','slash'),(2,'a%2Fb','percent'),(3,'a','plain'),(4,'a ','space'),(5,NULL,'null')");
    let (server, address) = start(fixture, database);
    for (key, expected) in [
        ("a%2Fb", vec!["slash"]),
        ("a%252Fb", vec!["percent"]),
        ("a", vec!["plain"]),
        ("a%20", vec!["space"]),
        ("a%2fb", vec![]),
        ("A%2Fb", vec![]),
        ("%61", vec![]),
    ] {
        for pattern in [
            format!("?s <{EDGE}> <http://example.test/n/{key}>; <http://example.test/value> ?value"),
            format!("?s <{EDGE}> ?o; <http://example.test/value> ?value FILTER(?o = <http://example.test/n/{key}>)"),
            format!("?s <{EDGE}> ?o; <http://example.test/value> ?value FILTER(sameTerm(<http://example.test/n/{key}>, ?o))"),
        ] {
            let query = format!("SELECT ?value WHERE {{ {pattern} }}");
            let result = rows(address, fixture, &query);
            let actual: Vec<_> = result.iter().map(|r| r["value"]["value"].as_str().unwrap()).collect();
            assert_eq!(actual, expected, "{query}");
        }
    }
    for (key, count) in [("1", 1), ("01", 0), ("%2B1", 0), ("1.0", 0)] {
        for pattern in [
            format!("<http://example.test/id/{key}> <http://example.test/value> ?value"),
            format!(
                "?s <http://example.test/value> ?value FILTER(?s = <http://example.test/id/{key}>)"
            ),
        ] {
            let query = format!("SELECT ?value WHERE {{ {pattern} }}");
            assert_eq!(rows(address, fixture, &query).len(), count, "{query}");
        }
    }
    database.assert_encrypted_sessions();
    drop(server);
    let mut integers = vec![
        ("SMALLINT", "-32768"),
        ("INTEGER", "-2147483648"),
        ("BIGINT", "-9223372036854775808"),
        ("BIGINT", "9223372036854775807"),
    ];
    if !postgres {
        integers.extend([
            ("BIGINT UNSIGNED", "18446744073709551615"),
            ("YEAR", "0"),
            ("INT(5) ZEROFILL", "1"),
        ]);
    }
    for (kind, lexical) in integers {
        sql(database, "DELETE FROM items");
        let alter = if postgres {
            format!("ALTER TABLE items ALTER COLUMN src TYPE {kind} USING src::{kind}")
        } else {
            format!("ALTER TABLE items MODIFY src {kind}")
        };
        sql(database, &alter);
        sql(
            database,
            &format!("INSERT INTO items(id,src,value) VALUES(1,{lexical},'integer')"),
        );
        let (server, address) = start(fixture, database);
        let expected = format!("http://example.test/n/{lexical}");
        let query = format!("SELECT ?o WHERE {{ ?s <{EDGE}> ?o }}");
        assert_eq!(rows(address, fixture, &query)[0]["o"]["value"], expected);
        let query = format!("SELECT ?s WHERE {{ ?s <{EDGE}> <{expected}> }}");
        assert_eq!(rows(address, fixture, &query).len(), 1, "{kind}: {query}");
        drop(server);
    }
    sql(database, "DELETE FROM items");
    sql(
        database,
        if postgres {
            "ALTER TABLE items ALTER COLUMN src TYPE CHAR(8)"
        } else {
            "ALTER TABLE items MODIFY src CHAR(8) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"
        },
    );
    sql(
        database,
        "INSERT INTO items(id,src,value) VALUES(1,'a/b','slash')",
    );
    let (server, address) = start(fixture, database);
    let suffix = if postgres {
        "a%2Fb%20%20%20%20%20"
    } else {
        "a%2Fb"
    };
    let query = format!("SELECT ?s WHERE {{ ?s <{EDGE}> <http://example.test/n/{suffix}> }}");
    assert_eq!(
        rows(address, fixture, &query)[0]["s"]["value"],
        "http://example.test/id/1"
    );
    drop(server);
    assert_scalar_constants(fixture, database, postgres);
    if !postgres {
        assert_date_bags(fixture, database);
    }
}

#[test]
#[ignore = "requires owned pinned PostgreSQL/MySQL TLS fixtures"]
fn native_static_template_constants_are_exact() {
    for postgres in [true, false] {
        let fixture = Fixture::new();
        let database = Database::start(&fixture, postgres);
        sql(&database, "ALTER TABLE items ADD COLUMN id INTEGER PRIMARY KEY DEFAULT 0; ALTER TABLE items ADD COLUMN src VARCHAR(64)");
        assert_static_constants(&fixture, &database, postgres);
    }
}

fn assert_date_bags(fixture: &Fixture, database: &Database) {
    sql(database, "DELETE FROM items; ALTER TABLE items MODIFY src DATE; SET SESSION sql_mode='ALLOW_INVALID_DATES'; INSERT INTO items(id,src,value) VALUES (1,'2001-00-03','one'),(2,'2001-00-03','duplicate'),(3,'2001-00-04','two'),(4,NULL,'absent')");
    fixture.write("first.ttl", r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> rr:logicalTable [rr:tableName "items"];
 rr:subject <http://example.test/item>;
 rr:predicateObjectMap [rr:predicate <http://example.test/edge>; rr:objectMap [rr:template "http://example.test/n/{src}"]];
 rr:predicateObjectMap [rr:predicate <http://example.test/date>; rr:objectMap [rr:column "src"; rr:datatype <http://www.w3.org/2001/XMLSchema#date>]]."#);
    fixture.write("ontology.ttl", "<http://example.test/edge> a <http://www.w3.org/2002/07/owl#ObjectProperty> . <http://example.test/date> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .");
    let (server, address) = start(fixture, database);
    let listing = rows(
        address,
        fixture,
        &format!("SELECT ?o WHERE {{ ?s <{EDGE}> ?o }}"),
    );
    let actual: BTreeSet<_> = listing
        .iter()
        .map(|row| row["o"]["value"].as_str().unwrap())
        .collect();
    assert_eq!(
        listing.len(),
        2,
        "D1 removes duplicate triples, not distinct partial dates"
    );
    assert_eq!(
        actual,
        BTreeSet::from([
            "http://example.test/n/2001-00-03",
            "http://example.test/n/2001-00-04"
        ])
    );
    let count = rows(
        address,
        fixture,
        &format!("SELECT (COUNT(*) AS ?n) WHERE {{ ?s <{EDGE}> ?o }}"),
    );
    assert_eq!(count[0]["n"]["value"], "2");
    let literal = rows(
        address,
        fixture,
        "SELECT ?o WHERE { ?s <http://example.test/date> ?o }",
    );
    let actual: BTreeSet<_> = literal
        .iter()
        .map(|row| {
            assert_eq!(
                row["o"]["datatype"],
                "http://www.w3.org/2001/XMLSchema#date"
            );
            row["o"]["value"].as_str().unwrap()
        })
        .collect();
    assert_eq!(literal.len(), 2);
    assert_eq!(actual, BTreeSet::from(["2001-00-03", "2001-00-04"]));
    database.assert_encrypted_sessions();
    drop(server);
}

fn assert_scalar_constants(fixture: &Fixture, database: &Database, postgres: bool) {
    let cases = if postgres {
        vec![
            ("BYTEA", "decode('00ff61622f', 'hex')", "00FF61622F"),
            ("BYTEA", "decode('', 'hex')", ""),
            ("BYTEA", "decode('0000', 'hex')", "0000"),
            ("NUMERIC(30,6)", "-12.34", "-12.340000"),
            ("NUMERIC", "1.00", "1.00"),
            ("BOOLEAN", "TRUE", "true"),
            ("BOOLEAN", "FALSE", "false"),
        ]
    } else {
        vec![
            ("DATE", "'2024-02-29'", "2024-02-29"),
            ("DATE", "'0001-01-01'", "0001-01-01"),
            ("DATE", "'0000-00-00'", "0000-00-00"),
            ("DATE", "'2001-00-03'", "2001-00-03"),
            ("DATE", "'2001-02-00'", "2001-02-00"),
            ("DATE", "'2001-02-31'", "2001-02-31"),
            ("DATE", "'9999-12-31'", "9999-12-31"),
            (
                "DATETIME(6)",
                "'2024-02-29 01:02:03.000001'",
                "2024-02-29T01:02:03.000001",
            ),
            (
                "DATETIME(6)",
                "'0000-00-00 00:00:00'",
                "0000-00-00T00:00:00",
            ),
            (
                "DATETIME(6)",
                "'2001-00-03 12:34:56.000001'",
                "2001-00-03T12:34:56.000001",
            ),
            (
                "DATETIME(6)",
                "'2001-02-31 12:34:56.000001'",
                "2001-02-31T12:34:56.000001",
            ),
            (
                "DATETIME(1)",
                "'2001-02-00 12:34:56.1'",
                "2001-02-00T12:34:56.100000",
            ),
            (
                "DATETIME(6)",
                "'2001-00-03 00:00:00.000000'",
                "2001-00-03T00:00:00",
            ),
            (
                "TIMESTAMP(6) NULL",
                "'2024-02-29 01:02:03.000001'",
                "2024-02-29T01:02:03.000001",
            ),
            ("TIME(6)", "'-838:59:59'", "-838:59:59"),
            ("TIME(6)", "'838:59:59'", "838:59:59"),
            ("TIME(6)", "'-838:59:58.999999'", "-838:59:58.999999"),
            ("TIME(1)", "'12:34:56.1'", "12:34:56.100000"),
            ("TIME(1)", "'25:02:03.1'", "25:02:03.100000"),
            (
                "TIMESTAMP(6) NULL",
                "'0000-00-00 00:00:00'",
                "0000-00-00T00:00:00",
            ),
            ("TIME(6)", "'-00:00:00.000001'", "-00:00:00.000001"),
            ("TIME(6)", "'-00:00:00.000000'", "00:00:00"),
            ("BIT(1)", "b'0'", "00"),
            ("BIT(1)", "b'1'", "01"),
            ("BIT(9)", "b'1'", "0001"),
            ("BIT(9)", "b'111111111'", "01FF"),
            ("BIT(64)", "1", "0000000000000001"),
            ("BIT(64)", "18446744073709551615", "FFFFFFFFFFFFFFFF"),
            ("VARBINARY(8)", "X'00FF61622F'", "00FF61622F"),
            ("VARBINARY(8)", "X''", ""),
            ("BLOB", "X'00'", "00"),
            ("BINARY(8)", "X'00FF61622F'", "00FF61622F000000"),
            ("DECIMAL(30,6)", "-12.34", "-12.340000"),
            ("DECIMAL(30,6)", "0", "0.000000"),
            ("DECIMAL(30,6)", "-0.000000", "0.000000"),
            (
                "DECIMAL(30,6)",
                "123456789012345678901234.123456",
                "123456789012345678901234.123456",
            ),
            ("DECIMAL(12,4) ZEROFILL", "12.34", "00000012.3400"),
        ]
    };
    for (kind, value, lexical) in cases {
        sql(
            database,
            "DELETE FROM items; ALTER TABLE items DROP COLUMN src",
        );
        sql(
            database,
            &format!("ALTER TABLE items ADD COLUMN src {kind}"),
        );
        if kind.starts_with("TIMESTAMP") {
            // New reader sessions inherit this owned fixture's non-UTC zone.
            sql(database, "SET GLOBAL time_zone = '+05:30'");
        }
        let legacy_dates = if kind.starts_with("TIMESTAMP") || kind.starts_with("DATE") {
            // Fixture writer only: qualify existing partial/invalid/zero dates
            // without relaxing the application reader's normal SQL mode.
            "SET SESSION sql_mode = 'ALLOW_INVALID_DATES'; "
        } else {
            ""
        };
        sql(
            database,
            &format!(
                "{legacy_dates}INSERT INTO items(id,src,value) VALUES (1,{value},'match'),(2,NULL,'absent')"
            ),
        );
        let (server, address) = start(fixture, database);
        // The independent oracle's new temporal values escape colons only.
        let suffix = lexical.replace(':', "%3A");
        let iri = format!("http://example.test/n/{suffix}");
        let result = rows(
            address,
            fixture,
            &format!("SELECT ?o WHERE {{ ?s <{EDGE}> ?o }}"),
        );
        assert_eq!(result.len(), 1, "{kind}: NULL is not a term");
        assert_eq!(result[0]["o"]["value"], iri, "{kind}: decoder premise");
        for pattern in [
            format!("?s <{EDGE}> <{iri}>"),
            format!("?s <{EDGE}> ?o FILTER(?o = <{iri}>)"),
            format!("?s <{EDGE}> ?o FILTER(sameTerm(<{iri}>, ?o))"),
        ] {
            let query = format!("SELECT ?s WHERE {{ {pattern} }}");
            let result = rows(address, fixture, &query);
            assert_eq!(result.len(), 1, "{kind}: {query}");
            assert_eq!(result[0]["s"]["value"], "http://example.test/id/1");
        }
        let query = format!("SELECT (COUNT(*) AS ?n) WHERE {{ ?s <{EDGE}> <{iri}> }}");
        assert_eq!(rows(address, fixture, &query)[0]["n"]["value"], "1");
        let query = format!("SELECT ?s WHERE {{ ?s <{EDGE}> ?o FILTER(?o != <{iri}>) }}");
        assert!(
            rows(address, fixture, &query).is_empty(),
            "NULL is not false: {query}"
        );
        let mut wrongs = vec![format!("{suffix}x"), format!("{suffix}%20")];
        let query = format!("SELECT ?s ?o WHERE {{ ?s <http://example.test/value> ?v OPTIONAL {{ ?s <{EDGE}> ?o FILTER(sameTerm(?o, <{iri}>)) }} }}");
        let optional = rows(address, fixture, &query);
        assert_eq!(optional.len(), 2, "{kind}: OPTIONAL preserves absent term");
        let present = optional
            .iter()
            .find(|row| row["s"]["value"] == "http://example.test/id/1")
            .unwrap();
        let absent = optional
            .iter()
            .find(|row| row["s"]["value"] == "http://example.test/id/2")
            .unwrap();
        assert_eq!(present["o"]["value"], iri);
        assert!(absent.get("o").is_none());
        if suffix.to_lowercase() != suffix {
            wrongs.push(suffix.to_lowercase());
        }
        if kind == "BOOLEAN" {
            wrongs.extend([lexical.to_uppercase(), "1".into(), "t".into()]);
        }
        if kind.starts_with("BIT") {
            let unpadded = lexical.trim_start_matches('0');
            if unpadded != lexical {
                wrongs.push(unpadded.into());
            }
        }
        if kind.starts_with("TIME") || kind.starts_with("DATETIME") {
            wrongs.push(suffix.replace("%3A", ":"));
            if lexical.contains('T') {
                wrongs.push(suffix.replace('T', "%20"));
            }
            if let Some((whole, fraction)) = suffix.split_once('.') {
                wrongs.push(format!("{whole}.{}", fraction.trim_end_matches('0')));
            } else {
                wrongs.push(format!("{suffix}.000000"));
            }
            wrongs.retain(|wrong| wrong != &suffix);
        }
        if kind.starts_with("DECIMAL") {
            wrongs.push(format!("{lexical}0"));
            let trimmed = lexical.trim_end_matches('0');
            if trimmed != lexical {
                wrongs.push(trimmed.into());
            }
        }
        for wrong in wrongs {
            let query =
                format!("SELECT ?s WHERE {{ ?s <{EDGE}> <http://example.test/n/{wrong}> }}");
            assert!(rows(address, fixture, &query).is_empty(), "{kind}: {query}");
        }
        database.assert_encrypted_sessions();
        drop(server);
    }
    sql(database, "DELETE FROM items; ALTER TABLE items DROP COLUMN src; ALTER TABLE items ADD COLUMN src VARCHAR(64)");
    if !postgres {
        sql(database, "SET GLOBAL time_zone = '+00:00'");
    }
}
