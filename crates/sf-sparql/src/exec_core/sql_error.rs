use crate::Error;

/// Flatten an `sf-sql` driver error's source chain into the message (the SQLite
/// chain is usually empty, so this is byte-identical to the old `Error::Sql(e)`).
pub(super) fn map_sql_err(e: sf_sql::Error) -> Error {
    use std::error::Error as _;
    // An uncovered PG result type (adapter `pg_value`) is preserved as a distinct
    // 501 skip — byte-identical to the pre-M3 `exec_pg` path, which returned
    // `sf_sparql::Error::Unsupported` directly from `pg_value` (never `Sql`).
    if let sf_sql::Error::Unsupported(m) = &e {
        return Error::Unsupported(m.clone());
    }
    let mut msg = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        msg.push_str(": ");
        msg.push_str(&s.to_string());
        src = s.source();
    }
    Error::Sql(msg)
}
