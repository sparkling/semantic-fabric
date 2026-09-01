//! Live PostgreSQL regression evidence for namespace-safe catalogue snapshots.

use sf_sql::introspect::{
    introspect_postgres, introspect_postgres_all, introspect_postgres_public_snapshot,
};

/// M4 wave-2 finding 4 RECEIPT: batched (`introspect_postgres_all`, 5 round
/// trips total) vs a per-table loop (`introspect_postgres` called once per
/// table, 5*N round trips) over 20 real tables — timed, and asserted
/// byte-identical. Gate-skips cleanly when no PostgreSQL server is reachable
/// (matches the crate's live-PG test convention, e.g. `backend::pg::tests`).
#[tokio::test]
async fn introspect_postgres_all_matches_per_table_loop_and_is_faster() {
    use tokio_postgres::NoTls;

    let base = std::env::var("SF_PG_URL").unwrap_or_else(|_| {
        let user = std::env::var("USER").unwrap_or_else(|_| "postgres".to_owned());
        format!("host=localhost port=5432 user={user}")
    });
    let conn_str = format!("{base} dbname=postgres");
    let Ok((client, connection)) = tokio_postgres::connect(&conn_str, NoTls).await else {
        eprintln!(
            "SKIP introspect_postgres_all_matches_per_table_loop_and_is_faster: \
             no live PostgreSQL reachable"
        );
        return;
    };
    tokio::spawn(async move {
        let _ = connection.await;
    });

    let db = format!("sf_sql_introspect_batch_test_{}", std::process::id());
    let _ = client
        .batch_execute(&format!("DROP DATABASE IF EXISTS {db}"))
        .await;
    client
        .batch_execute(&format!("CREATE DATABASE {db}"))
        .await
        .expect("create test db");

    let conn_str2 = format!("{base} dbname={db}");
    let (mut client2, connection2) = tokio_postgres::connect(&conn_str2, NoTls)
        .await
        .expect("connect to test db");
    tokio::spawn(async move {
        let _ = connection2.await;
    });

    // 20 tables, each with a PK, a UNIQUE column, and (past the first) an FK
    // to the previous table — so columns/keys/FKs/stats all have real work.
    const N: usize = 20;
    let mut ddl = String::new();
    for i in 0..N {
        let fk_col = if i > 0 {
            format!(", prev_id INTEGER REFERENCES t{}(id)", i - 1)
        } else {
            String::new()
        };
        ddl.push_str(&format!(
            "CREATE TABLE t{i} (id INTEGER PRIMARY KEY, code TEXT UNIQUE NOT NULL, \
             val INTEGER NOT NULL{fk_col});\n"
        ));
        let (extra_col, extra_val) = if i > 0 {
            (", prev_id", ", g")
        } else {
            ("", "")
        };
        ddl.push_str(&format!(
            "INSERT INTO t{i} (id, code, val{extra_col}) \
             SELECT g, 'code' || g, g % 7{extra_val} FROM generate_series(1, 50) AS g;\n"
        ));
    }
    ddl.push_str("ANALYZE;\n");
    ddl.push_str(
        "CREATE SCHEMA hostile;\n\
         CREATE TABLE hostile.t0 (hostile_id BIGINT PRIMARY KEY, payload BYTEA);\n\
         INSERT INTO hostile.t0 VALUES (1, decode('00', 'hex'));\n\
         ANALYZE hostile.t0;\n",
    );
    client2.batch_execute(&ddl).await.expect("seed 20 tables");

    let mut names: Vec<String> = (0..N).map(|i| format!("t{i}")).collect();
    names.sort();

    // OLD serve shape: one enumeration followed by this per-table loop.
    let start = std::time::Instant::now();
    let mut old_schemas = Vec::with_capacity(N);
    for name in &names {
        old_schemas.push(
            introspect_postgres(&client2, name)
                .await
                .expect("per-table introspect"),
        );
    }
    let old_elapsed = start.elapsed();

    // NEW shape: 5 set-based round trips total, regardless of N.
    let start = std::time::Instant::now();
    let new_schemas = introspect_postgres_all(&client2, &names)
        .await
        .expect("batched introspect");
    let new_elapsed = start.elapsed();

    eprintln!("introspect {N} PG tables: per_table_loop={old_elapsed:?} batched={new_elapsed:?}");
    assert_eq!(
        old_schemas, new_schemas,
        "batched introspection must match the per-table loop byte-for-byte"
    );

    let snapshot_schemas = introspect_postgres_public_snapshot(&mut client2)
        .await
        .expect("coherent public snapshot");
    assert_eq!(
        new_schemas, snapshot_schemas,
        "public snapshot must exclude the hostile same-name relation"
    );

    client2
        .batch_execute(
            "CREATE TABLE hostile.parent (id INTEGER PRIMARY KEY); \
             CREATE TABLE public.cross_schema_child ( \
                 id INTEGER PRIMARY KEY, \
                 parent_id INTEGER REFERENCES hostile.parent(id) \
             );",
        )
        .await
        .expect("create cross-schema FK fixture");
    let error = introspect_postgres(&client2, "cross_schema_child")
        .await
        .expect_err("unrepresentable qualified parent must fail closed");
    assert!(error.to_string().contains("schema-qualified"));

    drop(client2);
    let _ = client
        .batch_execute(&format!("DROP DATABASE IF EXISTS {db} WITH (FORCE)"))
        .await;
}
