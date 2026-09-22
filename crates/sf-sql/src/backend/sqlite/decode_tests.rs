use super::*;
use crate::backend::sqlite::SqliteBackend;
use crate::backend::{BranchStream, SqlBackend};
use rusqlite::Connection;
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use std::sync::atomic::{AtomicUsize, Ordering};

fn budget(units: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX))
}

/// The row/code-vector admission overhead `row()` charges before any per-cell
/// bound, for a single-column projection: `charge(1)` for each of the two
/// `work.vector(1)` calls, their `product(1, size_of::<T>())` allocation charge,
/// and the per-cell `work.charge(1)` visit -- computed from actual struct layout
/// (platform-independent), never guessed, mirroring the PG live-row test's
/// `row_units` pattern.
fn single_column_overhead() -> u64 {
    2 + std::mem::size_of::<Option<String>>() as u64
        + std::mem::size_of::<Option<XsdTypeCode>>() as u64
        + 1
}

// --- storage_class_code -----------------------------------------------

#[test]
fn storage_class_code_integer_maps_to_xsd_integer() {
    assert_eq!(
        storage_class_code(&ValueRef::Integer(42)),
        Some(XsdTypeCode::Integer)
    );
}

#[test]
fn storage_class_code_real_maps_to_xsd_double() {
    assert_eq!(
        storage_class_code(&ValueRef::Real(1.5)),
        Some(XsdTypeCode::Double)
    );
}

#[test]
fn storage_class_code_blob_maps_to_hex_binary() {
    assert_eq!(
        storage_class_code(&ValueRef::Blob(&[1, 2, 3])),
        Some(XsdTypeCode::HexBinary)
    );
}

#[test]
fn storage_class_code_text_and_null_carry_no_implied_type() {
    assert_eq!(storage_class_code(&ValueRef::Text(b"hi")), None);
    assert_eq!(storage_class_code(&ValueRef::Null), None);
}

// --- lexical --------------------------------------------------------------

#[test]
fn lexical_null_is_none() {
    assert_eq!(lexical(ValueRef::Null).unwrap(), None);
}

#[test]
fn lexical_integer_and_real_render_via_to_string() {
    assert_eq!(lexical(ValueRef::Integer(7)).unwrap(), Some("7".to_owned()));
    assert_eq!(
        lexical(ValueRef::Real(1.5)).unwrap(),
        Some("1.5".to_owned())
    );
}

#[test]
fn lexical_valid_utf8_text_passes_through() {
    assert_eq!(
        lexical(ValueRef::Text(b"hello")).unwrap(),
        Some("hello".to_owned())
    );
}

#[test]
fn lexical_non_utf8_text_is_a_hard_marshal_error() {
    let invalid = &[0xff, 0xfe][..];
    let err = lexical(ValueRef::Text(invalid)).unwrap_err();
    assert!(
        matches!(err, Error::Marshal(_)),
        "expected Marshal, got {err:?}"
    );
}

#[test]
fn lexical_bare_blob_is_a_hard_marshal_error() {
    // lexical() (unlike lexical_typed()) has no target-type context, so it
    // can never soundly decide a BLOB is hexBinary -- always errors.
    let err = lexical(ValueRef::Blob(&[1, 2, 3])).unwrap_err();
    assert!(
        matches!(err, Error::Marshal(_)),
        "expected Marshal, got {err:?}"
    );
}

// --- lexical_typed --------------------------------------------------------

#[test]
fn lexical_typed_blob_with_hexbinary_target_encodes_uppercase_hex() {
    let out = lexical_typed(
        ValueRef::Blob(&[0xde, 0xad, 0xbe, 0xef]),
        Some(XsdTypeCode::HexBinary),
        SourceWork::new(None),
    )
    .unwrap();
    assert_eq!(out, Some("DEADBEEF".to_owned()));
}

#[test]
fn lexical_typed_blob_without_hexbinary_target_still_errors() {
    // A BLOB feeding a non-hexBinary-typed column (or no declared type) has
    // no sound rendering -- falls through to lexical()'s hard error.
    let err = lexical_typed(ValueRef::Blob(&[1, 2, 3]), None, SourceWork::new(None)).unwrap_err();
    assert!(matches!(err, Error::Marshal(_)));
    let err2 = lexical_typed(
        ValueRef::Blob(&[1, 2, 3]),
        Some(XsdTypeCode::String),
        SourceWork::new(None),
    )
    .unwrap_err();
    assert!(matches!(err2, Error::Marshal(_)));
}

#[test]
fn lexical_typed_non_blob_delegates_to_lexical_regardless_of_code() {
    assert_eq!(
        lexical_typed(
            ValueRef::Integer(9),
            Some(XsdTypeCode::HexBinary),
            SourceWork::new(None)
        )
        .unwrap(),
        Some("9".to_owned())
    );
}

// --- row(): exact/N-1 boundaries per storage class -------------------------

#[test]
fn row_charges_exact_and_n_minus_one_boundaries_for_each_storage_class() {
    // (sql, per-cell bound) -- bound matches lexical_typed's own match arms.
    for (sql, bound) in [
        ("SELECT 42", 20u64),     // Integer
        ("SELECT -0.0", 327u64),  // Real: f64::to_string() worst case
        ("SELECT 'hi'", 2u64),    // Text: byte length
        ("SELECT NULL", 0u64),    // Null
        ("SELECT X'abff'", 4u64), // Blob (non-hexBinary target): byte length
    ] {
        let units = single_column_overhead() + bound;
        for cap in [units - 1, units] {
            let conn = Connection::open_in_memory().unwrap();
            let mut stmt = conn.prepare(sql).unwrap();
            let mut rows = stmt.query([]).unwrap();
            let r = rows.next().unwrap().unwrap();
            let control = budget(cap);
            let result = row(&r, &[None], &[None], 1, SourceWork::new(Some(&control)));
            if cap == units {
                assert!(result.is_ok(), "sql={sql} cap={cap}");
                assert_eq!(control.consumed(QueryCharge::SourceWork), units);
            } else {
                assert!(
                    matches!(
                        result,
                        Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                    ),
                    "sql={sql} cap={cap}"
                );
                assert_eq!(
                    control.checkpoint(),
                    Err(QueryControlError::SourceWorkExceeded)
                );
            }
        }
    }
}

// --- row(): sticky cancel/deadline at every charge point --------------------

struct Stop {
    budget: QueryBudget,
    at: usize,
    calls: AtomicUsize,
    reason: QueryControlError,
}

impl QueryControl for Stop {
    fn checkpoint(&self) -> std::result::Result<(), QueryControlError> {
        self.budget.checkpoint()
    }

    fn terminate(&self, reason: QueryControlError) -> QueryControlError {
        self.budget.terminate(reason)
    }

    fn consume(&self, kind: QueryCharge, units: u64) -> std::result::Result<(), QueryControlError> {
        self.budget.consume(kind, units)?;
        if self.calls.fetch_add(1, Ordering::Relaxed) + 1 == self.at {
            self.budget.terminate(self.reason);
        }
        self.budget.checkpoint()
    }
}

#[test]
fn every_row_charge_observes_sticky_cancel_and_deadline() {
    let conn = Connection::open_in_memory().unwrap();
    let mut stmt = conn.prepare("SELECT 42, -0.0, 'hi'").unwrap();
    let mut rows = stmt.query([]).unwrap();
    let r = rows.next().unwrap().unwrap();
    let decl_codes = [None, None, None];
    let pads = [None, None, None];

    let measured = Stop {
        budget: budget(u64::MAX),
        at: usize::MAX,
        calls: AtomicUsize::new(0),
        reason: QueryControlError::Cancelled,
    };
    row(&r, &decl_codes, &pads, 3, SourceWork::new(Some(&measured))).unwrap();
    let charge_count = measured.calls.load(Ordering::Relaxed);
    assert!(charge_count >= 6); // 2 vector admissions + 3 per-cell visits + 3 bound charges, at least

    for reason in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for at in 1..=charge_count {
            let control = Stop {
                budget: budget(u64::MAX),
                at,
                calls: AtomicUsize::new(0),
                reason,
            };
            let mut stmt = conn.prepare("SELECT 42, -0.0, 'hi'").unwrap();
            let mut rows = stmt.query([]).unwrap();
            let r = rows.next().unwrap().unwrap();
            assert!(
                matches!(
                    row(&r, &decl_codes, &pads, 3, SourceWork::new(Some(&control))),
                    Err(Error::QueryControl(found)) if found == reason
                ),
                "at={at} reason={reason:?}"
            );
            assert_eq!(control.checkpoint(), Err(reason));
        }
    }
}

// --- controlled-dispatch disconnection witness ------------------------------
//
// A permanent regression for the class of bug fixed by adding
// `SqliteBranch::next_row_controlled`: if dispatch ever again ignored its
// supplied control (e.g. reverted to the trait default, or hard-coded
// `SourceWork::new(None)`), a tiny budget would stop refusing rows and this
// test would fail. Manually reverted and confirmed failing, then restored,
// as part of this change's review.

#[tokio::test]
async fn controlled_dispatch_charges_the_supplied_control_not_a_disconnected_one() {
    let conn = Connection::open_in_memory().unwrap();
    let mut backend = SqliteBackend::new(&conn);
    let mut branch = backend.open_branch("SELECT -0.0", &[]).await.unwrap();
    let control = budget(1);
    assert!(matches!(
        branch.next_row_controlled(&control).await,
        Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
    ));
    assert_eq!(
        control.checkpoint(),
        Err(QueryControlError::SourceWorkExceeded)
    );
}

#[tokio::test]
async fn controlled_dispatch_succeeds_at_the_exact_measured_budget_and_matches_uncontrolled_value()
{
    let units = single_column_overhead() + 327; // Real bound
    let conn = Connection::open_in_memory().unwrap();

    let mut backend = SqliteBackend::new(&conn);
    let mut branch = backend.open_branch("SELECT -0.0", &[]).await.unwrap();
    let control = budget(units);
    let controlled = branch.next_row_controlled(&control).await.unwrap().unwrap();
    assert_eq!(control.consumed(QueryCharge::SourceWork), units);

    let mut backend2 = SqliteBackend::new(&conn);
    let mut branch2 = backend2.open_branch("SELECT -0.0", &[]).await.unwrap();
    let uncontrolled = branch2.next_row().await.unwrap().unwrap();
    assert_eq!(controlled.values, uncontrolled.values);
}

#[tokio::test]
async fn uncontrolled_dispatch_ignores_the_budget_entirely() {
    // next_row() never sees a control, so no budget -- not even zero -- could
    // stop it; this is the documented, unchanged uncontrolled behavior.
    let conn = Connection::open_in_memory().unwrap();
    let mut backend = SqliteBackend::new(&conn);
    let mut branch = backend.open_branch("SELECT -0.0", &[]).await.unwrap();
    assert!(branch.next_row().await.unwrap().is_some());
}
