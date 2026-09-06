use super::*;

#[test]
fn public_snapshot_timeouts_are_exact_and_transaction_local() {
    assert_eq!(
        SNAPSHOT_TIMEOUTS_SQL,
        "SET LOCAL statement_timeout = '5s'; SET LOCAL lock_timeout = '1s'; SELECT set_config('search_path','pg_catalog,public,pg_temp',true);"
    );
    assert!(!SNAPSHOT_TIMEOUTS_SQL.contains("session_replication_role"));
}

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
    assert!(TABLES_SQL.contains("table_schema::pg_catalog.text = $1"));
    assert!(EARLIER_RELATION_COLLISIONS_SQL.contains("c.relname::pg_catalog.text = ANY($1)"));
    assert!(EARLIER_RELATION_COLLISIONS_SQL.contains("n.nspname::pg_catalog.text = $2"));
    assert!(KEYS_SQL.contains("tc.table_name = kcu.table_name"));
    assert!(KEYS_SQL.contains("tc.table_catalog = kcu.table_catalog"));
}

#[test]
fn legacy_queries_have_server_side_cap_plus_one_limits() {
    assert!(TABLES_SQL.contains("LIMIT $3"));
    assert!(EARLIER_RELATION_COLLISIONS_SQL.contains("LIMIT $4"));
    for sql in [
        COLUMNS_SQL,
        KEYS_SQL,
        FOREIGN_KEYS_SQL,
        RELTUPLES_SQL,
        NDISTINCT_SQL,
    ] {
        assert!(sql.contains("LIMIT $4"));
    }
}

#[test]
fn every_legacy_text_projection_has_a_server_side_byte_envelope() {
    for (sql, sources, text_limit, row_limit) in [
        (TABLES_SQL, &["table_name"][..], "$2", "LIMIT $3"),
        (
            EARLIER_RELATION_COLLISIONS_SQL,
            &["c.relname"][..],
            "$3",
            "LIMIT $4",
        ),
        (
            COLUMNS_SQL,
            &["table_name", "column_name", "data_type", "is_nullable"][..],
            "$3",
            "LIMIT $4",
        ),
        (
            KEYS_SQL,
            &[
                "tc.table_name",
                "tc.constraint_type",
                "tc.constraint_name",
                "kcu.column_name",
            ][..],
            "$3",
            "LIMIT $4",
        ),
        (
            FOREIGN_KEYS_SQL,
            &[
                "child.relname",
                "con.conname",
                "ca.attname",
                "parent.relname",
                "parent_ns.nspname",
                "pa.attname",
            ][..],
            "$3",
            "LIMIT $4",
        ),
        (RELTUPLES_SQL, &["c.relname"][..], "$3", "LIMIT $4"),
        (
            NDISTINCT_SQL,
            &["tablename", "attname"][..],
            "$3",
            "LIMIT $4",
        ),
    ] {
        let text_fields = sources.len();
        assert_eq!(
            sql.matches("CASE WHEN pg_catalog.octet_length(").count(),
            text_fields
        );
        assert_eq!(sql.matches("AS bounded_text_").count(), text_fields);
        assert_eq!(sql.matches("AS sf_text_rejected").count(), 1);
        assert!(
            sql.find("AS sf_text_rejected") < sql.find("AS bounded_text_0"),
            "text rejection must be decoded before any projected text"
        );
        assert_eq!(
            sql.matches("pg_catalog.convert_to(").count(),
            text_fields * 2
        );
        assert_eq!(sql.matches("'UTF8'").count(), text_fields * 2);
        assert_eq!(sql.matches(text_limit).count(), text_fields * 2);
        assert!(sql.contains(row_limit));
        assert!(!sql.contains("substring("));
        assert!(!sql.contains("left("));

        let mut previous = sql
            .find("AS sf_text_rejected")
            .expect("every legacy row starts with one rejection guard");
        for (index, source) in sources.iter().enumerate() {
            let projection = format!(
                "THEN {source}::pg_catalog.text ELSE NULL::pg_catalog.text END AS bounded_text_{index}"
            );
            let position = sql
                .find(&projection)
                .unwrap_or_else(|| panic!("missing exact bounded projection: {projection}"));
            assert!(position > previous, "bounded aliases must remain ordered");
            previous = position;
        }
    }

    assert!(RELTUPLES_SQL.contains("AS row_estimate"));
    assert!(NDISTINCT_SQL.contains("AS distinct_estimate"));
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
