use sf_core::schema_identity::{MAX_COLUMNS_TOTAL_V1, MAX_IDENTIFIER_BYTES_V1, MAX_RELATIONS_V1};

use super::tests::{
    default_collation, dropped_attribute, live_attribute, relation_fact, type_fact,
};
use super::*;

fn assert_reason(
    candidates: Vec<Postgres16RelationCatalogFactV1>,
    attributes: Vec<Postgres16AttributeCatalogFactV1>,
    expected: PostgresSchemaIdentityUnavailableV1,
) {
    match normalize_postgres16_relations_v1(candidates, attributes) {
        Err(error) => assert_eq!(error, expected),
        Ok(_) => panic!("relation normalization unexpectedly succeeded"),
    }
}

fn assert_reason_with_limits(
    candidates: Vec<Postgres16RelationCatalogFactV1>,
    attributes: Vec<Postgres16AttributeCatalogFactV1>,
    limits: Postgres16RelationLimitsV1,
    expected: PostgresSchemaIdentityUnavailableV1,
) {
    match normalize_postgres16_relations_with_limits_v1(candidates, attributes, limits) {
        Err(error) => assert_eq!(error, expected),
        Ok(_) => panic!("bounded relation normalization unexpectedly succeeded"),
    }
}

fn generous_limits() -> Postgres16RelationLimitsV1 {
    Postgres16RelationLimitsV1 {
        relations: 16,
        physical_per_relation: 16,
        physical_total: 32,
        live_total: 32,
        raw_text_bytes: usize::MAX,
        semantic_text_bytes: usize::MAX,
    }
}

#[test]
fn physical_slot_topology_rejects_duplicates_gaps_ranges_and_orphans() {
    let expected = PostgresSchemaIdentityUnavailableV1::UnsupportedRelation;
    for numbers in [[0, 2], [-1, 2], [1, 1], [1, 3], [2, 3]] {
        assert_reason(
            vec![relation_fact(42, "orders", 2)],
            numbers
                .into_iter()
                .map(|number| live_attribute(42, number, "id", 23, "int4", false))
                .collect(),
            expected,
        );
    }
    assert_reason(
        vec![relation_fact(42, "orders", 2)],
        vec![live_attribute(42, 1, "id", 23, "int4", false)],
        expected,
    );
    assert_reason(
        Vec::new(),
        vec![live_attribute(999, 1, "id", 23, "int4", false)],
        expected,
    );

    assert_reason(
        vec![relation_fact(42, "a", 0), relation_fact(42, "b", 0)],
        Vec::new(),
        expected,
    );
    assert_reason(
        vec![relation_fact(42, "same", 0), relation_fact(43, "same", 0)],
        Vec::new(),
        expected,
    );
}

#[test]
fn dropped_and_live_join_shapes_cannot_be_substituted() {
    let type_error = PostgresSchemaIdentityUnavailableV1::UnsupportedType;
    let collation_error = PostgresSchemaIdentityUnavailableV1::UnsupportedCollation;

    let mut dropped_type_oid = dropped_attribute(42, 1);
    dropped_type_oid.attribute_type_oid = 23;
    assert_reason(
        vec![relation_fact(42, "orders", 1)],
        vec![dropped_type_oid],
        type_error,
    );
    let mut dropped_joined_type = dropped_attribute(42, 1);
    dropped_joined_type.joined_type = Some(type_fact(23, "int4", 0));
    assert_reason(
        vec![relation_fact(42, "orders", 1)],
        vec![dropped_joined_type],
        type_error,
    );
    let mut dropped_collation = dropped_attribute(42, 1);
    dropped_collation.joined_default_collation = Some(default_collation());
    assert_reason(
        vec![relation_fact(42, "orders", 1)],
        vec![dropped_collation],
        collation_error,
    );

    let base = live_attribute(42, 1, "id", 23, "int4", false);
    let mut live_zero_type = base.clone();
    live_zero_type.attribute_type_oid = 0;
    live_zero_type.joined_type = None;
    assert_reason(
        vec![relation_fact(42, "orders", 1)],
        vec![live_zero_type],
        type_error,
    );
    let mut live_missing_type = base.clone();
    live_missing_type.joined_type = None;
    assert_reason(
        vec![relation_fact(42, "orders", 1)],
        vec![live_missing_type],
        type_error,
    );
    let mut wrong_type_oid = base.clone();
    wrong_type_oid.joined_type.as_mut().unwrap().type_oid = 21;
    assert_reason(
        vec![relation_fact(42, "orders", 1)],
        vec![wrong_type_oid],
        type_error,
    );
    let mut wrong_dimensions = base.clone();
    wrong_dimensions.array_dimensions = 1;
    assert_reason(
        vec![relation_fact(42, "orders", 1)],
        vec![wrong_dimensions],
        type_error,
    );
    let mut wrong_modifier = base.clone();
    wrong_modifier.type_modifier = 7;
    assert_reason(
        vec![relation_fact(42, "orders", 1)],
        vec![wrong_modifier],
        type_error,
    );
    let mut wrong_collation = live_attribute(42, 1, "body", 25, "text", false);
    wrong_collation.collation_oid = 0;
    assert_reason(
        vec![relation_fact(42, "orders", 1)],
        vec![wrong_collation],
        collation_error,
    );
}

#[test]
fn live_columns_must_be_local_uninherited_and_uniquely_named() {
    let expected = PostgresSchemaIdentityUnavailableV1::UnsupportedRelation;
    let mut not_local = live_attribute(42, 1, "id", 23, "int4", false);
    not_local.is_local = false;
    assert_reason(
        vec![relation_fact(42, "orders", 1)],
        vec![not_local],
        expected,
    );
    for count in [-1, 1] {
        let mut inherited = live_attribute(42, 1, "id", 23, "int4", false);
        inherited.inheritance_count = count;
        assert_reason(
            vec![relation_fact(42, "orders", 1)],
            vec![inherited],
            expected,
        );
    }
    assert_reason(
        vec![relation_fact(42, "orders", 2)],
        vec![
            live_attribute(42, 1, "same", 23, "int4", false),
            live_attribute(42, 2, "same", 23, "int4", false),
        ],
        expected,
    );
}

#[test]
fn collection_and_identifier_caps_are_closed() {
    assert_reason(
        vec![relation_fact(42, "orders", 0); MAX_RELATIONS_V1 + 1],
        Vec::new(),
        PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            super::super::PostgresSchemaIdentityLimitCodeV1::RichRelations,
        ),
    );
    assert_reason(
        Vec::new(),
        vec![dropped_attribute(42, 1); MAX_COLUMNS_TOTAL_V1 + 1],
        PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            super::super::PostgresSchemaIdentityLimitCodeV1::PhysicalAttributes,
        ),
    );
    assert_reason(
        vec![relation_fact(42, "orders", 1_601)],
        Vec::new(),
        PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            super::super::PostgresSchemaIdentityLimitCodeV1::PhysicalAttributes,
        ),
    );

    let mut overlong_relation = relation_fact(42, "orders", 1);
    overlong_relation.relation_name = "r".repeat(MAX_IDENTIFIER_BYTES_V1 + 1).into_boxed_str();
    assert_reason(
        vec![overlong_relation],
        vec![live_attribute(42, 1, "id", 23, "int4", false)],
        PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            super::super::PostgresSchemaIdentityLimitCodeV1::TextBytes,
        ),
    );
    let mut overlong_column = live_attribute(42, 1, "id", 23, "int4", false);
    overlong_column.attribute_name = "c".repeat(MAX_IDENTIFIER_BYTES_V1 + 1).into_boxed_str();
    assert_reason(
        vec![relation_fact(42, "orders", 1)],
        vec![overlong_column],
        PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            super::super::PostgresSchemaIdentityLimitCodeV1::TextBytes,
        ),
    );
}

#[test]
fn exact_physical_per_relation_cap_accepts_dropped_slots_without_dense_gaps() {
    let mut attributes = Vec::with_capacity(MAX_PHYSICAL_ATTRIBUTES_PER_RELATION_PG16_V1);
    attributes.push(live_attribute(42, 1, "id", 23, "int4", false));
    attributes.extend(
        (2..=MAX_PHYSICAL_ATTRIBUTES_PER_RELATION_PG16_V1)
            .map(|number| dropped_attribute(42, i16::try_from(number).unwrap())),
    );
    let normalized = normalize_postgres16_relations_v1(
        vec![relation_fact(
            42,
            "orders",
            MAX_PHYSICAL_ATTRIBUTES_PER_RELATION_PG16_V1 as i16,
        )],
        attributes,
    )
    .unwrap();
    assert_eq!(normalized.relations[0].columns.len(), 1);
    assert_eq!(
        normalized.coordinates_by_relation_oid[&42]
            .attributes_by_number
            .len(),
        MAX_PHYSICAL_ATTRIBUTES_PER_RELATION_PG16_V1
    );
}

#[test]
fn independently_injected_limits_pin_each_collection_boundary() {
    assert_eq!(PRODUCTION_LIMITS.relations, 4_096);
    assert_eq!(PRODUCTION_LIMITS.physical_per_relation, 1_600);
    assert_eq!(PRODUCTION_LIMITS.physical_total, 65_536);
    assert_eq!(PRODUCTION_LIMITS.live_total, 65_536);

    let mut relation_limits = generous_limits();
    relation_limits.relations = 2;
    let exact_relations = vec![relation_fact(1, "a", 1), relation_fact(2, "b", 1)];
    let exact_attributes = vec![
        live_attribute(2, 1, "id", 23, "int4", false),
        live_attribute(1, 1, "id", 23, "int4", false),
    ];
    assert!(normalize_postgres16_relations_with_limits_v1(
        exact_relations.clone(),
        exact_attributes.clone(),
        relation_limits,
    )
    .is_ok());
    let mut too_many_relations = exact_relations;
    too_many_relations.push(relation_fact(3, "c", 1));
    assert_reason_with_limits(
        too_many_relations,
        exact_attributes,
        relation_limits,
        PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            super::super::PostgresSchemaIdentityLimitCodeV1::RichRelations,
        ),
    );

    let mut physical_limits = generous_limits();
    physical_limits.physical_per_relation = 3;
    physical_limits.physical_total = 5;
    physical_limits.live_total = 5;
    let exact_physical = vec![
        live_attribute(1, 1, "id", 23, "int4", false),
        dropped_attribute(1, 2),
        dropped_attribute(1, 3),
        live_attribute(2, 1, "id", 23, "int4", false),
        dropped_attribute(2, 2),
    ];
    assert!(normalize_postgres16_relations_with_limits_v1(
        vec![relation_fact(1, "a", 3), relation_fact(2, "b", 2)],
        exact_physical,
        physical_limits,
    )
    .is_ok());
    assert_reason_with_limits(
        vec![relation_fact(1, "a", 4)],
        Vec::new(),
        physical_limits,
        PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            super::super::PostgresSchemaIdentityLimitCodeV1::PhysicalAttributes,
        ),
    );
    assert_reason_with_limits(
        vec![relation_fact(1, "a", 3), relation_fact(2, "b", 3)],
        Vec::new(),
        physical_limits,
        PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            super::super::PostgresSchemaIdentityLimitCodeV1::PhysicalAttributes,
        ),
    );

    let mut live_limits = generous_limits();
    live_limits.physical_per_relation = 4;
    live_limits.physical_total = 4;
    live_limits.live_total = 3;
    assert_reason_with_limits(
        vec![relation_fact(1, "a", 4)],
        (1..=4)
            .map(|number| live_attribute(1, number, &format!("c{number}"), 23, "int4", false))
            .collect(),
        live_limits,
        PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            super::super::PostgresSchemaIdentityLimitCodeV1::LiveColumns,
        ),
    );
    live_limits.live_total = 1;
    assert!(normalize_postgres16_relations_with_limits_v1(
        vec![relation_fact(1, "a", 3)],
        vec![
            live_attribute(1, 1, "id", 23, "int4", false),
            dropped_attribute(1, 2),
            dropped_attribute(1, 3),
        ],
        live_limits,
    )
    .is_ok());

    let mut cross_relation_live_limits = generous_limits();
    cross_relation_live_limits.physical_per_relation = 2;
    cross_relation_live_limits.physical_total = 4;
    cross_relation_live_limits.live_total = 3;
    assert_reason_with_limits(
        vec![relation_fact(1, "a", 2), relation_fact(2, "b", 2)],
        vec![
            live_attribute(1, 1, "a1", 23, "int4", false),
            live_attribute(1, 2, "a2", 23, "int4", false),
            live_attribute(2, 1, "b1", 23, "int4", false),
            live_attribute(2, 2, "b2", 23, "int4", false),
        ],
        cross_relation_live_limits,
        PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            super::super::PostgresSchemaIdentityLimitCodeV1::LiveColumns,
        ),
    );
}

#[test]
fn raw_and_semantic_cumulative_text_limits_are_distinct() {
    let candidate = relation_fact(42, "orders", 1);
    let attribute = live_attribute(42, 1, "id", 23, "int4", false);
    let mut raw_limits = generous_limits();
    raw_limits.raw_text_bytes = 0;
    assert_reason_with_limits(
        vec![candidate.clone()],
        vec![attribute.clone()],
        raw_limits,
        PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            super::super::PostgresSchemaIdentityLimitCodeV1::CanonicalBody,
        ),
    );

    let mut semantic_limits = generous_limits();
    semantic_limits.semantic_text_bytes = 0;
    assert_reason_with_limits(
        vec![candidate],
        vec![attribute],
        semantic_limits,
        PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            super::super::PostgresSchemaIdentityLimitCodeV1::TextBytes,
        ),
    );
}

#[test]
fn every_relation_text_field_has_an_independent_bound_and_errors_are_redacted() {
    let expected = PostgresSchemaIdentityUnavailableV1::LimitExceeded(
        super::super::PostgresSchemaIdentityLimitCodeV1::TextBytes,
    );
    let oversized = "SECRET_SENTINEL".repeat(MAX_IDENTIFIER_BYTES_V1 + 1);
    let mut namespace = relation_fact(42, "orders", 1);
    namespace.namespace_name = oversized.clone().into_boxed_str();
    assert_reason(
        vec![namespace],
        vec![live_attribute(42, 1, "id", 23, "int4", false)],
        expected,
    );
    let mut access_method = relation_fact(42, "orders", 1);
    access_method.access_method_name = Some(oversized.clone().into_boxed_str());
    assert_reason(
        vec![access_method],
        vec![live_attribute(42, 1, "id", 23, "int4", false)],
        expected,
    );
    let mut dropped = dropped_attribute(42, 2);
    dropped.attribute_name = oversized.into_boxed_str();
    assert_reason(
        vec![relation_fact(42, "orders", 2)],
        vec![live_attribute(42, 1, "id", 23, "int4", false), dropped],
        expected,
    );
    assert!(!expected.to_string().contains("SECRET_SENTINEL"));
    assert!(!format!("{expected:?}").contains("SECRET_SENTINEL"));
}

#[test]
fn semantic_names_reject_empty_and_nul_while_dropped_names_are_ignored() {
    let expected = PostgresSchemaIdentityUnavailableV1::IdentityRejected;
    for name in ["", "bad\0name"] {
        let mut relation = relation_fact(42, "orders", 1);
        relation.relation_name = name.into();
        assert_reason(
            vec![relation],
            vec![live_attribute(42, 1, "id", 23, "int4", false)],
            expected,
        );
        let mut column = live_attribute(42, 1, "id", 23, "int4", false);
        column.attribute_name = name.into();
        assert_reason(vec![relation_fact(42, "orders", 1)], vec![column], expected);
    }

    for name in ["", "ignored\0dropped"] {
        let mut dropped = dropped_attribute(42, 2);
        dropped.attribute_name = name.into();
        assert!(normalize_postgres16_relations_v1(
            vec![relation_fact(42, "orders", 2)],
            vec![live_attribute(42, 1, "id", 23, "int4", false), dropped,],
        )
        .is_ok());
    }
}
