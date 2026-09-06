use super::*;
use mysql_async::Value;

fn lexical(value: Value, code: Option<XsdTypeCode>) -> Option<String> {
    mysql_value_to_string(value, code).expect("value marshals")
}

#[test]
fn null_maps_to_none() {
    assert_eq!(lexical(Value::NULL, None), None);
}

#[test]
fn utf8_bytes_pass_through() {
    assert_eq!(
        lexical(Value::Bytes(b"hello".to_vec()), Some(XsdTypeCode::String)),
        Some("hello".to_owned())
    );
}

#[test]
fn binary_bytes_are_hex_encoded_and_text_bytes_remain_strict() {
    assert_eq!(
        lexical(Value::Bytes(vec![0xff, 0xfe]), Some(XsdTypeCode::HexBinary)),
        Some("FFFE".to_owned())
    );
    let error = mysql_value_to_string(Value::Bytes(vec![0xff, 0xfe]), Some(XsdTypeCode::String))
        .unwrap_err();
    assert!(matches!(error, Error::Marshal(_)), "{error:?}");
}

#[test]
fn integer_and_float_variants_render_via_to_string() {
    assert_eq!(
        lexical(Value::Int(-42), Some(XsdTypeCode::Integer)),
        Some("-42".to_owned())
    );
    assert_eq!(
        lexical(Value::UInt(42), Some(XsdTypeCode::Integer)),
        Some("42".to_owned())
    );
    assert_eq!(
        lexical(Value::Float(1.5), Some(XsdTypeCode::Double)),
        Some("1.5".to_owned())
    );
    assert_eq!(
        lexical(Value::Double(2.5), Some(XsdTypeCode::Double)),
        Some("2.5".to_owned())
    );
}

#[test]
fn date_with_zero_time_renders_as_bare_date() {
    assert_eq!(
        lexical(
            Value::Date(2024, 3, 15, 0, 0, 0, 0),
            Some(XsdTypeCode::Date)
        ),
        Some("2024-03-15".to_owned())
    );
}

#[test]
fn metadata_distinguishes_date_from_midnight_datetime() {
    let value = Value::Date(2024, 3, 15, 0, 0, 0, 0);
    let date_only = lexical(value.clone(), Some(XsdTypeCode::Date));
    let midnight_datetime = lexical(value, Some(XsdTypeCode::DateTime));
    assert_eq!(date_only, Some("2024-03-15".to_owned()));
    assert_eq!(midnight_datetime, Some("2024-03-15 00:00:00".to_owned()));
}

#[test]
fn date_with_time_no_micros_renders_iso_t_separated() {
    assert_eq!(
        lexical(
            Value::Date(2024, 3, 15, 13, 45, 30, 0),
            Some(XsdTypeCode::DateTime)
        ),
        Some("2024-03-15 13:45:30".to_owned())
    );
}

#[test]
fn date_with_microseconds_renders_fractional_seconds() {
    assert_eq!(
        lexical(
            Value::Date(2024, 3, 15, 13, 45, 30, 123456),
            Some(XsdTypeCode::DateTime)
        ),
        Some("2024-03-15 13:45:30.123456".to_owned())
    );
}

#[test]
fn time_zero_days_no_micros() {
    assert_eq!(
        lexical(
            Value::Time(false, 0, 13, 45, 30, 0),
            Some(XsdTypeCode::Time)
        ),
        Some("13:45:30".to_owned())
    );
}

#[test]
fn time_negative_renders_leading_minus() {
    assert_eq!(
        lexical(Value::Time(true, 0, 13, 45, 30, 0), Some(XsdTypeCode::Time)),
        Some("-13:45:30".to_owned())
    );
}

#[test]
fn time_days_component_folds_into_total_hours() {
    // MySQL TIME can exceed 24h (elapsed-time semantics); `days` folds into
    // the hour count rather than being dropped or rendered separately.
    assert_eq!(
        lexical(Value::Time(false, 2, 3, 0, 0, 0), Some(XsdTypeCode::Time)),
        Some("51:00:00".to_owned()) // 2*24 + 3 = 51
    );
}

#[test]
fn time_with_microseconds_renders_fractional_seconds() {
    assert_eq!(
        lexical(
            Value::Time(false, 0, 13, 45, 30, 500000),
            Some(XsdTypeCode::Time)
        ),
        Some("13:45:30.500000".to_owned())
    );
}

#[test]
fn prepared_column_metadata_maps_mysql_natural_types() {
    assert_eq!(
        mysql_xsd_code(
            &Column::new(ColumnType::MYSQL_TYPE_LONG),
            MysqlTypeProfile::Native
        )
        .unwrap(),
        Some(XsdTypeCode::Integer)
    );
    assert_eq!(
        mysql_xsd_code(
            &Column::new(ColumnType::MYSQL_TYPE_TINY).with_column_length(1),
            MysqlTypeProfile::Native
        )
        .unwrap(),
        Some(XsdTypeCode::Integer)
    );
    assert_eq!(
        mysql_xsd_code(
            &Column::new(ColumnType::MYSQL_TYPE_VAR_STRING).with_character_set(63),
            MysqlTypeProfile::Native
        )
        .unwrap(),
        Some(XsdTypeCode::HexBinary)
    );
    assert_eq!(
        mysql_xsd_code(
            &Column::new(ColumnType::MYSQL_TYPE_VAR_STRING),
            MysqlTypeProfile::Native
        )
        .unwrap(),
        Some(XsdTypeCode::String)
    );
    assert_eq!(
        mysql_xsd_code(
            &Column::new(ColumnType::MYSQL_TYPE_DATETIME),
            MysqlTypeProfile::Native
        )
        .unwrap(),
        Some(XsdTypeCode::DateTime)
    );
}

#[test]
fn tinyint_one_compatibility_is_explicit_and_native_value_two_is_integer() {
    let column = Column::new(ColumnType::MYSQL_TYPE_TINY).with_column_length(1);
    let native = mysql_xsd_code(&column, MysqlTypeProfile::Native).unwrap();
    let w3c = mysql_xsd_code(&column, MysqlTypeProfile::W3cSql2008).unwrap();
    assert_eq!(native, Some(XsdTypeCode::Integer));
    assert_eq!(w3c, Some(XsdTypeCode::Boolean));
    assert_eq!(lexical(Value::Int(2), native), Some("2".to_owned()));
    assert_eq!(
        mysql_natural_xsd("tinyint(1)", MysqlTypeProfile::Native),
        native
    );
    assert_eq!(
        mysql_natural_xsd("tinyint(1)", MysqlTypeProfile::W3cSql2008),
        w3c
    );
}

#[test]
fn mysql_catalog_and_wire_type_laws_are_coherent() {
    for (sql_type, column, expected) in [
        (
            "mediumint unsigned",
            Column::new(ColumnType::MYSQL_TYPE_INT24),
            XsdTypeCode::Integer,
        ),
        (
            "datetime(6)",
            Column::new(ColumnType::MYSQL_TYPE_DATETIME2),
            XsdTypeCode::DateTime,
        ),
        (
            "longblob",
            Column::new(ColumnType::MYSQL_TYPE_LONG_BLOB).with_character_set(63),
            XsdTypeCode::HexBinary,
        ),
        (
            "longtext",
            Column::new(ColumnType::MYSQL_TYPE_LONG_BLOB),
            XsdTypeCode::String,
        ),
        (
            "bit(8)",
            Column::new(ColumnType::MYSQL_TYPE_BIT),
            XsdTypeCode::HexBinary,
        ),
    ] {
        assert_eq!(
            mysql_natural_xsd(sql_type, MysqlTypeProfile::Native),
            Some(expected),
            "{sql_type}"
        );
        assert_eq!(
            mysql_xsd_code(&column, MysqlTypeProfile::Native).unwrap(),
            Some(expected),
            "{sql_type}"
        );
    }
}

#[test]
fn metadata_arity_and_unsupported_types_fail_as_typed_errors() {
    assert!(matches!(ensure_row_arity(2, 1), Err(Error::Marshal(_))));
    let error = mysql_xsd_code(
        &Column::new(ColumnType::MYSQL_TYPE_GEOMETRY),
        MysqlTypeProfile::Native,
    )
    .unwrap_err();
    assert!(matches!(error, Error::Unsupported(_)), "{error:?}");
}
