//! Observe stopped native work, not merely dropped Rust futures or pool capacity.
use super::*;
use std::net::Shutdown;

const ASK: &str = "ASK { ?s <http://example.test/left> ?value }";
const CONSTRUCT: &str = "CONSTRUCT { ?s <http://example.test/left> ?value } WHERE { ?s <http://example.test/left> ?value }";

fn active(database: &Database, postgres: bool) -> usize {
    database.sql(if postgres {
        "SELECT count(*) FROM pg_stat_activity WHERE usename='sf_tls' AND state='active' AND wait_event='PgSleep'"
    } else {
        "SELECT count(*) FROM information_schema.PROCESSLIST WHERE USER='sf_tls' AND COMMAND='Execute' AND INFO LIKE '%SLEEP(%'"
    }).parse().unwrap()
}
fn wait_active(database: &Database, postgres: bool, expected: usize, bound: Duration) {
    let until = Instant::now() + bound;
    loop {
        let found = active(database, postgres);
        if found == expected {
            return;
        }
        assert!(
            Instant::now() < until,
            "native active count: expected {expected}, found {found}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}
pub(super) fn begin(address: SocketAddr, query: &str, token: &str) -> TcpStream {
    begin_format(address, query, token, "application/sparql-results+json")
}

pub(super) fn begin_format(
    address: SocketAddr,
    query: &str,
    token: &str,
    accept: &str,
) -> TcpStream {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(4)))
        .unwrap();
    write!(stream, "POST /sparql HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\nContent-Type: application/sparql-query\r\nAccept: {accept}\r\nContent-Length: {}\r\n\r\n{query}",query.len()).unwrap();
    stream
}

pub(super) fn assert_native_stop(fixture: &Fixture, database: &Database, postgres: bool) {
    let path = fixture.root.join("first.ttl");
    let original = std::fs::read_to_string(&path).unwrap();
    let table = if postgres {
        "public.items"
    } else {
        "sf_tls.items"
    };
    database.sql(&format!(
        "ALTER TABLE {table} ADD COLUMN cancel_delay BOOLEAN NOT NULL DEFAULT FALSE"
    ));
    let sql = if postgres {
        "SELECT value FROM items CROSS JOIN LATERAL (SELECT pg_catalog.pg_sleep(CASE WHEN cancel_delay THEN 6 ELSE 0 END)) hold"
    } else {
        "SELECT value FROM items WHERE SLEEP(IF(cancel_delay,6,0))=0"
    };
    let mapping = original
        .replace(
            r#"rr:tableName "items""#,
            &format!(r#"rr:sqlQuery "{sql}""#),
        )
        .replace(
            r#"rr:column "value""#,
            r#"rr:column "value" ; rr:datatype <http://www.w3.org/2001/XMLSchema#string>"#,
        );
    assert_ne!(original, mapping);
    std::fs::write(&path, mapping).unwrap();
    for (query, timeout, disconnect, native_policy, forced) in [
        (ASK, "1", false, false, false),
        (SINGLE, "30", true, false, false),
        (CONSTRUCT, "30", true, false, false),
        (ASK, "30", false, true, false),
        (ASK, "30", false, false, true),
        (SINGLE, "30", false, false, true),
        (CONSTRUCT, "30", false, false, true),
    ] {
        if native_policy && postgres {
            continue;
        }
        if native_policy {
            database.sql("SET GLOBAL max_execution_time=100");
        }
        database.sql(&format!("UPDATE {table} SET cancel_delay=FALSE"));
        let (mut command, address) = command(fixture, database, None);
        command.args([
            "--timeout-secs",
            timeout,
            "--pg-pool-size",
            "1",
            "--shutdown-timeout-secs",
            "1",
        ]);
        if !postgres {
            command.env(
                "SF_TLS_SOURCE",
                format!("{}?pool_min=0&pool_max=1", database.source),
            );
        }
        let mut server = Server(
            command
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let startup = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some((status, _)) = request(address, ASK, None) {
                assert_eq!(status, 401);
                break;
            }
            assert!(
                server.0.try_wait().unwrap().is_none(),
                "native cancellation child exited"
            );
            assert!(Instant::now() < startup, "native cancellation startup");
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(request(address, ASK, Some(&fixture.token)).unwrap().0, 200);
        database.sql(&format!("UPDATE {table} SET cancel_delay=TRUE"));
        let mut stream = begin(address, query, &fixture.token);
        if !native_policy {
            wait_active(database, postgres, 1, Duration::from_millis(800));
        }
        let stopped_at = Instant::now();
        if forced {
            assert!(Command::new("kill")
                .args(["-TERM", &server.0.id().to_string()])
                .status()
                .unwrap()
                .success());
        } else if disconnect {
            stream.shutdown(Shutdown::Both).unwrap();
            drop(stream);
        } else {
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            assert!(
                response.starts_with("HTTP/1.1 504"),
                "deadline did not return a typed failure: {response}"
            );
        }
        wait_active(database, postgres, 0, Duration::from_secs(3));
        assert!(
            stopped_at.elapsed() < Duration::from_secs(4),
            "statement reached its natural six-second completion"
        );
        if forced {
            let exit_bound = stopped_at + Duration::from_secs(4);
            let status = loop {
                if let Some(status) = server.0.try_wait().unwrap() {
                    break status;
                }
                assert!(
                    Instant::now() < exit_bound,
                    "forced shutdown did not exit within bound"
                );
                thread::sleep(Duration::from_millis(20));
            };
            assert!(status.success(), "forced shutdown must exit cleanly");
            assert!(TcpStream::connect(address).is_err());
            continue;
        }
        database.sql(&format!("UPDATE {table} SET cancel_delay=FALSE"));
        // Cap-one pool: the stopped statement must not block a fresh exact result.
        let (status, result) = request(address, ASK, Some(&fixture.token)).unwrap();
        assert_eq!(status, 200);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&result).unwrap()["boolean"],
            true
        );
        if native_policy {
            database.sql("SET GLOBAL max_execution_time=0");
        }
    }
    std::fs::write(path, original).unwrap();
}
