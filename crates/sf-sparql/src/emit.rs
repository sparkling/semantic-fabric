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
use sf_sql::backend::{NativeScalarKey, SqliteDecode, TextKey};
use sf_sql::Dialect;

use crate::iq::{
    collect_cond_cols, AggCol, AggKind, Aggregation, Branch, ColRef, HopExpr, OrderKey,
    PathClosure, PathKind, R2rmlGraphScope, SqlCond, StrMatchOp, TermDef,
};
use crate::{Error, Result};
mod metadata;
mod metadata_path;
mod metadata_projection;
mod metadata_ref_atom;
mod metadata_source;
mod metadata_subplan;
use metadata::branch_actuals_controlled;
mod condition_control;
mod condition_leaf;
mod condition_metadata;
mod path_sql;
mod scan;
mod source_control;
#[cfg(test)]
use metadata_source::source_actuals;
use metadata_source::source_actuals_controlled;
#[cfg(test)]
mod metadata_tests;
#[cfg(test)]
use scan::scan_actuals;
use scan::scan_ref_controlled;
pub(crate) use source_control::{
    live_metadata_sources_controlled, source_probe_controlled, validate_definition_columns,
    SourceSet,
};
mod aggregate_projection;
pub(crate) use aggregate_projection::projection_controlled;
#[cfg(test)]
#[path = "emit/encoding_tests.rs"]
mod encoding_tests;
mod encoding_ucschar;
mod iri_cmp;
mod projection_layout;
#[cfg(test)]
pub(crate) use projection_layout::projection_layout_with_distinct;
pub(crate) use projection_layout::{projection_layout, source_projection};
mod lexical_key;
mod literal_cmp;
mod literal_datatype;
mod literal_roles;
mod mysql_decimal_value;
mod mysql_float_value;
mod native_literal_key;
mod natural_decimal;
mod natural_literal;
mod path_comparison;
mod pg_decimal_value;
mod pg_float;
mod pg_float_value;
mod pg_numeric;
mod ref_atom;
use aggregate_projection::{aggregate_projection, AggregateProjection};
#[cfg(test)]
use path_comparison::subplan_actuals;
use path_comparison::{path_actuals_controlled, render_key_equality};

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
    scalars_by_source: std::sync::Arc<HashMap<String, HashMap<String, NativeScalarKey>>>,
    datatypes_by_source:
        std::sync::Arc<HashMap<String, HashMap<String, sf_core::datatype::XsdTypeCode>>>,
    suppress_path_collation: bool,
    character_keys: std::sync::Arc<std::sync::atomic::AtomicBool>,
    lexical_keys: std::sync::Arc<std::sync::atomic::AtomicBool>,
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

    #[cfg(test)]
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

/// Every live logical source nested in `branches`, in deterministic encounter
/// order. The executor uses a paid, variant-tagged [`SourceSet`] before probing;
/// keeping enumeration and identity separate makes the fail-closed ordering
/// explicit at the I/O boundary. This raw traversal is the semantic oracle.
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
#[cfg(test)]
pub(crate) fn validate_live_columns(
    branches: &[Branch],
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> Result<()> {
    validate_live_columns_controlled(
        branches,
        dialect,
        catalog,
        sf_sql::source_work::SourceWork::new(None),
    )
}

#[cfg(test)]
pub(crate) fn validate_live_columns_controlled(
    branches: &[Branch],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<()> {
    validate_source_root(ValidationRoot::Branches(branches), dialect, catalog, work)
}

pub(crate) fn validate_execution_columns(
    branches: &[Branch],
    scopes: &[Option<crate::DedupScope>],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<()> {
    validate_source_root(
        ValidationRoot::Execution(branches, scopes),
        dialect,
        catalog,
        work,
    )
}

pub(super) enum ValidationRoot<'a> {
    Execution(&'a [Branch], &'a [Option<crate::DedupScope>]),
    #[cfg(test)]
    Branches(&'a [Branch]),
    #[cfg(test)]
    Projection(
        &'a crate::iq::Scan,
        &'a [(Box<str>, TermMap)],
        &'a [SqlCond],
    ),
}

/// One queue owns every Branch/Scan/EXISTS/Projection continuation. Leaf checks
/// cannot recurse back into this driver; scopes are restored before resuming.
pub(super) fn validate_source_root(
    root: ValidationRoot<'_>,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<()> {
    use crate::iq::{Scan, ScanSource, SubPlanJoin};
    use sf_sql::source_work::SourceVec;
    use source_control::validation_error as error;
    work.charge(1).map_err(error)?;
    #[derive(Clone, Copy)]
    enum AliasSource<'a> {
        Base(&'a LogicalSource),
        Derived,
        Path,
        Projection(&'a [(Box<str>, TermMap)]),
        RefAtom(usize),
    }

    #[derive(Default)]
    struct AliasMap<'a>(sf_sql::source_work::SourceVec<(usize, AliasSource<'a>)>);
    impl<'a> AliasMap<'a> {
        fn position(
            &self,
            alias: usize,
            work: sf_sql::source_work::SourceWork<'_>,
        ) -> Result<std::result::Result<usize, usize>> {
            work.checkpoint()
                .map_err(source_control::validation_error)?;
            let (mut left, mut right) = (0, self.0.as_slice().len());
            while left < right {
                work.charge(1).map_err(source_control::validation_error)?;
                let middle = left + (right - left) / 2;
                match self.0.as_slice()[middle].0.cmp(&alias) {
                    std::cmp::Ordering::Equal => return Ok(Ok(middle)),
                    std::cmp::Ordering::Less => left = middle + 1,
                    std::cmp::Ordering::Greater => right = middle,
                }
            }
            Ok(Err(left))
        }
        fn get(
            &self,
            alias: usize,
            work: sf_sql::source_work::SourceWork<'_>,
        ) -> Result<Option<AliasSource<'a>>> {
            Ok(self
                .position(alias, work)?
                .ok()
                .map(|index| self.0.as_slice()[index].1))
        }
        fn insert(
            &mut self,
            alias: usize,
            source: AliasSource<'a>,
            work: sf_sql::source_work::SourceWork<'_>,
        ) -> Result<()> {
            match self.position(alias, work)? {
                Ok(index) => self.0.replace_copy(index, (alias, source), work),
                Err(index) => self.0.insert(index, (alias, source), work),
            }
            .map_err(source_control::validation_error)
        }
        fn copy(&self, work: sf_sql::source_work::SourceWork<'_>) -> Result<Self> {
            work.checkpoint()
                .map_err(source_control::validation_error)?;
            let mut output = Self::default();
            for entry in self.0.as_slice() {
                output
                    .0
                    .push(*entry, work)
                    .map_err(source_control::validation_error)?;
            }
            Ok(output)
        }
    }

    fn validate_ref(
        column: &ColRef,
        aliases: &AliasMap<'_>,
        dialect: Dialect,
        catalog: &ColumnCatalog,
        work: sf_sql::source_work::SourceWork<'_>,
    ) -> Result<()> {
        work.charge(1).map_err(source_control::validation_error)?;
        validate_named_ref(
            column.alias,
            &column.column,
            aliases,
            dialect,
            catalog,
            work,
        )
    }

    fn validate_named_ref(
        alias: usize,
        column: &str,
        aliases: &AliasMap<'_>,
        dialect: Dialect,
        catalog: &ColumnCatalog,
        work: sf_sql::source_work::SourceWork<'_>,
    ) -> Result<()> {
        work.charge(1).map_err(source_control::validation_error)?;
        let resolved = aliases.get(alias, work)?;
        if let Some(AliasSource::Base(source)) = resolved {
            catalog.validate_live_column_controlled(source, column, dialect, work)?;
        }
        if let Some(AliasSource::Projection(columns)) = resolved {
            scan::validate_output_controlled(columns, column, work)?;
        }
        if let Some(AliasSource::RefAtom(width)) = resolved {
            ref_atom::validate_output_controlled(width, column, work)?;
        }
        if matches!(resolved, Some(AliasSource::Path)) && !matches!(column, "sf_s" | "sf_o") {
            return Err(Error::Sql(
                "path relation has no required output column".into(),
            ));
        }
        Ok(())
    }

    fn validate_hop(
        hop: &HopExpr,
        dialect: Dialect,
        catalog: &ColumnCatalog,
        work: sf_sql::source_work::SourceWork<'_>,
    ) -> Result<()> {
        let mut pending = sf_sql::source_work::SourceVec::default();
        pending
            .push(hop, work)
            .map_err(source_control::validation_error)?;
        while let Some(hop) = pending.pop() {
            work.charge(1).map_err(source_control::validation_error)?;
            match hop {
                HopExpr::Pred(relation) => {
                    catalog.validate_live_column_controlled(
                        &relation.source,
                        &relation.subj_col,
                        dialect,
                        work,
                    )?;
                    catalog.validate_live_column_controlled(
                        &relation.source,
                        &relation.obj_col,
                        dialect,
                        work,
                    )?;
                }
                HopExpr::Inverse(inner) => pending
                    .push(inner.as_ref(), work)
                    .map_err(source_control::validation_error)?,
                HopExpr::Seq(left, right) => {
                    pending
                        .push(right.as_ref(), work)
                        .map_err(source_control::validation_error)?;
                    pending
                        .push(left.as_ref(), work)
                        .map_err(source_control::validation_error)?;
                }
                HopExpr::Alt(parts) | HopExpr::Nps(parts) => {
                    for part in parts.iter().rev() {
                        pending
                            .push(part, work)
                            .map_err(source_control::validation_error)?;
                    }
                }
            }
        }
        work.checkpoint().map_err(source_control::validation_error)
    }

    enum Task<'a> {
        Execution(&'a [Branch], &'a [Option<crate::DedupScope>]),
        Branches(&'a [Branch]),
        Scans(&'a [Scan]),
        OptScans(&'a [crate::iq::OptJoin]),
        Conditions(&'a [SqlCond]),
        OptConditions(&'a [crate::iq::OptJoin]),
        Joins(&'a [SubPlanJoin]),
        Enter(&'a Branch, Option<&'a crate::DedupScope>),
        Body(&'a Branch, Option<&'a crate::DedupScope>),
        Finish(&'a Branch),
        Join(&'a SubPlanJoin),
        Scan(&'a Scan),
        Install(usize, AliasSource<'a>),
        Condition(&'a SqlCond),
        Leave,
        Projection(&'a Scan, &'a [(Box<str>, TermMap)], &'a [SqlCond]),
        ProjectionInput(&'a Scan, &'a [(Box<str>, TermMap)], &'a [SqlCond]),
        ProjectionOutput(&'a Scan, &'a [(Box<str>, TermMap)], &'a [SqlCond]),
    }
    let mut pending = SourceVec::default();
    let mut scopes: SourceVec<AliasMap<'_>> = Default::default();
    match root {
        ValidationRoot::Execution(branches, scopes) => pending
            .push(Task::Execution(branches, scopes), work)
            .map_err(error)?,
        #[cfg(test)]
        ValidationRoot::Branches(branches) => pending
            .push(Task::Branches(branches), work)
            .map_err(error)?,
        #[cfg(test)]
        ValidationRoot::Projection(input, columns, guards) => pending
            .push(Task::Projection(input, columns, guards), work)
            .map_err(error)?,
    }
    while let Some(task) = pending.pop() {
        work.charge(1).map_err(error)?;
        match task {
            Task::Execution(items, scopes) => {
                if let Some((first, rest)) = items.split_first() {
                    let (scope, remaining) = scopes
                        .split_first()
                        .map_or((None, scopes), |(scope, rest)| (scope.as_ref(), rest));
                    pending
                        .push(Task::Execution(rest, remaining), work)
                        .map_err(error)?;
                    pending
                        .push(Task::Enter(first, scope), work)
                        .map_err(error)?;
                }
            }
            Task::Branches(items) => {
                if let Some((first, rest)) = items.split_first() {
                    pending.push(Task::Branches(rest), work).map_err(error)?;
                    pending
                        .push(Task::Enter(first, None), work)
                        .map_err(error)?;
                }
            }
            Task::Scans(items) => {
                if let Some((first, rest)) = items.split_first() {
                    pending.push(Task::Scans(rest), work).map_err(error)?;
                    pending.push(Task::Scan(first), work).map_err(error)?;
                }
            }
            Task::OptScans(items) => {
                if let Some((first, rest)) = items.split_first() {
                    pending.push(Task::OptScans(rest), work).map_err(error)?;
                    pending.push(Task::Scan(&first.scan), work).map_err(error)?;
                }
            }
            Task::Conditions(items) => {
                if let Some((first, rest)) = items.split_first() {
                    pending.push(Task::Conditions(rest), work).map_err(error)?;
                    pending.push(Task::Condition(first), work).map_err(error)?;
                }
            }
            Task::OptConditions(items) => {
                if let Some((first, rest)) = items.split_first() {
                    pending
                        .push(Task::OptConditions(rest), work)
                        .map_err(error)?;
                    pending
                        .push(Task::Conditions(&first.extra), work)
                        .map_err(error)?;
                    pending
                        .push(Task::Conditions(&first.on), work)
                        .map_err(error)?;
                }
            }
            Task::Joins(items) => {
                if let Some((first, rest)) = items.split_first() {
                    pending.push(Task::Joins(rest), work).map_err(error)?;
                    pending.push(Task::Join(first), work).map_err(error)?;
                }
            }
            Task::Enter(branch, scope) => {
                scopes.push(AliasMap::default(), work).map_err(error)?;
                pending
                    .push(Task::Body(branch, scope), work)
                    .map_err(error)?;
                pending
                    .push(Task::OptScans(&branch.opts), work)
                    .map_err(error)?;
                pending
                    .push(Task::Scans(&branch.core), work)
                    .map_err(error)?;
            }
            Task::Scan(scan) => match &scan.source {
                ScanSource::Logical(source) => pending
                    .push(Task::Install(scan.alias, AliasSource::Base(source)), work)
                    .map_err(error)?,
                ScanSource::Path { closure, .. } => {
                    validate_hop(&closure.hop, dialect, catalog, work)?;
                    pending
                        .push(Task::Install(scan.alias, AliasSource::Path), work)
                        .map_err(error)?;
                }
                ScanSource::RefAtom { input, columns } => {
                    ref_atom::validate_shape_controlled(input, columns, work)?;
                    pending
                        .push(
                            Task::Install(scan.alias, AliasSource::RefAtom(columns.len())),
                            work,
                        )
                        .map_err(error)?;
                    pending
                        .push(Task::Enter(input, None), work)
                        .map_err(error)?;
                }
                ScanSource::Projection {
                    input,
                    columns,
                    guards,
                    ..
                } => {
                    pending
                        .push(
                            Task::Install(scan.alias, AliasSource::Projection(columns)),
                            work,
                        )
                        .map_err(error)?;
                    pending
                        .push(Task::Projection(input, columns, guards), work)
                        .map_err(error)?;
                }
            },
            Task::Install(alias, source) => {
                let mut aliases = scopes.pop().expect("scan installation has a caller scope");
                aliases.insert(alias, source, work)?;
                scopes.push(aliases, work).map_err(error)?;
            }
            Task::Body(branch, scope) => {
                let mut aliases = scopes.pop().expect("branch body has a scope");
                for subplan in &branch.subplan_joins {
                    aliases.insert(subplan.alias, AliasSource::Derived, work)?;
                }
                let bindings = BindingView::merged(
                    &branch.bindings,
                    scope.map(|scope| &scope.key_bindings),
                    work,
                )?;
                for (_, definition) in bindings.iter() {
                    source_control::validate_definition_columns(
                        definition,
                        work,
                        |alias, column| {
                            validate_named_ref(alias, column, &aliases, dialect, catalog, work)
                        },
                    )?;
                }
                scopes.push(aliases, work).map_err(error)?;
                pending.push(Task::Finish(branch), work).map_err(error)?;
                pending
                    .push(Task::Joins(&branch.subplan_joins), work)
                    .map_err(error)?;
                pending
                    .push(Task::OptConditions(&branch.opts), work)
                    .map_err(error)?;
                pending
                    .push(Task::Conditions(&branch.where_conds), work)
                    .map_err(error)?;
            }
            Task::Join(join) => {
                pending
                    .push(Task::Branches(&join.plan.branches), work)
                    .map_err(error)?;
                pending
                    .push(Task::Conditions(&join.on), work)
                    .map_err(error)?;
            }
            Task::Finish(branch) => {
                let aliases = scopes.pop().expect("branch finish has a scope");
                if let Some(path) = &branch.path {
                    validate_hop(&path.hop, dialect, catalog, work)?;
                }
                if let Some(aggregation) = &branch.agg {
                    for key in &aggregation.keys {
                        work.charge(1).map_err(error)?;
                        for column in &key.cols {
                            validate_ref(column, &aliases, dialect, catalog, work)?;
                        }
                    }
                    for aggregate in &aggregation.aggs {
                        work.charge(1).map_err(error)?;
                        if let Some(column) = &aggregate.arg {
                            validate_ref(column, &aliases, dialect, catalog, work)?;
                        }
                    }
                }
            }
            Task::Leave => {
                scopes.pop().expect("condition exit has a scope");
            }
            Task::Condition(condition) => {
                let aliases = scopes.as_slice().last().expect("condition has a scope");
                let result: Result<()> = match condition {
                    SqlCond::Not(inner) => {
                        pending.push(Task::Condition(inner), work).map_err(error)
                    }
                    SqlCond::And(parts) | SqlCond::Or(parts) => {
                        pending.push(Task::Conditions(parts), work).map_err(error)
                    }
                    SqlCond::Exists { scans, conds } | SqlCond::NotExists { scans, conds } => {
                        let nested = aliases.copy(work)?;
                        scopes.push(nested, work).map_err(error)?;
                        pending.push(Task::Leave, work).map_err(error)?;
                        pending.push(Task::Conditions(conds), work).map_err(error)?;
                        pending.push(Task::Scans(scans), work).map_err(error)?;
                        Ok(())
                    }
                    SqlCond::PathExists { pc, conds, .. } => {
                        validate_hop(&pc.hop, dialect, catalog, work)?;
                        pending.push(Task::Conditions(conds), work).map_err(error)?;
                        Ok(())
                    }
                    SqlCond::ExpressionError => Ok(()),
                    SqlCond::IriCmp(cmp) => {
                        use crate::iq::iri_cmp::{IriOperand, IriPart};
                        for operand in [&cmp.left, &cmp.right] {
                            work.charge(1).map_err(source_control::validation_error)?;
                            match operand {
                                IriOperand::Column { column, .. } => {
                                    validate_ref(column, aliases, dialect, catalog, work)?
                                }
                                IriOperand::Template { parts, .. } => {
                                    for part in parts {
                                        work.charge(1).map_err(source_control::validation_error)?;
                                        if let IriPart::Column(column) = part {
                                            validate_ref(column, aliases, dialect, catalog, work)?;
                                        }
                                    }
                                }
                                IriOperand::Constant(_) => {}
                            }
                        }
                        Ok(())
                    }
                    SqlCond::LiteralCmp(cmp) => {
                        for column in cmp.columns() {
                            validate_ref(column, aliases, dialect, catalog, work)?;
                        }
                        Ok(())
                    }
                    SqlCond::ColEq(left, right)
                    | SqlCond::NativeColEq(left, right)
                    | SqlCond::NullSafeEq(left, right) => {
                        validate_ref(left, aliases, dialect, catalog, work)?;
                        validate_ref(right, aliases, dialect, catalog, work)
                    }
                    SqlCond::Cmp(column, _, _)
                    | SqlCond::NativeCmp(column, _, _)
                    | SqlCond::IsNotNull(column)
                    | SqlCond::DecodedIsNotNull(column)
                    | SqlCond::IsNull(column)
                    | SqlCond::StrMatch { col: column, .. } => {
                        validate_ref(column, aliases, dialect, catalog, work)
                    }
                    SqlCond::TemplateEq(left, left_alias, right, right_alias, _) => {
                        for segment in left {
                            work.charge(1).map_err(source_control::validation_error)?;
                            if let Segment::Column(column) = segment {
                                validate_named_ref(
                                    *left_alias,
                                    column,
                                    aliases,
                                    dialect,
                                    catalog,
                                    work,
                                )?;
                            }
                        }
                        for segment in right {
                            work.charge(1).map_err(source_control::validation_error)?;
                            if let Segment::Column(column) = segment {
                                validate_named_ref(
                                    *right_alias,
                                    column,
                                    aliases,
                                    dialect,
                                    catalog,
                                    work,
                                )?;
                            }
                        }
                        Ok(())
                    }
                };
                result?;
            }
            Task::Projection(input, columns, guards) => {
                pending
                    .push(Task::ProjectionInput(input, columns, guards), work)
                    .map_err(error)?;
                if let ScanSource::Projection {
                    input,
                    columns,
                    guards,
                    ..
                } = &input.source
                {
                    pending
                        .push(Task::Projection(input, columns, guards), work)
                        .map_err(error)?;
                }
            }
            Task::ProjectionInput(input, columns, guards) => {
                pending
                    .push(Task::ProjectionOutput(input, columns, guards), work)
                    .map_err(error)?;
                if let ScanSource::RefAtom { input, columns } = &input.source {
                    ref_atom::validate_shape_controlled(input, columns, work)?;
                    pending
                        .push(Task::Enter(input, None), work)
                        .map_err(error)?;
                }
            }
            Task::ProjectionOutput(input, columns, guards) => {
                projection_layout::validate_projection_level(
                    input, columns, guards, dialect, catalog, work,
                )?
            }
        }
    }
    work.checkpoint().map_err(error)
}

fn physical_row_identifier(source: &LogicalSource, raw: &str, dialect: Dialect) -> bool {
    matches!(source, LogicalSource::Table(_))
        && raw == "rowid"
        && matches!(dialect, Dialect::Postgres | Dialect::Sqlite)
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
    synthetic_subplan_catalog_controlled(branches, sf_sql::source_work::SourceWork::new(None))
        .expect("uncontrolled synthetic catalog cannot be refused")
}

fn synthetic_subplan_catalog_controlled(
    branches: &[Branch],
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<ColumnCatalog> {
    let mut catalog = ColumnCatalog::default();
    for b in branches {
        work.charge(1 + b.core.len() + b.opts.len() + b.subplan_joins.len())
            .map_err(source_control::validation_error)?;
        let sources: HashMap<usize, &LogicalSource> = b.alias_sources().into_iter().collect();
        let mut cols: Vec<ColRef> = Vec::new();
        let mut refused = None;
        // Pay each column reference before copying it; a refusal stops the walk.
        let mut push = |c: &ColRef| {
            if refused.is_none() {
                match work.charge(1 + c.column.len()) {
                    Ok(()) => cols.push(c.clone()),
                    Err(error) => refused = Some(error),
                }
            }
        };
        for def in b.bindings.values() {
            for c in def.columns() {
                push(&c);
            }
        }
        for cond in &b.where_conds {
            collect_cond_cols(cond, &mut push);
        }
        for opt in &b.opts {
            for cond in opt.on.iter().chain(opt.extra.iter()) {
                collect_cond_cols(cond, &mut push);
            }
        }
        for sp in &b.subplan_joins {
            for cond in &sp.on {
                collect_cond_cols(cond, &mut push);
            }
        }
        if let Some(error) = refused {
            return Err(source_control::validation_error(error));
        }
        let mut seen: std::collections::HashSet<(usize, &str)> = std::collections::HashSet::new();
        let mut merged: HashMap<usize, usize> = HashMap::new();
        for c in &cols {
            let Some(LogicalSource::Query(sql)) = sources.get(&c.alias) else {
                continue;
            };
            // Each distinct column reference scans the query text once.
            work.charge(1 + c.column.len())
                .map_err(source_control::validation_error)?;
            if !seen.insert((c.alias, &c.column)) {
                continue;
            }
            // Lowercase copy and alias scans of the query text, the key copy,
            // the lowercased column and the membership scan over the columns
            // already merged for this alias.
            work.product(sql.len(), 4)
                .map_err(source_control::validation_error)?;
            work.charge(1 + c.column.len() + merged.get(&c.alias).copied().unwrap_or(0))
                .map_err(source_control::validation_error)?;
            if crate::cascade::col_is_unquoted_alias(sql, &c.column) {
                catalog.merge(sources[&c.alias], c.column.to_lowercase());
                *merged.entry(c.alias).or_insert(0) += 1;
            }
        }
    }
    Ok(catalog)
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct AliasActuals {
    // Effective source datatype is not canonical-key or output-cast authority.
    datatype_columns: HashMap<String, Option<sf_core::datatype::XsdTypeCode>>,
    // None retains incompatible natural provenance; it must not fall back to
    // native equality after a coercing SubPlan loses a common decoder.
    natural_columns: HashMap<String, Option<sf_core::datatype::XsdTypeCode>>,
    scalar_columns: HashMap<String, NativeScalarKey>,
    source_kind: AliasSourceKind,
    columns: Vec<String>,
    path: bool,
    text_columns: HashMap<String, TextKey>,
    // Compiler-generated, already absolute static-template IRIs. This is not
    // inherited from arbitrary source text and is intersected at SubPlan joins.
    static_iri_columns: HashSet<String>,
    // Bounded compiler-rendered float text uses only IRI-unreserved bytes.
    // This permits encoding elision, not native scalar or absolute-IRI authority.
    iri_unreserved_columns: HashSet<String>,
    sqlite_columns: HashMap<String, SqliteDecode>,
    lexical_columns: HashMap<String, SqliteDecode>,
    // A retained decoded consumer can license an aligned RDF comparison while
    // another natural/typed consumer still forbids replacing the raw payload.
    lexical_comparison_columns: HashMap<String, SqliteDecode>,
}

type ActualColumns = HashMap<usize, AliasActuals>;

/// The source kind and actual columns of every scan alias in `b`, keyed by alias,
/// for physical-row handling and identifier resolution.
/// For SubPlan-join aliases, the "actual columns" are the projected variable names
/// from the nested Plan's `PlanForm::Select { vars }` (the names the derived table
/// exposes). SubPlan aliases are NOT in `alias_sources()` (they have no catalog
/// entry), so they are wired up here directly.
#[cfg(test)]
fn branch_actuals(b: &Branch, dialect: Dialect, catalog: &ColumnCatalog) -> ActualColumns {
    branch_actuals_controlled(
        b,
        dialect,
        catalog,
        sf_sql::source_work::SourceWork::new(None),
    )
    .expect("uncontrolled branch metadata construction")
}

/// A branch rendered to one parameterised SQL `SELECT`.
pub struct EmittedBranch {
    pub sql: String,
    /// Prepare-only same-IR SQL without engine-added collation decorations.
    /// Never executed; preserves SQLite native result decoding through COLLATE.
    pub metadata_sql: Option<String>,
    /// Engine-generated decoder call, never inferred from authored SQL text.
    pub sqlite_character_keys: bool,
    /// Query-owned lexical decoder and literal numeric comparison callbacks.
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
    emit_branch_with_modifiers(b, dialect, catalog, BranchModifiers::stored(b))
}

/// Borrowed, ordered execution bindings; shared names retain the base recipe.
pub(crate) enum BindingView<'a> {
    Direct(&'a std::collections::BTreeMap<String, TermDef>),
    Merged(Vec<(&'a str, &'a TermDef)>),
}

impl<'a> BindingView<'a> {
    /// Caller has proved equality for shared names. Preserve base ownership and
    /// BTreeMap order while admitting only borrowed pointer-vector storage.
    pub(crate) fn merged(
        base: &'a std::collections::BTreeMap<String, TermDef>,
        overlay: Option<&'a std::collections::BTreeMap<String, TermDef>>,
        work: sf_sql::source_work::SourceWork<'_>,
    ) -> Result<Self> {
        use source_control::validation_error as error;
        work.checkpoint().map_err(error)?;
        let Some(overlay) = overlay.filter(|map| !map.is_empty()) else {
            return Ok(Self::Direct(base));
        };
        let (mut left, mut right) = (base.iter().peekable(), overlay.iter().peekable());
        let mut entries = sf_sql::source_work::SourceVec::default();
        while left.peek().is_some() || right.peek().is_some() {
            work.charge(1).map_err(error)?;
            let ordering = match (left.peek(), right.peek()) {
                (Some((a, _)), Some((b, _))) => {
                    work.charge(a.len().min(b.len())).map_err(error)?;
                    a.cmp(b)
                }
                (Some(_), None) => std::cmp::Ordering::Less,
                _ => std::cmp::Ordering::Greater,
            };
            let (name, definition) = match ordering {
                std::cmp::Ordering::Less => left.next().expect("left entry"),
                std::cmp::Ordering::Greater => right.next().expect("right entry"),
                std::cmp::Ordering::Equal => {
                    right.next();
                    left.next().expect("equal base entry")
                }
            };
            entries
                .push((name.as_str(), definition), work)
                .map_err(error)?;
        }
        work.checkpoint().map_err(error)?;
        Ok(Self::Merged(entries.into_vec()))
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Direct(map) => map.len(),
            Self::Merged(entries) => entries.len(),
        }
    }

    fn get(
        &self,
        name: &str,
        work: sf_sql::source_work::SourceWork<'_>,
    ) -> Result<Option<&'a TermDef>> {
        for (candidate, definition) in self.iter() {
            work.charge(1).map_err(source_control::validation_error)?;
            work.charge(candidate.len().min(name.len()))
                .map_err(source_control::validation_error)?;
            match candidate.cmp(name) {
                std::cmp::Ordering::Equal => return Ok(Some(definition)),
                std::cmp::Ordering::Greater => break,
                std::cmp::Ordering::Less => {}
            }
        }
        work.checkpoint()
            .map_err(source_control::validation_error)?;
        Ok(None)
    }

    pub(crate) fn iter(&self) -> BindingIter<'_, 'a> {
        match self {
            Self::Direct(map) => BindingIter::Direct(map.iter()),
            Self::Merged(entries) => BindingIter::Merged(entries.iter()),
        }
    }
}

pub(crate) enum BindingIter<'view, 'a> {
    Direct(std::collections::btree_map::Iter<'a, String, TermDef>),
    Merged(std::slice::Iter<'view, (&'a str, &'a TermDef)>),
}

impl<'a> Iterator for BindingIter<'_, 'a> {
    type Item = (&'a str, &'a TermDef);
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Direct(iter) => iter
                .next()
                .map(|(name, definition)| (name.as_str(), definition)),
            Self::Merged(iter) => iter.next().copied(),
        }
    }
}

/// Scalar execution overlay; preparing a root never copies its branch forest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BranchModifiers {
    pub(crate) distinct: bool,
    limit: Option<usize>,
    offset: usize,
}

impl BranchModifiers {
    pub(crate) fn stored(b: &Branch) -> Self {
        Self {
            distinct: b.distinct,
            limit: b.limit,
            offset: b.offset,
        }
    }

    pub(crate) fn prepared(
        b: &Branch,
        single: bool,
        distinct: bool,
        unordered: bool,
        limit: Option<usize>,
        offset: usize,
    ) -> Self {
        let mut m = Self::stored(b);
        if single {
            m.distinct = distinct;
            if unordered {
                m.limit = limit;
                m.offset = offset;
            }
        }
        m
    }
}

pub(crate) fn emit_branch_with_modifiers(
    b: &Branch,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    modifiers: BranchModifiers,
) -> Result<EmittedBranch> {
    emit_branch_controlled(
        b,
        dialect,
        catalog,
        modifiers,
        sf_sql::source_work::SourceWork::new(None),
    )
}

pub(crate) fn emit_branch_controlled(
    b: &Branch,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    modifiers: BranchModifiers,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<EmittedBranch> {
    emit_branch_binding_view(
        b,
        &BindingView::Direct(&b.bindings),
        dialect,
        catalog,
        modifiers,
        work,
    )
}

pub(crate) fn emit_branch_binding_view(
    b: &Branch,
    bindings: &BindingView<'_>,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    modifiers: BranchModifiers,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<EmittedBranch> {
    work.checkpoint()
        .map_err(source_control::validation_error)?;
    let mut emission_catalog = catalog.clone();
    emission_catalog.character_keys = Default::default();
    emission_catalog.lexical_keys = Default::default();
    let catalog = &emission_catalog;
    let mut emitted = emit_branch_inner(b, bindings, dialect, catalog, modifiers, work)?;
    if dialect == Dialect::Sqlite && !catalog.suppress_path_collation {
        let mut metadata_catalog = catalog.clone();
        metadata_catalog.suppress_path_collation = true;
        let metadata = emit_branch_inner(b, bindings, dialect, &metadata_catalog, modifiers, work)?;
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
    bindings: &BindingView<'_>,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    modifiers: BranchModifiers,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<EmittedBranch> {
    emit_branch_keys(
        b,
        bindings,
        dialect,
        catalog,
        modifiers.distinct,
        modifiers,
        work,
    )
}

fn emit_branch_keys(
    b: &Branch,
    bindings: &BindingView<'_>,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    normalize_projection: bool,
    modifiers: BranchModifiers,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<EmittedBranch> {
    let actuals = branch_actuals_controlled(b, dialect, catalog, work)?;
    if let Some(pc) = &b.path {
        return emit_path_branch(b, pc, dialect, catalog, modifiers, work);
    }
    if let Some(agg) = &b.agg {
        return emit_agg_branch(b, agg, dialect, catalog, &actuals, modifiers, work);
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
    let (projection, term_dedup) = aggregate_projection::projection_from_bindings(
        b,
        bindings,
        dialect,
        modifiers.distinct,
        work,
    )?;
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
        Some(render_from_controlled(
            b,
            dialect,
            catalog,
            &actuals,
            &mut params,
            &mut pidx,
            work,
        )?)
    };
    let where_sql = render_where(
        &b.where_conds,
        dialect,
        catalog,
        &actuals,
        &mut params,
        &mut pidx,
        work,
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
    let numeric_keys = if modifiers.distinct
        && !term_dedup
        && matches!(dialect, Dialect::Postgres | Dialect::MySql)
    {
        pg_numeric::distinct_keys_for_projection(bindings, dialect, &actuals, &projection, work)?
    } else {
        Vec::new()
    };
    let numeric_distinct =
        modifiers.distinct && !term_dedup && numeric_keys.iter().any(Option::is_some);
    let literal_window = if modifiers.distinct && !term_dedup && dialect == Dialect::Sqlite {
        literal_roles::sqlite_distinct(bindings, catalog, &actuals, &projection, work)?
    } else {
        None
    };
    let (select_list, literal_output) = match &literal_window {
        Some(keys) => {
            let raw = projection
                .iter()
                .enumerate()
                .map(|(i, c)| format!("{} AS c{i}", colref(c, dialect, &actuals)))
                .collect::<Vec<_>>()
                .join(", ");
            let (select, output) = literal_roles::window(&raw, keys, projection.len());
            (select, Some(output))
        }
        None => (select_list, None),
    };
    let distinct =
        if modifiers.distinct && !term_dedup && !numeric_distinct && literal_window.is_none() {
            "DISTINCT "
        } else {
            ""
        };
    work.charge(select_list.len() + from.as_deref().map_or(0, str::len) + 64)
        .map_err(source_control::validation_error)?;
    let mut skeleton = match from {
        Some(f) => format!("SELECT {distinct}{select_list} FROM {f}"),
        None => format!("SELECT {distinct}{select_list}"),
    };
    if let Some(w) = where_sql {
        work.charge(w.len() + 7)
            .map_err(source_control::validation_error)?;
        skeleton.push_str(" WHERE ");
        skeleton.push_str(&w);
    }
    if numeric_distinct {
        work.charge(skeleton.len())
            .map_err(source_control::validation_error)?;
        work.product(numeric_keys.len(), 96)
            .map_err(source_control::validation_error)?;
        skeleton = pg_numeric::distinct_sql(skeleton, &numeric_keys);
    }
    if let Some(output) = literal_output {
        work.charge(skeleton.len() + output.len() + 96)
            .map_err(source_control::validation_error)?;
        skeleton = format!("SELECT {output} FROM ({skeleton}) __sf_literal_raw WHERE __sf_literal_raw.__sf_literal_rank = 1");
    }
    // ORDER BY precedes LIMIT/OFFSET (SPARQL §15: order, then slice).
    if let Some(order) = if numeric_distinct || literal_window.is_some() {
        pg_numeric::order(b, bindings, &projection, dialect, work)?
    } else {
        render_order(
            &b.order,
            bindings,
            dialect,
            catalog,
            &actuals,
            normalize_projection && !term_dedup,
            work,
        )?
    } {
        work.charge(order.len())
            .map_err(source_control::validation_error)?;
        skeleton.push_str(&order);
    }
    push_limit_offset(&mut skeleton, modifiers, dialect);

    source_control::sql_parse(&skeleton, work).map_err(source_control::validation_error)?;
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
fn push_limit_offset(skeleton: &mut String, m: BranchModifiers, dialect: Dialect) {
    match m.limit {
        Some(limit) => skeleton.push_str(&format!(" LIMIT {limit}")),
        None if m.offset > 0 => {
            if let Some(sentinel) = dialect.bare_offset_limit_sentinel() {
                skeleton.push_str(&format!(" LIMIT {sentinel}"));
            }
        }
        None => {}
    }
    if m.offset > 0 {
        skeleton.push_str(&format!(" OFFSET {}", m.offset));
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
    bindings: &BindingView<'_>,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    normalize: bool,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<Option<String>> {
    if order.is_empty() {
        return Ok(None);
    }
    let mut terms = Vec::with_capacity(order.len());
    for key in order {
        let def = bindings.get(&key.var, work)?.ok_or_else(|| {
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
/// **raw key columns** `sf_s` / `sf_o` (term-gen lifting): a bare
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

fn emit_path_branch(
    b: &Branch,
    pc: &PathClosure,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    modifiers: BranchModifiers,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<EmittedBranch> {
    // ORDER BY over a path result is handled at the exec layer (plan.order →
    // Rust-level order_cmp) — Branch.order is always empty here, so no guard needed.
    let projection = b.projection_with_distinct(modifiers.distinct);
    let mut params = Vec::new();
    let mut pidx = 0usize;

    // `cte` is the distinct-pairs relation the outer projection reads (colref binds
    // `t{alias}`). Its columns are the canonical `sf_s` / `sf_o` keys, never base
    // columns, so the outer projection / WHERE resolve against an empty catalog.
    let cte = format!("t{}", pc.alias);
    let outer_actuals = HashMap::from([(pc.alias, path_actuals_controlled(pc, catalog, work)?)]);
    let with = path_sql::prelude(pc, pc.alias, dialect, catalog, work)?;

    let select_list = projection
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{} AS c{i}", colref(c, dialect, &outer_actuals)))
        .collect::<Vec<_>>()
        .join(", ");

    let distinct = if modifiers.distinct { "DISTINCT " } else { "" };
    work.charge(with.len() + select_list.len() + cte.len() + 64)
        .map_err(source_control::validation_error)?;
    let mut skeleton = format!("{with} SELECT {distinct}{select_list} FROM {cte}");
    if let Some(w) = render_where(
        &b.where_conds,
        dialect,
        catalog,
        &outer_actuals,
        &mut params,
        &mut pidx,
        work,
    )? {
        skeleton.push_str(" WHERE ");
        skeleton.push_str(&w);
    }
    push_limit_offset(&mut skeleton, modifiers, dialect);

    source_control::sql_parse(&skeleton, work).map_err(source_control::validation_error)?;
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
    modifiers: BranchModifiers,
    work: sf_sql::source_work::SourceWork<'_>,
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
        Some(render_from_controlled(
            b,
            dialect,
            catalog,
            actuals,
            &mut params,
            &mut pidx,
            work,
        )?)
    };
    let where_sql = render_where(
        &b.where_conds,
        dialect,
        catalog,
        actuals,
        &mut params,
        &mut pidx,
        work,
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
                if let Some(keys) = (dialect == Dialect::Sqlite)
                    .then(|| literal_roles::sqlite_group_keys(b, agg, column, catalog, actuals))
                    .flatten()
                {
                    group_cols.extend(keys);
                } else {
                    group_cols.push(rendered.clone());
                }
                rendered
            }
            AggregateProjection::Aggregate(aggregate) => agg_expr_sql(aggregate, dialect, actuals),
            AggregateProjection::AvgOperand(column) => colref(column, dialect, actuals),
        };
        select_items.push(format!("{expression} AS c{i}"));
    }
    let select_list = select_items.join(", ");

    let distinct = if modifiers.distinct { "DISTINCT " } else { "" };
    work.charge(select_list.len() + from.as_deref().map_or(0, str::len) + 64)
        .map_err(source_control::validation_error)?;
    let mut skeleton = match from {
        Some(f) => format!("SELECT {distinct}{select_list} FROM {f}"),
        None => format!("SELECT {distinct}{select_list}"),
    };
    if let Some(w) = where_sql {
        work.charge(w.len() + 7)
            .map_err(source_control::validation_error)?;
        skeleton.push_str(" WHERE ");
        skeleton.push_str(&w);
    }
    if !group_cols.is_empty() {
        // The joined list and its copy into the skeleton.
        let joined = group_cols
            .iter()
            .fold(10usize, |n, c| n.saturating_add(c.len() + 2));
        work.charge(joined.saturating_mul(2))
            .map_err(source_control::validation_error)?;
        skeleton.push_str(" GROUP BY ");
        skeleton.push_str(&group_cols.join(", "));
    }
    // ORDER BY over an aggregate result is applied in `exec` (never pushed to SQL —
    // it sorts the reconstructed terms type-aware). LIMIT/OFFSET on a single agg
    // branch were pushed by `Plan::prepared_branches` only when unordered.
    push_limit_offset(&mut skeleton, modifiers, dialect);

    source_control::sql_parse(&skeleton, work).map_err(source_control::validation_error)?;
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

#[cfg(test)]
fn scan_ref(
    scan: &crate::iq::Scan,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    scan_ref_controlled(
        scan,
        dialect,
        catalog,
        params,
        pidx,
        sf_sql::source_work::SourceWork::new(None),
    )
}

fn render_from_controlled(
    b: &Branch,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    work.checkpoint()
        .map_err(source_control::validation_error)?;
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
        let (nested_sql, nested_params) =
            emit_subplan_sql_controlled(&sp.plan, dialect, catalog, work)?;
        // Rebase Postgres $N placeholders in the nested SQL from $1.. to $(pidx+1)..
        let rebased = rebase_placeholders_controlled(&nested_sql, dialect, *pidx, work)?;
        // Splice nested params into the parent's param vector at this text position.
        work.append_parameters(params, pidx, nested_params)
            .map_err(source_control::validation_error)?;
        work.charge(join_kw.len() + rebased.len() + 24)
            .map_err(source_control::validation_error)?;
        Ok(format!("{join_kw}({rebased}) t{}", sp.alias))
    };
    // Pay each rendered piece before copying it into the FROM text.
    let append = |from: &mut String, piece: String| -> Result<()> {
        work.charge(piece.len())
            .map_err(source_control::validation_error)?;
        from.push_str(&piece);
        Ok(())
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
            let scan = scan::template::restrict_optional_controlled(opt, dialect, catalog, work)?;
            append(
                &mut from,
                scan_ref_controlled(&scan, dialect, catalog, params, pidx, work)?,
            )?;
            from.push_str(" ON ");
            let conds: Vec<&SqlCond> = opt.on.iter().chain(opt.extra.iter()).collect();
            append(
                &mut from,
                render_conjunction(&conds, dialect, catalog, actuals, params, pidx, work)?,
            )?;
        }
        for sp in &b.subplan_joins {
            let join_kw = if sp.left {
                " LEFT JOIN "
            } else {
                " INNER JOIN "
            };
            append(&mut from, emit_sp(sp, params, pidx, join_kw)?)?;
            if !sp.on.is_empty() {
                from.push_str(" ON ");
                let conds: Vec<&SqlCond> = sp.on.iter().collect();
                append(
                    &mut from,
                    render_conjunction(&conds, dialect, catalog, actuals, params, pidx, work)?,
                )?;
            } else {
                from.push_str(" ON 1 = 1");
            }
        }
        from
    } else {
        let mut scans = b.core.iter();
        let first = scans.next().expect("core non-empty — checked above");
        let mut from = scan_ref_controlled(first, dialect, catalog, params, pidx, work)?;
        for s in scans {
            from.push_str(" CROSS JOIN ");
            append(
                &mut from,
                scan_ref_controlled(s, dialect, catalog, params, pidx, work)?,
            )?;
        }
        for opt in &b.opts {
            from.push_str(" LEFT JOIN ");
            let scan = scan::template::restrict_optional_controlled(opt, dialect, catalog, work)?;
            append(
                &mut from,
                scan_ref_controlled(&scan, dialect, catalog, params, pidx, work)?,
            )?;
            from.push_str(" ON ");
            let conds: Vec<&SqlCond> = opt.on.iter().chain(opt.extra.iter()).collect();
            append(
                &mut from,
                render_conjunction(&conds, dialect, catalog, actuals, params, pidx, work)?,
            )?;
        }
        for sp in &b.subplan_joins {
            let join_kw = if sp.left {
                " LEFT JOIN "
            } else {
                " INNER JOIN "
            };
            append(&mut from, emit_sp(sp, params, pidx, join_kw)?)?;
            if !sp.on.is_empty() {
                from.push_str(" ON ");
                let conds: Vec<&SqlCond> = sp.on.iter().collect();
                append(
                    &mut from,
                    render_conjunction(&conds, dialect, catalog, actuals, params, pidx, work)?,
                )?;
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
#[cfg(test)]
fn emit_subplan_sql(
    plan: &crate::Plan,
    dialect: Dialect,
    live_catalog: &ColumnCatalog,
) -> Result<(String, Vec<String>)> {
    emit_subplan_sql_controlled(
        plan,
        dialect,
        live_catalog,
        sf_sql::source_work::SourceWork::new(None),
    )
}

fn emit_subplan_sql_controlled(
    plan: &crate::Plan,
    dialect: Dialect,
    live_catalog: &ColumnCatalog,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<(String, Vec<String>)> {
    work.charge(1).map_err(source_control::validation_error)?;
    // Preparation changes only root scalar modifiers. Borrow the forest so a
    // unary wrapper chain does not recursively copy every remaining subtree.
    let branches = &plan.branches;
    let mut catalog = synthetic_subplan_catalog_controlled(branches, work)?;
    catalog.suppress_path_collation = live_catalog.suppress_path_collation;
    catalog.character_keys = std::sync::Arc::clone(&live_catalog.character_keys);
    catalog.lexical_keys = std::sync::Arc::clone(&live_catalog.lexical_keys);
    // Live top-level execution has already probed every recursively reachable
    // base source. Overlay those authoritative names so nested SubPlan emission
    // does not depend on the offline lexical alias-folding heuristic. The
    // synthetic entries remain only for dialect-neutral/offline emission and
    // source-free derived columns.
    let sources = match work.control() {
        Some(control) => source_control::live_metadata_sources_controlled(branches, control)
            .map_err(source_control::validation_error)?,
        None => live_metadata_sources(branches),
    };
    for source in sources {
        // One paid key copy per source, cloned once per overlaid map.
        let text_len = match source {
            LogicalSource::Table(table) => table.len(),
            LogicalSource::Query(query) => query.len(),
        };
        work.product(text_len + 2, 6)
            .map_err(source_control::validation_error)?;
        let key = source_key(source);
        let Some(columns) = live_catalog.by_source.get(&key) else {
            continue;
        };
        work.product(columns.len(), std::mem::size_of::<String>())
            .map_err(source_control::validation_error)?;
        for column in columns {
            work.charge(column.len())
                .map_err(source_control::validation_error)?;
        }
        std::sync::Arc::make_mut(&mut catalog.by_source).insert(key.clone(), columns.clone());
        macro_rules! overlay {
            ($field:ident) => {
                let map = std::sync::Arc::make_mut(&mut catalog.$field);
                match live_catalog.$field.get(&key) {
                    Some(live) => {
                        source_control::map_copy(live, work)
                            .map_err(source_control::validation_error)?;
                        map.insert(key.clone(), live.clone());
                    }
                    None => {
                        map.remove(&key);
                    }
                }
            };
        }
        overlay!(text_by_source);
        overlay!(sqlite_by_source);
        overlay!(scalars_by_source);
        overlay!(datatypes_by_source);
    }
    pg_float::validate_union(branches, dialect, &catalog, plan.distinct, work)?;
    mysql_float_value::identity::validate_union_controlled(branches, dialect, &catalog, work)?;
    let emitted = branches
        .iter()
        .map(|branch| {
            let modifiers = BranchModifiers::prepared(
                branch,
                branches.len() == 1,
                plan.distinct,
                plan.order.is_empty(),
                plan.limit,
                plan.offset,
            );
            emit_branch_keys(
                branch,
                &BindingView::Direct(&branch.bindings),
                dialect,
                &catalog,
                plan.distinct || modifiers.distinct,
                modifiers,
                work,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    if emitted.is_empty() {
        // Empty inner plan — a values-empty derived table: return a SELECT with no rows.
        // Use a dummy column so it is syntactically valid as a derived table.
        return Ok(("SELECT 1 AS __sf_empty WHERE 1 = 0".to_owned(), Vec::new()));
    }
    if emitted.len() == 1 {
        let e = emitted.into_iter().next().expect("one emitted branch");
        return Ok((e.sql, e.params));
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
    let numeric_keys = if plan.distinct {
        pg_numeric::union_keys(branches, dialect, &catalog, work)?
    } else {
        None
    };
    for e in emitted {
        let sql = rebase_placeholders_controlled(&e.sql, dialect, all_params.len(), work)?;
        if dialect == Dialect::Sqlite {
            all_sql.push(sql);
        } else {
            work.charge(sql.len() + 2)
                .map_err(source_control::validation_error)?;
            all_sql.push(format!("({sql})"));
        }
        let mut parameter_index = all_params.len();
        work.append_parameters(&mut all_params, &mut parameter_index, e.params)
            .map_err(source_control::validation_error)?;
    }
    let op = if plan.distinct && numeric_keys.is_none() {
        " UNION "
    } else {
        " UNION ALL "
    };
    for sql in &all_sql {
        work.charge(sql.len() + op.len())
            .map_err(source_control::validation_error)?;
    }
    let raw = all_sql.join(op);
    let sql = match numeric_keys {
        Some(keys) => {
            work.charge(raw.len())
                .map_err(source_control::validation_error)?;
            work.product(keys.len(), 96)
                .map_err(source_control::validation_error)?;
            pg_numeric::distinct_sql(raw, &keys)
        }
        None => raw,
    };
    Ok((sql, all_params))
}

/// Rebase positional `$N` placeholders in `sql` from base 1 to start at `base+1`,
/// for PostgreSQL numbered placeholders. SQLite uses `?` (positional by text order,
/// no numbering), so for SQLite (or when `base == 0`) returns `sql` unchanged.
fn rebase_placeholders(sql: &str, dialect: Dialect, base: usize) -> Result<String> {
    sf_sql::dialect::rebase_placeholders(sql, dialect, base)
        .map_err(|error| Error::Sql(error.to_string()))
}

/// The output copy, plus the tokenizer pass and character scan on PostgreSQL.
fn rebase_placeholders_controlled(
    sql: &str,
    dialect: Dialect,
    base: usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    let passes = if dialect == Dialect::Postgres && base > 0 {
        3
    } else {
        1
    };
    work.product(sql.len(), passes)
        .map_err(source_control::validation_error)?;
    rebase_placeholders(sql, dialect, base)
}

fn render_where(
    conds: &[SqlCond],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<Option<String>> {
    work.charge(1).map_err(source_control::validation_error)?;
    if conds.is_empty() {
        return Ok(None);
    }
    let mut refs = work
        .vector(conds.len())
        .map_err(source_control::validation_error)?;
    refs.extend(conds.iter());
    Ok(Some(render_conjunction(
        &refs, dialect, catalog, actuals, params, pidx, work,
    )?))
}

fn render_conjunction(
    conds: &[&SqlCond],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    condition_control::conjunction(conds, dialect, catalog, actuals, params, pidx, work)
}

#[cfg(test)]
fn render_cond(
    cond: &SqlCond,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    render_cond_controlled(
        cond,
        dialect,
        catalog,
        actuals,
        params,
        pidx,
        sf_sql::source_work::SourceWork::new(None),
    )
}

fn render_cond_controlled(
    cond: &SqlCond,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    condition_control::condition(cond, dialect, catalog, actuals, params, pidx, work)
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
    catalog: &ColumnCatalog,
    column: impl Fn(&str) -> String,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    use sf_core::ir::Segment;
    let mut parts = work
        .vector(segs.len())
        .map_err(source_control::validation_error)?;
    for seg in segs {
        work.charge(1).map_err(source_control::validation_error)?;
        parts.push(match seg {
            Segment::Literal(text) => {
                work.parameter(params, pidx, text)
                    .map_err(source_control::validation_error)?;
                dialect.placeholder(*pidx)
            }
            Segment::Column(c) => {
                let col = column(c);
                if encode_iri {
                    percent_encode_col_controlled(&col, dialect, catalog, work)?
                } else {
                    col
                }
            }
        });
    }
    // Joined bytes are copied once into the join buffer, then into its wrapper.
    for part in &parts {
        work.product(part.len(), 2)
            .map_err(source_control::validation_error)?;
    }
    work.product(parts.len(), 8)
        .map_err(source_control::validation_error)?;
    work.charge(8).map_err(source_control::validation_error)?;
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
    catalog: &ColumnCatalog,
    column: impl Fn(&str) -> String,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    use sf_core::ir::Segment;
    let mut parts = work
        .vector(segs.len())
        .map_err(source_control::validation_error)?;
    for segment in segs {
        let part = match segment {
            Segment::Literal(text) => {
                // Quoting at most doubles the literal.
                work.product(text.len(), 2)
                    .map_err(source_control::validation_error)?;
                sql_string_literal(text)
            }
            Segment::Column(name) if encode_iri => {
                percent_encode_col_controlled(&column(name), dialect, catalog, work)?
            }
            Segment::Column(name) => column(name),
        };
        // The part is copied once more into the joined expression.
        work.charge(part.len() + 4)
            .map_err(source_control::validation_error)?;
        parts.push(part);
    }
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
/// here). Used only for mapping-owned literals in template projections.
fn sql_string_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Percent-encode `col_sql`'s runtime value EXACTLY the way `sf_core::ir::
/// Template::expand`'s `encode_iri` arm does: RFC3987 *iunreserved* consists
/// of `ALPHA / DIGIT / "-" / "." / "_" / "~" / ucschar`. Other valid scalars
/// become uppercase percent-encoded UTF-8 bytes, never hexadecimal code points.
/// The shared core range table excludes private-use, C1 and noncharacters.
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
/// **SQLite now uses a query-local native function instead** (see
/// [`percent_encode_col_sqlite`]); the per-character SQL below remains the
/// PostgreSQL and MySQL design.
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
/// TIME. A byte passes through only when its complete UTF-8 scalar is in
/// RFC3987 ucschar; other valid scalars are percent-encoded byte by byte.
/// Invalid UTF-8 errors rather than becoming a valid escaped IRI. An isolated
/// intermediate byte cast is not independently valid UTF-8, but the final result
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

/// Pay the encoder expansion before building it. Each dialect's output is an
/// affine function of the column expression: a fixed body plus a constant
/// number of repetitions, calibrated once from the encoder itself.
fn percent_encode_col_controlled(
    col_sql: &str,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    if dialect == Dialect::Sqlite {
        // The native encoder is a query-local lexical key function.
        catalog
            .lexical_keys
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    static SHAPES: std::sync::OnceLock<[(usize, usize); 3]> = std::sync::OnceLock::new();
    let shapes = SHAPES.get_or_init(|| {
        [Dialect::Sqlite, Dialect::MySql, Dialect::Postgres].map(|dialect| {
            let fixed = percent_encode_col("", dialect).map_or(0, |sql| sql.len());
            let repeats = percent_encode_col("x", dialect).map_or(0, |sql| sql.len() - fixed);
            (fixed, repeats)
        })
    });
    let (fixed, repeats) = match dialect {
        Dialect::Sqlite => shapes[0],
        Dialect::MySql => shapes[1],
        Dialect::Postgres => shapes[2],
        _ => (0, 0),
    };
    work.product(col_sql.len(), repeats)
        .map_err(source_control::validation_error)?;
    work.charge(fixed + 1)
        .map_err(source_control::validation_error)?;
    percent_encode_col(col_sql, dialect)
}

/// SQLite: one call to the query-local native `__sf_percent_encode_v1`, which
/// applies `sf_core::ir::encoding::percent_encode_iri` itself (installed and
/// removed with the lexical decoder keys; the emitter raises that flag). The
/// earlier per-byte `WITH RECURSIVE` template was ~13 KB per column reference,
/// so identity queries produced hundreds of kilobytes of SQL per request.
/// `CAST(... AS TEXT)` lets SQLite spell numbers before encoding.
fn percent_encode_col_sqlite(col: &str) -> String {
    format!("__sf_percent_encode_v1(CAST({col} AS TEXT))")
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
    // Bind the BINARY cast once as `pre.b` and reference that short alias
    // everywhere below, instead of re-embedding the (potentially long) column
    // expression at each of the ~40 byte-range/length checks. This is a pure
    // SQL-text-size optimization: it does not change which bytes are read or
    // how they are classified, only how many times the source expression is
    // spelled out. `CAST(NULL AS BINARY) IS NULL`, so the null check is
    // unaffected; `pre` is always exactly one row, so this remains a scalar
    // expression usable anywhere `{col}` was used directly before.
    let ucschar = encoding_ucschar::byte_member("pre.b", Dialect::MySql);
    let valid_utf8 = encoding_ucschar::valid_non_ascii_byte("pre.b", Dialect::MySql);
    format!(
        "(SELECT CASE WHEN pre.b IS NULL THEN NULL ELSE COALESCE((\
SELECT /*+ SET_VAR(group_concat_max_len = {group_limit}) */ \
CONVERT(CAST(GROUP_CONCAT(\
CASE \
WHEN HEX(SUBSTRING(pre.b, n, 1)) BETWEEN '30' AND '39' \
OR HEX(SUBSTRING(pre.b, n, 1)) BETWEEN '41' AND '5A' \
OR HEX(SUBSTRING(pre.b, n, 1)) BETWEEN '61' AND '7A' \
OR HEX(SUBSTRING(pre.b, n, 1)) IN ('2D', '2E', '5F', '7E') \
OR {ucschar} \
THEN SUBSTRING(pre.b, n, 1) \
WHEN HEX(SUBSTRING(pre.b, n, 1)) >= '80' AND NOT {valid_utf8} \
THEN JSON_EXTRACT('semantic-fabric-invalid-utf8', '$') \
ELSE CAST(CONCAT('%', HEX(SUBSTRING(pre.b, n, 1))) AS BINARY) \
END ORDER BY n SEPARATOR ''\
) AS BINARY) USING utf8mb4)\
FROM JSON_TABLE(\
CASE WHEN LENGTH(pre.b) = 0 THEN '[]' \
WHEN LENGTH(pre.b) > LEAST(\
{max_input}, \
GREATEST(CAST(@@SESSION.group_concat_max_len AS SIGNED), 0) DIV 3, \
GREATEST(CAST(@@SESSION.max_allowed_packet AS SIGNED) - {packet_reserve}, 0) DIV 3\
) \
THEN 'semantic-fabric-percent-encoding-input-limit' \
ELSE CONCAT('[0', REPEAT(',0', LENGTH(pre.b) - 1), ']') END, \
'$[*]' COLUMNS (n FOR ORDINALITY)\
) AS sfpe\
), '') END \
FROM (SELECT CAST({col} AS BINARY) AS b) AS pre)",
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
    let ucschar = encoding_ucschar::codepoint_member("ascii(ch)");
    format!(
        "(SELECT CASE WHEN {col}::text IS NULL THEN NULL ELSE COALESCE((\
SELECT string_agg(\
CASE \
WHEN ascii(ch) BETWEEN 48 AND 57 \
OR ascii(ch) BETWEEN 65 AND 90 \
OR ascii(ch) BETWEEN 97 AND 122 \
OR ascii(ch) IN (45, 46, 95, 126) \
OR {ucschar} \
THEN ch \
ELSE (SELECT string_agg('%' || UPPER(LPAD(TO_HEX(get_byte(convert_to(ch, 'UTF8'), n)), 2, '0')), '' ORDER BY n) \
FROM generate_series(0, octet_length(ch) - 1) AS bytes(n)) \
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

    /// Regression guard for the fix that bound the MySQL BINARY cast once
    /// (`pre.b`) instead of re-embedding the source column expression at
    /// every byte-range/length check: before that fix MySQL's `repeats` was
    /// 148 (vs. PostgreSQL's 2 and SQLite's 1), so `SourceWork` charged for
    /// percent-encoding a MySQL column scaled ~74x worse per byte of column-
    /// expression length than the other dialects for the identical
    /// semantic check, not because of any larger actual workload. `<= 5`
    /// leaves headroom for incidental template growth while catching a
    /// reintroduced per-check re-embedding of the column expression.
    #[test]
    fn mysql_percent_encoder_repeats_stays_bounded() {
        let repeats = |dialect| {
            let fixed = percent_encode_col("", dialect).map_or(0, |sql| sql.len());
            let with_x = percent_encode_col("x", dialect).map_or(0, |sql| sql.len());
            with_x - fixed
        };
        assert!(
            repeats(Dialect::MySql) <= 5,
            "MySQL percent-encoding repeats={} (was 148 before pre.b binding)",
            repeats(Dialect::MySql)
        );
    }

    /// Mirrors [`percent_encode_col_sqlite_matches_reference_iri_encoding`]
    /// for MySQL, plus the two standalone-invalid-UTF-8 byte cases (0x80, a
    /// lone continuation byte; 0xC2, a lone 2-byte lead), which deliberately
    /// make the generated SQL raise via `JSON_EXTRACT` on invalid JSON per
    /// [`percent_encode_col_mysql`]'s own doc comment, rather than compare
    /// against `reference_encode` (which requires valid UTF-8 input).
    /// Verified against a live MySQL 8.4 instance during this fix's
    /// development: identical pass/fail and identical encoded output to the
    /// pre-fix template across every case here, only the generated SQL's
    /// `repeats` factor changed (148 -> 1).
    #[tokio::test]
    #[ignore = "requires a purpose-created isolated MySQL provider"]
    async fn mysql_percent_encoder_matches_reference_iri_encoding() {
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
        conn.query_drop("CREATE TEMPORARY TABLE t (v BLOB)")
            .await
            .unwrap();

        let mut cases: Vec<Option<Vec<u8>>> = vec![
            Some(b"a b/c".to_vec()),
            Some(b"A-z.0_9~".to_vec()),
            Some(b"".to_vec()),
            None,
            Some(b"X/Y".to_vec()),
            Some("你好/世界".as_bytes().to_vec()),
            Some(b"tab\ttab".to_vec()),
            Some(b"nul\0nul".to_vec()),
        ];
        for b in 0x20u8..=0x7e {
            cases.push(Some(vec![b]));
        }
        for b in 0..=0x1fu8 {
            cases.push(Some(vec![b]));
        }
        cases.push(Some(vec![0x7fu8]));

        let sql = percent_encode_col("t.v", Dialect::MySql).expect("MySQL is supported");
        let query = format!("SELECT {sql} FROM t");
        let mut mismatches = Vec::new();
        for v in &cases {
            conn.query_drop("DELETE FROM t").await.unwrap();
            conn.exec_drop("INSERT INTO t (v) VALUES (?)", (v.clone(),))
                .await
                .unwrap();
            let got = conn
                .query_first::<Option<String>, _>(&query)
                .await
                .unwrap()
                .flatten();
            let want = v
                .as_ref()
                .map(|bytes| reference_encode(&String::from_utf8_lossy(bytes)));
            if got != want {
                mismatches.push(format!("input={v:?} got={got:?} want={want:?}"));
            }
        }
        assert!(mismatches.is_empty(), "{mismatches:#?}");

        for invalid in [vec![0x80u8], vec![0xC2u8]] {
            conn.query_drop("DELETE FROM t").await.unwrap();
            conn.exec_drop("INSERT INTO t (v) VALUES (?)", (invalid.clone(),))
                .await
                .unwrap();
            let result = conn.query_first::<Option<String>, _>(&query).await;
            assert!(
                result.is_err(),
                "standalone-invalid UTF-8 {invalid:?} must fail closed, not silently encode"
            );
        }
        conn.disconnect()
            .await
            .unwrap_or_else(|_| panic!("close isolated MySQL connection failed"));
    }

    #[test]
    fn subplan_sql_pays_query_text_scans_and_parser_passes() {
        use sf_core::query_control::{QueryBudget, QueryCharge, QueryLimits};
        use sf_sql::source_work::SourceWork;
        let consumed = |padding: usize| {
            let maps = sf_mapping::parse_r2rml(&format!(
                r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
                <#m> rr:logicalTable [rr:sqlQuery "SELECT v AS o FROM items WHERE '{}' <> ''"];
                rr:subject <http://ex/s>;
                rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:objectMap [rr:column "o"]]."#,
                "x".repeat(padding)
            ))
            .unwrap();
            let plan = crate::parse_and_translate(
                "SELECT ?o WHERE { ?s <http://ex/p> ?o }",
                &maps,
                Dialect::Sqlite,
            )
            .unwrap();
            let control =
                QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
            emit_subplan_sql_controlled(
                &plan,
                Dialect::Sqlite,
                &ColumnCatalog::default(),
                SourceWork::new(Some(&control)),
            )
            .unwrap();
            control.consumed(QueryCharge::SourceWork)
        };
        // Query text is scanned per referencing column and re-parsed with the SQL.
        let (short, long) = (consumed(0), consumed(1000));
        assert!(long >= short + 2 * 1000, "short {short} long {long}");
    }

    #[test]
    fn percent_encoder_charge_equals_its_output_length() {
        use sf_core::query_control::{QueryBudget, QueryCharge, QueryLimits};
        use sf_sql::source_work::SourceWork;
        for dialect in [Dialect::Sqlite, Dialect::MySql, Dialect::Postgres] {
            for column in [
                "t.v",
                "(__sf_lexical_key_v1(t0.\"value\", 1, -1) COLLATE BINARY)",
            ] {
                let control =
                    QueryBudget::new(QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX));
                let sql = percent_encode_col_controlled(
                    column,
                    dialect,
                    &ColumnCatalog::default(),
                    SourceWork::new(Some(&control)),
                )
                .unwrap();
                assert_eq!(
                    control.consumed(QueryCharge::SourceWork),
                    sql.len() as u64 + 1,
                    "{dialect:?} {column}"
                );
            }
        }
    }

    fn branch_with(cond: SqlCond) -> Branch {
        let mut b = Branch::single(Scan {
            alias: 0,
            source: (LogicalSource::Table("emp".to_owned())).into(),
        });
        b.where_conds.push(cond);
        b
    }

    #[test]
    fn postgres_placeholder_rebase_preserves_authored_bytes() {
        let prefix = "SELECT 'é$1', \"$2\", $$δ$3$$, $tag$🍀$4$tag$, E'it\\'s $5', table$6 /* $7 /* $8 */ */ -- $9\n";
        let sql = format!("{prefix}$1, $20");
        assert_eq!(
            rebase_placeholders(&sql, Dialect::Postgres, 2).unwrap(),
            format!("{prefix}$3, $22")
        );
        assert_eq!(rebase_placeholders(&sql, Dialect::MySql, 2).unwrap(), sql);
        assert!(rebase_placeholders("SELECT $1", Dialect::Postgres, usize::MAX).is_err());
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
        let raw = sf_sql::source_work::SourceWork::new(None);
        let table_sql = render_template_inline(
            &template,
            false,
            Dialect::Postgres,
            &ColumnCatalog::default(),
            |column| render_immediate_source_column("sfs0", column, None, Dialect::Postgres),
            raw,
        )
        .unwrap();
        assert!(table_sql.contains("(sfs0.ctid)::text"), "{table_sql}");
        let query_sql = render_template_inline(
            &template,
            false,
            Dialect::Postgres,
            &ColumnCatalog::default(),
            |column| {
                render_immediate_source_column(
                    "sfs0",
                    column,
                    Some("SELECT 7 AS rowid"),
                    Dialect::Postgres,
                )
            },
            raw,
        )
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

    /// Compare SQL with the actual shared term recipe, not a second encoder.
    pub(super) fn reference_encode(value: &str) -> String {
        let mut out = String::new();
        sf_core::ir::Template::parse("{v}").unwrap().expand(
            &[("v", Some(value))][..],
            true,
            &mut out,
        );
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
        let _keys = sf_sql::backend::sqlite::lexical_keys(&conn).unwrap();
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
