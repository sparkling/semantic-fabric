use std::num::NonZeroU32;

use super::*;

mod bounds;
mod mutations;
mod vectors;

#[derive(Clone, Copy)]
enum DigestPart {
    Structural,
    Type,
    Constraint,
}

fn identifier(value: &str) -> IdentifierV1 {
    IdentifierV1::new(value).expect("valid test identifier")
}

fn profile(value: &str) -> ProfileIdV1 {
    ProfileIdV1::new(value).expect("valid test profile")
}

fn token(value: &str) -> TokenV1 {
    TokenV1::new(value).expect("valid test token")
}

fn qname(schema: Option<&str>, local: &str) -> QualifiedNameV1 {
    QualifiedNameV1 {
        catalog: None,
        schema: schema.map(identifier),
        local: identifier(local),
    }
}

fn relation_ref(local: &str) -> RelationRefV1 {
    RelationRefV1 {
        name: qname(Some("app"), local),
    }
}

fn column_key(ordinal: u32, name: &str) -> ColumnKeyV1 {
    ColumnKeyV1 {
        ordinal: NonZeroU32::new(ordinal).expect("non-zero test ordinal"),
        name: identifier(name),
    }
}

fn column(ordinal: u32, name: &str) -> ColumnInputV1 {
    ColumnInputV1 {
        ordinal: NonZeroU32::new(ordinal).expect("non-zero test ordinal"),
        name: identifier(name),
        source_type: SourceTypeV1 {
            native_name: qname(Some("std"), "i32"),
            family: TypeFamilyV1::SignedInteger,
            facets: vec![TypeFacetV1 {
                key: token("bits"),
                value: TypeFacetValueV1::U64(32),
            }],
        },
    }
}

fn empty_input() -> SchemaObservationInputV1 {
    SchemaObservationInputV1 {
        profiles: SchemaProfilesV1 {
            structural: profile("portable-base-v1"),
            types: profile("portable-type-v1"),
            constraints: profile("portable-constraint-v1"),
        },
        relations: Vec::new(),
        constraints: Vec::new(),
    }
}

fn non_empty_input() -> SchemaObservationInputV1 {
    let state = ConstraintStateV1 {
        validated: true,
        enforced: true,
    };
    SchemaObservationInputV1 {
        profiles: SchemaProfilesV1 {
            structural: profile("kat-struct-v1"),
            types: profile("kat-type-v1"),
            constraints: profile("kat-constraint-v1"),
        },
        relations: vec![
            RelationInputV1 {
                name: qname(Some("app"), "staff"),
                kind: RelationKindV1::BaseTable,
                columns: vec![column(1, "id"), column(2, "dept_id")],
            },
            RelationInputV1 {
                name: qname(Some("app"), "dept"),
                kind: RelationKindV1::BaseTable,
                columns: vec![column(1, "id")],
            },
        ],
        constraints: vec![
            ConstraintInputV1::ForeignKey {
                child: relation_ref("staff"),
                parent: relation_ref("dept"),
                state,
                match_kind: ForeignKeyMatchV1::Simple,
                pairs: vec![(column_key(2, "dept_id"), column_key(1, "id"))],
            },
            ConstraintInputV1::UniqueKey {
                relation: relation_ref("staff"),
                state,
                nulls: UniqueNullSemanticsV1::NullsDistinct,
                columns: vec![column_key(2, "dept_id")],
            },
            ConstraintInputV1::PrimaryKey {
                relation: relation_ref("staff"),
                state,
                columns: vec![column_key(1, "id")],
            },
            ConstraintInputV1::NotNull {
                column: ColumnRefV1 {
                    relation: relation_ref("staff"),
                    column: column_key(1, "id"),
                },
                state,
            },
            ConstraintInputV1::PrimaryKey {
                relation: relation_ref("dept"),
                state,
                columns: vec![column_key(1, "id")],
            },
            ConstraintInputV1::NotNull {
                column: ColumnRefV1 {
                    relation: relation_ref("dept"),
                    column: column_key(1, "id"),
                },
                state,
            },
        ],
    }
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0, "hex fixture must have whole bytes");
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("ASCII hex fixture");
            u8::from_str_radix(text, 16).expect("valid hex fixture")
        })
        .collect()
}

fn expected_preimage(domain: &[u8], body_hex: &str) -> Vec<u8> {
    let body = decode_hex(body_hex);
    let mut expected = Vec::with_capacity(domain.len() + 8 + body.len());
    expected.extend_from_slice(domain);
    expected.extend_from_slice(&(body.len() as u64).to_be_bytes());
    expected.extend_from_slice(&body);
    expected
}

fn assert_vector(
    input: SchemaObservationInputV1,
    part: DigestPart,
    domain: &[u8],
    body_hex: &str,
    expected_preimage_len: usize,
    expected_digest: &str,
) {
    let expected = expected_preimage(domain, body_hex);
    assert_eq!(expected.len(), expected_preimage_len);

    let (captured_identity, preimages) =
        build_with_preimages(input.clone()).expect("known-answer input must build");
    let ordinary_identity =
        ObservedSchemaIdentityV1::build(input).expect("ordinary streamed build must succeed");

    let (captured, captured_digest, ordinary_digest) = match part {
        DigestPart::Structural => (
            preimages.structural.as_slice(),
            captured_identity.structural().to_string(),
            ordinary_identity.structural().to_string(),
        ),
        DigestPart::Type => (
            preimages.types.as_slice(),
            captured_identity.types().to_string(),
            ordinary_identity.types().to_string(),
        ),
        DigestPart::Constraint => (
            preimages.constraints.as_slice(),
            captured_identity.constraints().to_string(),
            ordinary_identity.constraints().to_string(),
        ),
    };

    assert_eq!(captured, expected);
    assert_eq!(captured_digest, expected_digest);
    assert_eq!(ordinary_digest, expected_digest);
}

#[test]
fn remaining_scalar_boundaries_are_exact() {
    assert_eq!(
        TokenV1::new("a".repeat(MAX_PROFILE_TOKEN_BYTES_V1 + 1)).unwrap_err(),
        SchemaIdentityErrorV1::LimitExceeded {
            limit: SchemaIdentityLimitV1::ProfileOrTokenBytes,
            observed: 65,
            maximum: 64,
        }
    );
    assert_eq!(
        IdentifierV1::new("a".repeat(MAX_IDENTIFIER_BYTES_V1 + 1)).unwrap_err(),
        SchemaIdentityErrorV1::LimitExceeded {
            limit: SchemaIdentityLimitV1::IdentifierBytes,
            observed: 1_025,
            maximum: 1_024,
        }
    );
}

#[test]
fn production_limit_policy_is_the_normative_appendix_table() {
    let limits = KernelLimitsV1::PRODUCTION;
    let cases = [
        (limits.relations, MAX_RELATIONS_V1),
        (limits.columns_per_relation, MAX_COLUMNS_PER_RELATION_V1),
        (limits.columns_total, MAX_COLUMNS_TOTAL_V1),
        (limits.raw_constraints, MAX_RAW_CONSTRAINTS_V1),
        (limits.canonical_constraints, MAX_CANONICAL_CONSTRAINTS_V1),
        (limits.key_members, MAX_KEY_MEMBERS_V1),
        (limits.facets_per_column, MAX_FACETS_PER_COLUMN_V1),
        (limits.facets_total, MAX_FACETS_TOTAL_V1),
        (limits.facet_list_items, MAX_FACET_LIST_ITEMS_V1),
        (limits.facet_list_items_total, MAX_FACET_LIST_ITEMS_TOTAL_V1),
        (limits.utf8_payload_bytes, MAX_UTF8_PAYLOAD_BYTES_V1),
        (limits.structural_body_bytes, MAX_CANONICAL_BODY_BYTES_V1),
        (limits.type_body_bytes, MAX_CANONICAL_BODY_BYTES_V1),
        (limits.constraint_body_bytes, MAX_CANONICAL_BODY_BYTES_V1),
    ];
    for (actual, normative) in cases {
        assert_eq!(actual, normative);
    }
}
