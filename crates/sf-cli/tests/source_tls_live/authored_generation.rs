//! Protected authored generations exercised through the actual serving binary.
use super::*;
#[path = "authored_generation_checks.rs"]
mod checks;
#[path = "authored_generation_policy.rs"]
mod portable_policy;

fn profile(fixture: &Fixture, database: &Database) -> (Command, SocketAddr) {
    let (mut command, address) = command(fixture, database, None);
    command.args([
        "--require-verified-generation",
        "--reload-interval-secs",
        "1",
        "--pg-pool-size",
        "1",
        "--shutdown-timeout-secs",
        "1",
    ]);
    (command, address)
}

fn start(fixture: &Fixture, mut command: Command, address: SocketAddr) -> Server {
    let log = fixture.root.join("authored.stderr");
    let mut server = Server(
        command
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .unwrap(),
    );
    let until = Instant::now() + Duration::from_secs(40);
    loop {
        if let Some((status, _)) = request(address, SINGLE, None) {
            assert_eq!(status, 401);
            return server;
        }
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "authored startup failed: {}",
            std::fs::read_to_string(&log).unwrap()
        );
        assert!(Instant::now() < until, "authored startup deadline");
        thread::sleep(Duration::from_millis(25));
    }
}

fn value(address: SocketAddr, token: &str, expected: &str) {
    let (status, body) = request(address, SINGLE, Some(token)).unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let document: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(document["head"]["vars"], serde_json::json!(["s", "value"]));
    assert_eq!(
        document["results"]["bindings"],
        serde_json::json!([{
            "s":{"type":"uri", "value":"http://example.test/item"},
            "value":{"type":"literal", "value":expected}
        }])
    );
}

fn ready(address: SocketAddr) -> u16 {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(
        stream,
        "GET /readyz HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut wire = Vec::new();
    stream.take(4096).read_to_end(&mut wire).unwrap();
    decode_response(wire).0
}

fn await_ready(server: &mut Server, address: SocketAddr, status: u16) {
    let until = Instant::now() + Duration::from_secs(35);
    while ready(address) != status {
        assert!(server.0.try_wait().unwrap().is_none());
        assert!(
            Instant::now() < until,
            "authored readiness transition to {status}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn stop(server: &mut Server) {
    let before = Instant::now();
    assert_eq!(
        unsafe { libc::kill(server.0.id() as i32, libc::SIGTERM) },
        0
    );
    loop {
        if let Some(status) = server.0.try_wait().unwrap() {
            assert!(status.success(), "authored shutdown failed");
            return;
        }
        assert!(
            before.elapsed() < Duration::from_secs(6),
            "bounded authored shutdown"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "requires Docker and pinned owned PostgreSQL images; required in CI"]
fn public_authored_postgres_generation_is_protected() {
    for patch in ["16.9", "16.15"] {
        eprintln!("protected authored PostgreSQL {patch}");
        let fixture = Fixture::new();
        let database = Database::postgres_patch(&fixture, patch);
        assert_eq!(
            database.sql("SHOW server_version_num"),
            if patch == "16.9" { "160009" } else { "160015" }
        );
        // Authored mappings do not acquire Direct Mapping's primary-key rule.
        assert_eq!(database.sql("SELECT count(*) FROM pg_constraint WHERE conrelid='public.items'::regclass AND contype='p'"), "0");
        database.sql("REVOKE ALL ON DATABASE postgres FROM PUBLIC, sf_tls; GRANT CONNECT ON DATABASE postgres TO sf_tls; REVOKE ALL ON SCHEMA public FROM PUBLIC, sf_tls; GRANT USAGE ON SCHEMA public TO sf_tls; REVOKE ALL ON ALL TABLES IN SCHEMA public FROM PUBLIC, sf_tls; GRANT SELECT ON ALL TABLES IN SCHEMA public TO sf_tls; GRANT EXECUTE ON FUNCTION pg_catalog.pg_database_collation_actual_version(oid) TO sf_tls;");
        support::mapping(&fixture.root.join("first.ttl"), "http://example.test/left");
        fixture.write(
            "ontology.ttl",
            "<http://example.test/left> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .",
        );
        let mapping = std::fs::read_to_string(fixture.root.join("first.ttl")).unwrap();
        let (command, address) = profile(&fixture, &database);
        let mut server = start(&fixture, command, address);
        value(address, &fixture.token, "same");
        database.assert_encrypted_sessions();
        assert_eq!(
            request(address, SINGLE, Some("wrong-token")).unwrap().0,
            401
        );
        let (status, body) = request(
            address,
            "ASK { <http://example.test/item> <http://example.test/left> \"same\" }",
            Some(&fixture.token),
        )
        .unwrap();
        assert_eq!(status, 200);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["boolean"],
            true
        );
        let (status, body) = request_format(address, "CONSTRUCT { ?s <http://example.test/left> ?value } WHERE { ?s <http://example.test/left> ?value }", Some(&fixture.token), "application/n-triples").unwrap();
        assert_eq!(status, 200);
        assert_eq!(
            std::str::from_utf8(&body).unwrap(),
            "<http://example.test/item> <http://example.test/left> \"same\" .\n"
        );

        // NOWAIT is required before public execution. The test-owned DDL lock
        // remains held and the response contains no source connection details.
        let mut held = database.hold_table("items");
        let before = Instant::now();
        let (status, body) = request(address, SINGLE, Some(&fixture.token)).unwrap();
        assert_eq!(status, 503);
        assert!(before.elapsed() < Duration::from_secs(2));
        let body = std::str::from_utf8(&body).unwrap();
        assert!(!body.contains(&database.source) && !body.contains(&fixture.token));
        held.assert_held();
        await_ready(&mut server, address, 503);
        drop(held);
        await_ready(&mut server, address, 200);
        value(address, &fixture.token, "same");

        // An incompatible authored successor must fence the public path, never
        // silently use the old source. Correct files plus schema recover it.
        fixture.write(
            "first.ttl",
            &mapping.replace("rr:column \"value\"", "rr:column \"successor\""),
        );
        await_ready(&mut server, address, 503);
        assert_eq!(
            request(address, SINGLE, Some(&fixture.token)).unwrap().0,
            503
        );
        database.sql("ALTER TABLE public.items ADD COLUMN successor TEXT NOT NULL DEFAULT 'new'");
        await_ready(&mut server, address, 200);
        value(address, &fixture.token, "new");
        fixture.write("first.ttl", &mapping);
        checks::await_value(&mut server, address, &fixture.token, "same");

        checks::pinned_and_cancelled(&fixture, &database, &mut server, address, &mapping);
        // The qualified identity deliberately covers the whole public schema,
        // not just mapped tables. Unrelated unsupported DDL therefore fences it.
        database.sql(
            "CREATE TABLE public.healthy (rowid INTEGER); GRANT SELECT ON public.healthy TO sf_tls",
        );
        await_ready(&mut server, address, 503);
        assert_eq!(
            request(address, SINGLE, Some(&fixture.token)).unwrap().0,
            503
        );
        database.sql("DROP TABLE public.healthy");
        await_ready(&mut server, address, 200);
        value(address, &fixture.token, "same");
        // Only the serialized observer owns readiness; no query is needed to
        // detect an unavailable generation, and shutdown does not kill its owner.
        let mut held = database.hold_table("items");
        await_ready(&mut server, address, 503);
        stop(&mut server);
        held.assert_held();
        drop(held);
        checks::deadline_and_shutdown(&fixture, &database);
        checks::rejected_profiles(&fixture, &database, &mapping);
    }
    // Keep the existing exact CI entrypoint inclusive of portable qualification.
    portable_policy::public_authored_postgres_portable_policies();
}
