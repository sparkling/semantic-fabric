use super::*;

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
