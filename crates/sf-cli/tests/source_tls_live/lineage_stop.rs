//! Qualify the actual multi-origin producer, with native lock and sibling witnesses.
use super::*;
use std::collections::BTreeSet;
use std::net::Shutdown;
use stop_matrix::{active, bag, blocked_session, start, wire, Stop};

const CONSTRUCT: &str = "CONSTRUCT { ?s <http://example.test/left> ?value } WHERE { ?s <http://example.test/left> ?value }";

fn exact_lineage(body: &[u8], graph: bool, postgres: bool) {
    let rows: Vec<serde_json::Value> = std::str::from_utf8(body)
        .unwrap()
        .split('\u{1e}')
        .skip(1)
        .map(|part| serde_json::from_str(part).unwrap())
        .collect();
    assert_eq!(
        rows[0]["profile"],
        if graph {
            "bounded-mapping-source-graph-v1"
        } else {
            "bounded-mapping-source-v1"
        }
    );
    assert_eq!(rows.last().unwrap()["type"], "complete");
    assert_eq!(
        rows.last().unwrap()["solutions"],
        if postgres { 1 } else { 2 }
    );
    let mut values = Vec::new();
    for row in &rows[1..rows.len() - 1] {
        let quads = if graph {
            oxttl::NQuadsParser::new()
                .for_slice(row["dataset"].as_str().unwrap().as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        } else {
            let bindings = row["result"]["results"]["bindings"].as_array().unwrap();
            assert_eq!(bindings.len(), 1);
            assert_eq!(bindings[0]["s"]["value"], "http://example.test/item");
            values.push(bindings[0]["value"]["value"].as_str().unwrap().to_owned());
            oxjsonld::JsonLdParser::new()
                .for_slice(row["provenance"].to_string().as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        if graph {
            let product: Vec<_> = quads
                .iter()
                .filter(|q| q.graph_name == oxrdf::GraphName::DefaultGraph)
                .collect();
            assert_eq!(product.len(), 1);
            assert_eq!(product[0].subject.to_string(), "<http://example.test/item>");
            assert_eq!(product[0].predicate.as_str(), "http://example.test/left");
            let oxrdf::Term::Literal(value) = &product[0].object else {
                panic!("literal product")
            };
            values.push(value.value().to_owned());
            assert_eq!(
                quads
                    .iter()
                    .filter(|q| q.predicate == oxrdf::vocab::rdf::REIFIES)
                    .count(),
                1
            );
        }
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
    values.sort();
    assert_eq!(
        values,
        if postgres {
            vec!["A /%"]
        } else {
            vec!["A /%", "a /%"]
        }
    );
}

pub(super) fn failed_wire(response: &[u8]) {
    assert!(
        response.is_empty()
            || response.starts_with(b"HTTP/1.1 200 ")
            || response.starts_with(b"HTTP/1.1 504 ")
    );
    stop_matrix::assert_no_complete_union_success(response);
    if !response.starts_with(b"HTTP/1.1 200 ") {
        return;
    }
    let boundary = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let mut remaining = &response[boundary + 4..];
    let mut body = Vec::new();
    // Decode every available payload byte, including a partial final chunk.
    // A JSON completion marker split between HTTP chunks must still fail.
    while let Some(end) = remaining.windows(2).position(|w| w == b"\r\n") {
        let size =
            usize::from_str_radix(std::str::from_utf8(&remaining[..end]).unwrap(), 16).unwrap();
        assert_ne!(size, 0, "failed lineage emitted terminal HTTP chunk");
        remaining = &remaining[end + 2..];
        body.extend_from_slice(&remaining[..size.min(remaining.len())]);
        if remaining.len() < size + 2 {
            break;
        }
        assert_eq!(&remaining[size..size + 2], b"\r\n");
        remaining = &remaining[size + 2..];
    }
    for part in body.split(|b| *b == b'\x1e').skip(1) {
        if let Ok(record) = serde_json::from_slice::<serde_json::Value>(part) {
            assert_ne!(
                record["type"], "complete",
                "failed lineage emitted completion record"
            );
        }
    }
}

pub(super) fn assert_native_stop(fixture: &Fixture, postgres: &Database, mysql: &Database) {
    // The preceding owned federation matrix created healthy alongside items.
    // Each new CLI owns an independent cap-one pool; no shared endpoint is used.
    let target = Fixture::new();
    target.write(
        "ontology.ttl",
        &std::fs::read_to_string(fixture.root.join("ontology.ttl")).unwrap(),
    );
    target.write("first.ttl", &["a", "b"].map(|id| format!(r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<urn:map:{id}> a rr:TriplesMap ; rr:logicalTable [rr:tableName "items"] ; rr:subjectMap [rr:constant <http://example.test/item>] ; rr:predicateObjectMap [rr:predicate <http://example.test/left> ; rr:objectMap [rr:column "value" ; rr:datatype <http://www.w3.org/2001/XMLSchema#string>]] ."#)).join("\n"));
    let sibling = Fixture::new();
    sibling.write(
        "ontology.ttl",
        &std::fs::read_to_string(fixture.root.join("ontology.ttl")).unwrap(),
    );
    sibling.write(
        "first.ttl",
        &std::fs::read_to_string(fixture.root.join("first.ttl"))
            .unwrap()
            .replace("rr:tableName \"items\"", "rr:tableName \"healthy\""),
    );
    for (database, is_postgres) in [(postgres, true), (mysql, false)] {
        let (mut sibling_command, sibling_address) = command(&sibling, database, None);
        sibling_command.args(["--timeout-secs", "30", "--pg-pool-size", "1"]);
        if !is_postgres {
            sibling_command.env(
                "SF_TLS_SOURCE",
                format!("{}?pool_min=0&pool_max=1", database.source),
            );
        }
        let _sibling_server = start(sibling_command, sibling_address);
        let (status, body) = request(sibling_address, SINGLE, Some(&sibling.token)).unwrap();
        assert_eq!(status, 200);
        let sibling_expected = bag(&body);
        for query in [SINGLE, CONSTRUCT] {
            for stop in [Stop::Deadline, Stop::Disconnect, Stop::Shutdown] {
                eprintln!(
                    "native lineage: postgres={is_postgres}, graph={}, stop={stop:?}",
                    query == CONSTRUCT
                );
                let (mut command, address) = command(&target, database, None);
                command.args([
                    "--timeout-secs",
                    if matches!(stop, Stop::Deadline) {
                        "1"
                    } else {
                        "30"
                    },
                    "--pg-pool-size",
                    "1",
                    "--shutdown-timeout-secs",
                    "1",
                ]);
                if !is_postgres {
                    command.env(
                        "SF_TLS_SOURCE",
                        format!("{}?pool_min=0&pool_max=1", database.source),
                    );
                }
                let mut server = start(command, address);
                let (status, body) =
                    request_format(address, query, Some(&target.token), lineage::FORMAT).unwrap();
                assert_eq!(status, 200);
                exact_lineage(&body, query == CONSTRUCT, is_postgres);

                let mut sibling_lock = database.hold_table("healthy");
                let sibling_stream = cancellation::begin(sibling_address, SINGLE, &sibling.token);
                let sibling_id = blocked_session(database, is_postgres, "healthy");
                let mut target_lock = database.hold_table("items");
                let stream =
                    cancellation::begin_format(address, query, &target.token, lineage::FORMAT);
                let target_id = blocked_session(database, is_postgres, "items");
                assert_ne!(target_id, sibling_id);
                // Observe this exact live target, not an idle pool member that
                // a pool_min=0 backend may already have closed after success.
                let encrypted = database.sql(&if is_postgres {
                    format!("SELECT count(*) FROM pg_stat_ssl s JOIN pg_stat_activity a ON a.pid=s.pid WHERE a.pid={target_id} AND a.usename='sf_tls' AND s.ssl")
                } else {
                    format!("SELECT count(*) FROM performance_schema.status_by_thread s JOIN performance_schema.threads t USING (THREAD_ID) WHERE t.PROCESSLIST_ID={target_id} AND t.PROCESSLIST_USER='sf_tls' AND s.VARIABLE_NAME='Ssl_cipher' AND s.VARIABLE_VALUE <> ''")
                });
                assert_eq!(encrypted, "1", "blocked lineage target must be encrypted");
                let stopped_at = Instant::now();
                match stop {
                    Stop::Disconnect => {
                        stream.shutdown(Shutdown::Both).unwrap();
                        drop(stream);
                    }
                    Stop::Deadline => failed_wire(&wire(stream)),
                    Stop::Shutdown => {
                        assert!(Command::new("kill")
                            .args(["-TERM", &server.0.id().to_string()])
                            .status()
                            .unwrap()
                            .success());
                        failed_wire(&wire(stream));
                    }
                }
                while active(database, is_postgres, target_id) {
                    assert!(
                        stopped_at.elapsed() < Duration::from_secs(3),
                        "lineage native work survived cancellation"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                assert!(stopped_at.elapsed() < Duration::from_secs(3));
                target_lock.assert_held();
                sibling_lock.assert_held();
                assert!(
                    active(database, is_postgres, sibling_id),
                    "wrong same-credential session was cancelled"
                );
                assert_eq!(
                    blocked_session(database, is_postgres, "healthy"),
                    sibling_id
                );
                drop(sibling_lock);
                let (status, body) = decode_response(wire(sibling_stream));
                assert_eq!(status, 200);
                assert_eq!(bag(&body), sibling_expected);
                drop(target_lock);
                if matches!(stop, Stop::Shutdown) {
                    let status = loop {
                        if let Some(status) = server.0.try_wait().unwrap() {
                            break status;
                        }
                        assert!(stopped_at.elapsed() < Duration::from_secs(4));
                        thread::sleep(Duration::from_millis(10));
                    };
                    assert!(status.success());
                    assert!(stopped_at.elapsed() < Duration::from_secs(4));
                    assert!(TcpStream::connect(address).is_err());
                } else {
                    let (status, body) =
                        request_format(address, query, Some(&target.token), lineage::FORMAT)
                            .unwrap();
                    assert_eq!(status, 200, "lineage cap-one pool must recover");
                    exact_lineage(&body, query == CONSTRUCT, is_postgres);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_failure_oracle_checks_json_across_http_chunk_boundaries() {
        let header = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
        let framed = |parts: &[&str]| {
            let mut response = header.to_owned();
            for part in parts {
                response.push_str(&format!("{:x}\r\n{part}\r\n", part.len()));
            }
            response
        };
        failed_wire(b"");
        failed_wire(b"HTTP/1.1 504 Gateway Timeout\r\n\r\n");
        failed_wire(framed(&["\x1e{\"type\":\"header\"}\n"]).as_bytes());
        for response in [
            framed(&["\x1e{\"type\":\"com", "plete\"}\n"]),
            framed(&["\x1e{\"type\" : \"complete\"}\n"]),
            format!("{header}0\r\n\r\n"),
        ] {
            assert!(std::panic::catch_unwind(|| failed_wire(response.as_bytes())).is_err());
        }
    }
}
