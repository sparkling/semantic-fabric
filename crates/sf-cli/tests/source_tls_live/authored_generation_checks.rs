//! Failure and lifecycle witnesses for the owned authored-generation profile.
use super::*;
use std::net::Shutdown;

pub(super) fn await_value(server: &mut Server, address: SocketAddr, token: &str, expected: &str) {
    let until = Instant::now() + Duration::from_secs(35);
    loop {
        if let Some((200, body)) = request(address, SINGLE, Some(token)) {
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            if json["results"]["bindings"][0]["value"]["value"] == expected {
                value(address, token, expected);
                return;
            }
        }
        assert!(server.0.try_wait().unwrap().is_none());
        assert!(Instant::now() < until, "authored successor value");
        thread::sleep(Duration::from_millis(25));
    }
}

fn generation_session(database: &Database) -> u64 {
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        let result = database.sql("SELECT DISTINCT a.pid FROM pg_stat_activity a JOIN pg_locks l ON a.pid=l.pid WHERE a.usename='sf_tls' AND l.relation='public.items'::regclass AND l.mode='AccessShareLock' AND l.granted AND a.query LIKE 'SELECT%' AND a.query LIKE '%\"items\"%'");
        if let Ok(pid) = result.parse::<u64>() {
            assert!(pid > 0);
            return pid;
        }
        assert!(
            Instant::now() < until,
            "request-owned native generation lease"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn held_request(address: SocketAddr, token: &str) -> (TcpStream, Vec<u8>) {
    let mut stream = cancellation::begin(address, SINGLE, token);
    let mut headers = Vec::new();
    while !headers.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        headers.push(byte[0]);
        assert!(headers.len() < 8192);
    }
    assert!(headers.starts_with(b"HTTP/1.1 200 "));
    let text = std::str::from_utf8(&headers).unwrap().to_ascii_lowercase();
    assert!(text.contains("\r\ntransfer-encoding: chunked\r\n"));
    assert!(!text.contains("\r\ncontent-length:"));
    (stream, headers)
}

fn await_stopped(database: &Database, pid: u64) {
    let until = Instant::now() + Duration::from_secs(3);
    // Closing through the acknowledged rollback is safe too. Detachment is
    // mandatory only when cancellation/error prevented that clean close.
    let unsafe_owner = format!("SELECT count(*) FROM pg_stat_activity a WHERE a.pid={pid} AND NOT (a.state='idle' AND a.xact_start IS NULL AND a.backend_xmin IS NULL AND a.query='ROLLBACK' AND NOT EXISTS (SELECT 1 FROM pg_locks l WHERE l.pid=a.pid AND l.locktype='relation' AND l.granted))");
    while database.sql(&unsafe_owner) != "0" {
        assert!(
            Instant::now() < until,
            "stopped request retained native work or an unclean member"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

pub(super) fn pinned_and_cancelled(
    fixture: &Fixture,
    database: &Database,
    server: &mut Server,
    address: SocketAddr,
    mapping: &str,
) {
    // Many distinct rows exceed TCP and channel buffers, keeping native work
    // owned until the consumer drains. A single queued row would not prove this.
    fill_stream(database);
    let (stream, mut wire) = held_request(address, &fixture.token);
    let pid = generation_session(database);
    assert_eq!(
        database.sql(&format!("SELECT ssl FROM pg_stat_ssl WHERE pid={pid}")),
        "t"
    );
    fixture.write("first.ttl", "invalid authored replacement");
    await_ready(server, address, 503);
    assert_eq!(
        request(address, SINGLE, Some(&fixture.token)).unwrap().0,
        503
    );
    assert_eq!(database.sql(&format!("SELECT count(*) FROM pg_locks WHERE pid={pid} AND relation='public.items'::regclass AND mode='AccessShareLock' AND granted")), "1", "pinned old request retains its exact generation despite readiness fencing");
    fixture.write(
        "first.ttl",
        &mapping.replace("rr:column \"value\"", "rr:constant \"replacement\""),
    );
    await_ready(server, address, 200);
    value(address, &fixture.token, "replacement");
    stream.take(40_000_001).read_to_end(&mut wire).unwrap();
    assert!(wire.len() <= 40_000_000);
    let (status, body) = decode_response(wire);
    assert_eq!(status, 200);
    let old: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let rows = old["results"]["bindings"].as_array().unwrap();
    assert_eq!(rows.len(), 1024);
    let prefix = "x".repeat(32760);
    let ids = rows
        .iter()
        .map(|row| {
            assert_eq!(row["s"]["value"], "http://example.test/item");
            let value = row["value"]["value"].as_str().unwrap();
            assert_eq!(value.len(), 32768);
            value
                .strip_prefix(&prefix)
                .unwrap()
                .parse::<usize>()
                .unwrap()
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids, (1..=1024).collect());

    fixture.write("first.ttl", "invalid authored replacement");
    await_ready(server, address, 503);
    fixture.write("first.ttl", mapping);
    await_ready(server, address, 200);
    let (stream, _) = held_request(address, &fixture.token);
    let pid = generation_session(database);
    stream.shutdown(Shutdown::Both).unwrap();
    drop(stream);
    await_stopped(database, pid);
    reset_stream(database);
    fixture.write("first.ttl", mapping);
    await_ready(server, address, 200);
    value(address, &fixture.token, "same");
}

fn fill_stream(database: &Database) {
    database.sql("TRUNCATE public.items; INSERT INTO public.items(value) SELECT repeat('x',32760)||lpad(g::text,8,'0') FROM generate_series(1,1024) g");
}

fn reset_stream(database: &Database) {
    database.sql("TRUNCATE public.items; INSERT INTO public.items(value) VALUES ('same')");
}

pub(super) fn deadline_and_shutdown(fixture: &Fixture, database: &Database) {
    for shutdown in [false, true] {
        eprintln!("authored native stop: shutdown={shutdown}");
        fill_stream(database);
        let (mut command, address) = profile(fixture, database);
        command.args(["--timeout-secs", if shutdown { "30" } else { "2" }]);
        let mut server = start(fixture, command, address);
        let (stream, mut wire) = held_request(address, &fixture.token);
        let pid = generation_session(database);
        if shutdown {
            stop(&mut server);
        }
        await_stopped(database, pid);
        let result = stream.take(40_000_001).read_to_end(&mut wire);
        if let Err(error) = result {
            assert!(matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::UnexpectedEof
            ));
        }
        assert!(wire.len() <= 40_000_000);
        assert!(
            !wire.ends_with(b"\r\n0\r\n\r\n"),
            "stopped response completed successfully"
        );
        reset_stream(database);
        if !shutdown {
            value(address, &fixture.token, "same");
            stop(&mut server);
        }
    }
}

fn rejected(mut command: Command, address: SocketAddr) {
    let mut server = Server(
        command
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let until = Instant::now() + Duration::from_secs(35);
    loop {
        assert!(
            TcpStream::connect(address).is_err(),
            "rejected profile opened listener"
        );
        if let Some(status) = server.0.try_wait().unwrap() {
            assert!(!status.success());
            return;
        }
        assert!(Instant::now() < until, "rejected profile did not terminate");
        thread::sleep(Duration::from_millis(20));
    }
}

pub(super) fn rejected_profiles(fixture: &Fixture, database: &Database, mapping: &str) {
    fixture.write(
        "first.ttl",
        &mapping.replace(
            "rr:tableName \"items\"",
            "rr:sqlQuery \"SELECT value FROM public.items\"",
        ),
    );
    let (command, address) = profile(fixture, database);
    rejected(command, address);
    fixture.write("first.ttl", mapping);
    let (mut command, address) = profile(fixture, database);
    command.env("SF_TLS_ROOTS", fixture.certificates("untrusted"));
    rejected(command, address);
    let (mut command, address) = profile(fixture, database);
    command.args(["--reload-interval-secs", "0"]);
    rejected(command, address);
    database.sql("GRANT UPDATE ON public.items TO sf_tls");
    let (command, address) = profile(fixture, database);
    rejected(command, address);
    database.sql("REVOKE UPDATE ON public.items FROM sf_tls");

    // The request generation's 34 metadata units are reserved before pool I/O;
    // 33 cannot buy an unverified fallback. Full queries also pay executor work.
    let (mut command, address) = profile(fixture, database);
    command.args(["--max-source-work", "33"]);
    let mut server = start(fixture, command, address);
    let before = database.sql("SELECT count(*) FROM pg_stat_activity WHERE usename='sf_tls'");
    let (status, _) = request(address, SINGLE, Some(&fixture.token)).unwrap();
    assert_eq!(status, 429);
    assert_eq!(
        database.sql("SELECT count(*) FROM pg_stat_activity WHERE usename='sf_tls'"),
        before
    );
    stop(&mut server);
}
