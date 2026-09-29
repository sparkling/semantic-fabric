//! Startup assembly unit tests.

use sf_core::{Column, TableSchema};

use super::*;
use crate::SourceRef;

#[path = "ordinary_federation_fixture.rs"]
mod ordinary_federation_fixture;
#[path = "ordinary_federation_tests.rs"]
mod ordinary_federation_tests;

const BASE: &str = "http://example.com/live/";

fn table(primary_key: bool) -> TableSchema {
    let mut table = TableSchema::new("items");
    table.columns = vec![Column::new("id", "integer", true)];
    if primary_key {
        table.primary_key = vec!["id".to_owned()];
    }
    table
}

fn direct() -> PreparedMapping {
    PreparedMapping::new(
        &MappingRef::direct(BASE),
        source_id(0),
        &crate::test_support::empty_ontology(),
    )
    .unwrap()
}

fn authored_empty(index: usize) -> PreparedMapping {
    PreparedMapping::Authored(sf_core::SourceMapping::new(source_id(index), Vec::new()))
}

#[test]
fn direct_mapping_base_is_validated_before_source_open() {
    let error = PreparedMapping::new(
        &MappingRef::direct("not an absolute IRI"),
        source_id(0),
        &crate::test_support::empty_ontology(),
    )
    .expect_err("invalid base must fail during mapping preparation");
    assert_eq!(error.code(), "startup-configuration");
}

#[test]
fn postgres_direct_mapping_rejects_no_primary_key_tables() {
    let error = direct()
        .finish_for(BackendKind::Postgres, &[table(false)])
        .expect_err("PostgreSQL no-PK identity is not admitted for live serving");
    assert_eq!(error.code(), "startup-configuration");
}

#[test]
fn postgres_direct_mapping_accepts_declared_primary_keys() {
    let mapping = direct()
        .finish_for(BackendKind::Postgres, &[table(true)])
        .expect("declared primary key does not need a physical row identity");
    assert_eq!(mapping.len(), 1);
}

#[test]
fn authored_assembler_rejects_direct_mapping_before_connector_io() {
    let sqlite = SourceRef::inline("sqlite:/path/that/must/not/be/created.db")
        .resolve()
        .unwrap()
        .prepare()
        .unwrap();
    let postgres = SourceRef::inline("pg:host=database.invalid user=test")
        .resolve()
        .unwrap()
        .prepare()
        .unwrap();
    let mysql = SourceRef::inline("mysql://test@database.invalid/db")
        .resolve()
        .unwrap()
        .prepare()
        .unwrap();

    for source in [sqlite, mysql] {
        let error = admit_mapping_profile(&direct(), &source, None, None)
            .expect_err("non-PostgreSQL Direct Mapping must reject without connecting");
        assert_eq!(error.code(), "startup-configuration");
    }
    let error = admit_mapping_profile(&direct(), &postgres, None, None)
        .expect_err("Direct requires its dedicated leased lifecycle assembler");
    assert_eq!(error.code(), "startup-configuration");

    let postgres = SourceRef::inline("pg:host=database.invalid user=test")
        .resolve()
        .unwrap()
        .prepare()
        .unwrap();
    let error = admit_mapping_profile(
        &direct(),
        &postgres,
        Some(&authored_empty(1)),
        Some(
            &SourceRef::inline("pg:host=other.invalid user=test")
                .resolve()
                .unwrap()
                .prepare()
                .unwrap(),
        ),
    )
    .expect_err("Direct Mapping federation must reject without connecting");
    assert_eq!(error.code(), "startup-configuration");

    let primary_authored = authored_empty(0);
    let additional_direct = PreparedMapping::new(
        &MappingRef::direct(BASE),
        source_id(1),
        &crate::test_support::empty_ontology(),
    )
    .unwrap();
    let error = admit_mapping_profile(
        &primary_authored,
        &postgres,
        Some(&additional_direct),
        Some(
            &SourceRef::inline("pg:host=additional.invalid user=test")
                .resolve()
                .unwrap()
                .prepare()
                .unwrap(),
        ),
    )
    .expect_err("an authored plus Direct profile must reject without connecting");
    assert_eq!(error.code(), "startup-configuration");
}
