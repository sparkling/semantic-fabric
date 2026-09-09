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

fn assert_scalar_constants(fixture: &Fixture, database: &Database, postgres: bool) {
    let cases = if postgres {
        vec![
            ("BYTEA", "decode('00ff61622f', 'hex')", "00FF61622F"),
            ("BYTEA", "decode('', 'hex')", ""),
            ("BYTEA", "decode('0000', 'hex')", "0000"),
            ("BOOLEAN", "TRUE", "true"),
            ("BOOLEAN", "FALSE", "false"),
        ]
    } else {
        vec![
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
        sql(
            database,
            &format!(
                "INSERT INTO items(id,src,value) VALUES (1,{value},'match'),(2,NULL,'absent')"
            ),
        );
        let (server, address) = start(fixture, database);
        let iri = format!("http://example.test/n/{lexical}");
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
        let mut wrongs = vec![format!("{lexical}x"), format!("{lexical}%20")];
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
        if lexical.to_lowercase() != lexical {
            wrongs.push(lexical.to_lowercase());
        }
        if kind == "BOOLEAN" {
            wrongs.extend([lexical.to_uppercase(), "1".into(), "t".into()]);
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
}
