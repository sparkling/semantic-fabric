use sf_core::schema_identity::{
    SourceTypeV1, TypeFacetValueV1, MAX_FACET_TEXT_BYTES_V1, MAX_IDENTIFIER_BYTES_V1,
};

use super::*;

fn type_fact(oid: u32, name: &str) -> Postgres16ColumnTypeCatalogFactV1 {
    Postgres16ColumnTypeCatalogFactV1 {
        type_oid: oid,
        type_namespace: "pg_catalog".into(),
        type_name: name.into(),
        type_kind: 'b',
        type_is_defined: true,
        type_base_oid: 0,
        type_element_oid: 0,
        type_relation_oid: 0,
        array_dimensions: 0,
        type_modifier: -1,
        collation_oid: if matches!(name, "text" | "varchar" | "bpchar") {
            100
        } else {
            0
        },
    }
}

fn libc_collation() -> Postgres16DefaultCollationCatalogFactV1 {
    Postgres16DefaultCollationCatalogFactV1 {
        collation_oid: 100,
        collation_namespace: "pg_catalog".into(),
        collation_name: "default".into(),
        collation_provider: 'd',
        collation_encoding: -1,
        collation_is_deterministic: true,
        database_provider: 'c',
        database_collate: "distinct-collate".into(),
        database_ctype: "distinct-ctype".into(),
        database_icu_locale: None,
        database_icu_rules: None,
        database_recorded_version: None,
        actual_version: None,
    }
}

fn icu_collation() -> Postgres16DefaultCollationCatalogFactV1 {
    Postgres16DefaultCollationCatalogFactV1 {
        database_provider: 'i',
        database_icu_locale: Some("locale".into()),
        database_icu_rules: Some("rules".into()),
        ..libc_collation()
    }
}

fn facet<'a>(source_type: &'a SourceTypeV1, key: &str) -> &'a TypeFacetValueV1 {
    &source_type
        .facets
        .iter()
        .find(|facet| facet.key.as_str() == key)
        .unwrap_or_else(|| panic!("missing facet {key}"))
        .value
}

fn text_facet<'a>(source_type: &'a SourceTypeV1, key: &str) -> &'a str {
    match facet(source_type, key) {
        TypeFacetValueV1::Text(value) => value.as_str(),
        value => panic!("facet {key} was not text: {value:?}"),
    }
}

fn assert_reason(
    fact: &Postgres16ColumnTypeCatalogFactV1,
    collation: Option<&Postgres16DefaultCollationCatalogFactV1>,
    expected: PostgresSchemaIdentityUnavailableV1,
) {
    assert_eq!(
        normalize_postgres16_source_type_v1(fact, collation).unwrap_err(),
        expected
    );
}

fn text_limit_error() -> PostgresSchemaIdentityUnavailableV1 {
    PostgresSchemaIdentityUnavailableV1::LimitExceeded(
        super::super::PostgresSchemaIdentityLimitCodeV1::TextBytes,
    )
}

#[test]
fn every_explicit_temporal_precision_is_preserved() {
    for &(oid, name) in &[
        (1083, "time"),
        (1266, "timetz"),
        (1114, "timestamp"),
        (1184, "timestamptz"),
    ] {
        for precision in 0..=6 {
            let mut fact = type_fact(oid, name);
            fact.type_modifier = precision;
            let normalized = normalize_postgres16_source_type_v1(&fact, None).unwrap();
            assert_eq!(
                facet(&normalized, "fractional-second-precision"),
                &TypeFacetValueV1::U64(precision as u64)
            );
        }
    }
}

#[test]
fn non_character_collation_oid_rejects_without_joined_facts() {
    let mut fact = type_fact(20, "int8");
    fact.collation_oid = 100;
    assert_reason(
        &fact,
        None,
        PostgresSchemaIdentityUnavailableV1::UnsupportedCollation,
    );
}

#[test]
fn distinct_collate_and_ctype_fields_cannot_be_swapped() {
    let fact = type_fact(25, "text");
    let normalized = normalize_postgres16_source_type_v1(&fact, Some(&libc_collation())).unwrap();
    assert_eq!(
        text_facet(&normalized, "collation-collate"),
        "distinct-collate"
    );
    assert_eq!(text_facet(&normalized, "collation-ctype"), "distinct-ctype");
}

#[test]
fn optional_collation_text_distinguishes_null_from_empty() {
    let fact = type_fact(25, "text");
    for field in ["locale", "rules"] {
        let mut libc = libc_collation();
        if field == "locale" {
            libc.database_icu_locale = Some("".into());
        } else {
            libc.database_icu_rules = Some("".into());
        }
        assert_reason(
            &fact,
            Some(&libc),
            PostgresSchemaIdentityUnavailableV1::UnsupportedCollation,
        );
    }

    let mut icu = icu_collation();
    icu.database_icu_locale = Some("".into());
    icu.database_icu_rules = Some("".into());
    icu.database_recorded_version = Some("".into());
    icu.actual_version = Some("".into());
    let normalized = normalize_postgres16_source_type_v1(&fact, Some(&icu)).unwrap();
    assert_eq!(text_facet(&normalized, "collation-icu-locale"), "");
    assert_eq!(text_facet(&normalized, "collation-icu-rules"), "");
    assert_eq!(text_facet(&normalized, "collation-version"), "");
}

#[test]
fn negative_and_reverse_boundaries_fail_closed() {
    let type_error = PostgresSchemaIdentityUnavailableV1::UnsupportedType;
    let collation_error = PostgresSchemaIdentityUnavailableV1::UnsupportedCollation;

    let mut dimensions = type_fact(20, "int8");
    dimensions.array_dimensions = -1;
    assert_reason(&dimensions, None, type_error);

    for &(oid, name) in &[(1700, "numeric"), (1043, "varchar"), (1042, "bpchar")] {
        let mut fact = type_fact(oid, name);
        fact.type_modifier = -2;
        let collation = libc_collation();
        assert_reason(
            &fact,
            matches!(name, "varchar" | "bpchar").then_some(&collation),
            type_error,
        );
    }

    let character = type_fact(25, "text");
    let mut attribute_oid = character.clone();
    attribute_oid.collation_oid = 101;
    assert_reason(&attribute_oid, Some(&libc_collation()), collation_error);
    let mut joined_oid = libc_collation();
    joined_oid.collation_oid = 99;
    assert_reason(&character, Some(&joined_oid), collation_error);
    let mut encoding = libc_collation();
    encoding.collation_encoding = -2;
    assert_reason(&character, Some(&encoding), collation_error);

    for &(oid, name) in &[(1043, "varchar"), (1042, "bpchar")] {
        for modifier in [4, 10_485_765] {
            let mut fact = type_fact(oid, name);
            fact.type_modifier = modifier;
            assert_reason(&fact, Some(&libc_collation()), type_error);
        }
    }
}

#[test]
fn collation_rules_accept_the_exact_text_cap() {
    let fact = type_fact(25, "text");
    let rules = "r".repeat(MAX_FACET_TEXT_BYTES_V1);
    let mut collation = icu_collation();
    collation.database_icu_rules = Some(rules.clone().into_boxed_str());
    let normalized = normalize_postgres16_source_type_v1(&fact, Some(&collation)).unwrap();
    assert_eq!(text_facet(&normalized, "collation-icu-rules"), rules);
}

#[test]
fn every_catalogue_text_field_enforces_its_own_byte_cap() {
    let oversized_identifier = "i".repeat(MAX_IDENTIFIER_BYTES_V1 + 1);
    let oversized_facet = "f".repeat(MAX_FACET_TEXT_BYTES_V1 + 1);

    let mut type_namespace = type_fact(20, "int8");
    type_namespace.type_namespace = oversized_identifier.clone().into_boxed_str();
    assert_reason(&type_namespace, None, text_limit_error());

    let mut type_name = type_fact(20, "int8");
    type_name.type_name = oversized_identifier.clone().into_boxed_str();
    assert_reason(&type_name, None, text_limit_error());

    let character = type_fact(25, "text");
    let mut mutations = Vec::new();

    let mut collation_namespace = libc_collation();
    collation_namespace.collation_namespace = oversized_identifier.clone().into_boxed_str();
    mutations.push(collation_namespace);

    let mut collation_name = libc_collation();
    collation_name.collation_name = oversized_identifier.into_boxed_str();
    mutations.push(collation_name);

    let mut database_collate = libc_collation();
    database_collate.database_collate = oversized_facet.clone().into_boxed_str();
    mutations.push(database_collate);

    let mut database_ctype = libc_collation();
    database_ctype.database_ctype = oversized_facet.clone().into_boxed_str();
    mutations.push(database_ctype);

    let mut database_icu_locale = icu_collation();
    database_icu_locale.database_icu_locale = Some(oversized_facet.clone().into_boxed_str());
    mutations.push(database_icu_locale);

    let mut database_icu_rules = icu_collation();
    database_icu_rules.database_icu_rules = Some(oversized_facet.clone().into_boxed_str());
    mutations.push(database_icu_rules);

    let mut database_recorded_version = libc_collation();
    database_recorded_version.database_recorded_version =
        Some(oversized_facet.clone().into_boxed_str());
    mutations.push(database_recorded_version);

    let mut actual_version = libc_collation();
    actual_version.actual_version = Some(oversized_facet.into_boxed_str());
    mutations.push(actual_version);

    for mutation in &mutations {
        assert_reason(&character, Some(mutation), text_limit_error());
    }
}

#[test]
fn exact_identifier_caps_are_not_reported_as_limit_failures() {
    let exact_cap = "i".repeat(MAX_IDENTIFIER_BYTES_V1);
    let mut namespace = type_fact(20, "int8");
    namespace.type_namespace = exact_cap.clone().into_boxed_str();
    assert_reason(
        &namespace,
        None,
        PostgresSchemaIdentityUnavailableV1::UnsupportedType,
    );

    let mut name = type_fact(20, "int8");
    name.type_name = exact_cap.into_boxed_str();
    assert_reason(
        &name,
        None,
        PostgresSchemaIdentityUnavailableV1::UnsupportedType,
    );
}
