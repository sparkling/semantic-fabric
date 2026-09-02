use sf_core::schema_identity::{
    SourceTypeV1, TypeFacetValueV1, TypeFamilyV1, MAX_FACET_TEXT_BYTES_V1, MAX_IDENTIFIER_BYTES_V1,
};

use super::*;

const TYPES: &[(u32, &str, TypeFamilyV1)] = &[
    (16, "bool", TypeFamilyV1::Boolean),
    (21, "int2", TypeFamilyV1::SignedInteger),
    (23, "int4", TypeFamilyV1::SignedInteger),
    (20, "int8", TypeFamilyV1::SignedInteger),
    (1700, "numeric", TypeFamilyV1::ExactNumeric),
    (700, "float4", TypeFamilyV1::ApproximateNumeric),
    (701, "float8", TypeFamilyV1::ApproximateNumeric),
    (25, "text", TypeFamilyV1::Character),
    (1043, "varchar", TypeFamilyV1::Character),
    (1042, "bpchar", TypeFamilyV1::Character),
    (17, "bytea", TypeFamilyV1::Binary),
    (1082, "date", TypeFamilyV1::Date),
    (1083, "time", TypeFamilyV1::Time),
    (1266, "timetz", TypeFamilyV1::Time),
    (1114, "timestamp", TypeFamilyV1::Timestamp),
    (1184, "timestamptz", TypeFamilyV1::Timestamp),
    (114, "json", TypeFamilyV1::Json),
    (3802, "jsonb", TypeFamilyV1::Json),
    (2950, "uuid", TypeFamilyV1::Uuid),
];

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
        collation_oid: if is_character(name) { 100 } else { 0 },
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
        database_collate: "C.UTF-8".into(),
        database_ctype: "C.UTF-8".into(),
        database_icu_locale: None,
        database_icu_rules: None,
        database_recorded_version: None,
        actual_version: None,
    }
}

fn icu_collation() -> Postgres16DefaultCollationCatalogFactV1 {
    Postgres16DefaultCollationCatalogFactV1 {
        database_provider: 'i',
        database_collate: "en_US.UTF-8".into(),
        database_ctype: "en_US.UTF-8".into(),
        database_icu_locale: Some("en-US".into()),
        database_icu_rules: Some("&a<b".into()),
        database_recorded_version: Some("153.120".into()),
        actual_version: Some("153.120".into()),
        ..libc_collation()
    }
}

fn is_character(name: &str) -> bool {
    matches!(name, "text" | "varchar" | "bpchar")
}

fn normalize(
    fact: &Postgres16ColumnTypeCatalogFactV1,
) -> Result<SourceTypeV1, PostgresSchemaIdentityUnavailableV1> {
    let collation = libc_collation();
    normalize_postgres16_source_type_v1(fact, is_character(&fact.type_name).then_some(&collation))
}

fn keys(source_type: &SourceTypeV1) -> Vec<&str> {
    source_type
        .facets
        .iter()
        .map(|facet| facet.key.as_str())
        .collect()
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

fn assert_unavailable(
    fact: &Postgres16ColumnTypeCatalogFactV1,
    collation: Option<&Postgres16DefaultCollationCatalogFactV1>,
    expected: PostgresSchemaIdentityUnavailableV1,
) {
    assert_eq!(
        normalize_postgres16_source_type_v1(fact, collation).unwrap_err(),
        expected
    );
}

#[test]
fn all_bootstrap_oid_name_pairs_normalize_exactly() {
    for &(oid, name, family) in TYPES {
        let source_type = normalize(&type_fact(oid, name)).unwrap();
        assert!(source_type.native_name.catalog.is_none());
        assert_eq!(
            source_type.native_name.schema.as_ref().unwrap().as_str(),
            "pg_catalog"
        );
        assert_eq!(source_type.native_name.local.as_str(), name);
        assert_eq!(source_type.family, family);

        let expected_keys: &[&str] = match name {
            "int2" | "int4" | "int8" | "float4" | "float8" => &["bit-width"],
            "text" | "varchar" | "bpchar" => &[
                "collation-name",
                "collation-provider",
                "collation-deterministic",
                "collation-collate",
                "collation-ctype",
            ],
            "time" | "timetz" | "timestamp" | "timestamptz" => {
                &["fractional-second-precision", "with-time-zone"]
            }
            _ => &[],
        };
        assert_eq!(keys(&source_type), expected_keys, "wrong facets for {name}");
        let expected_width = match name {
            "int2" => Some(16),
            "int4" | "float4" => Some(32),
            "int8" | "float8" => Some(64),
            _ => None,
        };
        if let Some(width) = expected_width {
            assert_eq!(
                facet(&source_type, "bit-width"),
                &TypeFacetValueV1::U64(width)
            );
        }
    }
}

#[test]
fn type_catalogue_shape_and_oid_name_pair_are_exact() {
    let expected = PostgresSchemaIdentityUnavailableV1::UnsupportedType;
    let base = type_fact(20, "int8");
    let mut mutations = Vec::new();
    let mut value = base.clone();
    value.type_namespace = "public".into();
    mutations.push(value);
    let mut value = base.clone();
    value.type_kind = 'd';
    mutations.push(value);
    let mut value = base.clone();
    value.type_is_defined = false;
    mutations.push(value);
    let mut value = base.clone();
    value.type_base_oid = 20;
    mutations.push(value);
    let mut value = base.clone();
    value.type_element_oid = 20;
    mutations.push(value);
    let mut value = base.clone();
    value.type_relation_oid = 42;
    mutations.push(value);
    let mut value = base;
    value.array_dimensions = 1;
    mutations.push(value);
    for mutation in &mutations {
        assert_unavailable(mutation, None, expected);
    }

    for &(oid, correct_name, _) in TYPES {
        for &(_, candidate_name, _) in TYPES {
            if candidate_name != correct_name {
                assert_unavailable(&type_fact(oid, candidate_name), None, expected);
            }
        }
    }
    assert_unavailable(&type_fact(9_999, "mystery"), None, expected);
}

#[test]
fn fixed_types_require_typmod_minus_one() {
    for &(oid, name) in &[
        (16, "bool"),
        (21, "int2"),
        (23, "int4"),
        (20, "int8"),
        (700, "float4"),
        (701, "float8"),
        (25, "text"),
        (17, "bytea"),
        (1082, "date"),
        (114, "json"),
        (3802, "jsonb"),
        (2950, "uuid"),
    ] {
        assert!(normalize(&type_fact(oid, name)).is_ok());
        for modifier in [-2, 0, 1] {
            let mut fact = type_fact(oid, name);
            fact.type_modifier = modifier;
            assert_unavailable(
                &fact,
                is_character(name).then_some(&libc_collation()),
                PostgresSchemaIdentityUnavailableV1::UnsupportedType,
            );
        }
    }
}

#[test]
fn varchar_and_bpchar_typmods_are_exact() {
    for &(modifier, maximum) in &[(-1, None), (5, Some(1)), (10_485_764, Some(10_485_760))] {
        for &(oid, name) in &[(1043, "varchar"), (1042, "bpchar")] {
            let mut fact = type_fact(oid, name);
            fact.type_modifier = modifier;
            let source_type = normalize(&fact).unwrap();
            assert_eq!(source_type.native_name.local.as_str(), name);
            match maximum {
                Some(expected) => assert_eq!(
                    facet(&source_type, "character-maximum"),
                    &TypeFacetValueV1::U64(expected)
                ),
                None => assert!(!keys(&source_type).contains(&"character-maximum")),
            }
        }
    }
    for modifier in [4, 10_485_765] {
        let mut fact = type_fact(1043, "varchar");
        fact.type_modifier = modifier;
        assert_unavailable(
            &fact,
            Some(&libc_collation()),
            PostgresSchemaIdentityUnavailableV1::UnsupportedType,
        );
    }
}

#[test]
fn numeric_typmod_boundaries_and_reencoding_are_exact() {
    let mut unconstrained = type_fact(1700, "numeric");
    assert!(normalize(&unconstrained).unwrap().facets.is_empty());
    for &(modifier, precision, scale) in &[
        (66_588, 1, -1000),
        (66_540, 1, 1000),
        (65_537_052, 1000, -1000),
        (65_537_004, 1000, 1000),
    ] {
        unconstrained.type_modifier = modifier;
        let source_type = normalize(&unconstrained).unwrap();
        assert_eq!(
            facet(&source_type, "numeric-precision"),
            &TypeFacetValueV1::U64(precision)
        );
        assert_eq!(
            facet(&source_type, "numeric-scale"),
            &TypeFacetValueV1::I64(scale)
        );
        assert_eq!(keys(&source_type), ["numeric-precision", "numeric-scale"]);
    }
    for modifier in [3, 4, 65_601_540, 66_587, 66_541, 657_414] {
        unconstrained.type_modifier = modifier;
        assert_unavailable(
            &unconstrained,
            None,
            PostgresSchemaIdentityUnavailableV1::UnsupportedType,
        );
    }
}

#[test]
fn temporal_typmod_matrix_and_timezone_flags_are_exact() {
    for &(oid, name, zoned) in &[
        (1083, "time", false),
        (1266, "timetz", true),
        (1114, "timestamp", false),
        (1184, "timestamptz", true),
    ] {
        let mut fact = type_fact(oid, name);
        fact.type_modifier = -1;
        let implicit = normalize(&fact).unwrap();
        fact.type_modifier = 6;
        let explicit = normalize(&fact).unwrap();
        assert_eq!(implicit, explicit);
        assert_eq!(
            facet(&implicit, "fractional-second-precision"),
            &TypeFacetValueV1::U64(6)
        );
        assert_eq!(
            facet(&implicit, "with-time-zone"),
            &TypeFacetValueV1::Bool(zoned)
        );
        for modifier in [0, 6] {
            fact.type_modifier = modifier;
            assert!(normalize(&fact).is_ok());
        }
        for modifier in [-2, 7] {
            fact.type_modifier = modifier;
            assert_unavailable(
                &fact,
                None,
                PostgresSchemaIdentityUnavailableV1::UnsupportedType,
            );
        }
    }
}

#[test]
fn collation_coordinate_and_catalogue_shape_are_exact() {
    let expected = PostgresSchemaIdentityUnavailableV1::UnsupportedCollation;
    let character = type_fact(25, "text");
    assert_unavailable(&character, None, expected);
    let mut wrong_oid = character.clone();
    wrong_oid.collation_oid = 0;
    assert_unavailable(&wrong_oid, Some(&libc_collation()), expected);

    let non_character = type_fact(20, "int8");
    assert_unavailable(&non_character, Some(&libc_collation()), expected);
    let mut wrong_oid = non_character;
    wrong_oid.collation_oid = 100;
    assert_unavailable(&wrong_oid, Some(&libc_collation()), expected);

    let base = libc_collation();
    let mut mutations = Vec::new();
    let mut value = base.clone();
    value.collation_oid = 101;
    mutations.push(value);
    let mut value = base.clone();
    value.collation_namespace = "public".into();
    mutations.push(value);
    let mut value = base.clone();
    value.collation_name = "hostile".into();
    mutations.push(value);
    let mut value = base.clone();
    value.collation_provider = 'c';
    mutations.push(value);
    let mut value = base.clone();
    value.collation_encoding = 0;
    mutations.push(value);
    let mut value = base;
    value.collation_is_deterministic = false;
    mutations.push(value);
    for mutation in &mutations {
        assert_unavailable(&character, Some(mutation), expected);
    }
}

#[test]
fn libc_and_icu_provider_shapes_emit_only_recorded_catalogue_state() {
    let character = type_fact(25, "text");
    let libc = normalize_postgres16_source_type_v1(&character, Some(&libc_collation())).unwrap();
    assert_eq!(
        keys(&libc),
        [
            "collation-name",
            "collation-provider",
            "collation-deterministic",
            "collation-collate",
            "collation-ctype",
        ]
    );
    assert_eq!(text_facet(&libc, "collation-provider"), "libc");
    assert_eq!(text_facet(&libc, "collation-collate"), "C.UTF-8");
    assert_eq!(text_facet(&libc, "collation-ctype"), "C.UTF-8");
    assert_eq!(
        facet(&libc, "collation-deterministic"),
        &TypeFacetValueV1::Bool(true)
    );
    let TypeFacetValueV1::TypeName(collation_name) = facet(&libc, "collation-name") else {
        panic!("collation-name was not a qualified name");
    };
    assert!(collation_name.catalog.is_none());
    assert_eq!(
        collation_name.schema.as_ref().unwrap().as_str(),
        "pg_catalog"
    );
    assert_eq!(collation_name.local.as_str(), "default");

    let mut empty = libc_collation();
    empty.database_collate = "".into();
    empty.database_ctype = "".into();
    let empty = normalize_postgres16_source_type_v1(&character, Some(&empty)).unwrap();
    assert_eq!(text_facet(&empty, "collation-collate"), "");
    assert_eq!(text_facet(&empty, "collation-ctype"), "");

    let icu_facts = icu_collation();
    let icu = normalize_postgres16_source_type_v1(&character, Some(&icu_facts)).unwrap();
    assert_eq!(
        keys(&icu),
        [
            "collation-name",
            "collation-provider",
            "collation-deterministic",
            "collation-collate",
            "collation-ctype",
            "collation-icu-locale",
            "collation-icu-rules",
            "collation-version",
        ]
    );
    assert_eq!(text_facet(&icu, "collation-provider"), "icu");
    assert_eq!(text_facet(&icu, "collation-icu-locale"), "en-US");
    assert_eq!(text_facet(&icu, "collation-icu-rules"), "&a<b");
    assert_eq!(text_facet(&icu, "collation-version"), "153.120");

    let mut icu_without_rules = icu_collation();
    icu_without_rules.database_icu_rules = None;
    let icu_without_rules =
        normalize_postgres16_source_type_v1(&character, Some(&icu_without_rules)).unwrap();
    assert!(!keys(&icu_without_rules).contains(&"collation-icu-rules"));
}

#[test]
fn provider_and_version_mismatches_are_rejected() {
    let character = type_fact(25, "text");
    let expected = PostgresSchemaIdentityUnavailableV1::UnsupportedCollation;
    let mut libc_with_locale = libc_collation();
    libc_with_locale.database_icu_locale = Some("en-US".into());
    assert_unavailable(&character, Some(&libc_with_locale), expected);
    let mut libc_with_rules = libc_collation();
    libc_with_rules.database_icu_rules = Some("&a<b".into());
    assert_unavailable(&character, Some(&libc_with_rules), expected);
    let mut icu_without_locale = icu_collation();
    icu_without_locale.database_icu_locale = None;
    assert_unavailable(&character, Some(&icu_without_locale), expected);
    let mut unknown = libc_collation();
    unknown.database_provider = 'x';
    assert_unavailable(&character, Some(&unknown), expected);
    for (recorded, actual) in [(Some("1"), Some("2")), (Some("1"), None), (None, Some("1"))] {
        let mut mismatch = libc_collation();
        mismatch.database_recorded_version = recorded.map(Into::into);
        mismatch.actual_version = actual.map(Into::into);
        assert_unavailable(&character, Some(&mismatch), expected);
    }
}

#[test]
fn rich_catalogue_text_limits_are_closed_and_redacted() {
    let marker = "SECRET_SENTINEL";
    let mut hostile = type_fact(20, marker);
    let error = normalize_postgres16_source_type_v1(&hostile, None).unwrap_err();
    assert_eq!(error, PostgresSchemaIdentityUnavailableV1::UnsupportedType);
    assert!(!error.to_string().contains(marker));
    assert!(!format!("{error:?}").contains(marker));

    hostile.type_name = "x".repeat(MAX_IDENTIFIER_BYTES_V1 + 1).into_boxed_str();
    assert_unavailable(
        &hostile,
        None,
        PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            super::super::PostgresSchemaIdentityLimitCodeV1::TextBytes,
        ),
    );

    let character = type_fact(25, "text");
    let mut oversized = icu_collation();
    oversized.database_icu_rules = Some("x".repeat(MAX_FACET_TEXT_BYTES_V1 + 1).into_boxed_str());
    assert_unavailable(
        &character,
        Some(&oversized),
        PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            super::super::PostgresSchemaIdentityLimitCodeV1::TextBytes,
        ),
    );
}
