use super::super::text_key::CharacterKeyGuard;
use super::*;
use crate::backend::sqlite::{SqliteBackend, SqliteOwnedBackend, SqliteOwnedConnection};
use crate::backend::{BranchStream, SqlBackend};
use rusqlite::{functions::FunctionFlags, Connection};
use sf_core::query_control::{QueryBudget, QueryControlError, QueryLimits};
use std::sync::Arc;

const NAME: &str = "__sf_lexical_key_v1";
const SQL: &str =
    "SELECT __sf_lexical_key_v1(-0.0,0,-1) UNION ALL SELECT __sf_lexical_key_v1(X'abff',0,-1)";

fn absent(conn: &Connection) {
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_function_list WHERE name=? COLLATE NOCASE",
            [NAME],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn lexical_key_matches_row_decoder_without_canonicalizing_natural_values() {
    for (value, declared, padding, expected) in [
        (ValueRef::Real(-0.0), None, None, "-0"),
        (ValueRef::Integer(1), None, None, "1"),
        (ValueRef::Real(1.0), None, None, "1"),
        (ValueRef::Text(b"x\0y"), None, None, "x\0y"),
        (
            ValueRef::Text("é🐈".as_bytes()),
            Some(XsdTypeCode::String),
            Some(4),
            "é🐈  ",
        ),
        (ValueRef::Blob(b"\xab\xff"), None, None, "ABFF"),
        (ValueRef::Integer(1), Some(XsdTypeCode::Boolean), None, "1"),
        (
            ValueRef::Text(b"001"),
            Some(XsdTypeCode::Integer),
            None,
            "001",
        ),
        (
            ValueRef::Text(b"2026-09-08 00:00:00"),
            Some(XsdTypeCode::DateTime),
            None,
            "2026-09-08 00:00:00",
        ),
    ] {
        assert_eq!(
            lexical(value, SqliteDecode { declared, padding }, None)
                .unwrap()
                .as_deref(),
            Some(expected)
        );
    }
    let decode = SqliteDecode {
        declared: Some(XsdTypeCode::String),
        padding: None,
    };
    assert!(matches!(
        lexical(ValueRef::Blob(b"a"), decode, None),
        Err(Error::Marshal(_))
    ));
    assert!(matches!(
        lexical(ValueRef::Text(b"\xff"), decode, None),
        Err(Error::Marshal(_))
    ));
    assert_eq!(lexical(ValueRef::Null, decode, None).unwrap(), None);
}

#[tokio::test]
async fn borrowing_lexical_callback_is_opt_in_and_cleans_up_on_all_terminal_paths() {
    for drain in [false, true] {
        let conn = Connection::open_in_memory().unwrap();
        let mut backend = SqliteBackend::new(&conn);
        assert!(backend.open_branch(SQL, &[]).await.is_err());
        absent(&conn);
        let mut rows = backend
            .open_branch_with_identity(SQL, &[], None, false, true)
            .await
            .unwrap();
        assert_eq!(
            rows.next_row().await.unwrap().unwrap().values[0].as_deref(),
            Some("-0")
        );
        if drain {
            assert_eq!(
                rows.next_row().await.unwrap().unwrap().values[0].as_deref(),
                Some("ABFF")
            );
            assert!(rows.next_row().await.unwrap().is_none());
        }
        drop(rows);
        absent(&conn);
        for sql in [
            "SELECT __sf_lexical_key_v1(X'ff',1,-1)",
            "SELECT __sf_lexical_key_v1(1,10,-1)",
            "SELECT __sf_lexical_key_v1(1,0,-2)",
        ] {
            let mut rows = backend
                .open_branch_with_identity(sql, &[], None, false, true)
                .await
                .unwrap();
            assert!(matches!(rows.next_row().await, Err(Error::Marshal(_))));
            drop(rows);
            absent(&conn);
        }
        assert!(backend
            .open_branch_with_identity("SELECT missing FROM absent", &[], None, true, true)
            .await
            .is_err());
        absent(&conn);
    }
}

#[test]
fn lexical_callback_does_not_replace_application_callbacks() {
    for arity in [-1, 1, 3] {
        let conn = Connection::open_in_memory().unwrap();
        conn.create_scalar_function(
            "__SF_LEXICAL_KEY_V1",
            arity,
            FunctionFlags::SQLITE_UTF8,
            |_| Ok(73),
        )
        .unwrap();
        assert!(CharacterKeyGuard::install_lexical(&conn, true, None).is_err());
        let sql = if arity == 1 {
            "SELECT __sf_lexical_key_v1(0)"
        } else {
            "SELECT __sf_lexical_key_v1(0,0,0)"
        };
        assert_eq!(conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap(), 73);
    }
}

#[tokio::test]
async fn owned_lexical_budget_and_drop_release_callback_and_request() {
    for work in [31, 1000] {
        for drain in [false, true] {
            let owned = SqliteOwnedConnection::new(Connection::open_in_memory().unwrap());
            let budget = Arc::new(QueryBudget::new(QueryLimits::new(
                u64::MAX,
                work,
                u64::MAX,
                u64::MAX,
            )));
            let weak = Arc::downgrade(&budget);
            let mut backend = SqliteOwnedBackend::new_controlled_leased(
                owned.acquire().await.unwrap(),
                budget.clone(),
            );
            let mut rows = backend
                .open_branch_with_identity(SQL, &[], None, true, true)
                .await
                .unwrap();
            let first = rows.next_row().await;
            if work == 31 {
                assert!(matches!(
                    first,
                    Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                ));
                assert_eq!(budget.consumed(QueryCharge::SourceWork), 0);
            } else {
                assert!(first.unwrap().is_some());
                if drain {
                    assert!(rows.next_row().await.unwrap().is_some());
                    assert!(rows.next_row().await.unwrap().is_none());
                }
            }
            drop(rows);
            drop(backend);
            drop(budget);
            let _lease = owned.acquire().await.unwrap();
            assert!(weak.upgrade().is_none());
            absent(&owned.raw_connection().lock().unwrap());
        }
    }
}
