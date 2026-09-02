//! Authority boundary for relational compiler metadata.

use std::fmt;

use sf_sql::{Column, TableSchema};

/// Whether relational integrity constraints may authorize compiler rewrites.
///
/// The current product has no database-generation lease spanning compilation
/// through streamed execution. Consequently its only supported authority is
/// [`Unverified`](Self::Unverified): catalogue constraints are quarantined and
/// cannot authorize compiler rewrites through this schema. A future verified
/// mode requires an unforgeable backend lease, not another public enum variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConstraintAuthority {
    /// Structural catalogue observations remain useful, but integrity
    /// constraints are not stable enough to prove semantic rewrites.
    Unverified,
}

/// Whether observed SQL column types may authorize compiler decisions.
///
/// The serving product does not hold a database-generation lease from compile
/// through streamed execution, so a startup catalogue observation is always
/// [`Unverified`](Self::Unverified). A future verified serving mode needs an
/// unforgeable snapshot/lease capability; it must not be introduced as another
/// publicly constructible enum variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ColumnTypeAuthority {
    /// Type names are retained as observations, but may not prove a rewrite safe
    /// when different physical columns must remain type-compatible at execution.
    Unverified,
}

/// How one translation entry point is allowed to consume column-type facts.
///
/// The caller-authorized variant belongs only to the explicit raw translation
/// APIs, whose documentation requires a frozen or otherwise verified schema.
/// Product serving enters through [`crate::CompilerBinding`] and can obtain only
/// `Unverified`; this private distinction therefore cannot mint serving
/// authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColumnTypeUse {
    CallerAuthorizedFrozen,
    Unverified,
}

impl From<ColumnTypeAuthority> for ColumnTypeUse {
    fn from(authority: ColumnTypeAuthority) -> Self {
        match authority {
            ColumnTypeAuthority::Unverified => Self::Unverified,
        }
    }
}

/// Compiler-safe relational metadata with explicit constraint and type authority.
///
/// Raw introspection output cannot enter [`crate::CompilerBinding`] directly.
/// The only constructor deliberately removes PK, UNIQUE, FK,
/// functional-dependency, and NOT-NULL claims while retaining table/column
/// names, SQL-type observations, and estimates. Types remain available for
/// diagnostics and structurally safe decisions, but their `Unverified` authority
/// prevents them from proving compatibility between different physical columns.
pub struct CompilerSchema {
    tables: Vec<TableSchema>,
    constraint_authority: ConstraintAuthority,
    column_type_authority: ColumnTypeAuthority,
}

impl CompilerSchema {
    /// Quarantine integrity constraints from a mutable or otherwise unverified
    /// catalogue observation.
    pub fn from_unverified_observation(tables: Vec<TableSchema>) -> Self {
        Self {
            tables: tables
                .into_iter()
                .map(quarantine_unverified_constraints)
                .collect(),
            constraint_authority: ConstraintAuthority::Unverified,
            column_type_authority: ColumnTypeAuthority::Unverified,
        }
    }

    pub const fn constraint_authority(&self) -> ConstraintAuthority {
        self.constraint_authority
    }

    pub const fn column_type_authority(&self) -> ColumnTypeAuthority {
        self.column_type_authority
    }

    pub(crate) fn tables(&self) -> &[TableSchema] {
        &self.tables
    }
}

impl fmt::Debug for CompilerSchema {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompilerSchema")
            .field("constraint_authority", &self.constraint_authority)
            .field("column_type_authority", &self.column_type_authority)
            .field("table_count", &self.tables.len())
            .finish()
    }
}

/// Exhaustively reconstruct the current schema types so a future constraint
/// field cannot silently bypass quarantine without updating this function.
fn quarantine_unverified_constraints(table: TableSchema) -> TableSchema {
    let TableSchema {
        name,
        columns,
        primary_key: _,
        unique: _,
        foreign_keys: _,
        functional_dependencies: _,
        row_estimate,
    } = table;
    let columns = columns
        .into_iter()
        .map(|column| {
            let Column {
                name,
                sql_type,
                not_null: _,
                distinct_estimate,
            } = column;
            Column {
                name,
                sql_type,
                not_null: false,
                distinct_estimate,
            }
        })
        .collect();
    TableSchema {
        name,
        columns,
        primary_key: Vec::new(),
        unique: Vec::new(),
        foreign_keys: Vec::new(),
        functional_dependencies: Vec::new(),
        row_estimate,
    }
}
