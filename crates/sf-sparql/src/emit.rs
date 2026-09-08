//! Emit — render an optimized [`Branch`] to dialect SQL (ADR-0007 step 6).
//!
//! Two invariants from the substrate (ADR-0010 §A / R1, ADR-0006):
//!
//! * **Values are bound parameters only.** Every constant from the query becomes
//!   a placeholder (`?` / `$n`) and its lexical value is returned in
//!   [`EmittedBranch::params`]; nothing is interpolated into the SQL text.
//! * **AST, not string assembly.** The rendered skeleton is round-tripped through
//!   the `sqlparser` AST via [`sf_sql::Dialect::emit_via_ast`], so the emitted
//!   statement is the `Display` of a parsed tree.
//!
//! Term construction is **not** here — the SELECT projects raw key columns
//! (ADR-0007 lifting); RDF terms are built during reconstruction ([`crate::exec`]).
//!
//! **SQL identifier case-folding (SQL:2008 §5.4 / per-dialect).** An `rr:column` /
//! `rr:sqlQuery` output-column value carries the *author's* identifier. A regular
//! (unquoted) identifier is case-folded by the DBMS — PostgreSQL folds to
//! lowercase — so the mapping's `"StudentId"` must bind to the column the source
//! actually exposes (`studentid`). Each emitted column reference is therefore
//! resolved against the source's *introspected* column names ([`ColumnCatalog`]):
//! an **exact** match wins (a delimited, case-exact identifier), else a **single
//! ASCII-case-insensitive** match (the regular-identifier folding the W3C suite
//! and every dialect honour). Live execution rejects missing, duplicate, and
//! case-fold-ambiguous metadata before opening a cursor; dialect-neutral
//! [`emit_branch`] remains permissive and emits an unresolved identifier as written.
//! Reconstruction is untouched: it reads result columns by position and matches
//! the *raw* IR column strings, so only the rendered SQL text changes here.

use std::collections::{HashMap, HashSet};

use sf_core::ir::{LogicalSource, Segment, TermMap};
use sf_sql::backend::{SqliteDecode, TextKey};
use sf_sql::Dialect;

use crate::iq::{
    collect_cond_cols, AggCol, AggKind, Aggregation, Branch, ColRef, HopExpr, OrderKey,
    PathClosure, PathKind, R2rmlGraphScope, SqlCond, StrMatchOp, TermDef,
};
use crate::{Error, Result};
mod scan;
use scan::{scan_actuals, scan_ref};
mod aggregate_projection;
mod lexical_key;
mod path_comparison;
mod ref_atom;
use aggregate_projection::{aggregate_projection, AggregateProjection};
use path_comparison::{path_actuals, path_key_expression, render_key_equality, subplan_actuals};

/// The introspected (actual) column names of each logical source, so a mapping's
/// regular-identifier column references resolve to the column the live DBMS truly
/// exposes after its identifier folding (SQL:2008; PostgreSQL lowercases unquoted
/// names). Built by the executor from the connection ([`crate::exec`] /
/// [`crate::exec_pg`]); an empty catalog disables resolution (every reference is
/// emitted as written — the dialect-neutral [`emit_branch`] path).
#[derive(Clone, Debug, Default)]
pub struct ColumnCatalog {
    by_source: std::sync::Arc<HashMap<String, Vec<String>>>,
    text_by_source: std::sync::Arc<HashMap<String, HashMap<String, TextKey>>>,
    sqlite_by_source: std::sync::Arc<HashMap<String, HashMap<String, SqliteDecode>>>,
    suppress_path_collation: bool,
    character_keys: std::sync::Arc<std::sync::atomic::AtomicBool>,
    lexical_keys: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl ColumnCatalog {
    /// Record `source`'s actual result-column names (in any order).
    pub fn insert(&mut self, source: &LogicalSource, columns: Vec<String>) {
        std::sync::Arc::make_mut(&mut self.text_by_source).remove(&source_key(source));
        std::sync::Arc::make_mut(&mut self.sqlite_by_source).remove(&source_key(source));
        std::sync::Arc::make_mut(&mut self.by_source).insert(source_key(source), columns);
    }

    pub(crate) fn insert_live_result(
        &mut self,
        source: &LogicalSource,
        columns: Vec<sf_sql::backend::ResultColumn>,
    ) -> Result<()> {
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
        Ok(())
    }

    /// Record one live source's metadata, rejecting an unusable result schema
    /// without including mapping identifiers or query text in the error.
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
    fn merge(&mut self, source: &LogicalSource, column: String) {
        let entry = std::sync::Arc::make_mut(&mut self.by_source)
            .entry(source_key(source))
            .or_default();
        if !entry.contains(&column) {
            entry.push(column);
        }
    }

    fn columns(&self, source: &LogicalSource) -> Option<&[String]> {
        self.by_source.get(&source_key(source)).map(Vec::as_slice)
    }

    fn validate_live_column(
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

/// Stable, variant-tagged identity for one logical source. A table name and an
/// `rr:sqlQuery` can produce byte-identical probe SQL, but they are still distinct
/// metadata authorities and must never share a catalog entry or dedup key.
pub(crate) fn logical_source_identity(source: &LogicalSource) -> String {
    source_key(source)
}

/// Every live logical source nested in `branches`, in deterministic encounter
/// order. The executor deduplicates this list by [`logical_source_identity`]
/// before probing; keeping enumeration and identity separate makes the fail-closed
/// ordering explicit at the I/O boundary.
pub(crate) fn live_metadata_sources(branches: &[Branch]) -> Vec<&LogicalSource> {
    fn hop_sources<'a>(hop: &'a HopExpr, out: &mut Vec<&'a LogicalSource>) {
        match hop {
            HopExpr::Pred(relation) => out.push(&relation.source),
            HopExpr::Inverse(inner) => hop_sources(inner, out),
            HopExpr::Seq(left, right) => {
                hop_sources(left, out);
                hop_sources(right, out);
            }
            HopExpr::Alt(parts) | HopExpr::Nps(parts) => {
                for part in parts {
                    hop_sources(part, out);
                }
            }
        }
    }

    fn condition_sources<'a>(condition: &'a SqlCond, out: &mut Vec<&'a LogicalSource>) {
        match condition {
            SqlCond::Not(inner) => condition_sources(inner, out),
            SqlCond::And(parts) | SqlCond::Or(parts) => {
                for part in parts {
                    condition_sources(part, out);
                }
            }
            SqlCond::NotExists { scans, conds } | SqlCond::Exists { scans, conds } => {
                for scan in scans {
                    scan_sources(scan, out);
                }
                for condition in conds {
                    condition_sources(condition, out);
                }
            }
            SqlCond::PathExists { pc, conds, .. } => {
                hop_sources(&pc.hop, out);
                for condition in conds {
                    condition_sources(condition, out);
                }
            }
            _ => {}
        }
    }

    fn branch_sources<'a>(branch: &'a Branch, out: &mut Vec<&'a LogicalSource>) {
        for scan in branch
            .core
            .iter()
            .chain(branch.opts.iter().map(|join| &join.scan))
        {
            scan_sources(scan, out);
        }
        if let Some(path) = &branch.path {
            hop_sources(&path.hop, out);
        }
        for condition in &branch.where_conds {
            condition_sources(condition, out);
        }
        for join in &branch.opts {
            for condition in join.on.iter().chain(join.extra.iter()) {
                condition_sources(condition, out);
            }
        }
        for subplan in &branch.subplan_joins {
            for condition in &subplan.on {
                condition_sources(condition, out);
            }
            for inner in &subplan.plan.branches {
                branch_sources(inner, out);
            }
        }
    }

    fn scan_sources<'a>(scan: &'a crate::iq::Scan, out: &mut Vec<&'a LogicalSource>) {
        match &scan.source {
            crate::iq::ScanSource::Logical(source) => out.push(source),
            crate::iq::ScanSource::Path { closure, .. } => hop_sources(&closure.hop, out),
            crate::iq::ScanSource::Projection { input, .. } => scan_sources(input, out),
            crate::iq::ScanSource::RefAtom { input, .. } => branch_sources(input, out),
        }
    }

    let mut sources = Vec::new();
    for branch in branches {
        branch_sources(branch, &mut sources);
    }
    sources
}

/// Validate every raw base-source column that live emission can reference before
/// any branch cursor opens. Offline [`emit_branch`] remains permissive because it
/// never calls this preflight and continues to emit mapping-authored identifiers.
pub(crate) fn validate_live_columns(
    branches: &[Branch],
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> Result<()> {
    #[derive(Clone, Copy)]
    enum AliasSource<'a> {
        Base(&'a LogicalSource),
        Derived,
        Path,
        Projection(&'a [(Box<str>, TermMap)]),
        RefAtom(usize),
    }

    fn scan_alias<'a>(
        scan: &'a crate::iq::Scan,
        dialect: Dialect,
        catalog: &ColumnCatalog,
    ) -> Result<AliasSource<'a>> {
        match &scan.source {
            crate::iq::ScanSource::RefAtom { input, columns } => {
                crate::iq::scan::ref_atom::validate_shape(input, columns)?;
                validate_branch(input, dialect, catalog)?;
                Ok(AliasSource::RefAtom(columns.len()))
            }
            crate::iq::ScanSource::Logical(source) => Ok(AliasSource::Base(source)),
            crate::iq::ScanSource::Path { closure, .. } => {
                validate_hop(&closure.hop, dialect, catalog)?;
                Ok(AliasSource::Path)
            }
            crate::iq::ScanSource::Projection {
                input,
                columns,
                guards,
                ..
            } => {
                scan::validate_projection(input, columns, guards, dialect, catalog)?;
                Ok(AliasSource::Projection(columns))
            }
        }
    }

    fn validate_ref(
        column: &ColRef,
        aliases: &HashMap<usize, AliasSource<'_>>,
        dialect: Dialect,
        catalog: &ColumnCatalog,
    ) -> Result<()> {
        if let Some(AliasSource::Base(source)) = aliases.get(&column.alias) {
            catalog.validate_live_column(source, &column.column, dialect)?;
        }
        if let Some(AliasSource::Projection(columns)) = aliases.get(&column.alias) {
            scan::validate_output(columns, &column.column)?;
        }
        if let Some(AliasSource::RefAtom(width)) = aliases.get(&column.alias) {
            ref_atom::validate_output(*width, &column.column)?;
        }
        if matches!(aliases.get(&column.alias), Some(AliasSource::Path))
            && !matches!(column.column.as_ref(), "sf_s" | "sf_o")
        {
            return Err(Error::Sql(
                "path relation has no required output column".into(),
            ));
        }
        Ok(())
    }

    fn validate_hop(hop: &HopExpr, dialect: Dialect, catalog: &ColumnCatalog) -> Result<()> {
        match hop {
            HopExpr::Pred(relation) => {
                catalog.validate_live_column(&relation.source, &relation.subj_col, dialect)?;
                catalog.validate_live_column(&relation.source, &relation.obj_col, dialect)
            }
            HopExpr::Inverse(inner) => validate_hop(inner, dialect, catalog),
            HopExpr::Seq(left, right) => {
                validate_hop(left, dialect, catalog)?;
                validate_hop(right, dialect, catalog)
            }
            HopExpr::Alt(parts) | HopExpr::Nps(parts) => {
                for part in parts {
                    validate_hop(part, dialect, catalog)?;
                }
                Ok(())
            }
        }
    }

    fn validate_condition(
        condition: &SqlCond,
        aliases: &HashMap<usize, AliasSource<'_>>,
        dialect: Dialect,
        catalog: &ColumnCatalog,
    ) -> Result<()> {
        match condition {
            SqlCond::ColEq(left, right)
            | SqlCond::NativeColEq(left, right)
            | SqlCond::NullSafeEq(left, right) => {
                validate_ref(left, aliases, dialect, catalog)?;
                validate_ref(right, aliases, dialect, catalog)
            }
            SqlCond::Cmp(column, _, _)
            | SqlCond::NativeCmp(column, _, _)
            | SqlCond::IsNotNull(column)
            | SqlCond::IsNull(column)
            | SqlCond::StrMatch { col: column, .. } => {
                validate_ref(column, aliases, dialect, catalog)
            }
            SqlCond::Not(inner) => validate_condition(inner, aliases, dialect, catalog),
            SqlCond::And(parts) | SqlCond::Or(parts) => {
                for part in parts {
                    validate_condition(part, aliases, dialect, catalog)?;
                }
                Ok(())
            }
            SqlCond::NotExists { scans, conds } | SqlCond::Exists { scans, conds } => {
                // EXISTS/MINUS aliases live in a nested SQL scope. Insert them
                // only while validating that scope: flattening them into the
                // outer map can collide with a derived SubPlan alias and make an
                // outer positional `cN` look like a base-table column.
                let mut nested = aliases.clone();
                for scan in scans {
                    nested.insert(scan.alias, scan_alias(scan, dialect, catalog)?);
                }
                for condition in conds {
                    validate_condition(condition, &nested, dialect, catalog)?;
                }
                Ok(())
            }
            SqlCond::PathExists { pc, conds, .. } => {
                validate_hop(&pc.hop, dialect, catalog)?;
                for condition in conds {
                    validate_condition(condition, aliases, dialect, catalog)?;
                }
                Ok(())
            }
            SqlCond::TemplateEq(left, left_alias, right, right_alias, _) => {
                for segment in left {
                    if let Segment::Column(column) = segment {
                        validate_ref(
                            &ColRef::new(*left_alias, column.clone()),
                            aliases,
                            dialect,
                            catalog,
                        )?;
                    }
                }
                for segment in right {
                    if let Segment::Column(column) = segment {
                        validate_ref(
                            &ColRef::new(*right_alias, column.clone()),
                            aliases,
                            dialect,
                            catalog,
                        )?;
                    }
                }
                Ok(())
            }
        }
    }

    fn validate_branch(branch: &Branch, dialect: Dialect, catalog: &ColumnCatalog) -> Result<()> {
        let mut aliases = HashMap::new();
        for scan in &branch.core {
            aliases.insert(scan.alias, scan_alias(scan, dialect, catalog)?);
        }
        for join in &branch.opts {
            aliases.insert(join.scan.alias, scan_alias(&join.scan, dialect, catalog)?);
        }
        for subplan in &branch.subplan_joins {
            // The derived table exposes positional `cN` columns, not live base
            // metadata. Insert it after base scans, matching `branch_actuals`.
            aliases.insert(subplan.alias, AliasSource::Derived);
        }
        for definition in branch.bindings.values() {
            for column in definition.columns() {
                validate_ref(&column, &aliases, dialect, catalog)?;
            }
        }
        for condition in &branch.where_conds {
            validate_condition(condition, &aliases, dialect, catalog)?;
        }
        for join in &branch.opts {
            for condition in join.on.iter().chain(&join.extra) {
                validate_condition(condition, &aliases, dialect, catalog)?;
            }
        }
        for subplan in &branch.subplan_joins {
            for condition in &subplan.on {
                validate_condition(condition, &aliases, dialect, catalog)?;
            }
            for inner in &subplan.plan.branches {
                validate_branch(inner, dialect, catalog)?;
            }
        }
        if let Some(path) = &branch.path {
            validate_hop(&path.hop, dialect, catalog)?;
        }
        if let Some(aggregation) = &branch.agg {
            for key in &aggregation.keys {
                for column in &key.cols {
                    validate_ref(column, &aliases, dialect, catalog)?;
                }
            }
            for aggregate in &aggregation.aggs {
                if let Some(column) = &aggregate.arg {
                    validate_ref(column, &aliases, dialect, catalog)?;
                }
            }
        }
        Ok(())
    }

    for branch in branches {
        validate_branch(branch, dialect, catalog)?;
    }
    Ok(())
}

fn physical_row_identifier(source: &LogicalSource, raw: &str, dialect: Dialect) -> bool {
    matches!(source, LogicalSource::Table(_))
        && raw == "rowid"
        && matches!(dialect, Dialect::Postgres | Dialect::Sqlite)
}

/// Render a raw property-path endpoint from its base scan. Direct Mapping uses
/// the synthetic `rowid` key for no-primary-key table subjects; PostgreSQL's
/// equivalent is `ctid`, exactly as for ordinary [`colref`] emission.
fn path_endpoint_sql(
    source: &LogicalSource,
    raw: &str,
    source_alias: &str,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> String {
    if dialect == Dialect::Postgres && physical_row_identifier(source, raw, dialect) {
        return format!("({source_alias}.ctid)::text");
    }
    let name = resolve_col(raw, catalog.columns(source));
    format!("{source_alias}.{}", dialect.quote_ident(name))
}

/// A translate-time [`ColumnCatalog`] for offline SubPlan embedding
/// ([`crate::Plan::emitted`]). Offline derived-table SQL has no live DB connection,
/// so [`colref`]'s `actuals` lookup otherwise falls back to quoting every identifier
/// exact-case.
/// That is correct for a genuinely delimited column, but WRONG for a *regular*
/// (bare) `rr:sqlQuery` output alias: PostgreSQL folds it to lowercase at
/// declaration, so an exact-case-quoted reference to it does not exist (W3C
/// R2RMLTC0014b — `SELECT (…) AS jobTypeURI` folds to `jobtypeuri`; a downstream
/// `t5."jobTypeURI"` 42703s; confirmed live against PostgreSQL 17).
///
/// A typed walk identifies the referenced columns, then
/// [`crate::cascade::col_is_unquoted_alias`] lexically scans the owning
/// `rr:sqlQuery` for an explicit bare `AS` alias and seeds its folded name. That
/// bounded heuristic is not SQL-token/comment/string-literal aware and is therefore
/// non-authoritative. Live execution does not rely on it for base sources:
/// [`emit_subplan_sql`] overlays the recursively probed live catalog before nested
/// emission. A `LogicalSource::Table` has no alias declaration to inspect and is
/// left alone.
pub(crate) fn synthetic_subplan_catalog(branches: &[Branch]) -> ColumnCatalog {
    let mut catalog = ColumnCatalog::default();
    for b in branches {
        let sources: HashMap<usize, &LogicalSource> = b.alias_sources().into_iter().collect();
        let mut cols: Vec<ColRef> = Vec::new();
        for def in b.bindings.values() {
            cols.extend(def.columns());
        }
        for cond in &b.where_conds {
            collect_cond_cols(cond, &mut |c| cols.push(c.clone()));
        }
        for opt in &b.opts {
            for cond in opt.on.iter().chain(opt.extra.iter()) {
                collect_cond_cols(cond, &mut |c| cols.push(c.clone()));
            }
        }
        for sp in &b.subplan_joins {
            for cond in &sp.on {
                collect_cond_cols(cond, &mut |c| cols.push(c.clone()));
            }
        }
        for c in &cols {
            let Some(LogicalSource::Query(sql)) = sources.get(&c.alias) else {
                continue;
            };
            if crate::cascade::col_is_unquoted_alias(sql, &c.column) {
                catalog.merge(sources[&c.alias], c.column.to_lowercase());
            }
        }
    }
    catalog
}

/// A collision-free key for a logical source (a table name can never equal an SQL
/// query, but the kind prefix makes that explicit).
fn source_key(source: &LogicalSource) -> String {
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
fn resolve_col<'a>(raw: &'a str, actual: Option<&'a [String]>) -> &'a str {
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
enum AliasSourceKind {
    Table,
    Query,
    Derived,
}

#[derive(Clone, Debug)]
struct AliasActuals {
    source_kind: AliasSourceKind,
    columns: Vec<String>,
    path: bool,
    text_columns: HashMap<String, TextKey>,
    sqlite_columns: HashMap<String, SqliteDecode>,
    lexical_columns: HashMap<String, SqliteDecode>,
}

type ActualColumns = HashMap<usize, AliasActuals>;

fn source_actuals(source: &LogicalSource, catalog: &ColumnCatalog) -> AliasActuals {
    AliasActuals {
        sqlite_columns: catalog
            .sqlite_by_source
            .get(&source_key(source))
            .cloned()
            .unwrap_or_default(),
        lexical_columns: HashMap::new(),
        source_kind: match source {
            LogicalSource::Table(_) => AliasSourceKind::Table,
            LogicalSource::Query(_) => AliasSourceKind::Query,
        },
        columns: catalog.columns(source).unwrap_or_default().to_vec(),
        path: false,
        text_columns: catalog
            .text_by_source
            .get(&source_key(source))
            .cloned()
            .unwrap_or_default(),
    }
}

/// The source kind and actual columns of every scan alias in `b`, keyed by alias,
/// for physical-row handling and identifier resolution.
/// For SubPlan-join aliases, the "actual columns" are the projected variable names
/// from the nested Plan's `PlanForm::Select { vars }` (the names the derived table
/// exposes). SubPlan aliases are NOT in `alias_sources()` (they have no catalog
/// entry), so they are wired up here directly.
fn branch_actuals(b: &Branch, dialect: Dialect, catalog: &ColumnCatalog) -> ActualColumns {
    let mut out = HashMap::new();
    for (alias, source) in b.alias_sources() {
        out.insert(alias, source_actuals(source, catalog));
    }
    for scan in b.core.iter().chain(b.opts.iter().map(|join| &join.scan)) {
        out.insert(scan.alias, scan_actuals(scan, dialect, catalog));
    }
    if let Some(path) = &b.path {
        out.insert(path.alias, path_actuals(path, catalog));
    }
    // SubPlan derived-table aliases: their columns are the positional names the
    // inner `emit_branch` assigns (`c0`, `c1`, …), NOT the SPARQL variable names.
    // The outer branch's bindings use `ColRef(sp_alias, "c{i}")` after remapping.
    for sp in &b.subplan_joins {
        out.insert(sp.alias, subplan_actuals(&sp.plan, dialect, catalog));
    }
    out
}

/// A branch rendered to one parameterised SQL `SELECT`.
pub struct EmittedBranch {
    pub sql: String,
    /// Prepare-only same-IR SQL without engine-added collation decorations.
    /// Never executed; preserves SQLite native result decoding through COLLATE.
    pub metadata_sql: Option<String>,
    /// Engine-generated decoder call, never inferred from authored SQL text.
    pub sqlite_character_keys: bool,
    pub sqlite_lexical_keys: bool,
    /// The result-set schema: column `i` is `projection[i]` (positional — the
    /// reconstruction reads by position, not by the cosmetic `AS c{i}` label).
    pub projection: Vec<ColRef>,
    /// Bound parameter values, in placeholder order.
    pub params: Vec<String>,
}

/// Render one branch with no column-name resolution (the dialect-neutral path,
/// used where a live catalog is unavailable — every identifier emitted as written).
pub fn emit_branch(b: &Branch, dialect: Dialect) -> Result<EmittedBranch> {
    emit_branch_with(b, dialect, &ColumnCatalog::default())
}

/// Render one branch, resolving each column reference against `catalog` so a
/// mapping's regular identifiers bind to the columns the live source exposes after
/// its identifier folding (see the module docs).
pub fn emit_branch_with(
    b: &Branch,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> Result<EmittedBranch> {
    let mut emission_catalog = catalog.clone();
    emission_catalog.character_keys = Default::default();
    emission_catalog.lexical_keys = Default::default();
    let catalog = &emission_catalog;
    let mut emitted = emit_branch_inner(b, dialect, catalog)?;
    if dialect == Dialect::Sqlite && !catalog.suppress_path_collation {
        let mut metadata_catalog = catalog.clone();
        metadata_catalog.suppress_path_collation = true;
        let metadata = emit_branch_inner(b, dialect, &metadata_catalog)?;
        if metadata.projection != emitted.projection || metadata.params != emitted.params {
            return Err(Error::Sql(
                "path metadata twin changed projection or parameters".into(),
            ));
        }
        if metadata.sql != emitted.sql {
            emitted.metadata_sql = Some(metadata.sql);
        }
    }
    emitted.sqlite_character_keys = catalog
        .character_keys
        .load(std::sync::atomic::Ordering::Relaxed);
    emitted.sqlite_lexical_keys = catalog
        .lexical_keys
        .load(std::sync::atomic::Ordering::Relaxed);
    Ok(emitted)
}

fn emit_branch_inner(
    b: &Branch,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> Result<EmittedBranch> {
    emit_branch_keys(b, dialect, catalog, b.distinct)
}

fn emit_branch_keys(
    b: &Branch,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    normalize_projection: bool,
) -> Result<EmittedBranch> {
    let actuals = branch_actuals(b, dialect, catalog);
    if let Some(pc) = &b.path {
        return emit_path_branch(b, pc, dialect, catalog);
    }
    if let Some(agg) = &b.agg {
        return emit_agg_branch(b, agg, dialect, catalog, &actuals);
    }
    // ADR-0025 (C.3): SQL `DISTINCT` dedups RAW columns, so it implements SPARQL DISTINCT
    // (dedup on the RECONSTRUCTED term) only when every projected term is INJECTIVE in its
    // raw columns. A non-injective template (distinct raw tuples → the same RDF term, e.g.
    // `http://ex/{a}{b}` over `(1,23)`/`(12,3)`) would survive SQL DISTINCT as duplicates
    // SPARQL must collapse — a silent wrong answer. Sound 501 — UNLESS `b` also qualifies
    // for Run 4 Wave C0d's term-level dedup ([`crate::cascade::eligible_for_term_dedup`]):
    // a branch whose relation stands alone in its plan slot answers instead of refusing —
    // `term_dedup` below suppresses the SQL `DISTINCT` this function would otherwise emit,
    // leaving the raw, duplicate-bearing rows for `exec_core::run_branches` to dedup AFTER
    // reconstruction, on the actual term values. Injective templates need neither path —
    // SQL DISTINCT already implements SPARQL DISTINCT for them.
    let term_dedup = crate::cascade::eligible_for_term_dedup(b);
    if b.distinct && !term_dedup {
        for def in b.bindings.values() {
            if !crate::cascade::binding_is_injective(def) {
                return Err(Error::Unsupported(
                    "SELECT DISTINCT over a non-injective term (a multi-column template that \
                     maps distinct raw tuples to the same RDF term) cannot be pushed to SQL \
                     DISTINCT soundly → 501 (ADR-0025 C.3)"
                        .to_owned(),
                ));
            }
        }
    }
    let projection = b.projection();
    let mut params = Vec::new();
    let mut pidx = 0usize;

    // FROM (+ LEFT JOIN ON params) MUST render before WHERE so positional
    // placeholders bind in text order. A core-less branch with no SubPlan joins
    // AND no opts (an inline `VALUES` constant row — all `Const` bindings)
    // renders as a one-row `SELECT <const exprs>` with no FROM. A core-less
    // branch WITH SubPlan joins (ADR-0023 M5 Wave 2: SubPlan anchor) or opts (a
    // core-less `LeftJoin` left side, e.g. `{} OPTIONAL {...}`) needs
    // `render_from`'s synthetic/SubPlan anchor — omitting it left the opt's own
    // columns referenced with no FROM clause ever introducing their alias.
    let from = if b.core.is_empty() && b.subplan_joins.is_empty() && b.opts.is_empty() {
        None
    } else {
        Some(render_from(
            b,
            dialect,
            catalog,
            &actuals,
            &mut params,
            &mut pidx,
        )?)
    };
    let where_sql = render_where(
        &b.where_conds,
        dialect,
        catalog,
        &actuals,
        &mut params,
        &mut pidx,
    )?;

    let select_list = if projection.is_empty() {
        "1 AS c0".to_owned()
    } else {
        projection
            .iter()
            .enumerate()
            .map(|(i, c)| {
                format!(
                    "{} AS c{i}",
                    if normalize_projection && !term_dedup {
                        path_comparison::rdf_column(c, dialect, catalog, &actuals)
                    } else {
                        colref(c, dialect, &actuals)
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };

    // `term_dedup` skips SQL DISTINCT even though `b.distinct` is set (see the C.3 gate
    // above) — its raw, non-injective duplicates are collapsed downstream, by TERM, not
    // by raw-column SQL DISTINCT (which would be the unsound operation C.3 refuses).
    let distinct = if b.distinct && !term_dedup {
        "DISTINCT "
    } else {
        ""
    };
    let mut skeleton = match from {
        Some(f) => format!("SELECT {distinct}{select_list} FROM {f}"),
        None => format!("SELECT {distinct}{select_list}"),
    };
    if let Some(w) = where_sql {
        skeleton.push_str(" WHERE ");
        skeleton.push_str(&w);
    }
    // ORDER BY precedes LIMIT/OFFSET (SPARQL §15: order, then slice).
    if let Some(order) = render_order(
        &b.order,
        b,
        dialect,
        catalog,
        &actuals,
        normalize_projection && !term_dedup,
    )? {
        skeleton.push_str(&order);
    }
    push_limit_offset(&mut skeleton, b, dialect);

    let sql = dialect
        .emit_via_ast(&skeleton)
        .map_err(|e| Error::Sql(e.to_string()))?;
    Ok(EmittedBranch {
        sql,
        metadata_sql: None,
        sqlite_character_keys: false,
        sqlite_lexical_keys: false,
        projection,
        params,
    })
}

/// Push a `LIMIT`/`OFFSET` tail onto `skeleton` (called after `WHERE`/`ORDER BY`
/// have already been rendered, per each caller's own SQL clause order). A BARE
/// `OFFSET` (no `LIMIT`) is a genuine SPARQL shape (`OFFSET n` with no `LIMIT`
/// is valid syntax) but not every dialect's grammar accepts a standalone
/// `OFFSET` clause — `Dialect::bare_offset_limit_sentinel` renders an explicit
/// "no limit" `LIMIT` first for the dialects that need one (confirmed live: a
/// bare `OFFSET n` is a SQLite/MySQL syntax error, so this genuinely fixed a
/// live-emission failure, not a hypothetical one).
fn push_limit_offset(skeleton: &mut String, b: &Branch, dialect: Dialect) {
    match b.limit {
        Some(limit) => skeleton.push_str(&format!(" LIMIT {limit}")),
        None if b.offset > 0 => {
            if let Some(sentinel) = dialect.bare_offset_limit_sentinel() {
                skeleton.push_str(&format!(" LIMIT {sentinel}"));
            }
        }
        None => {}
    }
    if b.offset > 0 {
        skeleton.push_str(&format!(" OFFSET {}", b.offset));
    }
}

/// Render an `ORDER BY` clause that pins the SPARQL value-space order, or `None`
/// when there are no keys. Each key is a bound variable; its `rr:column` term map
/// lowers to a raw column whose SQL order equals the RDF lexical/value order (a
/// homogeneous literal column, or an IRI whose value *is* the column). NULL
/// placement is **always explicit** — `NULLS FIRST` for ASC, `NULLS LAST` for DESC
/// — never the dialect default (PostgreSQL: NULLS LAST; SQLite: NULLS FIRST), so an
/// unbound key sorts first (ASC) / last (DESC) on every engine. A key bound to a
/// non-`rr:column` term (a constructed template IRI, COALESCE, …) cannot be ordered
/// soundly in SQL (the constructed string ≠ the column order) → honest 501.
fn render_order(
    order: &[OrderKey],
    b: &Branch,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    normalize: bool,
) -> Result<Option<String>> {
    if order.is_empty() {
        return Ok(None);
    }
    let mut terms = Vec::with_capacity(order.len());
    for key in order {
        let def = b.bindings.get(&key.var).ok_or_else(|| {
            Error::Unsupported(format!(
                "ORDER BY ?{} is not a bound variable → 501",
                key.var
            ))
        })?;
        let col = order_column(def).ok_or_else(|| {
            Error::Unsupported(format!(
                "ORDER BY ?{} is not an rr:column term — a constructed/derived sort \
                 key cannot be ordered soundly in SQL → 501",
                key.var
            ))
        })?;
        let dir = if key.descending {
            "DESC NULLS LAST"
        } else {
            "ASC NULLS FIRST"
        };
        terms.push(format!(
            "{} {dir}",
            if normalize {
                path_comparison::rdf_column(&col, dialect, catalog, actuals)
            } else {
                colref(&col, dialect, actuals)
            }
        ));
    }
    Ok(Some(format!(" ORDER BY {}", terms.join(", "))))
}

/// The single raw column an ORDER BY key lowers to, iff the key is a bound
/// `rr:column` term map (the SQL-order-sound case). A template / COALESCE / CONCAT /
/// constant has no such column → `None` (the caller defers to 501).
fn order_column(def: &TermDef) -> Option<ColRef> {
    match def {
        TermDef::Derived {
            term_map: TermMap::Column(c, _),
            alias,
        } => Some(ColRef::new(*alias, c.clone())),
        TermDef::R2rmlBlank {
            term_map: TermMap::Column(c, _),
            alias,
            graph:
                R2rmlGraphScope::Default
                | R2rmlGraphScope::Mapped {
                    term_map: TermMap::Constant(_),
                    ..
                },
        } => Some(ColRef::new(*alias, c.clone())),
        _ => None,
    }
}

/// Render a property-path closure branch to a (possibly `WITH RECURSIVE`) CTE
/// (ADR-0007 *recursive paths compile to source-dialect recursive CTEs*).
///
/// The hop relation ([`HopExpr`]) compiles to a subquery yielding the canonical
/// **raw key columns** `sf_s` / `sf_o` (term-gen lifting; see [`hop_sql`]): a bare
/// predicate is a base scan, and `^p`/`p/q`/`p|q`/`!p` are nested subqueries over
/// the same keys. A second relation reduces it to the distinct reachable node
/// pairs the outer projection reads as `t{alias}` — SPARQL paths are set-semantics
/// over node **pairs**, so this `SELECT DISTINCT sf_s, sf_o` is what keeps
/// `SELECT ?s WHERE {?s P ?o}` a correct bag of `?s` even when `?o` is dropped.
///
/// Per [`PathKind`]:
/// * `One` (`^p`, `p/q`, `p|q`) — just the distinct hop pairs, no recursion; `!p`
///   (NPS) is the bag exception — its `UNION ALL` hop is kept un-`DISTINCT`ed.
/// * `ZeroOrOne` (`p?`) — the hop ∪ the reflexive `(x, x)` pairs over the active
///   graph's nodes (only over a single-predicate bare leaf, `unfold`-enforced).
/// * `OneOrMore` (`P+`) — a `WITH RECURSIVE` finite-pair fixed point. Its `UNION`
///   deduplicates on `(sf_s, sf_o)`, so cyclic revisits are discarded and recursion
///   ends only when no new reachable pair exists. This is exact for a finite source
///   relation; no successful result is cut off at an arbitrary walk depth.
/// * `ZeroOrMore` (`P*`) — `OneOrMore` plus the reflexive `(x, x)` pairs.
///
/// RDF terms are still built only at the outer projection ([`crate::exec`]).
/// The `WITH …` prelude defining the path closure's distinct-pairs relation
/// `t{alias}(sf_s, sf_o)` — a plain CTE for length-1 shapes, a `WITH RECURSIVE` for `+`/`*`.
/// Shared by [`emit_path_branch`] (a standalone path result) and the `PathExists`
/// correlated-EXISTS emission (ADR-0025 Tier-2 gap 1); both reference `t{alias}.sf_s`/`.sf_o`.
fn path_with_prelude(
    pc: &PathClosure,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> Result<String> {
    let cte = format!("t{}", pc.alias);
    let hop = hop_sql(&pc.hop, dialect, catalog);
    let (sf_s, sf_o) = (dialect.quote_ident("sf_s"), dialect.quote_ident("sf_o"));
    Ok(match pc.kind {
        PathKind::One => {
            let one_distinct = if matches!(pc.hop, HopExpr::Nps(_)) {
                ""
            } else {
                "DISTINCT "
            };
            format!(
                "WITH {cte}({sf_s}, {sf_o}) AS \
                 (SELECT {one_distinct}{sf_s}, {sf_o} FROM ({hop}) hx)"
            )
        }
        PathKind::ZeroOrOne => {
            let refl = reflexive_sql(&pc.hop, dialect, catalog)?;
            format!(
                "WITH {cte}({sf_s}, {sf_o}) AS (SELECT DISTINCT {sf_s}, {sf_o} FROM \
                 (SELECT {sf_s}, {sf_o} FROM ({hop}) hx UNION {refl}) z)"
            )
        }
        PathKind::OneOrMore | PathKind::ZeroOrMore => {
            let cte_raw = format!("t{}r", pc.alias);
            let one_hop = format!("SELECT {sf_s}, {sf_o} FROM ({hop}) hx");
            let anchor = if matches!(pc.kind, PathKind::ZeroOrMore) {
                let refl = reflexive_sql(&pc.hop, dialect, catalog)?;
                format!("{one_hop} UNION {refl}")
            } else {
                one_hop
            };
            let recursive = format!(
                "SELECT c.{sf_s} AS {sf_s}, h.{sf_o} AS {sf_o} \
                 FROM {cte_raw} c JOIN ({hop}) h ON c.{sf_o} = h.{sf_s}"
            );
            format!(
                "WITH RECURSIVE {cte_raw}({sf_s}, {sf_o}) AS ({anchor} UNION {recursive}), \
                 {cte}({sf_s}, {sf_o}) AS (SELECT DISTINCT {sf_s}, {sf_o} FROM {cte_raw})"
            )
        }
    })
}

/// Render a path closure as a self-contained derived-table SQL string:
/// `{with} SELECT sf_s, sf_o FROM t{cte_alias}` (ADR-0033). `cte_alias` is a
/// FRESH alias for the closure's OWN internal CTE naming, distinct from
/// `pc.alias` — the caller ([`crate::iq::lower::convert_path_branches`]) keeps
/// `pc.alias` as the OUTER `Scan`'s alias, so every pre-existing
/// `TermDef::Derived{alias: pc.alias, column: "sf_s"/"sf_o"}` binding keeps
/// resolving unchanged against this derived table's identically-named output
/// columns — zero cross-tree rewriting. Reuses [`path_with_prelude`] verbatim
/// (only the closure's `alias` is rebased to `cte_alias` first).
pub(crate) fn path_as_derived_table_sql(
    pc: &PathClosure,
    cte_alias: usize,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> Result<String> {
    let mut inner = pc.clone();
    inner.alias = cte_alias;
    let with = path_with_prelude(&inner, dialect, catalog)?;
    let (sf_s, sf_o) = (dialect.quote_ident("sf_s"), dialect.quote_ident("sf_o"));
    Ok(format!("{with} SELECT {sf_s}, {sf_o} FROM t{cte_alias}"))
}

fn emit_path_branch(
    b: &Branch,
    pc: &PathClosure,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> Result<EmittedBranch> {
    // ORDER BY over a path result is handled at the exec layer (plan.order →
    // Rust-level order_cmp) — Branch.order is always empty here, so no guard needed.
    let projection = b.projection();
    let mut params = Vec::new();
    let mut pidx = 0usize;

    // `cte` is the distinct-pairs relation the outer projection reads (colref binds
    // `t{alias}`). Its columns are the canonical `sf_s` / `sf_o` keys, never base
    // columns, so the outer projection / WHERE resolve against an empty catalog.
    let cte = format!("t{}", pc.alias);
    let outer_actuals = HashMap::from([(pc.alias, path_actuals(pc, catalog))]);
    let with = path_with_prelude(pc, dialect, catalog)?;

    let select_list = projection
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{} AS c{i}", colref(c, dialect, &outer_actuals)))
        .collect::<Vec<_>>()
        .join(", ");

    let distinct = if b.distinct { "DISTINCT " } else { "" };
    let mut skeleton = format!("{with} SELECT {distinct}{select_list} FROM {cte}");
    if let Some(w) = render_where(
        &b.where_conds,
        dialect,
        catalog,
        &outer_actuals,
        &mut params,
        &mut pidx,
    )? {
        skeleton.push_str(" WHERE ");
        skeleton.push_str(&w);
    }
    push_limit_offset(&mut skeleton, b, dialect);

    let sql = dialect
        .emit_via_ast(&skeleton)
        .map_err(|e| Error::Sql(e.to_string()))?;
    Ok(EmittedBranch {
        sql,
        metadata_sql: None,
        sqlite_character_keys: false,
        sqlite_lexical_keys: false,
        projection,
        params,
    })
}

/// Render a GROUP BY + aggregates branch (SPARQL §11) to one parameterised SQL
/// `SELECT … GROUP BY …`. The grouping keys are their **raw key columns** (term
/// construction is rebuilt at reconstruction, ADR-0007), grouped and projected;
/// each aggregate is `COUNT`/`SUM`/`AVG`/`MIN`/`MAX(…)` over its single raw column
/// (or `COUNT(*)`). Implicit grouping (no keys) emits no `GROUP BY`, yielding one
/// row over all inner rows — and one row even when the inner is empty (`COUNT(*)`
/// = 0). The projection is built in lockstep with the SELECT list so reconstruction
/// reads each key/aggregate by position. The aggregate result columns are synthetic
/// (computed in SQL), so their `§10` type is set explicitly at reconstruction (see
/// [`TermDef::Agg`]), never read from a base column.
fn emit_agg_branch(
    b: &Branch,
    agg: &Aggregation,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
) -> Result<EmittedBranch> {
    let mut params = Vec::new();
    let mut pidx = 0usize;

    // FROM (+ LEFT JOIN ON params) renders before WHERE so positional placeholders
    // bind in text order. A core-less inner with no SubPlan join and no opts
    // either (an empty BGP) renders without FROM; a core-less inner WITH a
    // SubPlan join (the SQL agg-over-UNION pushdown: the pooled arms' derived
    // table is the sole FROM relation) or opts (aggregating over `{} OPTIONAL
    // {...}}`) still needs `render_from` — mirrors `emit_branch_with`'s condition.
    let from = if b.core.is_empty() && b.subplan_joins.is_empty() && b.opts.is_empty() {
        None
    } else {
        Some(render_from(
            b,
            dialect,
            catalog,
            actuals,
            &mut params,
            &mut pidx,
        )?)
    };
    let where_sql = render_where(
        &b.where_conds,
        dialect,
        catalog,
        actuals,
        &mut params,
        &mut pidx,
    )?;

    // The projection + SELECT list, in lockstep: grouping-key raw columns first
    // (also the GROUP BY columns), then each aggregate expression.
    let layout = aggregate_projection(agg, dialect);
    let projection: Vec<ColRef> = layout.iter().map(|item| item.column().clone()).collect();
    let mut select_items: Vec<String> = Vec::new();
    let mut group_cols: Vec<String> = Vec::new();
    for (i, item) in layout.iter().enumerate() {
        let expression = match item {
            AggregateProjection::Key(column) => {
                let rendered = colref(column, dialect, actuals);
                group_cols.push(rendered.clone());
                rendered
            }
            AggregateProjection::Aggregate(aggregate) => agg_expr_sql(aggregate, dialect, actuals),
            AggregateProjection::AvgOperand(column) => colref(column, dialect, actuals),
        };
        select_items.push(format!("{expression} AS c{i}"));
    }
    let select_list = select_items.join(", ");

    let distinct = if b.distinct { "DISTINCT " } else { "" };
    let mut skeleton = match from {
        Some(f) => format!("SELECT {distinct}{select_list} FROM {f}"),
        None => format!("SELECT {distinct}{select_list}"),
    };
    if let Some(w) = where_sql {
        skeleton.push_str(" WHERE ");
        skeleton.push_str(&w);
    }
    if !group_cols.is_empty() {
        skeleton.push_str(" GROUP BY ");
        skeleton.push_str(&group_cols.join(", "));
    }
    // ORDER BY over an aggregate result is applied in `exec` (never pushed to SQL —
    // it sorts the reconstructed terms type-aware). LIMIT/OFFSET on a single agg
    // branch were pushed by `Plan::prepared_branches` only when unordered.
    push_limit_offset(&mut skeleton, b, dialect);

    let sql = dialect
        .emit_via_ast(&skeleton)
        .map_err(|e| Error::Sql(e.to_string()))?;
    Ok(EmittedBranch {
        sql,
        metadata_sql: None,
        sqlite_character_keys: false,
        sqlite_lexical_keys: false,
        projection,
        params,
    })
}

/// The SQL for one aggregate output column (`COUNT(*)` / `COUNT`/`SUM`/`AVG`/`MIN`/
/// `MAX(<col>)`, with optional `DISTINCT`). `MIN`/`MAX` ignore DISTINCT (it never
/// changes the extremum), but it is rendered when requested for faithfulness.
fn agg_expr_sql(a: &AggCol, dialect: Dialect, actuals: &ActualColumns) -> String {
    let d = if a.distinct { "DISTINCT " } else { "" };
    let func = match a.kind {
        AggKind::Count => "COUNT",
        AggKind::Sum => "SUM",
        AggKind::Avg => "AVG",
        AggKind::Min => "MIN",
        AggKind::Max => "MAX",
    };
    match &a.arg {
        // COUNT(*) — the only argument-less form (DISTINCT is rejected upstream).
        None => format!("{func}(*)"),
        Some(col) => format!("{func}({d}{})", colref(col, dialect, actuals)),
    }
}

/// Render a [`HopExpr`] to a relation expression yielding the canonical raw key
/// columns `sf_s` / `sf_o` (term-construction lifting, ADR-0007): a bare predicate
/// is a base scan; `^p` swaps the keys; `p/q` joins on the middle node; `p|q` /
/// `!p` set-union the pairs. The result is a `SELECT …` body to be wrapped in
/// `(…) alias` by the caller. Leaf base-column references are resolved against the
/// live catalog (SQL:2008 identifier folding; see the module docs).
///
/// `HopExpr::Pred` is the ONLY arm that reads raw base columns directly, so it is
/// the ONLY arm that needs a NULL guard: R2RML §11 generates no triple at all when
/// a referenced column is NULL (matching what the non-path `atom()` triple-pattern
/// emission already enforces via its `obj_null_guard`, `unfold.rs`) — without the
/// guard, a NULL-valued subject or object column becomes a phantom one-hop pair
/// that a recursive closure then chains through transitively, poisoning every node
/// that can reach it. Every composite arm (`Inverse`/`Seq`/`Alt`/`Nps`) only ever
/// recomposes an already-guarded inner `hop_sql`'s `sf_s`/`sf_o`, so the leaf guard
/// alone makes every composite sound too — no separate guard needed there.
fn hop_sql(hop: &HopExpr, dialect: Dialect, catalog: &ColumnCatalog) -> String {
    let (sf_s, sf_o) = (dialect.quote_ident("sf_s"), dialect.quote_ident("sf_o"));
    match hop {
        HopExpr::Pred(rel) => {
            let src = source_sql(&rel.source, dialect);
            let s = path_endpoint_sql(&rel.source, rel.subj_col.as_ref(), "h0", dialect, catalog);
            let o = path_endpoint_sql(&rel.source, rel.obj_col.as_ref(), "h0", dialect, catalog);
            let s = path_key_expression(s, &rel.source, &rel.subj_col, dialect, catalog);
            let o = path_key_expression(o, &rel.source, &rel.obj_col, dialect, catalog);
            format!(
                "SELECT {s} AS {sf_s}, {o} AS {sf_o} FROM {src} h0 \
                 WHERE {s} IS NOT NULL AND {o} IS NOT NULL"
            )
        }
        HopExpr::Inverse(inner) => {
            let inner_sql = hop_sql(inner, dialect, catalog);
            format!("SELECT x.{sf_o} AS {sf_s}, x.{sf_s} AS {sf_o} FROM ({inner_sql}) x")
        }
        HopExpr::Seq(a, b) => {
            let a_sql = hop_sql(a, dialect, catalog);
            let b_sql = hop_sql(b, dialect, catalog);
            format!(
                "SELECT a.{sf_s} AS {sf_s}, b.{sf_o} AS {sf_o} \
                 FROM ({a_sql}) a JOIN ({b_sql}) b ON a.{sf_o} = b.{sf_s}"
            )
        }
        HopExpr::Alt(parts) => parts
            .iter()
            .map(|p| {
                let psql = hop_sql(p, dialect, catalog);
                format!("SELECT {sf_s}, {sf_o} FROM ({psql}) u")
            })
            .collect::<Vec<_>>()
            .join(" UNION "),
        // NPS carries BAG semantics (one solution per matching triple): `UNION ALL`
        // over the per-predicate DISTINCT pairs so a pair connected by two
        // complement predicates yields two rows (matching the oracle), while a
        // duplicate row WITHIN one predicate (the same virtual triple) stays
        // collapsed by that predicate's `DISTINCT`. The `PathKind::One` wrapper
        // omits its outer `DISTINCT` to preserve this bag (see `emit_path_branch`).
        HopExpr::Nps(parts) => parts
            .iter()
            .map(|p| {
                let psql = hop_sql(p, dialect, catalog);
                format!("SELECT DISTINCT {sf_s}, {sf_o} FROM ({psql}) u")
            })
            .collect::<Vec<_>>()
            .join(" UNION ALL "),
    }
}

/// The reflexive `(x, x)` pairs over a bare-predicate hop's node set (its subjects
/// ∪ objects) — the ZeroLengthPath component of `P*` / `p?`. `unfold` only emits
/// reflexive kinds over a single-predicate bare leaf, so a composite hop here is a
/// programming error surfaced as 501.
///
/// Reads `subj_col`/`obj_col` directly (like `hop_sql`'s `HopExpr::Pred` leaf, but
/// NOT through it), so it needs the identical NULL guard: a row where EITHER
/// column is NULL generates no triple at all (R2RML §11), hence contributes
/// NEITHER a subject-node NOR an object-node to this predicate's graph — without
/// the guard, a NULL-valued column would seed a phantom `(NULL, NULL)` reflexive
/// pair. Both `UNION` halves share the SAME row-level guard (not a per-column
/// guard on just the column each half projects): a row failing the OTHER column's
/// NULL check still generates no triple, so its own column is not a valid node
/// either.
fn reflexive_sql(hop: &HopExpr, dialect: Dialect, catalog: &ColumnCatalog) -> Result<String> {
    let rel = hop.as_pred().ok_or_else(|| {
        Error::Unsupported("reflexive (P*/p?) path over a composite hop → 501".to_owned())
    })?;
    let src = source_sql(&rel.source, dialect);
    let s = path_endpoint_sql(&rel.source, rel.subj_col.as_ref(), "h0", dialect, catalog);
    let o = path_endpoint_sql(&rel.source, rel.obj_col.as_ref(), "h0", dialect, catalog);
    let s = path_key_expression(s, &rel.source, &rel.subj_col, dialect, catalog);
    let o = path_key_expression(o, &rel.source, &rel.obj_col, dialect, catalog);
    let (sf_s, sf_o) = (dialect.quote_ident("sf_s"), dialect.quote_ident("sf_o"));
    Ok(format!(
        "SELECT {s} AS {sf_s}, {s} AS {sf_o} FROM {src} h0 \
         WHERE {s} IS NOT NULL AND {o} IS NOT NULL \
         UNION SELECT {o} AS {sf_s}, {o} AS {sf_o} FROM {src} h0 \
         WHERE {s} IS NOT NULL AND {o} IS NOT NULL"
    ))
}

/// A base source rendered **without** a binding alias (the CTE bodies attach their
/// own `h` / `c` aliases): a quoted table name, or a parenthesised `rr:sqlQuery`.
fn source_sql(source: &LogicalSource, dialect: Dialect) -> String {
    match source {
        LogicalSource::Table(t) => dialect.quote_ident(t),
        LogicalSource::Query(q) => format!("({q})"),
    }
}

fn render_from(
    b: &Branch,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    // SubPlan derived-table joins (ADR-0023 M5 Wave 2): nested Plans inlined as
    // `(SELECT …) AS t{alias}`. Nested params are spliced at this text-order position
    // (load-bearing for prepared-statement binding — positional order matters).
    //
    // When `core` is non-empty the first core scan is the FROM anchor; SubPlan joins
    // follow as INNER/LEFT JOIN. When `core` is empty AND there are SubPlan joins the
    // first SubPlan becomes the FROM anchor (no CROSS JOIN keyword before it).
    let emit_sp = |sp: &crate::iq::SubPlanJoin,
                   params: &mut Vec<String>,
                   pidx: &mut usize,
                   join_kw: &str|
     -> Result<String> {
        let (nested_sql, nested_params) = emit_subplan_sql(&sp.plan, dialect, catalog)?;
        let nested_count = nested_params.len();
        // Rebase Postgres $N placeholders in the nested SQL from $1.. to $(pidx+1)..
        let rebased = rebase_placeholders(&nested_sql, dialect, *pidx)?;
        // Splice nested params into the parent's param vector at this text position.
        params.extend(nested_params);
        // Advance pidx past the nested params so subsequent ON conditions number correctly.
        *pidx += nested_count;
        Ok(format!("{join_kw}({rebased}) t{}", sp.alias))
    };

    let from = if b.core.is_empty() {
        // No base (core) scans. A synthetic one-row anchor stands in for the
        // `IqNode::True` / empty-BGP identity this branch's own left side represents
        // (SPARQL's `{} OPTIONAL {X}` always has exactly one solution to extend), and
        // EVERY opt / SubPlan attaches to it via its own JOIN — never as a hard FROM
        // anchor that would wrongly drop that guaranteed row on zero matches.
        //
        // Opts render BEFORE SubPlans — the SAME order as the core-bearing branch below
        // — so a SubPlan whose `on` correlates on a prior opt (or earlier SubPlan)
        // references a table already emitted to its LEFT (valid SQL). Previously this
        // path made the FIRST SubPlan the FROM anchor and emitted it with NO `ON` clause
        // AND rendered opts AFTER SubPlans: a SubPlan correlated on an opt then either
        // silently DROPPED its correlation (a wrong answer — an uncorrelated cross join)
        // or referenced an opt emitted to its right (a crash) — both ADR-0007 violations.
        // The uniform "(SELECT 1) anchor, then opts, then SubPlans" order fixes them and
        // matches the already-shipped core-less-plus-opts fix
        // (`bare_group_as_leftjoin_left_no_longer_mis_aliases`). A one-row cross join is
        // `=_bag`-transparent, so the previously-anchor-was-a-SubPlan cases (an
        // uncorrelated `{} OPTIONAL {sub}`, the SQL agg-over-UNION pushdown) keep their
        // meaning — only their cosmetic SQL shape changes.
        let mut from = "(SELECT 1) t_empty".to_owned();
        for opt in &b.opts {
            from.push_str(" LEFT JOIN ");
            from.push_str(&scan_ref(&opt.scan, dialect, catalog, params, pidx)?);
            from.push_str(" ON ");
            let conds: Vec<&SqlCond> = opt.on.iter().chain(opt.extra.iter()).collect();
            from.push_str(&render_conjunction(
                &conds, dialect, catalog, actuals, params, pidx,
            )?);
        }
        for sp in &b.subplan_joins {
            let join_kw = if sp.left {
                " LEFT JOIN "
            } else {
                " INNER JOIN "
            };
            from.push_str(&emit_sp(sp, params, pidx, join_kw)?);
            if !sp.on.is_empty() {
                from.push_str(" ON ");
                let conds: Vec<&SqlCond> = sp.on.iter().collect();
                from.push_str(&render_conjunction(
                    &conds, dialect, catalog, actuals, params, pidx,
                )?);
            } else {
                from.push_str(" ON 1 = 1");
            }
        }
        from
    } else {
        let mut scans = b.core.iter();
        let first = scans.next().expect("core non-empty — checked above");
        let mut from = scan_ref(first, dialect, catalog, params, pidx)?;
        for s in scans {
            from.push_str(" CROSS JOIN ");
            from.push_str(&scan_ref(s, dialect, catalog, params, pidx)?);
        }
        for opt in &b.opts {
            from.push_str(" LEFT JOIN ");
            from.push_str(&scan_ref(&opt.scan, dialect, catalog, params, pidx)?);
            from.push_str(" ON ");
            let conds: Vec<&SqlCond> = opt.on.iter().chain(opt.extra.iter()).collect();
            from.push_str(&render_conjunction(
                &conds, dialect, catalog, actuals, params, pidx,
            )?);
        }
        for sp in &b.subplan_joins {
            let join_kw = if sp.left {
                " LEFT JOIN "
            } else {
                " INNER JOIN "
            };
            from.push_str(&emit_sp(sp, params, pidx, join_kw)?);
            if !sp.on.is_empty() {
                from.push_str(" ON ");
                let conds: Vec<&SqlCond> = sp.on.iter().collect();
                from.push_str(&render_conjunction(
                    &conds, dialect, catalog, actuals, params, pidx,
                )?);
            } else {
                from.push_str(" ON 1 = 1");
            }
        }
        from
    };
    Ok(from)
}

/// Render all prepared branches of a nested [`Plan`] to a single SQL SELECT string
/// (for embedding as a derived table). Recursively probed live names override the
/// offline lexical fallback. Multi-branch plans become a `UNION ALL`. Returns
/// `(sql_text, params)` — params in text order, placeholders starting from 1.
fn emit_subplan_sql(
    plan: &crate::Plan,
    dialect: Dialect,
    live_catalog: &ColumnCatalog,
) -> Result<(String, Vec<String>)> {
    let branches = plan.prepared_branches();
    let mut catalog = synthetic_subplan_catalog(&branches);
    catalog.suppress_path_collation = live_catalog.suppress_path_collation;
    catalog.character_keys = std::sync::Arc::clone(&live_catalog.character_keys);
    catalog.lexical_keys = std::sync::Arc::clone(&live_catalog.lexical_keys);
    // Live top-level execution has already probed every recursively reachable
    // base source. Overlay those authoritative names so nested SubPlan emission
    // does not depend on the offline lexical alias-folding heuristic. The
    // synthetic entries remain only for dialect-neutral/offline emission and
    // source-free derived columns.
    for source in live_metadata_sources(&branches) {
        if let Some(columns) = live_catalog.columns(source) {
            catalog.insert(source, columns.to_vec());
            if let Some(text) = live_catalog.text_by_source.get(&source_key(source)) {
                std::sync::Arc::make_mut(&mut catalog.text_by_source)
                    .insert(source_key(source), text.clone());
            }
            if let Some(sqlite) = live_catalog.sqlite_by_source.get(&source_key(source)) {
                std::sync::Arc::make_mut(&mut catalog.sqlite_by_source)
                    .insert(source_key(source), sqlite.clone());
            }
        }
    }
    let emitted = branches
        .iter()
        .map(|branch| emit_branch_keys(branch, dialect, &catalog, plan.distinct || branch.distinct))
        .collect::<Result<Vec<_>>>()?;
    if emitted.is_empty() {
        // Empty inner plan — a values-empty derived table: return a SELECT with no rows.
        // Use a dummy column so it is syntactically valid as a derived table.
        return Ok(("SELECT 1 AS __sf_empty WHERE 1 = 0".to_owned(), Vec::new()));
    }
    if emitted.len() == 1 {
        let e = &emitted[0];
        return Ok((e.sql.clone(), e.params.clone()));
    }
    // Multiple branches: `UNION ALL` (bag semantics) by default, or `UNION` (dedup) when the
    // plan carries a DISTINCT (a multi-branch DISTINCT SubPlan, ADR-0025 Tier-2 gap 2 — the
    // pooling requires injective cross-arm reconstruction, so SQL `UNION`'s raw-column dedup
    // equals SPARQL DISTINCT on the reconstructed terms). SQLite's compound-select grammar
    // does NOT accept a parenthesised `select-core` as a UNION operand (`(SELECT …) UNION …`
    // is a syntax error there — the q9 agg-pushdown wave's first live failure); PG/MySQL
    // accept it. So SQLite joins the arms bare.
    let mut all_sql = Vec::new();
    let mut all_params = Vec::new();
    for e in &emitted {
        if dialect == Dialect::Sqlite {
            all_sql.push(e.sql.clone());
        } else {
            all_sql.push(format!("({})", e.sql));
        }
        all_params.extend(e.params.clone());
    }
    let op = if plan.distinct {
        " UNION "
    } else {
        " UNION ALL "
    };
    let sql = all_sql.join(op);
    Ok((sql, all_params))
}

/// Rebase positional `$N` placeholders in `sql` from base 1 to start at `base+1`,
/// for PostgreSQL numbered placeholders. SQLite uses `?` (positional by text order,
/// no numbering), so for SQLite (or when `base == 0`) returns `sql` unchanged.
fn rebase_placeholders(sql: &str, dialect: Dialect, base: usize) -> Result<String> {
    if dialect != Dialect::Postgres || base == 0 {
        return Ok(sql.to_owned());
    }
    // Replace each `$N` → `$(N + base)` by scanning the string bytes.
    let mut out = String::with_capacity(sql.len() + 16);
    let bytes = sql.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
            i += 1; // skip '$'
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let n: usize = sql[start..i].parse().map_err(|_| {
                Error::Sql(format!(
                    "rebase_placeholders: non-numeric after $: {}",
                    &sql[start..i]
                ))
            })?;
            out.push('$');
            out.push_str(&(n + base).to_string());
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    Ok(out)
}

fn render_where(
    conds: &[SqlCond],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<Option<String>> {
    if conds.is_empty() {
        return Ok(None);
    }
    let refs: Vec<&SqlCond> = conds.iter().collect();
    Ok(Some(render_conjunction(
        &refs, dialect, catalog, actuals, params, pidx,
    )?))
}

fn render_conjunction(
    conds: &[&SqlCond],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    if conds.is_empty() {
        return Ok("1 = 1".to_owned());
    }
    Ok(conds
        .iter()
        .map(|c| render_cond(c, dialect, catalog, actuals, params, pidx))
        .collect::<Result<Vec<_>>>()?
        .join(" AND "))
}

fn render_cond(
    cond: &SqlCond,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    Ok(match cond {
        SqlCond::ColEq(a, b) => render_key_equality(a, b, dialect, catalog, actuals),
        SqlCond::NativeColEq(a, b) => format!(
            "{} = {}",
            colref(a, dialect, actuals),
            colref(b, dialect, actuals)
        ),
        SqlCond::NullSafeEq(a, b) => {
            let (la, lb) = (colref(a, dialect, actuals), colref(b, dialect, actuals));
            let equal = render_key_equality(a, b, dialect, catalog, actuals);
            format!("({equal} OR {la} IS NULL OR {lb} IS NULL)")
        }
        SqlCond::Cmp(a, op, val) | SqlCond::NativeCmp(a, op, val) => {
            params.push(val.clone());
            *pidx += 1;
            format!(
                "{} {} {}",
                if matches!(cond, SqlCond::NativeCmp(..)) {
                    colref(a, dialect, actuals)
                } else {
                    path_comparison::rdf_column(a, dialect, catalog, actuals)
                },
                op.as_sql(),
                dialect.placeholder(*pidx)
            )
        }
        SqlCond::StrMatch { col, op, param } => {
            // The pattern/regex is a bound parameter (ADR-0010 R1) — never inlined.
            // The `ESCAPE '\'` char is a fixed engine constant (not query data), so
            // it is part of the trusted skeleton, like an identifier.
            params.push(param.clone());
            *pidx += 1;
            let ph = dialect.placeholder(*pidx);
            let c = colref(col, dialect, actuals);
            match op {
                StrMatchOp::CoarseLexicalEqual => match dialect {
                    Dialect::Sqlite => {
                        format!("(typeof({c}) <> 'text' OR rtrim({c}, ' ') = rtrim({ph}, ' '))")
                    }
                    Dialect::Postgres => format!(
                        "(pg_catalog.pg_typeof({c}) NOT IN ('pg_catalog.text'::regtype, \
                         'pg_catalog.varchar'::regtype) OR CAST({c} AS TEXT) = {ph})"
                    ),
                    Dialect::MySql => format!(
                        "(CASE WHEN @@character_set_client <> 'utf8mb4' \
                         OR @@character_set_connection <> 'utf8mb4' \
                         OR @@character_set_results IS NULL \
                         OR @@character_set_results <> 'utf8mb4' \
                         OR CHARSET({c}) = 'binary' THEN TRUE \
                         WHEN JSON_VALID(CAST({c} AS CHAR)) THEN TRUE \
                         ELSE RTRIM(CAST({c} AS CHAR)) = RTRIM({ph}) END)"
                    ),
                    _ => return Err(Error::Unsupported("bounded join reducer dialect".into())),
                },
                StrMatchOp::Like => format!("{c} LIKE {ph} ESCAPE '\\'"),
                StrMatchOp::RegexMatch => format!("{c} ~ {ph}"),
                StrMatchOp::RegexMatchI => format!("{c} ~* {ph}"),
            }
        }
        SqlCond::IsNotNull(a) => format!("{} IS NOT NULL", colref(a, dialect, actuals)),
        SqlCond::IsNull(a) => format!("{} IS NULL", colref(a, dialect, actuals)),
        SqlCond::Not(c) => format!(
            "(NOT {})",
            render_cond(c, dialect, catalog, actuals, params, pidx)?
        ),
        SqlCond::And(cs) => {
            let refs: Vec<&SqlCond> = cs.iter().collect();
            format!(
                "({})",
                render_conjunction(&refs, dialect, catalog, actuals, params, pidx)?
            )
        }
        SqlCond::Or(cs) => {
            if cs.is_empty() {
                return Ok("1 = 0".to_owned());
            }
            let parts: Vec<String> = cs
                .iter()
                .map(|c| render_cond(c, dialect, catalog, actuals, params, pidx))
                .collect::<Result<Vec<_>>>()?;
            format!("({})", parts.join(" OR "))
        }
        // MINUS anti-join (SPARQL §8.3): a correlated `NOT EXISTS` over the right
        // pattern's scans. The inner WHERE renders the right pattern's own
        // conditions plus the shared-variable correlation equalities (which name the
        // outer left aliases), so the whole left row is dropped when a compatible
        // right row exists. A core-less right side (an inline VALUES) renders without
        // FROM. Inner placeholders bind in text order at this position.
        SqlCond::NotExists { scans, conds } | SqlCond::Exists { scans, conds } => {
            let neg = matches!(cond, SqlCond::NotExists { .. });
            let from = scans
                .iter()
                .map(|scan| scan_ref(scan, dialect, catalog, params, pidx))
                .collect::<Result<Vec<_>>>()?
                .join(" CROSS JOIN ");
            let mut nested_actuals = actuals.clone();
            for scan in scans {
                nested_actuals.insert(scan.alias, scan_actuals(scan, dialect, catalog));
            }
            let refs: Vec<&SqlCond> = conds.iter().collect();
            let where_sql =
                render_conjunction(&refs, dialect, catalog, &nested_actuals, params, pidx)?;
            let kw = if neg { "NOT EXISTS" } else { "EXISTS" };
            if from.is_empty() {
                format!("{kw} (SELECT 1 WHERE {where_sql})")
            } else {
                format!("{kw} (SELECT 1 FROM {from} WHERE {where_sql})")
            }
        }
        // ADR-0025 Tier-2 gap 1: a correlated [NOT] EXISTS whose inner is a property-path
        // CLOSURE — the recursive-CTE distinct-pairs table `t{pc.alias}(sf_s, sf_o)`. The
        // prelude resolves its own base columns against the live `catalog` (threaded through
        // this render chain), so ALL path kinds — including the reflexive `P*`/`P?`, whose
        // prelude calls the fallible `reflexive_sql` — render here; the `?` propagates any
        // prelude error soundly instead of the old empty-catalog `unwrap_or_default`.
        SqlCond::PathExists { pc, conds, negated } => {
            let with = path_with_prelude(pc, dialect, catalog)?;
            let mut nested_actuals = actuals.clone();
            nested_actuals.insert(pc.alias, path_actuals(pc, catalog));
            let refs: Vec<&SqlCond> = conds.iter().collect();
            let where_sql =
                render_conjunction(&refs, dialect, catalog, &nested_actuals, params, pidx)?;
            let kw = if *negated { "NOT EXISTS" } else { "EXISTS" };
            format!(
                "{kw} ({with} SELECT 1 FROM t{} WHERE {where_sql})",
                pc.alias
            )
        }
        // Run 4 Wave B3 — `unify::align_templates`'s shape-mismatch fallback:
        // render each side as a SQL string concatenation and compare with
        // `=`. See `render_template_concat`'s doc comment for the
        // dialect-support boundary and the NULL-propagation soundness
        // argument (why a NULL underlying column correctly excludes the row
        // rather than needing special-casing here).
        SqlCond::TemplateEq(sx, a1, sy, a2, encode_iri) => {
            let r1 = render_template_concat(
                sx,
                *encode_iri,
                dialect,
                |c| path_comparison::rdf_column(&ColRef::new(*a1, c), dialect, catalog, actuals),
                params,
                pidx,
            )?;
            let r2 = render_template_concat(
                sy,
                *encode_iri,
                dialect,
                |c| path_comparison::rdf_column(&ColRef::new(*a2, c), dialect, catalog, actuals),
                params,
                pidx,
            )?;
            format!("{r1} = {r2}")
        }
    })
}

/// Render one template's segments as a dialect-appropriate SQL string
/// concatenation — [`SqlCond::TemplateEq`]'s per-side rendering (Run 4 Wave
/// B3). Every `Segment::Literal` is a bound parameter (ADR-0010 R1): the
/// text is mapping-trusted, not query-supplied, but this still follows the
/// blanket "values are bound parameters only" rule rather than hand-rolling
/// SQL string-literal escaping. Every `Segment::Column` renders through the
/// SAME [`colref`] every other `SqlCond` arm uses, additionally wrapped in
/// [`percent_encode_col`] when `encode_iri` (below).
///
/// **Percent-encoding soundness (Run 4 B-repair FIX 2).** `sf_core::ir::
/// Template::expand`'s `encode_iri` flag percent-encodes EVERY column value
/// — never a template's own literal segments — when (and only when) the
/// template constructs an IRI (R2RML §7.3 / RFC 3987); a plain-literal
/// template's expansion is raw, unencoded column text. Before this fix,
/// EVERY `Segment::Column` rendered here as a bare `colref`, regardless of
/// kind — sound for two templates whose column values happen to carry no
/// IRI-encodable character, but wrong in general: a single-column template
/// `http://ex.org/v/{va}` with `va = "X/Y"` expands (RDF) to
/// `http://ex.org/v/X%2FY` (the `/` encoded), while a two-column template
/// `http://ex.org/v/{vb1}/{vb2}` with `vb1="X", vb2="Y"` expands to
/// `http://ex.org/v/X/Y` (the `/` a literal template character) — DIFFERENT
/// IRIs, yet the old raw-concat rendering compared them EQUAL (false
/// positive), and conversely could compare two RDF-equal IRIs unequal.
/// `encode_iri` is one flag for BOTH sides ([`SqlCond::TemplateEq`]'s own
/// doc comment: `align_templates`'s caller only ever builds this variant
/// when both sides share one `TermType`), so each `Segment::Column` here is
/// consistently encoded, or consistently left raw, matching whichever
/// `Template::expand` itself would do for that same template.
///
/// **NULL-propagation soundness.** On every dialect rendered below, `||`
/// (Postgres/SQLite) and `CONCAT(...)` (MySQL) return SQL `NULL` if ANY
/// operand is `NULL` — so `render(t1) = render(t2)` evaluates to UNKNOWN
/// (never `TRUE`) whenever a referenced column is `NULL`, and a WHERE/JOIN
/// condition that is UNKNOWN excludes the row, same as `FALSE`. This is NOT
/// an approximation: `sf_core::term::generate_into`'s `TermMap::Template`
/// arm (`Template::expand`, per R2RML §11) ALREADY treats "any referenced
/// column is NULL" as "this variable is UNBOUND" for that row when
/// RECONSTRUCTING the SAME template in Rust — `sf-core/term.rs`'s
/// `null_in_template_yields_no_term` test locks this in. So a NULL-collapsed
/// concatenation here excludes EXACTLY the rows whose SPARQL-level operand
/// would have been unbound anyway (comparing against an unbound variable is
/// a type error ⇒ the row is excluded from a FILTER, and an unbound shared
/// variable cannot correlate a join either) — the SQL and RDF answers agree
/// by construction, not by coincidence. [`percent_encode_col`]'s own three
/// per-dialect implementations preserve this EXPLICITLY (a `CASE WHEN col IS
/// NULL THEN NULL ELSE …` wrapper, not incidental `NULL`-propagation through
/// some other operator) — see its own doc comment for why an explicit
/// wrapper is required rather than assumed.
///
/// **Dialect support.** Only the three PRODUCTION-WIRED dialects
/// (`sf_sql::Dialect`'s own grouping) are implemented: PostgreSQL/SQLite via
/// ANSI `||`, MySQL via `CONCAT(...)` (`||` is boolean OR there by default,
/// per MySQL's non-default `PIPES_AS_CONCAT` sql_mode). Every OTHER dialect
/// returns `Unsupported` rather than guessing — e.g. SQL Server's own
/// `CONCAT()` function treats `NULL` as an EMPTY STRING (breaking the
/// soundness argument above outright), and `+`, SQL Server's NULL-safe
/// concat operator, is unverified against this rendering; Oracle/DuckDB/etc.
/// are ANSI-`||`-following by reputation but likewise unverified here —
/// "sound over complete", the same bar `str_match`'s PostgreSQL-only `LIKE`
/// pushdown already sets for an analogous dialect-behavior gap.
fn render_template_concat(
    segs: &[sf_core::ir::Segment],
    encode_iri: bool,
    dialect: Dialect,
    column: impl Fn(&str) -> String,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    use sf_core::ir::Segment;
    let mut parts = Vec::with_capacity(segs.len());
    for seg in segs {
        parts.push(match seg {
            Segment::Literal(text) => {
                params.push(text.to_string());
                *pidx += 1;
                dialect.placeholder(*pidx)
            }
            Segment::Column(c) => {
                let col = column(c);
                if encode_iri {
                    percent_encode_col(&col, dialect)?
                } else {
                    col
                }
            }
        });
    }
    match dialect {
        Dialect::Postgres | Dialect::Sqlite => Ok(format!("({})", parts.join(" || "))),
        Dialect::MySql => Ok(format!("CONCAT({})", parts.join(", "))),
        other => Err(Error::Unsupported(format!(
            "template-shape-mismatch equality (SQL CONCAT fallback) is not implemented for \
             {other:?} → 501 (never a silently wrong NULL/concat-operator guess)"
        ))),
    }
}

/// Render mapping-trusted template literals inline, with source-aware columns.
/// No query/policy values enter this parameter-free projection recipe.
pub(crate) fn render_template_inline(
    segs: &[sf_core::ir::Segment],
    encode_iri: bool,
    dialect: Dialect,
    column: impl Fn(&str) -> String,
) -> Result<String> {
    use sf_core::ir::Segment;
    let parts = segs
        .iter()
        .map(|segment| match segment {
            Segment::Literal(text) => Ok(sql_string_literal(text)),
            Segment::Column(name) if encode_iri => percent_encode_col(&column(name), dialect),
            Segment::Column(name) => Ok(column(name)),
        })
        .collect::<Result<Vec<_>>>()?;
    match dialect {
        Dialect::Postgres | Dialect::Sqlite => Ok(format!("({})", parts.join(" || "))),
        Dialect::MySql => Ok(format!("CONCAT({})", parts.join(", "))),
        _ => Err(Error::Unsupported("rendered projection dialect".into())),
    }
}

/// Render `column` against one immediate scan source at translate time.
///
/// `inner_sql == None` denotes a base [`LogicalSource::Table`]; `Some` denotes a
/// derived [`LogicalSource::Query`]. That distinction is load-bearing for Direct
/// Mapping's synthetic no-primary-key `rowid`: PostgreSQL reads base-table
/// `ctid`, while an authored or compiler-derived query output named `rowid`
/// remains an ordinary column. Query aliases also retain the existing bounded
/// bare-`AS` case-folding heuristic. Live emission uses the typed equivalent in
/// [`colref`].
pub(crate) fn render_immediate_source_column(
    alias: &str,
    column: &str,
    inner_sql: Option<&str>,
    dialect: Dialect,
) -> String {
    if dialect == Dialect::Postgres && column == "rowid" && inner_sql.is_none() {
        return format!("({alias}.ctid)::text");
    }
    if dialect == Dialect::Postgres
        && inner_sql.is_some_and(|sql| crate::cascade::col_is_unquoted_alias(sql, column))
    {
        format!("{alias}.{column}")
    } else {
        format!("{alias}.{}", dialect.quote_ident(column))
    }
}

/// A SQL single-quoted string literal for mapping-trusted text `text` — ANSI
/// `''`-doubling, the one escaping rule SQLite/PostgreSQL/MySQL all share for a
/// single-quoted string (unlike percent-encoding, no per-dialect split applies
/// here). [`render_template_inline`]'s only caller.
fn sql_string_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Percent-encode `col_sql`'s runtime value EXACTLY the way `sf_core::ir::
/// Template::expand`'s `encode_iri` arm does (`percent_encode_iri`, same
/// file): RFC 3987 *iunreserved* = `ALPHA / DIGIT / "-" / "." / "_" / "~"`
/// passes through; every OTHER byte (the FULL 0x00-0x7F complement, all 62
/// bytes, including every ASCII control byte) becomes `%XX` (uppercase
/// hex); non-ASCII passes through unchanged.
///
/// **History: why this is not a flat `REPLACE` chain.** An earlier version
/// nested one `REPLACE` call per encodable byte — `REPLACE` being ANSI-
/// portable, the obvious building block. That fails for two INDEPENDENT
/// reasons, both found empirically against [`Dialect::emit_via_ast`] (the
/// `sqlparser` AST round-trip every emitted statement goes through):
/// 1. **SQLite** has a hard recursion-depth ceiling (`sqlparser`'s
///    `DEFAULT_REMAINING_DEPTH`) — a flat chain over the full 62-byte set
///    (deeper than the empirically measured ~41-44-level ceiling inside a
///    realistic WHERE clause) errors "recursion limit exceeded" outright.
/// 2. **PostgreSQL** is far worse: not a lower ceiling but EXPONENTIAL
///    parse time in nesting depth, well before any hard limit fires
///    (measured: depth 8 ≈ 14ms, depth 12 ≈ 99ms, depth 20 ≈ over 16
///    SECONDS) — general to nested-function-call parsing in `sqlparser`'s
///    PG dialect (reproduced with a single-argument `UPPER(...)` chain, not
///    just `REPLACE`), so no flat chain wide enough for full coverage is
///    viable there at any practical depth.
///
/// **The fix: per-character SQL, not per-character SQL TEXT NESTING.** Each
/// dialect gets its OWN O(1)-parse-depth encoder — a single `WITH RECURSIVE`
/// (or, for PostgreSQL, `unnest(...) WITH ORDINALITY`) that iterates the
/// STRING'S OWN characters/bytes as ROWS, classifies each with one `CASE`,
/// and reassembles via an ORDER-preserving aggregate (`group_concat`/
/// `string_agg`/`GROUP_CONCAT`, all `... ORDER BY ...` — SQLite 3.44+, this
/// project's bundled 3.46.0 confirmed; PostgreSQL and MySQL support it
/// natively). Parse depth is CONSTANT regardless of the encode-set size or
/// the runtime string length — confirmed fast (single-digit milliseconds)
/// against the SAME realistic OR-IS-NULL-wrapped, multi-column WHERE clause
/// that broke the flat-chain design, for all three dialects.
///
/// **Byte- vs. character-oriented, per dialect — not interchangeable.**
/// SQLite's/MySQL's plain `LENGTH()`/`SUBSTRING()` on a TEXT argument are
/// NOT reliable byte-accurate iterators (SQLite's is character-counting and
/// silently truncates at an embedded NUL, exactly like a C string; MySQL's
/// `LENGTH()` is byte-oriented but its plain `SUBSTRING()` is CHARACTER-
/// oriented — an internally inconsistent pairing that walks past a
/// multi-byte character's true end) — both confirmed by direct, deliberate
/// probing before this design was settled on, both fixed by an explicit
/// `CAST(... AS BLOB)` (SQLite) / `CAST(... AS BINARY)` (MySQL) so every
/// function in the chain is consistently byte-oriented; a non-ASCII
/// multi-byte character is then walked and reassembled ONE RAW BYTE AT A
/// TIME (every continuation/lead byte is ≥ 0x80, so "byte ≥ 0x80 passes
/// through unchanged" correctly reconstructs it without ever needing to
/// understand UTF-8 structure) — confirmed an ISOLATED intermediate byte
/// cast is not independently valid UTF-8, but the FINAL reassembled result
/// is. PostgreSQL's `text` is different on both counts: it cannot contain a
/// NUL byte at all (the server rejects it outright — confirmed live,
/// `ERROR: invalid byte sequence for encoding "UTF8": 0x00` — so there is
/// no NUL case to handle), and `string_to_array(text, NULL)` natively splits
/// by CHARACTER (not byte), which is the natural, already-decoded unit
/// there — `ascii(ch)` gives the correct code point for classification
/// (including non-ASCII), so no BLOB-equivalent cast is needed.
///
/// **NULL-propagation, correctly this time.** `sf_core::term::generate_into`
/// treats "any referenced column is NULL" as "this variable is UNBOUND" for
/// that row (`null_in_template_yields_no_term`, sf-core), so a NULL column
/// must render as SQL NULL — but the natural per-character aggregate
/// (`group_concat`/`string_agg`/`GROUP_CONCAT`) returns NULL over ZERO
/// input rows REGARDLESS of why there were zero rows: a genuinely NULL
/// column (nothing to iterate) and a genuinely EMPTY, non-NULL string
/// (also nothing to iterate, but should encode to `""`, not NULL) are
/// otherwise indistinguishable through the aggregate alone — confirmed by
/// direct probing: an early version without the NULL-vs-empty split below
/// wrongly rendered EACH of "NULL" and "empty string" as if the OTHER,
/// AND wrongly emitted a bare stray `%` for the empty-string case (the
/// iterator's own "at least one row" base case still fired past the end of
/// a zero-length string). Every implementation below is therefore the SAME
/// three-part shape: `CASE WHEN col IS NULL THEN NULL ELSE COALESCE(
/// <per-character aggregate>, '') END` — the outer CASE separates "no
/// value" from "empty value" (COALESCE alone cannot), and COALESCE only
/// then normalizes the (now unambiguous) zero-character case to `''`.
///
/// Every literal in these templates (hex-range bounds, the `-._~` char
/// list, dialect keywords) is a fixed SQL-syntax constant under this
/// module's own control, not query- or mapping-supplied data — inlining
/// them is the "fixed engine constant… part of the trusted skeleton" rule
/// this file's `LIKE ESCAPE '\'` rendering already uses (`render_cond`'s
/// `StrMatch` arm), not a departure from ADR-0010 R1 (which governs values
/// that originate from the SPARQL query or the mapping, neither of which
/// this function ever touches — the ONLY runtime input is the column
/// reference itself, already-resolved SQL text, not a bound value).
fn percent_encode_col(col_sql: &str, dialect: Dialect) -> Result<String> {
    Ok(match dialect {
        Dialect::Sqlite => percent_encode_col_sqlite(col_sql),
        Dialect::MySql => percent_encode_col_mysql(col_sql),
        Dialect::Postgres => percent_encode_col_postgres(col_sql),
        other => {
            return Err(Error::Unsupported(format!(
                "IRI-template percent-encoding is not implemented for {other:?} → 501 \
                 (never a silently wrong un-encoded comparison)"
            )))
        }
    })
}

/// SQLite: `CAST(... AS BLOB)` throughout (byte-oriented `LENGTH`/`substr`,
/// sidestepping TEXT-mode `LENGTH`'s NUL-terminated character counting —
/// see [`percent_encode_col`]'s doc comment). `hex()`, not `unicode()`, for
/// byte classification (`unicode()` reinterprets an isolated non-ASCII byte
/// as a UTF-8 decode attempt and returns the U+FFFD replacement code point
/// for an invalid standalone continuation/lead byte — confirmed live;
/// `hex()` returns the raw byte unconditionally). `group_concat(...ORDER BY
/// n)` needs SQLite ≥ 3.44 (this project's bundled `libsqlite3-sys` ships
/// 3.46.0, confirmed live).
fn percent_encode_col_sqlite(col: &str) -> String {
    format!(
        "(SELECT CASE WHEN {col} IS NULL THEN NULL ELSE COALESCE((\
WITH RECURSIVE seq(n) AS (\
SELECT 1 WHERE LENGTH(CAST({col} AS BLOB)) > 0 \
UNION ALL \
SELECT n + 1 FROM seq WHERE n < LENGTH(CAST({col} AS BLOB))\
) \
SELECT group_concat(\
CASE \
WHEN hex(substr(CAST({col} AS BLOB), n, 1)) BETWEEN '30' AND '39' \
OR hex(substr(CAST({col} AS BLOB), n, 1)) BETWEEN '41' AND '5A' \
OR hex(substr(CAST({col} AS BLOB), n, 1)) BETWEEN '61' AND '7A' \
OR hex(substr(CAST({col} AS BLOB), n, 1)) IN ('2D', '2E', '5F', '7E') \
OR hex(substr(CAST({col} AS BLOB), n, 1)) >= '80' \
THEN CAST(substr(CAST({col} AS BLOB), n, 1) AS TEXT) \
ELSE '%' || hex(substr(CAST({col} AS BLOB), n, 1)) \
END, '' ORDER BY n\
) FROM seq\
), '') END)"
    )
}

/// MySQL: `CAST(... AS BINARY)` throughout — MySQL's `LENGTH()` is
/// byte-oriented but its plain `SUBSTRING()` is CHARACTER-oriented (a
/// confirmed-live, internally inconsistent pairing this cast reconciles,
/// mirroring the SQLite BLOB cast for the identical class of unit
/// mismatch); the pass-through branch and the `%XX` branch are BOTH kept as
/// `BINARY` inside `GROUP_CONCAT` so the aggregate never implicitly
/// re-interprets an in-flight (possibly standalone-invalid) byte as text,
/// and the FINAL aggregated result is converted back to `utf8mb4` once, at
/// the very end (mirrors the SQLite per-byte-cast pattern: only the fully
/// reassembled result needs to be valid UTF-8, confirmed live).
///
/// `JSON_TABLE(... FOR ORDINALITY)` supplies a row per byte. This is
/// deliberately not a correlated recursive CTE: MySQL 8.4 materializes such
/// a CTE using one outer row's length when the expression is projected for
/// several rows, which silently truncates/pads other values. `JSON_TABLE` is
/// implicitly lateral in MySQL and evaluates its document per outer row.
/// The `SET_VAR` optimizer hint requests `group_concat_max_len = 1,000,000`
/// for this query only; MySQL's 1024-byte default otherwise silently truncates
/// the result. The input guard does not trust the hint: it takes the minimum of
/// the hard 333,333-byte profile bound, the statement-observed
/// `@@SESSION.group_concat_max_len / 3`, and the statement-observed
/// `(@@SESSION.max_allowed_packet - 4096) / 3`. One source byte can expand to
/// three output bytes, and the 4-KiB packet reserve covers protocol/query
/// framing. Above that conservative dynamic bound the JSON document is
/// deliberately invalid, so execution fails before `GROUP_CONCAT` can return a
/// truncated IRI.
const MYSQL_GROUP_CONCAT_MAX_LEN: usize = 1_000_000;
const MYSQL_PERCENT_ENCODE_MAX_INPUT_BYTES: usize = MYSQL_GROUP_CONCAT_MAX_LEN / 3;
const MYSQL_PACKET_RESERVE_BYTES: usize = 4_096;

fn percent_encode_col_mysql(col: &str) -> String {
    format!(
        "(SELECT CASE WHEN {col} IS NULL THEN NULL ELSE COALESCE((\
SELECT /*+ SET_VAR(group_concat_max_len = {group_limit}) */ \
CONVERT(CAST(GROUP_CONCAT(\
CASE \
WHEN HEX(SUBSTRING(CAST({col} AS BINARY), n, 1)) BETWEEN '30' AND '39' \
OR HEX(SUBSTRING(CAST({col} AS BINARY), n, 1)) BETWEEN '41' AND '5A' \
OR HEX(SUBSTRING(CAST({col} AS BINARY), n, 1)) BETWEEN '61' AND '7A' \
OR HEX(SUBSTRING(CAST({col} AS BINARY), n, 1)) IN ('2D', '2E', '5F', '7E') \
OR HEX(SUBSTRING(CAST({col} AS BINARY), n, 1)) >= '80' \
THEN SUBSTRING(CAST({col} AS BINARY), n, 1) \
ELSE CAST(CONCAT('%', HEX(SUBSTRING(CAST({col} AS BINARY), n, 1))) AS BINARY) \
END ORDER BY n SEPARATOR ''\
) AS BINARY) USING utf8mb4)\
FROM JSON_TABLE(\
CASE WHEN LENGTH(CAST({col} AS BINARY)) = 0 THEN '[]' \
WHEN LENGTH(CAST({col} AS BINARY)) > LEAST(\
{max_input}, \
GREATEST(CAST(@@SESSION.group_concat_max_len AS SIGNED), 0) DIV 3, \
GREATEST(CAST(@@SESSION.max_allowed_packet AS SIGNED) - {packet_reserve}, 0) DIV 3\
) \
THEN 'semantic-fabric-percent-encoding-input-limit' \
ELSE CONCAT('[0', REPEAT(',0', LENGTH(CAST({col} AS BINARY)) - 1), ']') END, \
'$[*]' COLUMNS (n FOR ORDINALITY)\
) AS sfpe\
), '') END)",
        group_limit = MYSQL_GROUP_CONCAT_MAX_LEN,
        max_input = MYSQL_PERCENT_ENCODE_MAX_INPUT_BYTES,
        packet_reserve = MYSQL_PACKET_RESERVE_BYTES,
    )
}

/// PostgreSQL: character-oriented (`string_to_array(text, NULL)` natively
/// splits by character — the correct unit for `text`, which is always
/// well-formed and cannot carry an embedded NUL at all, confirmed live).
/// `ascii(ch)` gives the numeric code point for classification (correct
/// for non-ASCII too, unlike a collation-dependent text comparison against
/// `chr(128)` would be). `unnest(...) WITH ORDINALITY` supplies the
/// position `string_agg(... ORDER BY ord)` reassembles by.
fn percent_encode_col_postgres(col: &str) -> String {
    format!(
        "(SELECT CASE WHEN {col}::text IS NULL THEN NULL ELSE COALESCE((\
SELECT string_agg(\
CASE \
WHEN ascii(ch) BETWEEN 48 AND 57 \
OR ascii(ch) BETWEEN 65 AND 90 \
OR ascii(ch) BETWEEN 97 AND 122 \
OR ch IN ('-', '.', '_', '~') \
OR ascii(ch) >= 128 \
THEN ch \
ELSE '%' || UPPER(LPAD(TO_HEX(ascii(ch)), 2, '0')) \
END, '' ORDER BY ord\
) FROM unnest(string_to_array({col}::text, NULL)) WITH ORDINALITY AS t(ch, ord)\
), '') END)"
    )
}

fn colref(c: &ColRef, dialect: Dialect, actuals: &ActualColumns) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iq::{Scan, StrMatchOp};
    use sf_core::ir::{LogicalSource, TermSpec};

    fn branch_with(cond: SqlCond) -> Branch {
        let mut b = Branch::single(Scan {
            alias: 0,
            source: (LogicalSource::Table("emp".to_owned())).into(),
        });
        b.where_conds.push(cond);
        b
    }

    /// A `LIKE` pushdown renders `ESCAPE '\'`, binds its pattern as a parameter,
    /// and round-trips through the AST (ADR-0020 §2 / ADR-0010 R1).
    #[test]
    fn like_renders_escape_and_binds_pattern() {
        let cond = SqlCond::StrMatch {
            col: ColRef::new(0, "name"),
            op: StrMatchOp::Like,
            param: "%a\\%b%".to_owned(),
        };
        let e = emit_branch(&branch_with(cond), Dialect::Sqlite).unwrap();
        let up = e.sql.to_uppercase();
        assert!(up.contains("LIKE") && up.contains("ESCAPE"), "{}", e.sql);
        assert!(e.sql.contains('?'), "bound placeholder: {}", e.sql);
        assert!(
            !e.sql.contains("a%b"),
            "value must not be inlined: {}",
            e.sql
        );
        assert_eq!(e.params, vec!["%a\\%b%".to_owned()]);
    }

    /// Identifier resolution (SQL:2008 folding): an exact match wins (a case-exact /
    /// delimited identifier), else a unique ASCII-case-insensitive match (a regular
    /// identifier the DBMS folded), else the identifier as written.
    #[test]
    fn resolve_col_prefers_exact_then_case_insensitive() {
        let cols = vec!["studentid".to_owned(), "ID".to_owned(), "Name".to_owned()];
        // Regular `StudentId` → no exact, single CI match → the folded actual name.
        assert_eq!(resolve_col("StudentId", Some(&cols)), "studentid");
        // Delimited/case-exact `Name` → exact match wins (never folded away).
        assert_eq!(resolve_col("Name", Some(&cols)), "Name");
        // `ID` exact match.
        assert_eq!(resolve_col("ID", Some(&cols)), "ID");
        // No such column → emitted as written (the source surfaces the error).
        assert_eq!(resolve_col("missing", Some(&cols)), "missing");
        // Unknown source → as written.
        assert_eq!(resolve_col("Whatever", None), "Whatever");
    }

    /// A branch emitted with a catalog resolves its regular-identifier column
    /// reference to the folded column the source actually exposes.
    #[test]
    fn emit_branch_with_resolves_folded_identifier() {
        let mut b = Branch::single(Scan {
            alias: 0,
            source: (LogicalSource::Table("Student".to_owned())).into(),
        });
        b.where_conds
            .push(SqlCond::IsNotNull(ColRef::new(0, "StudentId")));
        let mut catalog = ColumnCatalog::default();
        catalog.insert(
            &LogicalSource::Table("Student".to_owned()),
            vec!["studentid".to_owned()],
        );
        let e = emit_branch_with(&b, Dialect::Postgres, &catalog).unwrap();
        assert!(e.sql.contains("\"studentid\""), "{}", e.sql);
        assert!(!e.sql.contains("\"StudentId\""), "{}", e.sql);
        // Reconstruction still keys on the raw IR identifier (position-based read).
        assert_eq!(&*e.projection[0].column, "StudentId");
    }

    #[test]
    fn live_validation_preserves_physical_row_identifier_exceptions() {
        let mut branch = Branch::single(Scan {
            alias: 0,
            source: (LogicalSource::Table("no_pk".to_owned())).into(),
        });
        branch.bindings.insert(
            "s".to_owned(),
            TermDef::Derived {
                term_map: TermMap::Column("value".into(), TermSpec::plain_literal()),
                alias: 0,
            },
        );
        branch.path = Some(PathClosure {
            alias: 1,
            kind: PathKind::One,
            hop: HopExpr::Pred(crate::iq::HopRelation {
                source: LogicalSource::Table("no_pk".to_owned()),
                subj_col: "rowid".into(),
                obj_col: "value".into(),
            }),
        });
        let mut catalog = ColumnCatalog::default();
        catalog.insert(
            &LogicalSource::Table("no_pk".to_owned()),
            vec!["value".to_owned()],
        );

        assert!(
            validate_live_columns(std::slice::from_ref(&branch), Dialect::Sqlite, &catalog).is_ok()
        );
        assert!(
            validate_live_columns(std::slice::from_ref(&branch), Dialect::Postgres, &catalog)
                .is_ok()
        );
        assert!(validate_live_columns(&[branch], Dialect::MySql, &catalog).is_err());
    }

    #[test]
    fn postgres_rowid_rewrite_stops_at_a_derived_query_boundary() {
        let mut table = Branch::single(Scan {
            alias: 0,
            source: (LogicalSource::Table("no_pk".to_owned())).into(),
        });
        table
            .where_conds
            .push(SqlCond::IsNotNull(ColRef::new(0, "rowid")));
        let table_sql = emit_branch(&table, Dialect::Postgres).unwrap().sql;
        assert!(table_sql.contains("(t0.ctid)::TEXT"), "{table_sql}");

        let mut query = Branch::single(Scan {
            alias: 0,
            source: (LogicalSource::Query(
                "SELECT (sfs0.ctid)::text AS rowid FROM no_pk sfs0".to_owned(),
            ))
            .into(),
        });
        query
            .where_conds
            .push(SqlCond::IsNotNull(ColRef::new(0, "rowid")));
        let query_sql = emit_branch(&query, Dialect::Postgres).unwrap().sql;
        assert!(query_sql.contains("t0.\"rowid\""), "{query_sql}");
        assert!(!query_sql.contains("t0.ctid"), "{query_sql}");
    }

    #[test]
    fn translate_time_rowid_rendering_is_base_table_only() {
        assert_eq!(
            render_immediate_source_column("sfs0", "rowid", None, Dialect::Postgres),
            "(sfs0.ctid)::text"
        );
        assert_eq!(
            render_immediate_source_column(
                "sfs0",
                "rowid",
                Some("SELECT 7 AS rowid"),
                Dialect::Postgres,
            ),
            "sfs0.rowid"
        );
        assert_eq!(
            render_immediate_source_column(
                "sfs0",
                "rowid",
                Some("SELECT 7 AS \"rowid\""),
                Dialect::Postgres,
            ),
            "sfs0.\"rowid\""
        );

        let template = vec![
            sf_core::ir::Segment::Literal("urn:row:".into()),
            sf_core::ir::Segment::Column("rowid".into()),
        ];
        let table_sql = render_template_inline(&template, false, Dialect::Postgres, |column| {
            render_immediate_source_column("sfs0", column, None, Dialect::Postgres)
        })
        .unwrap();
        assert!(table_sql.contains("(sfs0.ctid)::text"), "{table_sql}");
        let query_sql = render_template_inline(&template, false, Dialect::Postgres, |column| {
            render_immediate_source_column(
                "sfs0",
                column,
                Some("SELECT 7 AS rowid"),
                Dialect::Postgres,
            )
        })
        .unwrap();
        assert!(query_sql.contains("sfs0.rowid"), "{query_sql}");
        assert!(!query_sql.contains("ctid"), "{query_sql}");
    }

    /// PostgreSQL regex pushdown renders `~` / `~*` with a numbered, bound param.
    #[test]
    fn pg_regex_renders_operator_and_binds_pattern() {
        let e = emit_branch(
            &branch_with(SqlCond::StrMatch {
                col: ColRef::new(0, "name"),
                op: StrMatchOp::RegexMatchI,
                param: "^a.*".to_owned(),
            }),
            Dialect::Postgres,
        )
        .unwrap();
        assert!(e.sql.contains("~*"), "{}", e.sql);
        assert!(
            e.sql.contains("$1"),
            "numbered bound placeholder: {}",
            e.sql
        );
        assert!(
            !e.sql.contains("^a"),
            "pattern must not be inlined: {}",
            e.sql
        );
        assert_eq!(e.params, vec!["^a.*".to_owned()]);
    }

    /// Rust-side reimplementation of `sf_core::ir`'s private `percent_encode_iri`,
    /// for comparison ONLY (that function isn't `pub`) — verified byte-identical
    /// to it via `sf-core`'s own `expand_writes_through_and_percent_encodes_iris`
    /// test's fixtures, reproduced inline below.
    fn reference_encode(value: &str) -> String {
        let mut out = String::new();
        for ch in value.chars() {
            if ch.is_ascii() {
                let byte = ch as u8;
                if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                    out.push(ch);
                } else {
                    out.push_str(&format!("%{byte:02X}"));
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    /// Every dialect's [`percent_encode_col`] must reconstruct EXACTLY what
    /// `sf_core::ir::percent_encode_iri` computes, across the full encodable
    /// byte range (every printable ASCII special AND every control byte,
    /// 0x00-0x1F/0x7F) plus the edge cases design-time probing against a
    /// live SQLite connection found real bugs in: an embedded NUL byte
    /// (SQLite's TEXT-mode `LENGTH` is NUL-terminated and silently
    /// truncates — closed by `CAST(... AS BLOB)`, see
    /// [`percent_encode_col_sqlite`]'s doc comment), an empty-but-non-NULL
    /// string (a naive recursive-CTE base case still fired once past the
    /// end, wrongly emitting a bare `%`), a genuinely NULL column (the
    /// aggregate collapses an empty AND a NULL input to the same result
    /// unless explicitly distinguished), and a multi-byte UTF-8 (CJK)
    /// character (each of its individual bytes is standalone-invalid UTF-8,
    /// exercising the byte-level reassembly path). SQLite only here (no
    /// live-server dependency for a routine `cargo test` run) — PostgreSQL
    /// and MySQL were validated the identical way against live servers
    /// during this fix's development; see [`percent_encode_col_postgres`]/
    /// [`percent_encode_col_mysql`]'s own doc comments for the
    /// dialect-specific bugs their first drafts had (a PG NULL/empty
    /// conflation; a MySQL `LENGTH`-vs-`SUBSTRING` byte/character unit
    /// mismatch; and MySQL `group_concat_max_len` silent truncation).
    #[test]
    fn percent_encode_col_sqlite_matches_reference_iri_encoding() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (v TEXT)", []).unwrap();

        let mut cases: Vec<Option<String>> = vec![
            Some("a b/c".to_owned()),
            Some("A-z.0_9~".to_owned()),
            Some("".to_owned()),
            None,
            Some("X/Y".to_owned()),
            Some("你好/世界".to_owned()), // non-ASCII must pass through raw
            Some("tab\ttab".to_owned()),  // control byte 0x09
            Some("nul\u{0}nul".to_owned()), // embedded NUL, 0x00
        ];
        for b in 0x20u8..=0x7e {
            cases.push(Some((b as char).to_string())); // every printable ASCII byte
        }
        for b in 0..=0x1fu8 {
            cases.push(Some((b as char).to_string())); // every control byte
        }
        cases.push(Some((0x7fu8 as char).to_string()));

        let sql = percent_encode_col("t.v", Dialect::Sqlite).expect("SQLite is supported");
        let query = format!("SELECT {sql} FROM t");
        let mut mismatches = Vec::new();
        for v in &cases {
            conn.execute("DELETE FROM t", []).unwrap();
            conn.execute("INSERT INTO t (v) VALUES (?1)", [v]).unwrap();
            let got: Option<String> = conn.query_row(&query, [], |r| r.get(0)).unwrap();
            let want = v.as_deref().map(reference_encode);
            if got != want {
                mismatches.push(format!("input={v:?} got={got:?} want={want:?}"));
            }
        }
        assert!(mismatches.is_empty(), "{mismatches:#?}");
    }

    #[test]
    fn mysql_percent_encoder_round_trips_through_the_ast_boundary() {
        let expression =
            percent_encode_col("sfs0.`value`", Dialect::MySql).expect("MySQL is supported");
        let skeleton = format!("SELECT {expression} FROM `source` sfs0");
        let emitted = Dialect::MySql
            .emit_via_ast(&skeleton)
            .expect("MySQL JSON_TABLE encoder must pass the SQL AST boundary");
        assert!(emitted.contains("JSON_TABLE"), "{emitted}");
        assert!(emitted.contains("FOR ORDINALITY"), "{emitted}");
        assert!(
            emitted.contains("group_concat_max_len = 1000000"),
            "{emitted}"
        );
        assert!(emitted.contains("LEAST(333333"), "{emitted}");
        assert!(
            emitted.contains("@@SESSION.group_concat_max_len"),
            "{emitted}"
        );
        assert!(
            emitted.contains("@@SESSION.max_allowed_packet"),
            "{emitted}"
        );
        assert!(emitted.contains("- 4096"), "{emitted}");
        assert!(
            emitted.contains("semantic-fabric-percent-encoding-input-limit"),
            "{emitted}"
        );
    }

    #[tokio::test]
    #[ignore = "requires a purpose-created isolated MySQL provider"]
    async fn mysql_percent_encoder_limit_fails_instead_of_truncating() {
        use mysql_async::prelude::Queryable;

        let socket = std::env::var("SF_MYSQL_SOCKET")
            .expect("required-live MySQL socket must be configured");
        let opts: mysql_async::Opts = mysql_async::OptsBuilder::default()
            .user(Some("root"))
            .socket(Some(socket))
            .prefer_socket(Some(true))
            .stmt_cache_size(Some(0))
            .into();
        let mut conn = mysql_async::Conn::new(opts)
            .await
            .unwrap_or_else(|_| panic!("connect to isolated MySQL provider failed"));
        let expression = percent_encode_col_mysql("source_value.value");
        let oversized_query = format!(
            "SELECT {expression} FROM \
             (SELECT REPEAT(' ', {}) AS value) AS source_value",
            MYSQL_PERCENT_ENCODE_MAX_INPUT_BYTES + 1
        );
        let result: mysql_async::Result<Option<String>> = conn.query_first(oversized_query).await;
        assert!(result.is_err(), "oversized encoding must fail closed");
        let constrained = expression.replacen(
            "SET_VAR(group_concat_max_len = 1000000)",
            "SET_VAR(group_concat_max_len = 9)",
            1,
        );
        let constrained_query = format!(
            "SELECT {constrained} FROM \
             (SELECT REPEAT(' ', 4) AS value) AS source_value"
        );
        let result: mysql_async::Result<Option<String>> = conn.query_first(constrained_query).await;
        assert!(
            result.is_err(),
            "statement-observed aggregate ceiling must fail closed"
        );
        conn.disconnect()
            .await
            .unwrap_or_else(|_| panic!("close isolated MySQL connection failed"));
    }

    #[tokio::test]
    #[ignore = "requires a purpose-created MySQL provider pinned to an 8192-byte packet ceiling"]
    async fn mysql_percent_encoder_packet_ceiling_fails_instead_of_truncating() {
        use mysql_async::prelude::Queryable;

        let socket = std::env::var("SF_MYSQL_SOCKET")
            .expect("required-live MySQL socket must be configured");
        let opts: mysql_async::Opts = mysql_async::OptsBuilder::default()
            .user(Some("root"))
            .socket(Some(socket))
            .prefer_socket(Some(true))
            .stmt_cache_size(Some(0))
            .into();
        let mut limited = mysql_async::Conn::new(opts)
            .await
            .unwrap_or_else(|_| panic!("connect to packet-bounded MySQL provider failed"));
        let observed_packet: u64 = limited
            .query_first("SELECT @@SESSION.max_allowed_packet")
            .await
            .unwrap_or_else(|_| panic!("read constrained MySQL packet ceiling failed"))
            .unwrap_or_else(|| panic!("constrained MySQL packet ceiling is absent"));
        assert_eq!(
            observed_packet, 8192,
            "packet-bound evidence requires the exact isolated provider profile"
        );
        let packet_bound = (observed_packet.saturating_sub(MYSQL_PACKET_RESERVE_BYTES as u64)) / 3;
        let expression = percent_encode_col_mysql("source_value.value");
        let packet_query = format!(
            "SELECT {expression} FROM \
             (SELECT REPEAT(' ', {}) AS value) AS source_value",
            packet_bound + 1
        );
        let packet_result: mysql_async::Result<Option<String>> =
            limited.query_first(packet_query).await;
        limited
            .disconnect()
            .await
            .unwrap_or_else(|_| panic!("close constrained MySQL connection failed"));
        assert!(
            packet_result.is_err(),
            "statement-observed packet ceiling must fail closed"
        );
    }

    /// A dialect this module does not implement encoding for (Oracle, picked
    /// arbitrarily) declines soundly rather than guessing.
    #[test]
    fn percent_encode_col_unsupported_dialect_is_501() {
        assert!(matches!(
            percent_encode_col("t.v", Dialect::Oracle),
            Err(Error::Unsupported(_))
        ));
    }
}
