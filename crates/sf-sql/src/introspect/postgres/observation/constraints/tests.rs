use std::collections::BTreeMap;
use std::num::NonZeroU32;

use sf_core::schema_identity::{
    ColumnInputV1, IdentifierV1, QualifiedNameV1, RelationInputV1, RelationKindV1, SourceTypeV1,
    TypeFamilyV1,
};

use super::super::relation::{
    Postgres16AttributeCoordinateV1, Postgres16ColumnCoordinateV1, Postgres16RelationCoordinateV1,
};
use super::*;

const CHILD_RELATION_OID: u32 = 10_001;
const PARENT_RELATION_OID: u32 = 20_002;
const PARENT_INDEX_OID: u32 = 30_003;
const INT4_OPCLASS_OID: u32 = 1_978;
const TEXT_OPCLASS_OID: u32 = 3_126;

fn empty_relations() -> Postgres16NormalizedRelationsV1 {
    Postgres16NormalizedRelationsV1 {
        relations: Vec::new(),
        coordinates_by_relation_oid: BTreeMap::new(),
    }
}

fn identifier(value: &str) -> IdentifierV1 {
    IdentifierV1::new(value).unwrap()
}

fn relation(name: &str, column: &str, type_name: &str, family: TypeFamilyV1) -> RelationInputV1 {
    RelationInputV1 {
        name: QualifiedNameV1 {
            catalog: None,
            schema: Some(identifier("public")),
            local: identifier(name),
        },
        kind: RelationKindV1::BaseTable,
        columns: vec![ColumnInputV1 {
            ordinal: NonZeroU32::new(1).unwrap(),
            name: identifier(column),
            source_type: SourceTypeV1 {
                native_name: QualifiedNameV1 {
                    catalog: None,
                    schema: Some(identifier("pg_catalog")),
                    local: identifier(type_name),
                },
                family,
                facets: Vec::new(),
            },
        }],
    }
}

fn fk_relations(
    type_oid: u32,
    type_name: &str,
    family: TypeFamilyV1,
) -> Postgres16NormalizedRelationsV1 {
    let collation_oid = if type_oid == PG_VARCHAR_TYPE_OID {
        100
    } else {
        0
    };
    let coordinate = |relation_index, is_not_null| Postgres16RelationCoordinateV1 {
        relation_index,
        attributes_by_number: BTreeMap::from([(
            1,
            Postgres16AttributeCoordinateV1::Live(Postgres16ColumnCoordinateV1 {
                column_index: 0,
                type_oid,
                collation_oid,
                is_not_null,
            }),
        )]),
    };
    Postgres16NormalizedRelationsV1 {
        relations: vec![
            relation("child", "parent_id", type_name, family),
            relation("parent", "id", type_name, family),
        ],
        coordinates_by_relation_oid: BTreeMap::from([
            (CHILD_RELATION_OID, coordinate(0, false)),
            (PARENT_RELATION_OID, coordinate(1, true)),
        ]),
    }
}

fn add_parent_include_column(relations: &mut Postgres16NormalizedRelationsV1) {
    let mut column = relations.relations[1].columns[0].clone();
    column.ordinal = NonZeroU32::new(2).unwrap();
    column.name = identifier("payload");
    relations.relations[1].columns.push(column);
    let parent = relations
        .coordinates_by_relation_oid
        .get_mut(&PARENT_RELATION_OID)
        .unwrap();
    parent.attributes_by_number.insert(
        2,
        Postgres16AttributeCoordinateV1::Live(Postgres16ColumnCoordinateV1 {
            column_index: 1,
            type_oid: 23,
            collation_oid: 0,
            is_not_null: false,
        }),
    );
}

fn opclass_input_type_oid(source_type_oid: u32) -> u32 {
    if source_type_oid == PG_VARCHAR_TYPE_OID {
        PG_TEXT_TYPE_OID
    } else {
        source_type_oid
    }
}

fn index_position(source_type_oid: u32) -> Postgres16RawIndexPositionV1 {
    let operand_type_oid = opclass_input_type_oid(source_type_oid);
    Postgres16RawIndexPositionV1 {
        attnum: 1,
        opclass_oid: if operand_type_oid == PG_TEXT_TYPE_OID {
            TEXT_OPCLASS_OID
        } else {
            INT4_OPCLASS_OID
        },
        opclass_input_type_oid: operand_type_oid,
        collation_oid: if operand_type_oid == PG_TEXT_TYPE_OID {
            100
        } else {
            0
        },
        default_btree: true,
    }
}

fn raw_index(source_type_oid: u32) -> Postgres16RawIndexV1 {
    Postgres16RawIndexV1 {
        selected_oid: PARENT_INDEX_OID,
        observed_oid: PARENT_INDEX_OID,
        relation_oid: PARENT_RELATION_OID,
        key_count: 1,
        total_attribute_count: 1,
        all_attnums: vec![1],
        key_positions: vec![index_position(source_type_oid)],
        unique: true,
        primary: true,
        valid: true,
        ready: true,
        live: true,
        immediate: true,
        access_method_exact: true,
        expressions_absent: true,
        predicate_absent: true,
    }
}

fn valid_fk(
    source_type_oid: u32,
    operand_type_oid: u32,
    operator_oid: u32,
) -> Postgres16RawConstraintV1 {
    Postgres16RawConstraintV1::ForeignKey(Postgres16RawForeignKeyV1 {
        child_oid: CHILD_RELATION_OID,
        parent_oid: PARENT_RELATION_OID,
        child_attnums: vec![1],
        parent_attnums: vec![1],
        validated: true,
        match_code: 's',
        parent_index: raw_index(source_type_oid),
        equality_operators: vec![Postgres16RawEqualityOperatorV1 {
            opclass_oid: if source_type_oid == PG_VARCHAR_TYPE_OID {
                TEXT_OPCLASS_OID
            } else {
                INT4_OPCLASS_OID
            },
            parent_operand_type_oid: operand_type_oid,
            child_operand_type_oid: operand_type_oid,
            selected_oid: operator_oid,
            search_oid: operator_oid,
            is_catalog_equals_bool: true,
        }],
        triggers: Postgres16RawForeignKeyTriggersV1 {
            child_insert_ok: true,
            child_update_ok: true,
            parent_delete_ok: true,
            parent_update_ok: true,
            all_enabled: true,
        },
        types_and_facets_equal: true,
        operators_exact: true,
        triggers_exact: true,
        actions_supported: true,
    })
}

#[test]
fn empty_input_is_a_complete_empty_constraint_set() {
    assert_eq!(
        normalize_postgres16_constraints_v1(&empty_relations(), Vec::new()),
        Ok(Vec::new())
    );
}

#[test]
fn unknown_relation_and_column_references_fail_closed() {
    let relations = empty_relations();
    let raw = Postgres16RawConstraintV1::NotNull {
        relation_oid: 99,
        attnum: 1,
        validated: true,
    };
    assert_eq!(
        normalize_postgres16_constraints_v1(&relations, vec![raw]),
        Err(unsupported())
    );
}

#[test]
fn empty_and_oversized_key_members_are_rejected_before_lookup() {
    let relations = empty_relations();
    let index = Postgres16RawIndexV1 {
        selected_oid: 1,
        observed_oid: 1,
        relation_oid: 1,
        key_count: 0,
        total_attribute_count: 0,
        all_attnums: Vec::new(),
        key_positions: Vec::new(),
        unique: true,
        primary: false,
        valid: true,
        ready: true,
        live: true,
        immediate: true,
        access_method_exact: true,
        expressions_absent: true,
        predicate_absent: true,
    };
    let key = Postgres16RawConstraintV1::Unique(Postgres16RawUniqueV1 {
        key: Postgres16RawKeyV1 {
            relation_oid: 1,
            attnums: Vec::new(),
            validated: true,
            enforced: true,
            index,
        },
        nulls_not_distinct: false,
    });
    assert_eq!(
        normalize_postgres16_constraints_v1(&relations, vec![key]),
        Err(unsupported())
    );
}

#[test]
fn constraint_collection_cap_is_fail_closed() {
    let relations = empty_relations();
    let raw = std::iter::repeat_n(
        Postgres16RawConstraintV1::NotNull {
            relation_oid: 1,
            attnum: 1,
            validated: true,
        },
        MAX_CONSTRAINTS + 1,
    )
    .collect();
    assert_eq!(
        normalize_postgres16_constraints_v1(&relations, raw),
        Err(PostgresSchemaIdentityUnavailableV1::LimitExceeded(
            PostgresSchemaIdentityLimitCodeV1::RawConstraints,
        ),)
    );
}

#[test]
fn foreign_key_partial_match_and_arity_mismatch_are_rejected() {
    let relations = empty_relations();
    let fk = Postgres16RawConstraintV1::ForeignKey(Postgres16RawForeignKeyV1 {
        child_oid: 1,
        parent_oid: 2,
        child_attnums: vec![1],
        parent_attnums: vec![1, 2],
        validated: true,
        match_code: 'p',
        parent_index: Postgres16RawIndexV1 {
            relation_oid: 2,
            primary: false,
            ..raw_index(23)
        },
        equality_operators: Vec::new(),
        triggers: Postgres16RawForeignKeyTriggersV1 {
            child_insert_ok: true,
            child_update_ok: true,
            parent_delete_ok: true,
            parent_update_ok: true,
            all_enabled: true,
        },
        types_and_facets_equal: true,
        operators_exact: true,
        triggers_exact: true,
        actions_supported: true,
    });
    assert_eq!(
        normalize_postgres16_constraints_v1(&relations, vec![fk]),
        Err(unsupported())
    );
}

#[test]
fn foreign_key_operator_operand_type_oids_are_not_relation_oids() {
    let relations = fk_relations(23, "int4", TypeFamilyV1::SignedInteger);
    let normalized = normalize_postgres16_constraints_v1(&relations, vec![valid_fk(23, 23, 96)])
        .expect("int4 FK equality operands are pg_type OIDs");
    assert!(matches!(
        normalized.as_slice(),
        [ConstraintInputV1::ForeignKey { pairs, .. }] if pairs.len() == 1
    ));
}

#[test]
fn foreign_key_varchar_uses_the_text_default_opclass_operand() {
    let relations = fk_relations(1_043, "varchar", TypeFamilyV1::Character);
    assert!(normalize_postgres16_constraints_v1(&relations, vec![valid_fk(1_043, 25, 98)]).is_ok());
}

#[test]
fn foreign_key_disabled_trigger_state_is_structurally_valid_but_not_enforced() {
    let relations = fk_relations(23, "int4", TypeFamilyV1::SignedInteger);
    let Postgres16RawConstraintV1::ForeignKey(mut fk) = valid_fk(23, 23, 96) else {
        unreachable!("fixture is an FK");
    };
    fk.triggers.all_enabled = false;
    let normalized = normalize_postgres16_constraints_v1(
        &relations,
        vec![Postgres16RawConstraintV1::ForeignKey(fk)],
    )
    .expect("disabled D/R trigger state remains structurally observable");
    assert!(matches!(
        normalized.as_slice(),
        [ConstraintInputV1::ForeignKey { state, .. }] if state.validated && !state.enforced
    ));
}

#[test]
fn foreign_key_allows_a_resolved_include_tail_after_the_exact_key_prefix() {
    let mut relations = fk_relations(23, "int4", TypeFamilyV1::SignedInteger);
    add_parent_include_column(&mut relations);
    let Postgres16RawConstraintV1::ForeignKey(mut fk) = valid_fk(23, 23, 96) else {
        unreachable!("fixture is an FK");
    };
    fk.parent_index.total_attribute_count = 2;
    fk.parent_index.all_attnums.push(2);

    assert!(normalize_postgres16_constraints_v1(
        &relations,
        vec![Postgres16RawConstraintV1::ForeignKey(fk)]
    )
    .is_ok());
}

#[test]
fn foreign_key_rejects_unbound_parent_index_identity_and_shape_evidence() {
    let relations = fk_relations(23, "int4", TypeFamilyV1::SignedInteger);
    let Postgres16RawConstraintV1::ForeignKey(base) = valid_fk(23, 23, 96) else {
        unreachable!("fixture is an FK");
    };
    let mut invalid = Vec::new();

    let mut value = base.clone();
    value.parent_index.selected_oid = 0;
    invalid.push(value);
    let mut value = base.clone();
    value.parent_index.observed_oid += 1;
    invalid.push(value);
    let mut value = base.clone();
    value.parent_index.relation_oid = CHILD_RELATION_OID;
    invalid.push(value);
    let mut value = base.clone();
    value.parent_index.key_count = 2;
    invalid.push(value);
    let mut value = base.clone();
    value.parent_index.total_attribute_count = 0;
    invalid.push(value);
    let mut value = base.clone();
    value.parent_index.total_attribute_count = 33;
    invalid.push(value);
    let mut value = base.clone();
    value.parent_index.all_attnums.push(2);
    invalid.push(value);
    let mut value = base.clone();
    value.parent_index.all_attnums[0] = 2;
    invalid.push(value);
    let mut value = base.clone();
    value.parent_index.access_method_exact = false;
    invalid.push(value);
    let mut value = base.clone();
    value.parent_index.expressions_absent = false;
    invalid.push(value);
    let mut value = base;
    value.parent_index.predicate_absent = false;
    invalid.push(value);

    assert!(invalid.into_iter().all(|fk| {
        normalize_postgres16_constraints_v1(
            &relations,
            vec![Postgres16RawConstraintV1::ForeignKey(fk)],
        ) == Err(unsupported())
    }));
}

#[test]
fn foreign_key_rejects_unbound_parent_index_position_evidence() {
    let relations = fk_relations(23, "int4", TypeFamilyV1::SignedInteger);
    let Postgres16RawConstraintV1::ForeignKey(base) = valid_fk(23, 23, 96) else {
        unreachable!("fixture is an FK");
    };
    let mut invalid = Vec::new();

    let mut value = base.clone();
    value.parent_index.key_positions.clear();
    invalid.push(value);
    let mut value = base.clone();
    value.parent_index.key_positions[0].attnum = 2;
    invalid.push(value);
    let mut value = base.clone();
    value.parent_index.key_positions[0].opclass_oid = 0;
    invalid.push(value);
    let mut value = base.clone();
    value.parent_index.key_positions[0].opclass_input_type_oid = 25;
    invalid.push(value);
    let mut value = base.clone();
    value.parent_index.key_positions[0].collation_oid = 100;
    invalid.push(value);
    let mut value = base.clone();
    value.parent_index.key_positions[0].default_btree = false;
    invalid.push(value);
    let mut value = base;
    value.equality_operators[0].opclass_oid += 1;
    invalid.push(value);

    assert!(invalid.into_iter().all(|fk| {
        normalize_postgres16_constraints_v1(
            &relations,
            vec![Postgres16RawConstraintV1::ForeignKey(fk)],
        ) == Err(unsupported())
    }));
}

#[test]
fn foreign_key_rejects_unresolved_or_expression_like_include_tail() {
    let relations = fk_relations(23, "int4", TypeFamilyV1::SignedInteger);
    for include_attnum in [0, 2, 1] {
        let Postgres16RawConstraintV1::ForeignKey(mut fk) = valid_fk(23, 23, 96) else {
            unreachable!("fixture is an FK");
        };
        fk.parent_index.total_attribute_count = 2;
        fk.parent_index.all_attnums.push(include_attnum);
        assert_eq!(
            normalize_postgres16_constraints_v1(
                &relations,
                vec![Postgres16RawConstraintV1::ForeignKey(fk)]
            ),
            Err(unsupported())
        );
    }
}

#[test]
fn foreign_key_rejects_relation_or_unapproved_operand_type_oids() {
    let relations = fk_relations(23, "int4", TypeFamilyV1::SignedInteger);
    for operand_type_oid in [CHILD_RELATION_OID, PARENT_RELATION_OID, 20, 25] {
        assert_eq!(
            normalize_postgres16_constraints_v1(
                &relations,
                vec![valid_fk(23, operand_type_oid, 96)],
            ),
            Err(unsupported())
        );
    }
}

#[test]
fn foreign_key_rejects_a_synthesized_varchar_operator_signature() {
    let relations = fk_relations(1_043, "varchar", TypeFamilyV1::Character);
    assert_eq!(
        normalize_postgres16_constraints_v1(&relations, vec![valid_fk(1_043, 1_043, 1_070)]),
        Err(unsupported())
    );
}
