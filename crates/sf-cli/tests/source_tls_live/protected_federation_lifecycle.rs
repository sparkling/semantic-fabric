//! Public two-source publication, drift and retained lease recovery.
use super::*;

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

fn await_ready(fixture: &Fixture, server: &mut Server, address: SocketAddr, expected: u16) {
    let until = Instant::now() + Duration::from_secs(35);
    while ready(address) != expected {
        assert!(server.0.try_wait().unwrap().is_none());
        assert!(
            Instant::now() < until,
            "pair readiness {expected}: {}",
            std::fs::read_to_string(fixture.root.join("server.log")).unwrap()
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn await_values(server: &mut Server, address: SocketAddr, token: &str, expected: &[&str]) {
    let until = Instant::now() + Duration::from_secs(35);
    loop {
        let (status, body) = request(address, UNION, Some(token)).unwrap();
        if status == 200 {
            let document: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let mut values: Vec<_> = document["results"]["bindings"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["value"]["value"].as_str().unwrap())
                .collect();
            values.sort();
            assert_eq!(values, expected);
            ordinary(address, token, UNION, expected);
            return;
        }
        assert_eq!(status, 503);
        assert!(server.0.try_wait().unwrap().is_none());
        assert!(Instant::now() < until, "pair public recovery");
        thread::sleep(Duration::from_millis(25));
    }
}

pub(super) fn qualify(fixture: &Fixture, pair: [&Endpoint; 2]) {
    eprintln!(
        "protected lifecycle {} -> {}",
        pair[0].profile, pair[1].profile
    );
    for (slot, endpoint) in pair.iter().enumerate() {
        endpoint.sql(&format!(
            "DELETE FROM items; INSERT INTO items(value,tenant) VALUES('slot{slot}-old','a')"
        ));
    }
    let mappings = ["first.ttl", "second.ttl"]
        .map(|file| std::fs::read_to_string(fixture.root.join(file)).unwrap());
    pair[0].sql("UPDATE items SET successor='slot0-successor'");
    let (command, address) = profile(fixture, pair, false, "2000");
    let mut server = start(fixture, command, address);
    ordinary(address, &fixture.token, UNION, &["slot0-old", "slot1-old"]);
    fixture.write("second.ttl", "invalid replacement");
    await_ready(fixture, &mut server, address, 503);
    fixture.write(
        "first.ttl",
        &mappings[0].replace("rr:column \"value\"", "rr:column \"successor\""),
    );
    assert_eq!(
        request(address, UNION, Some(&fixture.token)).unwrap().0,
        503
    );
    fixture.write("second.ttl", &mappings[1]);
    await_ready(fixture, &mut server, address, 200);
    ordinary(
        address,
        &fixture.token,
        UNION,
        &["slot0-successor", "slot1-old"],
    );
    // Restore both column mappings before exercising actual mapped-column drift.
    fixture.write("first.ttl", "invalid replacement");
    await_ready(fixture, &mut server, address, 503);
    fixture.write("first.ttl", &mappings[0]);
    await_ready(fixture, &mut server, address, 200);
    for endpoint in pair {
        eprintln!("drift {}", endpoint.profile);
        endpoint.sql("ALTER TABLE items RENAME COLUMN value TO value_drift");
        await_ready(fixture, &mut server, address, 503);
        assert_eq!(
            request(address, UNION, Some(&fixture.token)).unwrap().0,
            503
        );
        endpoint.sql("ALTER TABLE items RENAME COLUMN value_drift TO value");
        await_ready(fixture, &mut server, address, 200);
        await_values(
            &mut server,
            address,
            &fixture.token,
            &["slot0-old", "slot1-old"],
        );
    }
    stop(server);

    pair[0].fill_stream();
    let (mut command, address) = profile(fixture, pair, false, "2000");
    command.args(["--reload-interval-secs", "86400"]);
    let server = start(fixture, command, address);
    let mut stream = cancellation::begin(address, UNION, &fixture.token);
    let mut headers = Vec::new();
    while !headers.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        headers.push(byte[0]);
        assert!(headers.len() < 8192);
    }
    assert!(headers.starts_with(b"HTTP/1.1 200 "));
    // The first unread 32 MiB arm backpressures execution; both local leases
    // must already exist, including the source whose arm has not begun.
    for endpoint in pair {
        endpoint.prepare_lease_witness();
        endpoint.await_lease(true);
    }
    stream.shutdown(std::net::Shutdown::Both).unwrap();
    drop(stream);
    for endpoint in pair {
        endpoint.await_lease(false);
        endpoint.reset();
    }
    ordinary(
        address,
        &fixture.token,
        UNION,
        &["same", "same", "same-b", "same-b"],
    );
    stop(server);

    // A failure opening either native source must release any previously
    // acquired peer lease before the next cap-one request can succeed.
    let (mut command, address) = profile(fixture, pair, false, "100");
    command.args(["--reload-interval-secs", "86400", "--timeout-secs", "2"]);
    let server = start(fixture, command, address);
    for endpoint in pair {
        if let Some(database) = &endpoint.database {
            let mut held = database.hold_table("items");
            let (status, body) = request(address, UNION, Some(&fixture.token)).unwrap();
            assert_eq!(
                status,
                if endpoint.profile == "mysql" {
                    504
                } else {
                    503
                }
            );
            assert!(!String::from_utf8_lossy(&body).contains(&database.source));
            held.assert_held();
            drop(held);
            ordinary(
                address,
                &fixture.token,
                UNION,
                &["same", "same", "same-b", "same-b"],
            );
        }
    }
    stop(server);
}
