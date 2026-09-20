//! Actual CLI composition of the five qualified native generation profiles.
use super::*;
#[path = "protected_federation_lifecycle.rs"]
mod lifecycle;
#[path = "protected_federation_sources.rs"]
mod sources;
use sources::Endpoint;

const JOIN: &str = "SELECT ?left ?right ?value WHERE { ?left <http://example.test/left> ?value . ?right <http://example.test/right> ?value }";
const REVERSE: &str = "SELECT ?left ?right ?value WHERE { ?right <http://example.test/right> ?value . ?left <http://example.test/left> ?value }";

fn profile(
    fixture: &Fixture,
    pair: [&Endpoint; 2],
    policy: bool,
    limit: &str,
) -> (Command, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_semantic-fabric"));
    command.env_clear().arg("serve");
    for (slot, source) in pair.into_iter().enumerate() {
        source.apply(&mut command, slot);
    }
    command
        .arg("--mapping")
        .arg(fixture.root.join("first.ttl"))
        .arg("--mapping-2")
        .arg(fixture.root.join("second.ttl"))
        .arg("--ontology")
        .arg(fixture.root.join("ontology.ttl"))
        .args([
            "--require-verified-generation",
            "--reload-interval-secs",
            "1",
            "--pg-pool-size",
            "1",
            "--sqlite-pool-size",
            "1",
            "--shutdown-timeout-secs",
            "1",
            "--max-result-items",
            limit,
            "--bind",
            &address.to_string(),
        ]);
    if policy {
        let subjects: Vec<_> = [("a","SF_A","items"),("b","SF_B","items"),("denied","SF_DENIED","uncovered")].into_iter().map(|(name,key,table)| serde_json::json!({"subjectRef":name,"credentialEnv":key,"portableRows":[{"sourceIndex":0,"table":table,"column":"tenant","valueEnv":format!("{key}_VALUE")},{"sourceIndex":1,"table":table,"column":"tenant","valueEnv":format!("{key}_VALUE")}]})).collect();
        command
            .args(["--auth-subjects-env", "SF_SUBJECTS"])
            .env(
                "SF_SUBJECTS",
                serde_json::json!({"schemaVersion":2,"subjects":subjects}).to_string(),
            )
            .env("SF_A", &fixture.token)
            .env("SF_B", "fixture-federated-b-01234567890123456789")
            .env("SF_DENIED", "fixture-federated-denied-01234567890123456789")
            .env("SF_A_VALUE", "a")
            .env("SF_B_VALUE", "b")
            .env("SF_DENIED_VALUE", "denied");
    } else {
        command
            .args(["--auth-token-env", "SF_A"])
            .env("SF_A", &fixture.token);
    }
    (command, address)
}

fn start(fixture: &Fixture, mut command: Command, address: SocketAddr) -> Server {
    let log = fixture.root.join("server.log");
    let mut server = Server(
        command
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(&log).unwrap())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        if let Some((status, _)) = request(address, UNION, None) {
            assert_eq!(status, 401);
            return server;
        }
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "{}",
            std::fs::read_to_string(&log).unwrap()
        );
        assert!(
            Instant::now() < deadline,
            "protected federation startup deadline"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn stop(mut server: Server) {
    assert_eq!(
        unsafe { libc::kill(server.0.id() as i32, libc::SIGTERM) },
        0
    );
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        if let Some(status) = server.0.try_wait().unwrap() {
            assert!(status.success());
            return;
        }
        assert!(Instant::now() < deadline, "protected federation shutdown");
        thread::sleep(Duration::from_millis(20));
    }
}

fn ordinary(
    address: SocketAddr,
    token: &str,
    query: &str,
    expected: &[&str],
) -> (serde_json::Value, Vec<String>) {
    let (status, body) = request(address, query, Some(token)).unwrap();
    assert_eq!(status, 200, "{query}: {}", String::from_utf8_lossy(&body));
    let document: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let mut values: Vec<_> = document["results"]["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            assert_eq!(row["value"]["type"], "literal");
            for name in if query == UNION {
                &["s"][..]
            } else {
                &["left", "right"][..]
            } {
                assert_eq!(
                    row[name],
                    serde_json::json!({"type":"uri","value":"http://example.test/item"})
                );
            }
            row["value"]["value"].as_str().unwrap()
        })
        .collect();
    values.sort();
    assert_eq!(values, expected);
    stop_matrix::bag(&body)
}

fn qualify(fixture: &Fixture, pair: [&Endpoint; 2], policy: bool) {
    let (command, address) = profile(fixture, pair, policy, "100");
    let server = start(fixture, command, address);
    let callers: Vec<_> = if policy {
        vec![
            (&*fixture.token, vec!["same", "same"], vec!["same"]),
            (
                "fixture-federated-b-01234567890123456789",
                vec!["same-b", "same-b"],
                vec!["same-b"],
            ),
        ]
    } else {
        vec![(
            &*fixture.token,
            vec!["same", "same", "same-b", "same-b"],
            vec!["same", "same-b"],
        )]
    };
    for (token, union_values, join_values) in callers {
        for _ in 0..2 {
            let union = ordinary(address, token, UNION, &union_values);
            let (status, body) =
                request_format(address, UNION, Some(token), lineage::FORMAT).unwrap();
            assert_eq!(status, 200);
            assert_eq!(
                federated_lineage::bag(&body, [union_values.len() / 2; 2]),
                union
            );
            for query in [JOIN, REVERSE] {
                let join = ordinary(address, token, query, &join_values);
                let (status, body) =
                    request_format(address, query, Some(token), lineage::FORMAT).unwrap();
                assert_eq!(status, 200);
                assert_eq!(federated_lineage::join_bag(&body), join);
            }
        }
    }
    if policy {
        assert_eq!(
            request(
                address,
                UNION,
                Some("fixture-federated-denied-01234567890123456789")
            )
            .unwrap()
            .0,
            403
        );
    }
    for endpoint in pair {
        endpoint.encrypted();
    }
    // MySQL's case-insensitive SQL equality may retain this candidate;
    // final RDF-term matching must reject it in either driving order.
    pair[1].sql("INSERT INTO items(value,tenant) VALUES('Same','a')");
    let expected: &[&str] = if policy {
        &["same"]
    } else {
        &["same", "same-b"]
    };
    ordinary(address, &fixture.token, JOIN, expected);
    ordinary(address, &fixture.token, REVERSE, expected);
    pair[1].reset();
    stop(server);
    // Join intermediates are source work; a one-result caller must not pay for
    // discarded/build-side rows as final results on any native lease variant.
    if policy {
        let (command, address) = profile(fixture, pair, true, "1");
        let server = start(fixture, command, address);
        ordinary(address, &fixture.token, JOIN, &["same"]);
        ordinary(address, &fixture.token, REVERSE, &["same"]);
        stop(server);
    }
}

fn fixture() -> Fixture {
    let fixture = Fixture::new();
    support::mapping(&fixture.root.join("first.ttl"), "http://example.test/left");
    support::mapping(
        &fixture.root.join("second.ttl"),
        "http://example.test/right",
    );
    // Bounded federation joins require an explicitly typed reversible key.
    for file in ["first.ttl", "second.ttl"] {
        let path = fixture.root.join(file);
        let mapping = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            path,
            mapping.replace(
                "rr:column \"value\"",
                "rr:column \"value\" ; rr:datatype <http://www.w3.org/2001/XMLSchema#string>",
            ),
        )
        .unwrap();
    }
    fixture.write("ontology.ttl","<http://example.test/left> a <http://www.w3.org/2002/07/owl#DatatypeProperty> . <http://example.test/right> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .");
    fixture
}

#[test]
#[ignore = "requires Docker and pinned owned PostgreSQL/MySQL images; required in CI"]
fn protected_ordered_profile_pairs_are_exact() {
    let profiles = ["WAL", "DELETE", "16.9", "16.15", "mysql"];
    let first: Vec<_> = profiles.into_iter().map(Endpoint::new).collect();
    let second: Vec<_> = profiles.into_iter().map(Endpoint::new).collect();
    let fixture = fixture();
    for left in &first {
        for right in &second {
            eprintln!("protected federation {} -> {}", left.profile, right.profile);
            qualify(&fixture, [left, right], false);
            qualify(&fixture, [left, right], true);
        }
    }
    for (left, right) in [(0, 3), (3, 0), (3, 4), (4, 3), (4, 0), (0, 4)] {
        lifecycle::qualify(&fixture, [&first[left], &second[right]]);
    }
}

#[test]
#[ignore = "focused lifecycle diagnostic; also covered by the required pair matrix"]
fn protected_pair_lifecycle() {
    let endpoints: Vec<_> = ["WAL", "16.15", "mysql"]
        .into_iter()
        .map(Endpoint::new)
        .collect();
    let fixture = fixture();
    for (left, right) in [(0, 1), (1, 0), (1, 2), (2, 1), (2, 0), (0, 2)] {
        lifecycle::qualify(&fixture, [&endpoints[left], &endpoints[right]]);
    }
}
