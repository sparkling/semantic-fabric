//! One fixed MySQL session profile, shared by candidates and request leases.
use mysql_async::{consts::StatusFlags, prelude::Queryable, Conn, Params};
use sf_sql::source_work::SourceWork;

use super::schema::{rows, text};
use crate::pg_generation::PgGenerationError;

const MODES: &str = "ONLY_FULL_GROUP_BY,STRICT_TRANS_TABLES,NO_ZERO_IN_DATE,NO_ZERO_DATE,ERROR_FOR_DIVISION_BY_ZERO,NO_ENGINE_SUBSTITUTION,NO_BACKSLASH_ESCAPES";
const SQL: &str = "SELECT LEFT(CAST(VERSION() AS BINARY),65), \
    LEFT(CAST(DATABASE() AS BINARY),257), LEFT(CAST(CURRENT_USER() AS BINARY),289), \
    LEFT(CAST(@@lower_case_table_names AS BINARY),2), \
    LEFT(CAST(@@session.sql_mode AS BINARY),1025), \
    LEFT(CAST(@@session.character_set_client AS BINARY),65), \
    LEFT(CAST(@@session.character_set_connection AS BINARY),65), \
    LEFT(CAST(@@session.character_set_results AS BINARY),65), \
    LEFT(CAST(@@session.collation_connection AS BINARY),65), \
    LEFT(CAST(@@session.time_zone AS BINARY),65), \
    LEFT(CAST(@@session.autocommit AS BINARY),2), \
    LEFT(CAST(@@session.transaction_isolation AS BINARY),33), \
    LEFT(CAST(@@session.transaction_read_only AS BINARY),2)";

pub(crate) async fn setup(conn: &mut Conn) -> Result<(), PgGenerationError> {
    // Each statement is acknowledged before START. No application-table reads
    // or early consistent snapshot may precede the complete relation barrier.
    for sql in [
        "SET SESSION autocommit=1".to_owned(),
        "SET SESSION TRANSACTION ISOLATION LEVEL REPEATABLE READ".to_owned(),
        "SET SESSION TRANSACTION READ ONLY".to_owned(),
        "SET SESSION time_zone='+00:00'".to_owned(),
        "SET NAMES utf8mb4 COLLATE utf8mb4_bin".to_owned(),
        format!("SET SESSION sql_mode='{MODES}'"),
    ] {
        conn.query_drop(sql)
            .await
            .map_err(|_| PgGenerationError::SourceUnavailable)?;
    }
    Ok(())
}

pub(crate) async fn capture(
    conn: &mut Conn,
    work: SourceWork<'_>,
    bytes: &mut usize,
) -> Result<Vec<Vec<Option<String>>>, PgGenerationError> {
    let result = rows(
        conn,
        SQL,
        Params::Empty,
        1,
        &[64, 256, 288, 1, 1024, 64, 64, 64, 64, 64, 1, 32, 1],
        work,
        bytes,
    )
    .await?;
    let row = result.first().ok_or(PgGenerationError::CapabilityDrift)?;
    if text(row, 0)? != "8.4.11"
        || text(row, 1)?.is_empty()
        || text(row, 3)? != "0"
        || !valid_modes(text(row, 4)?)
        || (5..=7).any(|i| text(row, i).ok() != Some("utf8mb4"))
        || text(row, 8)? != "utf8mb4_bin"
        || text(row, 9)? != "+00:00"
        || text(row, 10)? != "1"
        || text(row, 11)? != "REPEATABLE-READ"
        || text(row, 12)? != "1"
    {
        return Err(PgGenerationError::CapabilityDrift);
    }
    Ok(result)
}

fn valid_modes(modes: &str) -> bool {
    modes.split(',').count() == MODES.split(',').count()
        && MODES
            .split(',')
            .all(|required| modes.split(',').any(|mode| required == mode))
}

pub(crate) fn transaction(conn: &Conn, active: bool) -> Result<(), PgGenerationError> {
    let flags = conn
        .last_ok_packet()
        .ok_or(PgGenerationError::SourceUnavailable)?
        .status_flags();
    let expected =
        StatusFlags::SERVER_STATUS_IN_TRANS | StatusFlags::SERVER_STATUS_IN_TRANS_READONLY;
    if (active && flags.contains(expected)) || (!active && !flags.intersects(expected)) {
        Ok(())
    } else {
        Err(PgGenerationError::CapabilityDrift)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_modes_preserve_authored_literals_and_exclude_alternate_operators() {
        assert!(valid_modes(MODES));
        assert!(!valid_modes(&MODES.replace(",NO_BACKSLASH_ESCAPES", "")));
        assert!(!valid_modes(&format!("{MODES},PIPES_AS_CONCAT")));
        assert!(!valid_modes(&format!("{MODES},PAD_CHAR_TO_FULL_LENGTH")));
    }
}
