//! Paid, iterative source inventory. Raw enumeration remains the semantic oracle.
use sf_core::ir::LogicalSource;
use sf_core::query_control::QueryControl;
use sf_sql::source_work::{SourceVec, SourceWork};

use crate::iq::{Branch, HopExpr, OptJoin, Scan, ScanSource, SqlCond, SubPlanJoin};
use sf_sql::Result;
use std::collections::HashMap;

pub(super) fn validation_error(error: sf_sql::Error) -> crate::Error {
    match error {
        sf_sql::Error::QueryControl(cause) => crate::Error::QueryControl(cause),
        sf_sql::Error::Emit(message) => crate::Error::Sql(message),
        other => crate::Error::Sql(other.to_string()),
    }
}

/// Prepay one numeric constant's admission before any parser reads it. The
/// whitespace trim, grammar/facet scans and library digit parse (oxsdatatypes
/// and core dec2flt scan a lexical at most twice) are each linear in its
/// length; callers pay bounded display buffers separately.
pub(super) fn numeric_lexical(value: &str, work: SourceWork<'_>) -> Result<()> {
    work.product(value.len(), 4)?;
    work.charge(1)
}

/// Prepay one SQL parser round trip over `sql`. The tokenizer and the
/// regenerated text are each byte-linear; the recursive-descent parse and its
/// AST are token-linear, and every token spans several bytes.
pub(super) fn sql_parse(sql: &str, work: SourceWork<'_>) -> Result<()> {
    work.product(sql.len(), 2)?;
    work.charge(1)
}

/// Prepay copying one per-source column map: every key string and value slot.
pub(super) fn map_copy<V>(map: &HashMap<String, V>, work: SourceWork<'_>) -> Result<()> {
    work.product(map.len(), std::mem::size_of::<(String, V)>())?;
    for key in map.keys() {
        work.charge(key.len())?;
    }
    Ok(())
}

impl super::ColumnCatalog {
    pub(super) fn validate_live_column_controlled(
        &self,
        source: &LogicalSource,
        raw: &str,
        dialect: sf_sql::Dialect,
        work: SourceWork<'_>,
    ) -> crate::Result<()> {
        let run = || -> Result<()> {
            let name = match source {
                LogicalSource::Table(name) | LogicalSource::Query(name) => name,
            };
            work.charge(name.len())?;
            work.charge(2)?;
            let key = super::source_key(source);
            work.charge(key.len())?;
            work.product(self.by_source.len(), key.len())?;
            work.charge(self.by_source.len())?;
            let columns = self.by_source.get(&key).ok_or_else(|| {
                sf_sql::Error::Emit(
                    "live metadata is unavailable for a required logical source".into(),
                )
            })?;
            let mut exact = 0;
            for column in columns {
                work.charge(1)?;
                work.charge(column.len().min(raw.len()))?;
                if column == raw {
                    exact += 1;
                }
            }
            if exact > 1 {
                return Err(sf_sql::Error::Emit(
                    "live metadata contains duplicate result-column names".into(),
                ));
            }
            if exact == 1 {
                return work.checkpoint();
            }
            let mut folded = 0;
            for column in columns {
                work.charge(1)?;
                work.charge(column.len().min(raw.len()))?;
                if column.eq_ignore_ascii_case(raw) {
                    folded += 1;
                }
            }
            if folded > 1 {
                return Err(sf_sql::Error::Emit(
                    "live metadata contains ambiguous result-column names".into(),
                ));
            }
            work.charge(5)?;
            if folded == 1 || super::physical_row_identifier(source, raw, dialect) {
                return work.checkpoint();
            }
            Err(sf_sql::Error::Emit(
                "live metadata is missing a required result column".into(),
            ))
        };
        run().map_err(validation_error)
    }
}

/// Borrowed variant-tagged keys avoid copying potentially large query text.
/// Binary search pays each actual byte comparison; insertion pays reference
/// movement before changing the sorted index. Encounter order stays in the caller.
#[derive(Default)]
pub(crate) struct SourceSet<'a>(SourceVec<(u8, &'a str)>);

impl<'a> SourceSet<'a> {
    pub(crate) fn insert(
        &mut self,
        source: &'a LogicalSource,
        control: &dyn QueryControl,
    ) -> Result<bool> {
        let key = match source {
            LogicalSource::Table(name) => (1, name.as_str()),
            LogicalSource::Query(query) => (0, query.as_str()),
        };
        self.insert_key(key, SourceWork::new(Some(control)))
    }

    pub(super) fn insert_key(&mut self, key: (u8, &'a str), work: SourceWork<'_>) -> Result<bool> {
        let (mut left, mut right) = (0, self.0.as_slice().len());
        while left < right {
            work.charge(1)?;
            let middle = left + (right - left) / 2;
            let candidate = self.0.as_slice()[middle];
            let mut order = candidate.0.cmp(&key.0);
            if order.is_eq() {
                work.charge(candidate.1.len())?;
                work.charge(key.1.len())?;
                order = candidate.1.cmp(key.1);
            }
            match order {
                std::cmp::Ordering::Equal => {
                    work.checkpoint()?;
                    return Ok(false);
                }
                std::cmp::Ordering::Less => left = middle + 1,
                std::cmp::Ordering::Greater => right = middle,
            }
        }
        self.0.insert(left, key, work)?;
        Ok(true)
    }
}

/// The live preflight owns its catalog exclusively until all source schemas
/// have been admitted. Never invoke Arc::make_mut and silently clone prior maps.
impl super::ColumnCatalog {
    pub(crate) fn insert_live_result_controlled(
        &mut self,
        source: &LogicalSource,
        columns: Vec<sf_sql::backend::ResultColumn>,
        control: &dyn QueryControl,
    ) -> Result<()> {
        let work = SourceWork::new(Some(control));
        work.checkpoint()?;
        let mut unique = SourceSet::default();
        for column in &columns {
            work.charge(1)?;
            if !unique.insert_key((0, &column.name), work)? {
                return Err(sf_sql::Error::Emit(
                    "live metadata contains duplicate result-column names".into(),
                ));
            }
        }
        let datatypes = metadata_map(&columns, work, |column| {
            column.natural_datatype.or_else(|| {
                column
                    .native_scalar
                    .and_then(super::natural_literal::source_code)
            })
        })?;
        let scalars = metadata_map(&columns, work, |column| column.native_scalar)?;
        let sqlite = metadata_map(&columns, work, |column| column.sqlite_decode)?;
        let text = metadata_map(&columns, work, |column| column.text_key)?;
        let mut names = work.vector(columns.len())?;
        for column in columns {
            work.charge(1)?;
            names.push(column.name);
        }
        let raw = match source {
            LogicalSource::Table(value) | LogicalSource::Query(value) => value,
        };
        work.charge(raw.len())?;
        work.charge(2)?;
        let key = super::source_key(source);
        macro_rules! insert {
            ($field:ident, $value:expr) => {{
                let map = std::sync::Arc::get_mut(&mut self.$field).ok_or_else(|| {
                    sf_sql::Error::Emit(
                        "live catalog preparation requires exclusive ownership".into(),
                    )
                })?;
                paid_insert(map, work.string(&key)?, $value, work)?;
            }};
        }
        insert!(by_source, names);
        insert!(text_by_source, text);
        insert!(sqlite_by_source, sqlite);
        insert!(scalars_by_source, scalars);
        insert!(datatypes_by_source, datatypes);
        work.checkpoint()
    }
}

fn metadata_map<T>(
    columns: &[sf_sql::backend::ResultColumn],
    work: SourceWork<'_>,
    value: impl Fn(&sf_sql::backend::ResultColumn) -> Option<T>,
) -> Result<std::collections::HashMap<String, T>> {
    let mut map = std::collections::HashMap::new();
    for column in columns {
        work.charge(1)?;
        if let Some(value) = value(column) {
            paid_insert(&mut map, work.string(&column.name)?, value, work)?;
        }
    }
    Ok(map)
}

fn paid_insert<T>(
    map: &mut std::collections::HashMap<String, T>,
    key: String,
    value: T,
    work: SourceWork<'_>,
) -> Result<()> {
    work.charge(1)?;
    work.product(1, std::mem::size_of::<(String, T)>())?;
    work.charge(key.len())?; // hash incoming key
                             // At most every existing key can collide; each comparison inspects no more
                             // bytes than the incoming key. This is local insertion work, not a bound on
                             // the whole query or the allocator's physical overgrant.
    work.product(map.len(), key.len())?;
    work.charge(map.len())?;
    if map.len() == map.capacity() {
        work.product(map.len(), std::mem::size_of::<(String, T)>())?;
        for old in map.keys() {
            work.charge(old.len())?;
        }
        map.try_reserve(1)
            .map_err(|_| sf_sql::Error::Emit("source catalog allocation failed".into()))?;
    }
    map.insert(key, value);
    work.checkpoint()
}

pub(crate) fn source_probe_controlled(
    source: &LogicalSource,
    dialect: sf_sql::Dialect,
    control: &dyn QueryControl,
) -> Result<String> {
    let work = SourceWork::new(Some(control));
    match source {
        LogicalSource::Query(query) => work.string(query),
        LogicalSource::Table(name) => {
            // quote_ident creates one owned Ident, scans it and doubles only
            // embedded quote bytes. Prepay that input and the quoted temporary
            // plus final probe, including both quote delimiters and SQL literals.
            work.charge(name.len())?; // Ident's owned input
            work.charge(name.len())?; // escape traversal
            work.product(name.len(), 2)?; // quoted temporary payload
            work.charge(2)?;
            work.product(name.len(), 2)?; // copied into the final probe
            work.charge("SELECT * FROM ".len() + " LIMIT 0".len() + 2)?;
            let probe = dialect.probe_sql(source);
            work.checkpoint()?;
            Ok(probe)
        }
    }
}

/// Validation needs borrowed column names, not a recursively cloned ColRef list.
/// Repeated graph-scope columns can be validated again without changing meaning.
pub(crate) fn validate_definition_columns(
    definition: &crate::iq::TermDef,
    work: SourceWork<'_>,
    mut validate: impl FnMut(usize, &str) -> crate::Result<()>,
) -> crate::Result<()> {
    use crate::iq::{R2rmlGraphScope, TermDef};
    fn term_columns(
        term: &sf_core::ir::TermMap,
        alias: usize,
        work: SourceWork<'_>,
        validate: &mut impl FnMut(usize, &str) -> crate::Result<()>,
    ) -> crate::Result<()> {
        work.charge(1).map_err(validation_error)?;
        match term {
            sf_core::ir::TermMap::Constant(_) => {}
            sf_core::ir::TermMap::Column(column, _) => validate(alias, column)?,
            sf_core::ir::TermMap::Template(template, _) => {
                for segment in template.segments() {
                    work.charge(1).map_err(validation_error)?;
                    if let sf_core::ir::Segment::Column(column) = segment {
                        validate(alias, column)?;
                    }
                }
            }
        }
        Ok(())
    }
    let mut pending = SourceVec::default();
    pending.push(definition, work).map_err(validation_error)?;
    while let Some(definition) = pending.pop() {
        work.charge(1).map_err(validation_error)?;
        match definition {
            TermDef::Const(_) => {}
            TermDef::Derived { term_map, alias } => {
                term_columns(term_map, *alias, work, &mut validate)?
            }
            TermDef::R2rmlBlank {
                term_map,
                alias,
                graph,
            } => {
                term_columns(term_map, *alias, work, &mut validate)?;
                work.charge(1).map_err(validation_error)?;
                if let R2rmlGraphScope::Mapped { term_map, alias } = graph {
                    term_columns(term_map, *alias, work, &mut validate)?;
                }
            }
            TermDef::Coalesce(left, right) => {
                pending
                    .push(right.as_ref(), work)
                    .map_err(validation_error)?;
                pending
                    .push(left.as_ref(), work)
                    .map_err(validation_error)?;
            }
            TermDef::Concat(parts) => {
                for part in parts.iter().rev() {
                    pending.push(part, work).map_err(validation_error)?;
                }
            }
            TermDef::Agg { col, .. } => validate(col.alias, &col.column)?,
            TermDef::ComposedTriple {
                subject,
                predicate,
                object,
            } => {
                for part in [object, predicate, subject] {
                    pending
                        .push(part.as_ref(), work)
                        .map_err(validation_error)?;
                }
            }
        }
    }
    work.checkpoint().map_err(validation_error)
}

enum Item<'a> {
    Branch(&'a Branch),
    Branches(&'a [Branch]),
    Scan(&'a Scan),
    Scans(&'a [Scan]),
    OptScans(&'a [OptJoin]),
    OptConditions(&'a [OptJoin]),
    Subplans(&'a [SubPlanJoin]),
    Condition(&'a SqlCond),
    Conditions(&'a [SqlCond]),
    Hop(&'a HopExpr),
    Hops(&'a [HopExpr]),
}

pub(crate) fn live_metadata_sources_controlled<'a>(
    branches: &'a [Branch],
    control: &dyn QueryControl,
) -> Result<Vec<&'a LogicalSource>> {
    let work = SourceWork::new(Some(control));
    let mut pending = SourceVec::default();
    let mut output = SourceVec::default();
    pending.push(Item::Branches(branches), work)?;
    while let Some(item) = pending.pop() {
        work.charge(1)?;
        match item {
            Item::Branches(items) => {
                if let Some((first, rest)) = items.split_first() {
                    pending.push(Item::Branches(rest), work)?;
                    pending.push(Item::Branch(first), work)?;
                }
            }
            Item::Scans(items) => {
                if let Some((first, rest)) = items.split_first() {
                    pending.push(Item::Scans(rest), work)?;
                    pending.push(Item::Scan(first), work)?;
                }
            }
            Item::Conditions(items) => {
                if let Some((first, rest)) = items.split_first() {
                    pending.push(Item::Conditions(rest), work)?;
                    pending.push(Item::Condition(first), work)?;
                }
            }
            Item::Hops(items) => {
                if let Some((first, rest)) = items.split_first() {
                    pending.push(Item::Hops(rest), work)?;
                    pending.push(Item::Hop(first), work)?;
                }
            }
            Item::OptScans(items) => {
                if let Some((first, rest)) = items.split_first() {
                    pending.push(Item::OptScans(rest), work)?;
                    pending.push(Item::Scan(&first.scan), work)?;
                }
            }
            Item::OptConditions(items) => {
                if let Some((first, rest)) = items.split_first() {
                    pending.push(Item::OptConditions(rest), work)?;
                    pending.push(Item::Conditions(&first.extra), work)?;
                    pending.push(Item::Conditions(&first.on), work)?;
                }
            }
            Item::Subplans(items) => {
                if let Some((first, rest)) = items.split_first() {
                    pending.push(Item::Subplans(rest), work)?;
                    pending.push(Item::Branches(&first.plan.branches), work)?;
                    pending.push(Item::Conditions(&first.on), work)?;
                }
            }
            Item::Branch(branch) => {
                pending.push(Item::Subplans(&branch.subplan_joins), work)?;
                pending.push(Item::OptConditions(&branch.opts), work)?;
                pending.push(Item::Conditions(&branch.where_conds), work)?;
                if let Some(path) = &branch.path {
                    pending.push(Item::Hop(&path.hop), work)?;
                }
                pending.push(Item::OptScans(&branch.opts), work)?;
                pending.push(Item::Scans(&branch.core), work)?;
            }
            Item::Scan(scan) => match &scan.source {
                ScanSource::Logical(source) => output.push(source, work)?,
                ScanSource::Path { closure, .. } => pending.push(Item::Hop(&closure.hop), work)?,
                ScanSource::Projection { input, .. } => pending.push(Item::Scan(input), work)?,
                ScanSource::RefAtom { input, .. } => pending.push(Item::Branch(input), work)?,
            },
            Item::Condition(condition) => match condition {
                SqlCond::Not(inner) => pending.push(Item::Condition(inner), work)?,
                SqlCond::And(inner) | SqlCond::Or(inner) => {
                    pending.push(Item::Conditions(inner), work)?
                }
                SqlCond::Exists { scans, conds } | SqlCond::NotExists { scans, conds } => {
                    pending.push(Item::Conditions(conds), work)?;
                    pending.push(Item::Scans(scans), work)?;
                }
                SqlCond::PathExists { pc, conds, .. } => {
                    pending.push(Item::Conditions(conds), work)?;
                    pending.push(Item::Hop(&pc.hop), work)?;
                }
                _ => {}
            },
            Item::Hop(hop) => match hop {
                HopExpr::Pred(relation) => output.push(&relation.source, work)?,
                HopExpr::Inverse(inner) => pending.push(Item::Hop(inner), work)?,
                HopExpr::Seq(left, right) => {
                    pending.push(Item::Hop(right), work)?;
                    pending.push(Item::Hop(left), work)?;
                }
                HopExpr::Alt(parts) | HopExpr::Nps(parts) => {
                    pending.push(Item::Hops(parts), work)?
                }
            },
        }
    }
    work.checkpoint()?;
    Ok(output.into_vec())
}

#[cfg(test)]
#[path = "source_control_tests.rs"]
mod tests;
