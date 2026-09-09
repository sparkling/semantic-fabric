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
            "SELECT count(*) FROM pragma_function_list WHERE name=? COLLATE NOCASE OR name='__sf_numeric_cmp_v1' COLLATE NOCASE OR name='__sf_iri_key_v1' COLLATE NOCASE",
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
    for (name, arity) in [
        (NAME, -1),
        (NAME, 1),
        (NAME, 3),
        ("__SF_NUMERIC_CMP_V1", -1),
        ("__SF_NUMERIC_CMP_V1", 1),
        ("__SF_NUMERIC_CMP_V1", 5),
        ("__SF_IRI_KEY_V1", -1),
        ("__SF_IRI_KEY_V1", 1),
        ("__SF_IRI_KEY_V1", 2),
    ] {
        let conn = Connection::open_in_memory().unwrap();
        conn.create_scalar_function(
            name.to_ascii_uppercase().as_str(),
            arity,
            FunctionFlags::SQLITE_UTF8,
            |_| Ok(73),
        )
        .unwrap();
        assert!(CharacterKeyGuard::install_lexical(&conn, true, None).is_err());
        let args = std::iter::repeat_n("0", arity.max(1) as usize)
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("SELECT {name}({args})");
        assert_eq!(
            conn.query_row(&sql, [], |r| r.get::<_, i64>(0)).unwrap(),
            73
        );
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

#[test]
fn numeric_callback_is_query_owned_charged_and_null_preserving() {
    let dtype = "http://www.w3.org/2001/XMLSchema#integer";
    let sql = format!("SELECT __sf_numeric_cmp_v1('10','{dtype}','9','{dtype}',4)");
    let charge = 128 + 3 + 2 * dtype.len() as u64;
    for limit in [charge - 1, charge] {
        let conn = Connection::open_in_memory().unwrap();
        assert!(conn.prepare(&sql).is_err());
        let budget = Arc::new(QueryBudget::new(QueryLimits::new(
            u64::MAX,
            limit,
            u64::MAX,
            u64::MAX,
        )));
        let weak = Arc::downgrade(&budget);
        let mut guard =
            CharacterKeyGuard::install_lexical(&conn, true, Some(budget.clone())).unwrap();
        let result = conn.query_row(&sql, [], |r| r.get::<_, String>(0));
        if limit < charge {
            assert!(matches!(
                guard.map_error(result.unwrap_err().into()),
                Error::QueryControl(QueryControlError::SourceWorkExceeded)
            ));
            assert_eq!(budget.consumed(QueryCharge::SourceWork), 0);
        } else {
            assert_eq!(result.unwrap(), "1");
            assert_eq!(budget.consumed(QueryCharge::SourceWork), charge);
        }
        guard.finish().unwrap();
        drop(budget);
        assert!(weak.upgrade().is_none());
        absent(&conn);
    }
    let conn = Connection::open_in_memory().unwrap();
    let mut guard = CharacterKeyGuard::install_lexical(&conn, true, None).unwrap();
    for (sql, expected) in [
        (
            format!("SELECT __sf_numeric_cmp_v1(NULL,'{dtype}','9','{dtype}',4)"),
            None,
        ),
        (
            format!("SELECT __sf_numeric_cmp_v1('invalid','{dtype}','9','{dtype}',4)"),
            None,
        ),
        ("SELECT __sf_lexical_key_v1('001',20,-1)".into(), Some("1")),
        (
            "SELECT __sf_lexical_key_v1(-0.0,16,-1)".into(),
            Some("-0.0E0"),
        ),
    ] {
        assert_eq!(
            conn.query_row(&sql, [], |r| r.get::<_, Option<String>>(0))
                .unwrap()
                .as_deref(),
            expected
        );
    }
    for sql in [
        format!("SELECT __sf_numeric_cmp_v1(1,'{dtype}','9','{dtype}',4)"),
        format!("SELECT __sf_numeric_cmp_v1('1','{dtype}','9','{dtype}',6)"),
    ] {
        let error = conn
            .query_row(&sql, [], |r| r.get::<_, String>(0))
            .unwrap_err();
        assert!(matches!(guard.map_error(error.into()), Error::Marshal(_)));
    }
    guard.finish().unwrap();
    absent(&conn);
}

#[test]
fn iri_callback_resolves_without_normalizing_absolute_iris_and_releases_state() {
    let conn = Connection::open_in_memory().unwrap();
    let sql = "SELECT __sf_iri_key_v1(?1, ?2)";
    assert!(conn.prepare(sql).is_err());
    let mut guard = CharacterKeyGuard::install_lexical(&conn, true, None).unwrap();
    for (value, base, expected) in [
        (Some("AB"), Some("http://ex/"), Some("http://ex/AB")),
        (Some("../AB"), Some("http://ex/dir/"), Some("http://ex/AB")),
        (
            Some("//other/AB"),
            Some("http://ex/"),
            Some("http://other/AB"),
        ),
        (Some("#x"), Some("http://ex/AB"), Some("http://ex/AB#x")),
        (Some("http://ex/a/../AB"), None, Some("http://ex/a/../AB")),
        (Some("http://ex/%ab"), None, Some("http://ex/%ab")),
        (None, Some("http://ex/"), None),
    ] {
        assert_eq!(
            conn.query_row(sql, [value, base], |r| r.get::<_, Option<String>>(0))
                .unwrap()
                .as_deref(),
            expected
        );
    }
    for (value, base) in [
        ("AB", None),
        ("bad value", Some("http://ex/")),
        ("x\0y", Some("http://ex/")),
    ] {
        let error = conn
            .query_row(sql, [Some(value), base], |r| r.get::<_, String>(0))
            .unwrap_err();
        assert!(matches!(guard.map_error(error.into()), Error::Marshal(_)));
    }
    guard.finish().unwrap();
    absent(&conn);
}

#[test]
fn iri_callback_charges_value_and_base_before_resolving() {
    let conn = Connection::open_in_memory().unwrap();
    let base = "http://ex/";
    let charge = 128 + 2 + base.len() as u64;
    for limit in [charge - 1, charge] {
        let budget = Arc::new(QueryBudget::new(QueryLimits::new(
            u64::MAX,
            limit,
            u64::MAX,
            u64::MAX,
        )));
        let weak = Arc::downgrade(&budget);
        let mut guard =
            CharacterKeyGuard::install_lexical(&conn, true, Some(budget.clone())).unwrap();
        let result = conn.query_row("SELECT __sf_iri_key_v1('AB', ?)", [base], |r| {
            r.get::<_, String>(0)
        });
        if limit < charge {
            assert!(matches!(
                guard.map_error(result.unwrap_err().into()),
                Error::QueryControl(QueryControlError::SourceWorkExceeded)
            ));
            assert_eq!(budget.consumed(QueryCharge::SourceWork), 0);
        } else {
            assert_eq!(result.unwrap(), "http://ex/AB");
            assert_eq!(budget.consumed(QueryCharge::SourceWork), charge);
        }
        guard.finish().unwrap();
        drop(budget);
        assert!(weak.upgrade().is_none());
        absent(&conn);
    }
}
