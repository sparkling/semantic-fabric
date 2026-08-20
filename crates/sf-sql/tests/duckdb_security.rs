#![cfg(feature = "duckdb-backend")]

use std::sync::{Arc, Mutex};

use sf_sql::backend::duckdb::DuckDbBackend;
use sf_sql::SqlBackend;

#[tokio::test]
async fn metadata_probe_rejects_non_query_and_multiple_statements() {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    let mut backend = DuckDbBackend::new(Arc::new(Mutex::new(conn)));

    let attach = backend.column_names("ATTACH ':memory:' AS other").await;
    assert!(
        attach.is_err(),
        "ATTACH must not execute as a metadata probe"
    );

    let multiple = backend.column_names("SELECT 1; SELECT 2").await;
    assert!(
        multiple.is_err(),
        "multiple statements must not execute as a metadata probe"
    );
}

#[tokio::test]
async fn limited_wrapped_query_probe_returns_column_names() {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE people(id INTEGER, name VARCHAR)")
        .unwrap();
    let mut backend = DuckDbBackend::new(Arc::new(Mutex::new(conn)));

    let names = backend
        .column_names("SELECT * FROM (SELECT id, name FROM people) AS sf_probe LIMIT 0")
        .await
        .unwrap();
    assert_eq!(names, ["id", "name"]);
}

#[tokio::test]
async fn accumulated_row_lexical_size_is_bounded() {
    use sf_sql::BranchStream;

    let conn = duckdb::Connection::open_in_memory().unwrap();
    let mut backend = DuckDbBackend::new(Arc::new(Mutex::new(conn)))
        .with_max_value_bytes(8)
        .with_max_row_bytes(8);
    let mut stream = backend
        .open_branch("SELECT '12345', '67890'", &[])
        .await
        .unwrap();

    let error = stream
        .next_row()
        .await
        .err()
        .expect("the aggregate row cap must reject two individually valid values");
    assert!(error.to_string().contains("row requires more than 8"));
}
