use crate::Error;

/// Flatten an `sf-sql` driver error's source chain into the message (the SQLite
/// chain is usually empty, so this is byte-identical to the old `Error::Sql(e)`).
pub(super) fn map_sql_err(e: sf_sql::Error) -> Error {
    use std::error::Error as _;
    // An uncovered PG result type (adapter `pg_value`) is preserved as a distinct
    // 501 skip — byte-identical to the pre-M3 `exec_pg` path, which returned
    // `sf_sparql::Error::Unsupported` directly from `pg_value` (never `Sql`).
    let e = match e {
        sf_sql::Error::QueryControl(error) => return Error::QueryControl(error),
        sf_sql::Error::Mysql(mysql_async::Error::Server(error)) if error.code == 3024 => {
            // ER_QUERY_TIMEOUT identifies the server's execution-time limit,
            // including a stricter source policy than the HTTP deadline. Do not
            // infer deadlines from SQL text or generic cancellation codes.
            return Error::QueryControl(
                sf_core::query_control::QueryControlError::DeadlineExceeded,
            );
        }
        sf_sql::Error::Unsupported(message) => return Error::Unsupported(message),
        other => other,
    };
    let mut msg = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        msg.push_str(": ");
        msg.push_str(&s.to_string());
        src = s.source();
    }
    Error::Sql(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_mysql_timeout_stays_typed_without_reclassifying_other_errors() {
        for code in [3024, 1317, 1064] {
            let error =
                sf_sql::Error::Mysql(mysql_async::Error::Server(mysql_async::ServerError {
                    code,
                    state: "HY000".into(),
                    message: "redacted provider detail".into(),
                }));
            let mapped = map_sql_err(error);
            if code == 3024 {
                assert!(matches!(
                    mapped,
                    Error::QueryControl(
                        sf_core::query_control::QueryControlError::DeadlineExceeded
                    )
                ));
            } else {
                assert!(matches!(mapped, Error::Sql(_)));
            }
        }
    }
}
