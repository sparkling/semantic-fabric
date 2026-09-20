//! End-to-end coverage for the governed SQL-canonicalization peer: a real
//! `semantic-fabric serve` process, a real SQLite source, and real HTTP
//! requests -- not a manually constructed `Branch`/`SqlCond`. Mirrors
//! `parser_lifetime.rs`'s server harness, but drives WHERE-clause SQL
//! *emission* (reached only once a compiled plan actually executes, per
//! `exec_core::driver::run_branches`) rather than SPARQL parsing.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const TOKEN: &str = "test-only-sql-canonicalize-bearer-123456";
const EXACT: &str = "SELECT ?value WHERE { ?s <http://example.test/value> ?value }";

struct Server {
    child: Child,
    root: PathBuf,
    address: SocketAddr,
}

impl Server {
    fn start() -> Self {
        Self::start_with_concurrency(1)
    }

    fn start_with_concurrency(max_concurrent_requests: u32) -> Self {
        let root =
            std::env::temp_dir().join(format!("sf-sql-canonicalize-{}", uuid::Uuid::new_v4()));
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
            .env("SF_SQL_CANONICALIZE_TEST_TOKEN", TOKEN)
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
                "SF_SQL_CANONICALIZE_TEST_TOKEN",
                "--timeout-secs",
                "20",
                "--max-query-len",
                "65536",
                "--max-concurrent-requests",
                &max_concurrent_requests.to_string(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
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
            assert!(Instant::now() < deadline, "server startup exceeded bound");
            std::thread::sleep(Duration::from_millis(20));
        }
        server
    }

    fn query(&self, query: &str) -> std::io::Result<String> {
        self.query_timeout(query, Duration::from_secs(3))
    }

    fn query_timeout(&self, query: &str, timeout: Duration) -> std::io::Result<String> {
        let mut socket = TcpStream::connect_timeout(&self.address, Duration::from_millis(200))?;
        socket.set_read_timeout(Some(timeout))?;
        socket.set_write_timeout(Some(timeout))?;
        write!(socket, "POST /sparql HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/sparql-query\r\nAccept: application/sparql-results+json\r\nContent-Length: {}\r\n\r\n{query}", self.address, query.len())?;
        let mut bytes = Vec::new();
        socket.take(1_048_576).read_to_end(&mut bytes)?;
        String::from_utf8(bytes).map_err(std::io::Error::other)
    }

    fn assert_exact_recovery(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let response = self.query(EXACT).unwrap();
            if response.starts_with("HTTP/1.1 200") {
                assert_exact_body(&response);
                return;
            }
            assert!(
                response.starts_with("HTTP/1.1 503"),
                "unexpected recovery status: {:?}",
                response.lines().next()
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

/// Structurally decode a chunked (or identity) HTTP body. `None` means the
/// stream is genuinely incomplete or malformed (a truncated final chunk, a
/// missing terminal `0\r\n\r\n`, or a corrupt chunk-size/trailer) -- never
/// approximated by a string-suffix heuristic, which a chunk whose *content*
/// happens to end in the literal bytes `0\r\n\r\n` would falsely satisfy, and
/// which a real terminator preceded by `.trim_end()` (stripping the exact
/// trailing `\r\n` it then searches for) can never satisfy at all.
fn try_decode_chunked_body(headers: &str, body: &str) -> Option<String> {
    if !headers
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        return Some(body.to_owned());
    }
    let mut decoded = String::new();
    let mut rest = body;
    loop {
        let (size, payload) = rest.split_once("\r\n")?;
        let size = usize::from_str_radix(size, 16).ok()?;
        if size == 0 {
            return (payload == "\r\n").then_some(decoded);
        }
        if payload.len() < size + 2 {
            return None;
        }
        decoded.push_str(&payload[..size]);
        if &payload[size..size + 2] != "\r\n" {
            return None;
        }
        rest = &payload[size + 2..];
    }
}

fn assert_exact_body(response: &str) {
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    let decoded = try_decode_chunked_body(headers, body).expect("complete, well-formed body");
    let result: serde_json::Value = serde_json::from_str(&decoded).unwrap();
    assert_eq!(result["head"]["vars"], serde_json::json!(["value"]));
    assert_eq!(
        result["results"]["bindings"],
        serde_json::json!([{"value":{"type":"literal", "value":"exact"}}])
    );
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

/// A real, isolated, SQLite-backed request whose WHERE clause is a plain
/// comparison: proves the governed peer produces the exact correct result,
/// not just that it fails closed under adversarial input.
#[test]
fn ordinary_query_through_the_isolated_peer_returns_the_exact_row() {
    let server = Server::start();
    let query =
        "SELECT ?value WHERE { ?s <http://example.test/value> ?value . FILTER(?value = \"exact\") }";
    let response = server.query(query).unwrap();
    assert!(
        response.starts_with("HTTP/1.1 200"),
        "{:?}",
        response.lines().next()
    );
    assert_exact_body(&response);
}

/// A real SPARQL `FILTER` with deep NOT nesting reaches
/// `exec_core::driver::run_branches` -> `emit::emit_branch_binding_view` at
/// EXECUTION time (after a real compile succeeds), the actual public library
/// surface the reproduced hang used, not a manually mutated `Branch`. Before
/// this change, the reproduced probe showed native `sqlparser` parse time
/// growing ~7.5x per two levels of nesting past depth ~30 (release mode);
/// this bounds and recovers the same shape instead of hanging the server.
#[test]
fn deep_where_clause_execution_is_bounded_and_server_recovers() {
    let mut server = Server::start();
    // Deep enough to be far past the measured exponential-time onset
    // (depth ~30-36 for this NOT/paren shape) while staying under the
    // compile-time `MAX_ALGEBRA_DEPTH_V1` (128) algebra-depth envelope, so
    // the query reaches execution rather than failing fast at compile time.
    const DEPTH: usize = 100;
    let query = format!(
        "SELECT ?value WHERE {{ ?s <http://example.test/value> ?value . FILTER({}(?value = \"exact\"){}) }}",
        "!(".repeat(DEPTH),
        ")".repeat(DEPTH),
    );
    let start = Instant::now();
    let response = server
        .query_timeout(&query, Duration::from_secs(18))
        .expect("bounded response, not a dropped/hung connection");
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(18),
        "deep WHERE-clause execution exceeded its bound: {elapsed:?}"
    );
    let status = server.child.try_wait().unwrap();
    assert!(
        status.is_none(),
        "deep WHERE-clause execution killed the serving process: {status:?}"
    );
    // Three outcomes are acceptable evidence of containment; the only
    // unacceptable one is a dropped/hung connection (already ruled out by
    // `query_timeout` above returning `Ok` within the bound):
    //   (a) a correct result -- the isolated peer parsed the deep skeleton
    //       within its rlimits;
    //   (b) a typed bounded rejection before any body was committed;
    //   (c) a `200` whose headers were already flushed (SPARQL results are
    //       streamed) before the isolated child was killed by its own
    //       `cpu_time_millis`/`wall_time_millis` ceiling -- HTTP cannot
    //       change a committed status line, so this surfaces as a body that
    //       ends before its chunked terminator, not a clean 5xx.
    if response.starts_with("HTTP/1.1 200") {
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        if try_decode_chunked_body(headers, body).is_some() {
            // A genuinely complete response must be the exact correct
            // answer, not merely "some" well-formed body.
            assert_exact_body(&response);
        }
    } else {
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
    }
    // Child cleanup and capacity recovery: the isolated peer's process is
    // reaped and the next ordinary request succeeds under the same
    // `--max-concurrent-requests 1` cap.
    server.assert_exact_recovery();
}
