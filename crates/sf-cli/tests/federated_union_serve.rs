//! Actual binary/startup proof for the bounded two-source UNION profile.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const BINARY: &str = env!("CARGO_BIN_EXE_semantic-fabric");
const QUERY: &str = "SELECT ?s ?value WHERE { \
    { ?s <http://example.test/left> ?value } UNION \
    { ?s <http://example.test/right> ?value } }";

struct Fixture {
    paths: Vec<PathBuf>,
}

impl Fixture {
    fn path(&mut self, suffix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sf_cli_federated_{suffix}_{}_{unique}",
            std::process::id()
        ));
        self.paths.push(path.clone());
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
        }
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

#[test]
fn cli_serves_the_two_source_union_vertical() {
    let mut fixture = Fixture { paths: Vec::new() };
    let first_db = fixture.path("first.db");
    let second_db = fixture.path("second.db");
    let first_mapping = fixture.path("first.ttl");
    let second_mapping = fixture.path("second.ttl");
    database(&first_db);
    database(&second_db);
    std::fs::write(&first_mapping, mapping("http://example.test/left")).unwrap();
    std::fs::write(&second_mapping, mapping("http://example.test/right")).unwrap();

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
            "--bind",
            &address.to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("launch semantic-fabric server");
    let mut server = Server(child);

    let deadline = Instant::now() + Duration::from_secs(10);
    let response = loop {
        if let Some(response) = request(address) {
            break response;
        }
        if let Some(status) = server.0.try_wait().expect("inspect server") {
            panic!("server exited before serving: {status}");
        }
        assert!(Instant::now() < deadline, "server startup timed out");
        thread::sleep(Duration::from_millis(25));
    };
    let body = response.split_once("\r\n\r\n").unwrap().1;
    assert_eq!(body.matches("\"value\":\"same\"").count(), 2, "{body}");
}
