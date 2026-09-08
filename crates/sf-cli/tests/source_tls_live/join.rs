//! Required encrypted mixed-driver join proof, both triple-pattern orders.
use super::*;
pub(super) const JOIN: &str = "SELECT ?left ?right WHERE { ?left <http://example.test/left> ?key . ?right <http://example.test/right> ?key }";
pub(super) const REVERSED: &str = "SELECT ?left ?right WHERE { ?right <http://example.test/right> ?key . ?left <http://example.test/left> ?key }";

pub(super) fn assert_joins(fixture: &Fixture, postgres: &Database, mysql: &Database) {
    postgres.sql("ALTER TABLE public.items ADD COLUMN id TEXT; DELETE FROM public.items; INSERT INTO public.items(value,id) VALUES ('A /%','p1'),('A /%','p2'),('A /%','p3');");
    mysql.sql("ALTER TABLE sf_tls.items ADD COLUMN id VARCHAR(32); DELETE FROM sf_tls.items; INSERT INTO sf_tls.items(value,id) VALUES ('A /%','m1'),('A /%','m2'),('A /%','m3'),('A /%','m4'),('a /%','false-positive');");
    for file in ["first.ttl", "second.ttl"] {
        let path = fixture.root.join(file);
        let old = std::fs::read_to_string(&path).unwrap();
        let new = old
            .replace(
                "rr:constant <http://example.test/item>",
                "rr:template \"http://example.test/item/{id}\"",
            )
            .replace(
                "rr:column \"value\"",
                "rr:column \"value\" ; rr:datatype <http://www.w3.org/2001/XMLSchema#string>",
            );
        assert_ne!(old, new);
        std::fs::write(path, new).unwrap();
    }
    let (mut command, address) = command(fixture, postgres, Some(mysql));
    let mut server = Server(
        command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    for query in [JOIN, REVERSED] {
        let deadline = Instant::now() + Duration::from_secs(40);
        let result = loop {
            if let Some((status, body)) = request(address, query, Some(&fixture.token)) {
                assert_eq!(
                    status,
                    200,
                    "join rejected: {}",
                    String::from_utf8_lossy(&body)
                );
                break serde_json::from_slice::<serde_json::Value>(&body).unwrap();
            }
            assert!(
                server.0.try_wait().unwrap().is_none(),
                "mixed join child exited"
            );
            assert!(Instant::now() < deadline, "mixed join did not start");
            thread::sleep(Duration::from_millis(25));
        };
        let rows = result["results"]["bindings"].as_array().unwrap();
        assert_eq!(rows.len(), 12, "exact native mixed-source bag");
        for p in 1..=3 {
            for m in 1..=4 {
                assert_eq!(
                    rows.iter()
                        .filter(|row| row["left"]["value"]
                            == format!("http://example.test/item/p{p}")
                            && row["right"]["value"] == format!("http://example.test/item/m{m}"))
                        .count(),
                    1
                );
            }
        }
    }
    assert_eq!(request(address, JOIN, None).unwrap().0, 401);
    postgres.assert_encrypted_sessions();
    mysql.assert_encrypted_sessions();
}
