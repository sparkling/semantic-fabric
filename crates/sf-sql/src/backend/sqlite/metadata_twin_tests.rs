use super::*;

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
