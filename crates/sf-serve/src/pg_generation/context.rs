//! Closed PostgreSQL session context admitted for a verified generation.

use crate::source::POSTGRES_GENERATION_SCOPE_SETTING;

use super::PgGenerationError;

const SESSION_CONTEXT_SQL: &str = "SELECT \
 pg_catalog.pg_backend_pid() AS backend_pid, \
 d.oid AS database_oid, d.datname::text AS database_name, \
 cr.oid AS current_role_oid, cr.rolname::text AS current_role_name, \
 sr.oid AS session_role_oid, sr.rolname::text AS session_role_name, \
 cr.rolsuper AS superuser, cr.rolinherit AS inherits_privileges, \
 cr.rolcreaterole AS creates_roles, cr.rolcreatedb AS creates_databases, \
 cr.rolreplication AS replicates, cr.rolbypassrls AS bypasses_row_security, \
 EXISTS ( \
   SELECT 1 FROM pg_catalog.pg_auth_members memberships \
   WHERE memberships.member = cr.oid \
 ) AS has_role_memberships, \
 d.datdba = cr.oid AS owns_database, \
 n.nspowner = cr.oid AS owns_public_schema, \
 EXISTS ( \
   SELECT 1 FROM pg_catalog.pg_class mapped \
   WHERE mapped.relnamespace = n.oid AND mapped.relkind = 'r' \
     AND mapped.relowner = cr.oid \
 ) AS owns_mapped_table, \
 pg_catalog.has_database_privilege(cr.oid, d.oid, 'CONNECT') AS database_connect, \
 pg_catalog.has_database_privilege(cr.oid, d.oid, 'CREATE') AS database_create, \
 pg_catalog.has_database_privilege(cr.oid, d.oid, 'TEMP') AS database_temporary, \
 pg_catalog.has_schema_privilege(cr.oid, n.oid, 'USAGE') AS schema_usage, \
 pg_catalog.has_schema_privilege(cr.oid, n.oid, 'CREATE') AS schema_create, \
 NOT EXISTS ( \
   SELECT 1 FROM pg_catalog.pg_class mapped \
   WHERE mapped.relnamespace = n.oid AND mapped.relkind = 'r' \
     AND NOT pg_catalog.has_table_privilege(cr.oid, mapped.oid, 'SELECT') \
 ) AS every_mapped_table_select, \
 EXISTS ( \
   SELECT 1 FROM pg_catalog.pg_class mapped \
   WHERE mapped.relnamespace = n.oid AND mapped.relkind = 'r' AND ( \
     pg_catalog.has_table_privilege(cr.oid, mapped.oid, 'INSERT') \
     OR pg_catalog.has_table_privilege(cr.oid, mapped.oid, 'UPDATE') \
     OR pg_catalog.has_table_privilege(cr.oid, mapped.oid, 'DELETE') \
     OR pg_catalog.has_table_privilege(cr.oid, mapped.oid, 'TRUNCATE') \
     OR pg_catalog.has_table_privilege(cr.oid, mapped.oid, 'REFERENCES') \
     OR pg_catalog.has_table_privilege(cr.oid, mapped.oid, 'TRIGGER') \
     OR pg_catalog.has_any_column_privilege(cr.oid, mapped.oid, 'INSERT') \
     OR pg_catalog.has_any_column_privilege(cr.oid, mapped.oid, 'UPDATE') \
     OR pg_catalog.has_any_column_privilege(cr.oid, mapped.oid, 'REFERENCES') \
   ) \
 ) AS any_mapped_table_mutation, \
 pg_catalog.has_function_privilege( \
   cr.oid, \
   ('pg_catalog.pg_database_collation_actual_version(oid)'::pg_catalog.regprocedure)::pg_catalog.oid, \
   'EXECUTE' \
 ) AS collation_probe_execute, \
 pg_catalog.current_setting('server_version_num')::int4 AS server_version_num, \
 pg_catalog.current_setting('search_path') AS search_path, \
 pg_catalog.current_setting('row_security') AS row_security, \
 pg_catalog.current_setting('session_replication_role') AS session_replication_role \
 FROM pg_catalog.pg_database d \
 JOIN pg_catalog.pg_namespace n ON n.nspname = 'public' \
 JOIN pg_catalog.pg_roles cr ON cr.rolname = CURRENT_USER \
 JOIN pg_catalog.pg_roles sr ON sr.rolname = SESSION_USER \
 WHERE d.datname = pg_catalog.current_database()";
pub(super) const POSTGRES_SESSION_CONTEXT_QUERY_COUNT_V1: u64 = 1;

pub(super) struct PgRuntimeRoleFacts {
    pub(super) superuser: bool,
    pub(super) inherits_privileges: bool,
    pub(super) creates_roles: bool,
    pub(super) creates_databases: bool,
    pub(super) replicates: bool,
    pub(super) bypasses_row_security: bool,
    pub(super) has_role_memberships: bool,
    pub(super) owns_database: bool,
    pub(super) owns_public_schema: bool,
    pub(super) owns_mapped_table: bool,
    pub(super) database_connect: bool,
    pub(super) database_create: bool,
    pub(super) database_temporary: bool,
    pub(super) schema_usage: bool,
    pub(super) schema_create: bool,
    pub(super) every_mapped_table_select: bool,
    pub(super) any_mapped_table_mutation: bool,
    pub(super) collation_probe_execute: bool,
}

#[derive(Clone, Eq, PartialEq)]
pub(super) struct PgSessionContext {
    pub(super) database_oid: u32,
    pub(super) database_name: String,
    pub(super) current_role_oid: u32,
    pub(super) current_role_name: String,
    pub(super) session_role_oid: u32,
    pub(super) session_role_name: String,
    pub(super) server_version_num: i32,
    pub(super) search_path: String,
    pub(super) row_security: String,
    pub(super) session_replication_role: String,
}

/// One capture also binds the process serving this exact transaction. The PID
/// is lease-local and therefore does not enter the cross-connection generation
/// expectation.
#[derive(Eq, PartialEq)]
pub(super) struct PgSessionObservation {
    context: PgSessionContext,
    backend_pid: i32,
}

impl PgSessionObservation {
    pub(super) const fn context(&self) -> &PgSessionContext {
        &self.context
    }

    pub(super) fn into_context(self) -> PgSessionContext {
        self.context
    }
}

pub(super) async fn capture_session_context(
    client: &tokio_postgres::Client,
) -> Result<PgSessionObservation, PgGenerationError> {
    capture_session_context_query(client, SESSION_CONTEXT_SQL).await
}

async fn capture_session_context_query(
    client: &tokio_postgres::Client,
    query: &str,
) -> Result<PgSessionObservation, PgGenerationError> {
    let row = context_read(client.query_one(query, &[]).await)?;
    let context = PgSessionContext {
        database_oid: field(&row, "database_oid")?,
        database_name: field(&row, "database_name")?,
        current_role_oid: field(&row, "current_role_oid")?,
        current_role_name: field(&row, "current_role_name")?,
        session_role_oid: field(&row, "session_role_oid")?,
        session_role_name: field(&row, "session_role_name")?,
        server_version_num: field(&row, "server_version_num")?,
        search_path: field(&row, "search_path")?,
        row_security: field(&row, "row_security")?,
        session_replication_role: field(&row, "session_replication_role")?,
    };
    let backend_pid = field(&row, "backend_pid")?;
    let runtime_role = PgRuntimeRoleFacts {
        superuser: field(&row, "superuser")?,
        inherits_privileges: field(&row, "inherits_privileges")?,
        creates_roles: field(&row, "creates_roles")?,
        creates_databases: field(&row, "creates_databases")?,
        replicates: field(&row, "replicates")?,
        bypasses_row_security: field(&row, "bypasses_row_security")?,
        has_role_memberships: field(&row, "has_role_memberships")?,
        owns_database: field(&row, "owns_database")?,
        owns_public_schema: field(&row, "owns_public_schema")?,
        owns_mapped_table: field(&row, "owns_mapped_table")?,
        database_connect: field(&row, "database_connect")?,
        database_create: field(&row, "database_create")?,
        database_temporary: field(&row, "database_temporary")?,
        schema_usage: field(&row, "schema_usage")?,
        schema_create: field(&row, "schema_create")?,
        every_mapped_table_select: field(&row, "every_mapped_table_select")?,
        any_mapped_table_mutation: field(&row, "any_mapped_table_mutation")?,
        collation_probe_execute: field(&row, "collation_probe_execute")?,
    };
    validate_session_context(&context, backend_pid)?;
    validate_runtime_role(&runtime_role)?;
    Ok(PgSessionObservation {
        context,
        backend_pid,
    })
}

#[cfg(test)]
pub(super) async fn capture_session_context_query_for_test(
    client: &tokio_postgres::Client,
    query: &str,
) -> Result<PgSessionObservation, PgGenerationError> {
    capture_session_context_query(client, query).await
}

fn field<T>(row: &tokio_postgres::Row, name: &str) -> Result<T, PgGenerationError>
where
    T: for<'a> tokio_postgres::types::FromSql<'a>,
{
    context_read(row.try_get(name))
}

pub(super) fn context_read<T, E>(result: Result<T, E>) -> Result<T, PgGenerationError> {
    result.map_err(|_| PgGenerationError::SourceUnavailable)
}

pub(super) fn validate_session_context(
    context: &PgSessionContext,
    backend_pid: i32,
) -> Result<(), PgGenerationError> {
    let bounded_identifiers = [
        context.database_name.as_str(),
        context.current_role_name.as_str(),
        context.session_role_name.as_str(),
    ];
    if backend_pid <= 0
        || context.database_oid == 0
        || context.current_role_oid == 0
        || context.session_role_oid == 0
        || context.current_role_oid != context.session_role_oid
        || context.current_role_name != context.session_role_name
        || bounded_identifiers
            .into_iter()
            .any(|value| value.is_empty() || value.len() > 63 || value.contains('\0'))
        || !matches!(context.server_version_num, 160_009 | 160_015)
        || context.search_path != POSTGRES_GENERATION_SCOPE_SETTING
        || context.row_security != "on"
        || context.session_replication_role != "origin"
    {
        return Err(PgGenerationError::CapabilityDrift);
    }
    Ok(())
}

pub(super) fn validate_runtime_role(facts: &PgRuntimeRoleFacts) -> Result<(), PgGenerationError> {
    if facts.superuser
        || facts.inherits_privileges
        || facts.creates_roles
        || facts.creates_databases
        || facts.replicates
        || facts.bypasses_row_security
        // With neither membership nor superuser/ownership capabilities, the
        // dedicated login has no path to SET ROLE into another authority.
        || facts.has_role_memberships
        || facts.owns_database
        || facts.owns_public_schema
        || facts.owns_mapped_table
        || !facts.database_connect
        || facts.database_create
        || facts.database_temporary
        || !facts.schema_usage
        || facts.schema_create
        || !facts.every_mapped_table_select
        || facts.any_mapped_table_mutation
        || !facts.collation_probe_execute
    {
        return Err(PgGenerationError::CapabilityDrift);
    }
    Ok(())
}
