use super::*;
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::backend::{BranchStream, SqlBackend};

fn budget(units: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX))
}

fn numeric_wire(ndigits: u16, weight: i16, sign: u16, dscale: u16, digits: &[i16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8 + digits.len() * 2);
    bytes.extend_from_slice(&ndigits.to_be_bytes());
    bytes.extend_from_slice(&weight.to_be_bytes());
    bytes.extend_from_slice(&sign.to_be_bytes());
    bytes.extend_from_slice(&dscale.to_be_bytes());
    for digit in digits {
        bytes.extend_from_slice(&digit.to_be_bytes());
    }
    bytes
}

#[test]
fn numeric_work_has_independent_exact_and_n_minus_one_boundaries() {
    for (wire, expected, units) in [
        (numeric_wire(0, 0, 0x0000, 0, &[]), "0".to_owned(), 13),
        (
            numeric_wire(3, 1, 0x0000, 3, &[1, 2345, 6780]),
            "12345.678".to_owned(),
            29,
        ),
        (
            numeric_wire(1, -1, 0x4000, 4, &[1]),
            "-0.0001".to_owned(),
            18,
        ),
    ] {
        for cap in [units - 1, units] {
            let control = budget(cap);
            let result = decode_numeric(&wire, SourceWork::new(Some(&control)));
            if cap == units {
                assert_eq!(result.unwrap(), expected);
                assert_eq!(control.consumed(QueryCharge::SourceWork), units);
            } else {
                assert!(matches!(
                    result,
                    Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                ));
                assert_eq!(
                    control.checkpoint(),
                    Err(QueryControlError::SourceWorkExceeded)
                );
            }
        }
    }
}

#[test]
fn numeric_refusal_precedes_header_error_classification() {
    let invalid = numeric_wire(0, 0, 0x1234, 0, &[]);
    assert!(matches!(
        decode_numeric(&invalid, SourceWork::new(Some(&budget(7)))),
        Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
    ));
    assert!(matches!(
        decode_numeric(&invalid, SourceWork::new(Some(&budget(u64::MAX)))),
        Err(Error::Marshal(message)) if message.contains("unrecognised sign")
    ));
}

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
fn every_numeric_charge_observes_sticky_cancel_and_deadline() {
    let wire = numeric_wire(3, 1, 0x0000, 3, &[1, 2345, 6780]);
    let measured = Stop {
        budget: budget(u64::MAX),
        at: usize::MAX,
        calls: AtomicUsize::new(0),
        reason: QueryControlError::Cancelled,
    };
    decode_numeric(&wire, SourceWork::new(Some(&measured))).unwrap();
    let charge_count = measured.calls.load(Ordering::Relaxed);
    assert!(charge_count >= 4);
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
            assert!(matches!(
                decode_numeric(&wire, SourceWork::new(Some(&control))),
                Err(Error::QueryControl(found)) if found == reason
            ));
            assert_eq!(control.checkpoint(), Err(reason));
        }
    }
}

#[test]
fn large_numeric_decode_is_checkpointed_and_byte_exact() {
    let digits = vec![1111; 32768];
    let wire = numeric_wire(32768, 32767, 0x0000, 2, &digits);
    let measured = budget(u64::MAX);
    let expected = format!("{}.00", "1111".repeat(32768));
    assert_eq!(
        decode_numeric(&wire, SourceWork::new(Some(&measured))).unwrap(),
        expected
    );
    let units = measured.consumed(QueryCharge::SourceWork);
    assert!(units > wire.len() as u64);
    assert!(matches!(
        decode_numeric(&wire, SourceWork::new(Some(&budget(units - 1)))),
        Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
    ));
}

#[test]
#[ignore = "requires SF_PG_URL: owned PostgreSQL decoder probe role"]
fn controlled_numeric_row_uses_the_supplied_source_budget() {
    let conn = std::env::var("SF_PG_URL").expect("owned decoder probe URL");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (client, connection) = tokio_postgres::connect(&conn, tokio_postgres::NoTls)
            .await
            .expect("connect to owned decoder probe role");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let open = || async {
            let mut backend = super::super::PgBackend::new(&client);
            backend
                .open_branch("SELECT src FROM items WHERE value = 'probe'", &[])
                .await
                .expect("open real PostgreSQL NUMERIC row")
        };
        let measured = budget(u64::MAX);
        let row = open()
            .await
            .next_row_controlled(&measured)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.values, vec![Some("12345.678".into())]);
        let units = measured.consumed(QueryCharge::SourceWork);
        let row_units = 4
            + std::mem::size_of::<Option<String>>()
            + std::mem::size_of::<Option<sf_core::datatype::XsdTypeCode>>();
        assert_eq!(units, row_units as u64 + 29);

        let exact = budget(units);
        assert_eq!(
            open()
                .await
                .next_row_controlled(&exact)
                .await
                .unwrap()
                .unwrap()
                .values,
            row.values
        );
        assert_eq!(exact.consumed(QueryCharge::SourceWork), units);
        let refused = budget(units - 1);
        assert!(matches!(
            open().await.next_row_controlled(&refused).await,
            Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
        ));
        assert_eq!(
            refused.checkpoint(),
            Err(QueryControlError::SourceWorkExceeded)
        );
        assert_eq!(
            open().await.next_row().await.unwrap().unwrap().values,
            row.values
        );
    });
}
