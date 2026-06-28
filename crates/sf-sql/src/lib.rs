//! `sf-sql` — the source/SQL layer (ADR-0003 core, ADR-0006).
//!
//! This crate owns everything between the virtualiser's rewriter (`sf-sparql`)
//! and a live relational source database:
//!
//! * [`dialect`] — per-DBMS [`Dialect`] selection, identifier quoting, and SQL
//!   emission through the `sqlparser` AST; injection-safe by construction
//!   (bound-parameter placeholders only, ADR-0010 §A/R1).
//! * [`schema`] — the dialect-neutral schema model (PK/FK/uniqueness) and
//!   distinct-key cardinality used by the optimizer (ADR-0007) and the
//!   cross-source cost planner.
//! * [`introspect`] — per-DBMS catalog introspection (SQLite fully; PostgreSQL
//!   integration-tested) that fills [`schema::TableSchema`].
//! * [`stream`] — bounded-memory, server-side-cursor result streaming (ADR-0006
//!   invariant; ADR-0010 §C).
//! * [`cost`] — the cross-source semi-join cost planner (a foundational,
//!   baked-in decision; ADR-0006).
//!
//! Architecture floor (ADR-0006): the **source database does the set-work**
//! (scan/join/DISTINCT/sort/spill); this crate emits SQL and streams rows back.
//! There is **no columnar/OLAP engine on the relational path** — native drivers
//! (`tokio-postgres`, `rusqlite`) and a SQL AST library (`sqlparser`) only.

pub mod cost;
pub mod dialect;
pub mod error;
pub mod introspect;
pub mod schema;
pub mod stream;

pub use cost::{plan_semijoin, CostConfig, Plan, ReducerForm, Side, SideStats};
pub use dialect::Dialect;
pub use error::{Error, Result};
pub use schema::{Column, ForeignKey, TableSchema};
pub use stream::{
    sqlite_column_decltypes, sqlite_column_names, sqlite_for_each, PgRowStream, SqliteRowStream,
};

/// A relational source the engine reads from. The dialect drives SQL emission
/// (`dialect`), introspection (`introspect`), and streaming (`stream`).
pub trait Source {
    /// The SQL dialect this source speaks.
    fn dialect(&self) -> Dialect;
}

/// Describe how a logical source binds to the SQL layer (keeps the IR dependency
/// wired for the rewriter; ADR-0006). The concrete binding is the per-dialect
/// introspection + emission in this crate.
pub fn describe(source: &sf_core::ir::LogicalSource) -> &'static str {
    match source {
        sf_core::ir::LogicalSource::Table(_) => "base table / SQL view (rr:tableName)",
        sf_core::ir::LogicalSource::Query(_) => "R2RML view (rr:sqlQuery)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Pg;
    impl Source for Pg {
        fn dialect(&self) -> Dialect {
            Dialect::Postgres
        }
    }

    #[test]
    fn source_reports_its_dialect() {
        assert_eq!(Pg.dialect(), Dialect::Postgres);
    }

    #[test]
    fn describe_distinguishes_table_from_view() {
        let table = sf_core::ir::LogicalSource::Table("emp".to_owned());
        let view = sf_core::ir::LogicalSource::Query("SELECT 1".to_owned());
        assert_ne!(describe(&table), describe(&view));
    }
}
