//! Actual results and origins under source RLS, never an ordinary-query receipt.
use super::*;
use oxrdf::{GraphName, Literal, NamedNode, Quad, Term, Triple};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashSet};

pub(super) const FORMAT: &str = "application/vnd.semantic-fabric.lineage+json-seq";

#[derive(Clone, Copy)]
pub(super) enum Shape {
    Constant,
    Multiple,
    Union,
    Join,
}

pub(super) async fn empty(cfg: Arc<ServeConfig>, token: &str, denied: &str) {
    let query = format!(
        "SELECT ?name WHERE {{ ?s <http://ex/name> ?name . ?s <http://ex/name> \"{denied}\" }}"
    );
    let response = request_format(cfg, token, &query, FORMAT).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(!text.contains("Alice") && !text.contains("Bob"));
    let records: Vec<Value> = text
        .split('\u{1e}')
        .skip(1)
        .map(|part| serde_json::from_str(part).unwrap())
        .collect();
    assert_eq!(
        records.len(),
        2,
        "empty authorized result has no solution/activity"
    );
    assert_eq!(records[0]["type"], "header");
    assert_eq!(records[1], json!({"type":"complete", "solutions":0}));
}

pub(super) async fn check(
    cfg: Arc<ServeConfig>,
    token: &str,
    query: &str,
    allowed: &str,
    denied: &str,
    shape: Shape,
) {
    let response = request_format(cfg, token, query, FORMAT).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], FORMAT);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert!(bytes.len() <= 65536);
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.contains(allowed));
    for secret in [
        denied, ALICE, BOB, "opaque-a", "opaque-b", "tenant-a", "tenant-b",
    ] {
        assert!(
            !text.contains(secret),
            "lineage exposed another row or raw identity"
        );
    }
    assert_eq!(bytes.first(), Some(&0x1e));
    assert_eq!(bytes.last(), Some(&b'\n'));
    let records: Vec<Value> = text
        .split('\u{1e}')
        .skip(1)
        .map(|part| serde_json::from_str(part).unwrap())
        .collect();
    let graph = query.starts_with("CONSTRUCT");
    let profile = match (shape, graph) {
        (Shape::Constant, false) => "constant-mapping-source-v1",
        (Shape::Constant, true) => "constant-mapping-source-graph-v1",
        (Shape::Multiple, false) => "bounded-mapping-source-v1",
        (Shape::Multiple, true) => "bounded-mapping-source-graph-v1",
        (Shape::Union, false) => "bounded-federated-union-lineage-v1",
        (Shape::Join, false) => "bounded-federated-join-lineage-v1",
        _ => panic!("unadvertised graph federation"),
    };
    let count = if matches!(shape, Shape::Union) { 2 } else { 1 };
    assert_eq!(records.len(), count + 2);
    let header = &records[0];
    assert_eq!(header["type"], "header");
    assert_eq!(header["profile"], profile);
    assert_eq!(header["rowKeys"], "not-provided");
    assert_eq!(records.last().unwrap()["type"], "complete");
    assert_eq!(records.last().unwrap()["solutions"], count);
    if graph {
        assert_eq!(records.last().unwrap()["tripleOccurrences"], 1);
    }
    for (ordinal, record) in records[1..records.len() - 1].iter().enumerate() {
        assert_eq!(record["ordinal"], ordinal);
        let quads = if graph {
            assert_eq!(record["type"], "graph-solution");
            let quads = oxttl::NQuadsParser::new()
                .for_slice(record["dataset"].as_str().unwrap().as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let id = if allowed == "Alice" { 1 } else { 2 };
            let expected = Triple::new(
                NamedNode::new(format!("http://ex/parent/{id}")).unwrap(),
                NamedNode::new("http://ex/name").unwrap(),
                Literal::new_simple_literal(allowed),
            );
            let product: HashSet<_> = quads
                .iter()
                .filter(|q| q.graph_name == GraphName::DefaultGraph)
                .map(|q| Triple::new(q.subject.clone(), q.predicate.clone(), q.object.clone()))
                .collect();
            assert_eq!(product, HashSet::from([expected.clone()]));
            let reifies: Vec<_> = quads
                .iter()
                .filter(|q| q.predicate == oxrdf::vocab::rdf::REIFIES)
                .collect();
            assert_eq!(reifies.len(), 1);
            assert_ne!(reifies[0].graph_name, GraphName::DefaultGraph);
            assert_eq!(reifies[0].object, Term::Triple(Box::new(expected)));
            quads
        } else {
            assert_eq!(record["type"], "solution");
            assert_eq!(record["result"]["head"]["vars"], json!(["name"]));
            assert_eq!(
                record["result"]["results"]["bindings"],
                json!([{"name":{"type":"literal","value":allowed}}])
            );
            let provenance = record["provenance"].to_string();
            assert!(
                !provenance.contains(allowed),
                "SELECT proof must not log source values"
            );
            oxjsonld::JsonLdParser::new()
                .for_slice(provenance.as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        let sources: &[u64] = match shape {
            Shape::Union => {
                if ordinal == 0 {
                    &[0]
                } else {
                    &[1]
                }
            }
            Shape::Join => &[0, 1],
            _ => &[0],
        };
        metadata(&quads, header, shape, sources);
    }
}

fn metadata(quads: &[Quad], header: &Value, shape: Shape, sources: &[u64]) {
    for field in ["snapshot", "logicalPlan", "policy"] {
        let values: HashSet<_> = quads
            .iter()
            .filter(|q| q.predicate.as_str() == format!("urn:semantic-fabric:lineage:{field}"))
            .map(|q| q.object.clone())
            .collect();
        assert_eq!(
            values,
            HashSet::from([Term::from(
                NamedNode::new(header[field].as_str().unwrap()).unwrap()
            )])
        );
    }
    let values = |predicate: &str| -> BTreeSet<String> {
        quads
            .iter()
            .filter(|q| q.predicate.as_str() == predicate)
            .map(|q| {
                let Term::Literal(value) = &q.object else {
                    panic!("literal metadata")
                };
                value.value().to_owned()
            })
            .collect()
    };
    assert_eq!(
        values("urn:semantic-fabric:lineage:sourceId"),
        sources.iter().map(u64::to_string).collect()
    );
    let maps = if matches!(shape, Shape::Multiple) {
        vec!["urn:rls-map:a", "urn:rls-map:b"]
    } else {
        vec!["http://example.com/base/#p"]
    };
    assert_eq!(
        values("urn:semantic-fabric:lineage:mappingId"),
        maps.into_iter().map(str::to_owned).collect()
    );
    assert!(!quads
        .iter()
        .any(|q| q.predicate.as_str() == "http://www.w3.org/ns/prov#wasDerivedFrom"));
    let used: BTreeSet<_> = quads
        .iter()
        .filter(|q| q.predicate.as_str() == "http://www.w3.org/ns/prov#used")
        .map(|q| {
            let Term::NamedNode(id) = &q.object else {
                panic!("opaque used identity")
            };
            id.as_str()
        })
        .collect();
    for prefix in [
        "urn:semantic-fabric:source:",
        "urn:semantic-fabric:mapping-document:",
    ] {
        assert_eq!(
            used.iter().filter(|id| id.starts_with(prefix)).count(),
            sources.len()
        );
    }
    if matches!(shape, Shape::Union | Shape::Join) {
        assert_eq!(header["sources"].as_array().unwrap().len(), 2);
        for &source in sources {
            for field in ["source", "mappingDocument"] {
                assert!(used.contains(header["sources"][source as usize][field].as_str().unwrap()));
            }
            if matches!(shape, Shape::Join) {
                let source = Term::from(
                    NamedNode::new(
                        header["sources"][source as usize]["source"]
                            .as_str()
                            .unwrap(),
                    )
                    .unwrap(),
                );
                let edge = quads
                    .iter()
                    .find(|q| {
                        q.predicate.as_str() == "urn:semantic-fabric:lineage:source"
                            && q.object == source
                    })
                    .unwrap();
                let oxrdf::NamedOrBlankNode::NamedNode(entry) = &edge.subject else {
                    panic!("mapping entry IRI")
                };
                assert!(used.contains(entry.as_str()));
                assert!(quads.iter().any(|q| q.subject == edge.subject
                    && q.predicate.as_str() == "urn:semantic-fabric:lineage:mappingId"
                    && q.object
                        == Term::Literal(Literal::new_simple_literal(
                            "http://example.com/base/#p"
                        ))));
            }
        }
    }
}
