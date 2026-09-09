//! Exact local image smoke. Reuses owned TLS fixtures; no external endpoints.
use super::{decode_response, request, support, Database, Fixture, SINGLE, UNION};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};

fn docker(args: &[&str]) -> Command {
    let mut command = Command::new("docker");
    command
        .env_remove("DOCKER_CONTEXT")
        .env_remove("DOCKER_HOST")
        .args(["--host", "unix:///var/run/docker.sock"])
        .args(args);
    command
}

fn run(command: &mut Command) -> Output {
    support::output(command, Duration::from_secs(30))
}

fn success(command: &mut Command) -> String {
    let result = run(command);
    assert!(
        result.status.success(),
        "image fixture failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap().trim().to_owned()
}

struct ImageServer(String);
impl Drop for ImageServer {
    fn drop(&mut self) {
        let _ = run(&mut docker(&["rm", "--force", &self.0]));
    }
}

fn create(
    image: &str,
    fixture: &Fixture,
    source: &str,
    roots: Option<&str>,
    second: Option<&Database>,
) -> (ImageServer, SocketAddr) {
    let address = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let mut command = docker(&[
        "create",
        "--network=host", // Only loopback test-owned database/listener ports.
        "--read-only",
        "--cap-drop=ALL",
        "--security-opt=no-new-privileges:true",
        "--pids-limit=128",
        "--memory=512m",
        "--tmpfs=/tmp:rw,noexec,nosuid,nodev,size=16777216,mode=1777",
        "--mount",
        &format!(
            "type=bind,src={},dst={},readonly",
            fixture.root.display(),
            fixture.root.display()
        ),
        "--env=SF_TLS_SOURCE",
        "--env=SF_TLS_BEARER",
    ]);
    command
        .env("SF_TLS_SOURCE", source)
        .env("SF_TLS_BEARER", &fixture.token);
    if let Some(roots) = roots {
        command.arg("--env=SF_TLS_ROOTS").env("SF_TLS_ROOTS", roots);
    }
    if let Some(second) = second {
        command
            .args(["--env=SF_TLS_SOURCE_2", "--env=SF_TLS_ROOTS_2"])
            .env("SF_TLS_SOURCE_2", &second.source)
            .env("SF_TLS_ROOTS_2", &second.roots);
    }
    command
        .args([image, "serve", "--source-env", "SF_TLS_SOURCE", "--mapping"])
        .arg(fixture.root.join("first.ttl"))
        .arg("--ontology")
        .arg(fixture.root.join("ontology.ttl"))
        .args([
            "--auth-token-env",
            "SF_TLS_BEARER",
            "--bind",
            &address.to_string(),
            "--shutdown-timeout-secs",
            "2",
            "--metrics",
        ]);
    if roots.is_some() {
        command.args(["--source-tls-roots-env", "SF_TLS_ROOTS"]);
    }
    if second.is_some() {
        command
            .args([
                "--source-env-2",
                "SF_TLS_SOURCE_2",
                "--source-tls-roots-env-2",
                "SF_TLS_ROOTS_2",
                "--mapping-2",
            ])
            .arg(fixture.root.join("second.ttl"));
    }
    let id = success(&mut command);
    assert_eq!(id.len(), 64);
    assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
    let server = ImageServer(id);
    let config: serde_json::Value =
        serde_json::from_str(&success(&mut docker(&["inspect", &server.0]))).unwrap();
    assert_eq!(config[0]["Config"]["User"], "65532:65532");
    assert_eq!(config[0]["HostConfig"]["ReadonlyRootfs"], true);
    assert_eq!(
        config[0]["HostConfig"]["CapDrop"],
        serde_json::json!(["ALL"])
    );
    assert_eq!(config[0]["HostConfig"]["Privileged"], false);
    assert_eq!(config[0]["Image"], image);
    success(&mut docker(&["start", &server.0]));
    (server, address)
}

fn get(address: SocketAddr, path: &str) -> (u16, Vec<u8>) {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(1)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut wire = Vec::new();
    stream.take(65537).read_to_end(&mut wire).unwrap();
    assert!(wire.len() <= 65536);
    decode_response(wire)
}

fn assert_serves(
    server: &ImageServer,
    address: SocketAddr,
    fixture: &Fixture,
    federated: bool,
    verify_transport: impl FnOnce(),
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some((status, _)) = request(address, SINGLE, None) {
            assert_eq!(status, 401);
            break;
        }
        assert!(Instant::now() < deadline, "image did not become ready");
        let running = success(&mut docker(&[
            "inspect",
            "--format",
            "{{.State.Running}}",
            &server.0,
        ]));
        if running != "true" {
            let logs = run(&mut docker(&["logs", &server.0]));
            panic!(
                "image exited during startup: {} {}",
                String::from_utf8_lossy(&logs.stdout),
                String::from_utf8_lossy(&logs.stderr)
            );
        }
        thread::sleep(Duration::from_millis(30));
    }
    let query = if federated { UNION } else { SINGLE };
    assert_eq!(request(address, query, Some("wrong-token")).unwrap().0, 401);
    let (status, body) = request(address, query, Some(&fixture.token)).unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let row = serde_json::json!({
        "s":{"type":"uri", "value":"http://example.test/item"},
        "value":{"type":"literal", "value":"same"}
    });
    let expected = vec![row; if federated { 2 } else { 1 }];
    assert_eq!(json["head"]["vars"], serde_json::json!(["s", "value"]));
    assert_eq!(json["results"]["bindings"], serde_json::json!(expected));
    for (path, state) in [("/livez", "live"), ("/readyz", "ready")] {
        let (status, body) = get(address, path);
        assert_eq!(status, 200);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({"status":state})
        );
    }
    assert_eq!(get(address, "/metrics").0, 200);
    let nested = format!("SELECT ({}1 AS ?x) WHERE {{}}", "(".repeat(4096));
    let start = Instant::now();
    let (status, _) = request(address, &nested, Some(&fixture.token)).unwrap();
    assert!(matches!(status, 400 | 429 | 500 | 504));
    assert!(start.elapsed() < Duration::from_secs(4));
    let (status, recovered) = request(address, query, Some(&fixture.token)).unwrap();
    assert_eq!(status, 200);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&recovered).unwrap(),
        json
    );
    verify_transport();
    // SIGTERM reaches the exec-form product PID 1. Docker's force-kill deadline
    // exceeds the configured two-second drain plus fixed cleanup slack.
    success(&mut docker(&["stop", "--time", "8", &server.0]));
    assert_eq!(
        success(&mut docker(&[
            "inspect",
            "--format",
            "{{.State.ExitCode}}",
            &server.0
        ])),
        "0"
    );
}

#[test]
#[ignore = "requires SF_SERVING_IMAGE_ID and owned Docker TLS fixtures"]
fn minimal_image_serves_all_backends_read_only_and_non_root() {
    let image = std::env::var("SF_SERVING_IMAGE_ID")
        .expect("explicit immutable locally built SF_SERVING_IMAGE_ID is required");
    assert!(image.starts_with("sha256:") && image.len() == 71);
    assert!(image[7..].bytes().all(|b| b.is_ascii_hexdigit()));
    let help = success(&mut docker(&[
        "run",
        "--rm",
        "--network=none",
        "--read-only",
        &image,
        "--help",
    ]));
    assert!(help.contains("serve"));
    for forbidden in ["conformance", "bench"] {
        assert!(!help.contains(forbidden));
        assert_eq!(
            run(&mut docker(&[
                "run",
                "--rm",
                "--network=none",
                "--read-only",
                &image,
                forbidden
            ]))
            .status
            .code(),
            Some(2)
        );
    }
    assert_eq!(
        success(&mut docker(&[
            "run",
            "--rm",
            "--network=none",
            "--read-only",
            &image,
            "--version"
        ])),
        format!("semantic-fabric {}", env!("CARGO_PKG_VERSION"))
    );
    let linkage = success(&mut docker(&["run", "--rm", "--network=none", "--read-only", "--entrypoint=/bin/sh", &image, "-c",
        "test \"$(id -u)\" = 65532 && for tool in cargo rustc node npm npx; do if command -v \"$tool\"; then exit 1; fi; done; ldd /usr/local/bin/semantic-fabric"]));
    assert!(!linkage.contains("not found"));
    let fixture = Fixture::new();
    support::mapping(&fixture.root.join("first.ttl"), "http://example.test/left");
    support::mapping(
        &fixture.root.join("second.ttl"),
        "http://example.test/right",
    );
    fixture.write("ontology.ttl", "@prefix owl: <http://www.w3.org/2002/07/owl#> .\n<http://example.test/left> a owl:DatatypeProperty .\n<http://example.test/right> a owl:DatatypeProperty .\n");
    let path = fixture.root.join("items.db");
    let sqlite = rusqlite::Connection::open(&path).unwrap();
    sqlite
        .execute_batch(
            "CREATE TABLE items(value TEXT NOT NULL); INSERT INTO items VALUES ('same');",
        )
        .unwrap();
    drop(sqlite);
    // These are disposable public fixture inputs, not credentials. Non-root
    // image readability must not depend on the developer's ambient umask.
    for name in ["first.ttl", "second.ttl", "items.db"] {
        std::fs::set_permissions(
            fixture.root.join(name),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
    }
    let source = format!("sqlite:{}", path.display());
    let (server, address) = create(&image, &fixture, &source, None, None);
    assert_serves(&server, address, &fixture, false, || {});
    drop(server);
    let postgres = Database::start(&fixture, true);
    let mysql = Database::start(&fixture, false);
    for database in [&postgres, &mysql] {
        let (server, address) = create(
            &image,
            &fixture,
            &database.source,
            Some(&database.roots),
            None,
        );
        assert_serves(&server, address, &fixture, false, || {
            database.assert_encrypted_sessions()
        });
        let wrong_roots = if std::ptr::eq(database, &postgres) {
            &mysql.roots
        } else {
            &postgres.roots
        };
        let (rejected, address) =
            create(&image, &fixture, &database.source, Some(wrong_roots), None);
        assert_eq!(success(&mut docker(&["wait", &rejected.0])), "1");
        assert!(
            request(address, SINGLE, Some(&fixture.token)).is_none(),
            "wrong CA must not expose a listener"
        );
    }
    let (server, address) = create(
        &image,
        &fixture,
        &postgres.source,
        Some(&postgres.roots),
        Some(&mysql),
    );
    assert_serves(&server, address, &fixture, true, || {
        postgres.assert_encrypted_sessions();
        mysql.assert_encrypted_sessions();
    });
    eprintln!("Exact image {image}: non-root/read-only SQLite, PostgreSQL {}, MySQL {}, and TLS UNION smoke passed", postgres.sql("SHOW server_version"), mysql.sql("SELECT VERSION()"));
}
