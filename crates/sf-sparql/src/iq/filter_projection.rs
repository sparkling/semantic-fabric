//! Borrowed, paid equivalent of emit::source_projection. The layout remains in
//! first-encounter order, with the same DISTINCT, aggregate and SQLite AVG rules.
//! No owned ColRef/TermDef copies or hidden unmetered projection/dedup traversal.
use crate::build::control::{BuildVec, BuildWork};
use crate::iq::{AggKind, Branch, ColRef, R2rmlGraphScope, SqlCond, TermDef};
use crate::Result;
use sf_core::ir::{Segment, TermMap};
use sf_sql::Dialect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Column<'a> {
    pub alias: usize,
    pub name: &'a str,
}
impl<'a> From<&'a ColRef> for Column<'a> {
    fn from(c: &'a ColRef) -> Self {
        Self {
            alias: c.alias,
            name: &c.column,
        }
    }
}
type Columns<'a> = BuildVec<Option<Column<'a>>>;

pub(super) fn projection<'a>(
    branch: &'a Branch,
    distinct: bool,
    dialect: Dialect,
    work: BuildWork<'_>,
) -> Result<Vec<Option<Column<'a>>>> {
    let work = work.enter()?;
    let mut out = BuildVec::new(Vec::new());
    if let Some(agg) = branch.agg.as_ref().filter(|_| branch.path.is_none()) {
        for key in &agg.keys {
            work.charge(1)?;
            for column in &key.cols {
                work.push(&mut out, Some(Column::from(column)))?;
            }
        }
        for aggregate in &agg.aggs {
            work.push(&mut out, None)?;
            if dialect == Dialect::Sqlite && aggregate.kind == AggKind::Avg {
                if let Some(column) = &aggregate.arg {
                    work.push(&mut out, Some(Column::from(column)))?;
                }
            }
        }
    } else {
        for def in branch.bindings.values() {
            term(def, &mut out, work)?;
        }
        if !distinct {
            for cond in &branch.where_conds {
                condition(cond, &mut out, work)?;
            }
            for opt in &branch.opts {
                work.charge(1)?;
                for cond in opt.on.iter().chain(&opt.extra) {
                    condition(cond, &mut out, work)?;
                }
            }
            for join in &branch.subplan_joins {
                work.charge(1)?;
                for cond in &join.on {
                    condition(cond, &mut out, work)?;
                }
            }
        }
    }
    work.checkpoint()?;
    Ok(out.into_inner())
}

fn push<'a>(column: Column<'a>, out: &mut Columns<'a>, work: BuildWork<'_>) -> Result<()> {
    for existing in &out.values {
        work.charge(1)?;
        if let Some(existing) = existing {
            work.charge(existing.name.len().min(column.name.len()))?;
            if *existing == column {
                return Ok(());
            }
        }
    }
    work.push(out, Some(column))
}

fn segments<'a>(
    parts: &'a [Segment],
    alias: usize,
    out: &mut Columns<'a>,
    work: BuildWork<'_>,
) -> Result<()> {
    for part in parts {
        work.charge(1)?;
        if let Segment::Column(name) = part {
            push(Column { alias, name }, out, work)?;
        }
    }
    Ok(())
}

fn map<'a>(
    map: &'a TermMap,
    alias: usize,
    out: &mut Columns<'a>,
    work: BuildWork<'_>,
) -> Result<()> {
    let work = work.enter()?;
    match map {
        TermMap::Constant(_) => Ok(()),
        TermMap::Column(name, _) => push(Column { alias, name }, out, work),
        TermMap::Template(t, _) => segments(t.segments(), alias, out, work),
    }
}

fn term<'a>(def: &'a TermDef, out: &mut Columns<'a>, work: BuildWork<'_>) -> Result<()> {
    let work = work.enter()?;
    match def {
        TermDef::Const(_) => {}
        TermDef::Derived { term_map, alias } => map(term_map, *alias, out, work)?,
        TermDef::R2rmlBlank {
            term_map,
            alias,
            graph,
        } => {
            map(term_map, *alias, out, work)?;
            work.charge(1)?;
            if let R2rmlGraphScope::Mapped { term_map, alias } = graph {
                map(term_map, *alias, out, work)?;
            }
        }
        TermDef::Coalesce(a, b) => {
            term(a, out, work)?;
            term(b, out, work)?;
        }
        TermDef::Concat(parts) => {
            for part in parts {
                term(part, out, work)?;
            }
        }
        TermDef::Agg { col, .. } => push(Column::from(col), out, work)?,
        TermDef::ComposedTriple {
            subject,
            predicate,
            object,
        } => {
            term(subject, out, work)?;
            term(predicate, out, work)?;
            term(object, out, work)?;
        }
    }
    Ok(())
}

fn iri<'a>(
    operand: &'a super::IriOperand,
    out: &mut Columns<'a>,
    work: BuildWork<'_>,
) -> Result<()> {
    work.charge(1)?;
    match operand {
        super::IriOperand::Column { column, .. } => push(Column::from(column), out, work)?,
        super::IriOperand::Template { parts, .. } => {
            for part in parts {
                work.charge(1)?;
                if let super::IriPart::Column(column) = part {
                    push(Column::from(column), out, work)?;
                }
            }
        }
        super::IriOperand::Constant(_) => {}
    }
    Ok(())
}

fn condition<'a>(cond: &'a SqlCond, out: &mut Columns<'a>, work: BuildWork<'_>) -> Result<()> {
    let work = work.enter()?;
    match cond {
        SqlCond::ExpressionError
        | SqlCond::NotExists { .. }
        | SqlCond::Exists { .. }
        | SqlCond::PathExists { .. } => {}
        SqlCond::LiteralCmp(cmp) => {
            for operand in [&cmp.left, &cmp.right] {
                work.charge(1)?;
                if let crate::iq::literal_cmp::LiteralOperand::Column { column, .. } = operand {
                    push(Column::from(column), out, work)?;
                }
            }
        }
        SqlCond::IriCmp(cmp) => {
            iri(&cmp.left, out, work)?;
            iri(&cmp.right, out, work)?;
        }
        SqlCond::ColEq(a, b) | SqlCond::NativeColEq(a, b) | SqlCond::NullSafeEq(a, b) => {
            push(Column::from(a), out, work)?;
            push(Column::from(b), out, work)?;
        }
        SqlCond::Cmp(c, _, _)
        | SqlCond::NativeCmp(c, _, _)
        | SqlCond::IsNotNull(c)
        | SqlCond::DecodedIsNotNull(c)
        | SqlCond::IsNull(c)
        | SqlCond::StrMatch { col: c, .. } => push(Column::from(c), out, work)?,
        SqlCond::Not(c) => condition(c, out, work)?,
        SqlCond::And(parts) | SqlCond::Or(parts) => {
            for part in parts {
                condition(part, out, work)?;
            }
        }
        SqlCond::TemplateEq(a, a_alias, b, b_alias, _) => {
            segments(a, *a_alias, out, work)?;
            segments(b, *b_alias, out, work)?;
        }
    }
    Ok(())
}
