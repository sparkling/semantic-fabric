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
