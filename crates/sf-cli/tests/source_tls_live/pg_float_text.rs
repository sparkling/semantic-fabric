//! Native SQL numeric text must agree with the independent Rust lexical parser.
use super::*;

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const TYPES: &[&str] = &[
    "double",
    "float",
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
#[ignore = "requires owned pinned PostgreSQL TLS fixture"]
fn postgres_text_numeric_values_are_exact() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, true);
    assert_all(&fixture, &database);
}

fn lexicals() -> Vec<String> {
    let mut values: Vec<String> = [
        "0",
        "-0",
        "+0",
        "1",
        "-1",
        "1.1",
        "-1.1",
        " \t1.10\r\n",
        ".5",
        "1.",
        "1e2",
        "1E+0002",
        "INF",
        "-INF",
        "NaN",
        "+INF",
        "inf",
        "Infinity",
        "+NaN",
        "",
        ".",
        "1e",
        "1e+",
        "1e-1e0",
        "1_0",
        "0x1p0",
        "1\n2",
        "--1",
        "１",
        "\u{a0}1",
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
        "1.000000059604644776",
        "4611686293305294849",
        "1e309",
        "1e-500",
        "-1e-500",
        "2.4703282292062327e-324",
        "2.4703282292062328e-324",
        "1.7976931348623157e308",
        "1.7976931348623159e308",
        "7.006492321624085e-46",
        "7.006492321624086e-46",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for midpoint in [
        "1.00000000000000011102230246251565404236316680908203125",
        "1.000000059604644775390625",
    ] {
        values.push(midpoint.into());
        values.push(format!("{midpoint}{}1", "0".repeat(1300)));
        values.push(format!("-{midpoint}{}1", "0".repeat(1300)));
        let below = format!(
            "{}4{}",
            midpoint.strip_suffix('5').unwrap(),
            "9".repeat(1300)
        );
        values.push(below.clone());
        values.push(format!("-{below}"));
    }
    values.extend([
        format!("0.{}1e1301", "0".repeat(1300)),
        format!("1{}e-1300", "0".repeat(1300)),
        format!("1e{}1", "0".repeat(1300)),
        format!("1e{}", "9".repeat(1300)),
        format!("1e-{}", "9".repeat(1300)),
        format!("-0e{}", "9".repeat(1300)),
        format!("-0e-{}", "9".repeat(1300)),
        format!("1.{}x", "0".repeat(1300)),
    ]);
    values
}

pub(super) fn assert_all(fixture: &Fixture, database: &Database) {
    let lexicals = lexicals();
    fixture.write("first.ttl", &format!(r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
      <#m> rr:logicalTable [rr:tableName "items"]; rr:subjectMap [rr:template "http://example.test/text/{{numeric_id}}"];
      {}; rr:predicateObjectMap [rr:predicate <http://example.test/marker>; rr:objectMap [rr:column "value"]]."#, TYPES.iter().map(|kind| format!(r#"rr:predicateObjectMap [rr:predicate <http://example.test/{kind}>; rr:objectMap [rr:column "numeric_text"; rr:datatype <{XSD}{kind}>]]"#)).collect::<Vec<_>>().join(";\n")));
    fixture.write("ontology.ttl", &TYPES.iter().copied().chain(["marker"]).map(|kind| format!("<http://example.test/{kind}> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .")).collect::<Vec<_>>().join("\n"));
    sql(
        database,
        "DELETE FROM items; CREATE COLLATION sf_float_lexical (provider=icu,locale='und-u-ks-level1',deterministic=false); ALTER TABLE items ADD COLUMN numeric_text TEXT COLLATE sf_float_lexical; ALTER TABLE items ADD COLUMN numeric_id BIGINT",
    );
    let inserts = lexicals
        .iter()
        .enumerate()
        .map(|(id, value)| {
            format!(
                "({id},convert_from(decode('{}','hex'),'UTF8'),'same')",
                value
                    .as_bytes()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    sql(
        database,
        &format!(
            "INSERT INTO items(numeric_id,numeric_text,value) VALUES {inserts},(9999,NULL,'same')"
        ),
    );
    for character in [false, true] {
        if character {
            sql(
                database,
                "ALTER TABLE items ALTER COLUMN numeric_text TYPE CHARACTER(2048) COLLATE sf_float_lexical",
            );
        }
        let (server, address) = start(fixture, database);
        for kind in TYPES {
            for (right_lexical, right_kind) in [
                ("0", "double"),
                ("1", "double"),
                ("INF", "double"),
                ("NaN", "double"),
                ("1.1", "float"),
                ("1", "float"),
                ("inf", "float"),
            ] {
                // Full operator matrix for the new floating parsers; derived
                // integer facets additionally check ordered and negated use.
                let ops: &[&str] = if matches!(*kind, "float" | "double") {
                    &["=", "!=", "<", "<=", ">", ">="]
                } else {
                    &["=", "<"]
                };
                for op in ops {
                    for negate in [false, true] {
                        let expression =
                            format!("?o {op} \"{right_lexical}\"^^<{XSD}{right_kind}>");
                        let expression = if negate {
                            format!("!({expression})")
                        } else {
                            expression
                        };
                        let query = format!("SELECT ?s WHERE {{ ?s <http://example.test/{kind}> ?o FILTER({expression}) }}");
                        let use_float = *kind != "double" && right_kind != "double";
                        let promote = |value: &str, kind: &str| {
                            if use_float {
                                sf_core::numeric_compare::promote_to_float(
                                    value,
                                    &format!("{XSD}{kind}"),
                                )
                                .map(f64::from)
                            } else {
                                sf_core::numeric_compare::promote_to_double(
                                    value,
                                    &format!("{XSD}{kind}"),
                                )
                            }
                        };
                        let right = promote(right_lexical, right_kind);
                        let expected: BTreeSet<_> = lexicals
                            .iter()
                            .enumerate()
                            .filter_map(|(id, value)| {
                                let (left, right) = (promote(value, kind)?, right?);
                                let matches = match *op {
                                    "=" => left == right,
                                    "!=" => left != right,
                                    "<" => left < right,
                                    "<=" => left <= right,
                                    ">" => left > right,
                                    ">=" => left >= right,
                                    _ => unreachable!(),
                                };
                                (matches != negate)
                                    .then(|| format!("http://example.test/text/{id}"))
                            })
                            .collect();
                        let rows = complete_rows(address, fixture, &query);
                        assert_eq!(rows.len(), expected.len(), "character={character}: {query}");
                        assert_eq!(
                            rows.iter()
                                .map(|r| r["s"]["value"].as_str().unwrap().to_owned())
                                .collect::<BTreeSet<_>>(),
                            expected,
                            "character={character}: {query}"
                        );
                    }
                }
            }
        }
        database.assert_encrypted_sessions();
        for id in [
            5,
            lexicals
                .iter()
                .position(|v| v.starts_with("1.000") && v.len() > 1100)
                .unwrap(),
        ] {
            let query = format!("SELECT ?o WHERE {{ <http://example.test/text/{id}> <http://example.test/double> ?o FILTER(?o > 0e0) }}");
            let rows = complete_rows(address, fixture, &query);
            assert_eq!(rows.len(), 1);
            let expected = if character {
                format!("{:<2048}", lexicals[id])
            } else {
                lexicals[id].clone()
            };
            assert_eq!(
                rows[0]["o"]["value"], expected,
                "value conversion must not replace raw RDF output"
            );
            assert_eq!(rows[0]["o"]["datatype"], format!("{XSD}double"));
        }
        drop(server);
    }
    assert_policy(fixture, database, &lexicals);
    sql(
        database,
        "DELETE FROM items; ALTER TABLE items DROP COLUMN numeric_text; ALTER TABLE items DROP COLUMN numeric_id; DROP COLLATION sf_float_lexical",
    );
}

fn assert_policy(fixture: &Fixture, database: &Database, lexicals: &[String]) {
    sql(
        database,
        "UPDATE items SET value=CASE WHEN numeric_id % 2=0 THEN 'same' ELSE 'denied' END",
    );
    let (mut command, address) = command_with_admission(
        fixture,
        database,
        None,
        &["--auth-subjects-env", "SF_POLICY_SUBJECTS"],
    );
    command.env("SF_POLICY_VALUE", "same").env("SF_POLICY_SUBJECTS", serde_json::json!({"schemaVersion":2,"subjects":[{
        "subjectRef":"text-numeric-reader", "credentialEnv":"SF_TLS_BEARER",
        "portableRows":[{"sourceIndex":0,"table":"items","column":"value","valueEnv":"SF_POLICY_VALUE"}]
    }]}).to_string());
    let (server, address) = start_command(fixture, command, address);
    let subjects = (0..lexicals.len())
        .map(|id| format!("<http://example.test/text/{id}>"))
        .collect::<Vec<_>>()
        .join(" ");
    for op in ["=", "!=", "<", "<=", ">", ">="] {
        let expected: BTreeSet<_> = lexicals
            .iter()
            .enumerate()
            .filter_map(|(id, lexical)| {
                let v =
                    sf_core::numeric_compare::promote_to_double(lexical, &format!("{XSD}double"))?;
                let matches = match op {
                    "=" => v == 1.0,
                    "!=" => v != 1.0,
                    "<" => v < 1.0,
                    "<=" => v <= 1.0,
                    ">" => v > 1.0,
                    ">=" => v >= 1.0,
                    _ => unreachable!(),
                };
                (id % 2 == 0 && matches).then(|| format!("http://example.test/text/{id}"))
            })
            .collect();
        let body = format!("?s <http://example.test/double> ?o FILTER(?o {op} 1e0)");
        for pattern in [
            body.clone(),
            format!("VALUES ?s {{ {subjects} }} FILTER EXISTS {{ {body} }}"),
        ] {
            let query = format!("SELECT ?s WHERE {{ {pattern} }}");
            let rows = complete_rows(address, fixture, &query);
            assert_eq!(rows.len(), expected.len(), "{query}");
            assert_eq!(
                rows.iter()
                    .map(|r| r["s"]["value"].as_str().unwrap().to_owned())
                    .collect::<BTreeSet<_>>(),
                expected,
                "{query}"
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
    database.assert_encrypted_sessions();
    drop(server);
}
