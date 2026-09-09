//! Actual binary/startup proof for the bounded two-source UNION profile.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const BINARY: &str = env!("CARGO_BIN_EXE_semantic-fabric");
const QUERY: &str = "SELECT ?s ?value WHERE { \
    { ?s <http://example.test/left> ?value } UNION \
    { ?s <http://example.test/right> ?value } }";
static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);
#[path = "federated_union_serve/join.rs"]
mod join;
#[path = "federated_union_serve/processor_base.rs"]
mod processor_base;
#[path = "federated_union_serve/reload.rs"]
mod reload;

struct Fixture {
    root: PathBuf,
    paths: Vec<PathBuf>,
}

impl Fixture {
    fn new() -> Self {
        loop {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "sf_cli_federated_{}_{}_{}",
                std::process::id(),
                timestamp,
                sequence
            ));
            match std::fs::create_dir(&root) {
                Ok(()) => {
                    return Self {
                        root,
                        paths: Vec::new(),
                    };
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create isolated fixture directory: {error}"),
            }
        }
    }

    fn path(&mut self, suffix: &str) -> PathBuf {
        let path = self.root.join(suffix);
        self.paths.push(path.clone());
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
        }
        let _ = std::fs::remove_dir(&self.root);
    }
}

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn mapping(predicate: &str) -> String {
    format!(
        r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> a rr:TriplesMap ;
  rr:logicalTable [ rr:tableName "items" ] ;
  rr:subjectMap [ rr:constant <http://example.test/item> ] ;
  rr:predicateObjectMap [ rr:predicate <{predicate}> ; rr:objectMap [ rr:column "value" ] ] ."#
    )
}

fn database(path: &PathBuf) {
    let connection = rusqlite::Connection::open(path).expect("create SQLite fixture");
    connection
        .execute_batch(
            "CREATE TABLE items(value TEXT NOT NULL); INSERT INTO items VALUES ('same');",
        )
        .expect("populate SQLite fixture");
}

fn available_address() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve test address");
    let address = listener.local_addr().unwrap();
    drop(listener);
    address
}

fn request(address: SocketAddr) -> Option<String> {
    request_with_token(address, None).filter(|response| response.starts_with("HTTP/1.1 200"))
}

fn request_with_token(address: SocketAddr, token: Option<&str>) -> Option<String> {
    request_query(address, QUERY, token)
}

fn request_query(address: SocketAddr, query: &str, token: Option<&str>) -> Option<String> {
    let auth = token
        .map(|token| format!("Authorization: Bearer {token}\r\n"))
        .unwrap_or_default();
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(100)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    write!(
        stream,
        "POST /sparql HTTP/1.1\r\nHost: {address}\r\n{auth}Connection: close\r\nContent-Type: application/sparql-query\r\nAccept: application/sparql-results+json\r\nContent-Length: {}\r\n\r\n{query}",
        query.len()
    )
    .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    Some(response)
}

fn metrics(address: SocketAddr) -> Option<String> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(100)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    write!(
        stream,
        "GET /metrics HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    response.starts_with("HTTP/1.1 200").then_some(response)
}

fn start_server(enable_metrics: bool) -> (Fixture, SocketAddr, Server) {
    start_with_token(enable_metrics, None)
}

fn start_with_token(enable_metrics: bool, token: Option<&str>) -> (Fixture, SocketAddr, Server) {
    start_configured(enable_metrics, token, false)
}

fn start_configured(
    enable_metrics: bool,
    token: Option<&str>,
    use_config: bool,
) -> (Fixture, SocketAddr, Server) {
    start_reloading(enable_metrics, token, use_config, 0)
}

fn start_reloading(
    enable_metrics: bool,
    token: Option<&str>,
    use_config: bool,
    reload_interval: u64,
) -> (Fixture, SocketAddr, Server) {
    let mut fixture = Fixture::new();
    let first_db = fixture.path("first.db");
    let second_db = fixture.path("second.db");
    let first_mapping = fixture.path("first.ttl");
    let second_mapping = fixture.path("second.ttl");
    let ontology = fixture.path("ontology.ttl");
    database(&first_db);
    database(&second_db);
    std::fs::write(&first_mapping, mapping("http://example.test/left")).unwrap();
    std::fs::write(&second_mapping, mapping("http://example.test/right")).unwrap();
    std::fs::write(
        &ontology,
        "@prefix owl: <http://www.w3.org/2002/07/owl#> .\n\
         <http://example.test/left> a owl:DatatypeProperty .\n\
         <http://example.test/right> a owl:DatatypeProperty .\n",
    )
    .unwrap();

    let address = available_address();
    let mut command = Command::new(BINARY);
    command.env_clear();
    if use_config {
        let config = fixture.path("serve.toml");
        let value = toml::toml! {
            [source]
            source_env = "SF_TEST_SOURCE"
            source_env_2 = "SF_TEST_SOURCE_2"
            [mappings]
            mapping = (first_mapping.to_str().unwrap())
            mapping_2 = (second_mapping.to_str().unwrap())
            [graphs]
            ontology = (ontology.to_str().unwrap())
            [governance]
            timeout_secs = 0
            [serve]
            bind = "invalid-file-bind"
            [observability]
            metrics = true
            log_level = "invalid-file-level"
            [security]
            auth_token_env = "SF_OLD_TOKEN"
            allow_unauthenticated = false
        };
        std::fs::write(&config, toml::to_string(&value).unwrap()).unwrap();
        command
            .args(["serve", "--config"])
            .arg(config)
            .args([
                "--bind",
                &address.to_string(),
                "--auth-token-env",
                "SF_TEST_QUERY_BEARER",
            ])
            .arg(format!("--metrics={enable_metrics}"))
            .env("SF_TEST_SOURCE", format!("sqlite:{}", first_db.display()))
            .env(
                "SF_TEST_SOURCE_2",
                format!("sqlite:{}", second_db.display()),
            )
            .env("SF_TEST_QUERY_BEARER", token.unwrap())
            .env("SEMANTIC_FABRIC_TIMEOUT_SECS", "30")
            .env("SEMANTIC_FABRIC_LOG_LEVEL", "info")
            .env("SEMANTIC_FABRIC_BIND", "invalid-env-bind");
    } else {
        command.args([
            "serve",
            "--source",
            &format!("sqlite:{}", first_db.display()),
            "--mapping",
            first_mapping.to_str().unwrap(),
            "--source-2",
            &format!("sqlite:{}", second_db.display()),
            "--mapping-2",
            second_mapping.to_str().unwrap(),
            "--ontology",
            ontology.to_str().unwrap(),
            "--bind",
            &address.to_string(),
        ]);
        if let Some(token) = token {
            command
                .args(["--auth-token-env", "SF_TEST_QUERY_BEARER"])
                .env("SF_TEST_QUERY_BEARER", token);
        } else {
            command.arg("--allow-unauthenticated");
        }
        if enable_metrics {
            command.arg("--metrics");
        }
    }
    let child = command
        .args(["--reload-interval-secs", &reload_interval.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("launch semantic-fabric server");
    (fixture, address, Server(child))
}

fn wait_for_query(address: SocketAddr, server: &mut Server) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(response) = request(address) {
            return response;
        }
        if let Some(status) = server.0.try_wait().expect("inspect server") {
            panic!("server exited before serving: {status}");
        }
        assert!(Instant::now() < deadline, "server startup timed out");
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn cli_serves_the_two_source_union_vertical() {
    let (_fixture, address, mut server) = start_server(false);
    let response = wait_for_query(address, &mut server);
    let body = response.split_once("\r\n\r\n").unwrap().1;
    assert_eq!(body.matches("\"value\":\"same\"").count(), 2, "{body}");
}

#[test]
fn cli_bearer_reference_protects_real_two_source_queries() {
    assert_authenticated_union(false);
}

#[test]
fn layered_configuration_serves_authenticated_union_with_effective_overrides() {
    assert_authenticated_union(true);
}

fn assert_authenticated_union(use_config: bool) {
    const TOKEN: &str = "test-only-native-cli-token-0123456789";
    let (_fixture, address, mut server) = start_configured(false, Some(TOKEN), use_config);
    let deadline = Instant::now() + Duration::from_secs(10);
    let denied = loop {
        if let Some(response) = request_with_token(address, None) {
            break response;
        }
        assert!(server.0.try_wait().unwrap().is_none());
        assert!(
            Instant::now() < deadline,
            "protected server startup timed out"
        );
        thread::sleep(Duration::from_millis(25));
    };
    assert!(denied.starts_with("HTTP/1.1 401"));
    let allowed = request_with_token(address, Some(TOKEN)).unwrap();
    assert!(allowed.starts_with("HTTP/1.1 200"));
    assert_eq!(allowed.matches("\"value\":\"same\"").count(), 2);
    let wrong = request_with_token(address, Some("wrong")).unwrap();
    assert!(wrong.starts_with("HTTP/1.1 401"));
    assert!(!wrong.contains(TOKEN));
    assert!(
        metrics(address).is_none(),
        "false CLI metrics override must keep the route disabled"
    );
}

#[test]
fn cli_exposes_only_opted_in_bounded_prometheus_metrics() {
    let (_fixture, address, mut server) = start_server(true);
    let query_response = wait_for_query(address, &mut server);
    assert!(query_response.starts_with("HTTP/1.1 200"));

    let response = metrics(address).expect("enabled metrics endpoint");
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    assert!(
        headers.contains("content-type: text/plain; version=0.0.4; charset=utf-8"),
        "headers={headers}"
    );
    assert!(
        headers.contains("cache-control: no-store"),
        "headers={headers}"
    );
    assert!(headers.contains("x-content-type-options: nosniff"));
    assert!(
        body.contains("# TYPE sf_query_total counter"),
        "body={body}"
    );
    assert!(body.contains("sf_query_total{"), "body={body}");
    assert!(body.contains("status=\"success\""), "body={body}");
    assert!(body.contains("body=\"complete\""), "body={body}");
    assert!(
        body.contains("# TYPE sf_query_duration_seconds histogram"),
        "body={body}"
    );
    assert!(body.contains("sf_query_duration_seconds_bucket{"));
    for forbidden in [QUERY, "http://example.test/left", "sqlite:"] {
        assert!(
            !body.contains(forbidden),
            "forbidden={forbidden}, body={body}"
        );
    }
}

#[cfg(unix)]
#[test]
fn cli_handles_real_sigterm_and_exits_cleanly() {
    let (_fixture, address, mut server) = start_server(false);
    let _ = wait_for_query(address, &mut server);

    // SAFETY: `server` owns this live child PID, and `kill` does not retain the pointer-free
    // integer argument. The child is reaped below and again defensively by `Drop`.
    let sent = unsafe { libc::kill(server.0.id() as libc::pid_t, libc::SIGTERM) };
    assert_eq!(sent, 0, "send SIGTERM to semantic-fabric child");

    let deadline = Instant::now() + Duration::from_secs(3);
    let status = loop {
        if let Some(status) = server.0.try_wait().expect("inspect shutdown") {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "server did not handle SIGTERM within the test bound"
        );
        thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success(), "SIGTERM shutdown status: {status}");
    assert!(
        TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_err(),
        "listener still accepted after process shutdown"
    );
}
