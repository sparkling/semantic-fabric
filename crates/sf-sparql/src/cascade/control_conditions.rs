//! Controlled condition discovery, constant fixpoint and stable selection moves.
use super::{control_fd::same, Branch, CmpOp, ColRef, SqlCond};
use crate::build::control::{BuildVec, BuildWork};
use crate::Result;

pub(super) struct ColumnRemap<'a> {
    pub parent: usize,
    pub child: usize,
    pub pairs: &'a [(Box<str>, Box<str>)],
}

impl ColumnRemap<'_> {
    fn name(&self, name: &mut Box<str>, work: BuildWork<'_>) -> Result<()> {
        for (parent, child) in self.pairs {
            work.charge(1)?;
            if super::control_join::equal(parent, name, work)? {
                *name = work.string(child)?.into();
                break;
            }
        }
        work.checkpoint()
    }

    fn column(&self, col: &mut ColRef, work: BuildWork<'_>) -> Result<()> {
        work.charge(1)?;
        if col.alias == self.parent {
            // Discovery proves that all parent references have a replacement.
            // Keep raw's unmatched-column behavior for defensive callers.
            for (parent, child) in self.pairs {
                work.charge(1)?;
                if super::control_join::equal(parent, &col.column, work)? {
                    col.column = work.string(child)?.into();
                    col.alias = self.child;
                    break;
                }
            }
        }
        work.checkpoint()
    }

    pub(super) fn condition(&self, value: &mut SqlCond, work: BuildWork<'_>) -> Result<()> {
        use crate::iq::{
            iri_cmp::{IriOperand, IriPart},
            literal_cmp::LiteralOperand,
        };
        let work = work.enter()?;
        match value {
            SqlCond::ExpressionError => (),
            SqlCond::LiteralCmp(cmp) => {
                for operand in [&mut cmp.left, &mut cmp.right] {
                    work.charge(1)?;
                    if let LiteralOperand::Column { column, .. } = operand {
                        self.column(column, work)?;
                    }
                }
            }
            SqlCond::IriCmp(cmp) => {
                for operand in [&mut cmp.left, &mut cmp.right] {
                    work.charge(1)?;
                    match operand {
                        IriOperand::Column { column, .. } => self.column(column, work)?,
                        IriOperand::Template { parts, .. } => {
                            for part in parts {
                                work.charge(1)?;
                                if let IriPart::Column(column) = part {
                                    self.column(column, work)?;
                                }
                            }
                        }
                        IriOperand::Constant(_) => (),
                    }
                }
            }
            SqlCond::ColEq(a, b) | SqlCond::NativeColEq(a, b) | SqlCond::NullSafeEq(a, b) => {
                self.column(a, work)?;
                self.column(b, work)?;
            }
            SqlCond::Cmp(col, ..)
            | SqlCond::NativeCmp(col, ..)
            | SqlCond::IsNotNull(col)
            | SqlCond::DecodedIsNotNull(col)
            | SqlCond::IsNull(col)
            | SqlCond::StrMatch { col, .. } => self.column(col, work)?,
            SqlCond::Not(inner) => self.condition(inner, work)?,
            SqlCond::And(children)
            | SqlCond::Or(children)
            | SqlCond::Exists {
                conds: children, ..
            }
            | SqlCond::NotExists {
                conds: children, ..
            }
            | SqlCond::PathExists {
                conds: children, ..
            } => {
                for child in children {
                    work.charge(1)?;
                    self.condition(child, work)?;
                }
            }
            SqlCond::TemplateEq(left, a, right, b, _) => {
                for (segments, alias) in [(left, a), (right, b)] {
                    work.charge(1)?;
                    if *alias != self.parent {
                        continue;
                    }
                    for segment in segments {
                        work.charge(1)?;
                        if let super::Segment::Column(name) = segment {
                            self.name(name, work)?;
                        }
                    }
                    *alias = self.child;
                }
            }
        }
        work.checkpoint()
    }
}

pub(super) fn has_subquery(branch: &Branch, work: BuildWork<'_>) -> Result<bool> {
    fn has(condition: &SqlCond, work: BuildWork<'_>) -> Result<bool> {
        let work = work.enter()?;
        match condition {
            SqlCond::NotExists { .. } | SqlCond::Exists { .. } | SqlCond::PathExists { .. } => {
                Ok(true)
            }
            SqlCond::Not(inner) => has(inner, work),
            SqlCond::And(children) | SqlCond::Or(children) => {
                for child in children {
                    work.charge(1)?;
                    if has(child, work)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            _ => Ok(false),
        }
    }
    for condition in &branch.where_conds {
        work.charge(1)?;
        if has(condition, work)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn find<'a>(
    known: &[(&ColRef, &'a str)],
    col: &ColRef,
    work: BuildWork<'_>,
) -> Result<Option<&'a str>> {
    for (candidate, value) in known {
        work.charge(1)?;
        if same(candidate, col, work)? {
            return Ok(Some(*value));
        }
    }
    Ok(None)
}

fn equal(a: &str, b: &str, work: BuildWork<'_>) -> Result<bool> {
    work.charge(1)?;
    work.charge(a.len().min(b.len()))?;
    let equal = a == b;
    work.checkpoint()?;
    Ok(equal)
}

/// Constant inference needs only borrowed columns and values, not raw's copies.
/// Borrowed references always point into the unchanged branch, never the work vector.
pub(super) fn consistent(branch: &Branch, work: BuildWork<'_>) -> Result<bool> {
    let mut known = BuildVec::new(Vec::new());
    for condition in &branch.where_conds {
        work.charge(1)?;
        if let SqlCond::Cmp(col, CmpOp::Eq, value) = condition {
            if let Some(previous) = find(&known.values, col, work)? {
                if !equal(previous, value, work)? {
                    return Ok(false);
                }
            } else {
                work.push(&mut known, (col, value.as_str()))?;
            }
        }
    }
    loop {
        work.charge(1)?;
        let mut changed = false;
        for condition in &branch.where_conds {
            work.charge(1)?;
            let SqlCond::ColEq(a, b) = condition else {
                continue;
            };
            for (source, target) in [(a, b), (b, a)] {
                work.charge(1)?;
                if let Some(value) = find(&known.values, source, work)? {
                    if let Some(previous) = find(&known.values, target, work)? {
                        if !equal(previous, value, work)? {
                            return Ok(false);
                        }
                    } else {
                        work.push(&mut known, (target, value))?;
                        changed = true;
                    }
                }
            }
        }
        work.checkpoint()?;
        if !changed {
            return Ok(true);
        }
    }
}

fn flatten(condition: SqlCond, output: &mut BuildVec<SqlCond>, work: BuildWork<'_>) -> Result<()> {
    let work = work.enter()?;
    match condition {
        SqlCond::And(children) => {
            for child in children {
                work.charge(1)?;
                flatten(child, output, work)?;
            }
            Ok(())
        }
        other => work.push(output, other),
    }
}

/// The caller owns the unpublished branch. On refusal it discards that candidate;
/// no rollback allocation or mutation of an already published Plan is involved.
pub(super) fn selection(branch: &mut Branch, work: BuildWork<'_>) -> Result<()> {
    work.charge(1)?;
    let mut flat = BuildVec::new(Vec::new());
    for condition in std::mem::take(&mut branch.where_conds) {
        work.charge(1)?;
        flatten(condition, &mut flat, work)?;
    }
    let mut selections = BuildVec::new(Vec::new());
    let mut joins = BuildVec::new(Vec::new());
    for condition in flat.values {
        work.charge(1)?;
        let mut first = None;
        let mut multiple = false;
        super::condition_columns_with_work(&condition, work, &mut |alias, _| {
            work.charge(1)?;
            if let Some(first) = first {
                multiple |= first != alias;
            } else {
                first = Some(alias);
            }
            Ok(())
        })?;
        work.push(
            if multiple {
                &mut joins
            } else {
                &mut selections
            },
            condition,
        )?;
    }
    for condition in joins.values {
        work.charge(1)?;
        work.push(&mut selections, condition)?;
    }
    work.checkpoint()?;
    branch.where_conds = selections.into_inner();
    Ok(())
}

fn disjunction<'a>(
    condition: &'a SqlCond,
    work: BuildWork<'_>,
) -> Result<Option<(&'a ColRef, Vec<&'a str>)>> {
    work.charge(1)?;
    let SqlCond::Or(arms) = condition else {
        return Ok(None);
    };
    let mut column = None;
    let mut values = BuildVec::new(Vec::new());
    for arm in arms {
        work.charge(1)?;
        let SqlCond::Cmp(candidate, CmpOp::Eq, value) = arm else {
            return Ok(None);
        };
        if let Some(column) = column {
            if !same(column, candidate, work)? {
                return Ok(None);
            }
        } else {
            column = Some(candidate);
        }
        work.push(&mut values, value.as_str())?;
    }
    Ok(column.map(|column| (column, values.into_inner())))
}

fn equality(column: &ColRef, value: &str, work: BuildWork<'_>) -> Result<SqlCond> {
    let column = ColRef::new(column.alias, work.string(&column.column)?);
    let value = work.string(value)?;
    Ok(SqlCond::Cmp(column, CmpOp::Eq, value))
}

/// Retain the raw left-value order (including duplicates), narrowing only when
/// the raw rule would do so. Each successful round removes one condition.
pub(super) fn intersect(branch: &mut Branch, work: BuildWork<'_>) -> Result<bool> {
    loop {
        work.charge(1)?;
        let mut changed = false;
        'outer: for i in 0..branch.where_conds.len() {
            work.charge(1)?;
            let Some((left, left_values)) = disjunction(&branch.where_conds[i], work)? else {
                continue;
            };
            for j in i + 1..branch.where_conds.len() {
                work.charge(1)?;
                let Some((right, right_values)) = disjunction(&branch.where_conds[j], work)? else {
                    continue;
                };
                if !same(left, right, work)? {
                    continue;
                }
                let mut intersection = BuildVec::new(Vec::new());
                for value in &left_values {
                    work.charge(1)?;
                    for candidate in &right_values {
                        work.charge(1)?;
                        if equal(value, candidate, work)? {
                            work.push(&mut intersection, *value)?;
                            break;
                        }
                    }
                }
                if intersection.values.is_empty() {
                    return Ok(false);
                }
                if intersection.values.len() < left_values.len().max(right_values.len()) {
                    let replacement = if let [value] = intersection.values.as_slice() {
                        equality(left, value, work)?
                    } else {
                        let mut arms = work.vector(intersection.values.len())?;
                        for value in intersection.values {
                            work.charge(1)?;
                            arms.push(equality(left, value, work)?);
                        }
                        SqlCond::Or(arms)
                    };
                    // Reserve compacting the tail before touching the candidate.
                    let moved = branch.where_conds.len() - j - 1;
                    work.charge(moved + 1)?;
                    if let crate::CompilerWorkMode::Metered(context) = work.mode {
                        context
                            .reserve_checked_product(&[moved, std::mem::size_of::<SqlCond>()])?;
                    }
                    work.checkpoint()?;
                    branch.where_conds.remove(j);
                    branch.where_conds[i] = replacement;
                    changed = true;
                    break 'outer;
                }
            }
        }
        work.checkpoint()?;
        if !changed {
            return Ok(true);
        }
    }
}

pub(super) fn left_contradictions(
    branch: &mut Branch,
    schema: &super::SchemaMap<'_>,
    work: BuildWork<'_>,
) -> Result<()> {
    let mut index = 0;
    while index < branch.opts.len() {
        work.charge(1)?;
        let mut contradiction = false;
        if let Some((keep, _)) = super::control_join::left_key(branch, index, schema, work)? {
            let optional = &branch.opts[index];
            'extra: for condition in &optional.extra {
                work.charge(1)?;
                let SqlCond::Cmp(column, CmpOp::Eq, value) = condition else {
                    continue;
                };
                if column.alias != optional.scan.alias {
                    continue;
                }
                for condition in &branch.where_conds {
                    work.charge(1)?;
                    if let SqlCond::Cmp(other, CmpOp::Eq, other_value) = condition {
                        if other.alias == keep
                            && equal(&other.column, &column.column, work)?
                            && !equal(other_value, value, work)?
                        {
                            contradiction = true;
                            break 'extra;
                        }
                    }
                }
            }
        }
        if contradiction {
            let drop = branch.opts[index].scan.alias;
            let mut keep = work.vector(branch.bindings.len())?;
            for definition in branch.bindings.values() {
                work.charge(1)?;
                let mut only_dropped = true;
                for (alias, _) in super::term_columns_with_work(definition, work)? {
                    work.charge(1)?;
                    if alias != drop {
                        only_dropped = false;
                        break;
                    }
                }
                keep.push(!only_dropped);
            }
            work.charge(branch.bindings.len())?;
            super::control_rewrite::remove(&mut branch.opts, index, work)?;
            work.checkpoint()?;
            let mut keep = keep.into_iter();
            branch
                .bindings
                .retain(|_, _| keep.next().expect("paid contradiction decision"));
        } else {
            index += 1;
        }
    }
    work.checkpoint()
}

#[cfg(test)]
#[path = "control_tests.rs"]
mod tests;
