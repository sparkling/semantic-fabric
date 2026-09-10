//! Static template identity follows Rust's native decoder, not SQL text.
use super::*;
#[path = "mysql_mixed_float_iri.rs"]
pub(super) mod mixed;

#[test]
#[ignore = "requires owned pinned MySQL TLS fixture"]
fn mysql_native_float_template_iris_are_exact() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, false);
    assert_all(&fixture, &database);
}

pub(super) fn assert_all(fixture: &Fixture, database: &Database) {
    let values = native_float_values();
    let source = values
        .iter()
        .map(|v| format!("{:e}", f64::from(*v)))
        .collect::<Vec<_>>();
    let lexicals = values.iter().map(ToString::to_string).collect::<Vec<_>>();
    assert_width(fixture, database, "FLOAT", &lexicals, &source);
}

#[test]
#[ignore = "requires owned pinned MySQL TLS fixture"]
fn mysql_native_double_template_iris_are_exact() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, false);
    assert_double(&fixture, &database);
}

pub(super) fn assert_double(fixture: &Fixture, database: &Database) {
    let mut values = vec![
        1.1_f64,
        1.1,
        2.,
        0.,
        -0.,
        -1.1,
        f64::from_bits(1),
        f64::from_bits(2),
        f64::MIN_POSITIVE,
        f64::from_bits(f64::MIN_POSITIVE.to_bits() - 1),
        f64::MAX,
        -f64::MAX,
        1024. + 2_f64.powi(-14),
        -(1024. + 2_f64.powi(-14)),
    ];
    for exponent in [
        -1022, -1000, -512, -100, -40, -13, -1, 0, 1, 10, 64, 100, 512, 1000, 1023,
    ] {
        let bits = 2_f64.powi(exponent).to_bits();
        for offset in [-1_i64, 0, 1] {
            let value = f64::from_bits((bits as i64 + offset) as u64);
            values.extend([value, -value]);
        }
    }
    // Exactly representable decimal midpoints with an even lower coefficient,
    // plus adjacent doubles that must NOT be mistaken for a true tie. The odd
    // mantissa fits binary64 exactly; powers of five are computed as integers.
    for k in 1_u32..=24 {
        let five = 5_u64.pow(k);
        let odd = ((20_000_000_000_000_000 / five) / 4) * 4 + 1;
        let midpoint = (odd as f64) * 2_f64.powi(-(k as i32) - 1);
        for offset in [-1_i64, 0, 1] {
            let value = f64::from_bits((midpoint.to_bits() as i64 + offset) as u64);
            values.extend([value, -value]);
        }
    }
    let source = values.iter().map(|v| format!("{v:e}")).collect::<Vec<_>>();
    let lexicals = values.iter().map(ToString::to_string).collect::<Vec<_>>();
    assert_width(fixture, database, "DOUBLE", &lexicals, &source);
}

fn assert_width(
    fixture: &Fixture,
    database: &Database,
    storage: &str,
    values: &[String],
    source: &[String],
) {
    fixture.write(
        "first.ttl",
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
      <#m> rr:logicalTable [rr:tableName "sf_numeric_items"];
      rr:subjectMap [rr:template "http://example.test/numeric/{id}"];
      rr:predicateObjectMap [rr:predicate <http://example.test/edge>;
        rr:objectMap [rr:template "http://example.test/float/{v}"]]."#,
    );
    fixture.write(
        "ontology.ttl",
        "<http://example.test/edge> a <http://www.w3.org/2002/07/owl#ObjectProperty> .",
    );
    seed_native(database, storage, source);
    let (server, address) = start(fixture, database);
    for body in [
        "?s <http://example.test/edge> ?o . ?t <http://example.test/edge> ?o",
        "?s <http://example.test/edge> ?o . ?t <http://example.test/edge> ?p FILTER(?o = ?p)",
    ] {
        let result = rows(
            address,
            fixture,
            &format!("SELECT ?s ?t WHERE {{ {body} }}"),
        );
        let expected: BTreeSet<_> = values
            .iter()
            .enumerate()
            .flat_map(|(i, v)| {
                values
                    .iter()
                    .enumerate()
                    .filter_map(move |(j, w)| (v == w).then_some((i, j)))
            })
            .collect();
        let actual: BTreeSet<_> = result
            .iter()
            .map(|row| {
                let id = |key: &str| {
                    row[key]["value"]
                        .as_str()
                        .unwrap()
                        .rsplit('/')
                        .next()
                        .unwrap()
                        .parse::<usize>()
                        .unwrap()
                };
                (id("s"), id("t"))
            })
            .collect();
        assert_eq!(result.len(), expected.len());
        assert_eq!(actual, expected, "template joins preserve signed zero");
    }
    for value in values {
        let expected = values
            .iter()
            .enumerate()
            .filter(|(_, v)| *v == value)
            .map(|(i, _)| format!("http://example.test/numeric/{i}"))
            .collect();
        for pattern in [
            format!("?s <http://example.test/edge> <http://example.test/float/{value}>"),
            format!("?s <http://example.test/edge> ?o FILTER(?o = <http://example.test/float/{value}>)"),
            format!("?s <http://example.test/edge> ?o FILTER(sameTerm(<http://example.test/float/{value}>, ?o))"),
        ] {
            assert_rows(address, fixture, &format!("SELECT ?s WHERE {{ {pattern} }}"), &expected);
        }
    }
    for lexical in [
        "1.10", "01.1", "1.1E0", "0.0", "-0.0", "%31.1", "NaN", "inf",
    ] {
        assert_rows(address, fixture,
            &format!("SELECT ?s WHERE {{ ?s <http://example.test/edge> <http://example.test/float/{lexical}> }}"),
            &BTreeSet::new());
    }
    for pattern in [
        "?s <http://example.test/edge> ?o".to_owned(),
        "{ SELECT ?s ?o WHERE { ?s <http://example.test/edge> ?o } }".to_owned(),
    ] {
        for filter in [
            "FILTER(?o != <http://example.test/float/1.1>)",
            "FILTER(!sameTerm(?o, <http://example.test/float/1.1>))",
        ] {
            let expected = values
                .iter()
                .enumerate()
                .filter(|(_, v)| v.as_str() != "1.1")
                .map(|(i, _)| format!("http://example.test/numeric/{i}"))
                .collect();
            assert_rows(
                address,
                fixture,
                &format!("SELECT ?s WHERE {{ {pattern} {filter} }}"),
                &expected,
            );
        }
    }
    database.assert_encrypted_sessions();
    drop(server);
    sql(
        database,
        "UPDATE sf_numeric_items SET visible=CASE WHEN id % 2=0 THEN 'same' ELSE 'denied' END",
    );
    let (server, address) = policy_server(fixture, database);
    let subjects = (0..values.len())
        .map(|i| format!("<http://example.test/numeric/{i}>"))
        .collect::<Vec<_>>()
        .join(" ");
    let expected: BTreeSet<_> = values
        .iter()
        .enumerate()
        .filter(|(i, v)| i % 2 == 0 && v.as_str() == "1.1")
        .map(|(i, _)| format!("http://example.test/numeric/{i}"))
        .collect();
    let body = "?s <http://example.test/edge> ?o FILTER(?o = <http://example.test/float/1.1>)";
    for pattern in [
        body.to_owned(),
        format!("VALUES ?s {{ {subjects} }} FILTER EXISTS {{ {body} }}"),
    ] {
        assert_rows(
            address,
            fixture,
            &format!("SELECT ?s WHERE {{ {pattern} }}"),
            &expected,
        );
    }
    let output = rows(
        address,
        fixture,
        &format!("SELECT ?s ?o WHERE {{ VALUES ?s {{ {subjects} }} OPTIONAL {{ {body} }} }}"),
    );
    assert_eq!(output.len(), values.len());
    for row in output {
        assert_eq!(
            row.get("o").is_some(),
            expected.contains(row["s"]["value"].as_str().unwrap())
        );
    }
    database.assert_encrypted_sessions();
    drop(server);
    let mapping = std::fs::read_to_string(fixture.root.join("first.ttl")).unwrap();
    fixture.write(
        "first.ttl",
        &mapping.replace(
            "rr:subjectMap [rr:template \"http://example.test/numeric/{id}\"]",
            "rr:subject <http://example.test/item>",
        ),
    );
    let (server, address) = start(fixture, database);
    let expected: BTreeSet<_> = values
        .iter()
        .map(|v| format!("http://example.test/float/{v}"))
        .collect();
    for select in ["SELECT ?o", "SELECT DISTINCT ?o"] {
        let result = rows(
            address,
            fixture,
            &format!("{select} WHERE {{ ?s <http://example.test/edge> ?o }}"),
        );
        assert_eq!(result.len(), expected.len());
        let actual: BTreeSet<_> = result
            .iter()
            .map(|r| r["o"]["value"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(actual, expected, "signed-zero and boundary RDF terms");
    }
    assert_eq!(
        rows(
            address,
            fixture,
            "SELECT (COUNT(*) AS ?n) WHERE { ?s <http://example.test/edge> ?o }"
        )[0]["n"]["value"],
        expected.len().to_string()
    );
    database.assert_encrypted_sessions();
    drop(server);
    sql(database, "DROP TABLE sf_numeric_items");
    fixture.write("first.ttl", &mapping);
    for storage in [
        format!("{storage}(20,8)"),
        format!("{storage}(20,8) UNSIGNED ZEROFILL"),
    ] {
        seed_native(database, &storage, &["1.1".into(), "2.2".into()]);
        let (server, address) = start(fixture, database);
        for body in [
            "?s <http://example.test/edge> ?o",
            "{ SELECT ?s ?o WHERE { ?s <http://example.test/edge> ?o } }",
        ] {
            assert_rows(
                address,
                fixture,
                &format!(
                    "SELECT ?s WHERE {{ {body} FILTER(?o = <http://example.test/float/1.1>) }}"
                ),
                &BTreeSet::from(["http://example.test/numeric/0".into()]),
            );
        }
        database.assert_encrypted_sessions();
        drop(server);
        sql(database, "DROP TABLE sf_numeric_items");
    }
    // A second, differently shaped recipe must not fall back to MySQL CONCAT
    // coercion. Both mappings reconstruct the same IRI from the same row.
    fixture.write(
        "first.ttl",
        &format!(
            r#"{mapping}
      <#n> rr:logicalTable [rr:tableName "sf_numeric_items"];
      rr:subjectMap [rr:template "http://example.test/numeric/{{id}}"];
      rr:predicateObjectMap [rr:predicate <http://example.test/edge2>;
        rr:objectMap [rr:template "http://example.test/{{kind}}/{{v}}"]]."#
        ),
    );
    fixture.write("ontology.ttl",
        "<http://example.test/edge> a <http://www.w3.org/2002/07/owl#ObjectProperty> .\n<http://example.test/edge2> a <http://www.w3.org/2002/07/owl#ObjectProperty> .");
    seed_native(database, storage, &source[..8]);
    sql(
        database,
        "ALTER TABLE sf_numeric_items ADD kind VARCHAR(8) DEFAULT 'float'",
    );
    let (server, address) = start(fixture, database);
    for body in [
        "?s <http://example.test/edge> ?o; <http://example.test/edge2> ?o",
        "?s <http://example.test/edge> ?o; <http://example.test/edge2> ?p FILTER(sameTerm(?o, ?p))",
    ] {
        assert_rows(
            address,
            fixture,
            &format!("SELECT ?s WHERE {{ {body} }}"),
            &(0..8)
                .map(|i| format!("http://example.test/numeric/{i}"))
                .collect(),
        );
    }
    database.assert_encrypted_sessions();
    drop(server);
    sql(database, "DROP TABLE sf_numeric_items");
}
