//! Public exact numeric values over the owned MySQL wire/collation boundary.
use super::*;
use std::cmp::Ordering;
#[path = "mysql_float_values.rs"]
mod floating;
#[path = "mysql_numeric_native.rs"]
mod native;
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const TYPES: &[&str] = &[
    "decimal",
    "integer",
    "nonPositiveInteger",
    "negativeInteger",
    "nonNegativeInteger",
    "positiveInteger",
    "long",
    "int",
    "short",
    "byte",
    "unsignedLong",
    "unsignedInt",
    "unsignedShort",
    "unsignedByte",
];

#[test]
#[ignore = "requires owned pinned MySQL TLS fixture"]
fn mysql_integer_decimal_values_are_exact() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, false);
    assert_all(&fixture, &database);
}

fn mappings(fixture: &Fixture) {
    fixture.write("first.ttl", &format!(r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
      <#m> rr:logicalTable [rr:tableName "sf_numeric_items"]; rr:subjectMap [rr:template "http://example.test/numeric/{{id}}"];
      {}; rr:predicateObjectMap [rr:predicate <http://example.test/marker>; rr:objectMap [rr:column "visible"]]."#, TYPES.iter().map(|kind| format!(r#"rr:predicateObjectMap [rr:predicate <http://example.test/{kind}>; rr:objectMap [rr:column "v"; rr:datatype <{XSD}{kind}>]]"#)).collect::<Vec<_>>().join(";\n")));
    fixture.write("ontology.ttl", &TYPES.iter().copied().chain(["marker"]).map(|kind| format!("<http://example.test/{kind}> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .")).collect::<Vec<_>>().join("\n"));
}

pub(super) fn assert_all(fixture: &Fixture, database: &Database) {
    mappings(fixture);
    sql(database, "CREATE TABLE sf_numeric_items(id BIGINT PRIMARY KEY, v TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci, visible VARCHAR(32))");
    let lexicals: Vec<String> = [
        "0",
        "-0",
        "+0",
        "1",
        "1.00",
        "+0001.00",
        "2",
        "10",
        "-1.00",
        "-0.1",
        "-0.1001",
        ".001",
        ".01",
        "0.1000",
        " \t1.00\r\n",
        "1\n2",
        "1e2",
        "",
        ".",
        "inf",
        "NaN",
        "１",
        "\u{a0}1",
        "1\0",
        "127",
        "128",
        "-128",
        "-129",
        "255",
        "256",
        "65535",
        "65536",
        "32767",
        "32768",
        "2147483647",
        "2147483648",
        "4294967295",
        "4294967296",
        "9223372036854775807",
        "9223372036854775808",
        "-9223372036854775808",
        "-9223372036854775809",
        "18446744073709551615",
        "18446744073709551616",
    ]
    .into_iter()
    .map(str::to_owned)
    .chain([
        format!("1.{}1", "0".repeat(1500)),
        format!("-1.{}1", "0".repeat(1500)),
        format!("1{}", "0".repeat(1499)),
        format!("1{}1", "0".repeat(1498)),
        " \t\r 1\n\t ".into(),
        "1\u{2028}".into(),
        "1\u{2029}".into(),
        "1\u{85}".into(),
        "1\u{b}".into(),
        "1\u{c}".into(),
    ])
    .collect();
    insert(database, &lexicals);
    let (server, address) = start(fixture, database);
    fixed_witnesses(address, fixture);
    assert_values(address, fixture, &lexicals);
    let raw = rows(address, fixture, "SELECT ?o WHERE { <http://example.test/numeric/5> <http://example.test/decimal> ?o FILTER(?o = 1) }");
    assert_eq!(raw[0]["o"]["value"], "+0001.00");
    database.assert_encrypted_sessions();
    drop(server);
    assert_policy(fixture, database, &lexicals);
    for storage in ["VARCHAR(2048)", "CHAR(255)"] {
        sql(database, &format!("DELETE FROM sf_numeric_items; ALTER TABLE sf_numeric_items MODIFY v {storage} CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci"));
        let values: Vec<_> = lexicals
            .iter()
            .filter(|v| storage != "CHAR(255)" || v.len() <= 255)
            .cloned()
            .collect();
        insert(database, &values);
        let (server, address) = start(fixture, database);
        assert_values(address, fixture, &values);
        database.assert_encrypted_sessions();
        drop(server);
    }
    native::assert_all(fixture, database);
    sql(database, "DROP TABLE sf_numeric_items");
    floating::assert_all(fixture, database);
}

fn insert(database: &Database, lexicals: &[String]) {
    let inserts = lexicals
        .iter()
        .enumerate()
        .map(|(id, v)| {
            format!(
                "({id},CONVERT(UNHEX('{}') USING utf8mb4),'same')",
                v.as_bytes()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    sql(
        database,
        &format!("INSERT INTO sf_numeric_items(id,v,visible) VALUES {inserts},(9999,NULL,'same')"),
    );
}

fn fixed_witnesses(address: SocketAddr, fixture: &Fixture) {
    // Fixed witnesses establish numeric rather than collation/string semantics.
    for (query, ids) in [
        (
            "SELECT ?s WHERE { ?s <http://example.test/decimal> ?o FILTER(?o = 1.00) }",
            vec![3, 4, 5, 14, 48],
        ),
        (
            "SELECT ?s WHERE { ?s <http://example.test/decimal> ?o FILTER(?o > 1 && ?o < 10) }",
            vec![6, 44],
        ),
        (
            "SELECT ?s WHERE { ?s <http://example.test/byte> ?o FILTER(?o = 128) }",
            vec![],
        ),
    ] {
        let rows = rows(address, fixture, query);
        let expected: BTreeSet<_> = ids
            .into_iter()
            .map(|id| format!("http://example.test/numeric/{id}"))
            .collect();
        assert_eq!(rows.len(), expected.len(), "{query}");
        assert_eq!(
            rows.iter()
                .map(|r| r["s"]["value"].as_str().unwrap().to_owned())
                .collect::<BTreeSet<_>>(),
            expected,
            "{query}"
        );
    }
}

fn compare(a: &str, kind: &str, b: &str, bkind: &str) -> Option<Ordering> {
    // Validate grammar/facets using the Rust parser, but discard its floating
    // result. Independent exact oracle aligns every digit at a common scale.
    sf_core::numeric_compare::promote_to_double(a, &format!("{XSD}{kind}"))?;
    sf_core::numeric_compare::promote_to_double(b, &format!("{XSD}{bkind}"))?;
    let parts = |v: &str| {
        let v = v.trim_matches([' ', '\t', '\r', '\n']);
        let sign = v.starts_with('-');
        let (i, f) = v
            .trim_start_matches(['+', '-'])
            .split_once('.')
            .unwrap_or((v.trim_start_matches(['+', '-']), ""));
        (sign, i.to_owned(), f.to_owned())
    };
    let (an, ai, af) = parts(a);
    let (bn, bi, bf) = parts(b);
    let width = ai.len().max(bi.len());
    let scale = af.len().max(bf.len());
    let a = format!("{:0>width$}{:0<scale$}", ai, af);
    let b = format!("{:0>width$}{:0<scale$}", bi, bf);
    Some(
        match (
            an && a.bytes().any(|c| c != b'0'),
            bn && b.bytes().any(|c| c != b'0'),
        ) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (true, true) => b.cmp(&a),
            (false, false) => a.cmp(&b),
        },
    )
}

fn matches(order: Ordering, op: &str) -> bool {
    match op {
        "=" => order.is_eq(),
        "!=" => !order.is_eq(),
        "<" => order.is_lt(),
        "<=" => !order.is_gt(),
        ">" => order.is_gt(),
        ">=" => !order.is_lt(),
        _ => unreachable!(),
    }
}

fn assert_rows(address: SocketAddr, fixture: &Fixture, query: &str, expected: &BTreeSet<String>) {
    let rows = rows(address, fixture, query);
    assert_eq!(rows.len(), expected.len(), "{query}");
    assert_eq!(
        rows.iter()
            .map(|r| r["s"]["value"].as_str().unwrap().to_owned())
            .collect::<BTreeSet<_>>(),
        *expected,
        "{query}"
    );
}

fn assert_values(address: SocketAddr, fixture: &Fixture, lexicals: &[String]) {
    let long_integer = format!("1{}", "0".repeat(1499));
    let long_decimal = format!("1.{}1", "0".repeat(1500));
    for kind in TYPES {
        for (right, rkind) in [
            ("10", "integer"),
            ("1.00", "decimal"),
            ("-0.1", "decimal"),
            ("inf", "decimal"),
            ("128", "byte"),
            (&long_integer, "integer"),
            (&long_decimal, "decimal"),
        ] {
            for op in ["=", "!=", "<", "<=", ">", ">="] {
                for negate in [false, true] {
                    let expression = format!("?o {op} \"{right}\"^^<{XSD}{rkind}>");
                    let expression = if negate {
                        format!("!({expression})")
                    } else {
                        expression
                    };
                    let query=format!("SELECT ?s WHERE {{ ?s <http://example.test/{kind}> ?o FILTER({expression}) }}");
                    let expected = lexicals
                        .iter()
                        .enumerate()
                        .filter_map(|(id, v)| {
                            if matches(compare(v, kind, right, rkind)?, op) != negate {
                                Some(format!("http://example.test/numeric/{id}"))
                            } else {
                                None
                            }
                        })
                        .collect();
                    assert_rows(address, fixture, &query, &expected);
                }
            }
        }
    }
}

fn policy_server(fixture: &Fixture, database: &Database) -> (Server, SocketAddr) {
    let (mut command, address) = command_with_admission(
        fixture,
        database,
        None,
        &["--auth-subjects-env", "SF_POLICY_SUBJECTS"],
    );
    command.env("SF_POLICY_VALUE","same").env("SF_POLICY_SUBJECTS",serde_json::json!({"schemaVersion":2,"subjects":[{
        "subjectRef":"numeric-reader","credentialEnv":"SF_TLS_BEARER","portableRows":[{"sourceIndex":0,"table":"sf_numeric_items","column":"visible","valueEnv":"SF_POLICY_VALUE"}]
    }]}).to_string());
    start_command(fixture, command, address)
}

fn assert_policy(fixture: &Fixture, database: &Database, lexicals: &[String]) {
    sql(
        database,
        "UPDATE sf_numeric_items SET visible=CASE WHEN id % 2=0 THEN 'same' ELSE 'denied' END",
    );
    let (server, address) = policy_server(fixture, database);
    let subjects = (0..lexicals.len())
        .map(|id| format!("<http://example.test/numeric/{id}>"))
        .collect::<Vec<_>>()
        .join(" ");
    for op in ["=", "!=", "<", "<=", ">", ">="] {
        let expected: BTreeSet<_> = lexicals
            .iter()
            .enumerate()
            .filter_map(|(id, v)| {
                if id % 2 == 0 && matches(compare(v, "decimal", "1.00", "decimal")?, op) {
                    Some(format!("http://example.test/numeric/{id}"))
                } else {
                    None
                }
            })
            .collect();
        let body = format!("?s <http://example.test/decimal> ?o FILTER(?o {op} 1.00)");
        for pattern in [
            body.clone(),
            format!("VALUES ?s {{ {subjects} }} FILTER EXISTS {{ {body} }}"),
        ] {
            assert_rows(
                address,
                fixture,
                &format!("SELECT ?s WHERE {{ {pattern} }}"),
                &expected,
            );
        }
        let query=format!("SELECT ?s ?marker WHERE {{ VALUES ?s {{ {subjects} }} OPTIONAL {{ {body} . ?s <http://example.test/marker> ?marker }} }}");
        let rows = rows(address, fixture, &query);
        assert_eq!(rows.len(), lexicals.len());
        for row in rows {
            assert_eq!(
                row.get("marker").is_some(),
                expected.contains(row["s"]["value"].as_str().unwrap()),
                "{query}: {row}"
            );
        }
    }
    database.assert_encrypted_sessions();
    drop(server);
}

fn rows(address: SocketAddr, fixture: &Fixture, query: &str) -> Vec<serde_json::Value> {
    let response = stop_matrix::wire(cancellation::begin(address, query, &fixture.token));
    assert!(
        response.starts_with(b"HTTP/1.1 200 ") && response.ends_with(b"0\r\n\r\n"),
        "incomplete query {query}: {}; server: {}",
        String::from_utf8_lossy(&response),
        std::fs::read_to_string(fixture.root.join("query-profile.stderr")).unwrap_or_default()
    );
    let (_, body) = decode_response(response);
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    body["results"]["bindings"].as_array().unwrap().clone()
}
