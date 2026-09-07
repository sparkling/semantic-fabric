//! Fixed, evidence-only PostgreSQL observation probe (ADR-0051).

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use serde::Serialize;
use sf_sql::introspect::{
    introspect_postgres_public_observed_snapshot_with_evidence, Postgres16PublicObservedSnapshotV1,
    PostgresObservationCommitV1, PostgresObservationEvidenceV1, PostgresObservationPhaseV1,
    PostgresObservationSavepointV1, PostgresObservationStreamEvidenceV1,
    PostgresObservationStreamTerminalV1, PostgresObservationStreamV1,
    POSTGRES_PROFILE_PREQUALIFICATION_QUERY_COUNT_V1,
    POSTGRES_RICH_CAPTURE_CATALOGUE_QUERY_COUNT_V1,
};
use tokio_postgres::{Client, Config, NoTls, Row};

const SOCKET: &str = "/var/run/postgresql";
const DATABASE: &str = "sf_observation_qualification_v1";
const OWNER: &str = "sf_observation_owner_v1";
const OBSERVER: &str = "sf_observation_observer_v1";
const APPLICATION: &str = "semantic-fabric-pg-observation-qualification-v1";
const MAX_OUTPUT_BYTES: usize = 262_144;
const QUALIFICATION_FAILURE_EXIT: u8 = 78;

const ROLE_PREFLIGHT_SQL: &str = r#"
WITH role_facts AS (
  SELECT r.oid AS role_oid, r.rolsuper, r.rolinherit, r.rolbypassrls,
         r.rolcreaterole, r.rolcreatedb, r.rolreplication,
         d.datdba = r.oid AS database_owner
  FROM pg_catalog.pg_roles r
  JOIN pg_catalog.pg_database d ON d.datname = pg_catalog.current_database()
  WHERE r.rolname = CURRENT_USER
), public_tables AS (
  SELECT c.oid, c.relowner
  FROM pg_catalog.pg_class c
  JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
  WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p')
), table_acl AS (
  SELECT COALESCE(pg_catalog.bool_and(
    pg_catalog.has_table_privilege(CURRENT_USER, oid, 'SELECT')
    AND NOT pg_catalog.has_table_privilege(CURRENT_USER, oid, 'INSERT')
    AND NOT pg_catalog.has_table_privilege(CURRENT_USER, oid, 'UPDATE')
    AND NOT pg_catalog.has_table_privilege(CURRENT_USER, oid, 'DELETE')
    AND NOT pg_catalog.has_table_privilege(CURRENT_USER, oid, 'TRUNCATE')
    AND NOT pg_catalog.has_table_privilege(CURRENT_USER, oid, 'REFERENCES')
    AND NOT pg_catalog.has_table_privilege(CURRENT_USER, oid, 'TRIGGER')
  ), false) AS exact
  FROM public_tables
)
SELECT r.rolsuper, r.database_owner, r.rolinherit, r.rolbypassrls,
       pg_catalog.pg_has_role(CURRENT_USER, 'sf_observation_owner_v1', 'SET') AS can_set_role,
       (r.rolcreaterole OR r.rolcreatedb OR r.rolreplication
        OR pg_catalog.has_database_privilege(CURRENT_USER, pg_catalog.current_database(), 'CREATE')
        OR pg_catalog.has_database_privilege(CURRENT_USER, pg_catalog.current_database(), 'TEMPORARY')
        OR pg_catalog.has_schema_privilege(CURRENT_USER, 'public', 'CREATE')
        OR EXISTS (SELECT 1 FROM public_tables t WHERE t.relowner = r.role_oid)) AS can_ddl,
       (pg_catalog.has_database_privilege(CURRENT_USER, pg_catalog.current_database(), 'CONNECT')
        AND NOT pg_catalog.has_database_privilege(CURRENT_USER, pg_catalog.current_database(), 'CREATE')
        AND NOT pg_catalog.has_database_privilege(CURRENT_USER, pg_catalog.current_database(), 'TEMPORARY')
        AND pg_catalog.has_schema_privilege(CURRENT_USER, 'public', 'USAGE')
        AND NOT pg_catalog.has_schema_privilege(CURRENT_USER, 'public', 'CREATE')
        AND pg_catalog.has_function_privilege(
          CURRENT_USER, 'pg_catalog.pg_database_collation_actual_version(oid)', 'EXECUTE')
        AND NOT EXISTS (
          SELECT 1 FROM pg_catalog.pg_auth_members m WHERE m.member = r.role_oid)
        AND a.exact) AS privileges_exact
FROM role_facts r CROSS JOIN table_acl a
"#;

const ALL_STREAMS: [PostgresObservationStreamV1; 10] = [
    PostgresObservationStreamV1::LegacyTables,
    PostgresObservationStreamV1::LegacyEarlierCollisions,
    PostgresObservationStreamV1::LegacyColumns,
    PostgresObservationStreamV1::LegacyKeys,
    PostgresObservationStreamV1::LegacyForeignKeys,
    PostgresObservationStreamV1::LegacyRelationStatistics,
    PostgresObservationStreamV1::LegacyColumnStatistics,
    PostgresObservationStreamV1::RichRelations,
    PostgresObservationStreamV1::RichAttributes,
    PostgresObservationStreamV1::RichCatalogConstraints,
];

const ALL_PHASES: [PostgresObservationPhaseV1; 10] = [
    PostgresObservationPhaseV1::Guard,
    PostgresObservationPhaseV1::RelationsStream,
    PostgresObservationPhaseV1::AttributesStream,
    PostgresObservationPhaseV1::RelationNormalization,
    PostgresObservationPhaseV1::NotNullDerivation,
    PostgresObservationPhaseV1::ConstraintBudget,
    PostgresObservationPhaseV1::CatalogConstraintsStream,
    PostgresObservationPhaseV1::ConstraintNormalization,
    PostgresObservationPhaseV1::LegacyComparison,
    PostgresObservationPhaseV1::IdentityBuild,
];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProbeEnvelope {
    preflight: Preflight,
    observation: Observation,
    query_accounting: QueryAccounting,
    lifecycle: Lifecycle,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Preflight {
    network_mode: &'static str,
    fixed_database: bool,
    unix_socket: bool,
    comparison_role: ComparisonRole,
    owner_identity_equal: bool,
    legacy_constraint_visibility_differs: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
#[derive(Clone, Copy)]
struct ComparisonRole {
    superuser: bool,
    database_owner: bool,
    inherit: bool,
    bypass_rls: bool,
    can_set_role: bool,
    can_ddl: bool,
    privileges_exact: bool,
}

impl ComparisonRole {
    fn admitted(&self) -> bool {
        !self.superuser
            && !self.database_owner
            && !self.inherit
            && !self.bypass_rls
            && !self.can_set_role
            && !self.can_ddl
            && self.privileges_exact
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Observation {
    server_version: String,
    server_version_num: i32,
    guard: &'static str,
    row_counts: RowCounts,
    streaming: RichStreaming,
    identity: Identity,
    legacy_coordinate_comparison: &'static str,
    error_code: Option<&'static str>,
    failure_phase: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RowCounts {
    relations: usize,
    attributes: usize,
    not_null_constraints: usize,
    catalog_constraints: usize,
    combined_constraints: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RichStreaming {
    relations: StreamEvidence,
    attributes: StreamEvidence,
    catalog_constraints: StreamEvidence,
}

#[derive(Serialize)]
struct Identity {
    structural: String,
    types: String,
    constraints: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamEvidence {
    cap: usize,
    polled: usize,
    decoded: usize,
    retained_peak: usize,
    overflow: bool,
    terminal: &'static str,
}

impl From<PostgresObservationStreamEvidenceV1> for StreamEvidence {
    fn from(value: PostgresObservationStreamEvidenceV1) -> Self {
        Self {
            cap: value.cap(),
            polled: value.polled(),
            decoded: value.decoded(),
            retained_peak: value.retained_peak(),
            overflow: value.overflow(),
            terminal: value.terminal().as_str(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct QueryAccounting {
    prequalification_guard: u64,
    rich_capture: u64,
    guard_queries_observed: usize,
    stream_queries_observed: usize,
    total_queries_observed: usize,
    streams: Vec<AccountedStream>,
}

#[derive(Serialize)]
struct AccountedStream {
    id: &'static str,
    evidence: StreamEvidence,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Lifecycle {
    savepoint: &'static str,
    recovery: &'static str,
    commit: &'static str,
    phases_completed: Vec<&'static str>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    if std::env::args_os().len() != 1 {
        return ExitCode::from(QUALIFICATION_FAILURE_EXIT);
    }
    let output = match run().await {
        Ok(output) => output,
        Err(()) => return ExitCode::from(QUALIFICATION_FAILURE_EXIT),
    };
    if std::io::stdout().write_all(output.as_bytes()).is_err() {
        return ExitCode::from(QUALIFICATION_FAILURE_EXIT);
    }
    ExitCode::SUCCESS
}

async fn run() -> Result<String, ()> {
    let mut observer = connect(OBSERVER).await?;
    let role = comparison_role(&observer).await?;
    if !role.admitted() {
        return Err(());
    }
    let server_version = server_version(&observer).await?;
    let mut owner = connect(OWNER).await?;
    let (owner_result, owner_evidence) =
        introspect_postgres_public_observed_snapshot_with_evidence(&mut owner).await;
    let owner_snapshot = owner_result.map_err(|_| ())?;
    let (observer_result, observer_evidence) =
        introspect_postgres_public_observed_snapshot_with_evidence(&mut observer).await;
    let observer_snapshot = observer_result.map_err(|_| ())?;
    validate_complete_evidence(&owner_evidence)?;
    validate_complete_evidence(&observer_evidence)?;
    validate_equal_rich_evidence(&owner_evidence, &observer_evidence)?;
    let owner_identity = owner_snapshot.availability().identity().ok_or(())?;
    let observer_identity = observer_snapshot.availability().identity().ok_or(())?;
    if owner_identity != observer_identity {
        return Err(());
    }
    let visibility_differs =
        legacy_constraint_count(&owner_snapshot)? != legacy_constraint_count(&observer_snapshot)?;
    if !visibility_differs {
        return Err(());
    }
    let version_num = observer_evidence.server_version_num().ok_or(())?;
    validate_server_version(&server_version, version_num)?;
    let envelope = build_envelope(
        role,
        server_version,
        observer_snapshot,
        observer_evidence,
        visibility_differs,
    )?;
    let json = serde_json::to_string(&envelope).map_err(|_| ())?;
    if json.len() > MAX_OUTPUT_BYTES {
        return Err(());
    }
    Ok(json)
}

fn fixed_config(role: &'static str) -> Config {
    let mut config = Config::new();
    config
        .host_path(Path::new(SOCKET))
        .port(5432)
        .dbname(DATABASE)
        .user(role)
        .application_name(APPLICATION)
        // `session_replication_role` is already guarded by the observation
        // transaction and cannot be set by the deliberately unprivileged
        // qualification roles. Keep only the client encoding startup pin.
        .options("-c client_encoding=UTF8")
        .connect_timeout(Duration::from_secs(5));
    config
}

async fn connect(role: &'static str) -> Result<Client, ()> {
    let (client, connection) = fixed_config(role).connect(NoTls).await.map_err(|_| ())?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    Ok(client)
}

async fn comparison_role(client: &Client) -> Result<ComparisonRole, ()> {
    let row = client
        .query_one(ROLE_PREFLIGHT_SQL, &[])
        .await
        .map_err(|_| ())?;
    Ok(ComparisonRole {
        superuser: boolean(&row, "rolsuper")?,
        database_owner: boolean(&row, "database_owner")?,
        inherit: boolean(&row, "rolinherit")?,
        bypass_rls: boolean(&row, "rolbypassrls")?,
        can_set_role: boolean(&row, "can_set_role")?,
        can_ddl: boolean(&row, "can_ddl")?,
        privileges_exact: boolean(&row, "privileges_exact")?,
    })
}

fn boolean(row: &Row, column: &'static str) -> Result<bool, ()> {
    row.try_get(column).map_err(|_| ())
}

async fn server_version(client: &Client) -> Result<String, ()> {
    client
        .query_one(
            "SELECT ('PostgreSQL ' || pg_catalog.current_setting('server_version'))::text",
            &[],
        )
        .await
        .and_then(|row| row.try_get(0))
        .map_err(|_| ())
}

fn validate_server_version(version: &str, version_num: i32) -> Result<(), ()> {
    let patch = match version_num {
        160_009 => "16.9",
        160_015 => "16.15",
        _ => return Err(()),
    };
    let prefix = format!("PostgreSQL {patch}");
    if version.len() <= 256
        && version.is_ascii()
        && version.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
        && (version == prefix || version.starts_with(&(prefix + " ")))
    {
        Ok(())
    } else {
        Err(())
    }
}

fn validate_complete_evidence(evidence: &PostgresObservationEvidenceV1) -> Result<(), ()> {
    let streams_complete = ALL_STREAMS.iter().all(|stream| {
        let value = evidence.stream(*stream);
        value.started() && value.terminal() == PostgresObservationStreamTerminalV1::Complete
    });
    let phases_complete = ALL_PHASES
        .iter()
        .all(|phase| evidence.phase_was_started(*phase) && evidence.phase_was_completed(*phase));
    if streams_complete
        && phases_complete
        && evidence.guard_passed()
        && evidence.guard_queries_started() == 2
        && evidence.failure().is_none()
        && evidence.savepoint() == PostgresObservationSavepointV1::Released
        && evidence.commit() == PostgresObservationCommitV1::Complete
    {
        Ok(())
    } else {
        Err(())
    }
}

fn validate_equal_rich_evidence(
    owner: &PostgresObservationEvidenceV1,
    observer: &PostgresObservationEvidenceV1,
) -> Result<(), ()> {
    let streams = [
        PostgresObservationStreamV1::RichRelations,
        PostgresObservationStreamV1::RichAttributes,
        PostgresObservationStreamV1::RichCatalogConstraints,
    ];
    if streams
        .iter()
        .all(|stream| owner.stream(*stream) == observer.stream(*stream))
        && owner.recorded_constraint_counts() == observer.recorded_constraint_counts()
        && owner.server_version_num() == observer.server_version_num()
    {
        Ok(())
    } else {
        Err(())
    }
}

fn legacy_constraint_count(snapshot: &Postgres16PublicObservedSnapshotV1) -> Result<usize, ()> {
    snapshot
        .legacy_tables()
        .iter()
        .try_fold(0usize, |count, table| {
            let primary = usize::from(!table.primary_key.is_empty());
            count
                .checked_add(primary)
                .and_then(|value| value.checked_add(table.unique.len()))
                .and_then(|value| value.checked_add(table.foreign_keys.len()))
                .ok_or(())
        })
}

fn build_envelope(
    role: ComparisonRole,
    server_version: String,
    snapshot: Postgres16PublicObservedSnapshotV1,
    evidence: PostgresObservationEvidenceV1,
    visibility_differs: bool,
) -> Result<ProbeEnvelope, ()> {
    let identity = snapshot.availability().identity().ok_or(())?;
    let (not_null, catalog, combined) = evidence.recorded_constraint_counts();
    let relations = evidence.stream(PostgresObservationStreamV1::RichRelations);
    let attributes = evidence.stream(PostgresObservationStreamV1::RichAttributes);
    let constraints = evidence.stream(PostgresObservationStreamV1::RichCatalogConstraints);
    let observation = Observation {
        server_version,
        server_version_num: evidence.server_version_num().ok_or(())?,
        guard: "pass",
        row_counts: RowCounts {
            relations: relations.decoded(),
            attributes: attributes.decoded(),
            not_null_constraints: not_null,
            catalog_constraints: catalog,
            combined_constraints: combined,
        },
        streaming: RichStreaming {
            relations: relations.into(),
            attributes: attributes.into(),
            catalog_constraints: constraints.into(),
        },
        identity: Identity {
            structural: identity.structural().to_string(),
            types: identity.types().to_string(),
            constraints: identity.constraints().to_string(),
        },
        // Availability can exist only after the internal LegacyComparison
        // phase has matched the collected legacy relation/column coordinates.
        legacy_coordinate_comparison: "equal",
        error_code: None,
        failure_phase: None,
    };
    let streams = ALL_STREAMS
        .iter()
        .map(|stream| AccountedStream {
            id: stream.as_str(),
            evidence: evidence.stream(*stream).into(),
        })
        .collect();
    let phases_completed = ALL_PHASES.iter().map(|phase| phase.as_str()).collect();
    Ok(ProbeEnvelope {
        preflight: Preflight {
            network_mode: "none",
            fixed_database: true,
            unix_socket: true,
            comparison_role: role,
            owner_identity_equal: true,
            legacy_constraint_visibility_differs: visibility_differs,
        },
        observation,
        query_accounting: QueryAccounting {
            prequalification_guard: POSTGRES_PROFILE_PREQUALIFICATION_QUERY_COUNT_V1,
            rich_capture: POSTGRES_RICH_CAPTURE_CATALOGUE_QUERY_COUNT_V1,
            guard_queries_observed: evidence.guard_queries_started(),
            stream_queries_observed: ALL_STREAMS.len(),
            total_queries_observed: evidence.guard_queries_started() + ALL_STREAMS.len(),
            streams,
        },
        lifecycle: Lifecycle {
            savepoint: evidence.savepoint().as_str(),
            recovery: "not-needed",
            commit: evidence.commit().as_str(),
            phases_completed,
        },
    })
}

#[cfg(test)]
#[path = "postgres-observation-qualification/tests.rs"]
mod tests;
