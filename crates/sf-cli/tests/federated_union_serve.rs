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
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(100)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    write!(
        stream,
        "POST /sparql HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Type: application/sparql-query\r\nAccept: application/sparql-results+json\r\nContent-Length: {}\r\n\r\n{QUERY}",
        QUERY.len()
    )
    .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    response.starts_with("HTTP/1.1 200").then_some(response)
}

fn start_server() -> (Fixture, SocketAddr, Server) {
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
    let child = Command::new(BINARY)
        .args([
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
        ])
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
    let (_fixture, address, mut server) = start_server();
    let response = wait_for_query(address, &mut server);
    let body = response.split_once("\r\n\r\n").unwrap().1;
    assert_eq!(body.matches("\"value\":\"same\"").count(), 2, "{body}");
}

#[cfg(unix)]
#[test]
fn cli_handles_real_sigterm_and_exits_cleanly() {
    let (_fixture, address, mut server) = start_server();
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
