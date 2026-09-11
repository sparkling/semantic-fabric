//! Request-controlled LOWER entry walks and result-variable materialization.
//! These are actual alias/scope operations, not a whole-LOWER work estimate.
use crate::build::control::{BuildVec, BuildWork};
use crate::iq::iri_cmp::{IriOperand, IriPart};
use crate::iq::literal_cmp::LiteralOperand;
use crate::iq::node::{IqCond, IqNode, Var};
use crate::iq::{Branch, SqlCond};
use crate::{CompilerWorkMode, Result};
use sf_core::query_control::QueryControlError;

pub(super) fn starting_alias(node: &IqNode, work: BuildWork<'_>) -> Result<usize> {
    let maximum = tree(node, work)?;
    work.charge(1)?;
    maximum.checked_add(1).ok_or_else(|| match work.mode {
        CompilerWorkMode::Metered(cx) => {
            cx.reject_build_resource(QueryControlError::AccountingOverflow)
        }
        CompilerWorkMode::Uncontrolled => QueryControlError::AccountingOverflow.into(),
    })
}

fn tree(node: &IqNode, work: BuildWork<'_>) -> Result<usize> {
    let work = work.enter()?;
    Ok(match node {
        IqNode::Extensional { scan, .. } => scan.alias,
        IqNode::InnerJoin { children, cond } => trees(children, work)?.max(conditions(cond, work)?),
        IqNode::LeftJoin { left, right, cond } => tree(left, work)?
            .max(tree(right, work)?)
            .max(conditions(cond, work)?),
        IqNode::Filter { child, cond } => tree(child, work)?.max(conditions(cond, work)?),
        IqNode::Construction { child, .. }
        | IqNode::Distinct { child }
        | IqNode::Aggregation { child, .. }
        | IqNode::Slice { child, .. }
        | IqNode::OrderBy { child, .. } => tree(child, work)?,
        IqNode::Union { children, .. } => trees(children, work)?,
        IqNode::Path { closure } => closure.alias,
        IqNode::Values { .. }
        | IqNode::Empty { .. }
        | IqNode::True
        | IqNode::Intensional { .. }
        | IqNode::UnresolvedPath { .. } => 0,
    })
}

fn trees(nodes: &[IqNode], work: BuildWork<'_>) -> Result<usize> {
    let mut maximum = 0;
    for node in nodes {
        work.charge(1)?;
        maximum = maximum.max(tree(node, work)?);
    }
    Ok(maximum)
}

fn conditions(conds: &[IqCond], work: BuildWork<'_>) -> Result<usize> {
    let mut maximum = 0;
    for cond in conds {
        work.charge(1)?;
        maximum = maximum.max(condition(cond, work)?);
    }
    Ok(maximum)
}

fn condition(cond: &IqCond, work: BuildWork<'_>) -> Result<usize> {
    let work = work.enter()?;
    match cond {
        IqCond::Sql(sql) => sql_condition(sql, work),
        IqCond::And(parts) | IqCond::Or(parts) => conditions(parts, work),
        IqCond::Not(inner) => condition(inner, work),
        IqCond::Exists(inner) | IqCond::NotExists { inner, .. } => tree(inner, work),
        IqCond::Expr(_) => Ok(0),
    }
}

fn sql_conditions(conds: &[SqlCond], work: BuildWork<'_>) -> Result<usize> {
    let mut maximum = 0;
    for cond in conds {
        work.charge(1)?;
        maximum = maximum.max(sql_condition(cond, work)?);
    }
    Ok(maximum)
}

fn sql_condition(cond: &SqlCond, work: BuildWork<'_>) -> Result<usize> {
    let work = work.enter()?;
    let mut maximum = 0;
    match cond {
        SqlCond::ExpressionError => (),
        SqlCond::LiteralCmp(cmp) => {
            for operand in [&cmp.left, &cmp.right] {
                work.charge(1)?;
                if let LiteralOperand::Column { column, .. } = operand {
                    maximum = maximum.max(column.alias);
                }
            }
        }
        SqlCond::IriCmp(cmp) => {
            for operand in [&cmp.left, &cmp.right] {
                work.charge(1)?;
                match operand {
                    IriOperand::Column { column, .. } => maximum = maximum.max(column.alias),
                    IriOperand::Template { parts, .. } => {
                        // columns() filters literals internally. Visit/pay them
                        // too, including a wide template with zero columns.
                        for part in parts {
                            work.charge(1)?;
                            if let IriPart::Column(column) = part {
                                maximum = maximum.max(column.alias);
                            }
                        }
                    }
                    IriOperand::Constant(_) => (),
                }
            }
        }
        SqlCond::ColEq(left, right)
        | SqlCond::NativeColEq(left, right)
        | SqlCond::NullSafeEq(left, right) => maximum = left.alias.max(right.alias),
        SqlCond::Cmp(column, _, _)
        | SqlCond::NativeCmp(column, _, _)
        | SqlCond::IsNotNull(column)
        | SqlCond::DecodedIsNotNull(column)
        | SqlCond::IsNull(column)
        | SqlCond::StrMatch { col: column, .. } => maximum = column.alias,
        SqlCond::Not(inner) => maximum = sql_condition(inner, work)?,
        SqlCond::And(parts) | SqlCond::Or(parts) => maximum = sql_conditions(parts, work)?,
        SqlCond::NotExists { scans, conds } | SqlCond::Exists { scans, conds } => {
            for scan in scans {
                work.charge(1)?;
                maximum = maximum.max(scan.alias);
            }
            maximum = maximum.max(sql_conditions(conds, work)?);
        }
        SqlCond::PathExists { pc, conds, .. } => {
            maximum = pc.alias.max(sql_conditions(conds, work)?);
        }
        SqlCond::TemplateEq(_, left_alias, _, right_alias, _) => {
            maximum = (*left_alias).max(*right_alias);
        }
    }
    Ok(maximum)
}

pub(super) fn record_project(
    target: &mut Option<Vec<Var>>,
    project: &[Var],
    work: BuildWork<'_>,
) -> Result<()> {
    if target.is_none() {
        // This typed operation measures/reserves the exact single copy. Do not
        // also charge a second vector/payload reservation for the same copy.
        *target = Some(work.mode.clone_variables(project)?);
    }
    work.checkpoint()
}

pub(super) fn record_output_scope(
    target: &mut Option<Vec<Var>>,
    node: &IqNode,
    work: BuildWork<'_>,
) -> Result<()> {
    if target.is_none() {
        *target = Some(crate::build::controlled_output_vars(node, work)?);
    }
    work.checkpoint()
}

pub(super) fn plan_vars(
    project: Option<Vec<Var>>,
    branches: &[Branch],
    work: BuildWork<'_>,
) -> Result<Vec<String>> {
    if let Some(project) = project {
        let mut out = work.vector(project.len())?;
        for name in project {
            out.push(work.string(&name)?);
        }
        return Ok(out);
    }
    if matches!(work.mode, CompilerWorkMode::Uncontrolled) {
        return Ok(super::visible_vars(branches));
    }
    // Same lexical sorted uniqueness as the raw BTreeSet, with explicit
    // comparisons and logical vector growth instead of hidden set-node work.
    let mut out = BuildVec::new(Vec::<String>::new());
    for branch in branches {
        work.charge(1)?;
        for name in branch.bindings.keys() {
            work.charge(1)?;
            let (mut low, mut high) = (0, out.values.len());
            while low < high {
                let middle = low + (high - low) / 2;
                work.charge(1)?;
                work.charge(name.len())?;
                work.charge(out.values[middle].len())?;
                if out.values[middle] < *name {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            if let Some(existing) = out.values.get(low) {
                work.charge(1)?;
                work.charge(name.len())?;
                work.charge(existing.len())?;
                if existing == name {
                    continue;
                }
            }
            let moved = out.values.len() - low;
            work.charge(moved)?;
            if let CompilerWorkMode::Metered(cx) = work.mode {
                cx.reserve_checked_product(&[moved, std::mem::size_of::<String>()])?;
            }
            work.push(&mut out, work.string(name)?)?;
            out.values[low..].rotate_right(1);
            work.checkpoint()?;
        }
    }
    Ok(out.into_inner())
}
