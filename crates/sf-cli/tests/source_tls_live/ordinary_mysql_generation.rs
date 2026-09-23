//! G3 ordinary-mode MySQL: no `--require-verified-generation`, reload disabled.
//!
//! Ordinary startup binds the protected MySQL generation when admission permits
//! it, so replacing the mapped table after startup is refused rather than
//! answered from the replacement as if it were the observed table.
use super::*;

#[test]
#[ignore = "requires Docker and pinned owned MySQL8.4.11 image; required in CI"]
fn ordinary_authored_mysql_refuses_replaced_table() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, false);
    assert_eq!(database.sql("SELECT VERSION()"), "8.4.11");
    support::mapping(&fixture.root.join("first.ttl"), "http://example.test/left");
    fixture.write(
        "ontology.ttl",
        "<http://example.test/left> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .",
    );
    let (mut command, address) = command(&fixture, &database, None);
    command.args(["--shutdown-timeout-secs", "1"]);
    command.env(
        "SF_TLS_SOURCE",
        format!("{}?pool_min=1&pool_max=1", database.source),
    );
    let mut server = start(&fixture, command, address);
    value(address, &fixture.token, "same");

    // Same name and column, different meaning: the compiled SQL still runs.
    database.sql(
        "DROP TABLE sf_tls.items; \
         CREATE TABLE sf_tls.items(value TEXT NOT NULL, unit TEXT); \
         INSERT INTO sf_tls.items VALUES ('replaced', 'other'); \
         GRANT SELECT ON sf_tls.items TO 'sf_tls'@'%';",
    );
    let (status, body) = request(address, SINGLE, Some(&fixture.token)).unwrap();
    assert!(
        !(status == 200 && String::from_utf8_lossy(&body).contains("replaced")),
        "ordinary mode answered from a replaced table"
    );
    assert_eq!(
        status,
        503,
        "{}; server: {}",
        String::from_utf8_lossy(&body),
        std::fs::read_to_string(fixture.root.join("authored.stderr")).unwrap_or_default()
    );
    stop(&mut server);
}
