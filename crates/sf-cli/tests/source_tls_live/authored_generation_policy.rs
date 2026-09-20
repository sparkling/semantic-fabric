//! Portable subjects on protected PostgreSQL generations through the shipped CLI.
use super::*;

const B: &str = "fixture-only-authored-policy-b-0123456789";
const DENIED: &str = "fixture-only-authored-policy-denied-0123456789";

fn policy_command(fixture: &Fixture, database: &Database) -> (Command, SocketAddr) {
    let (mut command, address) = command_with_admission(
        fixture,
        database,
        None,
        &["--auth-subjects-env", "SF_SUBJECTS"],
    );
    let subjects: Vec<_> = [("a", "SF_A", "items"), ("b", "SF_B", "items"), ("denied", "SF_DENIED", "other")]
        .into_iter().map(|(subject, credential, table)| serde_json::json!({
            "subjectRef":subject, "credentialEnv":credential,
            "portableRows":[{"sourceIndex":0,"table":table,"column":"tenant","valueEnv":format!("{credential}_VALUE")}]
        })).collect();
    command
        .args([
            "--require-verified-generation",
            "--reload-interval-secs",
            "1",
            "--pg-pool-size",
            "1",
            "--shutdown-timeout-secs",
            "1",
        ])
        .env(
            "SF_SUBJECTS",
            serde_json::json!({"schemaVersion":2,"subjects":subjects}).to_string(),
        )
        .env("SF_A", &fixture.token)
        .env("SF_B", B)
        .env("SF_DENIED", DENIED)
        .env("SF_A_VALUE", "a")
        .env("SF_B_VALUE", "b")
        .env("SF_DENIED_VALUE", "denied");
    (command, address)
}

fn bag(address: SocketAddr, token: &str, expected: &[&str]) {
    let (status, body) = request(address, SINGLE, Some(token)).unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let mut actual: Vec<_> = json["results"]["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            assert_eq!(
                row["s"],
                serde_json::json!({"type":"uri","value":"http://example.test/item"})
            );
            assert_eq!(row["value"]["type"], "literal");
            row["value"]["value"].as_str().unwrap()
        })
        .collect();
    actual.sort();
    assert_eq!(actual, expected);
}

fn pinned_policy_stream(
    fixture: &Fixture,
    database: &Database,
    server: &mut Server,
    address: SocketAddr,
    mapping: &str,
) {
    database.sql("TRUNCATE public.items; INSERT INTO public.items(value,tenant) SELECT repeat('x',32760)||lpad(g::text,8,'0'),'a' FROM generate_series(1,1024) g; INSERT INTO public.items(value,tenant) VALUES('hidden-b','b')");
    let (stream, mut wire) = checks::held_request(address, &fixture.token);
    let pid = checks::generation_session(database);
    assert_eq!(
        database.sql(&format!("SELECT ssl FROM pg_stat_ssl WHERE pid={pid}")),
        "t"
    );
    fixture.write("first.ttl", "invalid authored replacement");
    await_ready(server, address, 503);
    assert_eq!(request(address, SINGLE, Some(B)).unwrap().0, 503);
    fixture.write(
        "first.ttl",
        &mapping.replace("rr:column \"value\"", "rr:constant \"replacement\""),
    );
    await_ready(server, address, 200);
    bag(address, B, &["replacement"]);
    stream.take(40_000_001).read_to_end(&mut wire).unwrap();
    assert!(wire.len() <= 40_000_000);
    let (status, body) = decode_response(wire);
    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let rows = json["results"]["bindings"].as_array().unwrap();
    assert_eq!(rows.len(), 1024);
    let prefix = "x".repeat(32760);
    let ids: std::collections::BTreeSet<_> = rows
        .iter()
        .map(|row| {
            let value = row["value"]["value"].as_str().unwrap();
            assert_eq!(value.len(), 32768);
            value
                .strip_prefix(&prefix)
                .unwrap()
                .parse::<usize>()
                .unwrap()
        })
        .collect();
    assert_eq!(ids, (1..=1024).collect());
    checks::await_stopped(database, pid);
    fixture.write("first.ttl", "invalid authored replacement");
    await_ready(server, address, 503);
    fixture.write("first.ttl", mapping);
    await_ready(server, address, 200);
    // Disconnect also releases the policy-bearing native generation owner.
    let (stream, _) = checks::held_request(address, &fixture.token);
    let pid = checks::generation_session(database);
    stream.shutdown(std::net::Shutdown::Both).unwrap();
    drop(stream);
    checks::await_stopped(database, pid);
    bag(address, B, &["hidden-b"]);
    database.sql(
        "TRUNCATE public.items; INSERT INTO public.items(value,tenant) VALUES('a','a'),('b','b')",
    );
    bag(address, &fixture.token, &["a"]);
    bag(address, B, &["b"]);
}

#[test]
#[ignore = "requires Docker and pinned owned PostgreSQL images; required in CI"]
pub(super) fn public_authored_postgres_portable_policies() {
    for patch in ["16.9", "16.15"] {
        eprintln!("protected portable policies PostgreSQL {patch}");
        let fixture = Fixture::new();
        let database = Database::postgres_patch(&fixture, patch);
        assert_eq!(
            database.sql("SHOW server_version_num"),
            if patch == "16.9" { "160009" } else { "160015" }
        );
        database.sql("ALTER TABLE public.items ADD COLUMN tenant TEXT NOT NULL DEFAULT 'a'; TRUNCATE public.items; INSERT INTO public.items(value,tenant) VALUES('a','a'),('a','a'),('b','b'),('b','b'); REVOKE ALL ON DATABASE postgres FROM PUBLIC, sf_tls; GRANT CONNECT ON DATABASE postgres TO sf_tls; REVOKE ALL ON SCHEMA public FROM PUBLIC, sf_tls; GRANT USAGE ON SCHEMA public TO sf_tls; REVOKE ALL ON ALL TABLES IN SCHEMA public FROM PUBLIC, sf_tls; GRANT SELECT ON ALL TABLES IN SCHEMA public TO sf_tls; GRANT EXECUTE ON FUNCTION pg_catalog.pg_database_collation_actual_version(oid) TO sf_tls;");
        support::mapping(&fixture.root.join("first.ttl"), "http://example.test/left");
        fixture.write(
            "ontology.ttl",
            "<http://example.test/left> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .",
        );
        let mapping = std::fs::read_to_string(fixture.root.join("first.ttl")).unwrap();
        let (command, address) = policy_command(&fixture, &database);
        let mut server = start(&fixture, command, address);
        for _ in 0..2 {
            // RDF equality collapses duplicate triples with this constant subject.
            bag(address, &fixture.token, &["a"]);
            bag(address, B, &["b"]);
        }
        database.assert_encrypted_sessions();
        let ordered = format!("{SINGLE} ORDER BY ?value");
        for query in [SINGLE, ordered.as_str()] {
            assert_eq!(request(address, query, Some(DENIED)).unwrap().0, 403);
        }
        assert_eq!(request(address, &ordered, Some(B)).unwrap().0, 501);
        stop(&mut server);

        // Delay the observer so its global fence cannot mask request authorization.
        let (mut command, address) = policy_command(&fixture, &database);
        command.args(["--reload-interval-secs", "30"]);
        let mut server = start(&fixture, command, address);
        let mut held = database.hold_table("items");
        let before = Instant::now();
        assert_eq!(request(address, SINGLE, Some(DENIED)).unwrap().0, 403);
        assert_eq!(request(address, SINGLE, Some(B)).unwrap().0, 503);
        assert!(before.elapsed() < Duration::from_secs(2));
        held.assert_held();
        drop(held);
        bag(address, B, &["b"]);
        stop(&mut server);

        let (command, address) = policy_command(&fixture, &database);
        let mut server = start(&fixture, command, address);
        database.sql("ALTER TABLE public.items RENAME COLUMN tenant TO tenant_drift");
        await_ready(&mut server, address, 503);
        let until = Instant::now() + Duration::from_millis(2500);
        while Instant::now() < until {
            assert_eq!(ready(address), 503);
            assert_eq!(request(address, SINGLE, Some(B)).unwrap().0, 503);
            thread::sleep(Duration::from_millis(100));
        }
        database.sql("ALTER TABLE public.items RENAME COLUMN tenant_drift TO tenant");
        await_ready(&mut server, address, 200);
        bag(address, &fixture.token, &["a"]);
        bag(address, B, &["b"]);
        pinned_policy_stream(&fixture, &database, &mut server, address, &mapping);
        stop(&mut server);
    }
}
