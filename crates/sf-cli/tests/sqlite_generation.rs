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
        self.command_with_admission(&["--auth-token-env", "SF_SQLITE_GENERATION_BEARER"])
    }
    fn command_with_admission(&self, admission: &[&str]) -> (Command, SocketAddr) {
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
            .args(admission)
            .args([
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

const TOKEN_B: &str = "fixture-only-sqlite-generation-b-0123456789";
const TOKEN_DENIED: &str = "fixture-only-sqlite-generation-denied-0123456789";

fn portable_command(fixture: &Fixture) -> (Command, SocketAddr) {
    let (mut command, address) =
        fixture.command_with_admission(&["--auth-subjects-env", "SF_SUBJECTS"]);
    let subjects: Vec<_> = [("a", "SF_A", "items"), ("b", "SF_B", "items"), ("denied", "SF_DENIED", "other")]
        .into_iter().map(|(subject, credential, table)| serde_json::json!({
            "subjectRef":subject,"credentialEnv":credential,
            "portableRows":[{"sourceIndex":0,"table":table,"column":"tenant","valueEnv":format!("{credential}_VALUE")}]
        })).collect();
    command
        .env(
            "SF_SUBJECTS",
            serde_json::json!({"schemaVersion":2,"subjects":subjects}).to_string(),
        )
        .env("SF_A", TOKEN)
        .env("SF_B", TOKEN_B)
        .env("SF_DENIED", TOKEN_DENIED)
        .env("SF_A_VALUE", "a")
        .env("SF_B_VALUE", "b")
        .env("SF_DENIED_VALUE", "denied");
    (command, address)
}

fn policy_values(address: SocketAddr, token: &str) -> Option<Vec<String>> {
    let (status, body) = request(
        address,
        "/sparql",
        Some(SELECT),
        "application/sparql-results+json",
        Some(token),
    )?;
    if status != 200 {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let mut values: Vec<_> = json["results"]["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["value"]["value"].as_str().unwrap().to_owned())
        .collect();
    values.sort();
    Some(values)
}

#[test]
fn protected_sqlite_portable_subjects_remain_isolated_across_reload_and_denial() {
    for journal in ["WAL", "DELETE"] {
        let fixture = Fixture::new(journal);
        fixture.sql("ALTER TABLE items ADD COLUMN tenant TEXT NOT NULL DEFAULT 'a'; INSERT INTO items VALUES(3,8,'b'),(4,8,'b')");
        let (command, address) = portable_command(&fixture);
        let mut server = start(&fixture, command, address);
        for _ in 0..2 {
            assert_eq!(
                policy_values(address, TOKEN),
                Some(vec!["7".into(), "7".into()])
            );
            assert_eq!(
                policy_values(address, TOKEN_B),
                Some(vec!["8".into(), "8".into()])
            );
        }
        let ordered = format!("{SELECT} ORDER BY ?value");
        for query in [SELECT, ordered.as_str()] {
            assert_eq!(
                request(
                    address,
                    "/sparql",
                    Some(query),
                    "application/sparql-results+json",
                    Some(TOKEN_DENIED)
                )
                .unwrap()
                .0,
                403
            );
        }
        assert_eq!(
            request(
                address,
                "/sparql",
                Some(&ordered),
                "application/sparql-results+json",
                Some(TOKEN)
            )
            .unwrap()
            .0,
            501
        );
        fixture.write(
            "mapping.ttl",
            &MAPPING.replace("rr:column \"value\"", "rr:column \"successor\""),
        );
        wait(&fixture, &mut server, || ready(address) == Some(503));
        fixture.sql("ALTER TABLE items ADD COLUMN successor INTEGER; UPDATE items SET successor=CASE tenant WHEN 'a' THEN 17 ELSE 18 END");
        wait(&fixture, &mut server, || {
            policy_values(address, TOKEN) == Some(vec!["17".into(), "17".into()])
        });
        assert_eq!(
            policy_values(address, TOKEN_B),
            Some(vec!["18".into(), "18".into()])
        );
        fixture.write("mapping.ttl", MAPPING);
        wait(&fixture, &mut server, || {
            policy_values(address, TOKEN) == Some(vec!["7".into(), "7".into()])
        });
        assert_eq!(
            policy_values(address, TOKEN_B),
            Some(vec!["8".into(), "8".into()])
        );
        fixture.sql("ALTER TABLE items RENAME COLUMN tenant TO tenant_drift");
        wait(&fixture, &mut server, || ready(address) == Some(503));
        let fenced_until = Instant::now() + Duration::from_millis(2500);
        while Instant::now() < fenced_until {
            assert_eq!(ready(address), Some(503));
            assert_eq!(
                request(
                    address,
                    "/sparql",
                    Some(SELECT),
                    "application/sparql-results+json",
                    Some(TOKEN)
                )
                .unwrap()
                .0,
                503
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        fixture.sql("ALTER TABLE items RENAME COLUMN tenant_drift TO tenant");
        wait(&fixture, &mut server, || {
            policy_values(address, TOKEN) == Some(vec!["7".into(), "7".into()])
        });
        assert_eq!(
            policy_values(address, TOKEN_B),
            Some(vec!["8".into(), "8".into()])
        );
        drop(server);

        // A denied caller cannot spend even the first generation-observation unit.
        let (mut command, address) = portable_command(&fixture);
        command.args(["--max-source-work", "0"]);
        let _server = start(&fixture, command, address);
        for (token, status) in [(TOKEN_DENIED, 403), (TOKEN, 429)] {
            assert_eq!(
                request(
                    address,
                    "/sparql",
                    Some(SELECT),
                    "application/sparql-results+json",
                    Some(token)
                )
                .unwrap()
                .0,
                status
            );
        }
    }
}
