use super::*;

#[test]
fn catalogue_queries_bind_complete_relation_identity() {
    for (sql, schema_identity) in [
        (COLUMNS_SQL, "table_schema"),
        (KEYS_SQL, "table_schema"),
        (FOREIGN_KEYS_SQL, "nspname"),
        (RELTUPLES_SQL, "nspname"),
        (NDISTINCT_SQL, "schemaname"),
    ] {
        assert!(sql.contains("ANY($1)"));
        assert!(sql.contains("$2"));
        assert!(sql.contains(schema_identity));
    }
    assert!(TABLES_SQL.contains("table_schema = $1"));
    assert!(EARLIER_RELATION_COLLISIONS_SQL.contains("c.relname = ANY($1)"));
    assert!(EARLIER_RELATION_COLLISIONS_SQL.contains("n.nspname = $2"));
    assert!(KEYS_SQL.contains("tc.table_name = kcu.table_name"));
    assert!(KEYS_SQL.contains("tc.table_catalog = kcu.table_catalog"));
}

#[test]
fn legacy_queries_have_server_side_cap_plus_one_limits() {
    for sql in [TABLES_SQL, EARLIER_RELATION_COLLISIONS_SQL] {
        assert!(sql.contains("LIMIT $2") || sql.contains("LIMIT $3"));
    }
    for sql in [
        COLUMNS_SQL,
        KEYS_SQL,
        FOREIGN_KEYS_SQL,
        RELTUPLES_SQL,
        NDISTINCT_SQL,
    ] {
        assert!(sql.contains("LIMIT $3"));
    }
}

#[test]
fn cross_schema_foreign_keys_fail_closed() {
    assert!(require_same_schema("public", "public", "child", "fk").is_ok());
    let error = require_same_schema("public", "private", "child", "fk")
        .expect_err("unrepresentable qualified parent must fail closed");
    assert!(error.to_string().contains("schema-qualified"));
}

#[test]
fn earlier_relation_collision_message_is_non_ambiguous() {
    let error = Error::Introspection(
        "PostgreSQL public relation name(s) collide with the earlier pg_catalog execution scope: pg_class; qualified relation identity is not yet supported".into(),
    );
    assert!(error.to_string().contains("pg_catalog"));
    assert!(error.to_string().contains("qualified relation identity"));
}
