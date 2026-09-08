//! Public ordinary RDF equality under owned native case-insensitive collations.
use super::*;

pub(super) fn assert_ordinary(address: SocketAddr, fixture: &Fixture) {
    let node = |s: &str| format!("http://example.test/n/{}", s.replace(' ', "%20"));
    let expected: BTreeSet<_> = [("a", "B"), ("b", "c"), ("A", "B"), ("s", "a "), ("a", "z")]
        .map(|(s, o)| (node(s), node(o)))
        .into_iter()
        .collect();
    for distinct in ["", "DISTINCT "] {
        let query = format!("SELECT {distinct}?s ?o WHERE {{ ?s <{EDGE}> ?o }}");
        let result = rows(address, fixture, &query);
        let actual: BTreeSet<_> = result
            .iter()
            .map(|row| {
                (
                    row["s"]["value"].as_str().unwrap().to_owned(),
                    row["o"]["value"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        assert_eq!(actual, expected, "{query}");
        assert_eq!(result.len(), expected.len(), "{query}");
    }
    let count = rows(
        address,
        fixture,
        &format!("SELECT (COUNT(*) AS ?n) WHERE {{ ?s <{EDGE}> ?o }}"),
    );
    assert_eq!(count[0]["n"]["value"], "5");
    for query in [
        format!("ASK {{ <{}> <{EDGE}> ?o }}", node("B")),
        format!("ASK {{ ?s <{EDGE}> <{}> }}", node("C")),
    ] {
        let (status, body) = request_format(
            address,
            &query,
            Some(&fixture.token),
            "application/sparql-results+json",
        )
        .unwrap();
        assert_eq!(status, 200, "{query}");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["boolean"],
            false,
            "{query}"
        );
    }
    for (tail, expected) in [
        (format!(". ?o <{EDGE}> ?next"), 0),
        (format!("OPTIONAL {{ ?o <{EDGE}> ?next }}"), 5),
        (format!("FILTER EXISTS {{ ?o <{EDGE}> ?next }}"), 0),
        (format!("FILTER NOT EXISTS {{ ?o <{EDGE}> ?next }}"), 5),
        (format!("MINUS {{ ?o <{EDGE}> ?next }}"), 5),
    ] {
        let query = format!("SELECT ?s ?o ?next WHERE {{ ?s <{EDGE}> ?o {tail} }}");
        let result = rows(address, fixture, &query);
        assert_eq!(result.len(), expected, "{query}");
        assert!(result.iter().all(|r| r.get("next").is_none()), "{query}");
    }
}

pub(super) fn assert_references(fixture: &Fixture, database: &Database, postgres: bool) {
    let text = if postgres {
        "VARCHAR(32) COLLATE path_ci"
    } else {
        "VARCHAR(32) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"
    };
    sql(database, &format!("CREATE TABLE ref_child(s {text}, fk {text}); CREATE TABLE ref_parent(k {text}, label {text}); INSERT INTO ref_child VALUES ('one','a'),('two','a'),('three','b'); INSERT INTO ref_parent VALUES ('A','target'),('a','target'),('b','other'),('b','Other')"));
    if postgres {
        sql(database, "GRANT SELECT ON ref_child, ref_parent TO sf_tls");
    }
    let mapping = format!(
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#parent> rr:logicalTable [rr:tableName "ref_parent"]; rr:subjectMap [rr:template "http://example.test/n/{{LABEL}}"].
<#child> rr:logicalTable [rr:tableName "ref_child"]; rr:subjectMap [rr:template "http://example.test/n/{{S}}"];
 rr:predicateObjectMap [rr:predicate <{EDGE}>; rr:objectMap [rr:parentTriplesMap <#parent>; rr:joinCondition [rr:child "FK"; rr:parent "K"]]]."#
    );
    fixture.write("first.ttl", &mapping);
    let (server, address) = start(fixture, database);
    for (query, expected) in [
        (format!("SELECT ?o WHERE {{ ?s <{EDGE}> ?o }}"), 4),
        (format!("SELECT DISTINCT ?o WHERE {{ ?s <{EDGE}> ?o }}"), 3),
        (format!("SELECT ?o WHERE {{ <http://example.test/n/one> <{EDGE}> ?o }}"), 1),
        (format!("SELECT ?s WHERE {{ ?s <{EDGE}> <http://example.test/n/target> }}"), 2),
        (format!("SELECT ?s WHERE {{ ?s <{EDGE}> <http://example.test/n/TARGET> }}"), 0),
        (format!("SELECT ?s ?o ?r WHERE {{ ?s <{EDGE}> ?o OPTIONAL {{ ?r <{EDGE}> ?o }} }}"), 6),
        (format!("SELECT ?s ?o WHERE {{ ?s <{EDGE}> ?o FILTER EXISTS {{ ?s <{EDGE}> <http://example.test/n/target> }} }}"), 2),
        (format!("SELECT ?s ?o WHERE {{ ?s <{EDGE}> ?o FILTER NOT EXISTS {{ ?s <{EDGE}> <http://example.test/n/target> }} }}"), 2),
        (format!("SELECT ?s ?o WHERE {{ ?s <{EDGE}> ?o MINUS {{ ?s <{EDGE}> <http://example.test/n/target> }} }}"), 2),
    ] {
        let answer = std::panic::catch_unwind(|| rows(address, fixture, &query))
            .unwrap_or_else(|_| panic!("native Ref query failed: {query}; server: {}", std::fs::read_to_string(fixture.root.join("query-profile.stderr")).unwrap()));
        assert_eq!(answer.len(), expected, "{query}");
    }
    let count = rows(
        address,
        fixture,
        &format!("SELECT (COUNT(*) AS ?n) WHERE {{ ?s <{EDGE}> ?o }}"),
    );
    assert_eq!(count[0]["n"]["value"], "4");
    database.assert_encrypted_sessions();
    drop(server);
    fixture.write("first.ttl", MAPPING);
}
