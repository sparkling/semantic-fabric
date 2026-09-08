use super::*;
use crate::backend::sqlite::{SqliteBackend, SqliteOwnedBackend, SqliteOwnedConnection};
use crate::backend::{BranchStream, SqlBackend};
use sf_core::query_control::{QueryBudget, QueryControlError, QueryLimits};

const SQL: &str =
    "SELECT __sf_character_key_v1('a', 4) AS key UNION ALL SELECT __sf_character_key_v1('b', 2)";

fn absent(conn: &Connection) {
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_function_list WHERE name = ?",
            [NAME],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0, "callback must not survive query cleanup");
}

#[test]
fn shared_decoder_preserves_unicode_nul_numeric_and_overlong_text() {
    for (value, width, expected) in [
        (ValueRef::Text("é🐈".as_bytes()), 4, "é🐈  "),
        (ValueRef::Text(b"x\0y"), 4, "x\0y "),
        (ValueRef::Text(b"longer"), 2, "longer"),
        (ValueRef::Text(b""), 2, "  "),
        (ValueRef::Text(b"    "), 2, "    "),
        (ValueRef::Integer(7), 3, "7  "),
        (ValueRef::Real(1.5), 4, "1.5 "),
    ] {
        assert_eq!(
            character(value, width, None).unwrap().as_deref(),
            Some(expected)
        );
    }
    assert_eq!(character(ValueRef::Null, 4, None).unwrap(), None);
    assert!(matches!(
        character(ValueRef::Blob(b"a"), 4, None),
        Err(Error::Marshal(_))
    ));
    assert!(matches!(
        character(ValueRef::Text(b"\xff"), 4, None),
        Err(Error::Marshal(_))
    ));
}

#[tokio::test]
async fn borrowing_cleanup_on_eof_drop_and_marshal_failure() {
    let conn = Connection::open_in_memory().unwrap();
    for drain in [false, true] {
        let mut backend = SqliteBackend::new(&conn);
        let mut rows = backend
            .open_branch_with_decoder(SQL, &[], None, true)
            .await
            .unwrap();
        assert_eq!(
            rows.next_row().await.unwrap().unwrap().values[0].as_deref(),
            Some("a   ")
        );
        if drain {
            assert_eq!(
                rows.next_row().await.unwrap().unwrap().values[0].as_deref(),
                Some("b ")
            );
            assert!(rows.next_row().await.unwrap().is_none());
        }
        drop(rows);
        absent(&conn); // backend still holds an inactive prepared statement
    }
    let mut backend = SqliteBackend::new(&conn);
    let mut rows = backend
        .open_branch_with_decoder("SELECT __sf_character_key_v1(X'ff',4)", &[], None, true)
        .await
        .unwrap();
    assert!(matches!(rows.next_row().await, Err(Error::Marshal(_))));
    drop(rows);
    absent(&conn);
    assert!(backend
        .open_branch_with_decoder(
            "SELECT __sf_character_key_v1('a',4) FROM missing",
            &[],
            None,
            true
        )
        .await
        .is_err());
    absent(&conn);
}

#[test]
fn collisions_do_not_replace_any_existing_arity_or_case() {
    for arity in [-1, 1, 2] {
        let conn = Connection::open_in_memory().unwrap();
        conn.create_scalar_function(
            "__SF_CHARACTER_KEY_V1",
            arity,
            FunctionFlags::SQLITE_UTF8,
            |_| Ok(73),
        )
        .unwrap();
        assert!(matches!(
            CharacterKeyGuard::install(&conn, true, None),
            Err(Error::Emit(_))
        ));
        let original: i64 = conn
            .query_row(
                if arity == 1 {
                    "SELECT __sf_character_key_v1(0)"
                } else {
                    "SELECT __sf_character_key_v1(0,0)"
                },
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(original, 73);
    }
}

#[tokio::test]
async fn authored_sql_text_cannot_activate_the_engine_callback() {
    let conn = Connection::open_in_memory().unwrap();
    conn.create_scalar_function(NAME, 2, FunctionFlags::SQLITE_UTF8, |_| Ok(73))
        .unwrap();
    conn.execute_batch("CREATE TABLE __sf_character_key_v1_data(value TEXT); INSERT INTO __sf_character_key_v1_data VALUES('__sf_character_key_v1');").unwrap();
    let mut backend = SqliteBackend::new(&conn);
    let mut rows = backend
        .open_branch(
            "SELECT value, __sf_character_key_v1(0,0) FROM __sf_character_key_v1_data",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        rows.next_row().await.unwrap().unwrap().values,
        vec![Some(NAME.into()), Some("73".into())]
    );
    assert!(rows.next_row().await.unwrap().is_none());
}

#[tokio::test]
async fn owned_budget_failure_is_typed_and_does_not_retain_request_state() {
    let owned = SqliteOwnedConnection::new(Connection::open_in_memory().unwrap());
    let budget = Arc::new(QueryBudget::new(QueryLimits::new(
        u64::MAX,
        3,
        u64::MAX,
        u64::MAX,
    )));
    let weak = Arc::downgrade(&budget);
    let mut backend =
        SqliteOwnedBackend::new_controlled_leased(owned.acquire().await.unwrap(), budget.clone());
    let mut rows = backend
        .open_branch_with_decoder(SQL, &[], None, true)
        .await
        .unwrap();
    assert!(matches!(
        rows.next_row().await,
        Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
    ));
    assert_eq!(budget.consumed(QueryCharge::SourceWork), 0);
    drop(rows);
    drop(backend);
    drop(budget);
    let _next = owned.acquire().await.unwrap();
    assert!(weak.upgrade().is_none());
    absent(&owned.raw_connection().lock().unwrap());
}

#[tokio::test]
async fn owned_cleanup_on_success_and_early_receiver_drop() {
    for drain in [false, true] {
        let owned = SqliteOwnedConnection::new(Connection::open_in_memory().unwrap());
        let budget = Arc::new(QueryBudget::new(QueryLimits::new(
            u64::MAX,
            u64::MAX,
            u64::MAX,
            u64::MAX,
        )));
        let weak = Arc::downgrade(&budget);
        let mut backend = SqliteOwnedBackend::new_controlled_leased(
            owned.acquire().await.unwrap(),
            budget.clone(),
        );
        let mut rows = backend
            .open_branch_with_decoder(SQL, &[], None, true)
            .await
            .unwrap();
        assert!(rows.next_row().await.unwrap().is_some());
        if drain {
            assert!(rows.next_row().await.unwrap().is_some());
            assert!(rows.next_row().await.unwrap().is_none());
        }
        drop(rows);
        drop(backend);
        drop(budget);
        let _next = owned.acquire().await.unwrap();
        assert!(weak.upgrade().is_none());
        absent(&owned.raw_connection().lock().unwrap());
    }
}

#[test]
fn removal_failure_leaves_only_inert_callback_and_rejects_reinstallation() {
    let conn = Connection::open_in_memory().unwrap();
    let budget = Arc::new(QueryBudget::new(QueryLimits::new(
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    )));
    let weak = Arc::downgrade(&budget);
    let mut key = CharacterKeyGuard::install(&conn, true, Some(budget.clone())).unwrap();
    let mut statement = conn.prepare(SQL).unwrap();
    let mut rows = statement.query([]).unwrap();
    assert!(rows.next().unwrap().is_some());
    // Simulate a raw out-of-lease cursor. Normal adapters reset/drop it first.
    assert!(key.finish().is_err());
    drop(budget);
    assert!(weak.upgrade().is_none());
    drop(rows);
    drop(statement);
    assert!(conn.query_row(SQL, [], |r| r.get::<_, String>(0)).is_err());
    assert!(CharacterKeyGuard::install(&conn, true, None).is_err());
}
