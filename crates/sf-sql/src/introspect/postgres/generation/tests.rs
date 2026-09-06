use super::*;
use crate::introspect::PostgresSchemaIdentityUnavailableV1;

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn transaction_probe_and_mode_query_are_closed_and_exact() {
    assert_eq!(
        EXPLICIT_TRANSACTION_PROBE_SQL,
        "SAVEPOINT sf_generation_transaction_probe; RELEASE SAVEPOINT sf_generation_transaction_probe;"
    );
    assert_eq!(
        TRANSACTION_MODE_SQL
            .matches("pg_catalog.current_setting")
            .count(),
        2
    );
    assert!(TRANSACTION_MODE_SQL.contains("'repeatable read'"));
    assert!(TRANSACTION_MODE_SQL.contains("'on'"));
    assert_eq!(OBSERVATION_SAVEPOINT_SQL, "SAVEPOINT sf_observed_schema_v1");
    assert_eq!(
        OBSERVATION_RELEASE_SQL,
        "RELEASE SAVEPOINT sf_observed_schema_v1"
    );
    assert_eq!(
        OBSERVATION_RECOVER_SQL,
        "ROLLBACK TO SAVEPOINT sf_observed_schema_v1; RELEASE SAVEPOINT sf_observed_schema_v1"
    );
}

#[test]
fn only_clean_pre_legacy_guard_mismatches_may_downgrade() {
    use crate::introspect::PostgresSchemaIdentityGuardCodeV1 as Guard;
    use crate::introspect::PostgresSchemaIdentityLimitCodeV1 as Limit;
    use PostgresSchemaIdentityUnavailableV1 as Unavailable;

    for reason in [
        Unavailable::ProfileNotImplemented,
        Unavailable::UnqualifiedEnginePatch,
        Unavailable::IdentityRejected,
        Unavailable::GuardUnsupported(Guard::ServerEncoding),
        Unavailable::GuardUnsupported(Guard::IndexKeyLimit),
        Unavailable::GuardUnsupported(Guard::IntegerDatetimes),
        Unavailable::GuardUnsupported(Guard::ReplicationRole),
        Unavailable::GuardUnsupported(Guard::PublicNamespace),
        Unavailable::GuardUnsupported(Guard::CurrentDatabase),
    ] {
        assert!(guard_failure_may_downgrade(reason), "{reason:?}");
    }
    for reason in [
        Unavailable::GuardUnsupported(Guard::ClientEncoding),
        Unavailable::GuardUnsupported(Guard::IdentifierLength),
        Unavailable::GuardUnsupported(Guard::SearchPath),
        Unavailable::LegacyCoordinateMismatch,
        Unavailable::CatalogQuery,
        Unavailable::CatalogDecode,
        Unavailable::LimitExceeded(Limit::RichRelations),
        Unavailable::UnsupportedRelation,
        Unavailable::UnsupportedType,
        Unavailable::UnsupportedCollation,
        Unavailable::UnsupportedConstraint,
    ] {
        assert!(!guard_failure_may_downgrade(reason), "{reason:?}");
    }
}

#[test]
fn lock_sql_is_deterministic_exact_only_and_rigorously_quoted() {
    let sql = build_public_base_table_lock_sql(&names(&["z", "a\"b", "dotted.name"]))
        .unwrap()
        .unwrap();
    assert_eq!(
        sql,
        "LOCK TABLE ONLY \"public\".\"a\"\"b\", ONLY \"public\".\"dotted.name\", ONLY \"public\".\"z\" IN ACCESS SHARE MODE NOWAIT"
    );
    assert_eq!(sql.matches("ONLY \"public\".").count(), 3);
    assert!(sql.ends_with(" IN ACCESS SHARE MODE NOWAIT"));
}

#[test]
fn empty_relation_set_emits_no_lock_statement() {
    assert_eq!(build_public_base_table_lock_sql(&[]).unwrap(), None);
}

#[test]
fn exact_case_distinct_names_are_not_collapsed() {
    let sql = build_public_base_table_lock_sql(&names(&["alpha", "Alpha"]))
        .unwrap()
        .unwrap();
    assert!(sql.contains("\"Alpha\""));
    assert!(sql.contains("\"alpha\""));
}

#[test]
fn malformed_or_duplicate_relation_names_fail_closed() {
    for values in [
        names(&[""]),
        names(&["same", "same"]),
        names(&["nul\0name"]),
        vec!["x".repeat(MAX_POSTGRES_IDENTIFIER_BYTES_V1 + 1)],
    ] {
        let error = build_public_base_table_lock_sql(&values)
            .expect_err("invalid lock input must fail before SQL execution");
        let display = error.to_string();
        assert!(display.starts_with("introspection error:"));
        for value in &values {
            if !value.is_empty() {
                assert!(!display.contains(value));
                assert!(!format!("{error:?}").contains(value));
            }
        }
    }
}

#[test]
fn relation_count_cap_plus_one_fails_before_lock_sql_allocation() {
    let values = std::iter::repeat_n("same", MAX_LEGACY_RELATIONS_PG16_V1 + 1)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let error = build_public_base_table_lock_sql(&values)
        .expect_err("cap plus one must fail before sorting or SQL construction");
    assert!(error.to_string().contains("table count"));
}

#[test]
fn postgres_identifier_byte_bound_is_inclusive() {
    let at_cap = "é".repeat(31) + "x";
    assert_eq!(at_cap.len(), MAX_POSTGRES_IDENTIFIER_BYTES_V1);
    assert!(build_public_base_table_lock_sql(&[at_cap]).is_ok());

    let over_cap = "é".repeat(32);
    assert_eq!(over_cap.len(), MAX_POSTGRES_IDENTIFIER_BYTES_V1 + 1);
    let error = build_public_base_table_lock_sql(std::slice::from_ref(&over_cap))
        .expect_err("64-byte identifier must reject");
    assert!(!error.to_string().contains(&over_cap));
}

#[test]
fn lock_sqlstate_preserves_schema_drift_and_privilege_classification() {
    for code in [SqlState::UNDEFINED_TABLE, SqlState::UNDEFINED_SCHEMA] {
        assert_eq!(
            classify_lock_sqlstate(Some(&code)),
            PostgresPublicTableLockFailure::RelationSetChanged
        );
    }
    assert_eq!(
        classify_lock_sqlstate(Some(&SqlState::INSUFFICIENT_PRIVILEGE)),
        PostgresPublicTableLockFailure::InsufficientPrivilege
    );
    assert_eq!(
        classify_lock_sqlstate(Some(&SqlState::LOCK_NOT_AVAILABLE)),
        PostgresPublicTableLockFailure::Unavailable
    );
    assert_eq!(
        classify_lock_sqlstate(None),
        PostgresPublicTableLockFailure::Unavailable
    );
}

#[test]
fn observation_failures_do_not_turn_transport_errors_into_profile_drift() {
    assert_eq!(
        classify_observation_failure(PostgresSchemaIdentityUnavailableV1::CatalogQuery),
        PostgresGenerationObservationFailure::SourceUnavailable
    );
    assert_eq!(
        classify_observation_failure(PostgresSchemaIdentityUnavailableV1::UnsupportedType),
        PostgresGenerationObservationFailure::ProfileUnavailable(
            PostgresSchemaIdentityUnavailableV1::UnsupportedType
        )
    );
}
