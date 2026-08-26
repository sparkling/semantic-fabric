use std::sync::{Arc, Mutex};

use super::*;

#[test]
fn duckdb_backend_streams_rows() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE emp (id INTEGER, name VARCHAR, salary DOUBLE);
             INSERT INTO emp VALUES (1, 'Alice', 90000.5);
             INSERT INTO emp VALUES (2, 'Bob', 80000.0);
             INSERT INTO emp VALUES (3, 'Carol', NULL);",
        )
        .unwrap();
        let mut backend = DuckDbBackend::new(Arc::new(Mutex::new(conn)));

        let cols = backend
            .column_names("SELECT * FROM emp LIMIT 0")
            .await
            .unwrap();
        assert_eq!(cols, vec!["id", "name", "salary"]);

        let mut stream = backend
            .open_branch("SELECT id, name, salary FROM emp ORDER BY id", &[])
            .await
            .unwrap();

        let row1 = stream.next_row().await.unwrap().unwrap();
        assert_eq!(row1.values[0].as_deref(), Some("1"));
        assert_eq!(row1.values[1].as_deref(), Some("Alice"));
        assert_eq!(row1.values[2].as_deref(), Some("90000.5"));

        let row2 = stream.next_row().await.unwrap().unwrap();
        assert_eq!(row2.values[0].as_deref(), Some("2"));
        assert_eq!(row2.values[1].as_deref(), Some("Bob"));

        let row3 = stream.next_row().await.unwrap().unwrap();
        assert_eq!(row3.values[2], None, "NULL salary should be None");
        assert!(stream.next_row().await.unwrap().is_none());
    });
}

/// Metadata discovery must not execute result-producing expressions.
#[test]
fn duckdb_column_names_uses_a_zero_row_probe() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        let mut backend = DuckDbBackend::new(Arc::new(Mutex::new(conn)));

        let cols = backend
            .column_names(
                "SELECT i, CASE WHEN i >= 0 THEN error('metadata query consumed a row') END AS boom FROM range(100000000) AS values(i)",
            )
            .await
            .unwrap();

        assert_eq!(cols, vec!["i", "boom"]);
    });
}

#[test]
fn duckdb_backend_parameter_binding() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let conn = duckdb::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE val (n INTEGER);
             INSERT INTO val VALUES (10);
             INSERT INTO val VALUES (20);
             INSERT INTO val VALUES (30);",
        )
        .unwrap();
        let mut backend = DuckDbBackend::new(Arc::new(Mutex::new(conn)));
        let mut stream = backend
            .open_branch("SELECT n FROM val WHERE n > ?", &["15".to_owned()])
            .await
            .unwrap();

        let r1 = stream.next_row().await.unwrap().unwrap();
        let r2 = stream.next_row().await.unwrap().unwrap();
        let mut got = vec![
            r1.values[0].as_deref().unwrap().parse::<i32>().unwrap(),
            r2.values[0].as_deref().unwrap().parse::<i32>().unwrap(),
        ];
        got.sort_unstable();
        assert_eq!(got, vec![20, 30]);
        assert!(stream.next_row().await.unwrap().is_none());
    });
}

#[tokio::test]
async fn duckdb_backend_maps_common_r2rml_scalar_types() {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    let mut backend = DuckDbBackend::new(Arc::new(Mutex::new(conn)));
    let mut stream = backend
        .open_branch(
            "SELECT \
             170141183460469231731687303715884105728::UHUGEINT, \
             12.340::DECIMAL(10,3), \
             DATE '1969-12-31', \
             TIME '01:02:03.004005', \
             TIMESTAMP '1969-12-31 23:59:59.500000', \
             TIMESTAMPTZ '1969-12-31 23:59:59.500000+00'",
            &[],
        )
        .await
        .unwrap();
    let row = stream.next_row().await.unwrap().unwrap();
    assert_eq!(
        row.values,
        vec![
            Some("170141183460469231731687303715884105728".into()),
            Some("12.340".into()),
            Some("1969-12-31".into()),
            Some("01:02:03.004005".into()),
            Some("1969-12-31 23:59:59.500".into()),
            Some("1969-12-31T23:59:59.500Z".into()),
        ]
    );
    assert_eq!(
        row.codes,
        vec![
            Some(XsdTypeCode::Integer),
            Some(XsdTypeCode::Decimal),
            Some(XsdTypeCode::Date),
            Some(XsdTypeCode::Time),
            Some(XsdTypeCode::DateTime),
            Some(XsdTypeCode::DateTime),
        ]
    );
    assert!(stream.next_row().await.unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opening_a_second_branch_never_blocks_the_async_executor() {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    let shared = Arc::new(Mutex::new(conn));
    let mut first_backend = DuckDbBackend::new(Arc::clone(&shared));
    let mut second_backend = DuckDbBackend::new(Arc::clone(&shared));
    let mut first_stream = first_backend
        .open_branch("SELECT range::INTEGER FROM range(4)", &[])
        .await
        .unwrap();
    assert!(first_stream.next_row().await.unwrap().is_some());

    let second =
        tokio::spawn(async move { second_backend.open_branch("SELECT 1", &[]).await.map(drop) });
    tokio::time::timeout(std::time::Duration::from_secs(1), second)
        .await
        .expect("open_branch must not synchronously wait for the shared connection")
        .expect("second branch task should join")
        .expect("second branch should open");
    drop(first_stream);
}

#[tokio::test]
async fn configured_value_limit_rejects_before_copying_text() {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    let mut backend = DuckDbBackend::new(Arc::new(Mutex::new(conn))).with_max_value_bytes(4);
    let mut stream = backend.open_branch("SELECT 'hello'", &[]).await.unwrap();
    let error = match stream.next_row().await {
        Err(error) => error,
        Ok(_) => panic!("oversized DuckDB text should be rejected"),
    };
    assert!(
        error
            .to_string()
            .contains("exceeding the configured per-value limit"),
        "{error}"
    );
}

#[tokio::test]
async fn configured_value_limit_covers_blob_lexical_size_and_enum_labels() {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TYPE mood AS ENUM ('happy', 'sad')")
        .unwrap();
    let shared = Arc::new(Mutex::new(conn));

    let mut blob_backend = DuckDbBackend::new(Arc::clone(&shared)).with_max_value_bytes(4);
    let mut blob_stream = blob_backend
        .open_branch("SELECT from_hex('010203')", &[])
        .await
        .unwrap();
    let blob_error = match blob_stream.next_row().await {
        Err(error) => error,
        Ok(_) => panic!("oversized DuckDB blob should be rejected"),
    };
    assert!(blob_error
        .to_string()
        .contains("DuckDB blob value requires 6"));

    let mut enum_backend = DuckDbBackend::new(shared).with_max_value_bytes(4);
    let mut enum_stream = enum_backend
        .open_branch("SELECT 'happy'::mood", &[])
        .await
        .unwrap();
    let enum_error = match enum_stream.next_row().await {
        Err(error) => error,
        Ok(_) => panic!("oversized DuckDB enum should be rejected"),
    };
    assert!(enum_error
        .to_string()
        .contains("DuckDB enum value requires 5"));
}

#[tokio::test]
async fn streaming_crosses_arrow_batch_boundaries() {
    const ROWS: usize = 5_000;
    let conn = duckdb::Connection::open_in_memory().unwrap();
    let mut backend = DuckDbBackend::new(Arc::new(Mutex::new(conn)));
    let mut stream = backend
        .open_branch("SELECT i::INTEGER FROM range(5000) values_(i)", &[])
        .await
        .unwrap();

    let mut count = 0usize;
    while let Some(row) = stream.next_row().await.unwrap() {
        let expected = count.to_string();
        assert_eq!(row.values[0].as_deref(), Some(expected.as_str()));
        count += 1;
    }
    assert_eq!(count, ROWS);
}

#[tokio::test]
async fn typed_multi_parameter_binding_uses_all_positions() {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    let mut backend = DuckDbBackend::new(Arc::new(Mutex::new(conn)));
    let mut stream = backend
        .open_branch(
            "SELECT ?::BOOLEAN, ?::DECIMAL(8,2), ?::DATE",
            &["true".into(), "12.50".into(), "2024-02-29".into()],
        )
        .await
        .unwrap();
    let row = stream.next_row().await.unwrap().unwrap();
    assert_eq!(
        row.values,
        vec![
            Some("true".into()),
            Some("12.50".into()),
            Some("2024-02-29".into())
        ]
    );
    assert_eq!(
        row.codes,
        vec![
            Some(XsdTypeCode::Boolean),
            Some(XsdTypeCode::Decimal),
            Some(XsdTypeCode::Date),
        ]
    );
}

#[tokio::test]
async fn unsupported_nested_values_fail_explicitly() {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    let mut backend = DuckDbBackend::new(Arc::new(Mutex::new(conn)));
    let mut stream = backend.open_branch("SELECT [1, 2]", &[]).await.unwrap();
    let error = match stream.next_row().await {
        Err(error) => error,
        Ok(_) => panic!("DuckDB nested values should be unsupported"),
    };
    assert!(matches!(error, Error::Unsupported(_)), "{error}");
}

/// Regression for blocking the async executor on a shared connection mutex.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn column_names_concurrent_deadlock_regression() {
    const N_CONTENDERS: usize = 4;
    const ROWS: usize = 20_000;

    let conn = duckdb::Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!(
        "CREATE TABLE emp (id INTEGER, name VARCHAR);
         INSERT INTO emp SELECT range, 'Person' || range FROM range({ROWS});"
    ))
    .unwrap();
    let shared = Arc::new(Mutex::new(conn));

    let run = async {
        let mut backend = DuckDbBackend::new(Arc::clone(&shared));
        backend
            .column_names("SELECT id, name FROM emp LIMIT 0")
            .await
            .expect("winner column_names");
        let mut stream = backend
            .open_branch("SELECT id, name FROM emp ORDER BY id", &[])
            .await
            .expect("winner open_branch");
        let row1 = stream.next_row().await.expect("winner row 1");
        assert!(row1.is_some(), "expected at least one row");

        let mut handles = Vec::with_capacity(N_CONTENDERS);
        for _ in 0..N_CONTENDERS {
            let shared = Arc::clone(&shared);
            handles.push(tokio::spawn(async move {
                let mut backend = DuckDbBackend::new(shared);
                let cols = backend
                    .column_names("SELECT id, name FROM emp LIMIT 0")
                    .await
                    .expect("contender column_names");
                assert_eq!(cols, vec!["id", "name"]);
            }));
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut n = 1usize;
        while stream.next_row().await.expect("winner next_row").is_some() {
            n += 1;
        }
        assert_eq!(n, ROWS);

        for handle in handles {
            handle.await.expect("contender task join");
        }
    };

    tokio::time::timeout(std::time::Duration::from_secs(30), run)
        .await
        .expect(
            "column_names deadlock regression: contenders blocked every async worker while a cursor held the connection mutex",
        );
}

/// Dropping a stream interrupts a query before its first row and releases the
/// shared connection for the next operation.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_stream_interrupts_blocking_cursor() {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    let shared = Arc::new(Mutex::new(conn));
    let mut backend = DuckDbBackend::new(Arc::clone(&shared));
    let stream = backend
        .open_branch(
            "SELECT sum(sin(i::DOUBLE)) FROM range(1000000000) values_(i)",
            &[],
        )
        .await
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if shared.try_lock().is_err() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("DuckDB cursor worker should acquire the connection");
    drop(stream);

    let shared_for_probe = Arc::clone(&shared);
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::task::spawn_blocking(move || {
            let connection = shared_for_probe
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            connection.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
        }),
    )
    .await
    .expect("interrupted DuckDB cursor should release the connection")
    .expect("probe worker should join")
    .expect("connection should remain usable after interruption");
}

/// Cancellation after delivery has begun must release a producer blocked on
/// the cap-1 channel and must not leak its interrupt into the next query.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_stream_after_first_row_releases_connection_without_interrupt_spill() {
    let conn = duckdb::Connection::open_in_memory().unwrap();
    let shared = Arc::new(Mutex::new(conn));
    let mut backend = DuckDbBackend::new(Arc::clone(&shared));
    let mut stream = backend
        .open_branch("SELECT i::INTEGER FROM range(1000000000) values_(i)", &[])
        .await
        .unwrap();

    let first = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next_row())
        .await
        .expect("DuckDB stream should deliver its first row")
        .expect("first row should decode")
        .expect("range should not be empty");
    assert_eq!(first.values[0].as_deref(), Some("0"));
    drop(stream);

    let shared_for_probe = Arc::clone(&shared);
    let value = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::task::spawn_blocking(move || {
            let connection = shared_for_probe
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            connection.query_row("SELECT 42", [], |row| row.get::<_, i64>(0))
        }),
    )
    .await
    .expect("canceled streaming query should release the connection")
    .expect("probe worker should join")
    .expect("subsequent query should not inherit the interrupt");
    assert_eq!(value, 42);
}
