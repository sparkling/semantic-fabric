use super::super::PostgresSchemaIdentityGuardCodeV1;
use super::guard::CatalogGuardRowV1;
use super::relation_attribute::{CatalogAttributeRowV1, CatalogRelationRowV1};
use super::*;

fn valid_guard() -> CatalogGuardRowV1 {
    CatalogGuardRowV1 {
        server_version_num: 160_015,
        server_encoding: "UTF8".into(),
        client_encoding: "UTF8".into(),
        max_identifier_length: 63,
        max_index_keys: 32,
        integer_datetimes: "on".into(),
        session_replication_role: "origin".into(),
        search_path: "pg_catalog,public,pg_temp".into(),
        database_oid: 1,
        database_provider: "c".into(),
        database_collate: "C".into(),
        database_ctype: "C".into(),
        database_icu_locale: None,
        database_icu_rules: None,
        database_recorded_version: None,
        database_actual_version: None,
        public_namespace_count: 1,
        current_database_count: 1,
    }
}

#[test]
fn guard_validator_classifies_each_profile_gate() {
    let mut guard = valid_guard();
    assert!(guard.validate().is_ok());
    guard.search_path = "pg_catalog, public, pg_temp".into();
    assert!(guard.validate().is_ok());
    guard.server_version_num = 160_014;
    assert_eq!(
        guard.validate(),
        Err(PostgresSchemaIdentityUnavailableV1::UnqualifiedEnginePatch)
    );
    guard.server_version_num = 150_010;
    assert_eq!(
        guard.validate(),
        Err(PostgresSchemaIdentityUnavailableV1::ProfileNotImplemented)
    );
    guard = valid_guard();
    guard.server_encoding = "LATIN1".into();
    assert_eq!(
        guard.validate(),
        Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(
            PostgresSchemaIdentityGuardCodeV1::ServerEncoding
        ))
    );
    guard = valid_guard();
    guard.client_encoding = "LATIN1".into();
    assert_eq!(
        guard.validate(),
        Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(
            PostgresSchemaIdentityGuardCodeV1::ClientEncoding
        ))
    );
    guard = valid_guard();
    guard.max_identifier_length = 62;
    assert_eq!(
        guard.validate(),
        Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(
            PostgresSchemaIdentityGuardCodeV1::IdentifierLength
        ))
    );
    guard = valid_guard();
    guard.search_path = "public".into();
    assert_eq!(
        guard.validate(),
        Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(
            PostgresSchemaIdentityGuardCodeV1::SearchPath
        ))
    );
    guard = valid_guard();
    guard.public_namespace_count = 2;
    assert_eq!(
        guard.validate(),
        Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(
            PostgresSchemaIdentityGuardCodeV1::PublicNamespace
        ))
    );
    guard = valid_guard();
    guard.max_index_keys = 31;
    assert_eq!(
        guard.validate(),
        Err(PostgresSchemaIdentityUnavailableV1::GuardUnsupported(
            PostgresSchemaIdentityGuardCodeV1::IndexKeyLimit
        ))
    );
}

#[test]
fn text_envelope_is_nonempty_nulfree_and_bounded() {
    assert_eq!(
        BoundedCatalogTextV1::new("public".into()).unwrap().as_str(),
        "public"
    );
    assert_eq!(
        BoundedCatalogTextV1::new(String::new()),
        Err(PostgresSchemaIdentityUnavailableV1::IdentityRejected)
    );
    assert_eq!(
        BoundedCatalogTextV1::new("bad\0name".into()),
        Err(PostgresSchemaIdentityUnavailableV1::IdentityRejected)
    );
    assert_eq!(
        BoundedCatalogTextV1::new("x".repeat(MAX_CATALOG_TEXT_BYTES_V1 + 1)),
        Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            PostgresSchemaIdentityLimitCodeV1::TextBytes
        ))
    );
}

#[test]
fn array_envelope_rejects_empty_and_cap_plus_one() {
    assert!(validate_catalog_array_len(&[1u8; MAX_CATALOG_ARRAY_MEMBERS_V1]).is_ok());
    assert_eq!(
        validate_catalog_array_len::<u8>(&[]),
        Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            PostgresSchemaIdentityLimitCodeV1::KeyMembers
        ))
    );
    assert_eq!(
        validate_catalog_array_len(&[1u8; MAX_CATALOG_ARRAY_MEMBERS_V1 + 1]),
        Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            PostgresSchemaIdentityLimitCodeV1::KeyMembers
        ))
    );
}

#[test]
fn typed_rows_reject_overflow_and_incomplete_join_shapes() {
    let mut relation = CatalogRelationRowV1 {
        relation_oid: 1,
        relation_namespace_oid: 2,
        joined_namespace_oid: 2,
        namespace_name: "public".into(),
        relation_name: "orders".into(),
        text_overflow: false,
        relation_kind: 'r',
        persistence: 'p',
        is_partition: false,
        is_shared: false,
        row_security: false,
        force_row_security: false,
        of_type_oid: 0,
        rewrite_oid: 0,
        access_method_oid: 1,
        joined_access_method_oid: Some(1),
        access_method_namespace: Some("pg_catalog".into()),
        access_method_name: Some("heap".into()),
        access_method_type: Some('t'),
        inherits_as_child: false,
        inherits_as_parent: false,
        physical_attribute_count: 1,
    };
    assert!(relation.validate().is_ok());
    relation.text_overflow = true;
    assert!(relation.validate().is_err());
}

#[test]
fn typed_attribute_rows_require_live_names_but_allow_dropped_names_absent() {
    let dropped = CatalogAttributeRowV1 {
        relation_oid: 1,
        attribute_number: 1,
        attribute_name: None,
        text_overflow: false,
        is_dropped: true,
        is_local: false,
        inheritance_count: 9,
        is_not_null: false,
        attribute_type_oid: 0,
        array_dimensions: 0,
        type_modifier: -1,
        collation_oid: 0,
        joined_type_oid: None,
        joined_type_name: None,
        joined_type_namespace_oid: None,
        joined_type_kind: None,
        joined_type_is_defined: None,
        joined_type_base_oid: None,
        joined_type_element_oid: None,
        joined_type_relation_oid: None,
        joined_type_collation_oid: None,
        joined_collation_oid: None,
        joined_collation_name: None,
        joined_collation_namespace_oid: None,
        joined_collation_provider: None,
        joined_collation_encoding: None,
        joined_collation_is_deterministic: None,
        joined_collation_version: None,
        physical_ordinal: 1,
        physical_overflow: false,
    };
    assert!(dropped.validate().is_ok());
    let mut live = dropped.clone();
    live.is_dropped = false;
    assert!(live.validate().is_err());
    let mut overflow = dropped;
    overflow.physical_ordinal = 1_601;
    overflow.physical_overflow = true;
    assert_eq!(
        overflow.validate(),
        Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            PostgresSchemaIdentityLimitCodeV1::PhysicalAttributes
        ))
    );
}
