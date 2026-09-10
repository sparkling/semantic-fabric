//! Native floating constructors use Rust's wire lexical, not SQL widening.
use super::*;
#[path = "mysql_float_iri.rs"]
mod iri;

#[test]
#[ignore = "requires owned pinned MySQL TLS fixture"]
fn mysql_native_float_values_follow_wire_lexicals() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, false);
    assert_float(&fixture, &database);
}

fn native_float_values() -> Vec<f32> {
    let mut values = vec![
        1.1_f32,
        1.1,
        2.,
        0.,
        -0.,
        -1.1,
        f32::from_bits(1),
        f32::from_bits(2),
        f32::from_bits(3),
        f32::from_bits(f32::MIN_POSITIVE.to_bits() - 1),
        f32::MIN_POSITIVE,
        f32::from_bits(f32::MIN_POSITIVE.to_bits() + 1),
        f32::MAX,
        -f32::MAX,
        2_f32.powi(-12),
    ];
    for exponent in [-125, -100, -40, -13, -1, 0, 1, 10, 64, 100, 126] {
        let bits = 2_f32.powi(exponent).to_bits();
        for offset in [-1_i64, 0, 1] {
            let value = f32::from_bits((i64::from(bits) + offset) as u32);
            values.extend([value, -value]);
        }
    }
    values
}

fn assert_float(fixture: &Fixture, database: &Database) {
    mappings(fixture);
    let values = native_float_values();
    let lexicals: Vec<_> = values.iter().map(ToString::to_string).collect();
    // Seed the exact binary32 value widened to f64. A shortest f32 decimal
    // spelling may exceed FLT_MAX before MySQL's strict assignment rounding.
    let source: Vec<_> = values
        .iter()
        .map(|v| format!("{:e}", f64::from(*v)))
        .collect();
    seed_native(database, "FLOAT", &source);
    let (server, address) = start(fixture, database);
    assert_rows(
        address,
        fixture,
        "SELECT ?s WHERE { ?s <http://example.test/double> ?o FILTER(?o = 1.1e0) }",
        &BTreeSet::from([
            "http://example.test/numeric/0".into(),
            "http://example.test/numeric/1".into(),
        ]),
    );
    check_values(address, fixture, &lexicals, &["double", "natural", "float"]);
    for op in ["=", "!=", "<", "<=", ">", ">="] {
        let expected = lexicals
            .iter()
            .enumerate()
            .filter(|(_, v)| {
                compare_values(
                    promoted(v, "float", true).unwrap(),
                    promoted(v, "double", true).unwrap(),
                    op,
                )
            })
            .map(|(i, _)| format!("http://example.test/numeric/{i}"))
            .collect();
        assert_rows(address,fixture,&format!("SELECT ?s WHERE {{ ?s <http://example.test/float> ?a; <http://example.test/double> ?b FILTER(?a {op} ?b) }}"),&expected);
    }
    database.assert_encrypted_sessions();
    drop(server);
    policy(fixture, database, &lexicals);
    sql(database, "DROP TABLE sf_numeric_items");
    for storage in ["FLOAT(20,8)", "FLOAT(20,8) UNSIGNED ZEROFILL"] {
        seed_native(database, storage, &["1.1".into(), "2.2".into()]);
        let (server, address) = start(fixture, database);
        for predicate in ["float", "double", "natural"] {
            let constant = if predicate == "float" {
                format!("\"1.1\"^^<{XSD}float>")
            } else {
                "1.1e0".into()
            };
            let body = format!("?s <http://example.test/{predicate}> ?o");
            for pattern in [
                body.clone(),
                format!("{{ SELECT ?s ?o WHERE {{ {body} }} }}"),
            ] {
                assert_rows(
                    address,
                    fixture,
                    &format!("SELECT ?s WHERE {{ {pattern} FILTER(?o = {constant}) }}"),
                    &BTreeSet::from(["http://example.test/numeric/0".into()]),
                );
            }
        }
        database.assert_encrypted_sessions();
        drop(server);
        sql(database, "DROP TABLE sf_numeric_items");
    }
}

#[test]
#[ignore = "requires owned pinned MySQL TLS fixture"]
fn mysql_native_double_float_override_follows_wire_lexicals() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, false);
    assert_double_override(&fixture, &database);
}

fn assert_double_override(fixture: &Fixture, database: &Database) {
    mappings(fixture);
    let mut values = vec![
        1.0000000596046448_f64,
        1.0000001788139343,
        1024. + 2_f64.powi(-14), // exact midpoint 1024.00006103515625
        0.,
        -0.,
        1.1,
        -1.1,
        f64::MAX,
        -f64::MAX,
        f64::from_bits(1),
        -f64::from_bits(1),
    ];
    for bits in [
        0_u32,
        1,
        2,
        0x007f_fffe,
        0x007f_ffff,
        0x0080_0000,
        0x3f7f_ffff,
        0x3f80_0001,
        0x407f_ffff,
        0x4a00_0000,
        0x7f7f_fffe,
    ] {
        let midpoint = (f64::from(f32::from_bits(bits)) + f64::from(f32::from_bits(bits + 1))) / 2.;
        for offset in [-1_i64, 0, 1] {
            let v = f64::from_bits((midpoint.to_bits() as i64 + offset) as u64);
            values.extend([v, -v]);
        }
    }
    let overflow = 2_f64.powi(128) - 2_f64.powi(103);
    for offset in [-1_i64, 0, 1] {
        let v = f64::from_bits((overflow.to_bits() as i64 + offset) as u64);
        values.extend([v, -v]);
    }
    let lexicals: Vec<_> = values.iter().map(ToString::to_string).collect();
    let source: Vec<_> = values.iter().map(|v| format!("{v:e}")).collect();
    seed_native(database, "DOUBLE", &source);
    let (server, address) = start(fixture, database);
    assert_rows(address,fixture,&format!("SELECT ?s WHERE {{ VALUES ?s {{ <http://example.test/numeric/0> <http://example.test/numeric/1> }} ?s <http://example.test/float> ?o FILTER(?o = \"1.00000011920928955078125\"^^<{XSD}float>) }}"), &BTreeSet::from(["http://example.test/numeric/0".into(),"http://example.test/numeric/1".into()]));
    assert_rows(address,fixture,&format!("SELECT ?s WHERE {{ VALUES ?s {{ <http://example.test/numeric/2> }} ?s <http://example.test/float> ?o FILTER(?o = \"1024.0001220703125\"^^<{XSD}float>) }}"), &BTreeSet::from(["http://example.test/numeric/2".into()]));
    check_values(address, fixture, &lexicals, &["float"]);
    database.assert_encrypted_sessions();
    drop(server);
    policy(fixture, database, &lexicals);
    sql(database, "DROP TABLE sf_numeric_items");
}

pub(super) fn assert_all(fixture: &Fixture, database: &Database) {
    assert_float(fixture, database);
    assert_double_override(fixture, database);
    assert_identity(fixture, database);
    iri::assert_all(fixture, database);
    iri::assert_double(fixture, database);
    iri::mixed::assert_all(fixture, database);
}

#[test]
#[ignore = "requires owned pinned MySQL TLS fixture"]
fn mysql_native_float_identity_preserves_terms() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, false);
    assert_identity(&fixture, &database);
}

fn assert_identity(fixture: &Fixture, database: &Database) {
    for storage in ["FLOAT", "DOUBLE"] {
        mappings(fixture);
        seed_native(
            database,
            storage,
            &[
                "1.1".into(),
                "1.1".into(),
                "2".into(),
                "0".into(),
                "-0e0".into(),
            ],
        );
        let (server, address) = start(fixture, database);
        for predicate in ["natural", "double"] {
            for (lexical, ids) in [
                ("0.0E0", vec![3]),
                ("-0.0E0", vec![4]),
                ("1.1E0", vec![0, 1]),
                ("1.10E0", vec![]),
                ("0", vec![]),
                ("NaN", vec![]),
                ("INF", vec![]),
            ] {
                let literal = format!("\"{lexical}\"^^<{XSD}double>");
                let expected = ids
                    .iter()
                    .map(|i| format!("http://example.test/numeric/{i}"))
                    .collect();
                for body in [
                    format!("?s <http://example.test/{predicate}> {literal}"),
                    format!(
                        "?s <http://example.test/{predicate}> ?o FILTER(sameTerm(?o, {literal}))"
                    ),
                    format!(
                        "?s <http://example.test/{predicate}> ?o FILTER(sameTerm({literal}, ?o))"
                    ),
                ] {
                    assert_rows(
                        address,
                        fixture,
                        &format!("SELECT ?s WHERE {{ {body} }}"),
                        &expected,
                    );
                }
                let complement = (0..5)
                    .filter(|i| !ids.contains(i))
                    .map(|i| format!("http://example.test/numeric/{i}"))
                    .collect();
                assert_rows(address, fixture, &format!("SELECT ?s WHERE {{ ?s <http://example.test/{predicate}> ?o FILTER(!sameTerm(?o, {literal})) }}"), &complement);
            }
        }
        assert_rows(address, fixture, "SELECT ?s WHERE { ?s <http://example.test/natural> ?o; <http://example.test/double> ?o }", &(0..5).map(|i| format!("http://example.test/numeric/{i}")).collect());
        database.assert_encrypted_sessions();
        drop(server);
        let mapping = std::fs::read_to_string(fixture.root.join("first.ttl")).unwrap();
        fixture.write(
            "first.ttl",
            &mapping.replace(
                "rr:subjectMap [rr:template \"http://example.test/numeric/{id}\"]",
                "rr:subject <http://example.test/numeric/item>",
            ),
        );
        let (server, address) = start(fixture, database);
        for predicate in ["natural", "double", "float"] {
            for select in ["SELECT ?o", "SELECT DISTINCT ?o"] {
                let query = format!("{select} WHERE {{ <http://example.test/numeric/item> <http://example.test/{predicate}> ?o }}");
                let result = rows(address, fixture, &query);
                assert_eq!(result.len(), 4, "{storage}: {query}");
                let values: BTreeSet<_> = result
                    .iter()
                    .map(|r| r["o"]["value"].as_str().unwrap())
                    .collect();
                let expected = if predicate == "float" {
                    BTreeSet::from(["0", "-0", "1.1", "2"])
                } else {
                    BTreeSet::from(["0.0E0", "-0.0E0", "1.1E0", "2.0E0"])
                };
                assert_eq!(values, expected, "{storage}: {query}");
            }
        }
        database.assert_encrypted_sessions();
        drop(server);
        sql(database, "DROP TABLE sf_numeric_items");
    }
}

fn mappings(fixture: &Fixture) {
    fixture.write("first.ttl",&format!(r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
      <#m> rr:logicalTable [rr:tableName "sf_numeric_items"]; rr:subjectMap [rr:template "http://example.test/numeric/{{id}}"];
      rr:predicateObjectMap [rr:predicate <http://example.test/float>; rr:objectMap [rr:column "v"; rr:datatype <{XSD}float>]];
      rr:predicateObjectMap [rr:predicate <http://example.test/double>; rr:objectMap [rr:column "v"; rr:datatype <{XSD}double>]];
      rr:predicateObjectMap [rr:predicate <http://example.test/natural>; rr:objectMap [rr:column "v"]];
      rr:predicateObjectMap [rr:predicate <http://example.test/marker>; rr:objectMap [rr:column "visible"]]."#));
    fixture.write("ontology.ttl",&["float","double","natural","marker"].map(|p|format!("<http://example.test/{p}> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .")).join("\n"));
}

fn seed_native(database: &Database, storage: &str, values: &[String]) {
    let inserts = values
        .iter()
        .enumerate()
        .map(|(i, v)| format!("({i},CAST('{v}' AS DOUBLE),'same')"))
        .collect::<Vec<_>>()
        .join(",");
    sql(database,&format!("CREATE TABLE sf_numeric_items(id BIGINT PRIMARY KEY,v {storage},visible VARCHAR(32)); INSERT INTO sf_numeric_items VALUES {inserts},(9999,NULL,'same')"));
}

fn check_values(address: SocketAddr, fixture: &Fixture, values: &[String], predicates: &[&str]) {
    for predicate in predicates {
        let kind = if *predicate == "natural" {
            "double"
        } else {
            predicate
        };
        // Projection keeps original decoder spelling (authored Float) or Rust
        // natural canonical Double; the SQL comparison value is never projected.
        let output = rows(
            address,
            fixture,
            &format!(
                "SELECT ?s ?o WHERE {{ ?s <http://example.test/{predicate}> ?o FILTER(?o = ?o) }}"
            ),
        );
        assert_eq!(output.len(), values.len());
        for row in output {
            let id: usize = row["s"]["value"]
                .as_str()
                .unwrap()
                .rsplit('/')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            let mut expected = values[id].clone();
            if kind == "double" {
                expected.clear();
                sf_core::datatype::canonical_lexical(
                    &values[id],
                    sf_core::datatype::XsdTypeCode::Double,
                    &mut expected,
                )
                .unwrap();
            }
            assert_eq!(row["o"]["value"], expected);
            assert_eq!(row["o"]["datatype"], format!("{XSD}{kind}"));
        }
        for (right, rkind) in [
            ("1.1", "double"),
            ("0", "float"),
            ("1.00000011920928955078125", "float"),
            ("1024.0001220703125", "double"),
            ("NaN", "float"),
            ("INF", "float"),
            ("-INF", "double"),
            ("inf", "double"),
        ] {
            let double = kind == "double" || rkind == "double";
            let r = promoted(right, rkind, double);
            for op in ["=", "!=", "<", "<=", ">", ">="] {
                for reverse in [false, true] {
                    for negate in [false, true] {
                        let c = format!("\"{right}\"^^<{XSD}{rkind}>");
                        let expression = if reverse {
                            format!("{c} {op} ?o")
                        } else {
                            format!("?o {op} {c}")
                        };
                        let expression = if negate {
                            format!("!({expression})")
                        } else {
                            expression
                        };
                        let expected = values
                            .iter()
                            .enumerate()
                            .filter_map(|(i, v)| {
                                let l = promoted(v, kind, double)?;
                                let r = r?;
                                let yes = if reverse {
                                    compare_values(r, l, op)
                                } else {
                                    compare_values(l, r, op)
                                };
                                (yes != negate).then(|| format!("http://example.test/numeric/{i}"))
                            })
                            .collect();
                        assert_rows(address,fixture,&format!("SELECT ?s WHERE {{ ?s <http://example.test/{predicate}> ?o FILTER({expression}) }}"),&expected);
                    }
                }
            }
        }
    }
}
