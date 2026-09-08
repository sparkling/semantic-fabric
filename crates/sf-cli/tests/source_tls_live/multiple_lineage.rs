//! Same owned native TLS fixture: actual multiple-map origins, not candidates.
use super::*;
use serde_json::Value;
use std::collections::BTreeSet;

pub(super) fn assert_responses(fixture: &Fixture, database: &Database) {
    // Only this aggregate test's private generated mapping is replaced, with no
    // concurrent reload reader. Restore it before the existing lifecycle matrix.
    let original = std::fs::read_to_string(fixture.root.join("first.ttl")).unwrap();
    let mapping=["a","b","unused"].map(|id| {
        let predicate=if id=="unused" {"http://example.test/right"} else {"http://example.test/left"};
        format!(r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<urn:map:{id}> a rr:TriplesMap ; rr:logicalTable [ rr:tableName "items" ] ; rr:subjectMap [ rr:constant <http://example.test/item> ] ; rr:predicateObjectMap [ rr:predicate <{predicate}> ; rr:objectMap [ rr:column "value" ; rr:datatype <http://www.w3.org/2001/XMLSchema#string> ] ] ."#)
    }).join("\n");
    fixture.write("first.ttl", &mapping);
    let registry = serde_json::json!({"schemaVersion":2,"subjects":[
        {"subjectRef":"private-multi-allow","credentialEnv":"SF_TLS_BEARER","portableRows":[{"sourceIndex":0,"table":"items","column":"value","valueEnv":"SF_MULTI_ALLOW"}]},
        {"subjectRef":"private-multi-deny","credentialEnv":"SF_MULTI_SECOND","portableRows":[{"sourceIndex":0,"table":"items","column":"value","valueEnv":"SF_MULTI_DENY"}]}
    ]});
    let second = uuid::Uuid::new_v4().simple().to_string();
    let (mut command, address) = command_with_admission(
        fixture,
        database,
        None,
        &["--auth-subjects-env", "SF_TLS_SUBJECTS"],
    );
    let mut server = Server(
        command
            .env("SF_TLS_SUBJECTS", registry.to_string())
            .env("SF_MULTI_SECOND", &second)
            .env("SF_MULTI_ALLOW", "same")
            .env("SF_MULTI_DENY", "not-present")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some((status, _)) = request_format(address, SINGLE, None, lineage::FORMAT) {
            assert_eq!(status, 401);
            break;
        }
        if server.0.try_wait().unwrap().is_some() {
            let mut diagnostic = String::new();
            server
                .0
                .stderr
                .take()
                .unwrap()
                .take(16384)
                .read_to_string(&mut diagnostic)
                .unwrap();
            // This fixture's generated endpoint/bearers are never diagnostics.
            for secret in [&database.source, &fixture.token, &second] {
                diagnostic = diagnostic.replace(secret, "[redacted]");
            }
            for part in database
                .source
                .split(|c: char| !c.is_ascii_alphanumeric())
                .filter(|part| part.len() > 12)
            {
                diagnostic = diagnostic.replace(part, "[redacted]");
            }
            panic!("multi-origin startup exited: {diagnostic}");
        }
        assert!(Instant::now() < until, "multi-origin startup deadline");
        thread::sleep(Duration::from_millis(20));
    }
    let graph="CONSTRUCT { ?s <http://example.test/left> ?value } WHERE { ?s <http://example.test/left> ?value }";
    // Startup diagnostics are captured above; continuously discard subsequent
    // bounded test logs so a full stderr pipe cannot stall the serving child.
    let mut stderr = server.0.stderr.take().unwrap();
    let log_drain =
        thread::spawn(move || std::io::copy(&mut stderr, &mut std::io::sink()).unwrap());
    let union="SELECT ?s ?value WHERE { { ?s <http://example.test/left> ?value } UNION { ?s <http://example.test/left> ?value } }";
    let join="SELECT ?s ?value WHERE { ?s <http://example.test/left> ?value . ?t <http://example.test/left> ?value }";
    for (token, allowed) in [
        (&fixture.token, true),
        (&second, false),
        (&fixture.token, true),
    ] {
        for query in [SINGLE, union, join, graph] {
            let (status, bytes) =
                request_format(address, query, Some(token), lineage::FORMAT).unwrap();
            assert_eq!(status, 200);
            let text = std::str::from_utf8(&bytes).unwrap();
            for secret in [
                fixture.token.as_str(),
                second.as_str(),
                "private-multi-allow",
                "private-multi-deny",
            ] {
                assert!(!text.contains(secret));
            }
            if !allowed {
                assert!(!text.contains("same"));
            }
            let rows: Vec<Value> = text
                .split('\u{1e}')
                .skip(1)
                .map(|part| serde_json::from_str(part).unwrap())
                .collect();
            assert_eq!(rows.last().unwrap()["type"], "complete");
            let expected = if !allowed {
                0
            } else if query == union {
                2
            } else {
                1
            };
            assert_eq!(rows.last().unwrap()["solutions"], expected);
            assert_eq!(rows[0]["rowKeys"], "not-provided");
            if query == graph {
                let dataset = rows
                    .iter()
                    .filter_map(|row| row["dataset"].as_str())
                    .collect::<String>();
                let quads = oxttl::NQuadsParser::new()
                    .for_slice(dataset.as_bytes())
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                assert_eq!(
                    quads
                        .iter()
                        .filter(|q| q.graph_name == oxrdf::GraphName::DefaultGraph)
                        .count(),
                    expected
                );
                assert_eq!(
                    quads
                        .iter()
                        .filter(|q| q.predicate == oxrdf::vocab::rdf::REIFIES)
                        .count(),
                    expected
                );
                let maps: BTreeSet<_> = quads
                    .iter()
                    .filter(|q| q.predicate.as_str() == "urn:semantic-fabric:lineage:mappingId")
                    .map(|q| q.object.to_string())
                    .collect();
                assert_eq!(
                    maps,
                    if allowed {
                        BTreeSet::from(["\"urn:map:a\"".into(), "\"urn:map:b\"".into()])
                    } else {
                        BTreeSet::new()
                    }
                );
            } else {
                for row in &rows[1..rows.len() - 1] {
                    assert_eq!(
                        row["result"]["results"]["bindings"][0]["value"]["value"],
                        "same"
                    );
                    let provenance = row["provenance"].to_string();
                    assert!(!provenance.contains("same"));
                    let quads = oxjsonld::JsonLdParser::new()
                        .for_slice(provenance.as_bytes())
                        .collect::<Result<Vec<_>, _>>()
                        .unwrap();
                    let maps: BTreeSet<_> = quads
                        .iter()
                        .filter(|q| q.predicate.as_str() == "urn:semantic-fabric:lineage:mappingId")
                        .map(|q| q.object.to_string())
                        .collect();
                    assert_eq!(
                        maps,
                        BTreeSet::from(["\"urn:map:a\"".into(), "\"urn:map:b\"".into()])
                    );
                }
            }
        }
    }
    let mut lock = database.hold_table("items");
    let excluded =
        "SELECT ?s ?value WHERE { ?s <http://example.test/left> ?value FILTER(?value = \"same\") }";
    assert_eq!(
        request_format(address, excluded, Some(&fixture.token), lineage::FORMAT)
            .unwrap()
            .0,
        501
    );
    lock.assert_held();
    drop(lock);
    database.assert_encrypted_sessions();
    drop(server);
    log_drain.join().unwrap();
    fixture.write("first.ttl", &original);
}
