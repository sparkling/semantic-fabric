use std::collections::BTreeMap;

use crate::schema::TableSchema;

use super::{Postgres16NormalizedRelationsV1, PostgresSchemaIdentityUnavailableV1};

pub(in crate::introspect::postgres::observation) fn compare_postgres16_legacy_coordinates_v1(
    legacy: &[TableSchema],
    rich: &Postgres16NormalizedRelationsV1,
) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
    if legacy.len() != rich.relations.len() {
        return Err(PostgresSchemaIdentityUnavailableV1::LegacyCoordinateMismatch);
    }
    let mut legacy_by_name = BTreeMap::new();
    for relation in legacy {
        if legacy_by_name
            .insert(relation.name.as_str(), relation)
            .is_some()
        {
            return Err(PostgresSchemaIdentityUnavailableV1::LegacyCoordinateMismatch);
        }
    }
    for relation in &rich.relations {
        let legacy_relation = legacy_by_name
            .get(relation.name.local.as_str())
            .ok_or(PostgresSchemaIdentityUnavailableV1::LegacyCoordinateMismatch)?;
        if legacy_relation.columns.len() != relation.columns.len()
            || legacy_relation.columns.iter().zip(&relation.columns).any(
                |(legacy_column, rich_column)| {
                    legacy_column.name.as_str() != rich_column.name.as_str()
                },
            )
        {
            return Err(PostgresSchemaIdentityUnavailableV1::LegacyCoordinateMismatch);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use sf_core::schema::{Column, ForeignKey, FunctionalDep, TableSchema};

    use super::super::normalize_postgres16_relations_v1;
    use super::super::tests::{live_attribute, relation_fact};
    use super::*;

    fn rich() -> Postgres16NormalizedRelationsV1 {
        normalize_postgres16_relations_v1(
            vec![relation_fact(9, "beta", 1), relation_fact(4, "alpha", 2)],
            vec![
                live_attribute(9, 1, "payload", 25, "text", false),
                live_attribute(4, 2, "label", 25, "text", false),
                live_attribute(4, 1, "id", 23, "int4", true),
            ],
        )
        .unwrap()
    }

    fn legacy() -> Vec<TableSchema> {
        let mut beta = TableSchema::new("beta");
        beta.columns = vec![Column::new("payload", "arbitrary", true)];
        beta.primary_key = vec!["payload".to_owned()];
        beta.row_estimate = Some(99);

        let mut alpha = TableSchema::new("alpha");
        alpha.columns = vec![
            Column::new("id", "not-the-rich-type", false),
            Column::new("label", "also-ignored", true),
        ];
        alpha.columns[0].distinct_estimate = Some(7);
        alpha.unique = vec![vec!["label".to_owned()]];
        alpha.foreign_keys = vec![ForeignKey {
            columns: vec!["id".to_owned()],
            parent_table: "beta".to_owned(),
            parent_columns: vec!["payload".to_owned()],
        }];
        alpha.functional_dependencies = vec![FunctionalDep {
            det: vec!["id".to_owned()],
            dep: vec!["label".to_owned()],
        }];
        vec![beta, alpha]
    }

    fn assert_mismatch(legacy: Vec<TableSchema>) {
        assert_eq!(
            compare_postgres16_legacy_coordinates_v1(&legacy, &rich()),
            Err(PostgresSchemaIdentityUnavailableV1::LegacyCoordinateMismatch)
        );
    }

    #[test]
    fn comparison_is_setwise_for_relations_and_ignores_non_coordinate_legacy_facts() {
        compare_postgres16_legacy_coordinates_v1(&legacy(), &rich()).unwrap();
        let mut reversed = legacy();
        reversed.reverse();
        compare_postgres16_legacy_coordinates_v1(&reversed, &rich()).unwrap();
        compare_postgres16_legacy_coordinates_v1(
            &[],
            &normalize_postgres16_relations_v1(Vec::new(), Vec::new()).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn every_relation_coordinate_difference_is_rejected() {
        let mut missing = legacy();
        missing.pop();
        assert_mismatch(missing);

        let mut extra = legacy();
        extra.push(TableSchema::new("extra"));
        assert_mismatch(extra);

        let mut renamed = legacy();
        renamed[0].name = "renamed".to_owned();
        assert_mismatch(renamed);

        let mut duplicate = legacy();
        duplicate[1].name = duplicate[0].name.clone();
        assert_mismatch(duplicate);
    }

    #[test]
    fn every_dense_column_coordinate_difference_is_rejected() {
        let mut missing = legacy();
        missing[1].columns.pop();
        assert_mismatch(missing);

        let mut extra = legacy();
        extra[0].columns.push(Column::new("extra", "int", false));
        assert_mismatch(extra);

        let mut reordered = legacy();
        reordered[1].columns.reverse();
        assert_mismatch(reordered);

        let mut renamed = legacy();
        renamed[1].columns[0].name = "renamed".to_owned();
        assert_mismatch(renamed);
    }
}
