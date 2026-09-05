use sf_core::schema_identity::{RelationKindV1, TypeFamilyV1};

use super::*;

pub(super) fn relation_fact(
    relation_oid: u32,
    relation_name: &str,
    physical_attribute_count: i16,
) -> Postgres16RelationCatalogFactV1 {
    Postgres16RelationCatalogFactV1 {
        relation_oid,
        relation_namespace_oid: 2_200,
        joined_namespace_oid: 2_200,
        namespace_name: "public".into(),
        relation_name: relation_name.into(),
        relation_kind: 'r',
        persistence: 'p',
        is_shared: false,
        is_partition: false,
        row_security: false,
        force_row_security: false,
        of_type_oid: 0,
        rewrite_oid: 0,
        access_method_oid: 2,
        joined_access_method_oid: Some(2),
        access_method_name: Some("heap".into()),
        access_method_type: Some('t'),
        inherits_as_child: false,
        inherits_as_parent: false,
        physical_attribute_count,
    }
}

pub(super) fn type_fact(
    oid: u32,
    name: &str,
    collation_oid: u32,
) -> Postgres16ColumnTypeCatalogFactV1 {
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
        collation_oid,
    }
}

pub(super) fn default_collation() -> Postgres16DefaultCollationCatalogFactV1 {
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

pub(super) fn live_attribute(
    relation_oid: u32,
    attribute_number: i16,
    attribute_name: &str,
    type_oid: u32,
    type_name: &str,
    is_not_null: bool,
) -> Postgres16AttributeCatalogFactV1 {
    let collation_oid = if matches!(type_name, "text" | "varchar" | "bpchar") {
        100
    } else {
        0
    };
    Postgres16AttributeCatalogFactV1 {
        relation_oid,
        attribute_number,
        attribute_name: attribute_name.into(),
        is_dropped: false,
        is_local: true,
        inheritance_count: 0,
        is_not_null,
        attribute_type_oid: type_oid,
        array_dimensions: 0,
        type_modifier: -1,
        collation_oid,
        joined_type: Some(type_fact(type_oid, type_name, collation_oid)),
        joined_default_collation: (collation_oid == 100).then(default_collation),
    }
}

pub(super) fn dropped_attribute(
    relation_oid: u32,
    attribute_number: i16,
) -> Postgres16AttributeCatalogFactV1 {
    Postgres16AttributeCatalogFactV1 {
        relation_oid,
        attribute_number,
        attribute_name: "........pg.dropped.2........".into(),
        is_dropped: true,
        is_local: false,
        inheritance_count: -1,
        is_not_null: true,
        attribute_type_oid: 0,
        array_dimensions: 7,
        type_modifier: 42,
        collation_oid: 999,
        joined_type: None,
        joined_default_collation: None,
    }
}

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

#[test]
fn live_dropped_live_slots_become_dense_columns_and_immutable_coordinates() {
    let normalized = normalize_postgres16_relations_v1(
        vec![relation_fact(42, "orders", 3)],
        vec![
            live_attribute(42, 3, "description", 25, "text", false),
            dropped_attribute(42, 2),
            live_attribute(42, 1, "id", 23, "int4", true),
        ],
    )
    .unwrap();

    assert_eq!(normalized.relations.len(), 1);
    let relation = &normalized.relations[0];
    assert!(relation.name.catalog.is_none());
    assert_eq!(relation.name.schema.as_ref().unwrap().as_str(), "public");
    assert_eq!(relation.name.local.as_str(), "orders");
    assert_eq!(relation.kind, RelationKindV1::BaseTable);
    assert_eq!(relation.columns.len(), 2);
    assert_eq!(relation.columns[0].ordinal.get(), 1);
    assert_eq!(relation.columns[0].name.as_str(), "id");
    assert_eq!(
        relation.columns[0].source_type.family,
        TypeFamilyV1::SignedInteger
    );
    assert_eq!(relation.columns[1].ordinal.get(), 2);
    assert_eq!(relation.columns[1].name.as_str(), "description");
    assert_eq!(
        relation.columns[1].source_type.family,
        TypeFamilyV1::Character
    );

    let coordinates = normalized.coordinates_by_relation_oid.get(&42).unwrap();
    assert_eq!(coordinates.relation_index, 0);
    let Postgres16AttributeCoordinateV1::Live(id) = &coordinates.attributes_by_number[&1] else {
        panic!("attnum 1 must be live");
    };
    assert_eq!(id.column_index, 0);
    assert_eq!(id.type_oid, 23);
    assert_eq!(id.collation_oid, 0);
    assert!(id.is_not_null);
    assert!(matches!(
        coordinates.attributes_by_number[&2],
        Postgres16AttributeCoordinateV1::Dropped
    ));
    let Postgres16AttributeCoordinateV1::Live(description) = &coordinates.attributes_by_number[&3]
    else {
        panic!("attnum 3 must be live");
    };
    assert_eq!(description.column_index, 1);
    assert_eq!(description.type_oid, 25);
    assert_eq!(description.collation_oid, 100);
    assert!(!description.is_not_null);
}

#[test]
fn empty_candidate_scope_is_valid_but_empty_relation_is_not() {
    let empty = normalize_postgres16_relations_v1(Vec::new(), Vec::new()).unwrap();
    assert!(empty.relations.is_empty());
    assert!(empty.coordinates_by_relation_oid.is_empty());

    assert_reason(
        vec![relation_fact(42, "empty", 1)],
        vec![dropped_attribute(42, 1)],
        PostgresSchemaIdentityUnavailableV1::UnsupportedRelation,
    );
}

#[test]
fn relation_admission_predicates_are_independently_closed() {
    let base = relation_fact(42, "orders", 1);
    let mut mutations = Vec::new();
    let mut value = base.clone();
    value.relation_oid = 0;
    mutations.push(value);
    let mut value = base.clone();
    value.relation_namespace_oid = 0;
    mutations.push(value);
    let mut value = base.clone();
    value.joined_namespace_oid = 2_201;
    mutations.push(value);
    let mut value = base.clone();
    value.namespace_name = "private".into();
    mutations.push(value);
    for kind in ['p', 'f'] {
        let mut value = base.clone();
        value.relation_kind = kind;
        mutations.push(value);
    }
    for persistence in ['u', 't'] {
        let mut value = base.clone();
        value.persistence = persistence;
        mutations.push(value);
    }
    let mut value = base.clone();
    value.is_shared = true;
    mutations.push(value);
    let mut value = base.clone();
    value.is_partition = true;
    mutations.push(value);
    let mut value = base.clone();
    value.row_security = true;
    mutations.push(value);
    let mut value = base.clone();
    value.force_row_security = true;
    mutations.push(value);
    let mut value = base.clone();
    value.of_type_oid = 23;
    mutations.push(value);
    let mut value = base.clone();
    value.rewrite_oid = 99;
    mutations.push(value);
    let mut value = base.clone();
    value.access_method_oid = 0;
    mutations.push(value);
    let mut value = base.clone();
    value.joined_access_method_oid = None;
    mutations.push(value);
    let mut value = base.clone();
    value.joined_access_method_oid = Some(3);
    mutations.push(value);
    let mut value = base.clone();
    value.access_method_name = None;
    mutations.push(value);
    let mut value = base.clone();
    value.access_method_name = Some("ao_row".into());
    mutations.push(value);
    let mut value = base.clone();
    value.access_method_type = None;
    mutations.push(value);
    let mut value = base.clone();
    value.access_method_type = Some('i');
    mutations.push(value);
    let mut value = base.clone();
    value.inherits_as_child = true;
    mutations.push(value);
    let mut value = base.clone();
    value.inherits_as_parent = true;
    mutations.push(value);
    let mut value = base;
    value.physical_attribute_count = -1;
    mutations.push(value);

    for mut candidate in mutations {
        let relation_oid = candidate.relation_oid;
        let attributes = vec![live_attribute(relation_oid, 1, "id", 23, "int4", true)];
        candidate.physical_attribute_count = candidate.physical_attribute_count.min(1);
        assert_reason(
            vec![candidate],
            attributes,
            PostgresSchemaIdentityUnavailableV1::UnsupportedRelation,
        );
    }
}

#[test]
fn closed_public_base_table_profile_rejects_censused_views() {
    for kind in ['v', 'm'] {
        let mut view = relation_fact(42, "visible_but_unsupported", 1);
        view.relation_kind = kind;
        assert_reason(
            vec![view],
            Vec::new(),
            PostgresSchemaIdentityUnavailableV1::UnsupportedRelation,
        );
    }
}

#[test]
fn relation_names_and_output_order_are_exact_not_oid_ordered() {
    let normalized = normalize_postgres16_relations_v1(
        vec![relation_fact(90, "zeta", 1), relation_fact(7, "Älpha", 1)],
        vec![
            live_attribute(90, 1, "CaseSensitive", 23, "int4", false),
            live_attribute(7, 1, "é", 23, "int4", false),
        ],
    )
    .unwrap();
    let names: Vec<_> = normalized
        .relations
        .iter()
        .map(|relation| relation.name.local.as_str())
        .collect();
    assert_eq!(names, ["zeta", "Älpha"]);
    assert_eq!(
        normalized.relations[0].columns[0].name.as_str(),
        "CaseSensitive"
    );
    assert_eq!(normalized.relations[1].columns[0].name.as_str(), "é");
}

#[test]
fn shuffled_rows_coherent_handle_changes_and_dropped_names_do_not_change_relations() {
    let baseline = normalize_postgres16_relations_v1(
        vec![relation_fact(90, "zeta", 2), relation_fact(7, "alpha", 1)],
        vec![
            live_attribute(90, 1, "id", 23, "int4", true),
            dropped_attribute(90, 2),
            live_attribute(7, 1, "body", 25, "text", false),
        ],
    )
    .unwrap();

    let mut zeta = relation_fact(900, "zeta", 2);
    zeta.relation_namespace_oid = 8_000;
    zeta.joined_namespace_oid = 8_000;
    zeta.access_method_oid = 700;
    zeta.joined_access_method_oid = Some(700);
    let mut alpha = relation_fact(70, "alpha", 1);
    alpha.relation_namespace_oid = 8_000;
    alpha.joined_namespace_oid = 8_000;
    alpha.access_method_oid = 700;
    alpha.joined_access_method_oid = Some(700);
    let mut renamed_dropped = dropped_attribute(900, 2);
    renamed_dropped.attribute_name = "hostile-but-ignored".into();
    let changed = normalize_postgres16_relations_v1(
        vec![alpha, zeta],
        vec![
            renamed_dropped,
            live_attribute(70, 1, "body", 25, "text", false),
            live_attribute(900, 1, "id", 23, "int4", true),
        ],
    )
    .unwrap();

    assert_eq!(baseline.relations, changed.relations);
    assert_eq!(
        changed.relation_by_oid(70).unwrap().name.local.as_str(),
        "alpha"
    );
    assert!(changed.attribute_by_number(900, 2).is_some());
    let (column, coordinate) = changed.live_column_by_number(900, 1).unwrap();
    assert_eq!(column.name.as_str(), "id");
    assert_eq!(coordinate.column_index, 0);
    assert!(changed.live_column_by_number(900, 2).is_none());
    assert_eq!(changed.clone().into_relations(), changed.relations);
}
