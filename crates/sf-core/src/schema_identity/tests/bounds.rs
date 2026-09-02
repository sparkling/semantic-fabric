use super::*;
use crate::schema_identity::encode::{build_with_limits, digest_body};
use crate::schema_identity::validate::submitted_utf8_bytes;
use crate::schema_identity::{check_body_limit, check_limit, checked_body_add, checked_utf8_add};

fn assert_limit(
    error: SchemaIdentityErrorV1,
    kind: SchemaIdentityLimitV1,
    observed: u64,
    maximum: u64,
) {
    assert_eq!(
        error,
        SchemaIdentityErrorV1::LimitExceeded {
            limit: kind,
            observed,
            maximum,
        }
    );
}

#[rustfmt::skip]
fn limited_build(input: SchemaObservationInputV1, configure: impl FnOnce(&mut KernelLimitsV1))
    -> Result<ObservedSchemaIdentityV1, SchemaIdentityErrorV1> {
    let mut limits = KernelLimitsV1::PRODUCTION;
    configure(&mut limits);
    build_with_limits(input, limits)
}

#[rustfmt::skip]
fn assert_limit_kind(error: SchemaIdentityErrorV1, expected: SchemaIdentityLimitV1) {
    assert!(matches!(error, SchemaIdentityErrorV1::LimitExceeded { limit, .. }
        if limit == expected), "expected {expected:?}, got {error:?}");
}

#[test]
fn scalar_limits_accept_the_maximum_and_reject_one_more_byte() {
    assert_eq!(
        ProfileIdV1::new("a".repeat(MAX_PROFILE_TOKEN_BYTES_V1))
            .unwrap()
            .byte_len(),
        MAX_PROFILE_TOKEN_BYTES_V1
    );
    assert_limit(
        ProfileIdV1::new("a".repeat(MAX_PROFILE_TOKEN_BYTES_V1 + 1)).unwrap_err(),
        SchemaIdentityLimitV1::ProfileOrTokenBytes,
        (MAX_PROFILE_TOKEN_BYTES_V1 + 1) as u64,
        MAX_PROFILE_TOKEN_BYTES_V1 as u64,
    );
    assert_eq!(
        TokenV1::new("a".repeat(MAX_PROFILE_TOKEN_BYTES_V1))
            .unwrap()
            .byte_len(),
        MAX_PROFILE_TOKEN_BYTES_V1
    );

    let exact_identifier = "é".repeat(MAX_IDENTIFIER_BYTES_V1 / 2);
    assert_eq!(
        IdentifierV1::new(exact_identifier).unwrap().byte_len(),
        MAX_IDENTIFIER_BYTES_V1
    );
    assert_limit(
        IdentifierV1::new("é".repeat(MAX_IDENTIFIER_BYTES_V1 / 2 + 1)).unwrap_err(),
        SchemaIdentityLimitV1::IdentifierBytes,
        (MAX_IDENTIFIER_BYTES_V1 + 2) as u64,
        MAX_IDENTIFIER_BYTES_V1 as u64,
    );

    assert_eq!(
        TextValueV1::new("x".repeat(MAX_FACET_TEXT_BYTES_V1))
            .unwrap()
            .byte_len(),
        MAX_FACET_TEXT_BYTES_V1
    );
    assert_limit(
        TextValueV1::new("x".repeat(MAX_FACET_TEXT_BYTES_V1 + 1)).unwrap_err(),
        SchemaIdentityLimitV1::FacetTextValueBytes,
        (MAX_FACET_TEXT_BYTES_V1 + 1) as u64,
        MAX_FACET_TEXT_BYTES_V1 as u64,
    );
}

#[rustfmt::skip]
#[test]
fn scalar_lexical_rules_are_exact_and_nul_free() {
    assert!(TextValueV1::new("").is_ok());
    assert!(matches!(
        IdentifierV1::new(""),
        Err(SchemaIdentityErrorV1::Invalid {
            code: SchemaIdentityInvalidCodeV1::Identifier,
            ..
        })
    ));
    for invalid in ["A", "-start", "end-", "has space", "nul\0byte"] {
        assert!(matches!(ProfileIdV1::new(invalid), Err(SchemaIdentityErrorV1::Invalid {
            code: SchemaIdentityInvalidCodeV1::ProfileId, .. })), "{invalid:?}");
        assert!(matches!(TokenV1::new(invalid), Err(SchemaIdentityErrorV1::Invalid {
            code: SchemaIdentityInvalidCodeV1::Token, .. })), "{invalid:?}");
    }
    assert!(ProfileIdV1::new("a.b_c-9").is_ok());
    assert!(matches!(
        TextValueV1::new("nul\0byte"),
        Err(SchemaIdentityErrorV1::Invalid {
            code: SchemaIdentityInvalidCodeV1::TextValue,
            ..
        })
    ));
}

#[test]
fn every_collection_guard_is_inclusive() {
    let cases = [
        (SchemaIdentityLimitV1::Relations, MAX_RELATIONS_V1),
        (
            SchemaIdentityLimitV1::ColumnsPerRelation,
            MAX_COLUMNS_PER_RELATION_V1,
        ),
        (SchemaIdentityLimitV1::ColumnsTotal, MAX_COLUMNS_TOTAL_V1),
        (
            SchemaIdentityLimitV1::RawConstraints,
            MAX_RAW_CONSTRAINTS_V1,
        ),
        (
            SchemaIdentityLimitV1::CanonicalConstraints,
            MAX_CANONICAL_CONSTRAINTS_V1,
        ),
        (SchemaIdentityLimitV1::KeyMembers, MAX_KEY_MEMBERS_V1),
        (
            SchemaIdentityLimitV1::FacetsPerColumn,
            MAX_FACETS_PER_COLUMN_V1,
        ),
        (SchemaIdentityLimitV1::FacetsTotal, MAX_FACETS_TOTAL_V1),
        (
            SchemaIdentityLimitV1::FacetListItems,
            MAX_FACET_LIST_ITEMS_V1,
        ),
        (
            SchemaIdentityLimitV1::FacetListItemsTotal,
            MAX_FACET_LIST_ITEMS_TOTAL_V1,
        ),
    ];
    for (kind, maximum) in cases {
        assert_eq!(check_limit(kind, maximum, maximum), Ok(()));
        assert_limit(
            check_limit(kind, maximum + 1, maximum).unwrap_err(),
            kind,
            (maximum + 1) as u64,
            maximum as u64,
        );
    }
    let production = KernelLimitsV1::PRODUCTION;
    assert_eq!(
        production.canonical_constraints,
        MAX_CANONICAL_CONSTRAINTS_V1
    );
    assert_eq!(production.raw_constraints, MAX_RAW_CONSTRAINTS_V1);
    assert!(production.canonical_constraints <= production.raw_constraints);
}

#[test]
fn public_builder_rejects_large_vectors_before_semantic_traversal() {
    let mut relations = empty_input();
    relations.relations = (0..=MAX_RELATIONS_V1)
        .map(|_| RelationInputV1 {
            name: qname(None, "duplicate-is-not-reached"),
            kind: RelationKindV1::BaseTable,
            columns: Vec::new(),
        })
        .collect();
    assert_limit(
        ObservedSchemaIdentityV1::build(relations).unwrap_err(),
        SchemaIdentityLimitV1::Relations,
        (MAX_RELATIONS_V1 + 1) as u64,
        MAX_RELATIONS_V1 as u64,
    );

    let mut columns = empty_input();
    columns.relations.push(RelationInputV1 {
        name: qname(None, "large"),
        kind: RelationKindV1::BaseTable,
        columns: vec![column(1, "duplicate-is-not-reached"); MAX_COLUMNS_PER_RELATION_V1 + 1],
    });
    assert_limit(
        ObservedSchemaIdentityV1::build(columns).unwrap_err(),
        SchemaIdentityLimitV1::ColumnsPerRelation,
        (MAX_COLUMNS_PER_RELATION_V1 + 1) as u64,
        MAX_COLUMNS_PER_RELATION_V1 as u64,
    );
}

#[test]
fn member_facet_and_list_caps_reject_before_duplicates() {
    let state = ConstraintStateV1 {
        validated: true,
        enforced: true,
    };
    for constraint in [
        ConstraintInputV1::PrimaryKey {
            relation: relation_ref("staff"),
            state,
            columns: vec![column_key(1, "id"); MAX_KEY_MEMBERS_V1 + 1],
        },
        ConstraintInputV1::ForeignKey {
            child: relation_ref("staff"),
            parent: relation_ref("dept"),
            state,
            match_kind: ForeignKeyMatchV1::Simple,
            pairs: vec![(column_key(1, "id"), column_key(1, "id")); MAX_KEY_MEMBERS_V1 + 1],
        },
    ] {
        let mut members = non_empty_input();
        members.constraints = vec![constraint];
        assert_limit(
            ObservedSchemaIdentityV1::build(members).unwrap_err(),
            SchemaIdentityLimitV1::KeyMembers,
            (MAX_KEY_MEMBERS_V1 + 1) as u64,
            MAX_KEY_MEMBERS_V1 as u64,
        );
    }

    let mut facets = non_empty_input();
    facets.relations[0].columns[0].source_type.facets = vec![
        TypeFacetV1 {
            key: token("duplicate"),
            value: TypeFacetValueV1::Bool(false),
        };
        MAX_FACETS_PER_COLUMN_V1 + 1
    ];
    assert_limit(
        ObservedSchemaIdentityV1::build(facets).unwrap_err(),
        SchemaIdentityLimitV1::FacetsPerColumn,
        (MAX_FACETS_PER_COLUMN_V1 + 1) as u64,
        MAX_FACETS_PER_COLUMN_V1 as u64,
    );

    for value in [
        TypeFacetValueV1::TextList(vec![
            TextValueV1::new("").unwrap();
            MAX_FACET_LIST_ITEMS_V1 + 1
        ]),
        TypeFacetValueV1::TypeNameList(vec![qname(None, "t"); MAX_FACET_LIST_ITEMS_V1 + 1]),
    ] {
        let mut list = non_empty_input();
        list.relations[0].columns[0].source_type.facets = vec![TypeFacetV1 {
            key: token("list"),
            value,
        }];
        assert_limit(
            ObservedSchemaIdentityV1::build(list).unwrap_err(),
            SchemaIdentityLimitV1::FacetListItems,
            (MAX_FACET_LIST_ITEMS_V1 + 1) as u64,
            MAX_FACET_LIST_ITEMS_V1 as u64,
        );
    }
}

#[test]
fn utf8_and_body_budget_primitives_are_checked_at_the_boundary() {
    let utf8_maximum = MAX_UTF8_PAYLOAD_BYTES_V1 as u64;
    assert_eq!(checked_utf8_add(utf8_maximum - 1, 1), Ok(utf8_maximum));
    assert_limit(
        checked_utf8_add(utf8_maximum, 1).unwrap_err(),
        SchemaIdentityLimitV1::Utf8PayloadBytes,
        utf8_maximum + 1,
        utf8_maximum,
    );
    assert_eq!(
        checked_utf8_add(u64::MAX, 1).unwrap_err(),
        SchemaIdentityErrorV1::ArithmeticOverflow {
            operation: SchemaIdentityOperationV1::Utf8PayloadAccounting,
        }
    );

    for kind in [
        SchemaIdentityLimitV1::StructuralBodyBytes,
        SchemaIdentityLimitV1::TypeBodyBytes,
        SchemaIdentityLimitV1::ConstraintBodyBytes,
    ] {
        assert_eq!(
            check_body_limit(kind, MAX_CANONICAL_BODY_BYTES_V1 as u64),
            Ok(())
        );
        assert_limit(
            check_body_limit(kind, MAX_CANONICAL_BODY_BYTES_V1 as u64 + 1).unwrap_err(),
            kind,
            MAX_CANONICAL_BODY_BYTES_V1 as u64 + 1,
            MAX_CANONICAL_BODY_BYTES_V1 as u64,
        );
    }
    assert_eq!(
        checked_body_add(u64::MAX, 1).unwrap_err(),
        SchemaIdentityErrorV1::ArithmeticOverflow {
            operation: SchemaIdentityOperationV1::CanonicalBodyAccounting,
        }
    );
}

#[test]
fn submitted_utf8_counts_occurrences_before_constraint_deduplication() {
    let input = non_empty_input();
    assert_eq!(submitted_utf8_bytes(&input).unwrap(), 174);
    let mut duplicate = input.clone();
    duplicate.constraints.push(input.constraints[0].clone());
    assert!(submitted_utf8_bytes(&duplicate).unwrap() > submitted_utf8_bytes(&input).unwrap());
}

#[test]
fn aggregate_caps_are_wired_through_normalization_and_preflight() {
    assert_limit_kind(
        limited_build(non_empty_input(), |limits| limits.columns_total = 2).unwrap_err(),
        SchemaIdentityLimitV1::ColumnsTotal,
    );
    assert_limit_kind(
        limited_build(non_empty_input(), |limits| limits.raw_constraints = 5).unwrap_err(),
        SchemaIdentityLimitV1::RawConstraints,
    );
    assert_limit_kind(
        limited_build(non_empty_input(), |limits| limits.canonical_constraints = 5).unwrap_err(),
        SchemaIdentityLimitV1::CanonicalConstraints,
    );
    assert_limit_kind(
        limited_build(non_empty_input(), |limits| limits.facets_total = 2).unwrap_err(),
        SchemaIdentityLimitV1::FacetsTotal,
    );

    let mut lists = non_empty_input();
    lists.relations[0].columns[0].source_type.facets = vec![
        TypeFacetV1 {
            key: token("a"),
            value: TypeFacetValueV1::TextList(vec![TextValueV1::new("x").unwrap(); 2]),
        },
        TypeFacetV1 {
            key: token("b"),
            value: TypeFacetValueV1::TypeNameList(vec![qname(None, "t"); 2]),
        },
    ];
    assert_limit_kind(
        limited_build(lists, |limits| limits.facet_list_items_total = 3).unwrap_err(),
        SchemaIdentityLimitV1::FacetListItemsTotal,
    );

    let utf8 = submitted_utf8_bytes(&non_empty_input()).unwrap();
    assert_limit_kind(
        limited_build(non_empty_input(), |limits| {
            limits.utf8_payload_bytes = usize::try_from(utf8 - 1).unwrap();
        })
        .unwrap_err(),
        SchemaIdentityLimitV1::Utf8PayloadBytes,
    );

    let body_cases: [BodyLimitCaseV1; 3] = [
        (
            SchemaIdentityLimitV1::StructuralBodyBytes,
            |limits: &mut KernelLimitsV1| limits.structural_body_bytes = 23,
        ),
        (SchemaIdentityLimitV1::TypeBodyBytes, |limits| {
            limits.type_body_bytes = 23;
        }),
        (SchemaIdentityLimitV1::ConstraintBodyBytes, |limits| {
            limits.constraint_body_bytes = 29;
        }),
    ];
    for (kind, configure) in body_cases {
        assert_limit_kind(limited_build(empty_input(), configure).unwrap_err(), kind);
    }
}

#[test]
fn canonical_cap_applies_after_unique_normalization_and_deduplication() {
    let unique = |columns| ConstraintInputV1::UniqueKey {
        relation: relation_ref("staff"),
        state: ConstraintStateV1 {
            validated: true,
            enforced: true,
        },
        nulls: UniqueNullSemanticsV1::NullsDistinct,
        columns,
    };
    let mut equivalent = non_empty_input();
    equivalent.constraints = vec![
        unique(vec![column_key(1, "id"), column_key(2, "dept_id")]),
        unique(vec![column_key(2, "dept_id"), column_key(1, "id")]),
    ];
    limited_build(equivalent, |limits| limits.canonical_constraints = 1).unwrap();

    let mut distinct = non_empty_input();
    distinct.constraints.truncate(2);
    assert_limit_kind(
        limited_build(distinct, |limits| limits.canonical_constraints = 1).unwrap_err(),
        SchemaIdentityLimitV1::CanonicalConstraints,
    );
}

#[test]
fn planned_length_mismatch_rejects_without_an_identity() {
    let error = digest_body(b"test-domain\0", 0, false, |sink| sink.write(&[1])).unwrap_err();
    assert!(matches!(
        error,
        SchemaIdentityErrorV1::Invalid {
            code: SchemaIdentityInvalidCodeV1::CanonicalLengthMismatch,
            ..
        }
    ));
}

#[test]
fn errors_are_bounded_redacted_and_have_no_source_chain() {
    let marker = "postgresql-secret-user-password-host-path-sql";
    let mut input = non_empty_input();
    input.relations[0].columns[1].name = identifier(marker);
    input.relations[0].columns[0].name = identifier(marker);
    let mut errors = vec![ObservedSchemaIdentityV1::build(input).unwrap_err()];
    let locations = [
        SchemaIdentityLocationV1::root(),
        SchemaIdentityLocationV1::at_relation(1),
        SchemaIdentityLocationV1::at_column(1, 2),
        SchemaIdentityLocationV1::at_facet(1, 2, 3),
        SchemaIdentityLocationV1::at_constraint(4),
        SchemaIdentityLocationV1::at_member(4, 5),
    ];
    let codes = [
        SchemaIdentityInvalidCodeV1::ProfileId,
        SchemaIdentityInvalidCodeV1::Token,
        SchemaIdentityInvalidCodeV1::Identifier,
        SchemaIdentityInvalidCodeV1::TextValue,
        SchemaIdentityInvalidCodeV1::RelationKind,
        SchemaIdentityInvalidCodeV1::EmptyRelation,
        SchemaIdentityInvalidCodeV1::EmptyKey,
        SchemaIdentityInvalidCodeV1::OrdinalContinuity,
        SchemaIdentityInvalidCodeV1::DuplicateRelation,
        SchemaIdentityInvalidCodeV1::DuplicateColumnOrdinal,
        SchemaIdentityInvalidCodeV1::DuplicateColumnName,
        SchemaIdentityInvalidCodeV1::DuplicateFacetKey,
        SchemaIdentityInvalidCodeV1::DuplicateKeyMember,
        SchemaIdentityInvalidCodeV1::DuplicateForeignKeyPair,
        SchemaIdentityInvalidCodeV1::DuplicateForeignKeyChild,
        SchemaIdentityInvalidCodeV1::DuplicateForeignKeyParent,
        SchemaIdentityInvalidCodeV1::DanglingRelation,
        SchemaIdentityInvalidCodeV1::DanglingColumn,
        SchemaIdentityInvalidCodeV1::MultiplePrimaryKey,
        SchemaIdentityInvalidCodeV1::CanonicalLengthMismatch,
    ];
    errors.extend(codes.into_iter().enumerate().map(|(index, code)| {
        SchemaIdentityErrorV1::Invalid {
            code,
            location: locations[index % locations.len()],
        }
    }));
    errors.push(SchemaIdentityErrorV1::LimitExceeded {
        limit: SchemaIdentityLimitV1::Utf8PayloadBytes,
        observed: u64::MAX,
        maximum: u64::MAX - 1,
    });
    errors.extend(
        [
            SchemaIdentityOperationV1::IndexConversion,
            SchemaIdentityOperationV1::U32LengthConversion,
            SchemaIdentityOperationV1::Utf8PayloadAccounting,
            SchemaIdentityOperationV1::CanonicalBodyAccounting,
            SchemaIdentityOperationV1::CanonicalStreamAccounting,
        ]
        .map(|operation| SchemaIdentityErrorV1::ArithmeticOverflow { operation }),
    );
    for error in errors {
        let display = error.to_string();
        let debug = format!("{error:?}");
        assert!(display.is_ascii() && debug.is_ascii());
        assert!(display.len() <= 256 && debug.len() <= 256);
        assert!(!display.contains(marker) && !debug.contains(marker));
        assert!(std::error::Error::source(&error).is_none());
    }
}

#[test]
fn empty_relations_and_ordinal_gaps_report_submitted_indexes() {
    let mut empty = empty_input();
    empty.relations.push(RelationInputV1 {
        name: qname(None, "empty"),
        kind: RelationKindV1::BaseTable,
        columns: Vec::new(),
    });
    match ObservedSchemaIdentityV1::build(empty).unwrap_err() {
        SchemaIdentityErrorV1::Invalid { code, location } => {
            assert_eq!(code, SchemaIdentityInvalidCodeV1::EmptyRelation);
            assert_eq!(location.relation(), Some(0));
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let mut gap = non_empty_input();
    gap.relations[0].columns[1].ordinal = NonZeroU32::new(3).unwrap();
    assert!(matches!(
        ObservedSchemaIdentityV1::build(gap),
        Err(SchemaIdentityErrorV1::Invalid {
            code: SchemaIdentityInvalidCodeV1::OrdinalContinuity,
            ..
        })
    ));
}
