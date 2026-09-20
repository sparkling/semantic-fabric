use super::*;
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use std::sync::atomic::{AtomicUsize, Ordering};

fn budget(units: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(u64::MAX, units, u64::MAX, u64::MAX))
}

#[test]
fn lexical_work_is_prospective_at_fixed_boundaries() {
    for (native, code, expected, units) in [
        (Value::NULL, None, None, 1),
        (
            Value::Bytes(b"hello".to_vec()),
            Some(XsdTypeCode::String),
            Some("hello"),
            6,
        ),
        (
            Value::Bytes(vec![0, 255]),
            Some(XsdTypeCode::HexBinary),
            Some("00FF"),
            7,
        ),
        (
            Value::Int(i64::MIN),
            Some(XsdTypeCode::Integer),
            Some("-9223372036854775808"),
            513,
        ),
        (
            Value::UInt(u64::MAX),
            Some(XsdTypeCode::Integer),
            Some("18446744073709551615"),
            513,
        ),
        (
            Value::Time(true, 2, 3, 4, 5, 6),
            Some(XsdTypeCode::Time),
            Some("-51:04:05.000006"),
            513,
        ),
    ] {
        for cap in [units - 1, units] {
            let meter = budget(cap);
            let actual = value(native.clone(), code, SourceWork::new(Some(&meter)));
            if cap == units {
                assert_eq!(actual.unwrap().as_deref(), expected);
                assert_eq!(meter.consumed(QueryCharge::SourceWork), units);
            } else {
                assert!(matches!(
                    actual,
                    Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                ));
                assert_eq!(
                    meter.checkpoint(),
                    Err(QueryControlError::SourceWorkExceeded)
                );
            }
            assert_eq!(meter.consumed(QueryCharge::CompilerWork), 0);
        }
    }
    // Refused admission must win before strict UTF8 validation.
    assert!(matches!(
        value(
            Value::Bytes(vec![255]),
            None,
            SourceWork::new(Some(&budget(1)))
        ),
        Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
    ));
    assert!(matches!(
        value(
            Value::Bytes(vec![255]),
            None,
            SourceWork::new(Some(&budget(2)))
        ),
        Err(Error::Marshal(_))
    ));
}

#[test]
fn row_threading_pays_storage_and_each_value_before_publication() {
    let codes = [Some(XsdTypeCode::String), Some(XsdTypeCode::HexBinary)];
    let storage = 2
        + 2 * std::mem::size_of::<Option<String>>()
        + 2
        + 4 * std::mem::size_of::<Option<XsdTypeCode>>();
    let units = storage as u64 + 2 + 6 + 7;
    for cap in [0, units - 1, units] {
        let meter = budget(cap);
        let calls = AtomicUsize::new(0);
        let result = row(
            2,
            &codes,
            |i| {
                calls.fetch_add(1, Ordering::Relaxed);
                Some(if i == 0 {
                    Value::Bytes(b"hello".to_vec())
                } else {
                    Value::Bytes(vec![0, 255])
                })
            },
            SourceWork::new(Some(&meter)),
        );
        if cap == units {
            let tuple = result.unwrap();
            assert_eq!(tuple.values, [Some("hello".into()), Some("00FF".into())]);
            assert_eq!(tuple.codes, codes);
            assert_eq!(meter.consumed(QueryCharge::SourceWork), units);
        } else {
            assert!(matches!(
                result,
                Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
            ));
            if cap == 0 {
                assert_eq!(calls.load(Ordering::Relaxed), 0);
            }
        }
    }
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
fn every_decode_charge_observes_sticky_terminal_state() {
    let run = |control: &dyn QueryControl| {
        row(
            2,
            &[None, None],
            |_| Some(Value::Bytes(b"text".to_vec())),
            SourceWork::new(Some(control)),
        )
    };
    let measured = Stop {
        budget: budget(u64::MAX),
        at: usize::MAX,
        calls: AtomicUsize::new(0),
        reason: QueryControlError::Cancelled,
    };
    run(&measured).unwrap();
    for reason in [
        QueryControlError::Cancelled,
        QueryControlError::DeadlineExceeded,
    ] {
        for at in 1..=measured.calls.load(Ordering::Relaxed) {
            let control = Stop {
                budget: budget(u64::MAX),
                at,
                calls: AtomicUsize::new(0),
                reason,
            };
            assert!(matches!(run(&control),Err(Error::QueryControl(found)) if found==reason));
            assert_eq!(control.checkpoint(), Err(reason));
        }
    }
}

#[test]
fn bounded_scalar_allowance_covers_extreme_lexical_values() {
    for native in [
        Value::Double(f64::MAX),
        Value::Double(f64::MIN_POSITIVE),
        Value::Double(f64::from_bits(1)),
        Value::Double(-f64::from_bits(1)),
        Value::Double(f64::NAN),
        Value::Double(f64::NEG_INFINITY),
        Value::Time(true, u32::MAX, 255, 255, 255, u32::MAX),
    ] {
        let text = value(native, None, SourceWork::new(Some(&budget(513))))
            .unwrap()
            .unwrap();
        assert!(text.len() <= 327);
    }
}
