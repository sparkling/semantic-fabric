//! Isolated startup evidence, never a provisioned Query admission.
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const TOKEN: &str = "fixture-only-generated-profile-0123456789";
const SELECT: &str = "SELECT ?value WHERE { ?s <http://example.test/value> ?value }";
const ASK: &str = "ASK { ?s <http://example.test/value> ?value }";
const CONSTRUCT: &str = "CONSTRUCT { ?s <http://example.test/value> ?value } WHERE { ?s <http://example.test/value> ?value }";
const MAPPING: &str = r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> a rr:TriplesMap; rr:logicalTable [rr:tableName "items"];
rr:subjectMap [rr:template "http://example.test/item/{id}"];
rr:predicateObjectMap [rr:predicate <http://example.test/value>; rr:objectMap [rr:column "value"]]."#;
const ONTOLOGY: &str = "<http://example.test/value> a <http://www.w3.org/2002/07/owl#DatatypeProperty> . <http://example.test/shape> <http://www.w3.org/ns/shacl#property> [ <http://www.w3.org/ns/shacl#path> <http://example.test/value>; <http://www.w3.org/ns/shacl#datatype> <http://www.w3.org/2001/XMLSchema#integer> ] .";
const PROFILE: &str = "x-semantic-fabric-profile";

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("sf-generated-startup-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        let fixture = Self(path);
        rusqlite::Connection::open(fixture.0.join("source.db")).unwrap()
            .execute_batch("CREATE TABLE items(id INTEGER, value INTEGER NOT NULL); INSERT INTO items VALUES(1,7)").unwrap();
        std::fs::write(fixture.0.join("mapping.ttl"), MAPPING).unwrap();
        std::fs::write(fixture.0.join("ontology.ttl"), ONTOLOGY).unwrap();
        fixture
    }
    fn command(&self, profile: Option<&str>, bearer: bool) -> (Command, SocketAddr) {
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
                "--bind",
                &address.to_string(),
                "--shutdown-timeout-secs",
                "1",
            ]);
        if let Some(profile) = profile {
            command.args(["--query-shape-profile", profile]);
        }
        if bearer {
            command
                .args(["--auth-token-env", "SF_GENERATED_TEST_TOKEN"])
                .env("SF_GENERATED_TEST_TOKEN", TOKEN);
        }
        (command, address)
    }
    fn start(&self, command: &mut Command, address: SocketAddr) -> Server {
        let mut server = Server(
            command
                .stdout(Stdio::null())
                .stderr(std::fs::File::create(self.0.join("stderr")).unwrap())
                .spawn()
                .unwrap(),
        );
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            if send(address, None, false, false).is_some_and(|reply| reply.status == 200) {
                return server;
            }
            assert!(
                server.0.try_wait().unwrap().is_none(),
                "startup exited: {}",
                self.diagnostics()
            );
            assert!(
                Instant::now() < until,
                "startup timed out: {}",
                self.diagnostics()
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    fn diagnostics(&self) -> String {
        std::fs::read_to_string(self.0.join("stderr")).unwrap_or_default()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Exclusive UUID directory containing only this fixture's own files.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Reply {
    status: u16,
    identity: Option<String>,
    body: Vec<u8>,
}
impl Reply {
    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

fn send(
    address: SocketAddr,
    query: Option<&str>,
    authenticated: bool,
    spoof: bool,
) -> Option<Reply> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(100)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .ok()?;
    let auth = if authenticated {
        format!("Authorization: Bearer {TOKEN}\r\n")
    } else {
        String::new()
    };
    let claim = if spoof {
        "x-semantic-fabric-profile: sfgp1:forged\r\n"
    } else {
        ""
    };
    let (method, path) = if query.is_some() {
        ("POST", "/sparql")
    } else {
        ("GET", "/readyz")
    };
    let query = query.unwrap_or("");
    write!(stream, "{method} {path} HTTP/1.1\r\nHost: {address}\r\n{auth}{claim}Connection: close\r\nContent-Type: application/sparql-query\r\nAccept: application/sparql-results+json\r\nContent-Length: {}\r\n\r\n{query}", query.len()).ok()?;
    let mut wire = Vec::new();
    stream.take(1 << 20).read_to_end(&mut wire).ok()?;
    let split = wire.windows(4).position(|v| v == b"\r\n\r\n")?;
    let headers = std::str::from_utf8(&wire[..split]).ok()?;
    let status = headers.split_whitespace().nth(1)?.parse().ok()?;
    let identity = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case(PROFILE))
        .map(|(_, value)| value.trim().to_owned());
    let body = &wire[split + 4..];
    let chunked = headers
        .lines()
        .any(|line| line.eq_ignore_ascii_case("transfer-encoding: chunked"));
    let body = if chunked {
        unchunk(body)?
    } else {
        body.to_vec()
    };
    Some(Reply {
        status,
        identity,
        body,
    })
}
fn unchunk(mut body: &[u8]) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    loop {
        let end = body.windows(2).position(|v| v == b"\r\n")?;
        let length = usize::from_str_radix(
            std::str::from_utf8(&body[..end]).ok()?.split(';').next()?,
            16,
        )
        .ok()?;
        body = body.get(end + 2..)?;
        if length == 0 {
            return Some(output);
        }
        output.extend_from_slice(body.get(..length)?);
        if body.get(length..length.checked_add(2)?)? != b"\r\n" {
            return None;
        }
        body = body.get(length + 2..)?;
    }
}

#[test]
fn cli_selects_generated_profile_without_changing_ordinary_or_subject_admission() {
    for profile in [None, Some("ordinary"), Some("generated-select-ask")] {
        let fixture = Fixture::new();
        let (mut command, address) = fixture.command(profile, true);
        let _server = fixture.start(&mut command, address);
        let generated = profile == Some("generated-select-ask");
        let denied = send(address, Some(SELECT), false, false).unwrap();
        assert_eq!(denied.status, 401);
        assert!(denied.identity.is_none());
        let mut previous = None;
        for query in [SELECT, ASK, SELECT] {
            let reply = send(address, Some(query), true, true).unwrap();
            assert_eq!(
                reply.status,
                200,
                "{}",
                String::from_utf8_lossy(&reply.body)
            );
            if query == ASK {
                assert_eq!(reply.json()["boolean"], true);
            } else {
                assert_eq!(
                    reply.json()["results"]["bindings"][0]["value"]["value"],
                    "7"
                );
            }
            assert_eq!(reply.identity.is_some(), generated);
            if generated {
                let identity = reply.identity.unwrap();
                assert!(identity.starts_with("sfgp1:"));
                assert_eq!(identity.len(), 70);
                if let Some(previous) = previous {
                    assert_eq!(identity, previous);
                }
                previous = Some(identity);
            }
        }
        if generated {
            for query in [
                CONSTRUCT,
                "SELECT ?s FROM <http://example.test/g> WHERE { ?s ?p ?o }",
                "SELECT ?s FROM NAMED <http://example.test/g> WHERE { ?s ?p ?o }",
                "SELECT ?s WHERE { ?s <http://example.test/uncovered> ?o }",
            ] {
                let reply = send(address, Some(query), true, false).unwrap();
                assert_eq!(
                    reply.status,
                    501,
                    "{}",
                    String::from_utf8_lossy(&reply.body)
                );
                assert!(reply.identity.is_none());
                assert_eq!(reply.json()["code"], "unsupported-query");
            }
        }
    }
}

#[test]
fn generated_profile_does_not_replace_default_deny() {
    let fixture = Fixture::new();
    let (mut command, address) = fixture.command(Some("generated-select-ask"), false);
    let _server = fixture.start(&mut command, address);
    let reply = send(address, Some(SELECT), true, false).unwrap();
    assert_eq!(reply.status, 403);
    assert!(reply.identity.is_none());
}

#[test]
fn invalid_profile_layers_fail_redacted_before_source_open() {
    const SENTINEL: &str = "PRIVATE-PROFILE-SENTINEL";
    for layer in ["cli", "env", "file"] {
        let fixture = Fixture::new();
        let mut command = Command::new(env!("CARGO_BIN_EXE_semantic-fabric"));
        command.env_clear().args([
            "serve",
            "--source-env",
            "SF_MUST_NOT_READ_SOURCE",
            "--mapping",
            "/mapping/not/read",
            "--ontology",
            "/ontology/not/read",
        ]);
        match layer {
            "cli" => {
                command.args(["--query-shape-profile", SENTINEL]);
            }
            "env" => {
                command.env("SEMANTIC_FABRIC_QUERY_SHAPE_PROFILE", SENTINEL);
            }
            _ => {
                let path = fixture.0.join("config.toml");
                std::fs::write(
                    &path,
                    format!("[serve]\nquery_shape_profile = \"{SENTINEL}\"\n"),
                )
                .unwrap();
                command.arg("--config").arg(path);
            }
        }
        let mut process = Server(
            command
                .stdout(Stdio::null())
                .stderr(std::fs::File::create(fixture.0.join("stderr")).unwrap())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(20);
        let status = loop {
            if let Some(status) = process.0.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "invalid-profile startup timed out"
            );
            std::thread::sleep(Duration::from_millis(25));
        };
        assert!(!status.success());
        let stderr = fixture.diagnostics();
        assert!(!stderr.contains(SENTINEL), "{stderr}");
        assert!(!stderr.contains("SF_MUST_NOT_READ_SOURCE"), "{stderr}");
        assert!(
            stderr.contains("profile") || stderr.contains("configuration"),
            "{stderr}"
        );
    }
}
