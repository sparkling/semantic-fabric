//! Reload through the shipped native CLI, including its layered options.
use super::*;

const TOKEN: &str = "test-only-reload-cli-token-0123456789";

fn wait_for(address: SocketAddr, server: &mut Server, accept: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut last = String::new();
    loop {
        if let Some(response) = request_with_token(address, Some(TOKEN)) {
            if accept(&response) {
                return response;
            }
            last = response;
        }
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "reload child exited"
        );
        assert!(
            Instant::now() < deadline,
            "automatic reload did not converge: {last}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn cli_automatically_reloads_both_sources_and_recovers_readiness() {
    let (fixture, address, mut server) = start_reloading(false, Some(TOKEN), true, 1);
    wait_for(address, &mut server, |r| {
        r.starts_with("HTTP/1.1 200") && r.matches("\"value\":\"same\"").count() == 2
    });
    for (file, predicate, value) in [
        ("first.ttl", "left", "new-left"),
        ("second.ttl", "right", "new-right"),
    ] {
        let database = fixture.root.join(file.replace(".ttl", ".db"));
        let connection = rusqlite::Connection::open(database).unwrap();
        connection
            .execute_batch("ALTER TABLE items ADD COLUMN refreshed TEXT;")
            .unwrap();
        connection
            .execute("UPDATE items SET refreshed=?1", [value])
            .unwrap();
        std::fs::write(
            fixture.root.join(file),
            mapping(&format!("http://example.test/{predicate}"))
                .replace("rr:column \"value\"", "rr:column \"refreshed\""),
        )
        .unwrap();
    }
    let response = wait_for(address, &mut server, |r| {
        r.starts_with("HTTP/1.1 200")
            && r.contains("\"value\":\"new-left\"")
            && r.contains("\"value\":\"new-right\"")
    });
    assert!(!response.contains("\"value\":\"same\""));
    assert!(request_with_token(address, None)
        .unwrap()
        .starts_with("HTTP/1.1 401"));
    let ontology_path = fixture.root.join("ontology.ttl");
    let ontology = std::fs::read_to_string(&ontology_path).unwrap();
    std::fs::write(&ontology_path, "not turtle").unwrap();
    wait_for(address, &mut server, |r| r.starts_with("HTTP/1.1 503"));
    for (route, status) in [("/readyz", 503), ("/livez", 200)] {
        let mut stream = TcpStream::connect(address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write!(
            stream,
            "GET {route} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with(&format!("HTTP/1.1 {status}")));
    }
    std::fs::write(ontology_path, ontology).unwrap();
    wait_for(address, &mut server, |r| {
        r.starts_with("HTTP/1.1 200") && r.contains("new-right")
    });
    #[cfg(unix)]
    {
        // Only this fixture's child is signalled and reaped.
        assert_eq!(
            unsafe { libc::kill(server.0.id() as libc::pid_t, libc::SIGTERM) },
            0
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = server.0.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(
                Instant::now() < deadline,
                "reload supervisor prevented shutdown"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}
