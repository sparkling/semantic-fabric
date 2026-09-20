use super::*;
use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};

#[test]
fn metadata_field_caps_precede_copy_and_source_work_refusal_preserves_accounting() {
    let value = Value::Bytes(b"abc".to_vec());
    for limit in [5, 6] {
        let budget = QueryBudget::new(QueryLimits::new(u64::MAX, limit, u64::MAX, u64::MAX));
        let mut bytes = 0;
        let result = field(Some(&value), 3, &mut bytes, SourceWork::new(Some(&budget)));
        if limit == 6 {
            assert_eq!(result.unwrap(), Some("abc".into()));
            assert_eq!(bytes, 3);
            assert_eq!(budget.consumed(QueryCharge::SourceWork), 6);
        } else {
            assert!(matches!(
                result,
                Err(PgGenerationError::Control(
                    QueryControlError::SourceWorkExceeded
                ))
            ));
            assert_eq!(bytes, 0);
        }
    }
    let budget = QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
    let work = SourceWork::new(Some(&budget));
    assert!(field(Some(&value), 2, &mut 0, work).is_err());
    assert_eq!(budget.consumed(QueryCharge::SourceWork), 0);
    let mut bytes = MAX_SCHEMA_BYTES - 3;
    assert!(field(Some(&value), 3, &mut bytes, work).is_ok());
    assert_eq!(bytes, MAX_SCHEMA_BYTES);
    assert!(field(Some(&Value::from("x")), 1, &mut bytes, work).is_err());
    assert_eq!(bytes, MAX_SCHEMA_BYTES);
}

#[test]
fn metadata_decode_rejects_missing_wrong_type_invalid_utf8_and_identifier_injection() {
    let work = SourceWork::new(None);
    for value in [None, Some(Value::Int(1)), Some(Value::Bytes(vec![255]))] {
        assert!(field(value.as_ref(), 64, &mut 0, work).is_err());
    }
    assert_eq!(field(Some(&Value::NULL), 64, &mut 0, work).unwrap(), None);
    for name in ["", "a.b", "`items`", "a;SELECT 1", "1first", "with space"] {
        assert!(!identifier(name), "{name}");
    }
    assert!(identifier("_Table9"));
    assert!(!identifier(&"x".repeat(64)));
}
