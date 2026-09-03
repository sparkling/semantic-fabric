use std::collections::BTreeMap;

use super::super::relation::{
    Postgres16AttributeCoordinateV1, Postgres16ColumnCoordinateV1, Postgres16NormalizedRelationsV1,
    Postgres16RelationCoordinateV1,
};
use super::*;

#[test]
fn observed_not_nulls_are_emitted_from_live_coordinates_only() {
    let relation_oid = 41;
    let relations = Postgres16NormalizedRelationsV1 {
        relations: Vec::new(),
        coordinates_by_relation_oid: BTreeMap::from([(
            relation_oid,
            Postgres16RelationCoordinateV1 {
                relation_index: 0,
                attributes_by_number: BTreeMap::from([
                    (1, live(true)),
                    (2, live(false)),
                    (3, Postgres16AttributeCoordinateV1::Dropped),
                ]),
            },
        )]),
    };

    assert_eq!(
        observed_not_null_constraints_v1(&relations).unwrap(),
        vec![Postgres16RawConstraintV1::NotNull {
            relation_oid,
            attnum: 1,
            validated: true,
        }]
    );
}

fn live(is_not_null: bool) -> Postgres16AttributeCoordinateV1 {
    Postgres16AttributeCoordinateV1::Live(Postgres16ColumnCoordinateV1 {
        column_index: 0,
        type_oid: 23,
        collation_oid: 0,
        is_not_null,
    })
}
