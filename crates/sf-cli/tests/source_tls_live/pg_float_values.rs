//! Public value comparisons use independent Rust IEEE operators as the oracle.
use super::*;

const DOUBLE: &str = "http://example.test/double";
const INTEGER_DOUBLE: &str = "http://example.test/integer-double";

fn compare(left: f64, right: f64, op: &str) -> bool {
    match op {
        "=" => left == right,
        "!=" => left != right,
        "<" => left < right,
        "<=" => left <= right,
        ">" => left > right,
        ">=" => left >= right,
        _ => unreachable!(),
    }
}

pub(super) fn assert_cross_width(address: SocketAddr, fixture: &Fixture) {
    let narrow = ["0.0E0", "-0.0E0", "1.1E0"];
    let wide = ["0.0E0", "-0.0E0", "1.1E0", "1.100000023841858E0"];
    for op in ["=", "!=", "<", "<=", ">", ">="] {
        for reverse in [false, true] {
            let expression = if reverse {
                format!("?v {op} ?o")
            } else {
                format!("?o {op} ?v")
            };
            let query = format!("SELECT ?o ?v WHERE {{ ?s <{NATURAL}> ?o . ?t <{EXPLICIT}> ?v FILTER({expression}) }}");
            let expected: BTreeSet<_> = narrow
                .iter()
                .flat_map(|left| {
                    wide.iter().filter_map(move |right| {
                        let (a, b) = (left.parse::<f64>().unwrap(), right.parse::<f64>().unwrap());
                        (if reverse {
                            compare(b, a, op)
                        } else {
                            compare(a, b, op)
                        })
                        .then_some((left.to_string(), right.to_string()))
                    })
                })
                .collect();
            let actual = complete_rows(address, fixture, &query);
            assert_eq!(actual.len(), expected.len(), "{query}: {actual:?}");
            assert_eq!(
                actual
                    .iter()
                    .map(|r| (
                        r["o"]["value"].as_str().unwrap().to_owned(),
                        r["v"]["value"].as_str().unwrap().to_owned()
                    ))
                    .collect::<BTreeSet<_>>(),
                expected,
                "{query}"
            );
        }
    }
}

pub(super) fn assert_constants(
    address: SocketAddr,
    fixture: &Fixture,
    predicate: &str,
    source: &BTreeSet<String>,
) {
    let typed = |lexical: &str, kind: &str| {
        format!("\"{lexical}\"^^<http://www.w3.org/2001/XMLSchema#{kind}>")
    };
    let source_float = predicate == "http://example.test/float";
    let source_double = matches!(predicate, NATURAL | DOUBLE | INTEGER_DOUBLE);
    for (literal, number) in [
        (typed("0", "double"), Some(0.0)),
        (typed("1.1", "double"), Some(1.1)),
        (typed("1.1", "float"), Some(f64::from(1.1_f32))),
        (typed("1", "float"), Some(1.0)),
        (typed("NaN", "float"), Some(f64::NAN)),
        (typed("INF", "float"), Some(f64::INFINITY)),
        (typed("1e-45", "float"), Some(f64::from(f32::from_bits(1)))),
        (
            typed("4611686018427387904", "float"),
            Some(4611686018427387904.0),
        ),
        (typed("inf", "float"), None),
        (typed("NaN", "double"), Some(f64::NAN)),
        (typed("INF", "double"), Some(f64::INFINITY)),
        (typed("5e-324", "double"), Some(f64::from_bits(1))),
        (typed(&"9".repeat(400), "integer"), Some(f64::INFINITY)),
        (typed("inf", "double"), None),
        (typed("1e2", "integer"), None),
        (typed("1", "int"), Some(1.0)),
        (typed("256", "unsignedByte"), None),
        (typed("-0", "negativeInteger"), None),
        ("\"1\"@en".into(), None),
    ] {
        // Decimal/integer-only comparisons retain their exact, non-IEEE lane.
        let float_only = (!source_double && literal.contains("XMLSchema#float>"))
            || (source_float && !literal.contains("XMLSchema#double>"));
        if !source_double && !source_float && !float_only && !literal.contains("XMLSchema#double>")
        {
            continue;
        }
        for op in ["=", "!=", "<", "<=", ">", ">="] {
            // Negation must preserve an expression error rather than expose
            // rows by treating an invalid lexical as ordinary false.
            for negate in [false, true] {
                let comparison = format!("?o {op} {literal}");
                let expression = if negate {
                    format!("!({comparison})")
                } else {
                    comparison
                };
                let query =
                    format!("SELECT ?o WHERE {{ ?s <{predicate}> ?o FILTER({expression}) }}");
                let expected: BTreeSet<_> = source
                    .iter()
                    .filter(|lexical| {
                        // Native raw infinity spellings are invalid authored
                        // XSD float lexicals, even though Rust accepts `inf`.
                        if source_float && matches!(lexical.as_str(), "inf" | "-inf") {
                            return false;
                        }
                        number.is_some_and(|right| {
                            let left = if float_only || source_float {
                                f64::from(lexical.parse::<f32>().unwrap())
                            } else {
                                lexical.parse::<f64>().unwrap()
                            };
                            compare(left, right, op) != negate
                        })
                    })
                    .cloned()
                    .collect();
                let rows = complete_rows(address, fixture, &query);
                assert_eq!(rows.len(), expected.len(), "{query}: {rows:?}");
                assert_eq!(
                    rows.iter()
                        .map(|r| r["o"]["value"].as_str().unwrap().to_owned())
                        .collect::<BTreeSet<_>>(),
                    expected,
                    "{query}"
                );
            }
        }
    }
}

pub(super) fn assert_numeric(fixture: &Fixture, database: &Database) {
    fixture.write("first.ttl", r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
      <#n> rr:logicalTable [rr:tableName "items"]; rr:subject <http://example.test/item>;
      rr:predicateObjectMap [rr:predicate <http://example.test/number>; rr:objectMap [rr:column "float_decimal"]];
      rr:predicateObjectMap [rr:predicate <http://example.test/float>; rr:objectMap [rr:column "float_decimal"; rr:datatype <http://www.w3.org/2001/XMLSchema#float>]];
      rr:predicateObjectMap [rr:predicate <http://example.test/double>; rr:objectMap [rr:column "float_decimal"; rr:datatype <http://www.w3.org/2001/XMLSchema#double>]];
      rr:predicateObjectMap [rr:predicate <http://example.test/integer-double>; rr:objectMap [rr:column "float_integer"; rr:datatype <http://www.w3.org/2001/XMLSchema#double>]];
      rr:predicateObjectMap [rr:predicate <http://example.test/raw>; rr:objectMap [rr:column "float_decimal"; rr:datatype <http://www.w3.org/2001/XMLSchema#string>]];
      rr:predicateObjectMap [rr:predicate <http://example.test/integer>; rr:objectMap [rr:column "float_integer"]]."#);
    fixture.write(
        "ontology.ttl",
        &([
            "number",
            "integer",
            "raw",
            "float",
            "double",
            "integer-double",
        ]
        .map(|name| {
            format!(
                "<http://example.test/{name}> a <http://www.w3.org/2002/07/owl#DatatypeProperty> ."
            )
        })
        .join("\n")),
    );
    sql(database, "DELETE FROM items; ALTER TABLE items ADD COLUMN float_decimal NUMERIC; ALTER TABLE items ADD COLUMN float_integer BIGINT");
    // Exercise exact rounding boundaries and the complete native NUMERIC
    // exponent range without a bounded-i64/decimal intermediate representation.
    let overflow = "(power(2::numeric,1024)-power(2::numeric,970))";
    let underflow = "(trim_scale(power(5::numeric,1075))::text||'e-1075')::numeric";
    let float_overflow = "(power(2::numeric,128)-power(2::numeric,103))";
    let float_underflow = "(trim_scale(power(5::numeric,150))::text||'e-150')::numeric";
    let expressions = [
        "1.000000059604644776".into(), // Direct f32 rounds above1; via f64 ties to1.
        "16777217".into(),
        "1.1".into(),
        "0".into(),
        "-1.1".into(),
        "9007199254740993".into(),
        "1e1000".into(),
        "-1e1000".into(),
        "1e-2000".into(),
        "-1e-2000".into(),
        overflow.into(),
        format!("{overflow}-1"),
        format!("{overflow}+1"),
        format!("-{overflow}"),
        format!("-{overflow}+1"),
        underflow.into(),
        format!("{underflow}-1e-1100"),
        format!("{underflow}+1e-1100"),
        format!("-({underflow})"),
        format!("-({underflow})-1e-1100"),
        float_overflow.into(),
        format!("{float_overflow}-1"),
        format!("{float_overflow}+1"),
        format!("-{float_overflow}"),
        format!("-{float_overflow}+1"),
        float_underflow.into(),
        format!("{float_underflow}-1e-200"),
        format!("{float_underflow}+1e-200"),
        format!("-({float_underflow})"),
        format!("-({float_underflow})-1e-200"),
    ];
    let inserts = expressions
        .iter()
        .map(|v| format!("({v},9007199254740993,'same')"))
        .collect::<Vec<_>>()
        .join(",");
    sql(database, &format!("INSERT INTO items(float_decimal,float_integer,value) VALUES {inserts},(NULL,NULL,'same'),(NULL,16777217,'same'),(NULL,4611686293305294849,'same')"));
    let (server, address) = start(fixture, database);
    for predicate in [
        DOUBLE,
        INTEGER_DOUBLE,
        "http://example.test/number",
        "http://example.test/integer",
        "http://example.test/float",
    ] {
        let source = complete_rows(
            address,
            fixture,
            &format!("SELECT ?o WHERE {{ ?s <{predicate}> ?o }}"),
        )
        .iter()
        .map(|r| r["o"]["value"].as_str().unwrap().to_owned())
        .collect();
        assert_constants(address, fixture, predicate, &source);
    }
    drop(server);
    assert_invalid_and_policy(fixture, database);
    sql(database, "DELETE FROM items; ALTER TABLE items DROP COLUMN float_decimal; ALTER TABLE items DROP COLUMN float_integer");
}

fn assert_invalid_and_policy(fixture: &Fixture, database: &Database) {
    for invalid in ["NaN", "Infinity", "-Infinity"] {
        sql(database, &format!("DELETE FROM items; INSERT INTO items(float_decimal,value) VALUES ('{invalid}','same')"));
        let (server, address) = start(fixture, database);
        for expression in [
            "?o > 0e0",
            "\"NaN\"^^<http://www.w3.org/2001/XMLSchema#double> = ?o",
            "\"inf\"^^<http://www.w3.org/2001/XMLSchema#double> = ?o",
            "?o > \"0\"^^<http://www.w3.org/2001/XMLSchema#float>",
            "\"NaN\"^^<http://www.w3.org/2001/XMLSchema#float> = ?o",
            "\"inf\"^^<http://www.w3.org/2001/XMLSchema#float> = ?o",
        ] {
            for pattern in [
                format!("?s <http://example.test/number> ?o FILTER({expression})"),
                format!("VALUES ?s {{ <http://example.test/item> }} FILTER EXISTS {{ ?s <http://example.test/number> ?o FILTER({expression}) }}"),
                format!("?s <http://example.test/raw> ?o FILTER({expression})"),
                format!("?s <http://example.test/raw> ?o FILTER(!({expression}))"),
                format!("?s <http://example.test/float> ?o FILTER({expression})"),
                format!("VALUES ?s {{ <http://example.test/item> }} FILTER EXISTS {{ ?s <http://example.test/float> ?o FILTER({expression}) }}"),
                format!("?s <{DOUBLE}> ?o FILTER({expression})"),
                format!("?s <{DOUBLE}> ?o FILTER(!({expression}))"),
                format!("VALUES ?s {{ <http://example.test/item> }} FILTER EXISTS {{ ?s <{DOUBLE}> ?o FILTER({expression}) }}"),
            ] {
                let query = format!("SELECT ?s WHERE {{ {pattern} }}");
                let response = stop_matrix::wire(cancellation::begin(address, &query, &fixture.token));
                stop_matrix::assert_no_complete_union_success(&response);
                let until = Instant::now() + Duration::from_secs(3);
                loop {
                    let (status, _) = request(address, "ASK {}", Some(&fixture.token)).unwrap();
                    if status == 200 { break; }
                    assert_eq!(status, 503);
                    assert!(Instant::now() < until, "invalid NUMERIC did not release admission");
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
        drop(server);
        sql(database, "UPDATE items SET value='denied'; INSERT INTO items(float_decimal,value) VALUES (1.1,'same')");
        let (mut command, address) = command_with_admission(
            fixture,
            database,
            None,
            &["--auth-subjects-env", "SF_POLICY_SUBJECTS"],
        );
        command.env("SF_POLICY_VALUE", "same").env("SF_POLICY_SUBJECTS", serde_json::json!({"schemaVersion":2,"subjects":[{
            "subjectRef":"numeric-reader", "credentialEnv":"SF_TLS_BEARER",
            "portableRows":[{"sourceIndex":0,"table":"items","column":"value","valueEnv":"SF_POLICY_VALUE"}]
        }]}).to_string());
        let (server, address) = start_command(fixture, command, address);
        for op in [">", ">=", "<", "<=", "=", "!="] {
            let constant = if matches!(op, ">" | ">=" | "!=") {
                "0e0"
            } else if matches!(op, "<" | "<=") {
                "2e0"
            } else {
                "1.1e0"
            };
            let float_constant = format!(
                "\"{}\"^^<http://www.w3.org/2001/XMLSchema#float>",
                constant.trim_end_matches("e0")
            );
            for pattern in [
                format!("?s <http://example.test/number> ?o FILTER(?o {op} {constant})"),
                format!("VALUES ?s {{ <http://example.test/item> }} FILTER EXISTS {{ ?s <http://example.test/number> ?o FILTER(?o {op} {constant}) }}"),
                format!("VALUES ?s {{ <http://example.test/item> }} OPTIONAL {{ ?s <http://example.test/number> ?o FILTER(?o {op} {constant}) }}"),
                format!("?s <http://example.test/number> ?o FILTER(?o {op} {float_constant})"),
                format!("VALUES ?s {{ <http://example.test/item> }} FILTER EXISTS {{ ?s <http://example.test/number> ?o FILTER(?o {op} {float_constant}) }}"),
                format!("VALUES ?s {{ <http://example.test/item> }} OPTIONAL {{ ?s <http://example.test/number> ?o FILTER(?o {op} {float_constant}) }}"),
                format!("?s <http://example.test/float> ?o FILTER(?o {op} {float_constant})"),
                format!("VALUES ?s {{ <http://example.test/item> }} FILTER EXISTS {{ ?s <http://example.test/float> ?o FILTER(?o {op} {float_constant}) }}"),
                format!("VALUES ?s {{ <http://example.test/item> }} OPTIONAL {{ ?s <http://example.test/float> ?o FILTER(?o {op} {float_constant}) }}"),
                format!("?s <{DOUBLE}> ?o FILTER(?o {op} {constant})"),
                format!("VALUES ?s {{ <http://example.test/item> }} FILTER EXISTS {{ ?s <{DOUBLE}> ?o FILTER(?o {op} {constant}) }}"),
                format!("VALUES ?s {{ <http://example.test/item> }} OPTIONAL {{ ?s <{DOUBLE}> ?o FILTER(?o {op} {constant}) }}"),
            ] {
                let query = format!("SELECT ?s WHERE {{ {pattern} }}");
                assert_eq!(complete_rows(address, fixture, &query).len(), 1, "{invalid}: {query}");
            }
        }
        for negate in ["", "!"] {
            let query = format!(
                "SELECT ?s WHERE {{ ?s <http://example.test/raw> ?o FILTER({negate}(?o < 0e0)) }}"
            );
            assert!(
                complete_rows(address, fixture, &query).is_empty(),
                "denied invalid NUMERIC must not poison a nonnumeric comparison"
            );
        }
        database.assert_encrypted_sessions();
        drop(server);
    }
}
