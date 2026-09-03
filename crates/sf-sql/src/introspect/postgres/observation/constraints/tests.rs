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
    let coordinate = |relation_index, is_not_null| Postgres16RelationCoordinateV1 {
        relation_index,
        attributes_by_number: BTreeMap::from([(
            1,
            Postgres16AttributeCoordinateV1::Live(Postgres16ColumnCoordinateV1 {
                column_index: 0,
                type_oid,
                collation_oid: 0,
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

fn valid_fk(operand_type_oid: u32, operator_oid: u32) -> Postgres16RawConstraintV1 {
    Postgres16RawConstraintV1::ForeignKey(Postgres16RawForeignKeyV1 {
        child_oid: CHILD_RELATION_OID,
        parent_oid: PARENT_RELATION_OID,
        child_attnums: vec![1],
        parent_attnums: vec![1],
        validated: true,
        match_code: 's',
        parent_index: Postgres16RawIndexV1 {
            relation_oid: PARENT_RELATION_OID,
            key_attnums: vec![1],
            unique: true,
            primary: true,
            valid: true,
            ready: true,
            live: true,
            immediate: true,
            btree_default: true,
        },
        equality_operators: vec![Postgres16RawEqualityOperatorV1 {
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
        relation_oid: 1,
        key_attnums: Vec::new(),
        unique: true,
        primary: false,
        valid: true,
        ready: true,
        live: true,
        immediate: true,
        btree_default: true,
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
            key_attnums: vec![1],
            unique: true,
            primary: false,
            valid: true,
            ready: true,
            live: true,
            immediate: true,
            btree_default: true,
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
    let normalized = normalize_postgres16_constraints_v1(&relations, vec![valid_fk(23, 96)])
        .expect("int4 FK equality operands are pg_type OIDs");
    assert!(matches!(
        normalized.as_slice(),
        [ConstraintInputV1::ForeignKey { pairs, .. }] if pairs.len() == 1
    ));
}

#[test]
fn foreign_key_varchar_uses_the_text_default_opclass_operand() {
    let relations = fk_relations(1_043, "varchar", TypeFamilyV1::Character);
    assert!(normalize_postgres16_constraints_v1(&relations, vec![valid_fk(25, 98)]).is_ok());
}

#[test]
fn foreign_key_rejects_relation_or_unapproved_operand_type_oids() {
    let relations = fk_relations(23, "int4", TypeFamilyV1::SignedInteger);
    for operand_type_oid in [CHILD_RELATION_OID, PARENT_RELATION_OID, 20, 25] {
        assert_eq!(
            normalize_postgres16_constraints_v1(&relations, vec![valid_fk(operand_type_oid, 96)],),
            Err(unsupported())
        );
    }
}

#[test]
fn foreign_key_rejects_a_synthesized_varchar_operator_signature() {
    let relations = fk_relations(1_043, "varchar", TypeFamilyV1::Character);
    assert_eq!(
        normalize_postgres16_constraints_v1(&relations, vec![valid_fk(1_043, 1_070)]),
        Err(unsupported())
    );
}
