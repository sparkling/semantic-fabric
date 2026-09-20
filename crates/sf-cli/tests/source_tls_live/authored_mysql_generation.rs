//! Protected authored MySQL profile through the actual TLS serving binary.
use super::*;
#[path = "authored_mysql_generation_checks.rs"]
mod lifecycle;
#[path = "authored_mysql_generation_policy.rs"]
mod portable;

fn profile(fixture: &Fixture, database: &Database) -> (Command, SocketAddr) {
    profile_interval(fixture, database, "1")
}

fn profile_interval(
    fixture: &Fixture,
    database: &Database,
    interval: &str,
) -> (Command, SocketAddr) {
    let (mut command, address) = command(fixture, database, None);
    command.args([
        "--require-verified-generation",
        "--reload-interval-secs",
        interval,
        "--shutdown-timeout-secs",
        "1",
    ]);
    command.env(
        "SF_TLS_SOURCE",
        format!("{}?pool_min=1&pool_max=1", database.source),
    );
    (command, address)
}

#[test]
#[ignore = "requires Docker and pinned owned MySQL8.4.11 image; required in CI"]
fn public_authored_mysql_generation_is_protected() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, false);
    assert_eq!(database.sql("SELECT VERSION()"), "8.4.11");
    support::mapping(&fixture.root.join("first.ttl"), "http://example.test/left");
    fixture.write(
        "ontology.ttl",
        "<http://example.test/left> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .",
    );
    let mapping = std::fs::read_to_string(fixture.root.join("first.ttl")).unwrap();
    // New sessions inherit deliberately incompatible defaults. The protected
    // profile must establish and verify its own settings before any table read.
    database.sql("SET GLOBAL autocommit=0; SET GLOBAL transaction_isolation='READ-COMMITTED'; SET GLOBAL sql_mode='PIPES_AS_CONCAT'; SET GLOBAL time_zone='+05:00'");
    let (command, address) = profile(&fixture, &database);
    let mut server = start(&fixture, command, address);
    database.sql("SET GLOBAL autocommit=1; SET GLOBAL transaction_isolation='REPEATABLE-READ'; SET GLOBAL time_zone='+00:00'");
    for _ in 0..2 {
        value(address, &fixture.token, "same");
    }
    database.assert_encrypted_sessions();
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
    lineage::assert_responses(address, &fixture, &database);
    fixture.write(
        "first.ttl",
        &mapping.replace(
            "rr:column \"value\"",
            r#"rr:template "a\\\\'{value}" ; rr:termType rr:Literal"#,
        ),
    );
    super::checks::await_value(&mut server, address, &fixture.token, "a\\'same");
    fixture.write("first.ttl", "invalid replacement");
    await_ready(&mut server, address, 503);
    assert_eq!(
        request(address, SINGLE, Some(&fixture.token)).unwrap().0,
        503
    );
    fixture.write("first.ttl", &mapping);
    await_ready(&mut server, address, 200);
    value(address, &fixture.token, "same");
    database.sql("ALTER TABLE sf_tls.items RENAME COLUMN value TO gone");
    await_ready(&mut server, address, 503);
    assert_eq!(
        request(address, SINGLE, Some(&fixture.token)).unwrap().0,
        503
    );
    database.sql("ALTER TABLE sf_tls.items RENAME COLUMN gone TO value");
    await_ready(&mut server, address, 200);
    value(address, &fixture.token, "same");
    stop(&mut server);
    lifecycle::qualify(&fixture, &database, &mapping);
    portable::qualify(&fixture, &database, &mapping);
    boundaries(&fixture, &database, &mapping);
}

fn boundaries(fixture: &Fixture, database: &Database, mapping: &str) {
    for (ddl, mapped) in [
        (
            "CREATE VIEW sf_tls.excluded AS SELECT value FROM sf_tls.items",
            "excluded",
        ),
        (
            "CREATE TABLE sf_tls.excluded(value TEXT) ENGINE=MyISAM",
            "excluded",
        ),
    ] {
        database.sql(ddl);
        fixture.write(
            "first.ttl",
            &mapping.replace(
                "rr:tableName \"items\"",
                &format!("rr:tableName \"{mapped}\""),
            ),
        );
        let (mut command, _) = profile(fixture, database);
        let result = support::output(&mut command, Duration::from_secs(40));
        assert_eq!(result.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&result.stderr).contains("startup-configuration"));
        if ddl.starts_with("CREATE VIEW") {
            database.sql("DROP VIEW sf_tls.excluded");
        } else {
            database.sql("DROP TABLE sf_tls.excluded");
        }
    }
    fixture.write(
        "first.ttl",
        &mapping.replace(
            "rr:tableName \"items\"",
            "rr:sqlQuery \"SELECT value FROM items\"",
        ),
    );
    let (mut command, _) = profile(fixture, database);
    let result = support::output(&mut command, Duration::from_secs(40));
    assert_eq!(result.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&result.stderr).contains("startup-configuration"));
    fixture.write("first.ttl", mapping);
    let (mut command, address) = profile_interval(fixture, database, "60");
    command.args(["--max-source-work", "0"]);
    let mut server = start(fixture, command, address);
    let mut lock = database.hold_table("items");
    assert_eq!(
        request(address, SINGLE, Some(&fixture.token)).unwrap().0,
        429
    );
    lock.assert_held();
    drop(lock);
    stop(&mut server);
}
