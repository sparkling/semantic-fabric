//! Percent-encoder shape, accounting and live-reference checks.
use super::*;

/// Regression guard for the fix that bound the MySQL BINARY cast once
/// (`pre.b`) instead of re-embedding the source column expression at
/// every byte-range/length check: before that fix MySQL's `repeats` was
/// 148 (vs. PostgreSQL's 2 and SQLite's 1), so `SourceWork` charged for
/// percent-encoding a MySQL column scaled ~74x worse per byte of column-
/// expression length than the other dialects for the identical
/// semantic check, not because of any larger actual workload. `<= 5`
/// leaves headroom for incidental template growth while catching a
/// reintroduced per-check re-embedding of the column expression.
#[test]
fn mysql_percent_encoder_repeats_stays_bounded() {
    let repeats = |dialect| {
        let fixed = percent_encode_col("", dialect).map_or(0, |sql| sql.len());
        let with_x = percent_encode_col("x", dialect).map_or(0, |sql| sql.len());
        with_x - fixed
    };
    assert!(
        repeats(Dialect::MySql) <= 5,
        "MySQL percent-encoding repeats={} (was 148 before pre.b binding)",
        repeats(Dialect::MySql)
    );
}

/// Mirrors [`percent_encode_col_sqlite_matches_reference_iri_encoding`]
/// for MySQL, plus the two standalone-invalid-UTF-8 byte cases (0x80, a
/// lone continuation byte; 0xC2, a lone 2-byte lead), which deliberately
/// make the generated SQL raise via `JSON_EXTRACT` on invalid JSON per
/// [`percent_encode_col_mysql`]'s own doc comment, rather than compare
/// against `reference_encode` (which requires valid UTF-8 input).
/// Verified against a live MySQL 8.4 instance during this fix's
/// development: identical pass/fail and identical encoded output to the
/// pre-fix template across every case here, only the generated SQL's
/// `repeats` factor changed (148 -> 1).
#[tokio::test]
#[ignore = "requires a purpose-created isolated MySQL provider"]
async fn mysql_percent_encoder_matches_reference_iri_encoding() {
    use mysql_async::prelude::Queryable;

    let socket =
        std::env::var("SF_MYSQL_SOCKET").expect("required-live MySQL socket must be configured");
    let opts: mysql_async::Opts = mysql_async::OptsBuilder::default()
        .user(Some("root"))
        .socket(Some(socket))
        .prefer_socket(Some(true))
        .stmt_cache_size(Some(0))
        .into();
    let mut conn = mysql_async::Conn::new(opts)
        .await
        .unwrap_or_else(|_| panic!("connect to isolated MySQL provider failed"));
    conn.query_drop("CREATE TEMPORARY TABLE t (v BLOB)")
        .await
        .unwrap();

    let mut cases: Vec<Option<Vec<u8>>> = vec![
        Some(b"a b/c".to_vec()),
        Some(b"A-z.0_9~".to_vec()),
        Some(b"".to_vec()),
        None,
        Some(b"X/Y".to_vec()),
        Some("你好/世界".as_bytes().to_vec()),
        Some(b"tab\ttab".to_vec()),
        Some(b"nul\0nul".to_vec()),
    ];
    for b in 0x20u8..=0x7e {
        cases.push(Some(vec![b]));
    }
    for b in 0..=0x1fu8 {
        cases.push(Some(vec![b]));
    }
    cases.push(Some(vec![0x7fu8]));

    let sql = percent_encode_col("t.v", Dialect::MySql).expect("MySQL is supported");
    let query = format!("SELECT {sql} FROM t");
    let mut mismatches = Vec::new();
    for v in &cases {
        conn.query_drop("DELETE FROM t").await.unwrap();
        conn.exec_drop("INSERT INTO t (v) VALUES (?)", (v.clone(),))
            .await
            .unwrap();
        let got = conn
            .query_first::<Option<String>, _>(&query)
            .await
            .unwrap()
            .flatten();
        let want = v
            .as_ref()
            .map(|bytes| reference_encode(&String::from_utf8_lossy(bytes)));
        if got != want {
            mismatches.push(format!("input={v:?} got={got:?} want={want:?}"));
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");

    for invalid in [vec![0x80u8], vec![0xC2u8]] {
        conn.query_drop("DELETE FROM t").await.unwrap();
        conn.exec_drop("INSERT INTO t (v) VALUES (?)", (invalid.clone(),))
            .await
            .unwrap();
        let result = conn.query_first::<Option<String>, _>(&query).await;
        assert!(
            result.is_err(),
            "standalone-invalid UTF-8 {invalid:?} must fail closed, not silently encode"
        );
    }
    conn.disconnect()
        .await
        .unwrap_or_else(|_| panic!("close isolated MySQL connection failed"));
}

#[test]
fn percent_encoder_charge_equals_its_output_length() {
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryLimits};
    use sf_sql::source_work::SourceWork;
    for dialect in [Dialect::Sqlite, Dialect::MySql, Dialect::Postgres] {
        for column in [
            "t.v",
            "(__sf_lexical_key_v1(t0.\"value\", 1, -1) COLLATE BINARY)",
        ] {
            let control =
                QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
            let sql = percent_encode_col_controlled(
                column,
                dialect,
                &ColumnCatalog::default(),
                SourceWork::new(Some(&control)),
            )
            .unwrap();
            assert_eq!(
                control.consumed(QueryCharge::SourceWork),
                sql.len() as u64 + 1,
                "{dialect:?} {column}"
            );
        }
    }
}

/// Every dialect's [`percent_encode_col`] must reconstruct EXACTLY what
/// `sf_core::ir::percent_encode_iri` computes, across the full encodable
/// byte range (every printable ASCII special AND every control byte,
/// 0x00-0x1F/0x7F) plus the edge cases design-time probing against a
/// live SQLite connection found real bugs in: an embedded NUL byte
/// (SQLite's TEXT-mode `LENGTH` is NUL-terminated and silently
/// truncates — closed by `CAST(... AS BLOB)`, see
/// [`percent_encode_col_sqlite`]'s doc comment), an empty-but-non-NULL
/// string (a naive recursive-CTE base case still fired once past the
/// end, wrongly emitting a bare `%`), a genuinely NULL column (the
/// aggregate collapses an empty AND a NULL input to the same result
/// unless explicitly distinguished), and a multi-byte UTF-8 (CJK)
/// character (each of its individual bytes is standalone-invalid UTF-8,
/// exercising the byte-level reassembly path). SQLite only here (no
/// live-server dependency for a routine `cargo test` run) — PostgreSQL
/// and MySQL were validated the identical way against live servers
/// during this fix's development; see [`percent_encode_col_postgres`]/
/// [`percent_encode_col_mysql`]'s own doc comments for the
/// dialect-specific bugs their first drafts had (a PG NULL/empty
/// conflation; a MySQL `LENGTH`-vs-`SUBSTRING` byte/character unit
/// mismatch; and MySQL `group_concat_max_len` silent truncation).
#[test]
fn percent_encode_col_sqlite_matches_reference_iri_encoding() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    let _keys = sf_sql::backend::sqlite::lexical_keys(&conn).unwrap();
    conn.execute("CREATE TABLE t (v TEXT)", []).unwrap();

    let mut cases: Vec<Option<String>> = vec![
        Some("a b/c".to_owned()),
        Some("A-z.0_9~".to_owned()),
        Some("".to_owned()),
        None,
        Some("X/Y".to_owned()),
        Some("你好/世界".to_owned()), // non-ASCII must pass through raw
        Some("tab\ttab".to_owned()),  // control byte 0x09
        Some("nul\u{0}nul".to_owned()), // embedded NUL, 0x00
    ];
    for b in 0x20u8..=0x7e {
        cases.push(Some((b as char).to_string())); // every printable ASCII byte
    }
    for b in 0..=0x1fu8 {
        cases.push(Some((b as char).to_string())); // every control byte
    }
    cases.push(Some((0x7fu8 as char).to_string()));

    let sql = percent_encode_col("t.v", Dialect::Sqlite).expect("SQLite is supported");
    let query = format!("SELECT {sql} FROM t");
    let mut mismatches = Vec::new();
    for v in &cases {
        conn.execute("DELETE FROM t", []).unwrap();
        conn.execute("INSERT INTO t (v) VALUES (?1)", [v]).unwrap();
        let got: Option<String> = conn.query_row(&query, [], |r| r.get(0)).unwrap();
        let want = v.as_deref().map(reference_encode);
        if got != want {
            mismatches.push(format!("input={v:?} got={got:?} want={want:?}"));
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
}

#[test]
fn mysql_percent_encoder_round_trips_through_the_ast_boundary() {
    let expression =
        percent_encode_col("sfs0.`value`", Dialect::MySql).expect("MySQL is supported");
    let skeleton = format!("SELECT {expression} FROM `source` sfs0");
    let emitted = Dialect::MySql
        .emit_via_ast(&skeleton)
        .expect("MySQL JSON_TABLE encoder must pass the SQL AST boundary");
    assert!(emitted.contains("JSON_TABLE"), "{emitted}");
    assert!(emitted.contains("FOR ORDINALITY"), "{emitted}");
    assert!(
        emitted.contains("group_concat_max_len = 1000000"),
        "{emitted}"
    );
    assert!(emitted.contains("LEAST(333333"), "{emitted}");
    assert!(
        emitted.contains("@@SESSION.group_concat_max_len"),
        "{emitted}"
    );
    assert!(
        emitted.contains("@@SESSION.max_allowed_packet"),
        "{emitted}"
    );
    assert!(emitted.contains("- 4096"), "{emitted}");
    assert!(
        emitted.contains("semantic-fabric-percent-encoding-input-limit"),
        "{emitted}"
    );
}

#[tokio::test]
#[ignore = "requires a purpose-created isolated MySQL provider"]
async fn mysql_percent_encoder_limit_fails_instead_of_truncating() {
    use mysql_async::prelude::Queryable;

    let socket =
        std::env::var("SF_MYSQL_SOCKET").expect("required-live MySQL socket must be configured");
    let opts: mysql_async::Opts = mysql_async::OptsBuilder::default()
        .user(Some("root"))
        .socket(Some(socket))
        .prefer_socket(Some(true))
        .stmt_cache_size(Some(0))
        .into();
    let mut conn = mysql_async::Conn::new(opts)
        .await
        .unwrap_or_else(|_| panic!("connect to isolated MySQL provider failed"));
    let expression = percent_encode_col_mysql("source_value.value");
    let oversized_query = format!(
        "SELECT {expression} FROM \
         (SELECT REPEAT(' ', {}) AS value) AS source_value",
        MYSQL_PERCENT_ENCODE_MAX_INPUT_BYTES + 1
    );
    let result: mysql_async::Result<Option<String>> = conn.query_first(oversized_query).await;
    assert!(result.is_err(), "oversized encoding must fail closed");
    let constrained = expression.replacen(
        "SET_VAR(group_concat_max_len = 1000000)",
        "SET_VAR(group_concat_max_len = 9)",
        1,
    );
    let constrained_query = format!(
        "SELECT {constrained} FROM \
         (SELECT REPEAT(' ', 4) AS value) AS source_value"
    );
    let result: mysql_async::Result<Option<String>> = conn.query_first(constrained_query).await;
    assert!(
        result.is_err(),
        "statement-observed aggregate ceiling must fail closed"
    );
    conn.disconnect()
        .await
        .unwrap_or_else(|_| panic!("close isolated MySQL connection failed"));
}

#[tokio::test]
#[ignore = "requires a purpose-created MySQL provider pinned to an 8192-byte packet ceiling"]
async fn mysql_percent_encoder_packet_ceiling_fails_instead_of_truncating() {
    use mysql_async::prelude::Queryable;

    let socket =
        std::env::var("SF_MYSQL_SOCKET").expect("required-live MySQL socket must be configured");
    let opts: mysql_async::Opts = mysql_async::OptsBuilder::default()
        .user(Some("root"))
        .socket(Some(socket))
        .prefer_socket(Some(true))
        .stmt_cache_size(Some(0))
        .into();
    let mut limited = mysql_async::Conn::new(opts)
        .await
        .unwrap_or_else(|_| panic!("connect to packet-bounded MySQL provider failed"));
    let observed_packet: u64 = limited
        .query_first("SELECT @@SESSION.max_allowed_packet")
        .await
        .unwrap_or_else(|_| panic!("read constrained MySQL packet ceiling failed"))
        .unwrap_or_else(|| panic!("constrained MySQL packet ceiling is absent"));
    assert_eq!(
        observed_packet, 8192,
        "packet-bound evidence requires the exact isolated provider profile"
    );
    let packet_bound = (observed_packet.saturating_sub(MYSQL_PACKET_RESERVE_BYTES as u64)) / 3;
    let expression = percent_encode_col_mysql("source_value.value");
    let packet_query = format!(
        "SELECT {expression} FROM \
         (SELECT REPEAT(' ', {}) AS value) AS source_value",
        packet_bound + 1
    );
    let packet_result: mysql_async::Result<Option<String>> =
        limited.query_first(packet_query).await;
    limited
        .disconnect()
        .await
        .unwrap_or_else(|_| panic!("close constrained MySQL connection failed"));
    assert!(
        packet_result.is_err(),
        "statement-observed packet ceiling must fail closed"
    );
}

/// A dialect this module does not implement encoding for (Oracle, picked
/// arbitrarily) declines soundly rather than guessing.
#[test]
fn percent_encode_col_unsupported_dialect_is_501() {
    assert!(matches!(
        percent_encode_col("t.v", Dialect::Oracle),
        Err(Error::Unsupported(_))
    ));
}
