use sf_core::ir::{Segment, TermMap};
use sf_core::{Column, ForeignKey, TableSchema};

use super::{
    direct_mapping_with_row_identity, validate_direct_mapping_base, DirectMappingRowIdentity,
    MAX_DIRECT_MAPPING_BASE_IRI_BYTES_V1, MAX_DIRECT_MAPPING_WORK_UNITS_V1,
};

const BASE: &str = "http://example.com/base/";

fn single_pk_table() -> TableSchema {
    let mut table = TableSchema::new("employees");
    table.columns = vec![
        Column::new("id", "integer", true),
        Column::new("name", "text", false),
    ];
    table.primary_key = vec!["id".to_owned()];
    table
}

#[test]
fn base_iri_is_validated_even_for_an_empty_schema() {
    let error = direct_mapping_with_row_identity(
        &[],
        "not an absolute IRI",
        DirectMappingRowIdentity::SqliteRowId,
    )
    .unwrap_err();
    assert!(error.to_string().contains("base IRI"), "{error}");
}

#[test]
fn base_iri_utf8_byte_cap_accepts_the_boundary_and_rejects_the_next_byte() {
    let prefix = "http://example.com/";
    let at_limit = format!(
        "{prefix}{}",
        "a".repeat(MAX_DIRECT_MAPPING_BASE_IRI_BYTES_V1 - prefix.len())
    );
    assert_eq!(at_limit.len(), MAX_DIRECT_MAPPING_BASE_IRI_BYTES_V1);
    validate_direct_mapping_base(&at_limit).unwrap();

    let over_limit = format!("{at_limit}x");
    let error = validate_direct_mapping_base(&over_limit).unwrap_err();
    assert!(error.to_string().contains("maximum"), "{error}");
}

#[test]
fn oversized_base_error_does_not_echo_the_rejected_iri() {
    let base = format!(
        "http://example.com/{}",
        "SECRET_SENTINEL".repeat(MAX_DIRECT_MAPPING_BASE_IRI_BYTES_V1)
    );
    let error = validate_direct_mapping_base(&base).unwrap_err().to_string();
    assert!(!error.contains("SECRET_SENTINEL"), "{error}");
}

#[test]
fn prospective_generated_byte_cap_rejects_before_ir_construction() {
    let prefix = "http://example.com/";
    let base = format!(
        "{prefix}{}",
        "b".repeat(MAX_DIRECT_MAPPING_BASE_IRI_BYTES_V1 - prefix.len())
    );
    let mut table = TableSchema::new("wide");
    table.columns = (0..9_000)
        .map(|index| Column::new(format!("c{index}"), "text", false))
        .collect();
    let error =
        direct_mapping_with_row_identity(&[table], &base, DirectMappingRowIdentity::SqliteRowId)
            .unwrap_err();
    assert!(
        error.to_string().contains("generated UTF-8 bytes"),
        "{error}"
    );
}

#[test]
fn prospective_work_cap_rejects_before_structural_validation() {
    let mut table = TableSchema::new("work");
    table.unique = (0..=MAX_DIRECT_MAPPING_WORK_UNITS_V1)
        .map(|_| Vec::new())
        .collect();
    let error =
        direct_mapping_with_row_identity(&[table], BASE, DirectMappingRowIdentity::SqliteRowId)
            .unwrap_err();
    assert!(error.to_string().contains("work units"), "{error}");
}

#[test]
fn require_primary_key_rejects_a_no_key_table() {
    let mut table = TableSchema::new("events");
    table.columns = vec![Column::new("payload", "text", false)];
    let error = direct_mapping_with_row_identity(
        &[table],
        BASE,
        DirectMappingRowIdentity::RequirePrimaryKey,
    )
    .unwrap_err();
    assert!(error.to_string().contains("no primary key"), "{error}");
}

#[test]
fn explicit_postgres_conformance_identity_supports_a_no_key_table() {
    let mut table = TableSchema::new("events");
    table.columns = vec![Column::new("payload", "text", false)];
    let maps = direct_mapping_with_row_identity(
        &[table],
        BASE,
        DirectMappingRowIdentity::PostgresCtidConformance,
    )
    .unwrap();
    let TermMap::Template(template, _) = &maps[0].subject.term else {
        panic!("no-key subject must be a blank-node template")
    };
    assert!(template
        .segments()
        .contains(&Segment::Column("rowid".into())));
}

#[test]
fn sqlite_no_key_table_rejects_a_shadowed_rowid_alias_case_insensitively() {
    for name in ["rowid", "ROWID", "RowId"] {
        let mut table = TableSchema::new("events");
        table.columns = vec![Column::new(name, "text", false)];
        let error =
            direct_mapping_with_row_identity(&[table], BASE, DirectMappingRowIdentity::SqliteRowId)
                .unwrap_err();
        assert!(error.to_string().contains("shadows"), "{name}: {error}");
    }
}

#[test]
fn sqlite_other_shadowed_aliases_leave_the_selected_rowid_available() {
    let mut table = TableSchema::new("events");
    table.columns = vec![
        Column::new("_rowid_", "integer", false),
        Column::new("oid", "integer", false),
    ];
    let maps =
        direct_mapping_with_row_identity(&[table], BASE, DirectMappingRowIdentity::SqliteRowId)
            .unwrap();
    let TermMap::Template(template, _) = &maps[0].subject.term else {
        panic!("no-key subject must be a blank-node template")
    };
    assert!(template
        .segments()
        .contains(&Segment::Column("rowid".into())));
}

#[test]
fn sqlite_rowid_named_column_is_safe_when_a_primary_key_avoids_the_alias() {
    let mut table = TableSchema::new("events");
    table.columns = vec![
        Column::new("id", "integer", true),
        Column::new("rowid", "integer", false),
    ];
    table.primary_key = vec!["id".to_owned()];
    direct_mapping_with_row_identity(&[table], BASE, DirectMappingRowIdentity::SqliteRowId)
        .unwrap();
}

#[test]
fn duplicate_tables_and_columns_fail_closed() {
    let mut duplicate_columns = single_pk_table();
    duplicate_columns
        .columns
        .push(Column::new("id", "integer", true));
    assert!(direct_mapping_with_row_identity(
        &[duplicate_columns],
        BASE,
        DirectMappingRowIdentity::SqliteRowId,
    )
    .is_err());

    let table = single_pk_table();
    assert!(direct_mapping_with_row_identity(
        &[table.clone(), table],
        BASE,
        DirectMappingRowIdentity::SqliteRowId,
    )
    .is_err());
}

#[test]
fn malformed_and_dangling_foreign_keys_fail_closed() {
    let mut child = single_pk_table();
    child.foreign_keys.push(ForeignKey {
        columns: vec!["id".to_owned()],
        parent_table: "missing".to_owned(),
        parent_columns: vec!["id".to_owned()],
    });
    assert!(direct_mapping_with_row_identity(
        &[child],
        BASE,
        DirectMappingRowIdentity::SqliteRowId,
    )
    .is_err());

    let mut malformed = single_pk_table();
    malformed.foreign_keys.push(ForeignKey {
        columns: vec!["id".to_owned()],
        parent_table: "employees".to_owned(),
        parent_columns: Vec::new(),
    });
    assert!(direct_mapping_with_row_identity(
        &[malformed],
        BASE,
        DirectMappingRowIdentity::SqliteRowId,
    )
    .is_err());
}

#[test]
fn a_foreign_key_must_reference_a_declared_parent_key() {
    let mut parent = TableSchema::new("parent");
    parent.columns = vec![Column::new("value", "integer", true)];
    let mut child = single_pk_table();
    child.foreign_keys.push(ForeignKey {
        columns: vec!["id".to_owned()],
        parent_table: "parent".to_owned(),
        parent_columns: vec!["value".to_owned()],
    });
    assert!(direct_mapping_with_row_identity(
        &[child, parent],
        BASE,
        DirectMappingRowIdentity::SqliteRowId,
    )
    .is_err());
}
