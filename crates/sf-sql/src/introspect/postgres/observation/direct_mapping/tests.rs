use std::num::NonZeroU32;

use sf_core::schema_identity::{
    ColumnInputV1, ColumnKeyV1, ColumnRefV1, ConstraintInputV1, ConstraintStateV1,
    ForeignKeyMatchV1, IdentifierV1, QualifiedNameV1, RelationInputV1, RelationKindV1,
    RelationRefV1, SourceTypeV1, TypeFamilyV1, UniqueNullSemanticsV1,
};

use super::*;

fn qualified(schema: &str, local: &str) -> QualifiedNameV1 {
    QualifiedNameV1 {
        catalog: None,
        schema: Some(IdentifierV1::new(schema).unwrap()),
        local: IdentifierV1::new(local).unwrap(),
    }
}

fn column(ordinal: u32, name: &str, native: &str, family: TypeFamilyV1) -> ColumnInputV1 {
    ColumnInputV1 {
        ordinal: NonZeroU32::new(ordinal).unwrap(),
        name: IdentifierV1::new(name).unwrap(),
        source_type: SourceTypeV1 {
            native_name: qualified("pg_catalog", native),
            family,
            facets: Vec::new(),
        },
    }
}

fn relation(name: &str, columns: Vec<ColumnInputV1>) -> RelationInputV1 {
    RelationInputV1 {
        name: qualified("public", name),
        kind: RelationKindV1::BaseTable,
        columns,
    }
}

fn relation_ref(name: &str) -> RelationRefV1 {
    RelationRefV1 {
        name: qualified("public", name),
    }
}

fn key(ordinal: u32, name: &str) -> ColumnKeyV1 {
    ColumnKeyV1 {
        ordinal: NonZeroU32::new(ordinal).unwrap(),
        name: IdentifierV1::new(name).unwrap(),
    }
}

fn strong() -> ConstraintStateV1 {
    ConstraintStateV1 {
        validated: true,
        enforced: true,
    }
}

#[test]
fn projection_uses_rich_columns_types_keys_nullability_and_foreign_keys() {
    let relations = vec![
        relation(
            "child",
            vec![
                column(1, "id", "int8", TypeFamilyV1::SignedInteger),
                column(2, "parent_id", "int8", TypeFamilyV1::SignedInteger),
            ],
        ),
        relation(
            "parent",
            vec![
                column(1, "id", "int8", TypeFamilyV1::SignedInteger),
                column(2, "code", "text", TypeFamilyV1::Character),
            ],
        ),
    ];
    let constraints = vec![
        ConstraintInputV1::NotNull {
            column: ColumnRefV1 {
                relation: relation_ref("parent"),
                column: key(1, "id"),
            },
            state: strong(),
        },
        ConstraintInputV1::PrimaryKey {
            relation: relation_ref("parent"),
            state: strong(),
            columns: vec![key(1, "id")],
        },
        ConstraintInputV1::UniqueKey {
            relation: relation_ref("parent"),
            state: strong(),
            nulls: UniqueNullSemanticsV1::NullsDistinct,
            columns: vec![key(2, "code")],
        },
        ConstraintInputV1::ForeignKey {
            child: relation_ref("child"),
            parent: relation_ref("parent"),
            state: ConstraintStateV1 {
                validated: false,
                enforced: true,
            },
            match_kind: ForeignKeyMatchV1::Simple,
            pairs: vec![(key(2, "parent_id"), key(1, "id"))],
        },
    ];

    let tables = project_direct_mapping_tables_v1(&relations, &constraints).unwrap();
    assert_eq!(tables.len(), 2);
    assert_eq!(tables[0].name, "child");
    assert_eq!(tables[0].columns[0].sql_type, "bigint");
    assert!(!tables[0].columns[0].not_null);
    assert_eq!(tables[0].foreign_keys.len(), 1);
    assert_eq!(tables[0].foreign_keys[0].columns, ["parent_id"]);
    assert_eq!(tables[0].foreign_keys[0].parent_table, "parent");
    assert_eq!(tables[0].foreign_keys[0].parent_columns, ["id"]);
    assert_eq!(tables[1].primary_key, ["id"]);
    assert_eq!(tables[1].unique, [vec!["code".to_owned()]]);
    assert!(tables[1].columns[0].not_null);
    assert!(tables.iter().all(|table| table.row_estimate.is_none()));
    assert!(tables
        .iter()
        .all(|table| table.functional_dependencies.is_empty()));
}

#[test]
fn every_registered_native_type_has_the_information_schema_spelling() {
    use TypeFamilyV1 as Family;
    let cases = [
        ("bool", Family::Boolean, "boolean"),
        ("int2", Family::SignedInteger, "smallint"),
        ("int4", Family::SignedInteger, "integer"),
        ("int8", Family::SignedInteger, "bigint"),
        ("numeric", Family::ExactNumeric, "numeric"),
        ("float4", Family::ApproximateNumeric, "real"),
        ("float8", Family::ApproximateNumeric, "double precision"),
        ("text", Family::Character, "text"),
        ("varchar", Family::Character, "character varying"),
        ("bpchar", Family::Character, "character"),
        ("bytea", Family::Binary, "bytea"),
        ("date", Family::Date, "date"),
        ("time", Family::Time, "time without time zone"),
        ("timetz", Family::Time, "time with time zone"),
        (
            "timestamp",
            Family::Timestamp,
            "timestamp without time zone",
        ),
        ("timestamptz", Family::Timestamp, "timestamp with time zone"),
        ("json", Family::Json, "json"),
        ("jsonb", Family::Json, "jsonb"),
        ("uuid", Family::Uuid, "uuid"),
    ];
    for (native, family, expected) in cases {
        let source = column(1, "value", native, family).source_type;
        assert_eq!(direct_mapping_sql_type(&source), Ok(expected));
    }
}

#[test]
fn projection_rejects_lossy_or_divergent_rich_facts() {
    let relations = vec![relation(
        "items",
        vec![column(1, "id", "int8", TypeFamilyV1::SignedInteger)],
    )];
    let weak_not_null = ConstraintInputV1::NotNull {
        column: ColumnRefV1 {
            relation: relation_ref("items"),
            column: key(1, "id"),
        },
        state: ConstraintStateV1 {
            validated: false,
            enforced: true,
        },
    };
    assert_eq!(
        project_direct_mapping_tables_v1(&relations, &[weak_not_null]),
        Err(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)
    );

    let wrong_ordinal = ConstraintInputV1::PrimaryKey {
        relation: relation_ref("items"),
        state: strong(),
        columns: vec![key(1, "other")],
    };
    assert_eq!(
        project_direct_mapping_tables_v1(&relations, &[wrong_ordinal]),
        Err(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)
    );

    let mut wrong_family = relations.clone();
    wrong_family[0].columns[0].source_type.family = TypeFamilyV1::Character;
    assert_eq!(
        project_direct_mapping_tables_v1(&wrong_family, &[]),
        Err(PostgresSchemaIdentityUnavailableV1::UnsupportedType)
    );
}

#[test]
fn projection_rejects_unknown_constraint_semantics_and_duplicate_primary_keys() {
    let relations = vec![relation(
        "items",
        vec![column(1, "id", "int8", TypeFamilyV1::SignedInteger)],
    )];
    let unknown_unique = ConstraintInputV1::UniqueKey {
        relation: relation_ref("items"),
        state: strong(),
        nulls: UniqueNullSemanticsV1::ObservedUnknown,
        columns: vec![key(1, "id")],
    };
    assert_eq!(
        project_direct_mapping_tables_v1(&relations, &[unknown_unique]),
        Err(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)
    );

    let primary = ConstraintInputV1::PrimaryKey {
        relation: relation_ref("items"),
        state: strong(),
        columns: vec![key(1, "id")],
    };
    assert_eq!(
        project_direct_mapping_tables_v1(&relations, &[primary.clone(), primary]),
        Err(PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint)
    );
}
