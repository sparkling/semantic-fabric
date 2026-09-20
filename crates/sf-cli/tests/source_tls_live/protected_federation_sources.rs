use super::*;

pub(super) struct Endpoint {
    // Native provider must stop before its certificate directory is removed.
    pub(super) database: Option<Database>,
    fixture: Fixture,
    pub(super) profile: &'static str,
}

impl Endpoint {
    pub(super) fn new(profile: &'static str) -> Self {
        let fixture = Fixture::new();
        let database = match profile {
            "16.9" | "16.15" => Some(Database::postgres_patch(&fixture, profile)),
            "mysql" => Some(Database::start(&fixture, false)),
            "WAL" | "DELETE" => None,
            _ => unreachable!(),
        };
        let endpoint = Self {
            database,
            fixture,
            profile,
        };
        if profile == "mysql" {
            endpoint.sql("ALTER TABLE items ADD COLUMN tenant VARCHAR(16) NOT NULL DEFAULT 'a'");
        } else if endpoint.database.is_some() {
            endpoint.sql("ALTER TABLE items ADD COLUMN tenant text NOT NULL DEFAULT 'a'");
            endpoint.database.as_ref().unwrap().sql("REVOKE ALL ON DATABASE postgres FROM PUBLIC, sf_tls; GRANT CONNECT ON DATABASE postgres TO sf_tls; REVOKE ALL ON SCHEMA public FROM PUBLIC, sf_tls; GRANT USAGE ON SCHEMA public TO sf_tls; REVOKE ALL ON ALL TABLES IN SCHEMA public FROM PUBLIC, sf_tls; GRANT SELECT ON ALL TABLES IN SCHEMA public TO sf_tls; GRANT EXECUTE ON FUNCTION pg_catalog.pg_database_collation_actual_version(oid) TO sf_tls");
        } else {
            endpoint.sql(&format!("PRAGMA journal_mode={profile}; CREATE TABLE items(value TEXT NOT NULL, tenant TEXT NOT NULL)"));
        }
        endpoint.sql("ALTER TABLE items ADD COLUMN successor TEXT");
        endpoint.reset();
        endpoint
    }

    pub(super) fn reset(&self) {
        self.sql(
            "DELETE FROM items; INSERT INTO items(value,tenant) VALUES('same','a'),('same-b','b')",
        );
    }

    pub(super) fn sql(&self, sql: &str) {
        match &self.database {
            Some(database) => {
                let prefix = if self.profile == "mysql" {
                    "sf_tls"
                } else {
                    "public"
                };
                database.sql(&sql.replace("items", &format!("{prefix}.items")));
            }
            None => {
                rusqlite::Connection::open(self.fixture.root.join("source.db"))
                    .unwrap()
                    .execute_batch(sql)
                    .unwrap();
            }
        }
    }

    pub(super) fn apply(&self, command: &mut Command, slot: usize) {
        let suffix = if slot == 0 { "" } else { "-2" };
        let env = format!("SF_PAIR_SOURCE_{slot}");
        command.arg(format!("--source-env{suffix}")).arg(&env);
        match &self.database {
            Some(database) => {
                let source = if self.profile == "mysql" {
                    format!("{}?pool_min=1&pool_max=1", database.source)
                } else {
                    database.source.clone()
                };
                let roots = format!("SF_PAIR_ROOTS_{slot}");
                command
                    .env(env, source)
                    .arg(format!("--source-tls-roots-env{suffix}"))
                    .arg(&roots)
                    .env(roots, &database.roots);
            }
            None => {
                command.env(
                    env,
                    format!("sqlite:{}", self.fixture.root.join("source.db").display()),
                );
            }
        }
    }

    pub(super) fn encrypted(&self) {
        if let Some(database) = &self.database {
            if self.profile == "mysql" {
                // Completed protected requests may discard their connection;
                // successful queries must still satisfy the server's TLS rule.
                assert_eq!(
                    database
                        .sql("SELECT ssl_type FROM mysql.user WHERE User='sf_tls' AND Host='%'"),
                    "ANY"
                );
            } else {
                assert_eq!(
                    database.sql("SELECT type || ':' || auth_method FROM pg_hba_file_rules WHERE type <> 'local' ORDER BY rule_number"),
                    "hostssl:scram-sha-256\nhost:reject\nhostssl:scram-sha-256\nhost:reject"
                );
            }
        }
    }

    pub(super) fn fill_stream(&self) {
        let fill = match self.profile {
            "mysql" => "SET SESSION cte_max_recursion_depth=1100; INSERT INTO items(value,tenant) WITH RECURSIVE n AS (SELECT 1 AS n UNION ALL SELECT n+1 FROM n WHERE n<1024) SELECT CONCAT(REPEAT('x',32760),LPAD(n,8,'0')),'a' FROM n",
            "16.9" | "16.15" => "INSERT INTO items(value,tenant) SELECT repeat('x',32760)||lpad(g::text,8,'0'),'a' FROM generate_series(1,1024) g",
            _ => "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1024) INSERT INTO items(value,tenant) SELECT printf('%.*c%08d',32760,120,x),'a' FROM n",
        };
        self.sql(&format!("DELETE FROM items; {fill}"));
    }

    pub(super) fn prepare_lease_witness(&self) {
        if self.database.is_none() {
            assert_eq!(self.profile, "WAL");
            self.sql("INSERT INTO items(value,tenant) VALUES('post-snapshot','a')");
        }
    }

    pub(super) fn await_lease(&self, held: bool) {
        let until = Instant::now() + Duration::from_secs(4);
        loop {
            let found = if let Some(database) = &self.database {
                let sql = if self.profile == "mysql" {
                    "SELECT COUNT(DISTINCT th.PROCESSLIST_ID) FROM performance_schema.threads th JOIN performance_schema.metadata_locks ml ON ml.OWNER_THREAD_ID=th.THREAD_ID WHERE th.PROCESSLIST_USER='sf_tls' AND ml.OBJECT_TYPE='TABLE' AND ml.OBJECT_SCHEMA='sf_tls' AND ml.OBJECT_NAME='items' AND ml.LOCK_TYPE='SHARED_READ' AND ml.LOCK_STATUS='GRANTED'"
                } else {
                    "SELECT count(DISTINCT a.pid) FROM pg_stat_activity a JOIN pg_locks l ON l.pid=a.pid WHERE a.usename='sf_tls' AND a.xact_start IS NOT NULL AND l.locktype='relation' AND l.relation='public.items'::regclass AND l.mode='AccessShareLock' AND l.granted"
                };
                let count = database.sql(sql).parse::<u32>().unwrap();
                if held && count == 1 {
                    database.assert_encrypted_sessions();
                }
                count
            } else {
                let connection =
                    rusqlite::Connection::open(self.fixture.root.join("source.db")).unwrap();
                connection.busy_timeout(Duration::ZERO).unwrap();
                connection
                    .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                        row.get::<_, u32>(0)
                    })
                    .unwrap()
            };
            if found == u32::from(held) {
                return;
            }
            assert!(
                Instant::now() < until,
                "{} lease held={held}, observed {found}",
                self.profile
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}
