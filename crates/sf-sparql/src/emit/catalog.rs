//! Introspected source columns, alias metadata and column-reference rendering.
use super::*;

/// The introspected (actual) column names of each logical source, so a mapping's
/// regular-identifier column references resolve to the column the live DBMS truly
/// exposes after its identifier folding (SQL:2008; PostgreSQL lowercases unquoted
/// names). Built by the executor from the connection ([`crate::exec`] /
/// [`crate::exec_pg`]); an empty catalog disables resolution (every reference is
/// emitted as written — the dialect-neutral [`emit_branch`] path).
#[derive(Clone, Debug, Default)]
pub struct ColumnCatalog {
    pub(super) by_source: std::sync::Arc<HashMap<String, Vec<String>>>,
    pub(super) text_by_source: std::sync::Arc<HashMap<String, HashMap<String, TextKey>>>,
    pub(super) sqlite_by_source: std::sync::Arc<HashMap<String, HashMap<String, SqliteDecode>>>,
    pub(super) scalars_by_source: std::sync::Arc<HashMap<String, HashMap<String, NativeScalarKey>>>,
    pub(super) datatypes_by_source:
        std::sync::Arc<HashMap<String, HashMap<String, sf_core::datatype::XsdTypeCode>>>,
    pub(super) suppress_path_collation: bool,
    pub(super) character_keys: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub(super) lexical_keys: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl ColumnCatalog {
    /// Record `source`'s actual result-column names (in any order).
    pub fn insert(&mut self, source: &LogicalSource, columns: Vec<String>) {
        std::sync::Arc::make_mut(&mut self.text_by_source).remove(&source_key(source));
        std::sync::Arc::make_mut(&mut self.sqlite_by_source).remove(&source_key(source));
        std::sync::Arc::make_mut(&mut self.scalars_by_source).remove(&source_key(source));
        std::sync::Arc::make_mut(&mut self.datatypes_by_source).remove(&source_key(source));
        std::sync::Arc::make_mut(&mut self.by_source).insert(source_key(source), columns);
    }

    #[cfg(test)]
    pub(crate) fn insert_live_result(
        &mut self,
        source: &LogicalSource,
        columns: Vec<sf_sql::backend::ResultColumn>,
    ) -> Result<()> {
        let datatypes = columns
            .iter()
            .filter_map(|column| {
                column
                    .natural_datatype
                    .or_else(|| column.native_scalar.and_then(natural_literal::source_code))
                    .map(|code| (column.name.clone(), code))
            })
            .collect();
        let scalars = columns
            .iter()
            .filter_map(|column| column.native_scalar.map(|key| (column.name.clone(), key)))
            .collect();
        let sqlite = columns
            .iter()
            .filter_map(|column| column.sqlite_decode.map(|key| (column.name.clone(), key)))
            .collect();
        let text = columns
            .iter()
            .filter_map(|column| column.text_key.map(|key| (column.name.clone(), key)))
            .collect();
        self.insert_live(
            source,
            columns.into_iter().map(|column| column.name).collect(),
        )?;
        std::sync::Arc::make_mut(&mut self.text_by_source).insert(source_key(source), text);
        std::sync::Arc::make_mut(&mut self.sqlite_by_source).insert(source_key(source), sqlite);
        std::sync::Arc::make_mut(&mut self.scalars_by_source).insert(source_key(source), scalars);
        std::sync::Arc::make_mut(&mut self.datatypes_by_source)
            .insert(source_key(source), datatypes);
        Ok(())
    }

    /// Record one live source's metadata, rejecting an unusable result schema
    /// without including mapping identifiers or query text in the error.
    #[cfg(test)]
    pub(crate) fn insert_live(
        &mut self,
        source: &LogicalSource,
        columns: Vec<String>,
    ) -> Result<()> {
        let mut unique = HashSet::new();
        if columns.iter().any(|column| !unique.insert(column.as_str())) {
            return Err(Error::Sql(
                "live metadata contains duplicate result-column names".to_owned(),
            ));
        }
        self.insert(source, columns);
        Ok(())
    }

    /// Add `columns` to `source`'s entry, creating it if absent, keeping any names
    /// already recorded — unlike [`insert`](Self::insert), which replaces. Two
    /// different raw columns of the same source are folded one at a time
    /// ([`synthetic_subplan_catalog`]), so a second call must not erase the first.
    pub(super) fn merge(&mut self, source: &LogicalSource, column: String) {
        let entry = std::sync::Arc::make_mut(&mut self.by_source)
            .entry(source_key(source))
            .or_default();
        if !entry.contains(&column) {
            entry.push(column);
        }
    }

    pub(super) fn columns(&self, source: &LogicalSource) -> Option<&[String]> {
        self.by_source.get(&source_key(source)).map(Vec::as_slice)
    }

    #[cfg(test)]
    pub(super) fn validate_live_column(
        &self,
        source: &LogicalSource,
        raw: &str,
        dialect: Dialect,
    ) -> Result<()> {
        let Some(columns) = self.columns(source) else {
            return Err(Error::Sql(
                "live metadata is unavailable for a required logical source".to_owned(),
            ));
        };
        let exact = columns
            .iter()
            .filter(|column| column.as_str() == raw)
            .count();
        if exact > 1 {
            return Err(Error::Sql(
                "live metadata contains duplicate result-column names".to_owned(),
            ));
        }
        if exact == 1 {
            return Ok(());
        }
        let folded = columns
            .iter()
            .filter(|column| column.eq_ignore_ascii_case(raw))
            .count();
        if folded > 1 {
            return Err(Error::Sql(
                "live metadata contains ambiguous result-column names".to_owned(),
            ));
        }
        if folded == 1 || physical_row_identifier(source, raw, dialect) {
            return Ok(());
        }
        Err(Error::Sql(
            "live metadata is missing a required result column".to_owned(),
        ))
    }
}

pub(super) fn physical_row_identifier(source: &LogicalSource, raw: &str, dialect: Dialect) -> bool {
    matches!(source, LogicalSource::Table(_))
        && raw == "rowid"
        && matches!(dialect, Dialect::Postgres | Dialect::Sqlite)
}

/// A collision-free key for a logical source (a table name can never equal an SQL
/// query, but the kind prefix makes that explicit).
pub(super) fn source_key(source: &LogicalSource) -> String {
    match source {
        LogicalSource::Table(t) => format!("t:{t}"),
        LogicalSource::Query(q) => format!("q:{q}"),
    }
}

/// Resolve a column identifier against a source's actual columns: an exact match
/// (a case-exact / delimited identifier) wins; else a unique ASCII-case-insensitive
/// match (a regular identifier folded by the DBMS); else the identifier as written
/// (no such column — the source surfaces the error). `actual = None` ⇒ unknown
/// source, emit as written.
pub(super) fn resolve_col<'a>(raw: &'a str, actual: Option<&'a [String]>) -> &'a str {
    let Some(cols) = actual else { return raw };
    if cols.iter().any(|c| c == raw) {
        return raw;
    }
    let mut folded = cols.iter().filter(|c| c.eq_ignore_ascii_case(raw));
    match (folded.next(), folded.next()) {
        (Some(c), None) => c.as_str(),
        _ => raw,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AliasSourceKind {
    Table,
    Query,
    Derived,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AliasActuals {
    // Effective source datatype is not canonical-key or output-cast authority.
    pub(super) datatype_columns: HashMap<String, Option<sf_core::datatype::XsdTypeCode>>,
    // None retains incompatible natural provenance; it must not fall back to
    // native equality after a coercing SubPlan loses a common decoder.
    pub(super) natural_columns: HashMap<String, Option<sf_core::datatype::XsdTypeCode>>,
    pub(super) scalar_columns: HashMap<String, NativeScalarKey>,
    pub(super) source_kind: AliasSourceKind,
    pub(super) columns: Vec<String>,
    pub(super) path: bool,
    pub(super) text_columns: HashMap<String, TextKey>,
    // Compiler-generated, already absolute static-template IRIs. This is not
    // inherited from arbitrary source text and is intersected at SubPlan joins.
    pub(super) static_iri_columns: HashSet<String>,
    // Bounded compiler-rendered float text uses only IRI-unreserved bytes.
    // This permits encoding elision, not native scalar or absolute-IRI authority.
    pub(super) iri_unreserved_columns: HashSet<String>,
    pub(super) sqlite_columns: HashMap<String, SqliteDecode>,
    pub(super) lexical_columns: HashMap<String, SqliteDecode>,
    // A retained decoded consumer can license an aligned RDF comparison while
    // another natural/typed consumer still forbids replacing the raw payload.
    pub(super) lexical_comparison_columns: HashMap<String, SqliteDecode>,
}

pub(super) type ActualColumns = HashMap<usize, AliasActuals>;

/// The source kind and actual columns of every scan alias in `b`, keyed by alias,
/// for physical-row handling and identifier resolution.
/// For SubPlan-join aliases, the "actual columns" are the projected variable names
/// from the nested Plan's `PlanForm::Select { vars }` (the names the derived table
/// exposes). SubPlan aliases are NOT in `alias_sources()` (they have no catalog
/// entry), so they are wired up here directly.
#[cfg(test)]
pub(super) fn branch_actuals(
    b: &Branch,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> ActualColumns {
    branch_actuals_controlled(
        b,
        dialect,
        catalog,
        sf_sql::source_work::SourceWork::new(None),
    )
    .expect("uncontrolled branch metadata construction")
}

pub(super) fn colref(c: &ColRef, dialect: Dialect, actuals: &ActualColumns) -> String {
    // The Direct Mapping no-PK blank-node identifier is keyed on the source's
    // physical row id (`sf-mapping`'s synthetic `rowid` column). SQLite exposes
    // that as the `rowid` pseudo-column; PostgreSQL has no `rowid`, so render the
    // equivalent system tuple id `ctid` cast to text (the value is an existential
    // blank-node seed — only per-row uniqueness matters, ADR-0005). This exception
    // is source-aware: an internally wrapped or mapping-authored query can expose a
    // real result column named `rowid`, which must remain an ordinary derived-table
    // column rather than being silently rewritten to that query's unrelated `ctid`.
    if dialect == Dialect::Postgres
        && c.column.as_ref() == "rowid"
        && actuals
            .get(&c.alias)
            .is_some_and(|actual| actual.source_kind == AliasSourceKind::Table)
    {
        return format!("(t{}.ctid)::text", c.alias);
    }
    let name = resolve_col(
        &c.column,
        actuals
            .get(&c.alias)
            .map(|actual| actual.columns.as_slice()),
    );
    format!("t{}.{}", c.alias, dialect.quote_ident(name))
}
