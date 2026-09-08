//! Check returned lineage, not a header-only reload/readiness receipt.
use super::*;
use oxrdf::{GraphName, Literal, NamedNode, Quad, Term, Triple};
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};

pub(super) const MAP: &str = "http://example.com/base/#items";
pub(super) const GRAPH: &str = "CONSTRUCT { ?s <http://example.test/left> ?value } WHERE { ?s <http://example.test/left> ?value }";

#[derive(Debug)]
pub(super) struct Snapshot {
    pub header: Value,
    pub rows: Vec<String>,
    pub sources: BTreeSet<String>,
    pub documents: BTreeSet<String>,
    pub product: HashSet<Triple>,
}

pub(super) fn capture(
    address: SocketAddr,
    query: &str,
    token: &str,
    profile: &str,
    maps: &[&str],
) -> Snapshot {
    let (status, body) = request_format(address, query, Some(token), lineage::FORMAT).unwrap();
    assert_eq!(status, 200);
    assert!(!std::str::from_utf8(&body).unwrap().contains(token));
    let snapshot = parse(&body, profile, maps);
    if !profile.ends_with("graph-v1") && profile != "bounded-mapping-source-v1" {
        let (status, ordinary) = request(address, query, Some(token)).unwrap();
        assert_eq!(status, 200);
        let (head, rows) = stop_matrix::bag(&ordinary);
        assert_eq!(rows, snapshot.rows);
        assert_eq!(head["vars"], snapshot.header["variables"]);
    }
    snapshot
}

pub(super) fn values(snapshot: &Snapshot) -> Vec<String> {
    let mut values: Vec<_> = snapshot
        .rows
        .iter()
        .map(|row| {
            let row: Value = serde_json::from_str(row).unwrap();
            row["value"]["value"].as_str().unwrap().to_owned()
        })
        .collect();
    values.sort();
    values
}

pub(super) fn await_values(
    address: SocketAddr,
    token: &str,
    profile: &str,
    maps: &[&str],
    expected: &[&str],
) -> Snapshot {
    let until = Instant::now() + Duration::from_secs(40);
    let mut expected: Vec<_> = expected.iter().map(|s| (*s).to_owned()).collect();
    expected.sort();
    loop {
        if let Some((200, body)) = request_format(address, SINGLE, Some(token), lineage::FORMAT) {
            let snapshot = parse(&body, profile, maps);
            if values(&snapshot) == expected {
                return snapshot;
            }
        }
        assert!(Instant::now() < until, "lineage values did not converge");
        thread::sleep(Duration::from_millis(25));
    }
}

pub(super) fn parse(body: &[u8], profile: &str, maps: &[&str]) -> Snapshot {
    assert_eq!(body.first(), Some(&0x1e));
    assert_eq!(body.last(), Some(&b'\n'));
    let records: Vec<Value> = std::str::from_utf8(body)
        .unwrap()
        .split('\u{1e}')
        .skip(1)
        .map(|part| serde_json::from_str(part).unwrap())
        .collect();
    let header = records[0].clone();
    assert_eq!(header["type"], "header");
    assert_eq!(header["profile"], profile);
    assert_eq!(header["rowKeys"], "not-provided");
    assert_eq!(records.last().unwrap()["type"], "complete");
    assert_eq!(records.last().unwrap()["solutions"], records.len() - 2);
    let mut snapshot = Snapshot {
        header,
        rows: Vec::new(),
        sources: BTreeSet::new(),
        documents: BTreeSet::new(),
        product: HashSet::new(),
    };
    let mut bundles = BTreeSet::new();
    for (ordinal, record) in records[1..records.len() - 1].iter().enumerate() {
        assert_eq!(record["ordinal"], ordinal);
        let quads = if profile.ends_with("graph-v1") {
            assert_eq!(record["type"], "graph-solution");
            let quads = oxttl::NQuadsParser::new()
                .for_slice(record["dataset"].as_str().unwrap().as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            for q in quads
                .iter()
                .filter(|q| q.graph_name == GraphName::DefaultGraph)
            {
                snapshot.product.insert(Triple::new(
                    q.subject.clone(),
                    q.predicate.clone(),
                    q.object.clone(),
                ));
            }
            quads
        } else {
            assert_eq!(record["type"], "solution");
            assert!(bundles.insert(record["provenance"]["@id"].to_string()));
            assert_eq!(
                record["result"]["head"]["vars"],
                snapshot.header["variables"]
            );
            let rows = record["result"]["results"]["bindings"].as_array().unwrap();
            assert_eq!(rows.len(), 1);
            snapshot.rows.push(rows[0].to_string());
            oxjsonld::JsonLdParser::new()
                .for_slice(record["provenance"].to_string().as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        check_metadata(
            &quads,
            &snapshot.header,
            maps,
            &mut snapshot.sources,
            &mut snapshot.documents,
        );
    }
    snapshot.rows.sort();
    if profile == "bounded-federated-union-lineage-v1" {
        assert_eq!(federated_lineage::bag(body, [1, 1]).1, snapshot.rows);
    } else if profile == "bounded-federated-join-lineage-v1" {
        assert_eq!(federated_lineage::join_bag(body).1, snapshot.rows);
    }
    assert!(!snapshot.sources.is_empty() && !snapshot.documents.is_empty());
    snapshot
}

fn check_metadata(
    quads: &[Quad],
    header: &Value,
    maps: &[&str],
    sources: &mut BTreeSet<String>,
    documents: &mut BTreeSet<String>,
) {
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
    let actual: BTreeSet<_> = quads
        .iter()
        .filter(|q| q.predicate.as_str() == "urn:semantic-fabric:lineage:mappingId")
        .map(|q| {
            let Term::Literal(id) = &q.object else {
                panic!("mapping literal")
            };
            id.value()
        })
        .collect();
    assert_eq!(actual, maps.iter().copied().collect());
    assert!(!quads
        .iter()
        .any(|q| q.predicate.as_str() == "http://www.w3.org/ns/prov#wasDerivedFrom"));
    for q in quads
        .iter()
        .filter(|q| q.predicate.as_str() == "http://www.w3.org/ns/prov#used")
    {
        let Term::NamedNode(id) = &q.object else {
            panic!("opaque used identity")
        };
        if id.as_str().starts_with("urn:semantic-fabric:source:") {
            sources.insert(id.as_str().to_owned());
        }
        if id
            .as_str()
            .starts_with("urn:semantic-fabric:mapping-document:")
        {
            documents.insert(id.as_str().to_owned());
        }
    }
}

pub(super) fn check_graph(
    address: SocketAddr,
    token: &str,
    profile: &str,
    maps: &[&str],
    select: &Snapshot,
) {
    let graph = capture(address, GRAPH, token, profile, maps);
    let expected: HashSet<_> = select
        .rows
        .iter()
        .map(|row| {
            let row: Value = serde_json::from_str(row).unwrap();
            Triple::new(
                NamedNode::new(row["s"]["value"].as_str().unwrap()).unwrap(),
                NamedNode::new("http://example.test/left").unwrap(),
                Literal::new_simple_literal(row["value"]["value"].as_str().unwrap()),
            )
        })
        .collect();
    assert_eq!(graph.product, expected);
    assert_eq!(graph.header["policy"], select.header["policy"]);
    assert_ne!(graph.header["logicalPlan"], select.header["logicalPlan"]);
    // A periodic unchanged rebuild may publish between these two requests.
    // Each response's generation references are checked internally above.
    // Bounded graph origins use per-entry identities plus their mapping document.
    assert_eq!(graph.documents, select.documents);
}

pub(super) fn changed(before: &Snapshot, after: &Snapshot) {
    for field in ["snapshot", "logicalPlan"] {
        assert_ne!(before.header[field], after.header[field]);
    }
    assert_eq!(before.header["policy"], after.header["policy"]);
    assert_ne!(before.sources, after.sources);
    assert_ne!(before.documents, after.documents);
    assert!(before.documents.is_disjoint(&after.documents));
}

pub(super) fn health(address: SocketAddr, path: &str) -> Option<u16> {
    assert!(matches!(path, "/readyz" | "/livez"));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(100)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut wire = Vec::new();
    stream.take(65537).read_to_end(&mut wire).unwrap();
    assert!(wire.len() <= 65536);
    Some(decode_response(wire).0)
}

pub(super) fn invalidate_while_pinned(
    fixture: &Fixture,
    database: &Database,
    address: SocketAddr,
    query: &str,
    profile: &str,
    maps: &[&str],
    before: &Snapshot,
) {
    let postgres = database.source.starts_with("pg:");
    let mut held = database.hold_table("items");
    let stream = cancellation::begin_format(address, query, &fixture.token, lineage::FORMAT);
    let pid = stop_matrix::blocked_session(database, postgres, "items");
    fixture.write("first.ttl", "not turtle");
    let until = Instant::now() + Duration::from_secs(4);
    while health(address, "/readyz") != Some(503) {
        assert!(
            Instant::now() < until,
            "reload must fence detected invalid input"
        );
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(health(address, "/livez"), Some(200));
    assert_eq!(
        request_format(address, query, Some(&fixture.token), lineage::FORMAT)
            .unwrap()
            .0,
        503
    );
    assert_eq!(
        request_format(address, query, None, lineage::FORMAT)
            .unwrap()
            .0,
        401
    );
    held.assert_held();
    assert!(
        stop_matrix::active(database, postgres, pid),
        "readiness fence must not revoke pinned native work"
    );
    drop(held);
    let (status, body) = decode_response(stop_matrix::wire(stream));
    assert_eq!(status, 200);
    let pinned = parse(&body, profile, maps);
    // The native barrier proves this request acquired its lease before invalidation.
    // A no-op reload may have published since `before`: compare authored identity,
    // policy and exact results, not unrelated requests' resource generation IDs.
    assert_eq!(pinned.header["policy"], before.header["policy"]);
    assert_eq!(pinned.header["variables"], before.header["variables"]);
    assert_eq!(pinned.rows, before.rows);
    assert_eq!(pinned.documents, before.documents);
}
