//! Hostile parser inputs execute only in an owned serving process, never the test runner.
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const TOKEN: &str = "test-only-parser-lifetime-bearer-123456";
const EXACT: &str = "SELECT ?value WHERE { ?s <http://example.test/value> ?value }";

struct Server {
    child: Child,
    root: PathBuf,
    address: SocketAddr,
}

impl Server {
    fn start() -> Self {
        let root =
            std::env::temp_dir().join(format!("sf-parser-lifetime-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let db = root.join("fixture.db");
        let connection = rusqlite::Connection::open(&db).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE items(value TEXT NOT NULL); INSERT INTO items VALUES ('exact');",
            )
            .unwrap();
        drop(connection);
        std::fs::write(
            root.join("mapping.ttl"),
            r#"
            @prefix rr: <http://www.w3.org/ns/r2rml#> .
            <#Items> a rr:TriplesMap ; rr:logicalTable [ rr:tableName "items" ] ;
              rr:subject <http://example.test/item> ; rr:predicateObjectMap [
                rr:predicate <http://example.test/value> ; rr:objectMap [ rr:column "value" ] ] .
        "#,
        )
        .unwrap();
        std::fs::write(
            root.join("ontology.ttl"),
            "<http://example.test/value> a <http://www.w3.org/1999/02/22-rdf-syntax-ns#Property> .",
        )
        .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let mut command = Command::new(env!("CARGO_BIN_EXE_semantic-fabric"));
        command
            .env_clear()
            .env("SF_PARSER_TEST_TOKEN", TOKEN)
            .args([
                "serve",
                "--bind",
                &address.to_string(),
                "--source",
                &format!("sqlite:{}", db.display()),
            ])
            .arg("--mapping")
            .arg(root.join("mapping.ttl"))
            .arg("--ontology")
            .arg(root.join("ontology.ttl"))
            .args([
                "--auth-token-env",
                "SF_PARSER_TEST_TOKEN",
                "--timeout-secs",
                "1",
                "--max-query-len",
                "65536",
                "--max-concurrent-requests",
                "1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // A deliberately crashed fixture must not leave a core dump on the host.
        unsafe {
            command.pre_exec(|| {
                let limit = libc::rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                if libc::setrlimit(libc::RLIMIT_CORE, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().unwrap();
        let mut server = Self {
            child,
            root,
            address,
        };
        let startup = Instant::now();
        let deadline = startup + Duration::from_secs(30);
        loop {
            if let Ok(response) = server.query(EXACT) {
                if response.starts_with("HTTP/1.1 200") {
                    break;
                }
            }
            assert!(
                server.child.try_wait().unwrap().is_none(),
                "server failed startup"
            );
            assert!(
                Instant::now() < deadline,
                "server startup exceeded bound; last query: {:?}",
                server
                    .query(EXACT)
                    .map(|r| r.lines().next().unwrap_or("").to_owned())
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        eprintln!("owned server ready after {:?}", startup.elapsed());
        server
    }

    fn query(&self, query: &str) -> std::io::Result<String> {
        self.query_format(query, "application/sparql-results+json")
    }

    fn query_format(&self, query: &str, accept: &str) -> std::io::Result<String> {
        let mut socket = TcpStream::connect_timeout(&self.address, Duration::from_millis(100))?;
        socket.set_read_timeout(Some(Duration::from_secs(3)))?;
        socket.set_write_timeout(Some(Duration::from_secs(3)))?;
        write!(socket, "POST /sparql HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/sparql-query\r\nAccept: {accept}\r\nContent-Length: {}\r\n\r\n{query}", self.address, query.len())?;
        let mut bytes = Vec::new();
        socket.take(65536).read_to_end(&mut bytes)?;
        String::from_utf8(bytes).map_err(std::io::Error::other)
    }

    fn assert_exact_recovery(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let response = self.query(EXACT).unwrap();
            if response.starts_with("HTTP/1.1 200") {
                let (headers, body) = response.split_once("\r\n\r\n").unwrap();
                let mut decoded = String::new();
                if headers
                    .to_ascii_lowercase()
                    .contains("transfer-encoding: chunked")
                {
                    let mut rest = body;
                    loop {
                        let (size, payload) = rest.split_once("\r\n").unwrap();
                        let size = usize::from_str_radix(size, 16).unwrap();
                        if size == 0 {
                            assert_eq!(payload, "\r\n");
                            break;
                        }
                        decoded.push_str(&payload[..size]);
                        assert_eq!(&payload[size..size + 2], "\r\n");
                        rest = &payload[size + 2..];
                    }
                } else {
                    decoded.push_str(body);
                }
                let result: serde_json::Value = serde_json::from_str(&decoded).unwrap();
                assert_eq!(result["head"]["vars"], serde_json::json!(["value"]));
                assert_eq!(
                    result["results"]["bindings"],
                    serde_json::json!([
                        {"value":{"type":"literal", "value":"exact"}}
                    ])
                );
                return;
            }
            assert!(
                response.starts_with("HTTP/1.1 503"),
                "unexpected recovery status"
            );
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "server exited during recovery"
            );
            assert!(
                Instant::now() < deadline,
                "cap-one request capacity was not recovered"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        for name in ["fixture.db", "mapping.ttl", "ontology.ttl"] {
            let _ = std::fs::remove_file(self.root.join(name));
        }
        let _ = std::fs::remove_dir(&self.root);
    }
}

#[test]
fn deeply_nested_public_query_rejects_without_killing_server_and_recovers() {
    let mut server = Server::start();
    let queries = [
        format!(
            "SELECT ({}1{} AS ?x) WHERE {{}}",
            "(".repeat(4096),
            ")".repeat(4096)
        ),
        format!(
            r"SELECT ({}1{} AS ?x) WHERE {{}}",
            r"\u0028".repeat(4096),
            r"\u0029".repeat(4096)
        ),
        format!("SELECT ({}1 AS ?x) WHERE {{}}", "1+".repeat(4096)),
        format!("SELECT ({}1 AS ?x) WHERE {{}}", "(".repeat(4096)),
    ];
    for query in queries {
        for accept in [
            "application/sparql-results+json",
            "application/vnd.semantic-fabric.lineage+json-seq",
        ] {
            assert_bounded_rejection(&mut server, &query, accept);
        }
    }
}

fn assert_bounded_rejection(server: &mut Server, query: &str, accept: &str) {
    assert!(query.len() < 65536);
    let start = Instant::now();
    let response = server.query_format(query, accept);
    assert!(
        start.elapsed() < Duration::from_secs(4),
        "parser rejection exceeded bound"
    );
    let status = server.child.try_wait().unwrap();
    assert!(
        status.is_none(),
        "hostile public parse killed serving process: {status:?}"
    );
    let response = response.expect("bounded typed HTTP rejection, not a dropped connection");
    assert!(
        response.starts_with("HTTP/1.1 429")
            || response.starts_with("HTTP/1.1 504")
            || response.starts_with("HTTP/1.1 400")
            || response.starts_with("HTTP/1.1 500"),
        "expected typed bounded rejection, got {:?}",
        response.lines().next()
    );
    assert!(response
        .to_ascii_lowercase()
        .contains("application/problem+json"));
    assert!(!response.contains(query) && !response.contains("isolated parser failed"));
    server.assert_exact_recovery();
}
