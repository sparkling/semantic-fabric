//! Public Direct Mapping uses only a newly created, owned TLS provider.
use super::*;

const BASE: &str = "http://example.test/direct/";
const SELECT: &str =
    "SELECT ?s ?value WHERE { ?s <http://example.test/direct/items#value> ?value }";

fn prepare(fixture: &Fixture, database: &Database) {
    fixture.write("ontology.ttl", "@prefix owl: <http://www.w3.org/2002/07/owl#> .\n<http://example.test/direct/items> a owl:Class .\n<http://example.test/direct/items#id> a owl:DatatypeProperty .\n<http://example.test/direct/items#value> a owl:DatatypeProperty .\n<http://example.test/direct/items#next> a owl:DatatypeProperty .\n");
    database.sql("ALTER TABLE public.items ADD COLUMN id INTEGER NOT NULL DEFAULT 1 PRIMARY KEY; REVOKE ALL ON DATABASE postgres FROM PUBLIC, sf_tls; GRANT CONNECT ON DATABASE postgres TO sf_tls; REVOKE ALL ON SCHEMA public FROM PUBLIC, sf_tls; GRANT USAGE ON SCHEMA public TO sf_tls; REVOKE ALL ON ALL TABLES IN SCHEMA public FROM PUBLIC, sf_tls; GRANT SELECT ON ALL TABLES IN SCHEMA public TO sf_tls; GRANT EXECUTE ON FUNCTION pg_catalog.pg_database_collation_actual_version(oid) TO sf_tls;");
}

fn direct_command(fixture: &Fixture, database: &Database) -> (Command, SocketAddr) {
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
            "--direct-mapping-base",
            BASE,
            "--ontology",
        ])
        .arg(fixture.root.join("ontology.ttl"))
        .args([
            "--auth-token-env",
            "SF_TLS_BEARER",
            "--bind",
            &address.to_string(),
            "--pg-pool-size",
            "1",
            "--max-concurrent-requests",
            "1",
            "--shutdown-timeout-secs",
            "1",
        ])
        .env("SF_TLS_SOURCE", &database.source)
        .env("SF_TLS_ROOTS", &database.roots)
        .env("SF_TLS_BEARER", &fixture.token);
    (command, address)
}

fn start(fixture: &Fixture, mut command: Command, address: SocketAddr) -> Server {
    let log = fixture.root.join("direct.stderr");
    let mut server = Server(
        command
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .unwrap(),
    );
    let until = Instant::now() + Duration::from_secs(40);
    loop {
        if let Some((status, _)) = request(address, SELECT, None) {
            assert_eq!(status, 401);
            return server;
        }
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "Direct startup failed: {}",
            std::fs::read_to_string(&log).unwrap()
        );
        assert!(Instant::now() < until, "Direct startup deadline");
        thread::sleep(Duration::from_millis(25));
    }
}

fn assert_select(address: SocketAddr, token: &str) {
    let (status, body) = request(address, SELECT, Some(token)).unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let document: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(document["head"]["vars"], serde_json::json!(["s", "value"]));
    assert_eq!(
        document["results"]["bindings"],
        serde_json::json!([{
            "s":{"type":"uri", "value":"http://example.test/direct/items/id=1"},
            "value":{"type":"literal", "value":"same"}
        }])
    );
}

#[test]
#[ignore = "requires Docker and pinned owned PostgreSQL images; required in CI"]
fn public_direct_mapping_has_authenticated_tls_startup() {
    for patch in ["16.9", "16.15"] {
        let fixture = Fixture::new();
        let database = Database::postgres_patch(&fixture, patch);
        assert_eq!(
            database.sql("SHOW server_version_num"),
            if patch == "16.9" { "160009" } else { "160015" }
        );
        prepare(&fixture, &database);
        let ontology = std::fs::read_to_string(fixture.root.join("ontology.ttl")).unwrap();
        let (command, address) = direct_command(&fixture, &database);
        let mut server = start(&fixture, command, address);
        assert_select(address, &fixture.token);
        database.assert_encrypted_sessions();
        assert_eq!(database.sql("SELECT count(*) FROM pg_stat_ssl s JOIN pg_stat_activity a ON a.pid=s.pid WHERE a.usename='sf_tls' AND s.ssl"), "2", "independent request and control TLS sessions");
        assert_eq!(
            request(address, SELECT, Some("wrong-token")).unwrap().0,
            401
        );
        let (status, body) = request(address, "ASK { <http://example.test/direct/items/id=1> <http://example.test/direct/items#value> \"same\" }", Some(&fixture.token)).unwrap();
        assert_eq!(status, 200);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["boolean"],
            true
        );
        let (status, body) = request_format(address, "CONSTRUCT { ?s <http://example.test/direct/items#value> ?value } WHERE { ?s <http://example.test/direct/items#value> ?value }", Some(&fixture.token), "application/n-triples").unwrap();
        assert_eq!(status, 200);
        assert_eq!(std::str::from_utf8(&body).unwrap(), "<http://example.test/direct/items/id=1> <http://example.test/direct/items#value> \"same\" .\n");
        let (status, body) =
            request_format(address, SELECT, Some(&fixture.token), lineage::FORMAT).unwrap();
        assert_eq!(status, 200);
        let records: Vec<serde_json::Value> = std::str::from_utf8(&body)
            .unwrap()
            .split('\u{1e}')
            .skip(1)
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0]["sourceId"], 0);
        assert_eq!(records[2]["type"], "complete");
        assert_eq!(records[2]["solutions"], 1);
        // Direct keeps its startup ontology, not an authored-file reload worker.
        fixture.write("ontology.ttl", "invalid replacement ontology");
        database.sql("ALTER TABLE public.items ADD COLUMN next TEXT DEFAULT 'successor'");
        await_ready(&mut server, address, 503);
        await_ready(&mut server, address, 200);
        let (status, body) = request(
            address,
            "SELECT ?next WHERE { ?s <http://example.test/direct/items#next> ?next }",
            Some(&fixture.token),
        )
        .unwrap();
        assert_eq!(status, 200);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["results"]["bindings"],
            serde_json::json!([{"next":{"type":"literal","value":"successor"}}])
        );
        assert_select(address, &fixture.token);
        // Generation leases use NOWAIT: an incompatible DDL lock must reject
        // immediately, without waiting for or disturbing the external owner.
        let mut held = database.hold_table("items");
        let before = Instant::now();
        let (status, _) = request(address, SELECT, Some(&fixture.token)).unwrap();
        assert_eq!(status, 503);
        assert!(before.elapsed() < Duration::from_secs(2));
        held.assert_held();
        drop(held);
        await_ready(&mut server, address, 200);
        assert_select(address, &fixture.token);

        // Without query traffic, the independent observer also detects the
        // incompatible lock and fences readiness. Shutdown stays bounded.
        let mut held = database.hold_table("items");
        await_ready(&mut server, address, 503);
        stop(&mut server);
        held.assert_held();
        drop(held);

        fixture.write("ontology.ttl", &ontology);
        let untrusted = fixture.certificates("untrusted");
        let (mut command, address) = direct_command(&fixture, &database);
        command.env("SF_TLS_ROOTS", untrusted);
        assert_rejected(command, address);
        database.sql("ALTER TABLE public.items DROP CONSTRAINT items_pkey");
        let (command, address) = direct_command(&fixture, &database);
        assert_rejected(command, address);
    }
}

fn assert_rejected(mut command: Command, address: SocketAddr) {
    let mut server = Server(
        command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let until = Instant::now() + Duration::from_secs(35);
    loop {
        assert!(
            TcpStream::connect(address).is_err(),
            "invalid Direct profile bound a listener"
        );
        if let Some(status) = server.0.try_wait().unwrap() {
            assert!(!status.success());
            return;
        }
        assert!(
            Instant::now() < until,
            "invalid Direct profile did not fail closed"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn await_ready(server: &mut Server, address: SocketAddr, status: u16) {
    let until = Instant::now() + Duration::from_secs(35);
    loop {
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        write!(
            stream,
            "GET /readyz HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut wire = Vec::new();
        stream.take(4096).read_to_end(&mut wire).unwrap();
        if decode_response(wire).0 == status {
            return;
        }
        assert!(server.0.try_wait().unwrap().is_none());
        assert!(
            Instant::now() < until,
            "traffic-independent readiness transition to {status}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn stop(server: &mut Server) {
    let started = Instant::now();
    assert_eq!(
        unsafe { libc::kill(server.0.id() as i32, libc::SIGTERM) },
        0
    );
    loop {
        if let Some(status) = server.0.try_wait().unwrap() {
            assert!(status.success(), "Direct shutdown failed");
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "Direct shutdown exceeded shared drain/cleanup bound"
        );
        thread::sleep(Duration::from_millis(10));
    }
}
