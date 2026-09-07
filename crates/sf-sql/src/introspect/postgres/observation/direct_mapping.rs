//! Direct-Mapping DTO projection from admitted rich PostgreSQL facts.
//!
//! The compatibility `information_schema` DTOs are intentionally not inputs.
//! This keeps generation types, keys, nullability, and foreign keys inseparable
//! from the exact facts used to build the observed-schema identity.

use std::collections::HashMap;

use sf_core::schema_identity::{
    ColumnKeyV1, ConstraintInputV1, ConstraintStateV1, ForeignKeyMatchV1, QualifiedNameV1,
    RelationInputV1, RelationKindV1, TypeFamilyV1, UniqueNullSemanticsV1,
};

use crate::schema::{Column, ForeignKey, TableSchema};

use super::PostgresSchemaIdentityUnavailableV1;

pub(super) fn project_direct_mapping_tables_v1(
    relations: &[RelationInputV1],
    constraints: &[ConstraintInputV1],
) -> Result<Vec<TableSchema>, PostgresSchemaIdentityUnavailableV1> {
    let mut relation_indices = HashMap::with_capacity(relations.len());
    let mut tables = Vec::with_capacity(relations.len());
    for (index, relation) in relations.iter().enumerate() {
        validate_public_base_table(relation)?;
        if relation_indices
            .insert(relation.name.local.as_str(), index)
            .is_some()
        {
            return Err(unsupported_relation());
        }
        let mut table = TableSchema::new(relation.name.local.as_str());
        table.columns = relation
            .columns
            .iter()
            .enumerate()
            .map(|(column_index, column)| {
                let expected = u32::try_from(column_index + 1).map_err(|_| rejected())?;
                if column.ordinal.get() != expected {
                    return Err(rejected());
                }
                Ok(Column::new(
                    column.name.as_str(),
                    direct_mapping_sql_type(&column.source_type)?,
                    false,
                ))
            })
            .collect::<Result<_, _>>()?;
        tables.push(table);
    }

    for constraint in constraints {
        project_constraint(constraint, relations, &relation_indices, &mut tables)?;
    }
    Ok(tables)
}

fn project_constraint(
    constraint: &ConstraintInputV1,
    relations: &[RelationInputV1],
    relation_indices: &HashMap<&str, usize>,
    tables: &mut [TableSchema],
) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
    match constraint {
        ConstraintInputV1::NotNull { column, state } => {
            require_strong_state(*state)?;
            let relation_index = relation_index(&column.relation.name, relation_indices)?;
            let column_index = column_index(&relations[relation_index], &column.column)?;
            tables[relation_index].columns[column_index].not_null = true;
        }
        ConstraintInputV1::PrimaryKey {
            relation,
            state,
            columns,
        } => {
            require_strong_state(*state)?;
            let relation_index = relation_index(&relation.name, relation_indices)?;
            if !tables[relation_index].primary_key.is_empty() {
                return Err(unsupported_constraint());
            }
            tables[relation_index].primary_key = column_names(&relations[relation_index], columns)?;
        }
        ConstraintInputV1::UniqueKey {
            relation,
            state,
            nulls,
            columns,
        } => {
            require_strong_state(*state)?;
            if !matches!(
                nulls,
                UniqueNullSemanticsV1::NullsDistinct | UniqueNullSemanticsV1::NullsNotDistinct
            ) {
                return Err(unsupported_constraint());
            }
            let relation_index = relation_index(&relation.name, relation_indices)?;
            let names = column_names(&relations[relation_index], columns)?;
            tables[relation_index].unique.push(names);
        }
        ConstraintInputV1::ForeignKey {
            child,
            parent,
            state,
            match_kind,
            pairs,
        } => {
            if !state.enforced
                || !matches!(
                    match_kind,
                    ForeignKeyMatchV1::Simple | ForeignKeyMatchV1::Full
                )
            {
                return Err(unsupported_constraint());
            }
            let child_index = relation_index(&child.name, relation_indices)?;
            let parent_index = relation_index(&parent.name, relation_indices)?;
            if pairs.is_empty() {
                return Err(unsupported_constraint());
            }
            let mut columns = Vec::with_capacity(pairs.len());
            let mut parent_columns = Vec::with_capacity(pairs.len());
            for (child_column, parent_column) in pairs {
                let child_column_index = column_index(&relations[child_index], child_column)?;
                let parent_column_index = column_index(&relations[parent_index], parent_column)?;
                columns.push(
                    relations[child_index].columns[child_column_index]
                        .name
                        .as_str()
                        .into(),
                );
                parent_columns.push(
                    relations[parent_index].columns[parent_column_index]
                        .name
                        .as_str()
                        .into(),
                );
            }
            tables[child_index].foreign_keys.push(ForeignKey {
                columns,
                parent_table: relations[parent_index].name.local.as_str().into(),
                parent_columns,
            });
        }
    }
    Ok(())
}

fn validate_public_base_table(
    relation: &RelationInputV1,
) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
    if relation.columns.is_empty() {
        return Err(rejected());
    }
    if relation.kind != RelationKindV1::BaseTable
        || relation.name.catalog.is_some()
        || relation.name.schema.as_ref().map(|name| name.as_str()) != Some("public")
    {
        return Err(unsupported_relation());
    }
    Ok(())
}

fn relation_index(
    name: &QualifiedNameV1,
    indices: &HashMap<&str, usize>,
) -> Result<usize, PostgresSchemaIdentityUnavailableV1> {
    if name.catalog.is_some() || name.schema.as_ref().map(|value| value.as_str()) != Some("public")
    {
        return Err(unsupported_constraint());
    }
    indices
        .get(name.local.as_str())
        .copied()
        .ok_or_else(unsupported_constraint)
}

fn column_names(
    relation: &RelationInputV1,
    columns: &[ColumnKeyV1],
) -> Result<Vec<String>, PostgresSchemaIdentityUnavailableV1> {
    if columns.is_empty() {
        return Err(unsupported_constraint());
    }
    columns
        .iter()
        .map(|column| {
            let index = column_index(relation, column)?;
            Ok(relation.columns[index].name.as_str().into())
        })
        .collect()
}

fn column_index(
    relation: &RelationInputV1,
    key: &ColumnKeyV1,
) -> Result<usize, PostgresSchemaIdentityUnavailableV1> {
    let index = usize::try_from(key.ordinal.get() - 1).map_err(|_| rejected())?;
    relation
        .columns
        .get(index)
        .filter(|column| column.ordinal == key.ordinal && column.name == key.name)
        .map(|_| index)
        .ok_or_else(unsupported_constraint)
}

fn require_strong_state(
    state: ConstraintStateV1,
) -> Result<(), PostgresSchemaIdentityUnavailableV1> {
    if state.validated && state.enforced {
        Ok(())
    } else {
        Err(unsupported_constraint())
    }
}

fn direct_mapping_sql_type(
    source: &sf_core::schema_identity::SourceTypeV1,
) -> Result<&'static str, PostgresSchemaIdentityUnavailableV1> {
    if source.native_name.catalog.is_some()
        || source.native_name.schema.as_ref().map(|name| name.as_str()) != Some("pg_catalog")
    {
        return Err(PostgresSchemaIdentityUnavailableV1::UnsupportedType);
    }
    use TypeFamilyV1 as Family;
    match (source.native_name.local.as_str(), source.family) {
        ("bool", Family::Boolean) => Ok("boolean"),
        ("int2", Family::SignedInteger) => Ok("smallint"),
        ("int4", Family::SignedInteger) => Ok("integer"),
        ("int8", Family::SignedInteger) => Ok("bigint"),
        ("numeric", Family::ExactNumeric) => Ok("numeric"),
        ("float4", Family::ApproximateNumeric) => Ok("real"),
        ("float8", Family::ApproximateNumeric) => Ok("double precision"),
        ("text", Family::Character) => Ok("text"),
        ("varchar", Family::Character) => Ok("character varying"),
        ("bpchar", Family::Character) => Ok("character"),
        ("bytea", Family::Binary) => Ok("bytea"),
        ("date", Family::Date) => Ok("date"),
        ("time", Family::Time) => Ok("time without time zone"),
        ("timetz", Family::Time) => Ok("time with time zone"),
        ("timestamp", Family::Timestamp) => Ok("timestamp without time zone"),
        ("timestamptz", Family::Timestamp) => Ok("timestamp with time zone"),
        ("json", Family::Json) => Ok("json"),
        ("jsonb", Family::Json) => Ok("jsonb"),
        ("uuid", Family::Uuid) => Ok("uuid"),
        _ => Err(PostgresSchemaIdentityUnavailableV1::UnsupportedType),
    }
}

fn rejected() -> PostgresSchemaIdentityUnavailableV1 {
    PostgresSchemaIdentityUnavailableV1::IdentityRejected
}

fn unsupported_relation() -> PostgresSchemaIdentityUnavailableV1 {
    PostgresSchemaIdentityUnavailableV1::UnsupportedRelation
}

fn unsupported_constraint() -> PostgresSchemaIdentityUnavailableV1 {
    PostgresSchemaIdentityUnavailableV1::UnsupportedConstraint
}

#[cfg(test)]
mod tests;
