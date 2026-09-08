//! Required live TLS qualification, always using test-owned disposable providers.
#![cfg(unix)]
#[path = "source_tls_live/cancellation.rs"]
mod cancellation;
#[path = "source_tls_live/join.rs"]
mod join;
#[path = "source_tls_live/lineage.rs"]
mod lineage;
#[path = "source_tls_live/multiple_lineage.rs"]
mod multiple_lineage;
#[path = "source_tls_live/reload.rs"]
mod reload;
#[path = "source_tls_live/stop_matrix.rs"]
mod stop_matrix;
#[path = "source_tls_live/support.rs"]
mod support;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use support::{Database, Fixture};

const SINGLE: &str = "SELECT ?s ?value WHERE { ?s <http://example.test/left> ?value }";
const UNION: &str = "SELECT ?s ?value WHERE { { ?s <http://example.test/left> ?value } UNION { ?s <http://example.test/right> ?value } }";

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn command(
    fixture: &Fixture,
    first: &Database,
    second: Option<&Database>,
) -> (Command, SocketAddr) {
    command_with_admission(
        fixture,
        first,
        second,
        &["--auth-token-env", "SF_TLS_BEARER"],
    )
}

fn command_with_admission(
    fixture: &Fixture,
    first: &Database,
    second: Option<&Database>,
    admission: &[&str],
) -> (Command, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_semantic-fabric"));
    command
        .env_clear()
        .args([
            "serve",
            "--source-env",
            "SF_TLS_SOURCE",
            "--source-tls-roots-env",
            "SF_TLS_ROOTS",
            "--mapping",
        ])
        .arg(fixture.root.join("first.ttl"))
        .arg("--ontology")
        .arg(fixture.root.join("ontology.ttl"))
        .args(admission)
        .args(["--bind", &address.to_string(), "--log-level", "info"])
        .env("SF_TLS_SOURCE", &first.source)
        .env("SF_TLS_ROOTS", &first.roots)
        .env("SF_TLS_BEARER", &fixture.token);
    if let Some(second) = second {
        command
            .args([
                "--source-env-2",
                "SF_TLS_SOURCE_2",
                "--source-tls-roots-env-2",
                "SF_TLS_ROOTS_2",
                "--mapping-2",
            ])
            .arg(fixture.root.join("second.ttl"))
            .env("SF_TLS_SOURCE_2", &second.source)
            .env("SF_TLS_ROOTS_2", &second.roots);
    }
    (command, address)
}

fn request(address: SocketAddr, query: &str, token: Option<&str>) -> Option<(u16, Vec<u8>)> {
    request_format(address, query, token, "application/sparql-results+json")
}

fn request_format(
    address: SocketAddr,
    query: &str,
    token: Option<&str>,
    accept: &str,
) -> Option<(u16, Vec<u8>)> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(100)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let authorization = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    write!(stream, "POST /sparql HTTP/1.1\r\nHost: {address}\r\n{authorization}Connection: close\r\nContent-Type: application/sparql-query\r\nAccept: {accept}\r\nContent-Length: {}\r\n\r\n{query}", query.len()).unwrap();
    let mut wire = Vec::new();
    stream.take(65537).read_to_end(&mut wire).unwrap();
    assert!(wire.len() <= 65536);
    if accept == lineage::FORMAT && wire.starts_with(b"HTTP/1.1 200 ") {
        let end = wire.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let headers = std::str::from_utf8(&wire[..end])
            .unwrap()
            .to_ascii_lowercase();
        assert!(headers
            .lines()
            .any(|line| line == format!("content-type: {accept}")));
    }
    Some(decode_response(wire))
}

fn decode_response(wire: Vec<u8>) -> (u16, Vec<u8>) {
    let boundary = wire.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let headers = std::str::from_utf8(&wire[..boundary]).unwrap();
    let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
    let mut body = wire[boundary + 4..].to_vec();
    if headers
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        let mut decoded = Vec::new();
        let mut rest = body.as_slice();
        loop {
            let line = rest.windows(2).position(|w| w == b"\r\n").unwrap();
            let size =
                usize::from_str_radix(std::str::from_utf8(&rest[..line]).unwrap(), 16).unwrap();
            rest = &rest[line + 2..];
            if size == 0 {
                assert_eq!(rest, b"\r\n");
                break;
            }
            decoded.extend_from_slice(&rest[..size]);
            assert_eq!(&rest[size..size + 2], b"\r\n");
            rest = &rest[size + 2..];
        }
        body = decoded;
    }
    (status, body)
}

fn assert_serves(
    fixture: &Fixture,
    first: &Database,
    second: Option<&Database>,
    expected: &[&str],
) {
    let (mut command, address) = command(fixture, first, second);
    let mut server = Server(
        command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let query = if second.is_some() { UNION } else { SINGLE };
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        if let Some((status, _)) = request(address, query, None) {
            assert_eq!(status, 401);
            break;
        }
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "serving child exited during TLS startup"
        );
        assert!(Instant::now() < deadline, "serving child did not start");
        thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(request(address, query, Some("wrong-token")).unwrap().0, 401);
    let (status, body) = request(address, query, Some(&fixture.token)).unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let document: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(document["head"]["vars"], serde_json::json!(["s", "value"]));
    let bindings = document["results"]["bindings"].as_array().unwrap();
    assert_eq!(bindings.len(), expected.len());
    let mut values = Vec::new();
    for binding in bindings {
        assert_eq!(binding.as_object().unwrap().len(), 2);
        assert_eq!(
            binding["s"],
            serde_json::json!({"type":"uri", "value":"http://example.test/item"})
        );
        assert_eq!(binding["value"]["type"], "literal");
        values.push(binding["value"]["value"].as_str().unwrap());
        assert!(binding["value"]["xml:lang"].is_null());
        assert!(
            binding["value"]["datatype"].is_null()
                || binding["value"]["datatype"] == "http://www.w3.org/2001/XMLSchema#string"
        );
    }
    values.sort_unstable();
    let mut expected = expected.to_vec();
    expected.sort_unstable();
    assert_eq!(
        values, expected,
        "exact bag includes multiplicity and source-specific values"
    );
    first.assert_encrypted_sessions();
    if let Some(second) = second {
        second.assert_encrypted_sessions();
    } else {
        lineage::assert_responses(address, fixture, first);
    }
}

fn assert_rejects(mut command: Command, address: SocketAddr, fixture: &Fixture) {
    let mut secrets = vec![fixture.token.clone()];
    for (_, value) in command.get_envs() {
        if let Some(value) = value.and_then(|value| value.to_str()) {
            if let Some((_, tail)) = value.split_once("password=") {
                secrets.push(tail.split_whitespace().next().unwrap().to_owned());
            }
            if let Some((_, tail)) = value.split_once("mysql://") {
                secrets.push(
                    tail.split_once('@')
                        .unwrap()
                        .0
                        .split_once(':')
                        .unwrap()
                        .1
                        .to_owned(),
                );
            }
        }
    }
    let result = support::output(&mut command, Duration::from_secs(40));
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    let events: Vec<serde_json::Value> = String::from_utf8_lossy(&result.stderr)
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()
        .unwrap_or_else(|error| {
            let mut diagnostic = String::from_utf8_lossy(&result.stderr).into_owned();
            for secret in &secrets {
                diagnostic = diagnostic.replace(secret, "[test-secret]");
            }
            panic!("startup diagnostic was not JSON ({error}): {diagnostic}");
        });
    // The stable public vocabulary deliberately groups connection/source errors.
    // These cases change only trust/name after the same inputs served successfully.
    let failure = events.last().unwrap();
    assert_eq!(failure["event"], "startup.failed");
    assert_eq!(failure["failure"], "startup-source");
    for event in &events {
        assert_eq!(event["schema"], "semantic-fabric.telemetry.v1");
        assert!(
            event["event"] == "startup.failed" || event["event"] == "source.schema_observation"
        );
    }
    assert!(result.stderr.len() < 1024);
    for secret in &secrets {
        assert!(
            !String::from_utf8_lossy(&result.stderr).contains(secret),
            "source credential or bearer escaped startup diagnostic"
        );
    }
    assert!(TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_err());
}

#[test]
#[ignore = "requires Docker and the pinned disposable PostgreSQL/MySQL images; required in CI"]
fn authenticated_public_queries_require_verified_source_tls() {
    let fixture = Fixture::new();
    support::mapping(&fixture.root.join("first.ttl"), "http://example.test/left");
    support::mapping(
        &fixture.root.join("second.ttl"),
        "http://example.test/right",
    );
    fixture.write("ontology.ttl", "@prefix owl: <http://www.w3.org/2002/07/owl#> .\n<http://example.test/left> a owl:DatatypeProperty .\n<http://example.test/right> a owl:DatatypeProperty .\n");
    let mut postgres = Database::start(&fixture, true);
    let mut mysql = Database::start(&fixture, false);
    assert_ne!(postgres.roots, mysql.roots);
    postgres.sql("ALTER TABLE public.items ADD COLUMN refreshed TEXT; UPDATE public.items SET refreshed='postgres-reloaded'");
    mysql.sql("ALTER TABLE sf_tls.items ADD COLUMN refreshed VARCHAR(32); UPDATE sf_tls.items SET refreshed='mysql-reloaded'");
    for database in [&postgres, &mysql] {
        assert_serves(&fixture, database, None, &["same"]);
        lineage::assert_portable_policy(&fixture, database);
        multiple_lineage::assert_responses(&fixture, database);
        let expected = if std::ptr::eq(database, &postgres) {
            "postgres-reloaded"
        } else {
            "mysql-reloaded"
        };
        reload::assert_reloads(&fixture, database, None, &[expected]);
        cancellation::assert_native_stop(&fixture, database, std::ptr::eq(database, &postgres));
        let (mut wrong_ca, address) = command(&fixture, database, None);
        wrong_ca.env(
            "SF_TLS_ROOTS",
            if std::ptr::eq(database, &postgres) {
                &mysql.roots
            } else {
                &postgres.roots
            },
        );
        assert_rejects(wrong_ca, address, &fixture);
        let (mut wrong_name, address) = command(&fixture, database, None);
        wrong_name.env(
            "SF_TLS_SOURCE",
            database.source.replace("127.0.0.1", "localhost"),
        );
        assert_rejects(wrong_name, address, &fixture);
    }
    assert_serves(&fixture, &postgres, Some(&mysql), &["same", "same"]);
    reload::assert_reloads(
        &fixture,
        &postgres,
        Some(&mysql),
        &["postgres-reloaded", "mysql-reloaded"],
    );
    postgres.sql("UPDATE public.items SET value='postgres-only'");
    mysql.sql("UPDATE sf_tls.items SET value='mysql-only'");
    assert_serves(
        &fixture,
        &postgres,
        Some(&mysql),
        &["postgres-only", "mysql-only"],
    );
    let (mut wrong_second_ca, address) = command(&fixture, &postgres, Some(&mysql));
    wrong_second_ca.env("SF_TLS_ROOTS_2", &postgres.roots);
    assert_rejects(wrong_second_ca, address, &fixture);
    join::assert_joins(&fixture, &postgres, &mysql);
    stop_matrix::assert_federated_stop(&fixture, &postgres, &mysql);
    eprintln!(
        "Live TLS providers: PostgreSQL {}; MySQL {}",
        postgres.sql("SHOW server_version"),
        mysql.sql("SELECT VERSION()")
    );
    mysql.stop();
    postgres.stop();
}
