use std::cmp::Ordering;

use super::*;
use crate::schema_identity::encode::{
    self, write_column_key, write_constraint, write_qname, write_relation_coord, ByteSink,
};
use crate::schema_identity::validate::normalize;

const STRUCTURAL_DOMAIN: &[u8] = b"semantic-fabric:observed-schema:structural:v1\0";
const TYPE_DOMAIN: &[u8] = b"semantic-fabric:observed-schema:type:v1\0";
const CONSTRAINT_DOMAIN: &[u8] = b"semantic-fabric:observed-schema:constraint:v1\0";

fn catalogued_qname(catalog: Option<&str>, schema: Option<&str>, local: &str) -> QualifiedNameV1 {
    QualifiedNameV1 {
        catalog: catalog.map(identifier),
        schema: schema.map(identifier),
        local: identifier(local),
    }
}

fn all_facet_input() -> SchemaObservationInputV1 {
    let relation_name = catalogued_qname(Some("cat"), Some("app"), "r");
    let digest = std::array::from_fn(|index| u8::try_from(index).unwrap());
    let facets = vec![
        TypeFacetV1 {
            key: token("h"),
            value: TypeFacetValueV1::Digest32(digest),
        },
        TypeFacetV1 {
            key: token("b"),
            value: TypeFacetValueV1::U64(0x0102_0304_0506_0708),
        },
        TypeFacetV1 {
            key: token("aa"),
            value: TypeFacetValueV1::Bool(false),
        },
        TypeFacetV1 {
            key: token("g"),
            value: TypeFacetValueV1::TypeNameList(vec![
                catalogued_qname(None, Some("s"), "one"),
                catalogued_qname(Some("c"), None, "two"),
            ]),
        },
        TypeFacetV1 {
            key: token("c"),
            value: TypeFacetValueV1::I64(-2),
        },
        TypeFacetV1 {
            key: token("f"),
            value: TypeFacetValueV1::TextList(vec![
                TextValueV1::new("second").unwrap(),
                TextValueV1::new("first").unwrap(),
            ]),
        },
        TypeFacetV1 {
            key: token("e"),
            value: TypeFacetValueV1::TypeName(catalogued_qname(Some("types"), None, "named")),
        },
        TypeFacetV1 {
            key: token("d"),
            value: TypeFacetValueV1::Text(TextValueV1::new("").unwrap()),
        },
    ];
    SchemaObservationInputV1 {
        profiles: SchemaProfilesV1 {
            structural: profile("kat-struct-v1"),
            types: profile("kat-type-v1"),
            constraints: profile("kat-constraint-v1"),
        },
        relations: vec![RelationInputV1 {
            name: relation_name.clone(),
            kind: RelationKindV1::BaseTable,
            columns: vec![ColumnInputV1 {
                ordinal: NonZeroU32::new(1).unwrap(),
                name: identifier("c"),
                source_type: SourceTypeV1 {
                    native_name: catalogued_qname(Some("sys"), Some("std"), "t"),
                    family: TypeFamilyV1::Opaque,
                    facets,
                },
            }],
        }],
        constraints: vec![ConstraintInputV1::NotNull {
            column: ColumnRefV1 {
                relation: RelationRefV1 {
                    name: relation_name,
                },
                column: column_key(1, "c"),
            },
            state: ConstraintStateV1 {
                validated: false,
                enforced: false,
            },
        }],
    }
}

#[derive(Default)]
struct VecSink(Vec<u8>);

impl ByteSink for VecSink {
    fn write(&mut self, bytes: &[u8]) -> Result<(), SchemaIdentityErrorV1> {
        self.0.extend_from_slice(bytes);
        Ok(())
    }

    fn written(&self) -> u64 {
        u64::try_from(self.0.len()).unwrap()
    }
}

fn encoded(write: impl FnOnce(&mut VecSink) -> Result<(), SchemaIdentityErrorV1>) -> Vec<u8> {
    let mut sink = VecSink::default();
    write(&mut sink).unwrap();
    sink.0
}

fn vector_identity(input: SchemaObservationInputV1) -> ObservedSchemaIdentityV1 {
    ObservedSchemaIdentityV1::build(input).unwrap()
}

fn normalized_column(ordinal: u32, name: &str) -> NormalizedColumnV1 {
    let column = column(ordinal, name);
    NormalizedColumnV1 {
        ordinal: column.ordinal,
        name: column.name,
        source_type: column.source_type,
    }
}

fn assert_pairwise_encoding_order<T>(
    values: &[T],
    compare: impl Fn(&T, &T) -> Ordering,
    encode: impl Fn(&T) -> Vec<u8>,
) {
    for left in values {
        for right in values {
            let comparison = compare(left, right);
            let left_bytes = encode(left);
            let right_bytes = encode(right);
            assert_eq!(comparison, left_bytes.cmp(&right_bytes));
            assert_eq!(comparison == Ordering::Equal, left_bytes == right_bytes);
        }
    }
}

#[test]
fn empty_structural_known_answer() {
    assert_vector(
        empty_input(),
        DigestPart::Structural,
        STRUCTURAL_DOMAIN,
        concat!("00000010", "706f727461626c652d626173652d7631", "00000000"),
        78,
        "4934b444efc6178b895c553448fb486c50ff6c16feaf3179535d184bf885f1c5",
    );
}

#[test]
fn empty_type_known_answer() {
    assert_vector(
        empty_input(),
        DigestPart::Type,
        TYPE_DOMAIN,
        concat!("00000010", "706f727461626c652d747970652d7631", "00000000"),
        72,
        "bcf3964b5b48a6c5a6fc2786dd15cf333a8f193756b5b8a7b282665544369486",
    );
}

#[test]
fn empty_constraint_known_answer() {
    assert_vector(
        empty_input(),
        DigestPart::Constraint,
        CONSTRAINT_DOMAIN,
        concat!(
            "00000016",
            "706f727461626c652d636f6e73747261696e742d7631",
            "00000000"
        ),
        84,
        "0f8a5a297f308b392f87d9359aafa0a67fd3bc71385448524471e8de671b2884",
    );
}

#[test]
fn non_empty_structural_known_answer() {
    assert_vector(
        non_empty_input(),
        DigestPart::Structural,
        STRUCTURAL_DOMAIN,
        concat!(
            "0000000d6b61742d7374727563742d763100000002",
            "1100010000000361707000000004646570740100000001",
            "1200000001000000026964",
            "110001000000036170700000000573746166660100000002",
            "1200000001000000026964",
            "120000000200000007646570745f6964"
        ),
        160,
        "ddd247fe2adf8831e101a1273e3692c5586370da31c8877d403f75e948fa4f38",
    );
}

#[test]
fn non_empty_type_known_answer() {
    assert_vector(
        non_empty_input(),
        DigestPart::Type,
        TYPE_DOMAIN,
        concat!(
            "0000000b6b61742d747970652d763100000003",
            "2100010000000361707000000004646570740100000001000000026964",
            "000100000003737464000000036933320200000001",
            "220000000462697473020000000000000020",
            "210001000000036170700000000573746166660100000001000000026964",
            "000100000003737464000000036933320200000001",
            "220000000462697473020000000000000020",
            "21000100000003617070000000057374616666010000000200000007646570745f6964",
            "000100000003737464000000036933320200000001",
            "220000000462697473020000000000000020"
        ),
        278,
        "9b6ef1560bbe702ceb44b5bf0d2c46a2e1c5e88ceb1c134c334c3910f84e9574",
    );
}

#[test]
fn non_empty_constraint_known_answer() {
    assert_vector(
        non_empty_input(),
        DigestPart::Constraint,
        CONSTRAINT_DOMAIN,
        concat!(
            "000000116b61742d636f6e73747261696e742d763100000006",
            "31000100000003617070000000046465707401000000010000000269640101",
            "3100010000000361707000000005737461666601000000010000000269640101",
            "3200010000000361707000000004646570740101010000000100000001000000026964",
            "320001000000036170700000000573746166660101010000000100000001000000026964",
            "3300010000000361707000000005737461666601010101000000010000000200000007646570745f6964",
            "340001000000036170700000000573746166660100010000000361707000000004646570740101010100000001",
            "0000000200000007646570745f696400000001000000026964"
        ),
        325,
        "478af9a49cbceddc91b2e0b75cdfc1fc88a4a02f166bb6ee4b1b1005870beeb2",
    );
}

#[test]
fn all_facet_catalogued_false_state_known_answers() {
    let input = all_facet_input();
    assert_vector(
        input.clone(),
        DigestPart::Structural,
        STRUCTURAL_DOMAIN,
        "0000000d6b61742d7374727563742d76310000000111010000000363617401000000036170700000000172010000000112000000010000000163",
        112,
        "68a87a4082ece1588341450c23b457de1bdf308b68abdc343ad041be879382e3",
    );
    assert_vector(
        input.clone(),
        DigestPart::Type,
        TYPE_DOMAIN,
        concat!(
            "0000000b6b61742d747970652d7631000000012101000000036361740100000003617070000000017201000000010000000163",
            "0100000003737973010000000373746400000001747f00000008220000000261610100220000000162020102030405060708",
            "22000000016303fffffffffffffffe2200000001640400000000220000000165050100000005747970657300000000056e616d6564",
            "2200000001660600000002000000067365636f6e64000000056669727374220000000167070000000200010000000173000000036f6e65",
            "010000000163000000000374776f22000000016808000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
        ),
        310,
        "f6e72e5c1160674182b44db8308a082cf0f62d7f4c56f7c91e1cd8ba4e1ada30",
    );
    assert_vector(
        input,
        DigestPart::Constraint,
        CONSTRAINT_DOMAIN,
        "000000116b61742d636f6e73747261696e742d76310000000131010000000363617401000000036170700000000172010000000100000001630000",
        113,
        "ce8a4f844ad0858b4e3eab65833bed33b726cfdf9b56256ca774990d2c1ad7ab",
    );
}

#[test]
fn manual_comparators_equal_complete_encoded_byte_order() {
    let qnames = vec![
        catalogued_qname(None, None, "b"),
        catalogued_qname(None, None, "aa"),
        catalogued_qname(None, Some("s"), "a"),
        catalogued_qname(Some("c"), None, "a"),
        catalogued_qname(None, Some("x"), "a"),
        catalogued_qname(Some("x"), None, "a"),
        catalogued_qname(Some("c"), Some("s"), "a"),
    ];
    assert_pairwise_encoding_order(&qnames, encode::cmp_qname, |value| {
        encoded(|sink| write_qname(sink, value))
    });

    let mut input = non_empty_input();
    input.relations[1].columns.push(column(2, "alternate_id"));
    input.constraints.clear();
    let observation = normalize(input, KernelLimitsV1::PRODUCTION).unwrap();
    assert_pairwise_encoding_order(
        &observation.relations,
        encode::cmp_relation_coord,
        |value| encoded(|sink| write_relation_coord(sink, value)),
    );

    let columns = vec![
        normalized_column(1, "b"),
        normalized_column(1, "aa"),
        normalized_column(2, "a"),
    ];
    assert_pairwise_encoding_order(&columns, encode::cmp_column_key, |value| {
        encoded(|sink| write_column_key(sink, value))
    });

    let state = |validated, enforced| ConstraintStateV1 {
        validated,
        enforced,
    };
    let mut constraints = vec![
        NormalizedConstraintV1::NotNull {
            relation: 0,
            column: 0,
            state: state(false, false),
        },
        NormalizedConstraintV1::NotNull {
            relation: 0,
            column: 0,
            state: state(false, true),
        },
        NormalizedConstraintV1::NotNull {
            relation: 1,
            column: 1,
            state: state(true, false),
        },
        NormalizedConstraintV1::PrimaryKey {
            relation: 0,
            state: state(false, false),
            columns: vec![0],
        },
        NormalizedConstraintV1::PrimaryKey {
            relation: 0,
            state: state(true, true),
            columns: vec![0, 1],
        },
        NormalizedConstraintV1::PrimaryKey {
            relation: 0,
            state: state(true, true),
            columns: vec![1, 0],
        },
    ];
    for nulls in [
        UniqueNullSemanticsV1::NullsDistinct,
        UniqueNullSemanticsV1::NullsNotDistinct,
        UniqueNullSemanticsV1::ObservedUnknown,
    ] {
        constraints.push(NormalizedConstraintV1::UniqueKey {
            relation: 1,
            state: state(true, false),
            nulls,
            columns: vec![0],
        });
    }
    constraints.push(NormalizedConstraintV1::UniqueKey {
        relation: 1,
        state: state(true, false),
        nulls: UniqueNullSemanticsV1::ObservedUnknown,
        columns: vec![0, 1],
    });
    for match_kind in [
        ForeignKeyMatchV1::Simple,
        ForeignKeyMatchV1::Full,
        ForeignKeyMatchV1::Partial,
        ForeignKeyMatchV1::ObservedUnknown,
    ] {
        constraints.push(NormalizedConstraintV1::ForeignKey {
            child: 1,
            parent: 0,
            state: state(true, true),
            match_kind,
            pairs: vec![(0, 0)],
        });
    }
    constraints.extend([
        NormalizedConstraintV1::ForeignKey {
            child: 1,
            parent: 0,
            state: state(true, true),
            match_kind: ForeignKeyMatchV1::ObservedUnknown,
            pairs: vec![(0, 0), (1, 1)],
        },
        NormalizedConstraintV1::ForeignKey {
            child: 1,
            parent: 0,
            state: state(true, true),
            match_kind: ForeignKeyMatchV1::ObservedUnknown,
            pairs: vec![(1, 1), (0, 0)],
        },
    ]);
    assert_pairwise_encoding_order(
        &constraints,
        |left, right| encode::cmp_constraint(left, right, &observation.relations),
        |value| encoded(|sink| write_constraint(sink, value, &observation.relations)),
    );
}

fn assert_digest_changes(
    baseline: ObservedSchemaIdentityV1,
    changed: ObservedSchemaIdentityV1,
    expected: [bool; 3],
) {
    assert_eq!(baseline.structural() != changed.structural(), expected[0]);
    assert_eq!(baseline.types() != changed.types(), expected[1]);
    assert_eq!(baseline.constraints() != changed.constraints(), expected[2]);
}

#[test]
fn digest_independence_and_coordinate_propagation_are_exact() {
    let baseline = vector_identity(non_empty_input());
    assert_eq!(
        baseline.structural().as_bytes().as_slice(),
        decode_hex("ddd247fe2adf8831e101a1273e3692c5586370da31c8877d403f75e948fa4f38")
    );

    let mut structural_profile = non_empty_input();
    structural_profile.profiles.structural = profile("kat-struct-v2");
    assert_digest_changes(
        baseline,
        vector_identity(structural_profile),
        [true, false, false],
    );

    let mut type_fact = non_empty_input();
    type_fact.relations[0].columns[0].source_type.facets[0].value = TypeFacetValueV1::U64(64);
    assert_digest_changes(baseline, vector_identity(type_fact), [false, true, false]);

    let mut constraint_profile = non_empty_input();
    constraint_profile.profiles.constraints = profile("kat-constraint-v2");
    assert_digest_changes(
        baseline,
        vector_identity(constraint_profile),
        [false, false, true],
    );

    let mut renamed = non_empty_input();
    let people = qname(Some("app"), "people");
    renamed.relations[0].name = people.clone();
    renamed.relations[0].columns[0].name = identifier("employee_id");
    if let ConstraintInputV1::ForeignKey { child, .. } = &mut renamed.constraints[0] {
        child.name = people.clone();
    }
    if let ConstraintInputV1::UniqueKey { relation, .. } = &mut renamed.constraints[1] {
        relation.name = people.clone();
    }
    if let ConstraintInputV1::PrimaryKey {
        relation, columns, ..
    } = &mut renamed.constraints[2]
    {
        relation.name = people.clone();
        columns[0].name = identifier("employee_id");
    }
    if let ConstraintInputV1::NotNull { column, .. } = &mut renamed.constraints[3] {
        column.relation.name = people;
        column.column.name = identifier("employee_id");
    }
    assert_digest_changes(baseline, vector_identity(renamed), [true, true, true]);
}
