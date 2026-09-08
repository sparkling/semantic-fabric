//! Cancellation proof for the real mixed-source public paths, not dropped futures.
use super::*;
use std::net::Shutdown;

#[derive(Clone, Copy, Debug)]
enum Stop {
    Deadline,
    Disconnect,
    Shutdown,
}

fn start(mut command: Command, address: SocketAddr) -> Server {
    let mut server = Server(
        command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some((status, _)) = request(address, SINGLE, None) {
            assert_eq!(status, 401);
            return server;
        }
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "matrix server exited during startup"
        );
        assert!(Instant::now() < until, "matrix startup deadline");
        thread::sleep(Duration::from_millis(20));
    }
}

fn blocked_session(database: &Database, postgres: bool, table: &str) -> u64 {
    assert!(matches!(table, "items" | "healthy"));
    let query = if postgres {
        format!("SELECT pid FROM pg_stat_activity WHERE usename='sf_tls' AND state='active' AND wait_event_type='Lock' AND query LIKE '%{table}%'")
    } else {
        // Metadata locking can block COM_STMT_PREPARE, before Execute starts.
        format!("SELECT ID FROM information_schema.PROCESSLIST WHERE USER='sf_tls' AND STATE LIKE 'Waiting%lock%' AND INFO LIKE '%{table}%'")
    };
    let until = Instant::now() + Duration::from_millis(800);
    loop {
        let found = database.sql(&query);
        if !found.is_empty() {
            let id = found.parse().expect("exactly one target session");
            assert!(id > 0);
            return id;
        }
        assert!(
            Instant::now() < until,
            "no native lock wait on {table}, postgres={postgres}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn active(database: &Database, postgres: bool, id: u64) -> bool {
    database.sql(&if postgres {
        format!("SELECT count(*) FROM pg_stat_activity WHERE pid={id} AND usename='sf_tls' AND state='active'")
    } else {
        format!("SELECT count(*) FROM information_schema.PROCESSLIST WHERE ID={id} AND USER='sf_tls' AND COMMAND <> 'Sleep'")
    }) == "1"
}

fn wire(stream: TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    if let Err(error) = stream.take(65537).read_to_end(&mut bytes) {
        assert!(
            matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::UnexpectedEof
            ),
            "wire read: {error}"
        );
    }
    assert!(bytes.len() <= 65536);
    bytes
}

fn assert_no_complete_union_success(response: &[u8]) {
    if response.starts_with(b"HTTP/1.1 200") {
        let boundary = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let headers = std::str::from_utf8(&response[..boundary])
            .unwrap()
            .to_ascii_lowercase();
        assert!(headers.contains("transfer-encoding: chunked"));
        assert!(!headers.contains("content-length:"));
        assert!(
            !response.ends_with(b"\r\n0\r\n\r\n"),
            "failed UNION emitted complete chunked success"
        );
    }
}

fn bag(body: &[u8]) -> (serde_json::Value, Vec<String>) {
    let result: serde_json::Value = serde_json::from_slice(body).unwrap();
    let mut rows: Vec<_> = result["results"]["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect();
    rows.sort();
    (result["head"].clone(), rows)
}

pub(super) fn assert_federated_stop(fixture: &Fixture, postgres: &Database, mysql: &Database) {
    postgres.sql("CREATE TABLE public.healthy AS SELECT value,id FROM public.items; GRANT SELECT ON public.healthy TO sf_tls");
    mysql.sql("CREATE TABLE sf_tls.healthy AS SELECT value,id FROM sf_tls.items");
    let sibling_fixture = Fixture::new();
    sibling_fixture.write(
        "first.ttl",
        &std::fs::read_to_string(fixture.root.join("first.ttl"))
            .unwrap()
            .replace("rr:tableName \"items\"", "rr:tableName \"healthy\""),
    );
    sibling_fixture.write(
        "ontology.ttl",
        &std::fs::read_to_string(fixture.root.join("ontology.ttl")).unwrap(),
    );
    for (database, is_postgres) in [(postgres, true), (mysql, false)] {
        let (mut sibling_command, sibling_address) = command(&sibling_fixture, database, None);
        sibling_command.args(["--timeout-secs", "30", "--pg-pool-size", "1"]);
        if !is_postgres {
            sibling_command.env(
                "SF_TLS_SOURCE",
                format!("{}?pool_min=0&pool_max=1", database.source),
            );
        }
        let _sibling = start(sibling_command, sibling_address);
        let (status, body) =
            request(sibling_address, SINGLE, Some(&sibling_fixture.token)).unwrap();
        assert_eq!(status, 200);
        let sibling_bag = bag(&body);
        for query in [UNION, join::JOIN, join::REVERSED] {
            let is_join = query != UNION;
            for stop in [Stop::Deadline, Stop::Disconnect, Stop::Shutdown] {
                eprintln!(
                    "native federation: postgres={is_postgres}, join={is_join}, stop={stop:?}"
                );
                let (mut command, address) = command(fixture, postgres, Some(mysql));
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
                command.env(
                    "SF_TLS_SOURCE_2",
                    format!("{}?pool_min=0&pool_max=1", mysql.source),
                );
                let mut server = start(command, address);
                let (status, body) = request(address, query, Some(&fixture.token)).unwrap();
                assert_eq!(status, 200);
                let expected = bag(&body);
                assert_eq!(expected.1.len(), if is_join { 12 } else { 8 });

                // Both statements stay blocked until after target-stop assertions.
                // Distinct IDs and tables distinguish wrong-session cancellation.
                let mut sibling_lock = database.hold_table("healthy");
                let sibling_stream =
                    cancellation::begin(sibling_address, SINGLE, &sibling_fixture.token);
                let sibling_id = blocked_session(database, is_postgres, "healthy");
                let mut target_lock = database.hold_table("items");
                let stream = cancellation::begin(address, query, &fixture.token);
                let target_id = blocked_session(database, is_postgres, "items");
                assert_ne!(target_id, sibling_id);
                if is_join {
                    // A staged join must still be pre-header while the source waits.
                    stream.set_nonblocking(true).unwrap();
                    assert_eq!(
                        stream.peek(&mut [0u8; 1]).unwrap_err().kind(),
                        std::io::ErrorKind::WouldBlock
                    );
                    stream.set_nonblocking(false).unwrap();
                }
                let stopped_at = Instant::now();
                match stop {
                    Stop::Disconnect => {
                        stream.shutdown(Shutdown::Both).unwrap();
                        drop(stream);
                    }
                    Stop::Shutdown => {
                        assert!(Command::new("kill")
                            .args(["-TERM", &server.0.id().to_string()])
                            .status()
                            .unwrap()
                            .success());
                        let response = wire(stream);
                        if is_join {
                            assert!(!response.starts_with(b"HTTP/1.1 200"));
                        } else {
                            assert_no_complete_union_success(&response);
                        }
                    }
                    Stop::Deadline => {
                        let response = wire(stream);
                        if is_join {
                            assert!(
                                response.starts_with(b"HTTP/1.1 504"),
                                "join must fail before success: {}",
                                String::from_utf8_lossy(&response)
                            );
                        } else if response.starts_with(b"HTTP/1.1 200") {
                            assert_no_complete_union_success(&response);
                        } else {
                            assert!(response.is_empty() || response.starts_with(b"HTTP/1.1 504"));
                        }
                    }
                }
                while active(database, is_postgres, target_id) {
                    assert!(
                        stopped_at.elapsed() < Duration::from_secs(3),
                        "target native work survived cancellation"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                assert!(
                    stopped_at.elapsed() < Duration::from_secs(3),
                    "native stop exceeded its observation bound"
                );
                target_lock.assert_held();
                sibling_lock.assert_held();
                assert!(
                    active(database, is_postgres, sibling_id),
                    "unrelated same-credential statement was cancelled"
                );
                assert_eq!(
                    blocked_session(database, is_postgres, "healthy"),
                    sibling_id
                );
                drop(sibling_lock);
                let (status, body) = decode_response(wire(sibling_stream));
                assert_eq!(status, 200);
                assert_eq!(bag(&body), sibling_bag);
                drop(target_lock);
                if matches!(stop, Stop::Shutdown) {
                    let status = loop {
                        if let Some(status) = server.0.try_wait().unwrap() {
                            break status;
                        }
                        assert!(
                            stopped_at.elapsed() < Duration::from_secs(4),
                            "forced process exit bound"
                        );
                        thread::sleep(Duration::from_millis(10));
                    };
                    assert!(status.success());
                    assert!(stopped_at.elapsed() < Duration::from_secs(4));
                    assert!(TcpStream::connect(address).is_err());
                } else {
                    let (status, body) = request(address, query, Some(&fixture.token)).unwrap();
                    assert_eq!(status, 200, "both cap-one pools must recover");
                    assert_eq!(
                        bag(&body),
                        expected,
                        "full exact federated bag after cancellation"
                    );
                }
            }
        }
    }
}
