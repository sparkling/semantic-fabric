use super::*;
use crate::source::POSTGRES_RELATION_SCOPE_SETTING;

fn valid_context() -> PgSessionContext {
    PgSessionContext {
        database_oid: 1,
        database_name: "semantic_fabric_test".to_owned(),
        current_role_oid: 2,
        current_role_name: "semantic_fabric_reader".to_owned(),
        session_role_oid: 2,
        session_role_name: "semantic_fabric_reader".to_owned(),
        server_version_num: 160_009,
        search_path: POSTGRES_RELATION_SCOPE_SETTING.to_owned(),
        row_security: "on".to_owned(),
        session_replication_role: "origin".to_owned(),
    }
}

#[test]
fn session_context_rejects_role_or_policy_drift() {
    let valid = valid_context();
    assert!(validate_session_context(&valid, false, false).is_ok());

    let mut changed = valid.clone();
    changed.session_role_oid += 1;
    assert!(validate_session_context(&changed, false, false).is_err());
    assert!(validate_session_context(&valid, true, false).is_err());
    assert!(validate_session_context(&valid, false, true).is_err());
}

#[test]
fn transaction_timeouts_are_finite_and_numeric() {
    let budget = RequestBudget::after(
        Duration::from_secs(2),
        sf_core::query_control::QueryLimits::new(1, 100, 1, 1),
    );
    let sql = transaction_setup_sql(&budget).unwrap();
    assert!(sql.starts_with(BEGIN_GENERATION_SQL));
    assert!(sql.contains("SET LOCAL statement_timeout = "));
    assert!(sql.contains("SET LOCAL lock_timeout = 1000;"));
    assert!(!sql.contains('\''));
}

#[test]
fn live_postgres_profile_rejects_the_untyped_rowid_sentinel() {
    for name in ["rowid", "ROWID", "RowId"] {
        let mut table = TableSchema::new("items");
        table.columns = vec![sf_core::Column::new(name, "integer", true)];
        table.primary_key = vec![name.to_owned()];
        assert!(!postgres_direct_table_profile_is_unambiguous(&[table]));
    }

    let mut ordinary = TableSchema::new("items");
    ordinary.columns = vec![sf_core::Column::new("id", "integer", true)];
    ordinary.primary_key = vec!["id".to_owned()];
    assert!(postgres_direct_table_profile_is_unambiguous(&[ordinary]));
}
