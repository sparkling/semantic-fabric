use super::*;
use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};

fn connection() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE t(id INTEGER, d DATE, c CHARACTER(4)); INSERT INTO t VALUES(7,'2026-09-08','a');").unwrap();
    conn
}

#[tokio::test]
async fn borrowing_twin_preserves_decltypes_padding_and_bound_parameters() {
    let conn = connection();
    let mut backend = SqliteBackend::new(&conn);
    let mut stream = backend
        .open_branch_with_metadata(
            "SELECT d COLLATE BINARY AS d, c COLLATE BINARY AS c FROM t WHERE id = ?",
            &["7".into()],
            Some("SELECT d AS d, c AS c FROM t WHERE id = ?"),
        )
        .await
        .unwrap();
    let row = stream.next_row().await.unwrap().unwrap();
    assert_eq!(
        row.values,
        vec![Some("2026-09-08".into()), Some("a   ".into())]
    );
    assert_eq!(
        row.codes,
        vec![Some(XsdTypeCode::Date), Some(XsdTypeCode::String)]
    );
    assert!(stream.next_row().await.unwrap().is_none());
}

#[tokio::test]
async fn owned_twin_is_prepared_but_never_evaluated() {
    let conn = std::sync::Arc::new(std::sync::Mutex::new(connection()));
    let mut backend = SqliteOwnedBackend::new(conn);
    let mut stream = backend
        .open_branch_with_metadata(
            "SELECT d COLLATE BINARY AS d, 1 AS second FROM t",
            &[],
            Some("SELECT d AS d, abs(-9223372036854775808) AS second FROM t"),
        )
        .await
        .unwrap();
    let row = stream.next_row().await.unwrap().unwrap();
    assert_eq!(
        row.values,
        vec![Some("2026-09-08".into()), Some("1".into())]
    );
    assert_eq!(row.codes[0], Some(XsdTypeCode::Date));
    assert!(stream.next_row().await.unwrap().is_none());
}

#[tokio::test]
async fn both_adapters_reject_projection_mismatch_before_evaluation() {
    let conn = connection();
    let mut backend = SqliteBackend::new(&conn);
    assert!(matches!(
        backend
            .open_branch_with_metadata(
                "SELECT d, abs(-9223372036854775808) FROM t",
                &[],
                Some("SELECT d FROM t"),
            )
            .await,
        Err(Error::Emit(_))
    ));
    let conn = std::sync::Arc::new(std::sync::Mutex::new(connection()));
    let mut backend = SqliteOwnedBackend::new(conn);
    let mut stream = backend
        .open_branch_with_metadata(
            "SELECT d, abs(-9223372036854775808) FROM t",
            &[],
            Some("SELECT d FROM t"),
        )
        .await
        .unwrap();
    assert!(matches!(stream.next_row().await, Err(Error::Emit(_))));
}

fn metadata_budget(source: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(u64::MAX, source, u64::MAX, u64::MAX))
}

#[tokio::test]
async fn metadata_copies_refuse_at_exact_source_boundary_and_recover_owned_connection() {
    let query = "SELECT id,d,c FROM t";
    let conn = connection();
    let budget = metadata_budget(u64::MAX);
    let expected = result_columns(&conn, query).unwrap();
    let actual = result_columns_with_control(&conn, query, Some(&budget)).unwrap();
    assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
    let total = budget.consumed(QueryCharge::SourceWork);
    assert!(total > 0);
    assert_eq!(budget.consumed(QueryCharge::CompilerWork), 0);
    for remaining in [total - 1, total] {
        let control = metadata_budget(remaining);
        let result = result_columns_with_control(&conn, query, Some(&control));
        assert_eq!(result.is_ok(), remaining == total);
        if remaining < total {
            assert!(matches!(
                result,
                Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
            ));
        }
    }

    let owned = SqliteOwnedConnection::new(conn);
    for remaining in [total - 1, total] {
        // The owned bridge also prepays its worker-owned probe string.
        let control = std::sync::Arc::new(metadata_budget(remaining + query.len() as u64));
        let lease = owned.acquire().await.unwrap();
        let mut backend = SqliteOwnedBackend::new_controlled_leased(lease, control.clone());
        let result = backend
            .result_columns_controlled(query, control.as_ref())
            .await;
        assert_eq!(result.is_ok(), remaining == total);
        drop(backend);
        let lease = tokio::time::timeout(std::time::Duration::from_secs(1), owned.acquire())
            .await
            .expect("metadata worker must release the same connection")
            .unwrap();
        drop(lease);
    }
}

#[test]
fn terminal_metadata_request_does_not_reach_invalid_sql() {
    let conn = connection();
    for cause in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        let control = metadata_budget(u64::MAX);
        control.terminate(cause);
        let result = result_columns_with_control(&conn, "invalid SQL", Some(&control));
        assert!(matches!(result, Err(Error::QueryControl(actual)) if actual == cause));
        assert_eq!(control.consumed(QueryCharge::SourceWork), 0);
    }
}

#[test]
fn paid_declaration_scanner_preserves_raw_datatype_and_padding_laws() {
    for declaration in [
        "CHARACTER",
        "CHARACTER VARYING",
        "CHAR",
        "VARCHAR",
        "CLOB",
        "NCHAR",
        "NCHAR VARYING",
        "NVARCHAR",
        "NCLOB",
        "TEXT",
        "BPCHAR",
        "NAME",
        "UNKNOWN",
        "BINARY",
        "BINARY VARYING",
        "VARBINARY",
        "BINARY LARGE OBJECT",
        "BLOB",
        "BYTEA",
        "NUMERIC",
        "DECIMAL",
        "DEC",
        "SMALLINT",
        "INTEGER",
        "INT",
        "BIGINT",
        "INT2",
        "INT4",
        "INT8",
        "FLOAT",
        "REAL",
        "DOUBLE PRECISION",
        "DOUBLE",
        "FLOAT4",
        "FLOAT8",
        "BOOLEAN",
        "BOOL",
        "DATE",
        "TIME",
        "TIME WITHOUT TIME ZONE",
        "TIME WITH TIME ZONE",
        "TIMETZ",
        "TIMESTAMP",
        "TIMESTAMP WITHOUT TIME ZONE",
        "TIMESTAMP WITH TIME ZONE",
        "TIMESTAMPTZ",
        " char ( +0004 ) trailing",
        "CHARACTER(0)",
        "NCHAR(4",
        "CHAR(-1)",
        "CHAR(999999999999999999999999999999999)",
        "ſmallint",
        "\u{2003}character\t(4)",
        "char varying(4)",
        "unknown type",
        "",
        "αβγ",
    ]
    .into_iter()
    .map(str::to_owned)
    .chain(["a ".repeat(256)])
    {
        let expected = (
            datatype::natural_xsd(&declaration),
            char_pad_len(&declaration),
        );
        let control = metadata_budget(u64::MAX);
        let actual = crate::source_work::declaration_metadata(
            &declaration,
            crate::source_work::SourceWork::new(Some(&control)),
        )
        .unwrap();
        assert_eq!(actual, expected, "{declaration:?}");
        let total = control.consumed(QueryCharge::SourceWork);
        if total > 0 {
            assert!(crate::source_work::declaration_metadata(
                &declaration,
                crate::source_work::SourceWork::new(Some(&metadata_budget(total - 1))),
            )
            .is_err());
        }
    }
}

/// `column_meta`'s own third argument, direct: its sibling `result_columns_with_control`
/// already has exact-boundary coverage above, but `column_meta` (the owned worker's actual
/// call target) did not.
#[test]
fn column_meta_exact_boundary_is_pinned_by_an_assertion() {
    let conn = connection();
    let query = "SELECT id,d,c FROM t";
    let budget = metadata_budget(u64::MAX);
    let (_, _, nproj) = column_meta(&conn, query, Some(&budget)).unwrap();
    assert_eq!(nproj, 3);
    let total = budget.consumed(QueryCharge::SourceWork);
    assert!(total > 0, "governed column metadata must charge SourceWork");
    assert!(column_meta(&conn, query, Some(&metadata_budget(total))).is_ok());
    assert!(matches!(
        column_meta(&conn, query, Some(&metadata_budget(total - 1))),
        Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
    ));
}
