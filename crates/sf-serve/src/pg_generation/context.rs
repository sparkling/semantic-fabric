//! Closed PostgreSQL session context admitted for a verified generation.

use crate::source::POSTGRES_RELATION_SCOPE_SETTING;

use super::PgGenerationError;

const SESSION_CONTEXT_SQL: &str = "SELECT \
 d.oid AS database_oid, d.datname::text AS database_name, \
 cr.oid AS current_role_oid, cr.rolname::text AS current_role_name, \
 sr.oid AS session_role_oid, sr.rolname::text AS session_role_name, \
 cr.rolsuper, cr.rolbypassrls, \
 pg_catalog.current_setting('server_version_num')::int4 AS server_version_num, \
 pg_catalog.current_setting('search_path') AS search_path, \
 pg_catalog.current_setting('row_security') AS row_security, \
 pg_catalog.current_setting('session_replication_role') AS session_replication_role \
 FROM pg_catalog.pg_database d \
 JOIN pg_catalog.pg_roles cr ON cr.rolname = CURRENT_USER \
 JOIN pg_catalog.pg_roles sr ON sr.rolname = SESSION_USER \
 WHERE d.datname = pg_catalog.current_database()";

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

pub(super) async fn capture_session_context(
    client: &tokio_postgres::Client,
) -> Result<PgSessionContext, PgGenerationError> {
    let row = client
        .query_one(SESSION_CONTEXT_SQL, &[])
        .await
        .map_err(|_| PgGenerationError::SourceUnavailable)?;
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
    let elevated = field(&row, "rolsuper")?;
    let bypass_rls = field(&row, "rolbypassrls")?;
    validate_session_context(&context, elevated, bypass_rls)?;
    Ok(context)
}

fn field<T>(row: &tokio_postgres::Row, name: &str) -> Result<T, PgGenerationError>
where
    T: for<'a> tokio_postgres::types::FromSql<'a>,
{
    row.try_get(name)
        .map_err(|_| PgGenerationError::CapabilityDrift)
}

pub(super) fn validate_session_context(
    context: &PgSessionContext,
    elevated: bool,
    bypass_rls: bool,
) -> Result<(), PgGenerationError> {
    let bounded_identifiers = [
        context.database_name.as_str(),
        context.current_role_name.as_str(),
        context.session_role_name.as_str(),
    ];
    if context.database_oid == 0
        || context.current_role_oid == 0
        || context.session_role_oid == 0
        || context.current_role_oid != context.session_role_oid
        || context.current_role_name != context.session_role_name
        || bounded_identifiers
            .into_iter()
            .any(|value| value.is_empty() || value.len() > 63 || value.contains('\0'))
        || !matches!(context.server_version_num, 160_009 | 160_015)
        || context.search_path != POSTGRES_RELATION_SCOPE_SETTING
        || context.row_security != "on"
        || context.session_replication_role != "origin"
        || elevated
        || bypass_rls
    {
        return Err(PgGenerationError::CapabilityDrift);
    }
    Ok(())
}
