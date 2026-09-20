//! Native ordering prerequisite, not evidence of product generation admission.
use super::*;
use std::io::Read;
use std::sync::mpsc::{self, Receiver};

struct Session {
    child: Child,
    lines: Receiver<String>,
    errors: Receiver<String>,
    sequence: usize,
}

impl Session {
    fn new(database: &Database, user: &str) -> Self {
        assert!(matches!(user, "root" | "sf_tls"));
        let mut child = Command::new("docker")
            .args([
                "--host",
                "unix:///var/run/docker.sock",
                "exec",
                "-i",
                "--env",
            ])
            .arg(format!("MYSQL_PWD={}", database.password))
            .args([
                &database.id,
                "mysql",
                "--host=127.0.0.1",
                "--ssl-mode=VERIFY_IDENTITY",
                "--ssl-ca=/sf-fixture/mysql-ca.crt",
                "--batch",
                "--raw",
                "--skip-column-names",
                "--unbuffered",
                "--database=sf_tls",
            ])
            .arg(format!("--user={user}"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let password = database.password.clone();
        let (error_sender, errors) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut message = String::new();
            let _ = stderr.take(8192).read_to_string(&mut message);
            let _ = error_sender.send(message.replace(&password, "[fixture-secret]"));
        });
        let (sender, lines) = mpsc::sync_channel(32);
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            lines,
            errors,
            sequence: 0,
        }
    }

    fn send(&mut self, sql: &str) -> String {
        self.sequence += 1;
        let marker = format!("sf-ordering-{}", self.sequence);
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{sql}; SELECT '{marker}';").unwrap();
        stdin.flush().unwrap();
        marker
    }

    fn receive(&self, marker: &str) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut rows = Vec::new();
        loop {
            let line = self
                .lines
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| {
                    let diagnostic = self.errors.recv_timeout(Duration::from_millis(100));
                    panic!("native ordering stage {marker} failed: {error}; {diagnostic:?}")
                });
            if line == marker {
                return rows;
            }
            assert!(rows.len() < 16, "unexpected native fixture output");
            rows.push(line);
        }
    }

    fn stage(&mut self, sql: &str) -> Vec<String> {
        let marker = self.send(sql);
        self.receive(&marker)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(mut stdin) = self.child.stdin.take() {
            let _ = writeln!(stdin, "ROLLBACK; UNLOCK TABLES;");
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Database owns the exact disposable container, including a client
        // blocked on native MDL if this test unwinds before releasing its peer.
    }
}

fn lock_rows(database: &Database, session: u64) -> String {
    database.sql(&format!(
        "SELECT m.OBJECT_NAME,m.LOCK_TYPE,m.LOCK_STATUS \
         FROM performance_schema.metadata_locks m \
         JOIN performance_schema.threads t ON t.THREAD_ID=m.OWNER_THREAD_ID \
         WHERE t.PROCESSLIST_ID={session} AND m.OBJECT_SCHEMA='sf_tls' \
         AND m.OBJECT_TYPE='TABLE' ORDER BY m.OBJECT_NAME,m.LOCK_STATUS"
    ))
}

fn wait_for_locks(database: &Database, session: u64, expected: &[String]) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let actual = lock_rows(database, session);
        if expected
            .iter()
            .all(|line| actual.lines().any(|row| row == line))
        {
            return;
        }
        assert!(Instant::now() < deadline, "missing native MDL: {actual}");
        thread::sleep(Duration::from_millis(25));
    }
}

fn session_id(session: &mut Session) -> u64 {
    let rows = session.stage("SELECT CONNECTION_ID()");
    assert_eq!(rows.len(), 1);
    rows[0].parse().unwrap()
}

fn ordering_case(database: &Database, first: &str, second: &str, early_snapshot: bool) {
    assert!(matches!((first, second), ("a", "b") | ("b", "a")));
    database.sql(
        "DROP TABLE IF EXISTS sf_tls.a,sf_tls.b; \
        CREATE TABLE sf_tls.a(value INT NOT NULL) ENGINE=InnoDB; \
        CREATE TABLE sf_tls.b(value INT NOT NULL) ENGINE=InnoDB; \
        INSERT INTO sf_tls.a VALUES(7); INSERT INTO sf_tls.b VALUES(8)",
    );
    let original = database.sql(&format!("SELECT value FROM sf_tls.{first}"));
    let mut locker = Session::new(database, "root");
    assert!(locker
        .stage(&format!("LOCK TABLES {second} WRITE"))
        .is_empty());
    let mut reader = Session::new(database, "sf_tls");
    let id = session_id(&mut reader);
    database.assert_encrypted_sessions();
    let start = if early_snapshot {
        "START TRANSACTION WITH CONSISTENT SNAPSHOT"
    } else {
        "START TRANSACTION"
    };
    assert!(reader
        .stage(&format!(
            "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ; \
            SET TRANSACTION READ ONLY; {start}"
        ))
        .is_empty());
    let opened = reader.send(&format!(
        "SELECT 1 FROM {first} WHERE FALSE UNION ALL SELECT 1 FROM {second} WHERE FALSE"
    ));
    wait_for_locks(
        database,
        id,
        &[
            format!("{first}\tSHARED_READ\tGRANTED"),
            format!("{second}\tSHARED_READ\tPENDING"),
        ],
    );
    database.sql(&format!("UPDATE sf_tls.{first} SET value=value+100"));
    let updated = database.sql(&format!("SELECT value FROM sf_tls.{first}"));
    assert_ne!(original, updated);
    assert!(locker
        .stage(&format!(
            "ALTER TABLE {second} ADD COLUMN changed INT; UNLOCK TABLES"
        ))
        .is_empty());
    assert!(reader.receive(&opened).is_empty());
    assert_eq!(
        lock_rows(database, id),
        "a\tSHARED_READ\tGRANTED\nb\tSHARED_READ\tGRANTED"
    );

    // No application-table read precedes this point. The early-snapshot
    // control sees the old value, proving the partial-open barrier matters.
    let seen = reader.stage(&format!("SELECT value FROM {first}"));
    assert_eq!(seen, [if early_snapshot { original } else { updated }]);
    // MySQL's transactional data dictionary is stale too in the negative
    // control: taking the view early can miss the DDL committed while opening.
    let expected_columns: &[&str] = if early_snapshot {
        &["value"]
    } else {
        &["value", "changed"]
    };
    assert_eq!(
        reader.stage(&format!(
            "SELECT COLUMN_NAME FROM information_schema.COLUMNS \
        WHERE TABLE_SCHEMA='sf_tls' AND TABLE_NAME='{second}' ORDER BY ORDINAL_POSITION"
        )),
        expected_columns
    );
    database.sql(&format!("UPDATE sf_tls.{first} SET value=value+100"));
    assert_eq!(reader.stage(&format!("SELECT value FROM {first}")), seen);

    let mut ddl_a = Session::new(database, "root");
    let mut ddl_b = Session::new(database, "root");
    let ddl_a_id = session_id(&mut ddl_a);
    let ddl_b_id = session_id(&mut ddl_b);
    let a_done = ddl_a.send("ALTER TABLE a ADD COLUMN after_release INT");
    let b_done = ddl_b.send("ALTER TABLE b ADD COLUMN after_release INT");
    wait_for_locks(database, ddl_a_id, &["a\tEXCLUSIVE\tPENDING".into()]);
    wait_for_locks(database, ddl_b_id, &["b\tEXCLUSIVE\tPENDING".into()]);
    assert!(reader.stage("ROLLBACK").is_empty());
    assert!(ddl_a.receive(&a_done).is_empty());
    assert!(ddl_b.receive(&b_done).is_empty());
    assert!(lock_rows(database, id).is_empty());
    eprintln!("MySQL8.4.11 ordering {first}->{second}, early_snapshot={early_snapshot}: witnessed partial MDL, snapshot value, repeatability and rollback release");
}

#[test]
#[ignore = "requires Docker and the pinned disposable MySQL image; required in CI"]
fn all_table_locks_precede_the_read_view() {
    let fixture = Fixture::new();
    let database = Database::start(&fixture, false);
    assert_eq!(database.sql("SELECT VERSION()"), "8.4.11");
    for early_snapshot in [false, true] {
        for (first, second) in [("a", "b"), ("b", "a")] {
            ordering_case(&database, first, second, early_snapshot);
        }
    }
}
