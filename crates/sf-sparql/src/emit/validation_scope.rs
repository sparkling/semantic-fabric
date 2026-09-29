//! Alias scopes, leaf column checks and queue continuations owned by
//! `validate_source_root`. Hoisted verbatim; every charge keeps its position.
use super::*;
use crate::iq::{Scan, SubPlanJoin};

#[derive(Clone, Copy)]
pub(super) enum AliasSource<'a> {
    Base(&'a LogicalSource),
    Derived,
    Path,
    Projection(&'a [(Box<str>, TermMap)]),
    RefAtom(usize),
}

#[derive(Default)]
pub(super) struct AliasMap<'a>(sf_sql::source_work::SourceVec<(usize, AliasSource<'a>)>);
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
    pub(super) fn insert(
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
    pub(super) fn copy(&self, work: sf_sql::source_work::SourceWork<'_>) -> Result<Self> {
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

pub(super) fn validate_ref(
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

pub(super) fn validate_named_ref(
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

pub(super) fn validate_hop(
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

pub(super) enum Task<'a> {
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
