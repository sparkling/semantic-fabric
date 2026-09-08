//! Native HTTP qualification of the actual opt-in lineage path, using only the
//! parent's owned TLS providers and serving-only CLI child.
use super::*;
use oxrdf::{vocab::rdf, GraphName, Literal, NamedNode, Quad, Term, Triple};
use serde_json::Value;
use std::collections::HashSet;

pub(super) const FORMAT: &str = "application/vnd.semantic-fabric.lineage+json-seq";
const GRAPH: &str = "CONSTRUCT { ?s <http://example.test/left> ?value } WHERE { ?s <http://example.test/left> ?value }";

fn records(address: SocketAddr, query: &str, token: &str) -> Vec<Value> {
    // The parent checks the media type and demands complete HTTP chunk framing.
    let (status, bytes) = request_format(address, query, Some(token), FORMAT).unwrap();
    assert_eq!(status, 200);
    assert_eq!(bytes.first(), Some(&0x1e));
    assert_eq!(bytes.last(), Some(&b'\n'));
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(!text.contains(token));
    let rows: Vec<Value> = text
        .split('\u{1e}')
        .skip(1)
        .map(|part| serde_json::from_str(part).unwrap())
        .collect();
    let header = &rows[0];
    assert_eq!(header["type"], "header");
    assert_eq!(header["mappingId"], "http://example.com/base/#items");
    assert_eq!(header["sourceId"], 0);
    assert_eq!(header["rowKeys"], "not-provided");
    for (field, domain) in [
        ("snapshot", "snapshot"),
        ("logicalPlan", "logical-plan"),
        ("policy", "policy"),
    ] {
        let value = header[field].as_str().unwrap();
        let digest = value
            .strip_prefix(&format!("urn:semantic-fabric:{domain}:"))
            .unwrap();
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
    assert_eq!(rows.last().unwrap()["type"], "complete");
    rows
}

fn assert_metadata(quads: &[Quad], header: &Value) {
    let used: Vec<_> = quads
        .iter()
        .filter(|q| q.predicate.as_str() == "http://www.w3.org/ns/prov#used")
        .map(|q| &q.object)
        .collect();
    assert_eq!(used.len(), 2);
    for domain in ["source", "mapping-document"] {
        assert!(used.iter().any(|term| match term {
            Term::NamedNode(node) => node
                .as_str()
                .starts_with(&format!("urn:semantic-fabric:{domain}:")),
            _ => false,
        }));
    }
    for field in ["snapshot", "logicalPlan", "policy"] {
        let matching: Vec<_> = quads
            .iter()
            .filter(|q| q.predicate.as_str() == format!("urn:semantic-fabric:lineage:{field}"))
            .collect();
        assert_eq!(matching.len(), 1);
        assert_eq!(
            matching[0].object,
            NamedNode::new(header[field].as_str().unwrap())
                .unwrap()
                .into()
        );
    }
    assert!(quads.iter().any(
        |q| q.predicate.as_str() == "urn:semantic-fabric:lineage:mappingId"
            && q.object
                == Literal::new_simple_literal(header["mappingId"].as_str().unwrap()).into()
    ));
    assert!(!quads
        .iter()
        .any(|q| q.predicate.as_str() == "http://www.w3.org/ns/prov#wasDerivedFrom"));
}

fn assert_select(rows: &[Value], expected: usize) {
    assert_eq!(rows[0]["profile"], "constant-mapping-source-v1");
    assert_eq!(rows.len(), expected + 2);
    assert_eq!(rows.last().unwrap()["solutions"], expected);
    let mut bundles = HashSet::new();
    for (ordinal, row) in rows[1..rows.len() - 1].iter().enumerate() {
        assert_eq!(row["type"], "solution");
        assert_eq!(row["ordinal"], ordinal);
        let results = &row["result"];
        assert_eq!(results["head"]["vars"], serde_json::json!(["s", "value"]));
        let bindings = results["results"]["bindings"].as_array().unwrap();
        assert_eq!(bindings.len(), 1);
        assert_eq!(
            bindings[0]["s"],
            serde_json::json!({"type":"uri", "value":"http://example.test/item"})
        );
        assert_eq!(bindings[0]["value"]["type"], "literal");
        assert_eq!(bindings[0]["value"]["value"], "same");
        let json = row["provenance"].to_string();
        assert!(!json.contains("same"));
        assert!(bundles.insert(row["provenance"]["@id"].as_str().unwrap()));
        let quads = oxjsonld::JsonLdParser::new()
            .for_slice(json.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_metadata(&quads, &rows[0]);
    }
}

fn assert_graph(rows: &[Value], expected: usize) {
    assert_eq!(rows[0]["profile"], "constant-mapping-source-graph-v1");
    assert_eq!(rows[0]["blankNodeScope"], "response");
    assert_eq!(rows[0]["productGraph"], "default");
    assert_eq!(rows.len(), expected + 2);
    assert_eq!(rows.last().unwrap()["solutions"], expected);
    assert_eq!(rows.last().unwrap()["tripleOccurrences"], expected);
    let document: String = rows
        .iter()
        .filter_map(|row| row["dataset"].as_str())
        .collect();
    let quads = oxttl::NQuadsParser::new()
        .for_slice(document.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let product: HashSet<_> = quads
        .iter()
        .filter(|q| q.graph_name == GraphName::DefaultGraph)
        .map(|q| Triple::new(q.subject.clone(), q.predicate.clone(), q.object.clone()))
        .collect();
    let expected_product = if expected == 0 {
        HashSet::new()
    } else {
        HashSet::from([Triple::new(
            NamedNode::new("http://example.test/item").unwrap(),
            NamedNode::new("http://example.test/left").unwrap(),
            Literal::new_simple_literal("same"),
        )])
    };
    assert_eq!(product, expected_product);
    let reified: Vec<_> = quads
        .iter()
        .filter(|q| q.predicate == rdf::REIFIES)
        .collect();
    assert_eq!(reified.len(), expected);
    for q in reified {
        assert_ne!(q.graph_name, GraphName::DefaultGraph);
        let Term::Triple(triple) = &q.object else {
            panic!("native reifying triple term required");
        };
        assert!(product.contains(triple.as_ref()));
    }
    if expected == 0 {
        assert!(quads.is_empty());
    } else {
        assert_metadata(&quads, &rows[0]);
    }
    assert!(quads
        .iter()
        .filter(|q| q
            .predicate
            .as_str()
            .starts_with("http://www.w3.org/ns/prov#"))
        .all(|q| q.graph_name != GraphName::DefaultGraph));
}

pub(super) fn assert_responses(address: SocketAddr, fixture: &Fixture, database: &Database) {
    for query in [SINGLE, GRAPH] {
        for token in [None, Some("invalid-lineage-token")] {
            assert_eq!(
                request_format(address, query, token, FORMAT).unwrap().0,
                401
            );
        }
    }
    let first = records(address, SINGLE, &fixture.token);
    assert_select(&first, 1);
    let cached = records(address, SINGLE, &fixture.token);
    assert_select(&cached, 1);
    assert_eq!(first[0], cached[0]);
    assert_ne!(
        first[1]["provenance"]["@id"],
        cached[1]["provenance"]["@id"]
    );
    let graph = records(address, GRAPH, &fixture.token);
    assert_graph(&graph, 1);
    assert_eq!(first[0]["snapshot"], graph[0]["snapshot"]);
    assert_eq!(first[0]["policy"], graph[0]["policy"]);
    assert_ne!(first[0]["logicalPlan"], graph[0]["logicalPlan"]);
    let union = "SELECT ?s ?value WHERE { { ?s <http://example.test/left> ?value } UNION { ?s <http://example.test/left> ?value } }";
    assert_select(&records(address, union, &fixture.token), 2);
    let empty = GRAPH.replace(
        "?s <http://example.test/left> ?value }",
        "<urn:absent> <http://example.test/left> ?value }",
    );
    assert_graph(&records(address, &empty, &fixture.token), 0);
    let mut lock = database.hold_table("items");
    for query in [
        "ASK { ?s <http://example.test/left> ?value }",
        "SELECT ?s ?value WHERE { ?s <http://example.test/left> ?value FILTER(?value = \"same\") }",
    ] {
        assert_eq!(
            request_format(address, query, Some(&fixture.token), FORMAT)
                .unwrap()
                .0,
            501
        );
        lock.assert_held();
    }
    drop(lock);
    assert_graph(&records(address, GRAPH, &fixture.token), 1);
    database.assert_encrypted_sessions();
}

pub(super) fn assert_portable_policy(fixture: &Fixture, database: &Database) {
    let second = uuid::Uuid::new_v4().simple().to_string();
    let subject = |id: &str, credential: &str, value: &str| {
        serde_json::json!({
            "subjectRef":id, "credentialEnv":credential,
            "portableRows":[{"sourceIndex":0,"table":"items","column":"value","valueEnv":value}]
        })
    };
    let registry = serde_json::json!({"schemaVersion":2,"subjects":[
        subject("private-lineage-allow", "SF_TLS_BEARER", "SF_LINEAGE_ALLOW"),
        subject("private-lineage-deny", "SF_LINEAGE_SECOND", "SF_LINEAGE_DENY")
    ]});
    let (mut command, address) = command_with_admission(
        fixture,
        database,
        None,
        &["--auth-subjects-env", "SF_TLS_SUBJECTS"],
    );
    let mut server = Server(
        command
            .env("SF_TLS_SUBJECTS", registry.to_string())
            .env("SF_LINEAGE_SECOND", &second)
            .env("SF_LINEAGE_ALLOW", "same")
            .env("SF_LINEAGE_DENY", "not-present")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some((status, _)) = request_format(address, SINGLE, None, FORMAT) {
            assert_eq!(status, 401);
            break;
        }
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "portable lineage startup exited"
        );
        assert!(Instant::now() < until, "portable lineage startup deadline");
        thread::sleep(Duration::from_millis(20));
    }
    let mut policies = Vec::new();
    for (token, expected) in [(&fixture.token, 1), (&second, 0), (&fixture.token, 1)] {
        for query in [SINGLE, GRAPH] {
            let rows = records(address, query, token);
            if query == SINGLE {
                assert_select(&rows, expected);
            } else {
                assert_graph(&rows, expected);
            }
            let text = serde_json::to_string(&rows).unwrap();
            if expected == 0 {
                assert!(
                    !text.contains("same"),
                    "denied value leaked outside result records"
                );
            }
            for forbidden in [
                &fixture.token,
                &second,
                "private-lineage-allow",
                "private-lineage-deny",
                "not-present",
            ] {
                assert!(!text.contains(forbidden));
            }
            policies.push(rows[0]["policy"].clone());
        }
    }
    assert!(policies.windows(2).all(|pair| pair[0] == pair[1]));
    database.assert_encrypted_sessions();
}
