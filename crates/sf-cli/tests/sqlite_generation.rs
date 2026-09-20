//! The shipped CLI's protected SQLite profile, reload, refusal and recovery.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const SELECT: &str = "SELECT ?value WHERE { ?s <http://example.test/value> ?value }";
const MAPPING: &str = r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> a rr:TriplesMap; rr:logicalTable [rr:tableName "items"];
rr:subjectMap [rr:template "http://example.test/item/{id}"];
rr:predicateObjectMap [rr:predicate <http://example.test/value>; rr:objectMap [rr:column "value"]]."#;
const ONTOLOGY: &str = "<http://example.test/value> a <http://www.w3.org/2002/07/owl#DatatypeProperty> . <http://example.test/shape> <http://www.w3.org/ns/shacl#property> [ <http://www.w3.org/ns/shacl#path> <http://example.test/value>; <http://www.w3.org/ns/shacl#datatype> <http://www.w3.org/2001/XMLSchema#integer> ] .";
const TOKEN: &str = "fixture-only-sqlite-generation-0123456789";

struct Fixture(PathBuf);
impl Fixture {
    fn new(journal: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("sf-sqlite-generation-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let fixture = Self(root);
        fixture.sql(&format!("PRAGMA journal_mode={journal}; CREATE TABLE items(id INTEGER, value INTEGER NOT NULL); INSERT INTO items VALUES(1,7),(2,7)"));
        fixture.write("mapping.ttl", MAPPING);
        fixture.write("ontology.ttl", ONTOLOGY);
        fixture
    }
    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.0.join(name), text).unwrap();
    }
    fn sql(&self, sql: &str) {
        let conn = rusqlite::Connection::open(self.0.join("source.db")).unwrap();
        if sql.starts_with("PRAGMA") {
            conn.execute_batch(sql).unwrap();
        } else {
            conn.execute_batch(&format!("BEGIN IMMEDIATE; {sql}; COMMIT"))
                .unwrap();
        }
    }
    fn command(&self) -> (Command, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_semantic-fabric"));
        command
            .env_clear()
            .args(["serve", "--source"])
            .arg(format!("sqlite:{}", self.0.join("source.db").display()))
            .arg("--mapping")
            .arg(self.0.join("mapping.ttl"))
            .arg("--ontology")
            .arg(self.0.join("ontology.ttl"))
            .args([
                "--auth-token-env",
                "SF_SQLITE_GENERATION_BEARER",
                "--require-verified-generation",
                "--reload-interval-secs",
                "1",
                "--sqlite-pool-size",
                "1",
                "--shutdown-timeout-secs",
                "1",
                "--bind",
                &address.to_string(),
            ])
            .env("SF_SQLITE_GENERATION_BEARER", TOKEN);
        (command, address)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // This UUID directory contains only this fixture's own generated files.
        for entry in std::fs::read_dir(&self.0).unwrap().flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
        let _ = std::fs::remove_dir(&self.0);
    }
}
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn request(
    address: SocketAddr,
    route: &str,
    query: Option<&str>,
    accept: &str,
    token: Option<&str>,
) -> Option<(u16, Vec<u8>)> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(100)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .ok()?;
    let auth = token
        .map(|token| format!("Authorization: Bearer {token}\r\n"))
        .unwrap_or_default();
    let method = if query.is_some() { "POST" } else { "GET" };
    let query = query.unwrap_or("");
    write!(stream, "{method} {route} HTTP/1.1\r\nHost: {address}\r\n{auth}Connection: close\r\nContent-Type: application/sparql-query\r\nAccept: {accept}\r\nContent-Length: {}\r\n\r\n{query}", query.len()).ok()?;
    let mut wire = Vec::new();
    stream.take(1024 * 1024).read_to_end(&mut wire).ok()?;
    let split = wire.windows(4).position(|w| w == b"\r\n\r\n")?;
    let headers = std::str::from_utf8(&wire[..split]).ok()?;
    let status = headers.split_whitespace().nth(1)?.parse().ok()?;
    let mut rest = &wire[split + 4..];
    if !headers
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        return Some((status, rest.to_vec()));
    }
    let mut body = Vec::new();
    loop {
        let line = rest.windows(2).position(|w| w == b"\r\n")?;
        let size = usize::from_str_radix(std::str::from_utf8(&rest[..line]).ok()?, 16).ok()?;
        rest = &rest[line + 2..];
        if size == 0 {
            assert_eq!(rest, b"\r\n");
            break;
        }
        body.extend_from_slice(rest.get(..size)?);
        assert_eq!(rest.get(size..size + 2)?, b"\r\n");
        rest = rest.get(size + 2..)?;
    }
    Some((status, body))
}

fn ready(address: SocketAddr) -> Option<u16> {
    request(address, "/readyz", None, "application/json", None).map(|r| r.0)
}

fn wait(fixture: &Fixture, server: &mut Server, condition: impl Fn() -> bool) {
    let until = Instant::now() + Duration::from_secs(20);
    while !condition() {
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "server exited: {}",
            std::fs::read_to_string(fixture.0.join("stderr")).unwrap()
        );
        assert!(
            Instant::now() < until,
            "server failed to reach expected state: {}",
            std::fs::read_to_string(fixture.0.join("stderr")).unwrap()
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn start(fixture: &Fixture, mut command: Command, address: SocketAddr) -> Server {
    let mut server = Server(
        command
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(fixture.0.join("stderr")).unwrap())
            .spawn()
            .unwrap(),
    );
    wait(fixture, &mut server, || ready(address) == Some(200));
    server
}

fn values(address: SocketAddr, expected: &str) {
    let (status, body) = request(
        address,
        "/sparql",
        Some(SELECT),
        "application/sparql-results+json",
        Some(TOKEN),
    )
    .unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let value = serde_json::json!({"value":{"type":"literal","datatype":"http://www.w3.org/2001/XMLSchema#integer","value":expected}});
    assert_eq!(
        json["results"]["bindings"],
        serde_json::json!([value.clone(), value])
    );
}

#[test]
fn protected_sqlite_cli_serves_reloads_rejects_incompatible_schema_and_recovers() {
    for journal in ["WAL", "DELETE"] {
        let fixture = Fixture::new(journal);
        let (command, address) = fixture.command();
        let mut server = start(&fixture, command, address);
        values(address, "7");
        values(address, "7");
        assert_eq!(
            request(
                address,
                "/sparql",
                Some(SELECT),
                "application/sparql-results+json",
                None
            )
            .unwrap()
            .0,
            401
        );
        let (status, body) = request(
            address,
            "/sparql",
            Some("ASK { ?s <http://example.test/value> ?v }"),
            "application/sparql-results+json",
            Some(TOKEN),
        )
        .unwrap();
        assert_eq!(status, 200);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["boolean"],
            true
        );
        let construct = "CONSTRUCT { ?s <http://example.test/value> ?v } WHERE { ?s <http://example.test/value> ?v }";
        let (status, body) = request(
            address,
            "/sparql",
            Some(construct),
            "application/n-triples",
            Some(TOKEN),
        )
        .unwrap();
        assert_eq!(status, 200);
        assert_eq!(String::from_utf8(body).unwrap().lines().count(), 2);
        let (status, body) = request(
            address,
            "/sparql",
            Some(SELECT),
            "application/vnd.semantic-fabric.lineage+json-seq",
            Some(TOKEN),
        )
        .unwrap();
        assert_eq!(status, 200);
        let records: Vec<serde_json::Value> = body
            .split(|b| *b == 0x1e)
            .filter(|p| !p.is_empty())
            .map(|p| serde_json::from_slice(p).unwrap())
            .collect();
        assert_eq!(
            records.last().unwrap(),
            &serde_json::json!({"type":"complete","solutions":2})
        );

        fixture.sql("ALTER TABLE items RENAME TO old_items; CREATE TABLE items(id INTEGER, value TEXT); INSERT INTO items VALUES(1,'incompatible'),(2,'incompatible'); DROP TABLE old_items");
        wait(&fixture, &mut server, || ready(address) == Some(503));
        let (status, body) = request(
            address,
            "/sparql",
            Some(SELECT),
            "application/sparql-results+json",
            Some(TOKEN),
        )
        .unwrap();
        assert_eq!(status, 503);
        assert!(!String::from_utf8_lossy(&body).contains(fixture.0.to_str().unwrap()));
        assert!(!String::from_utf8_lossy(&body).contains(TOKEN));
        fixture.sql("DROP TABLE items; CREATE TABLE items(id INTEGER, value INTEGER); INSERT INTO items VALUES(1,9),(2,9)");
        wait(&fixture, &mut server, || ready(address) == Some(200));
        values(address, "9");
        fixture.write(
            "mapping.ttl",
            &MAPPING.replace("rr:column \"value\"", "rr:column \"successor\""),
        );
        wait(&fixture, &mut server, || ready(address) == Some(503));
        fixture.sql("ALTER TABLE items ADD COLUMN successor INTEGER NOT NULL DEFAULT 11");
        wait(&fixture, &mut server, || ready(address) == Some(200));
        values(address, "11");
        #[cfg(unix)]
        {
            assert_eq!(
                unsafe { libc::kill(server.0.id() as i32, libc::SIGTERM) },
                0
            );
            let until = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(status) = server.0.try_wait().unwrap() {
                    assert!(status.success());
                    break;
                }
                assert!(Instant::now() < until, "protected SQLite shutdown stalled");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

#[test]
fn protected_sqlite_source_work_refusal_has_no_unverified_fallback() {
    let fixture = Fixture::new("WAL");
    let (mut command, address) = fixture.command();
    command.args(["--max-source-work", "0"]);
    let _server = start(&fixture, command, address);
    let (status, _) = request(
        address,
        "/sparql",
        Some(SELECT),
        "application/sparql-results+json",
        Some(TOKEN),
    )
    .unwrap();
    assert_eq!(status, 429);
}
