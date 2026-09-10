//! Unlike wire widths can have equal numbers with unequal IRI spellings, or
//! unequal numbers with equal spellings. Neither SQL value equality nor a
//! widening cast is an RDF identity key.
use super::*;

#[test]
#[ignore = "requires owned pinned MySQL TLS fixture"]
fn mysql_mixed_float_template_iris_are_exact() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, false);
    assert_all(&fixture, &database);
}

pub(in super::super) fn assert_all(fixture: &Fixture, database: &Database) {
    let floats = native_float_values();
    let pairs = floats[..15]
        .iter()
        .flat_map(|v| {
            [
                (*v, f64::from(*v)),
                (*v, v.to_string().parse::<f64>().unwrap()),
            ]
        })
        .collect::<Vec<_>>();
    // Deliberate opposite witnesses: SQL numeric equality gets both wrong.
    assert_ne!(pairs[0].0.to_string(), pairs[0].1.to_string());
    assert_eq!(pairs[1].0.to_string(), pairs[1].1.to_string());
    assert_ne!(f64::from(pairs[1].0), pairs[1].1);
    sql(
        database,
        "CREATE TABLE sf_numeric_items(id BIGINT PRIMARY KEY,v FLOAT,w DOUBLE,visible VARCHAR(32))",
    );
    let inserts = pairs
        .iter()
        .enumerate()
        .map(|(i, (v, w))| {
            format!(
                "({i},CAST('{:e}' AS DOUBLE),CAST('{w:e}' AS DOUBLE),'same')",
                f64::from(*v)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    sql(
        database,
        &format!("INSERT INTO sf_numeric_items VALUES {inserts},(9999,NULL,NULL,'same')"),
    );
    fixture.write(
        "first.ttl",
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
      <#m> rr:logicalTable [rr:tableName "sf_numeric_items"];
      rr:subjectMap [rr:template "http://example.test/numeric/{id}"];
      rr:predicateObjectMap [rr:predicate <http://example.test/edge4>;
        rr:objectMap [rr:template "http://example.test/float/{v}"]];
      rr:predicateObjectMap [rr:predicate <http://example.test/edge8>;
        rr:objectMap [rr:template "http://example.test/float/{w}"]];
      rr:predicateObjectMap [rr:predicate <http://example.test/literal4>;
        rr:objectMap [rr:template "{v}"; rr:datatype <http://www.w3.org/2001/XMLSchema#float>]];
      rr:predicateObjectMap [rr:predicate <http://example.test/literal8>;
        rr:objectMap [rr:template "{w}"; rr:datatype <http://www.w3.org/2001/XMLSchema#float>]]."#,
    );
    fixture.write(
        "ontology.ttl",
        &["edge4", "edge8", "literal4", "literal8"]
            .map(|p| {
                let kind = if p.starts_with("literal") {
                    "DatatypeProperty"
                } else {
                    "ObjectProperty"
                };
                format!("<http://example.test/{p}> a <http://www.w3.org/2002/07/owl#{kind}> .")
            })
            .join("\n"),
    );
    let (server, address) = start(fixture, database);
    check(address, fixture, &pairs, false);
    database.assert_encrypted_sessions();
    drop(server);
    sql(
        database,
        "UPDATE sf_numeric_items SET visible=CASE WHEN id % 2=0 THEN 'same' ELSE 'denied' END",
    );
    let (server, address) = policy_server(fixture, database);
    check(address, fixture, &pairs, true);
    database.assert_encrypted_sessions();
    drop(server);
    sql(database, "DROP TABLE sf_numeric_items");
}

fn check(address: SocketAddr, fixture: &Fixture, pairs: &[(f32, f64)], policy: bool) {
    let visible = |i: usize| !policy || i.is_multiple_of(2);
    let expected: BTreeSet<_> = pairs
        .iter()
        .enumerate()
        .filter(|(i, _)| visible(*i))
        .flat_map(|(i, (v, _))| {
            pairs.iter().enumerate().filter_map(move |(j, (_, w))| {
                (visible(j) && v.to_string() == w.to_string()).then_some((i, j))
            })
        })
        .collect();
    for (left, right) in [("edge4", "edge8"), ("edge8", "edge4")] {
        for body in [
            format!("?s <http://example.test/{left}> ?o . ?t <http://example.test/{right}> ?o"),
            format!("?s <http://example.test/{left}> ?o . ?t <http://example.test/{right}> ?p FILTER(?o = ?p)"),
            format!("?s <http://example.test/{left}> ?o . ?t <http://example.test/{right}> ?p FILTER(sameTerm(?p,?o))"),
            format!("{{ SELECT ?s ?o WHERE {{ ?s <http://example.test/{left}> ?o }} }} ?t <http://example.test/{right}> ?o"),
        ] {
            let result = rows(address, fixture, &format!("SELECT ?s ?t WHERE {{ {body} }}"));
            let actual: BTreeSet<_> = result.iter().map(|r| if left=="edge4" {
                (id(r,"s"),id(r,"t"))
            } else { (id(r,"t"),id(r,"s")) }).collect();
            assert_eq!(result.len(), expected.len(), "exact mixed-width bag: {body}");
            assert_eq!(actual, expected, "mixed-width RDF identity: {body}");
        }
    }
    let all_pairs = pairs
        .iter()
        .enumerate()
        .filter(|(i, _)| visible(*i))
        .flat_map(|(i, _)| {
            pairs
                .iter()
                .enumerate()
                .filter_map(move |(j, _)| visible(j).then_some((i, j)))
        })
        .collect::<BTreeSet<_>>();
    for filter in ["?o != ?p", "!sameTerm(?o,?p)"] {
        // Keep each response below the fixture reader's existing 64KiB bound;
        // all pairs are still checked, without changing product/test limits.
        let result = (0..pairs.len()).collect::<Vec<_>>().chunks(5).flat_map(|batch| {
            let subjects = batch.iter().map(|i| format!("<http://example.test/numeric/{i}>")).collect::<Vec<_>>().join(" ");
            rows(address,fixture, &format!("SELECT ?s ?t WHERE {{ VALUES ?s {{ {subjects} }} ?s <http://example.test/edge4> ?o . ?t <http://example.test/edge8> ?p FILTER({filter}) }}"))
        }).collect::<Vec<_>>();
        let unequal = all_pairs
            .difference(&expected)
            .copied()
            .collect::<BTreeSet<_>>();
        assert_eq!(result.len(), unequal.len());
        assert_eq!(
            result
                .iter()
                .map(|r| (id(r, "s"), id(r, "t")))
                .collect::<BTreeSet<_>>(),
            unequal
        );
    }
    let subjects = (0..pairs.len())
        .map(|i| format!("<http://example.test/numeric/{i}>"))
        .collect::<Vec<_>>()
        .join(" ");
    let same_row = pairs
        .iter()
        .enumerate()
        .filter_map(|(i, (v, w))| {
            (visible(i) && v.to_string() == w.to_string())
                .then_some(format!("http://example.test/numeric/{i}"))
        })
        .collect();
    let body = "?s <http://example.test/edge4> ?o; <http://example.test/edge8> ?o";
    for expression in ["?a = ?b", "!(?a = ?b)"] {
        let query = format!("SELECT ?s WHERE {{ ?s <http://example.test/literal4> ?a; <http://example.test/literal8> ?b FILTER({expression}) }}");
        let response = stop_matrix::wire(cancellation::begin(address, &query, &fixture.token));
        assert!(
            response.starts_with(b"HTTP/1.1 501 "),
            "typed-template values must not borrow lexical identity: {}",
            String::from_utf8_lossy(&response)
        );
    }
    assert_rows(address, fixture, "SELECT ?s WHERE { ?s <http://example.test/literal4> ?a; <http://example.test/literal8> ?b FILTER(sameTerm(?a,?b)) }", &same_row);
    assert_rows(
        address,
        fixture,
        &format!("SELECT ?s WHERE {{ VALUES ?s {{ {subjects} }} FILTER EXISTS {{ {body} }} }}"),
        &same_row,
    );
    let result = rows(
        address,
        fixture,
        &format!("SELECT ?s ?o WHERE {{ VALUES ?s {{ {subjects} }} OPTIONAL {{ {body} }} }}"),
    );
    assert_eq!(result.len(), pairs.len());
    for row in result {
        let i = id(&row, "s");
        assert_eq!(
            row.get("o").is_some(),
            same_row.contains(&format!("http://example.test/numeric/{i}"))
        );
        if let Some(o) = row.get("o") {
            assert_eq!(
                o["value"],
                format!("http://example.test/float/{}", pairs[i].0)
            );
        }
    }
}

fn id(row: &serde_json::Value, key: &str) -> usize {
    row[key]["value"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .parse()
        .unwrap()
}
