//! Owned, disposable providers only: no externally supplied database endpoint.
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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

    fn certificates(&self, prefix: &str) -> String {
        use rcgen::{
            BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
            KeyUsagePurpose,
        };
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
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

impl Database {
    pub fn start(fixture: &Fixture, postgres: bool) -> Self {
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
            if postgres { POSTGRES } else { MYSQL },
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
