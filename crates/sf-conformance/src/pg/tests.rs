use super::*;

#[test]
fn local_provider_absence_is_typed_untested() {
    let outcome = unavailable::<Report>(LiveMode::LocalOptional, "connection refused".to_owned())
        .expect("local absence is allowed");
    assert!(matches!(
        outcome,
        LiveRun::Untested(UntestedReason::ProviderUnavailable { detail })
            if detail == "connection refused"
    ));
}

#[test]
fn ci_required_provider_absence_is_an_error() {
    let error = unavailable::<Report>(LiveMode::CiRequired, "connection refused".to_owned())
        .expect_err("required provider absence must fail");
    assert!(error.contains("required PostgreSQL provider"), "{error}");
    assert!(error.contains("connection refused"), "{error}");
}

#[test]
fn invalid_connection_configuration_does_not_echo_secrets() {
    let sentinel = "DO-NOT-ECHO-THIS-SECRET";
    let error = parse_base_config(&format!("{sentinel}=invalid")).unwrap_err();
    assert!(!error.contains(sentinel), "{error}");
}

#[test]
fn connection_failures_do_not_echo_provider_errors() {
    let sentinel = "postgres://user:DO-NOT-ECHO-THIS-SECRET@example.invalid/database";
    let error = connection_error(sentinel);
    assert_eq!(error, "PostgreSQL connection failed");
    assert!(!error.contains(sentinel), "{error}");
}

#[test]
fn scratch_database_names_are_bounded_identifiers() {
    let name = scratch_database_name().expect("scratch database name");
    assert!(name.len() <= 63, "{name}");
    assert!(name.chars().all(|character| character.is_ascii_lowercase()
        || character.is_ascii_digit()
        || character == '_'));
}
