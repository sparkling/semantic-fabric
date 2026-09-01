//! Compatibility facade for the dialect-neutral relational schema model.
//!
//! The value types are owned by `sf-core`; this module keeps the established
//! `sf_sql::schema::*` paths available to introspection and existing callers.

pub use sf_core::schema::{Column, ForeignKey, FunctionalDep, SideStats, TableSchema};
