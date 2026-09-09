//! Public native floating source values must follow their actual wire decoder.
use super::*;
#[path = "mysql_native_float_values.rs"]
mod float_overrides;

#[test]
#[ignore = "requires owned pinned MySQL TLS fixture"]
fn mysql_native_double_values_are_exact() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, false);
    assert_all(&fixture, &database);
}

pub(super) fn assert_all(fixture: &Fixture, database: &Database) {
    fixture.write("first.ttl", &format!(r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
      <#m> rr:logicalTable [rr:tableName "sf_numeric_items"]; rr:subjectMap [rr:template "http://example.test/numeric/{{id}}"];
      rr:predicateObjectMap [rr:predicate <http://example.test/double>; rr:objectMap [rr:column "v"; rr:datatype <{XSD}double>]];
      rr:predicateObjectMap [rr:predicate <http://example.test/natural>; rr:objectMap [rr:column "v"]];
      rr:predicateObjectMap [rr:predicate <http://example.test/marker>; rr:objectMap [rr:column "visible"]]."#));
    fixture.write("ontology.ttl", & ["double", "natural", "marker"].map(|p| format!("<http://example.test/{p}> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .")).join("\n"));
    sql(
        database,
        "CREATE TABLE sf_numeric_items(id BIGINT PRIMARY KEY,v DOUBLE,visible VARCHAR(32))",
    );
    let values = [
        1.1,
        1.1,
        2.,
        0.,
        -0.,
        -1.1,
        f64::from_bits(1),
        -f64::from_bits(1),
        f64::MIN_POSITIVE,
        f64::MAX,
        -f64::MAX,
        f64::from_bits(1f64.to_bits() + 1),
        1.0000000596046448,
    ];
    let inserts = values
        .iter()
        .enumerate()
        .map(|(i, v)| format!("({i},CAST('{v:e}' AS DOUBLE),'same')"))
        .collect::<Vec<_>>()
        .join(",");
    sql(
        database,
        &format!("INSERT INTO sf_numeric_items VALUES {inserts},(9999,NULL,'same')"),
    );
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
    for predicate in ["double", "natural"] {
        // Matching explicit and natural construction both emit canonical Double;
        // value filtering must not substitute a SQL lexical rendering.
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
            let mut canonical = String::new();
            sf_core::datatype::canonical_lexical(
                &values[id].to_string(),
                sf_core::datatype::XsdTypeCode::Double,
                &mut canonical,
            )
            .unwrap();
            assert_eq!(row["o"]["value"], canonical);
            assert_eq!(row["o"]["datatype"], format!("{XSD}double"));
        }
        for (right, kind) in [
            ("1.1", "double"),
            ("0", "double"),
            ("5e-324", "double"),
            ("1.7976931348623157e308", "double"),
            ("1.00000011920928955078125", "float"),
            ("NaN", "double"),
            ("INF", "double"),
            ("-INF", "double"),
            ("inf", "double"),
        ] {
            let right_value = promoted(right, kind, true);
            for op in ["=", "!=", "<", "<=", ">", ">="] {
                for reverse in [false, true] {
                    for negate in [false, true] {
                        let constant = format!("\"{right}\"^^<{XSD}{kind}>");
                        let comparison = if reverse {
                            format!("{constant} {op} ?o")
                        } else {
                            format!("?o {op} {constant}")
                        };
                        let comparison = if negate {
                            format!("!({comparison})")
                        } else {
                            comparison
                        };
                        let expected = values
                            .iter()
                            .enumerate()
                            .filter_map(|(i, v)| {
                                let r = right_value?;
                                let yes = if reverse {
                                    compare_values(r, *v, op)
                                } else {
                                    compare_values(*v, r, op)
                                };
                                (yes != negate).then(|| format!("http://example.test/numeric/{i}"))
                            })
                            .collect();
                        assert_rows(address, fixture, &format!("SELECT ?s WHERE {{ ?s <http://example.test/{predicate}> ?o FILTER({comparison}) }}"), &expected);
                    }
                }
            }
        }
    }
    for op in ["=", "!=", "<", "<=", ">", ">="] {
        let expected = values
            .iter()
            .enumerate()
            .filter(|(_, v)| compare_values(**v, **v, op))
            .map(|(i, _)| format!("http://example.test/numeric/{i}"))
            .collect();
        assert_rows(address, fixture, &format!("SELECT ?s WHERE {{ ?s <http://example.test/double> ?a; <http://example.test/natural> ?b FILTER(?a {op} ?b) }}"), &expected);
    }
    database.assert_encrypted_sessions();
    drop(server);
    sql(
        database,
        "UPDATE sf_numeric_items SET visible=CASE WHEN id % 2=0 THEN 'same' ELSE 'denied' END",
    );
    let (server, address) = policy_server(fixture, database);
    let subjects = (0..values.len())
        .chain([9999])
        .map(|i| format!("<http://example.test/numeric/{i}>"))
        .collect::<Vec<_>>()
        .join(" ");
    for op in ["=", "!=", "<", "<=", ">", ">="] {
        let expected: BTreeSet<_> = values
            .iter()
            .enumerate()
            .filter(|(i, v)| i % 2 == 0 && compare_values(**v, 1., op))
            .map(|(i, _)| format!("http://example.test/numeric/{i}"))
            .collect();
        let body = format!("?s <http://example.test/natural> ?o FILTER(?o {op} 1e0)");
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
        let output=rows(address,fixture,&format!("SELECT ?s ?marker WHERE {{ VALUES ?s {{ {subjects} }} OPTIONAL {{ {body} . ?s <http://example.test/marker> ?marker }} }}"));
        assert_eq!(output.len(), values.len() + 1);
        for row in output {
            assert_eq!(
                row.get("marker").is_some(),
                expected.contains(row["s"]["value"].as_str().unwrap())
            );
        }
    }
    database.assert_encrypted_sessions();
    drop(server);
    sql(database, "DROP TABLE sf_numeric_items");
    for storage in ["DOUBLE(20,4)", "DOUBLE(20,4) UNSIGNED ZEROFILL"] {
        sql(database,&format!("CREATE TABLE sf_numeric_items(id BIGINT PRIMARY KEY,v {storage},visible VARCHAR(32)); INSERT INTO sf_numeric_items VALUES(0,1.1,'same'),(1,2.2,'same'),(9999,NULL,'same')"));
        let (server, address) = start(fixture, database);
        for predicate in ["natural", "double"] {
            let body = format!("?s <http://example.test/{predicate}> ?o");
            for pattern in [
                body.clone(),
                format!("{{ SELECT ?s ?o WHERE {{ {body} }} }}"),
            ] {
                assert_rows(
                    address,
                    fixture,
                    &format!("SELECT ?s WHERE {{ {pattern} FILTER(?o = 1.1e0) }}"),
                    &BTreeSet::from(["http://example.test/numeric/0".into()]),
                );
            }
        }
        database.assert_encrypted_sessions();
        drop(server);
        sql(database, "DROP TABLE sf_numeric_items");
    }
    float_overrides::assert_all(fixture, database);
}
