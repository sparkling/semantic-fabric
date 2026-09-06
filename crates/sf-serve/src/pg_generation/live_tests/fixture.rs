//! Disposable PostgreSQL fixture shared by required-live generation tests.

use super::*;

const READER_PASSWORD: &str = "semantic-fabric-generation-test-only";
static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) struct Fixture {
    root_config: Config,
    root: Client,
    pub(super) admin: Client,
    pub(super) pool: deadpool_postgres::Pool,
    pub(super) database: String,
    pub(super) role: String,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct BackendIdentity {
    pub(super) pid: i32,
    pub(super) started_at: String,
}

impl Fixture {
    pub(super) async fn create(root_config: Config) -> Self {
        let suffix = FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let database = format!("sf_generation_{}_{}", std::process::id(), suffix);
        let role = format!("sf_generation_reader_{}_{}", std::process::id(), suffix);
        assert!(database
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'));
        assert!(role
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'));
        assert!(database.len() <= 63 && role.len() <= 63);

        let root = connect(root_config.clone()).await;
        cleanup_objects(&root, &database, &role)
            .await
            .expect("remove stale generation-test objects");
        let provisioned = async {
            root.batch_execute(&format!(
                "CREATE ROLE {role} LOGIN PASSWORD '{READER_PASSWORD}' \
                 NOSUPERUSER NOINHERIT NOCREATEDB NOCREATEROLE \
                 NOREPLICATION NOBYPASSRLS"
            ))
            .await
            .map_err(|error| error.to_string())?;
            root.batch_execute(&format!("CREATE DATABASE {database}"))
                .await
                .map_err(|error| error.to_string())?;
            root.batch_execute(&format!(
                "REVOKE ALL ON DATABASE {database} FROM PUBLIC; \
                 REVOKE ALL ON DATABASE {database} FROM {role}; \
                 GRANT CONNECT ON DATABASE {database} TO {role};"
            ))
            .await
            .map_err(|error| error.to_string())?;

            let mut database_config = root_config.clone();
            database_config.dbname(&database);
            let admin = connect_result(database_config.clone()).await?;
            admin
                .batch_execute(&format!(
                    "REVOKE ALL ON SCHEMA public FROM PUBLIC; \
                 REVOKE ALL ON SCHEMA public FROM {role}; \
                 CREATE TABLE public.parent (id integer PRIMARY KEY, label text NOT NULL); \
                 CREATE TABLE public.child ( \
                   id integer PRIMARY KEY, parent_id integer NOT NULL, \
                   alternate_parent_id integer, label text NOT NULL, \
                   CONSTRAINT child_parent_fk FOREIGN KEY (parent_id) \
                     REFERENCES public.parent(id) \
                 ); \
                 INSERT INTO public.parent VALUES (1, 'parent'); \
                 INSERT INTO public.child VALUES (1, 1, 1, 'child'); \
                 REVOKE ALL ON ALL TABLES IN SCHEMA public FROM PUBLIC; \
                 REVOKE ALL ON ALL TABLES IN SCHEMA public FROM {role}; \
                 GRANT USAGE ON SCHEMA public TO {role}; \
                 GRANT SELECT ON ALL TABLES IN SCHEMA public TO {role}; \
                 GRANT EXECUTE ON FUNCTION \
                   pg_catalog.pg_database_collation_actual_version(oid) TO {role};"
                ))
                .await
                .map_err(|error| error.to_string())?;

            let mut reader_config = database_config;
            reader_config.user(&role);
            reader_config.password(READER_PASSWORD);
            reader_config.options(POSTGRES_RELATION_SCOPE_OPTIONS);
            let manager = deadpool_postgres::Manager::from_config(
                reader_config,
                NoTls,
                deadpool_postgres::ManagerConfig {
                    recycling_method: deadpool_postgres::RecyclingMethod::Custom(
                        crate::source::POSTGRES_RELATION_SCOPE_RECYCLE_SQL.to_owned(),
                    ),
                },
            );
            let pool = deadpool_postgres::Pool::builder(manager)
                .max_size(1)
                .wait_timeout(Some(Duration::from_secs(2)))
                .runtime(deadpool_postgres::Runtime::Tokio1)
                .build()
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((admin, pool))
        }
        .await;
        let (admin, pool) = match provisioned {
            Ok(provisioned) => provisioned,
            Err(error) => {
                let cleanup = cleanup_objects(&root, &database, &role).await;
                panic!("provision isolated generation-test fixture: {error}; cleanup={cleanup:?}")
            }
        };

        Self {
            root_config,
            root,
            admin,
            pool,
            database,
            role,
        }
    }

    pub(super) async fn generation(
        &self,
    ) -> (Backend, SourceGeneration, RuntimeBindingIdentity, SourceId) {
        let source_id = SourceId::new(0).expect("source zero");
        let unlocked = open_generation_before_lock_for_test(&self.pool, source_id, &budget())
            .await
            .expect("open generation before its first lock");
        let snapshot_state = self
            .admin
            .query_one(
                "SELECT state::text AS state, backend_xmin IS NULL AS no_snapshot \
                 FROM pg_catalog.pg_stat_activity WHERE pid = $1",
                &[&unlocked.backend_pid()],
            )
            .await
            .expect("inspect pre-lock generation backend");
        assert_eq!(
            snapshot_state.get::<_, String>("state"),
            "idle in transaction"
        );
        assert!(snapshot_state.get::<_, bool>("no_snapshot"));
        unlocked
            .finish()
            .await
            .expect("rollback pre-lock generation probe");
        let mut probe = self
            .pool
            .get()
            .await
            .expect("acquire isolated observation probe");
        let snapshot = sf_sql::introspect::introspect_postgres_public_observed_snapshot(&mut probe)
            .await
            .expect("collect isolated PostgreSQL observation");
        assert!(
            snapshot.direct_mapping_tables().is_some(),
            "isolated PostgreSQL profile unavailable: {:?}",
            snapshot.availability().unavailable_reason()
        );
        drop(probe);
        let source = IntrospectedSource::observe_postgres(self.pool.clone())
            .await
            .expect("observe isolated PostgreSQL");
        let discovered_names = normalized_table_names(source.observed_schema())
            .expect("normalize isolated relation set");
        let observed =
            open_observed_generation(&self.pool, source_id, &discovered_names, &budget())
                .await
                .expect("open and observe isolated generation");
        let expected_mapping = sf_mapping::direct_mapping_for_source_with_row_identity(
            observed.tables(),
            "http://example.test/base/",
            source_id,
            sf_mapping::DirectMappingRowIdentity::RequirePrimaryKey,
        )
        .expect("generate exact expected Direct Mapping");
        let expected = Arc::new(PostgresDirectGeneration {
            source_id,
            base_iri: Arc::from("http://example.test/base/"),
            row_identity: sf_mapping::DirectMappingRowIdentity::RequirePrimaryKey,
            mapping_digest: MappingDigest::from_mapping(&expected_mapping),
            identity: observed.identity(),
            session: observed.session().clone(),
            tables: observed.tables().to_vec().into(),
        });
        observed
            .promote_expected(expected)
            .await
            .expect("promote exact isolated generation")
            .finish_bounded(&budget())
            .await
            .expect("revalidate exact isolated generation");
        let (source, mapping) = build_and_bind_direct_candidate(
            source,
            "http://example.test/base/",
            source_id,
            &budget(),
        )
        .await
        .expect("build verified Direct-Mapping candidate");
        assert!(!mapping.is_empty());
        let (backend, tables, observation, generation) = source.into_parts();
        assert_eq!(tables.len(), 2);
        let bound = observation.bind(BackendKind::Postgres, source_id);
        assert!(bound.is_available());
        (
            backend,
            generation,
            RuntimeBindingIdentity::fresh(),
            source_id,
        )
    }

    pub(super) async fn cleanup(self) {
        drop(self.pool);
        drop(self.admin);
        let mut cleanup_config = self.root_config.clone();
        if cleanup_config.get_dbname() == Some(self.database.as_str()) {
            cleanup_config.dbname("postgres");
        }
        let cleanup = connect(cleanup_config).await;
        cleanup_objects(&cleanup, &self.database, &self.role)
            .await
            .expect("drop isolated generation-test objects");
        drop(self.root);
    }
}

async fn cleanup_objects(root: &Client, database: &str, role: &str) -> Result<(), String> {
    let database_result = root
        .batch_execute(&format!("DROP DATABASE IF EXISTS {database} WITH (FORCE)"))
        .await;
    let role_result = root
        .batch_execute(&format!("DROP ROLE IF EXISTS {role}"))
        .await;
    match (database_result, role_result) {
        (Ok(()), Ok(())) => Ok(()),
        (database, role) => Err(format!("database={database:?}; role={role:?}")),
    }
}

pub(super) fn budget() -> RequestBudget {
    RequestBudget::after(
        Duration::from_secs(20),
        QueryLimits::new(1_000, 1_000, 1_000, 1_000_000),
    )
}

async fn connect(config: Config) -> Client {
    connect_result(config)
        .await
        .expect("connect test PostgreSQL")
}

async fn connect_result(config: Config) -> Result<Client, String> {
    let (client, connection) = config
        .connect(NoTls)
        .await
        .map_err(|error| error.to_string())?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok(client)
}

pub(super) async fn backend_identity(client: &Client) -> BackendIdentity {
    let row = client
        .query_one(
            "SELECT pg_catalog.pg_backend_pid() AS pid, backend_start::text AS started_at \
             FROM pg_catalog.pg_stat_activity \
             WHERE pid = pg_catalog.pg_backend_pid()",
            &[],
        )
        .await
        .expect("capture backend identity");
    BackendIdentity {
        pid: row.get("pid"),
        started_at: row.get("started_at"),
    }
}

pub(super) async fn wait_for_backend(fixture: &Fixture, pid: i32, sleeping: bool) {
    for _ in 0..500 {
        let reached: bool = fixture
            .admin
            .query_one(
                "SELECT CASE WHEN $2 THEN EXISTS ( \
                   SELECT 1 FROM pg_catalog.pg_stat_activity \
                   WHERE pid = $1 AND state = 'active' AND wait_event = 'PgSleep' \
                 ) ELSE NOT EXISTS ( \
                   SELECT 1 FROM pg_catalog.pg_stat_activity WHERE pid = $1 \
                 ) END",
                &[&pid, &sleeping],
            )
            .await
            .expect("inspect generation backend")
            .get(0);
        if reached {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("backend {pid} did not reach sleeping={sleeping}");
}

pub(super) async fn acquire(
    backend: &Backend,
    generation: &SourceGeneration,
    binding: &RuntimeBindingIdentity,
    source_id: SourceId,
) -> Result<VerifiedPostgresGenerationLease, PgGenerationError> {
    let requirement = generation
        .requirement(backend, binding)?
        .expect("verified source has one requirement");
    let mut leases = VerifiedGenerationLeases::acquire(vec![requirement], &budget()).await?;
    Ok(leases
        .take(source_id, binding)
        .expect("binding-matched generation lease"))
}
