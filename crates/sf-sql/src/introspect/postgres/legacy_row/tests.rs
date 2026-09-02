use std::error::Error as _;

use super::*;

#[test]
fn production_text_limit_matches_adr_0051() {
    assert_eq!(MAX_LEGACY_TEXT_BYTES_PG16_V1, 256);
    assert_eq!(LEGACY_TEXT_QUERY_LIMIT_PG16_V1, 256);
}

#[test]
fn defensive_text_check_counts_utf8_bytes_at_the_exact_boundary() {
    for value in ["a".repeat(256), "é".repeat(128)] {
        assert_eq!(
            validate_text(Some(value.clone()), "test text").expect("256 UTF-8 bytes are admitted"),
            value
        );
    }

    for value in ["a".repeat(257), format!("{}x", "é".repeat(128))] {
        validate_text(Some(value), "test text").expect_err("257 UTF-8 bytes must fail closed");
    }
}

#[test]
fn rejected_or_missing_text_has_a_fixed_source_free_error() {
    let marker = "private_marker_".repeat(24);
    let errors = [
        validate_text_guard(true).expect_err("server rejection must be fatal"),
        validate_text(None, "test text").expect_err("missing projection must be fatal"),
        validate_text(Some(marker.clone()), "test text")
            .expect_err("defensive client limit must be fatal"),
    ];

    for error in errors {
        let display = error.to_string();
        let debug = format!("{error:?}");
        assert!(display.is_ascii());
        assert!(display.len() <= 256);
        assert!(debug.len() <= 256);
        assert!(!display.contains("private_marker"));
        assert!(!debug.contains("private_marker"));
        assert!(error.source().is_none());
    }
}
