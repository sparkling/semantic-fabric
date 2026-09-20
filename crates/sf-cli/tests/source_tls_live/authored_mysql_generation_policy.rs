//! Caller isolation and policy-only schema drift on the protected MySQL path.
use super::*;
const B: &str = "fixture-only-mysql-policy-b-0123456789";
const DENIED: &str = "fixture-only-mysql-policy-denied-0123456789";

fn bag(address: SocketAddr, token: &str, expected: &[&str]) {
    let (status, body) = request(address, SINGLE, Some(token)).unwrap();
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let mut values: Vec<_> = json["results"]["bindings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["value"]["value"].as_str().unwrap())
        .collect();
    values.sort();
    assert_eq!(values, expected);
}

pub(super) fn qualify(fixture: &Fixture, database: &Database, mapping: &str) {
    database.sql("ALTER TABLE sf_tls.items ADD COLUMN tenant VARCHAR(8); TRUNCATE sf_tls.items; INSERT INTO sf_tls.items(value,tenant) VALUES('only-a','a'),('only-b','b')");
    fixture.write("first.ttl", mapping);
    let subjects: Vec<_> = [("a", "SF_A", "items"), ("b", "SF_B", "items"), ("denied", "SF_DENIED", "other")].into_iter().map(|(subject, credential, table)| serde_json::json!({
        "subjectRef":subject,"credentialEnv":credential,"portableRows":[{"sourceIndex":0,"table":table,"column":"tenant","valueEnv":format!("{credential}_VALUE")}]
    })).collect();
    let (mut command, address) = command_with_admission(
        fixture,
        database,
        None,
        &["--auth-subjects-env", "SF_SUBJECTS"],
    );
    command
        .args([
            "--require-verified-generation",
            "--reload-interval-secs",
            "1",
            "--shutdown-timeout-secs",
            "1",
        ])
        .env(
            "SF_TLS_SOURCE",
            format!("{}?pool_min=1&pool_max=1", database.source),
        )
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
    let mut server = start(fixture, command, address);
    for token in [&fixture.token[..], B, &fixture.token] {
        bag(
            address,
            token,
            &[if token == B { "only-b" } else { "only-a" }],
        );
    }
    let mut lock = database.hold_table("items");
    assert_eq!(request(address, SINGLE, Some(DENIED)).unwrap().0, 403);
    assert_eq!(
        request(address, &format!("{SINGLE} ORDER BY ?value"), Some(DENIED))
            .unwrap()
            .0,
        403
    );
    lock.assert_held();
    drop(lock);
    database.sql("ALTER TABLE sf_tls.items DROP COLUMN tenant");
    await_ready(&mut server, address, 503);
    let until = Instant::now() + Duration::from_millis(2300);
    while Instant::now() < until {
        assert_eq!(ready(address), 503);
        assert_eq!(
            request(address, SINGLE, Some(&fixture.token)).unwrap().0,
            503
        );
        thread::sleep(Duration::from_millis(100));
    }
    database.sql("ALTER TABLE sf_tls.items ADD COLUMN tenant VARCHAR(8); UPDATE sf_tls.items SET tenant=IF(value='only-a','a','b')");
    await_ready(&mut server, address, 200);
    bag(address, &fixture.token, &["only-a"]);
    bag(address, B, &["only-b"]);
    fixture.write(
        "first.ttl",
        &mapping.replace("rr:column \"value\"", "rr:constant \"replacement\""),
    );
    super::super::checks::await_value(&mut server, address, &fixture.token, "replacement");
    bag(address, B, &["replacement"]);
    stop(&mut server);
    fixture.write("first.ttl", mapping);
    database.sql("ALTER TABLE sf_tls.items DROP COLUMN tenant; TRUNCATE sf_tls.items; INSERT INTO sf_tls.items(value) VALUES('same')");
}
