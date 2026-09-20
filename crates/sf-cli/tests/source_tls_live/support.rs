//! Owned, disposable providers only: no externally supplied database endpoint.
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[path = "mysql_ordering.rs"]
mod mysql_ordering;

const POSTGRES: &str =
    "postgres@sha256:485935f94cc7165afa896978809c37b592dc07f0a37d2c8f645f12412d0212c8";
const MYSQL: &str = "mysql@sha256:1d6b6a8fcee8ff758ff151d017f5203cd06792a0e698f0a593c9dfcb14609cf0";

pub fn output(command: &mut Command, bound: Duration) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + bound;
    let timed_out = loop {
        if child.try_wait().unwrap().is_some() {
            break false;
        }
        if Instant::now() >= deadline {
            break true;
        }
        thread::sleep(Duration::from_millis(20));
    };
    if timed_out {
        let _ = child.kill();
    }
    let result = child.wait_with_output().unwrap();
    assert!(!timed_out, "fixture command exceeded its deadline");
    result
}

fn docker(arguments: &[&str]) -> Output {
    output(
        Command::new("docker")
            .args(["--host", "unix:///var/run/docker.sock"])
            .args(arguments),
        Duration::from_secs(60),
    )
}

fn success(result: Output) -> String {
    assert!(
        result.status.success(),
        "fixture command failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap().trim().to_owned()
}

pub struct Fixture {
    pub root: PathBuf,
    pub token: String,
}

impl Fixture {
    pub fn new() -> Self {
        let root = std::env::temp_dir().join(format!("sf-source-tls-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            root,
            token: uuid::Uuid::new_v4().simple().to_string(),
        }
    }

    pub fn write(&self, name: &str, content: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::write(&path, content).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        path
    }

    pub(super) fn certificates(&self, prefix: &str) -> String {
        use rcgen::{
            BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
            KeyUsagePurpose,
        };
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        // OpenSSL distinguishes this issuer from the leaf by subject name.
        params.distinguished_name.push(
            rcgen::DnType::CommonName,
            format!("semantic-fabric-{prefix}-fixture-ca"),
        );
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let key = KeyPair::generate().unwrap();
        let ca = params.self_signed(&key).unwrap().pem();
        let issuer = Issuer::new(params, key);
        let mut leaf = CertificateParams::new(vec!["127.0.0.1".into()]).unwrap();
        leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let key = KeyPair::generate().unwrap();
        self.write(
            &format!("{prefix}.crt"),
            &leaf.signed_by(&key, &issuer).unwrap().pem(),
        );
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.root.join(format!("{prefix}.key")))
            .unwrap()
            .write_all(key.serialize_pem().as_bytes())
            .unwrap();
        self.write(&format!("{prefix}-ca.crt"), &ca);
        ca
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // This uniquely created directory holds only this test's generated files.
        if let Ok(entries) = fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                let _ = fs::remove_file(entry.path());
            }
        }
        let _ = fs::remove_dir(&self.root);
    }
}

pub struct Database {
    id: String,
    pub source: String,
    pub roots: String,
    postgres: bool,
    password: String,
}

/// Own one interactive, test-only lock session until the assertion releases it.
pub struct TableLock<'a> {
    process: Child,
    release: &'static str,
    database: &'a Database,
    table: String,
    session: u64,
}
impl TableLock<'_> {
    pub fn assert_held(&mut self) {
        assert!(
            self.process.try_wait().unwrap().is_none(),
            "fixture lock owner exited"
        );
        let query = if self.database.postgres {
            format!("SELECT count(*) FROM pg_locks WHERE pid={} AND relation='public.{}'::regclass AND mode='AccessExclusiveLock' AND granted", self.session, self.table)
        } else {
            format!("SELECT count(*) FROM performance_schema.metadata_locks l JOIN performance_schema.threads t ON l.OWNER_THREAD_ID=t.THREAD_ID WHERE t.PROCESSLIST_ID={} AND l.OBJECT_SCHEMA='sf_tls' AND l.OBJECT_NAME='{}' AND l.LOCK_STATUS='GRANTED' AND l.LOCK_TYPE IN ('SHARED_NO_READ_WRITE','EXCLUSIVE')", self.session, self.table)
        };
        assert!(
            self.database.sql(&query).parse::<usize>().unwrap() > 0,
            "native locker no longer holds the table"
        );
    }
}
impl Drop for TableLock<'_> {
    fn drop(&mut self) {
        if let Some(mut stdin) = self.process.stdin.take() {
            let _ = writeln!(stdin, "{}", self.release);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while matches!(self.process.try_wait(), Ok(None)) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

impl Database {
    pub fn hold_table(&self, table: &str) -> TableLock<'_> {
        // Closed fixture identifiers only; this cannot address external data.
        assert!(matches!(table, "items" | "healthy"));
        let mut command = Command::new("docker");
        command.args(["--host", "unix:///var/run/docker.sock", "exec", "-i"]);
        let (lock, release) = if self.postgres {
            command.args([
                &self.id,
                "psql",
                "-XAtq",
                "-U",
                "postgres",
                "-d",
                "postgres",
                "-v",
                "ON_ERROR_STOP=1",
            ]);
            (
                format!("BEGIN; LOCK TABLE public.{table} IN ACCESS EXCLUSIVE MODE;"),
                "ROLLBACK;",
            )
        } else {
            command.args([
                "--env",
                &format!("MYSQL_PWD={}", self.password),
                &self.id,
                "mysql",
                "--user=root",
                "--batch",
                "--raw",
                "--skip-column-names",
                "--unbuffered",
            ]);
            (
                format!("LOCK TABLES sf_tls.{table} WRITE;"),
                "UNLOCK TABLES;",
            )
        };
        let mut owned = TableLock {
            process: command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
            release,
            database: self,
            table: table.to_owned(),
            session: 0,
        };
        let output = owned.process.stdout.take().unwrap();
        let (sent, received) = std::sync::mpsc::sync_channel(1);
        thread::spawn(move || {
            let ready = BufReader::new(output).lines().take(8).find_map(|line| {
                line.ok()?
                    .strip_prefix("sf-lock-held:")?
                    .parse::<u64>()
                    .ok()
            });
            let _ = sent.send(ready);
        });
        writeln!(
            owned.process.stdin.as_mut().unwrap(),
            "{lock} SELECT {};",
            if self.postgres {
                "'sf-lock-held:' || pg_backend_pid()"
            } else {
                "CONCAT('sf-lock-held:', CONNECTION_ID())"
            }
        )
        .unwrap();
        owned.session = received
            .recv_timeout(Duration::from_secs(3))
            .expect("fixture lock deadline")
            .expect("fixture lock not acquired");
        assert!(owned.session > 0);
        owned.assert_held();
        owned
    }

    pub fn start(fixture: &Fixture, postgres: bool) -> Self {
        Self::start_image(fixture, postgres, POSTGRES)
    }

    pub fn postgres_patch(fixture: &Fixture, patch: &str) -> Self {
        let image = match patch {
            "16.15" => POSTGRES,
            "16.9" => {
                "postgres@sha256:ddfe3e8713e3ee5b8f286082cb12512488dfbf3f5a1ecb0b74a42e6055af0a5f"
            }
            _ => panic!("unqualified fixture patch"),
        };
        Self::start_image(fixture, true, image)
    }

    fn start_image(fixture: &Fixture, postgres: bool, postgres_image: &str) -> Self {
        let prefix = if postgres { "pg" } else { "mysql" };
        let roots = fixture.certificates(prefix);
        fixture.write("pg_hba.conf", "local all all trust\nhostssl all all 0.0.0.0/0 scram-sha-256\nhost all all 0.0.0.0/0 reject\nhostssl all all ::/0 scram-sha-256\nhost all all ::/0 reject\n");
        let password = uuid::Uuid::new_v4().simple().to_string();
        let name = format!("sf-source-tls-{prefix}-{}", uuid::Uuid::new_v4());
        let bind = format!(
            "type=bind,src={},dst=/sf-fixture,readonly",
            fixture.root.display()
        );
        let credential = if postgres {
            format!("POSTGRES_PASSWORD={password}")
        } else {
            format!("MYSQL_ROOT_PASSWORD={password}")
        };
        let entry = if postgres {
            "cp /sf-fixture/pg.key /tmp/sf-server.key && chown postgres /tmp/sf-server.key && chmod 600 /tmp/sf-server.key && exec docker-entrypoint.sh postgres -c ssl=on -c ssl_cert_file=/sf-fixture/pg.crt -c ssl_key_file=/tmp/sf-server.key -c hba_file=/sf-fixture/pg_hba.conf"
        } else {
            "cp /sf-fixture/mysql.key /tmp/sf-server.key && chown mysql /tmp/sf-server.key && chmod 600 /tmp/sf-server.key && exec docker-entrypoint.sh mysqld --ssl-ca=/sf-fixture/mysql-ca.crt --ssl-cert=/sf-fixture/mysql.crt --ssl-key=/tmp/sf-server.key --require-secure-transport=ON"
        };
        let id = success(docker(&[
            "create",
            "--name",
            &name,
            "--cpus=1",
            "--memory=1g",
            "--mount",
            &bind,
            "--tmpfs",
            if postgres {
                "/var/lib/postgresql/data:rw"
            } else {
                "/var/lib/mysql:rw"
            },
            "--publish",
            if postgres {
                "127.0.0.1::5432"
            } else {
                "127.0.0.1::3306"
            },
            "--env",
            &credential,
            "--entrypoint",
            "/bin/sh",
            if postgres { postgres_image } else { MYSQL },
            "-c",
            entry,
        ]));
        assert!(id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit()));
        // Own the exact returned ID before start; cleanup also covers failed startup.
        let mut database = Self {
            id,
            source: String::new(),
            roots,
            postgres,
            password,
        };
        success(docker(&["start", &database.id]));
        let port = success(docker(&[
            "port",
            &database.id,
            if postgres { "5432/tcp" } else { "3306/tcp" },
        ]));
        let port: u16 = port.strip_prefix("127.0.0.1:").unwrap().parse().unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            // TCP readiness excludes each image's temporary Unix-only init server.
            let ready = if postgres {
                docker(&[
                    "exec",
                    &database.id,
                    "pg_isready",
                    "-h",
                    "127.0.0.1",
                    "-U",
                    "postgres",
                ])
            } else {
                database.mysql("SELECT 1", true)
            };
            if ready.status.success() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "disposable TLS provider did not become ready"
            );
            thread::sleep(Duration::from_millis(200));
        }
        let password = &database.password;
        if postgres {
            database.sql(&format!("CREATE ROLE sf_tls LOGIN PASSWORD '{password}' NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT; CREATE TABLE public.items(value TEXT NOT NULL); INSERT INTO public.items VALUES ('same'); GRANT USAGE ON SCHEMA public TO sf_tls; GRANT SELECT ON public.items TO sf_tls;"));
            database.source = format!(
                "pg:host=127.0.0.1 port={port} user=sf_tls password={password} dbname=postgres"
            );
        } else {
            database.sql(&format!("CREATE DATABASE sf_tls; CREATE TABLE sf_tls.items(value TEXT NOT NULL); INSERT INTO sf_tls.items VALUES ('same'); CREATE USER 'sf_tls'@'%' IDENTIFIED BY '{password}' REQUIRE SSL; GRANT SELECT ON sf_tls.* TO 'sf_tls'@'%';"));
            assert_eq!(database.sql("SELECT @@require_secure_transport"), "1");
            database.source = format!("mysql://sf_tls:{password}@127.0.0.1:{port}/sf_tls");
        }
        database
    }

    fn mysql(&self, sql: &str, tcp: bool) -> Output {
        let password = format!("MYSQL_PWD={}", self.password);
        let mut arguments = vec![
            "exec",
            "--env",
            &password,
            &self.id,
            "mysql",
            "--user=root",
            "--batch",
            "--skip-column-names",
        ];
        if tcp {
            arguments.extend(["--host=127.0.0.1", "--ssl-mode=REQUIRED"]);
        }
        arguments.extend(["--execute", sql]);
        docker(&arguments)
    }

    pub fn sql(&self, sql: &str) -> String {
        success(if self.postgres {
            docker(&[
                "exec",
                &self.id,
                "psql",
                "-U",
                "postgres",
                "-d",
                "postgres",
                "-v",
                "ON_ERROR_STOP=1",
                "-Atc",
                sql,
            ])
        } else {
            self.mysql(sql, false)
        })
    }

    pub fn assert_encrypted_sessions(&self) {
        let count = if self.postgres {
            self.sql("SELECT count(*) FROM pg_stat_ssl s JOIN pg_stat_activity a ON a.pid=s.pid WHERE a.usename='sf_tls' AND s.ssl")
        } else {
            self.sql("SELECT count(*) FROM performance_schema.status_by_thread s JOIN performance_schema.threads t USING (THREAD_ID) WHERE t.PROCESSLIST_USER='sf_tls' AND s.VARIABLE_NAME='Ssl_cipher' AND s.VARIABLE_VALUE <> ''")
        };
        assert!(
            count.parse::<usize>().unwrap() > 0,
            "no encrypted application session observed"
        );
    }

    pub fn stop(&mut self) {
        success(docker(&["rm", "--force", &self.id]));
        self.id.clear();
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        if !self.id.is_empty() {
            let _ = docker(&["rm", "--force", &self.id]);
        }
    }
}

pub fn mapping(path: &Path, predicate: &str) {
    fs::write(
        path,
        format!(
            r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
<#items> a rr:TriplesMap ; rr:logicalTable [ rr:tableName "items" ] ;
rr:subjectMap [ rr:constant <http://example.test/item> ] ;
rr:predicateObjectMap [ rr:predicate <{predicate}> ; rr:objectMap [ rr:column "value" ] ] ."#
        ),
    )
    .unwrap();
}
