//! Validation and backend-qualified row identity for Direct Mapping input.

use std::collections::{BTreeMap, BTreeSet};

use sf_core::{NamedNode, Result, TableSchema};

use super::term::encoded_len;

/// Maximum UTF-8 size of a Direct Mapping base IRI accepted at the public
/// boundary. This is checked before parsing or generated-string allocation.
pub const MAX_DIRECT_MAPPING_BASE_IRI_BYTES_V1: usize = 8 * 1_024;
/// Maximum prospective IR construction work for one generated mapping.
pub const MAX_DIRECT_MAPPING_WORK_UNITS_V1: usize = 262_144;
/// Maximum prospective UTF-8 payload retained by one generated mapping.
pub const MAX_DIRECT_MAPPING_GENERATED_UTF8_BYTES_V1: usize = 64 * 1_024 * 1_024;

/// The one SQLite pseudo-column name understood by the current SQL compiler.
/// Generation rejects a physical column that shadows it rather than silently
/// switching to another alias that downstream validation would treat as real.
pub(super) const SYNTHETIC_ROW_ID_COLUMN: &str = "rowid";

/// Backend-qualified physical identity available for a table without a declared
/// primary key. This is an execution fact, not a durable row identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectMappingRowIdentity {
    /// SQLite base-table `rowid`, valid for an ordinary table without a primary
    /// key. `WITHOUT ROWID` tables necessarily declare a primary key.
    SqliteRowId,
    /// PostgreSQL `ctid`, admitted only by isolated conformance execution. Live
    /// serving must reject it until one generation lease spans every branch.
    PostgresCtidConformance,
    /// Reject every table lacking a declared primary key.
    RequirePrimaryKey,
}

/// Validate the absolute document base before catalogue or data I/O.
pub fn validate_direct_mapping_base(base_iri: &str) -> Result<()> {
    if base_iri.len() > MAX_DIRECT_MAPPING_BASE_IRI_BYTES_V1 {
        return mapping_error(format!(
            "Direct Mapping base IRI is {} UTF-8 bytes; maximum is {}",
            base_iri.len(),
            MAX_DIRECT_MAPPING_BASE_IRI_BYTES_V1
        ));
    }
    NamedNode::new(base_iri).map(|_| ()).map_err(|error| {
        sf_core::Error::Mapping(format!(
            "invalid Direct Mapping base IRI {base_iri:?}: {error}"
        ))
    })
}

pub(super) fn validate_direct_mapping_input(
    tables: &[TableSchema],
    base_iri: &str,
    row_identity: DirectMappingRowIdentity,
) -> Result<()> {
    validate_direct_mapping_base(base_iri)?;
    validate_generation_budget(tables, base_iri)?;
    let mut by_name = BTreeMap::new();
    for table in tables {
        if table.name.is_empty() {
            return mapping_error("Direct Mapping table name must not be empty");
        }
        if by_name.insert(table.name.as_str(), table).is_some() {
            return mapping_error(format!(
                "Direct Mapping schema contains duplicate table {:?}",
                table.name
            ));
        }
        let mut columns = BTreeSet::new();
        for column in &table.columns {
            if column.name.is_empty() {
                return mapping_error(format!(
                    "Direct Mapping table {:?} contains an empty column name",
                    table.name
                ));
            }
            if !columns.insert(column.name.as_str()) {
                return mapping_error(format!(
                    "Direct Mapping table {:?} contains duplicate column {:?}",
                    table.name, column.name
                ));
            }
        }
        validate_key_columns(table, "primary key", &table.primary_key, &columns)?;
        validate_row_identity(table, row_identity)?;
        for unique in &table.unique {
            if unique.is_empty() {
                return mapping_error(format!(
                    "Direct Mapping table {:?} contains an empty unique key",
                    table.name
                ));
            }
            validate_key_columns(table, "unique key", unique, &columns)?;
        }
    }
    for table in tables {
        validate_foreign_keys(table, &by_name)?;
    }
    Ok(())
}

fn validate_row_identity(
    table: &TableSchema,
    row_identity: DirectMappingRowIdentity,
) -> Result<()> {
    if !table.primary_key.is_empty() {
        return Ok(());
    }
    match row_identity {
        DirectMappingRowIdentity::RequirePrimaryKey => mapping_error(format!(
            "Direct Mapping table {:?} has no primary key and this backend has no admitted row identity",
            table.name
        )),
        DirectMappingRowIdentity::SqliteRowId
        | DirectMappingRowIdentity::PostgresCtidConformance
            if table
                .columns
                .iter()
                .any(|column| column.name.eq_ignore_ascii_case(SYNTHETIC_ROW_ID_COLUMN)) =>
        {
            mapping_error(format!(
                "Direct Mapping table {:?} shadows the selected synthetic rowid alias",
                table.name
            ))
        }
        DirectMappingRowIdentity::SqliteRowId
        | DirectMappingRowIdentity::PostgresCtidConformance => Ok(()),
    }
}

#[derive(Default)]
struct GenerationBudget {
    work_units: usize,
    utf8_bytes: usize,
}

impl GenerationBudget {
    fn add_work(&mut self, added: usize) -> Result<()> {
        checked_limit_add(
            &mut self.work_units,
            added,
            MAX_DIRECT_MAPPING_WORK_UNITS_V1,
            "prospective generated work units",
        )
    }

    fn add_bytes(&mut self, added: usize) -> Result<()> {
        checked_limit_add(
            &mut self.utf8_bytes,
            added,
            MAX_DIRECT_MAPPING_GENERATED_UTF8_BYTES_V1,
            "prospective generated UTF-8 bytes",
        )
    }

    fn add_scaled_bytes(&mut self, bytes: usize, copies: usize) -> Result<()> {
        let added = bytes.checked_mul(copies).ok_or_else(|| {
            limit_error(
                "prospective generated UTF-8 bytes",
                MAX_DIRECT_MAPPING_GENERATED_UTF8_BYTES_V1,
            )
        })?;
        self.add_bytes(added)
    }
}

fn checked_limit_add(current: &mut usize, added: usize, maximum: usize, label: &str) -> Result<()> {
    let next = current
        .checked_add(added)
        .filter(|next| *next <= maximum)
        .ok_or_else(|| limit_error(label, maximum))?;
    *current = next;
    Ok(())
}

fn limit_error(label: &str, maximum: usize) -> sf_core::Error {
    sf_core::Error::Mapping(format!(
        "Direct Mapping {label} exceed the finite limit of {maximum}"
    ))
}

/// Bound the IR nodes and retained string payload before constructing any
/// `TriplesMap`. The byte count follows the strings retained by the generated
/// IR; the work count covers IR nodes plus key validation that precedes it.
fn validate_generation_budget(tables: &[TableSchema], base_iri: &str) -> Result<()> {
    let mut budget = GenerationBudget::default();
    for table in tables {
        budget.add_work(3)?; // triples map, subject map, class term
        let table_encoded = prospective_encoded_len(&table.name)?;
        budget.add_bytes(table.name.len())?; // logical source
        budget.add_scaled_bytes(base_iri.len(), 2)?; // map id and class
        budget.add_scaled_bytes(table_encoded, 3)?; // id, class, subject

        if table.primary_key.is_empty() {
            budget.add_work(2)?; // literal and row-identity template segments
            budget.add_bytes(1 + SYNTHETIC_ROW_ID_COLUMN.len())?;
        } else {
            budget.add_work(1)?; // template container
            budget.add_bytes(base_iri.len())?;
            budget.add_bytes(1)?; // slash before key components
            for (index, key) in table.primary_key.iter().enumerate() {
                budget.add_work(2)?; // literal prefix and column segment
                budget.add_bytes(prospective_encoded_len(key)?)?;
                budget.add_bytes(key.len())?;
                budget.add_bytes(1 + usize::from(index != 0))?; // '=' and optional ';'
            }
        }

        for column in &table.columns {
            budget.add_work(3)?; // POM, predicate term, object term
            budget.add_bytes(base_iri.len())?;
            budget.add_bytes(table_encoded)?;
            budget.add_bytes(1)?; // '#'
            budget.add_bytes(prospective_encoded_len(&column.name)?)?;
            budget.add_bytes(column.name.len())?;
        }
        for unique in &table.unique {
            budget.add_work(1)?;
            budget.add_work(unique.len())?;
        }
        for foreign_key in &table.foreign_keys {
            budget.add_work(3)?; // POM, predicate term, reference object map
            budget.add_scaled_bytes(base_iri.len(), 2)?;
            budget.add_bytes(table_encoded)?;
            budget.add_bytes(5)?; // '#ref-'
            budget.add_bytes(prospective_encoded_len(&foreign_key.parent_table)?)?;
            if foreign_key.columns.len() > 1 {
                budget.add_bytes(foreign_key.columns.len() - 1)?;
            }
            for (child, parent) in foreign_key.columns.iter().zip(&foreign_key.parent_columns) {
                budget.add_work(1)?; // join
                budget.add_bytes(prospective_encoded_len(child)?)?;
                budget.add_bytes(child.len())?;
                budget.add_bytes(parent.len())?;
            }
        }
    }
    Ok(())
}

fn prospective_encoded_len(value: &str) -> Result<usize> {
    encoded_len(value).ok_or_else(|| {
        limit_error(
            "prospective generated UTF-8 bytes",
            MAX_DIRECT_MAPPING_GENERATED_UTF8_BYTES_V1,
        )
    })
}

fn validate_foreign_keys(table: &TableSchema, tables: &BTreeMap<&str, &TableSchema>) -> Result<()> {
    let columns = table
        .columns
        .iter()
        .map(|column| column.name.as_str())
        .collect::<BTreeSet<_>>();
    for foreign_key in &table.foreign_keys {
        if foreign_key.columns.is_empty()
            || foreign_key.columns.len() != foreign_key.parent_columns.len()
        {
            return mapping_error(format!(
                "Direct Mapping table {:?} contains a malformed foreign key",
                table.name
            ));
        }
        validate_key_columns(table, "foreign key", &foreign_key.columns, &columns)?;
        let parent = tables
            .get(foreign_key.parent_table.as_str())
            .ok_or_else(|| {
                sf_core::Error::Mapping(format!(
                    "Direct Mapping foreign key from {:?} references missing table {:?}",
                    table.name, foreign_key.parent_table
                ))
            })?;
        let parent_columns = parent
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<BTreeSet<_>>();
        validate_key_columns(
            parent,
            "foreign-key parent",
            &foreign_key.parent_columns,
            &parent_columns,
        )?;
        let referenced = foreign_key
            .parent_columns
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        if !parent.is_composite_key(&referenced) {
            return mapping_error(format!(
                "Direct Mapping foreign key from {:?} does not reference a key of {:?}",
                table.name, parent.name
            ));
        }
    }
    Ok(())
}

fn validate_key_columns(
    table: &TableSchema,
    label: &str,
    key: &[String],
    columns: &BTreeSet<&str>,
) -> Result<()> {
    let mut seen = BTreeSet::new();
    for column in key {
        if !columns.contains(column.as_str()) || !seen.insert(column.as_str()) {
            return mapping_error(format!(
                "Direct Mapping {label} for table {:?} contains an unknown or duplicate column {:?}",
                table.name, column
            ));
        }
    }
    Ok(())
}

fn mapping_error<T>(message: impl Into<String>) -> Result<T> {
    Err(sf_core::Error::Mapping(message.into()))
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn budget_accepts_exact_limits_and_rejects_the_next_unit() {
        let mut budget = GenerationBudget {
            work_units: MAX_DIRECT_MAPPING_WORK_UNITS_V1 - 1,
            utf8_bytes: MAX_DIRECT_MAPPING_GENERATED_UTF8_BYTES_V1 - 1,
        };
        budget.add_work(1).unwrap();
        budget.add_bytes(1).unwrap();
        assert!(budget.add_work(1).is_err());
        assert!(budget.add_bytes(1).is_err());
    }

    #[test]
    fn budget_arithmetic_overflow_fails_closed() {
        let mut budget = GenerationBudget {
            work_units: usize::MAX,
            utf8_bytes: usize::MAX,
        };
        assert!(budget.add_work(1).is_err());
        assert!(budget.add_bytes(1).is_err());
        assert!(GenerationBudget::default()
            .add_scaled_bytes(usize::MAX, 2)
            .is_err());
    }
}
