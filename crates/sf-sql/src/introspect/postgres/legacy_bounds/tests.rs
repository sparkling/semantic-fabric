use super::*;

fn validate(names: &[&str], limits: LegacyInputLimitsV1) -> crate::Result<()> {
    validate_legacy_table_names(names.iter().copied(), limits)
}

#[test]
fn production_input_limits_match_adr_0051() {
    assert_eq!(PRODUCTION_LEGACY_INPUT_LIMITS_V1.max_relations, 4_096);
    assert_eq!(PRODUCTION_LEGACY_INPUT_LIMITS_V1.max_name_bytes, 256);
    assert_eq!(
        PRODUCTION_LEGACY_INPUT_LIMITS_V1.max_total_name_bytes,
        1_048_576
    );
}

#[test]
fn relation_count_bound_is_inclusive_and_counts_submitted_names() {
    assert!(validate_legacy_table_names(
        std::iter::repeat_n("same", 4_096),
        PRODUCTION_LEGACY_INPUT_LIMITS_V1,
    )
    .is_ok());
    let error = validate_legacy_table_names(
        std::iter::repeat_n("same", 4_097),
        PRODUCTION_LEGACY_INPUT_LIMITS_V1,
    )
    .expect_err("cap plus one must reject before deduplication");
    assert!(error.to_string().contains("table count"));
}

#[test]
fn individual_name_bound_counts_utf8_bytes() {
    let ascii_at_cap = "a".repeat(256);
    let utf8_at_cap = "é".repeat(128);
    let over_cap = format!("{utf8_at_cap}x");
    assert!(validate(
        &[ascii_at_cap.as_str(), utf8_at_cap.as_str()],
        PRODUCTION_LEGACY_INPUT_LIMITS_V1,
    )
    .is_ok());
    let error = validate(&[over_cap.as_str()], PRODUCTION_LEGACY_INPUT_LIMITS_V1)
        .expect_err("257 UTF-8 bytes must reject");
    assert!(error.to_string().contains("table name"));
}

#[test]
fn cumulative_bound_is_inclusive_and_non_vacuous() {
    let limits = LegacyInputLimitsV1 {
        max_relations: 3,
        max_name_bytes: 3,
        max_total_name_bytes: 5,
    };
    assert!(validate(&["aaa", "bb"], limits).is_ok());
    let error = validate(&["aaa", "bbb"], limits).expect_err("cumulative cap plus one rejects");
    assert!(error.to_string().contains("table-name bytes"));
}

#[test]
fn cumulative_arithmetic_overflow_is_rejected() {
    let error = checked_total_name_bytes(usize::MAX, 1, usize::MAX)
        .expect_err("usize addition must be checked");
    assert!(error.to_string().contains("table-name bytes"));
}

#[test]
fn empty_input_is_valid_and_errors_never_echo_input() {
    assert!(validate(&[], PRODUCTION_LEGACY_INPUT_LIMITS_V1).is_ok());
    let marker = "private_marker_".repeat(20);
    let error = validate(&[marker.as_str()], PRODUCTION_LEGACY_INPUT_LIMITS_V1)
        .expect_err("overlong marker must reject");
    assert!(!error.to_string().contains("private_marker"));
    assert!(!format!("{error:?}").contains("private_marker"));
}
