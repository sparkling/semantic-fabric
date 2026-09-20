//! Native witnesses for protected MySQL leases and dirty-connection disposal.
use super::*;
use std::net::Shutdown;

fn held(address: SocketAddr, token: &str) -> (TcpStream, Vec<u8>) {
    super::super::checks::held_request(address, token)
}

fn owner(database: &Database) -> u64 {
    let until = Instant::now() + Duration::from_secs(4);
    loop {
        let rows = database.sql("SELECT DISTINCT t.PROCESSLIST_ID FROM performance_schema.threads t JOIN performance_schema.metadata_locks m ON m.OWNER_THREAD_ID=t.THREAD_ID WHERE t.PROCESSLIST_USER='sf_tls' AND m.OBJECT_SCHEMA='sf_tls' AND m.OBJECT_NAME='items' AND m.LOCK_TYPE='SHARED_READ' AND m.LOCK_STATUS='GRANTED' AND t.PROCESSLIST_INFO LIKE '%items%' AND t.PROCESSLIST_INFO NOT LIKE 'SELECT 1 FROM%' AND t.PROCESSLIST_INFO NOT LIKE '%information_schema%'");
        if let Ok(id) = rows.parse::<u64>() {
            assert!(id > 0);
            assert_eq!(database.sql(&format!("SELECT count(*) FROM performance_schema.status_by_thread s JOIN performance_schema.threads t USING(THREAD_ID) WHERE t.PROCESSLIST_ID={id} AND s.VARIABLE_NAME='Ssl_cipher' AND s.VARIABLE_VALUE<>''")), "1");
            return id;
        }
        assert!(Instant::now() < until, "no exact native generation owner");
        thread::sleep(Duration::from_millis(20));
    }
}

fn stopped(database: &Database, id: u64) {
    let until = Instant::now() + Duration::from_secs(4);
    loop {
        let active = database.sql(&format!("SELECT count(*) FROM performance_schema.threads t WHERE t.PROCESSLIST_ID={id} AND (t.PROCESSLIST_COMMAND<>'Sleep' OR EXISTS (SELECT 1 FROM information_schema.innodb_trx x WHERE x.trx_mysql_thread_id={id}) OR EXISTS (SELECT 1 FROM performance_schema.metadata_locks m WHERE m.OWNER_THREAD_ID=t.THREAD_ID AND m.OBJECT_SCHEMA='sf_tls' AND m.OBJECT_TYPE='TABLE'))"));
        if active == "0" {
            return;
        }
        assert!(
            Instant::now() < until,
            "native generation owner remained dirty"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

// Deliberate32MiB backpressure fixtures use an explicit source allowance;
// small-profile acceptance in the parent retains the product defaults.
fn fill(database: &Database) {
    database.sql("TRUNCATE sf_tls.items; SET SESSION cte_max_recursion_depth=1100; INSERT INTO sf_tls.items(value) WITH RECURSIVE n AS (SELECT 1 AS n UNION ALL SELECT n+1 FROM n WHERE n<1024) SELECT CONCAT(REPEAT('x',32760),LPAD(n,8,'0')) FROM n");
}

fn reset(database: &Database) {
    database.sql("TRUNCATE sf_tls.items; INSERT INTO sf_tls.items(value) VALUES('same')");
}

fn exact_old(stream: TcpStream, mut wire: Vec<u8>) {
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
}

pub(super) fn qualify(fixture: &Fixture, database: &Database, mapping: &str) {
    eprintln!("MySQL protected old snapshot across activation and DML");
    fill(database);
    let (mut command, address) = profile(fixture, database);
    command.args(["--max-source-work", "100000000"]);
    let mut server = start(fixture, command, address);
    let (stream, wire) = held(address, &fixture.token);
    let id = owner(database);
    database.sql("UPDATE sf_tls.items SET value='new-data'");
    fixture.write("first.ttl", "invalid generation");
    await_ready(&mut server, address, 503);
    assert_eq!(
        request(address, SINGLE, Some(&fixture.token)).unwrap().0,
        503
    );
    fixture.write(
        "first.ttl",
        &mapping.replace("rr:column \"value\"", "rr:constant \"replacement\""),
    );
    await_ready(&mut server, address, 200);
    value(address, &fixture.token, "replacement");
    exact_old(stream, wire);
    stopped(database, id);
    stop(&mut server);
    fixture.write("first.ttl", mapping);
    reset(database);

    // Keep reload off this DDL witness interval; both mapped tables must remain
    // protected even when the public query only references the first mapping.
    database.sql("CREATE TABLE sf_tls.healthy(value TEXT NOT NULL) ENGINE=InnoDB; INSERT INTO sf_tls.healthy VALUES('healthy')");
    let second = mapping
        .replace("<#items>", "<#healthy>")
        .replace("rr:tableName \"items\"", "rr:tableName \"healthy\"")
        .replace("http://example.test/left", "http://example.test/right");
    fixture.write("first.ttl", &format!("{mapping}\n{second}"));
    fixture.write("ontology.ttl", "<http://example.test/left> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .\n<http://example.test/right> a <http://www.w3.org/2002/07/owl#DatatypeProperty> .");
    fill(database);
    let (mut command, address) = profile_interval(fixture, database, "60");
    command.args(["--max-source-work", "100000000"]);
    let mut server = start(fixture, command, address);
    let (stream, wire) = held(address, &fixture.token);
    let id = owner(database);
    assert_eq!(database.sql(&format!("SELECT m.OBJECT_NAME FROM performance_schema.metadata_locks m JOIN performance_schema.threads t ON t.THREAD_ID=m.OWNER_THREAD_ID WHERE t.PROCESSLIST_ID={id} AND m.OBJECT_SCHEMA='sf_tls' AND m.OBJECT_TYPE='TABLE' AND m.LOCK_TYPE='SHARED_READ' AND m.LOCK_STATUS='GRANTED' ORDER BY m.OBJECT_NAME")), "healthy\nitems");
    thread::scope(|scope| {
        let first =
            scope.spawn(|| database.sql("ALTER TABLE sf_tls.items ADD COLUMN released INT"));
        let second =
            scope.spawn(|| database.sql("ALTER TABLE sf_tls.healthy ADD COLUMN released INT"));
        let until = Instant::now() + Duration::from_secs(4);
        loop {
            let pending = database.sql("SELECT count(DISTINCT OBJECT_NAME) FROM performance_schema.metadata_locks WHERE OBJECT_SCHEMA='sf_tls' AND OBJECT_NAME IN ('items','healthy') AND LOCK_TYPE='EXCLUSIVE' AND LOCK_STATUS='PENDING'");
            if pending == "2" {
                break;
            }
            assert!(
                Instant::now() < until,
                "both DDL operations must wait on the request lease"
            );
            thread::sleep(Duration::from_millis(20));
        }
        exact_old(stream, wire);
        first.join().unwrap();
        second.join().unwrap();
    });
    stopped(database, id);
    stop(&mut server);
    partial_open_deadline(fixture, database);
    fixture.write("first.ttl", mapping);
    database.sql("DROP TABLE sf_tls.healthy; ALTER TABLE sf_tls.items DROP COLUMN released");

    for mode in ["disconnect", "deadline", "shutdown"] {
        eprintln!("MySQL protected native stop: {mode}");
        fill(database);
        let (mut command, address) = profile(fixture, database);
        command.args([
            "--max-source-work",
            "100000000",
            "--timeout-secs",
            if mode == "deadline" { "2" } else { "30" },
        ]);
        let mut server = start(fixture, command, address);
        let (stream, _) = held(address, &fixture.token);
        let id = owner(database);
        match mode {
            "disconnect" => {
                stream.shutdown(Shutdown::Both).unwrap();
                drop(stream);
            }
            "shutdown" => {
                stop(&mut server);
                drop(stream);
            }
            _ => {
                stopped(database, id);
                drop(stream);
            }
        }
        stopped(database, id);
        reset(database);
        if mode != "shutdown" {
            await_ready(&mut server, address, 200);
            value(address, &fixture.token, "same");
            value(address, &fixture.token, "same");
            stop(&mut server);
        }
    }
}

fn partial_open_deadline(fixture: &Fixture, database: &Database) {
    let (mut command, address) = profile_interval(fixture, database, "60");
    command.args(["--timeout-secs", "2"]);
    let mut server = start(fixture, command, address);
    let mut locker = database.hold_table("items");
    let stream = cancellation::begin(address, SINGLE, &fixture.token);
    let until = Instant::now() + Duration::from_secs(2);
    let id = loop {
        let rows = database.sql("SELECT DISTINCT t.PROCESSLIST_ID FROM performance_schema.threads t JOIN performance_schema.metadata_locks first ON first.OWNER_THREAD_ID=t.THREAD_ID JOIN performance_schema.metadata_locks second ON second.OWNER_THREAD_ID=t.THREAD_ID WHERE t.PROCESSLIST_USER='sf_tls' AND first.OBJECT_SCHEMA='sf_tls' AND first.OBJECT_NAME='healthy' AND first.LOCK_TYPE='SHARED_READ' AND first.LOCK_STATUS='GRANTED' AND second.OBJECT_SCHEMA='sf_tls' AND second.OBJECT_NAME='items' AND second.LOCK_TYPE='SHARED_READ' AND second.LOCK_STATUS='PENDING'");
        if let Ok(id) = rows.parse::<u64>() {
            break id;
        }
        assert!(
            Instant::now() < until,
            "missing request partial-MDL barrier"
        );
        thread::sleep(Duration::from_millis(20));
    };
    let mut wire = Vec::new();
    stream.take(8192).read_to_end(&mut wire).unwrap();
    assert_eq!(decode_response(wire).0, 504);
    stopped(database, id);
    locker.assert_held();
    drop(locker);
    reset(database);
    value(address, &fixture.token, "same");
    stop(&mut server);
}
