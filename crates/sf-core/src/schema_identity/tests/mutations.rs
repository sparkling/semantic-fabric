use super::*;
use crate::schema_identity::encode::{
    facet_value_tag, foreign_match_tag, type_family_tag, unique_null_tag,
};

fn identity(input: SchemaObservationInputV1) -> ObservedSchemaIdentityV1 {
    ObservedSchemaIdentityV1::build(input).expect("valid mutation fixture")
}

fn constraint_only(value: ConstraintInputV1) -> SchemaObservationInputV1 {
    let mut input = non_empty_input();
    input.constraints = vec![value];
    input
}

fn staff_pk(columns: Vec<ColumnKeyV1>) -> ConstraintInputV1 {
    ConstraintInputV1::PrimaryKey {
        relation: relation_ref("staff"),
        state: ConstraintStateV1 {
            validated: true,
            enforced: true,
        },
        columns,
    }
}

fn staff_unique(columns: Vec<ColumnKeyV1>) -> ConstraintInputV1 {
    ConstraintInputV1::UniqueKey {
        relation: relation_ref("staff"),
        state: ConstraintStateV1 {
            validated: true,
            enforced: true,
        },
        nulls: UniqueNullSemanticsV1::NullsDistinct,
        columns,
    }
}

fn two_by_two_input() -> SchemaObservationInputV1 {
    let mut input = non_empty_input();
    input.relations[1].columns.push(column(2, "alternate_id"));
    input.constraints.clear();
    input
}

fn invalid_code(input: SchemaObservationInputV1) -> SchemaIdentityInvalidCodeV1 {
    match ObservedSchemaIdentityV1::build(input).expect_err("fixture must reject") {
        SchemaIdentityErrorV1::Invalid { code, .. } => code,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

fn find(bytes: &[u8], needle: &[u8]) -> usize {
    bytes
        .windows(needle.len())
        .position(|window| window == needle)
        .expect("literal encoding must occur")
}

#[test]
fn unordered_inputs_are_canonical() {
    let original = identity(non_empty_input());
    let mut permuted = non_empty_input();
    permuted.relations[0].columns.reverse();
    permuted.relations.reverse();
    permuted.constraints.reverse();
    assert_eq!(identity(permuted), original);
}

#[test]
fn exact_duplicate_constraints_collapse_after_normalization() {
    let one = constraint_only(staff_unique(vec![
        column_key(1, "id"),
        column_key(2, "dept_id"),
    ]));
    let mut duplicates = one.clone();
    duplicates.constraints.push(staff_unique(vec![
        column_key(2, "dept_id"),
        column_key(1, "id"),
    ]));
    assert_eq!(identity(one), identity(duplicates));

    let primary = constraint_only(staff_pk(vec![column_key(1, "id")]));
    let mut duplicate_primary = primary.clone();
    duplicate_primary
        .constraints
        .push(staff_pk(vec![column_key(1, "id")]));
    assert_eq!(identity(primary), identity(duplicate_primary));
}

#[test]
fn unique_order_is_irrelevant_but_primary_key_order_is_semantic() {
    let unique_a = constraint_only(staff_unique(vec![
        column_key(1, "id"),
        column_key(2, "dept_id"),
    ]));
    let unique_b = constraint_only(staff_unique(vec![
        column_key(2, "dept_id"),
        column_key(1, "id"),
    ]));
    assert_eq!(identity(unique_a), identity(unique_b));

    let pk_a = constraint_only(staff_pk(vec![
        column_key(1, "id"),
        column_key(2, "dept_id"),
    ]));
    let pk_b = constraint_only(staff_pk(vec![
        column_key(2, "dept_id"),
        column_key(1, "id"),
    ]));
    assert_ne!(identity(pk_a).constraints(), identity(pk_b).constraints());
}

#[test]
fn foreign_key_pair_order_is_semantic() {
    let pairs = vec![
        (column_key(1, "id"), column_key(1, "id")),
        (column_key(2, "dept_id"), column_key(2, "alternate_id")),
    ];
    let make = |pairs| ConstraintInputV1::ForeignKey {
        child: relation_ref("staff"),
        parent: relation_ref("dept"),
        state: ConstraintStateV1 {
            validated: true,
            enforced: true,
        },
        match_kind: ForeignKeyMatchV1::Simple,
        pairs,
    };
    let mut left = two_by_two_input();
    left.constraints = vec![make(pairs.clone())];
    let mut right = two_by_two_input();
    right.constraints = vec![make(pairs.into_iter().rev().collect())];
    assert_ne!(identity(left).constraints(), identity(right).constraints());
}

#[test]
fn facet_records_sort_by_raw_key_and_list_order_remains_semantic() {
    let mut left = non_empty_input();
    left.relations[0].columns[0].source_type.facets = vec![
        TypeFacetV1 {
            key: token("b"),
            value: TypeFacetValueV1::U64(1),
        },
        TypeFacetV1 {
            key: token("aa"),
            value: TypeFacetValueV1::TextList(vec![
                TextValueV1::new("first").unwrap(),
                TextValueV1::new("second").unwrap(),
            ]),
        },
    ];
    let mut records_reversed = left.clone();
    records_reversed.relations[0].columns[0]
        .source_type
        .facets
        .reverse();
    assert_eq!(identity(left.clone()), identity(records_reversed));

    let (_, preimages) = build_with_preimages(left.clone()).unwrap();
    assert!(
        find(&preimages.types, &[0x22, 0, 0, 0, 2, b'a', b'a'])
            < find(&preimages.types, &[0x22, 0, 0, 0, 1, b'b'])
    );
    let ordered_list_digest = identity(left.clone()).types();
    if let TypeFacetValueV1::TextList(values) =
        &mut left.relations[0].columns[0].source_type.facets[1].value
    {
        values.reverse();
    }
    assert_ne!(identity(left).types(), ordered_list_digest);

    let mut names = non_empty_input();
    names.relations[0].columns[0].source_type.facets = vec![TypeFacetV1 {
        key: token("names"),
        value: TypeFacetValueV1::TypeNameList(vec![
            qname(Some("std"), "first"),
            qname(Some("std"), "second"),
        ]),
    }];
    let ordered_names = identity(names.clone()).types();
    if let TypeFacetValueV1::TypeNameList(values) =
        &mut names.relations[0].columns[0].source_type.facets[0].value
    {
        values.reverse();
    }
    assert_ne!(identity(names).types(), ordered_names);
}

#[test]
fn profiles_and_facts_affect_only_their_digest_domain() {
    let baseline = identity(non_empty_input());
    let mut type_changed = non_empty_input();
    type_changed.profiles.types = profile("kat-type-v2");
    let type_changed = identity(type_changed);
    assert_eq!(baseline.structural(), type_changed.structural());
    assert_ne!(baseline.types(), type_changed.types());
    assert_eq!(baseline.constraints(), type_changed.constraints());

    let mut constraint_changed = non_empty_input();
    if let ConstraintInputV1::NotNull { state, .. } = &mut constraint_changed.constraints[3] {
        state.validated = false;
    }
    let constraint_changed = identity(constraint_changed);
    assert_eq!(baseline.structural(), constraint_changed.structural());
    assert_eq!(baseline.types(), constraint_changed.types());
    assert_ne!(baseline.constraints(), constraint_changed.constraints());
}

#[test]
fn every_closed_enum_tag_is_exact() {
    let families = [
        TypeFamilyV1::Boolean,
        TypeFamilyV1::SignedInteger,
        TypeFamilyV1::UnsignedInteger,
        TypeFamilyV1::ExactNumeric,
        TypeFamilyV1::ApproximateNumeric,
        TypeFamilyV1::Character,
        TypeFamilyV1::Binary,
        TypeFamilyV1::Date,
        TypeFamilyV1::Time,
        TypeFamilyV1::Timestamp,
        TypeFamilyV1::Interval,
        TypeFamilyV1::Json,
        TypeFamilyV1::Uuid,
        TypeFamilyV1::Enum,
        TypeFamilyV1::Array,
        TypeFamilyV1::Domain,
        TypeFamilyV1::DynamicPerValue,
        TypeFamilyV1::Opaque,
    ];
    let tags = families.map(type_family_tag);
    assert_eq!(
        tags,
        [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10, 0x11, 0x7f,
        ]
    );
    assert_eq!(
        [
            unique_null_tag(UniqueNullSemanticsV1::NullsDistinct),
            unique_null_tag(UniqueNullSemanticsV1::NullsNotDistinct),
            unique_null_tag(UniqueNullSemanticsV1::ObservedUnknown),
        ],
        [1, 2, 3]
    );
    assert_eq!(
        [
            foreign_match_tag(ForeignKeyMatchV1::Simple),
            foreign_match_tag(ForeignKeyMatchV1::Full),
            foreign_match_tag(ForeignKeyMatchV1::Partial),
            foreign_match_tag(ForeignKeyMatchV1::ObservedUnknown),
        ],
        [1, 2, 3, 4]
    );

    let values = [
        TypeFacetValueV1::Bool(false),
        TypeFacetValueV1::U64(0x0102_0304_0506_0708),
        TypeFacetValueV1::I64(-2),
        TypeFacetValueV1::Text(TextValueV1::new("").unwrap()),
        TypeFacetValueV1::TypeName(QualifiedNameV1 {
            catalog: Some(identifier("catalog")),
            schema: Some(identifier("schema")),
            local: identifier("type"),
        }),
        TypeFacetValueV1::TextList(Vec::new()),
        TypeFacetValueV1::TypeNameList(Vec::new()),
        TypeFacetValueV1::Digest32([0; 32]),
    ];
    assert_eq!(
        values.each_ref().map(facet_value_tag),
        [1, 2, 3, 4, 5, 6, 7, 8]
    );
}

#[test]
fn boolean_order_and_signed_big_endian_bytes_are_literal() {
    for (validated, enforced, expected) in [
        (false, false, [0, 0]),
        (false, true, [0, 1]),
        (true, false, [1, 0]),
        (true, true, [1, 1]),
    ] {
        let input = constraint_only(ConstraintInputV1::NotNull {
            column: ColumnRefV1 {
                relation: relation_ref("staff"),
                column: column_key(1, "id"),
            },
            state: ConstraintStateV1 {
                validated,
                enforced,
            },
        });
        let (_, preimages) = build_with_preimages(input).unwrap();
        assert!(preimages.constraints.ends_with(&expected));
    }

    let mut input = non_empty_input();
    input.relations[0].columns[0].source_type.facets = vec![TypeFacetV1 {
        key: token("signed"),
        value: TypeFacetValueV1::I64(-2),
    }];
    let (_, preimages) = build_with_preimages(input).unwrap();
    assert!(preimages
        .types
        .windows(9)
        .any(|bytes| bytes == [3, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe]));
}

#[test]
fn exact_reference_and_duplicate_failures_are_typed() {
    let mut duplicate_relation = non_empty_input();
    let mut same_name_different_body = duplicate_relation.relations[0].clone();
    same_name_different_body.columns[0].name = identifier("renamed");
    duplicate_relation.relations.push(same_name_different_body);
    assert_eq!(
        invalid_code(duplicate_relation),
        SchemaIdentityInvalidCodeV1::DuplicateRelation
    );

    let mut duplicate_name = non_empty_input();
    duplicate_name.relations[0].columns[1].name = identifier("id");
    assert_eq!(
        invalid_code(duplicate_name),
        SchemaIdentityInvalidCodeV1::DuplicateColumnName
    );

    let mut duplicate_ordinal = non_empty_input();
    duplicate_ordinal.relations[0].columns[1].ordinal = NonZeroU32::new(1).unwrap();
    assert_eq!(
        invalid_code(duplicate_ordinal),
        SchemaIdentityInvalidCodeV1::DuplicateColumnOrdinal
    );

    let mut duplicate_facet = non_empty_input();
    let mut facet = duplicate_facet.relations[0].columns[0].source_type.facets[0].clone();
    facet.value = TypeFacetValueV1::Bool(false);
    duplicate_facet.relations[0].columns[0]
        .source_type
        .facets
        .push(facet);
    assert_eq!(
        invalid_code(duplicate_facet),
        SchemaIdentityInvalidCodeV1::DuplicateFacetKey
    );

    let mut wrong_name = constraint_only(staff_pk(vec![column_key(1, "wrong")]));
    assert_eq!(
        invalid_code(wrong_name.clone()),
        SchemaIdentityInvalidCodeV1::DanglingColumn
    );
    if let ConstraintInputV1::PrimaryKey { columns, .. } = &mut wrong_name.constraints[0] {
        columns[0] = column_key(2, "id");
    }
    assert_eq!(
        invalid_code(wrong_name),
        SchemaIdentityInvalidCodeV1::DanglingColumn
    );

    for mismatch in [
        qname(Some("app"), "missing"),
        qname(None, "staff"),
        catalogued_qname_for_reference("catalog", "app", "staff"),
    ] {
        let mut input = constraint_only(staff_pk(vec![column_key(1, "id")]));
        if let ConstraintInputV1::PrimaryKey { relation, .. } = &mut input.constraints[0] {
            relation.name = mismatch;
        }
        assert_eq!(
            invalid_code(input),
            SchemaIdentityInvalidCodeV1::DanglingRelation
        );
    }
}

fn catalogued_qname_for_reference(catalog: &str, schema: &str, local: &str) -> QualifiedNameV1 {
    QualifiedNameV1 {
        catalog: Some(identifier(catalog)),
        schema: Some(identifier(schema)),
        local: identifier(local),
    }
}

#[test]
fn empty_keys_multiple_primary_keys_and_fk_repetitions_reject() {
    assert_eq!(
        invalid_code(constraint_only(staff_pk(Vec::new()))),
        SchemaIdentityInvalidCodeV1::EmptyKey
    );
    for duplicate in [
        staff_pk(vec![column_key(1, "id"), column_key(1, "id")]),
        staff_unique(vec![column_key(2, "dept_id"), column_key(2, "dept_id")]),
    ] {
        assert_eq!(
            invalid_code(constraint_only(duplicate)),
            SchemaIdentityInvalidCodeV1::DuplicateKeyMember
        );
    }
    let mut multiple = non_empty_input();
    multiple.constraints = vec![
        staff_pk(vec![column_key(1, "id")]),
        staff_pk(vec![column_key(2, "dept_id")]),
    ];
    assert_eq!(
        invalid_code(multiple),
        SchemaIdentityInvalidCodeV1::MultiplePrimaryKey
    );

    let fk = |pairs| ConstraintInputV1::ForeignKey {
        child: relation_ref("staff"),
        parent: relation_ref("dept"),
        state: ConstraintStateV1 {
            validated: true,
            enforced: true,
        },
        match_kind: ForeignKeyMatchV1::Simple,
        pairs,
    };
    assert_eq!(
        invalid_code(constraint_only(fk(Vec::new()))),
        SchemaIdentityInvalidCodeV1::EmptyKey
    );
    let pair_a = (column_key(1, "id"), column_key(1, "id"));
    let pair_b = (column_key(2, "dept_id"), column_key(2, "alternate_id"));
    let mut exact = two_by_two_input();
    exact.constraints = vec![fk(vec![pair_a.clone(), pair_a.clone()])];
    assert_eq!(
        invalid_code(exact),
        SchemaIdentityInvalidCodeV1::DuplicateForeignKeyPair
    );
    let mut child = two_by_two_input();
    child.constraints = vec![fk(vec![
        pair_a.clone(),
        (pair_a.0.clone(), pair_b.1.clone()),
    ])];
    assert_eq!(
        invalid_code(child),
        SchemaIdentityInvalidCodeV1::DuplicateForeignKeyChild
    );
    let mut parent = two_by_two_input();
    parent.constraints = vec![fk(vec![pair_a.clone(), (pair_b.0, pair_a.1)])];
    assert_eq!(
        invalid_code(parent),
        SchemaIdentityInvalidCodeV1::DuplicateForeignKeyParent
    );
}

#[test]
fn exact_unicode_bytes_are_not_normalized() {
    let mut composed = non_empty_input();
    composed.relations[0].name.local = identifier("café");
    composed.constraints.clear();
    let mut decomposed = composed.clone();
    decomposed.relations[0].name.local = identifier("café");
    assert_ne!(
        identity(composed).structural(),
        identity(decomposed).structural()
    );
}

#[test]
fn self_referencing_foreign_key_is_representable() {
    let mut input = non_empty_input();
    input.constraints = vec![ConstraintInputV1::ForeignKey {
        child: relation_ref("staff"),
        parent: relation_ref("staff"),
        state: ConstraintStateV1 {
            validated: false,
            enforced: false,
        },
        match_kind: ForeignKeyMatchV1::Full,
        pairs: vec![(column_key(2, "dept_id"), column_key(1, "id"))],
    }];
    identity(input);
}
