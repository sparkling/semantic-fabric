//! Bounded interactive psql session inside the exact disposable TLS fixture.
use super::*;
use std::io::Read;
use std::sync::mpsc::{self, Receiver};

pub(super) struct Session {
    child: Child,
    lines: Receiver<String>,
    errors: Receiver<String>,
    sequence: usize,
}

impl Session {
    pub(super) fn new(database: &Database) -> Self {
        let mut child = Command::new("docker")
            .args([
                "--host",
                "unix:///var/run/docker.sock",
                "exec",
                "-i",
                "--env",
            ])
            .arg(format!("PGPASSWORD={}", database.password))
            .args([
                "--env",
                "PGSSLMODE=verify-full",
                "--env",
                "PGSSLROOTCERT=/sf-fixture/pg-ca.crt",
                &database.id,
                "psql",
                "-XAtq",
                "-h",
                "127.0.0.1",
                "-U",
                "sf_tls",
                "-d",
                "postgres",
                "-v",
                "ON_ERROR_STOP=1",
            ])
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

    pub(super) fn stage(&mut self, sql: &str) -> Vec<String> {
        self.sequence += 1;
        let marker = format!("sf-pg-authority-{}", self.sequence);
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{sql}; SELECT '{marker}';").unwrap();
        stdin.flush().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut rows = Vec::new();
        loop {
            let line = self
                .lines
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| {
                    let diagnostic = self.errors.recv_timeout(Duration::from_millis(100));
                    panic!("native PostgreSQL stage {marker} failed: {error}; {diagnostic:?}")
                });
            if line == marker {
                return rows;
            }
            assert!(rows.len() < 32, "unexpected native fixture output");
            rows.push(line);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(mut stdin) = self.child.stdin.take() {
            let _ = writeln!(stdin, "ROLLBACK;\\q");
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
