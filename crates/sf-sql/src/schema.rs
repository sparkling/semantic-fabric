//! Source schema model — primary keys, foreign keys, uniqueness, and
//! distinct-key cardinality (ADR-0006 *Cross-source semi-join cost*; ADR-0007
//! join elimination; ADR-0015 type determination).
//!
//! This is the *catalog read* both the SPARQL→SQL optimizer (ADR-0007 FK/PK and
//! self-join elimination) and the cross-source semi-join planner ([`crate::cost`])
//! depend on. The actual per-DBMS introspection lives in [`crate::introspect`];
//! this module owns the dialect-neutral model and the cardinality helpers.

use crate::cost::SideStats;

/// One source column and its catalog facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    /// Column name (R2RML term maps reference columns by name).
    pub name: String,
    /// The raw catalog SQL type name (feeds `sf-core`'s `DbTypeMap`, ADR-0015).
    pub sql_type: String,
    /// `NOT NULL`? (a non-nullable unique column is a true key — ADR-0007).
    pub not_null: bool,
    /// Estimated distinct values, when the catalog/statistics provide it
    /// (`sqlite_stat1`, `pg_stats.n_distinct`). `None` ⇒ unknown.
    pub distinct_estimate: Option<u64>,
}

impl Column {
    /// A column with an unknown distinct estimate.
    pub fn new(name: impl Into<String>, sql_type: impl Into<String>, not_null: bool) -> Self {
        Self {
            name: name.into(),
            sql_type: sql_type.into(),
            not_null,
            distinct_estimate: None,
        }
    }
}

/// A foreign key: `columns` of this table reference `parent_columns` of
/// `parent_table` (positionally aligned). Drives FK/PK join elimination (ADR-0007).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignKey {
    pub columns: Vec<String>,
    pub parent_table: String,
    pub parent_columns: Vec<String>,
}

/// A table's introspected schema.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableSchema {
    /// Table name as referenced by the mapping IR.
    pub name: String,
    pub columns: Vec<Column>,
    /// Primary-key column names, in key order. Empty ⇒ no declared PK.
    pub primary_key: Vec<String>,
    /// Unique constraints/indexes, each a column set (in index order).
    pub unique: Vec<Vec<String>>,
    pub foreign_keys: Vec<ForeignKey>,
    /// Estimated total rows (`pg_class.reltuples`, `sqlite_stat1` row count).
    /// `None` ⇒ no statistics (e.g. `ANALYZE` never run).
    pub row_estimate: Option<u64>,
}

impl TableSchema {
    /// An empty schema for `name`.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Look up a column by name.
    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }

    /// Is `column` a single-column unique key (the sole PK column, or a
    /// single-column unique constraint)? Such a column has one row per value, so
    /// its distinct-key cardinality equals the row count — the cleanest input to
    /// the semi-join cost model and the FK/PK join-elimination rules (ADR-0007).
    pub fn is_unique_key(&self, column: &str) -> bool {
        let pk_sole = self.primary_key.len() == 1 && self.primary_key[0] == column;
        let uniq_sole = self
            .unique
            .iter()
            .any(|cols| cols.len() == 1 && cols[0] == column);
        pk_sole || uniq_sole
    }

    /// Are `cols` a composite unique key (the exact PK or an exact UNIQUE
    /// constraint)? Order-insensitive — the set of column names must match
    /// exactly. Used for multi-column FK/PK join elimination (ADR-0007).
    pub fn is_composite_key(&self, cols: &[&str]) -> bool {
        let matches = |key: &[String]| {
            key.len() == cols.len() && cols.iter().all(|c| key.iter().any(|k| k == c))
        };
        matches(&self.primary_key) || self.unique.iter().any(|u| matches(u))
    }

    /// Best available **distinct-key cardinality** for `column` — the selectivity
    /// driver for cross-source side selection (ADR-0006).
    ///
    /// Priority: an explicit `distinct_estimate` from statistics; else, for a
    /// single-column unique key, the row estimate (every value distinct); else
    /// `None` (unknown — the caller falls back to a sketch / `EXPLAIN` probe,
    /// ADR-0006, deferred).
    pub fn distinct_key_cardinality(&self, column: &str) -> Option<u64> {
        if let Some(col) = self.column(column) {
            if let Some(d) = col.distinct_estimate {
                return Some(d);
            }
        }
        if self.is_unique_key(column) {
            return self.row_estimate;
        }
        None
    }

    /// Assemble [`SideStats`] for a join on `key_column`, for the cross-source
    /// cost planner. Returns `None` when neither distinct-key cardinality nor a
    /// row estimate is known (the planner needs at least the distinct count).
    pub fn side_stats(&self, key_column: &str) -> Option<SideStats> {
        let distinct = self.distinct_key_cardinality(key_column)?;
        let rows = self.row_estimate.unwrap_or(distinct);
        Some(SideStats::new(distinct, rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn employees() -> TableSchema {
        let mut t = TableSchema::new("employees");
        t.columns = vec![
            Column::new("id", "integer", true),
            Column::new("dept_id", "integer", true),
            Column::new("name", "text", false),
        ];
        t.primary_key = vec!["id".to_owned()];
        t.unique = vec![vec!["name".to_owned()]];
        t.foreign_keys = vec![ForeignKey {
            columns: vec!["dept_id".to_owned()],
            parent_table: "departments".to_owned(),
            parent_columns: vec!["id".to_owned()],
        }];
        t.row_estimate = Some(10_000);
        t
    }

    #[test]
    fn single_column_pk_is_a_unique_key() {
        let t = employees();
        assert!(t.is_unique_key("id"));
        assert!(t.is_unique_key("name")); // single-column unique constraint
        assert!(!t.is_unique_key("dept_id"));
    }

    #[test]
    fn unique_key_cardinality_falls_back_to_row_estimate() {
        let t = employees();
        // No explicit distinct estimate, but the PK is unique → row estimate.
        assert_eq!(t.distinct_key_cardinality("id"), Some(10_000));
        // Non-key column with no statistics → unknown.
        assert_eq!(t.distinct_key_cardinality("dept_id"), None);
    }

    #[test]
    fn explicit_distinct_estimate_wins() {
        let mut t = employees();
        t.columns[1].distinct_estimate = Some(42); // dept_id has stats
        assert_eq!(t.distinct_key_cardinality("dept_id"), Some(42));
        let stats = t.side_stats("dept_id").unwrap();
        assert_eq!(stats.distinct_keys, 42);
        assert_eq!(stats.rows, 10_000);
    }

    #[test]
    fn side_stats_unknown_without_any_estimate() {
        let mut t = TableSchema::new("t");
        t.columns = vec![Column::new("c", "integer", false)];
        assert!(t.side_stats("c").is_none());
    }
}
