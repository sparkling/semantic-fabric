//! Required real-process public join test on the minimal serving binary.
use super::*;
const JOIN: &str = "SELECT ?left ?right WHERE { ?left <http://example.test/left> ?key . ?right <http://example.test/right> ?key }";

#[test]
fn cli_joins_two_sources_with_hidden_keys_and_exact_multiplicity() {
    let token = "test-only-native-join-token-0123456789";
    let (fixture, address, mut server) = start_reloading(false, Some(token), true, 1);
    let startup_deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if request_with_token(address, Some(token))
            .is_some_and(|response| response.starts_with("HTTP/1.1 200"))
        {
            break;
        }
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "initial child exited"
        );
        assert!(
            Instant::now() < startup_deadline,
            "initial serving path did not start"
        );
        thread::sleep(Duration::from_millis(25));
    }
    for (name, predicate, count) in [("first", "left", 3), ("second", "right", 4)] {
        let conn = rusqlite::Connection::open(fixture.root.join(format!("{name}.db"))).unwrap();
        conn.execute_batch("ALTER TABLE items ADD COLUMN id TEXT; DELETE FROM items;")
            .unwrap();
        for index in 0..count {
            conn.execute(
                "INSERT INTO items(value,id) VALUES (?1,?2)",
                ["same", &format!("{predicate}-{index}")],
            )
            .unwrap();
        }
        let mapping = mapping(&format!("http://example.test/{predicate}"))
            .replace(
                "rr:constant <http://example.test/item>",
                "rr:template \"http://example.test/item/{id}\"",
            )
            .replace(
                "rr:column \"value\"",
                "rr:column \"value\" ; rr:datatype <http://www.w3.org/2001/XMLSchema#string>",
            );
        std::fs::write(fixture.root.join(format!("{name}.ttl")), mapping).unwrap();
    }
    let deadline = Instant::now() + Duration::from_secs(15);
    let response = loop {
        if let Some(response) = request_query(address, JOIN, Some(token)) {
            if response.starts_with("HTTP/1.1 200") {
                break response;
            }
        }
        assert!(server.0.try_wait().unwrap().is_none(), "join child exited");
        assert!(Instant::now() < deadline, "join never became exercisable");
        thread::sleep(Duration::from_millis(25));
    };
    let body = response.split_once("\r\n\r\n").unwrap().1;
    let result: serde_json::Value = serde_json::from_str(body).unwrap();
    let rows = result["results"]["bindings"].as_array().unwrap();
    assert_eq!(rows.len(), 12);
    for left in 0..3 {
        for right in 0..4 {
            assert_eq!(
                rows.iter()
                    .filter(|row| row["left"]["value"]
                        == format!("http://example.test/item/left-{left}")
                        && row["right"]["value"]
                            == format!("http://example.test/item/right-{right}"))
                    .count(),
                1
            );
        }
    }
    assert!(request_query(address, JOIN, None)
        .unwrap()
        .starts_with("HTTP/1.1 401"));
}
