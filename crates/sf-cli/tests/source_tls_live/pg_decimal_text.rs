//! Full-precision public comparison oracle, independent of SQL normalization.
use super::*;
use std::cmp::Ordering;

fn compare(a: &str, kind: &str, b: &str, bkind: &str) -> Option<Ordering> {
    // The established Rust parser validates grammar and facets only. Its
    // floating result is deliberately discarded, never used for comparison.
    sf_core::numeric_compare::promote_to_double(a, &format!("{XSD}{kind}"))?;
    sf_core::numeric_compare::promote_to_double(b, &format!("{XSD}{bkind}"))?;
    let parts = |v: &str| {
        let v = v.trim_matches([' ', '\t', '\r', '\n']);
        let sign = v.starts_with('-');
        let v = v.trim_start_matches(['+', '-']);
        let (i, f) = v.split_once('.').unwrap_or((v, ""));
        (sign, i.to_owned(), f.to_owned())
    };
    let (an, ai, af) = parts(a);
    let (bn, bi, bf) = parts(b);
    // Align complete digit sequences to a common decimal scale. No prefix,
    // bounded integer parse, floating approximation or SQL tuple comparator.
    let width = ai.len().max(bi.len());
    let scale = af.len().max(bf.len());
    let a = format!("{:0>width$}{:0<scale$}", ai, af);
    let b = format!("{:0>width$}{:0<scale$}", bi, bf);
    let an = an && a.bytes().any(|c| c != b'0');
    let bn = bn && b.bytes().any(|c| c != b'0');
    Some(match (an, bn) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (true, true) => b.cmp(&a),
        (false, false) => a.cmp(&b),
    })
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

pub(super) fn assert_values(address: SocketAddr, fixture: &Fixture, lexicals: &[String]) {
    assert_invalid_constants(address, fixture, "decimal");
    let long_integer = format!("1{}", "0".repeat(1499));
    let long_decimal = format!("1.{}1", "0".repeat(1500));
    for kind in &TYPES[2..] {
        for (right, rkind) in [
            ("10", "integer"),
            ("1.00", "decimal"),
            ("-1.0", "decimal"),
            ("0.1", "decimal"),
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
                    let query = format!("SELECT ?s WHERE {{ ?s <http://example.test/{kind}> ?o FILTER({expression}) }}");
                    let expected: BTreeSet<_> = lexicals
                        .iter()
                        .enumerate()
                        .filter_map(|(id, v)| {
                            if matches(compare(v, kind, right, rkind)?, op) != negate {
                                Some(format!("http://example.test/text/{id}"))
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

fn assert_invalid_constants(address: SocketAddr, fixture: &Fixture, predicate: &str) {
    for op in ["=", "!=", "<", "<=", ">", ">="] {
        for negation in ["", "!"] {
            let query = format!(
                r#"SELECT ?s WHERE {{ ?s <http://example.test/{predicate}> ?o FILTER({negation}("\u0000"^^<{XSD}decimal> {op} ?o)) }}"#
            );
            assert!(
                complete_rows(address, fixture, &query).is_empty(),
                "{query}"
            );
        }
    }
}

#[test]
#[ignore = "requires owned pinned PostgreSQL TLS fixture"]
fn postgres_invalid_decimal_constants_are_expression_errors() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, true);
    fixture.write("first.ttl", r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
      <#m> rr:logicalTable [rr:tableName "items"]; rr:subject <http://example.test/item>;
      rr:predicateObjectMap [rr:predicate <http://example.test/decimal>; rr:objectMap [rr:column "value"; rr:datatype <http://www.w3.org/2001/XMLSchema#decimal>]]."#);
    fixture.write(
        "ontology.ttl",
        "<http://example.test/decimal> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .",
    );
    sql(
        &database,
        "DELETE FROM items; INSERT INTO items(value) VALUES ('1.00')",
    );
    let (server, address) = start(&fixture, &database);
    assert_invalid_constants(address, &fixture, "decimal");
    drop(server);
    // A query lexical error cannot hide an invalid admitted native value.
    sql(&database, "DELETE FROM items; ALTER TABLE items ALTER COLUMN value TYPE NUMERIC USING NULL::NUMERIC; INSERT INTO items(value) VALUES ('NaN')");
    let (server, address) = start(&fixture, &database);
    let query = format!(
        r#"SELECT ?s WHERE {{ ?s <http://example.test/decimal> ?o FILTER("\u0000"^^<{XSD}decimal> = ?o) }}"#
    );
    let response = stop_matrix::wire(cancellation::begin(address, &query, &fixture.token));
    stop_matrix::assert_no_complete_union_success(&response);
    drop(server);
}

pub(super) fn assert_native(fixture: &Fixture, database: &Database) {
    sql(database, "DELETE FROM items; ALTER TABLE items ALTER COLUMN numeric_text TYPE NUMERIC USING NULL::NUMERIC");
    let lexicals: Vec<String> = [
        "0",
        "0.00",
        "1",
        "1.0",
        "-1.00",
        "2",
        "10",
        "0.1001",
        "0.0100",
        "128",
        "9223372036854775808",
    ]
    .into_iter()
    .map(str::to_owned)
    .chain([
        "9".repeat(1500),
        format!("1{}", "0".repeat(1499)),
        format!("1{}1", "0".repeat(1498)),
        format!("1.{}1", "0".repeat(1500)),
        format!("-0.{}1", "0".repeat(1500)),
    ])
    .collect();
    let inserts = lexicals
        .iter()
        .enumerate()
        .map(|(id, v)| format!("({id},'{v}'::numeric,'same')"))
        .collect::<Vec<_>>()
        .join(",");
    sql(
        database,
        &format!(
            "INSERT INTO items(numeric_id,numeric_text,value) VALUES {inserts},(9999,NULL,'same')"
        ),
    );
    let (server, address) = start(fixture, database);
    assert_values(address, fixture, &lexicals);
    drop(server);
    sql(database, "DELETE FROM items; ALTER TABLE items ALTER COLUMN numeric_text TYPE BIGINT USING NULL::BIGINT");
    let lexicals: Vec<String> = [
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
    .collect();
    let inserts = lexicals
        .iter()
        .enumerate()
        .map(|(id, v)| format!("({id},{v},'same')"))
        .collect::<Vec<_>>()
        .join(",");
    sql(
        database,
        &format!(
            "INSERT INTO items(numeric_id,numeric_text,value) VALUES {inserts},(9999,NULL,'same')"
        ),
    );
    let (server, address) = start(fixture, database);
    assert_values(address, fixture, &lexicals);
    drop(server);
}

fn assert_rows(address: SocketAddr, fixture: &Fixture, query: &str, expected: &BTreeSet<String>) {
    let rows = complete_rows(address, fixture, query);
    assert_eq!(rows.len(), expected.len(), "{query}");
    assert_eq!(
        rows.iter()
            .map(|r| r["s"]["value"].as_str().unwrap().to_owned())
            .collect::<BTreeSet<_>>(),
        *expected,
        "{query}"
    );
}

pub(super) fn assert_policy(address: SocketAddr, fixture: &Fixture, lexicals: &[String]) {
    let subjects = (0..lexicals.len())
        .map(|id| format!("<http://example.test/text/{id}>"))
        .collect::<Vec<_>>()
        .join(" ");
    for op in ["=", "!=", "<", "<=", ">", ">="] {
        let expected: BTreeSet<_> = lexicals
            .iter()
            .enumerate()
            .filter_map(|(id, v)| {
                if id % 2 == 0 && matches(compare(v, "decimal", "1.00", "decimal")?, op) {
                    Some(format!("http://example.test/text/{id}"))
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
        let query = format!("SELECT ?s ?marker WHERE {{ VALUES ?s {{ {subjects} }} OPTIONAL {{ {body} . ?s <http://example.test/marker> ?marker }} }}");
        let rows = complete_rows(address, fixture, &query);
        assert_eq!(rows.len(), lexicals.len(), "{query}");
        for row in rows {
            let matched = expected.contains(row["s"]["value"].as_str().unwrap());
            assert_eq!(row.get("marker").is_some(), matched, "{query}: {row}");
            if matched {
                assert_eq!(row["marker"]["value"], "same");
            }
        }
    }
}
