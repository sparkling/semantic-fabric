//! Public MySQL floating-promotion checks use Rust IEEE values as their oracle.
use super::*;
#[path = "mysql_native_floats.rs"]
mod native_floats;

#[test]
#[ignore = "requires owned pinned MySQL TLS fixture"]
fn mysql_floating_numeric_values_are_exact() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, false);
    assert_all(&fixture, &database);
}

#[test]
#[ignore = "requires owned pinned MySQL TLS fixture"]
fn mysql_floating_policy_is_exact() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, false);
    mapping(&fixture);
    sql(
        &database,
        "CREATE TABLE sf_numeric_items(id BIGINT PRIMARY KEY,v TEXT,visible VARCHAR(32))",
    );
    let values = values();
    insert(&database, &values);
    policy(&fixture, &database, &values);
}

fn mapping(fixture: &Fixture) {
    let kinds: Vec<_> = TYPES.iter().copied().chain(["float", "double"]).collect();
    fixture.write("first.ttl", &format!(r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
      <#m> rr:logicalTable [rr:tableName "sf_numeric_items"]; rr:subjectMap [rr:template "http://example.test/numeric/{{id}}"];
      {}; rr:predicateObjectMap [rr:predicate <http://example.test/marker>; rr:objectMap [rr:column "visible"]]."#,
      kinds.iter().map(|kind| format!(r#"rr:predicateObjectMap [rr:predicate <http://example.test/{kind}>; rr:objectMap [rr:column "v"; rr:datatype <{XSD}{kind}>]]"#)).collect::<Vec<_>>().join(";\n")));
    fixture.write("ontology.ttl", &kinds.into_iter().chain(["marker"]).map(|kind|format!("<http://example.test/{kind}> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .")).collect::<Vec<_>>().join("\n"));
}

fn dyadic(coefficient: u64, exponent: i32) -> String {
    let mut digits: Vec<_> = coefficient
        .to_string()
        .bytes()
        .rev()
        .map(|b| b - b'0')
        .collect();
    for _ in 0..exponent.unsigned_abs() {
        let mut carry = 0;
        for d in &mut digits {
            let p = *d * if exponent < 0 { 5 } else { 2 } + carry;
            *d = p % 10;
            carry = p / 10;
        }
        if carry > 0 {
            digits.push(carry);
        }
    }
    let coefficient: String = digits
        .into_iter()
        .rev()
        .map(|d| char::from(b'0' + d))
        .collect();
    if exponent >= 0 {
        return coefficient;
    }
    let mut decimal = format!(
        "{:0>width$}",
        coefficient,
        width = exponent.unsigned_abs() as usize + 1
    );
    decimal.insert(decimal.len() - exponent.unsigned_abs() as usize, '.');
    decimal
}

fn values() -> Vec<String> {
    let mut values: Vec<_> = [
        "1",
        "1.00",
        "+0001.00",
        "2",
        "10",
        "NaN",
        "inf",
        "garbage",
        "INF",
        "-INF",
        "0",
        "-0",
        "+0",
        ".1",
        "-0.1",
        "1e-999999999999",
        "-1e999999999999",
        "1e999999999999",
        "1e-46",
        "1e-45",
        "1e-324",
        "5e-324",
        " \t1.00\r\n",
        "1\n2",
        "1\u{2028}",
        "1\u{85}",
        "1\0",
        "１",
        "\u{a0}1",
        "127",
        "128",
        "-128",
        "-129",
        "255",
        "256",
        "18446744073709551615",
        "18446744073709551616",
        "9223372036854775807",
        "9223372036854775808",
        "1.000000059604644775390625",
        "1.000000178813934326171875",
        "0.9999999701976776123046875",
        "2.00000011920928955078125",
        "1.000000000000000111022302462516",
        "-1.000000000000000111022302462516",
        "340282346638528859811704183484516925440",
        "340282356779733661637539395458142568447",
        "340282356779733661637539395458142568448",
        "340282356779733661637539395458142568449",
        "1.7976931348623157e308",
        "1.7976931348623159e308",
    ]
    .into_iter()
    .map(str::to_owned)
    .chain([
        format!("1.000000059604644775390625{}1", "0".repeat(1500)),
        format!("1.000000059604644775390624{}9", "9".repeat(1500)),
        format!(
            "1.00000000000000011102230246251565404236316680908203125{}1",
            "0".repeat(1500)
        ),
        format!("1{}e-1500", "0".repeat(1500)),
        format!("0.{}1e1501", "0".repeat(1500)),
    ])
    .collect();
    for (coefficient, exponent) in [
        (1, -150),
        ((1 << 24) - 1, -150),
        ((1 << 24) + 1, -150),
        ((1 << 24) - 1, -149),
        (1, -1075),
        ((1 << 53) + 1, -53),
    ] {
        let midpoint = dyadic(coefficient, exponent);
        let mut below = midpoint.clone().into_bytes();
        *below.last_mut().unwrap() -= 1;
        let below = format!("{}{}", String::from_utf8(below).unwrap(), "9".repeat(1500));
        for v in [
            midpoint.clone(),
            below,
            format!("{midpoint}{}1", "0".repeat(1500)),
        ] {
            values.push(format!("-{v}"));
            values.push(v);
        }
    }
    let overflow = dyadic((1 << 54) - 1, 970);
    let mut below = overflow.clone().into_bytes();
    *below.last_mut().unwrap() -= 1;
    values.extend([
        overflow.clone(),
        String::from_utf8(below).unwrap(),
        format!("{overflow}.1"),
    ]);
    values
}

fn promoted(v: &str, kind: &str, double: bool) -> Option<f64> {
    let dt = format!("{XSD}{kind}");
    if double {
        sf_core::numeric_compare::promote_to_double(v, &dt)
    } else {
        sf_core::numeric_compare::promote_to_float(v, &dt).map(f64::from)
    }
}

fn compare_values(a: f64, b: f64, op: &str) -> bool {
    match op {
        "=" => a == b,
        "!=" => a != b,
        "<" => a < b,
        "<=" => a <= b,
        ">" => a > b,
        ">=" => a >= b,
        _ => unreachable!(),
    }
}

fn matrix(address: SocketAddr, fixture: &Fixture, values: &[String], kinds: &[&str]) {
    for kind in kinds {
        for (right, rkind) in [
            ("1.00000011920928955078125", "float"),
            ("1.0000000000000002", "double"),
            ("NaN", "double"),
            ("INF", "float"),
            ("0", "double"),
            ("0", "float"),
            ("inf", "double"),
        ] {
            let double = *kind == "double" || rkind == "double";
            let r = promoted(right, rkind, double);
            for op in ["=", "!=", "<", "<=", ">", ">="] {
                for negate in [false, true] {
                    let predicate = format!("?o {op} \"{right}\"^^<{XSD}{rkind}>");
                    let predicate = if negate {
                        format!("!({predicate})")
                    } else {
                        predicate
                    };
                    let expected = values
                        .iter()
                        .enumerate()
                        .filter_map(|(i, v)| {
                            if compare_values(promoted(v, kind, double)?, r?, op) != negate {
                                Some(format!("http://example.test/numeric/{i}"))
                            } else {
                                None
                            }
                        })
                        .collect();
                    assert_rows(address,fixture,&format!("SELECT ?s WHERE {{ ?s <http://example.test/{kind}> ?o FILTER({predicate}) }}"),&expected);
                }
            }
        }
    }
    for (left, right) in [
        ("float", "double"),
        ("decimal", "float"),
        ("integer", "double"),
    ] {
        let double = right == "double";
        for op in ["=", "!=", "<", "<=", ">", ">="] {
            let expected = values
                .iter()
                .enumerate()
                .filter_map(|(i, v)| {
                    if compare_values(promoted(v, left, double)?, promoted(v, right, double)?, op) {
                        Some(format!("http://example.test/numeric/{i}"))
                    } else {
                        None
                    }
                })
                .collect();
            assert_rows(address,fixture,&format!("SELECT ?s WHERE {{ ?s <http://example.test/{left}> ?a; <http://example.test/{right}> ?b FILTER(?a {op} ?b) }}"),&expected);
        }
    }
}

pub(super) fn assert_all(fixture: &Fixture, database: &Database) {
    mapping(fixture);
    sql(database,"CREATE TABLE sf_numeric_items(id BIGINT PRIMARY KEY, v TEXT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci, visible VARCHAR(32))");
    let values = values();
    insert(database, &values);
    let (server, address) = start(fixture, database);
    assert_rows(
        address,
        fixture,
        "SELECT ?s WHERE { VALUES ?s { <http://example.test/numeric/0> <http://example.test/numeric/1> <http://example.test/numeric/2> } ?s <http://example.test/decimal> ?o FILTER(?o = 1e0) }",
        &BTreeSet::from([
            "http://example.test/numeric/0".into(),
            "http://example.test/numeric/1".into(),
            "http://example.test/numeric/2".into(),
        ]),
    );
    let kinds: Vec<_> = TYPES.iter().copied().chain(["float", "double"]).collect();
    matrix(address, fixture, &values, &kinds);
    for kind in ["float", "double"] {
        let raw=rows(address,fixture,&format!("SELECT ?o WHERE {{ <http://example.test/numeric/2> <http://example.test/{kind}> ?o FILTER(?o = 1e0) }}"));
        assert_eq!(raw[0]["o"]["value"], "+0001.00");
        assert_eq!(raw[0]["o"]["datatype"], format!("{XSD}{kind}"));
    }
    database.assert_encrypted_sessions();
    drop(server);
    policy(fixture, database, &values);
    for storage in ["VARCHAR(4096)", "CHAR(255)"] {
        sql(database,&format!("DELETE FROM sf_numeric_items; ALTER TABLE sf_numeric_items MODIFY v {storage} CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci"));
        let values: Vec<_> = values
            .iter()
            .filter(|v| storage != "CHAR(255)" || v.len() <= 255)
            .cloned()
            .collect();
        insert(database, &values);
        let (server, address) = start(fixture, database);
        matrix(address, fixture, &values, &["float", "double", "byte"]);
        database.assert_encrypted_sessions();
        drop(server);
    }
    for (storage, values) in [
        (
            "DECIMAL(65,30)",
            vec![
                "0.000000000000000000000000000000",
                "1.000000000000000111022302462516",
                "-1.000000000000000111022302462516",
                "1.000000059604644775390625000001",
                "1.000000059604644775390624999999",
            ],
        ),
        (
            "DECIMAL(8,2) UNSIGNED ZEROFILL",
            vec!["0.00", "1.00", "10.00", "999999.99"],
        ),
        (
            "BIGINT",
            vec![
                "0",
                "1",
                "-1",
                "16777217",
                "9223372036854775807",
                "-9223372036854775808",
            ],
        ),
        (
            "BIGINT(20) UNSIGNED ZEROFILL",
            vec!["0", "1", "16777217", "9223372036854775807"],
        ),
        ("YEAR", vec!["0", "1901", "2000", "2155"]),
    ] {
        sql(
            database,
            &format!(
                "DELETE FROM sf_numeric_items; ALTER TABLE sf_numeric_items MODIFY v {storage}"
            ),
        );
        let inserts = values
            .iter()
            .enumerate()
            .map(|(i, v)| format!("({i},{v},'same')"))
            .collect::<Vec<_>>()
            .join(",");
        sql(
            database,
            &format!("INSERT INTO sf_numeric_items VALUES {inserts},(9999,NULL,'same')"),
        );
        let (server, address) = start(fixture, database);
        let values: Vec<_> = values.into_iter().map(str::to_owned).collect();
        matrix(
            address,
            fixture,
            &values,
            &["float", "double", "decimal", "integer", "byte"],
        );
        database.assert_encrypted_sessions();
        drop(server);
    }
    native::assert_source_validation(fixture, database);
    sql(database, "DROP TABLE sf_numeric_items");
    native_floats::assert_all(fixture, database);
}

fn policy(fixture: &Fixture, database: &Database, values: &[String]) {
    sql(
        database,
        "UPDATE sf_numeric_items SET visible=CASE WHEN id % 2=0 THEN 'same' ELSE 'denied' END",
    );
    let (server, address) = policy_server(fixture, database);
    let subjects = (0..values.len())
        .map(|i| format!("<http://example.test/numeric/{i}>"))
        .collect::<Vec<_>>()
        .join(" ");
    for kind in ["float", "double"] {
        for op in ["=", "!=", "<", "<=", ">", ">="] {
            let expected: BTreeSet<_> = values
                .iter()
                .enumerate()
                .filter_map(|(i, v)| {
                    if i % 2 == 0 && compare_values(promoted(v, kind, true)?, 1., op) {
                        Some(format!("http://example.test/numeric/{i}"))
                    } else {
                        None
                    }
                })
                .collect();
            let body = format!("?s <http://example.test/{kind}> ?o FILTER(?o {op} 1e0)");
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
            assert_eq!(rows.len(), values.len());
            for row in rows {
                assert_eq!(
                    row.get("marker").is_some(),
                    expected.contains(row["s"]["value"].as_str().unwrap()),
                    "{query}: {row}"
                );
            }
        }
    }
    database.assert_encrypted_sessions();
    drop(server);
}
