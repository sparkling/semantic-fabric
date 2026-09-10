use std::collections::BTreeMap;

use super::{PlanMeasureError, Walker, Work};
use crate::iq::node::{AggArg, AggDef, BindDef, ColOrConst, IqCond, IqNode, Var};
use crate::iq::TermDef;

pub(super) fn visit_node<'a>(
    walker: &mut Walker<'a>,
    node: &'a IqNode,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match node {
        IqNode::Construction {
            child,
            subst,
            project,
        } => {
            walker.push(depth, Work::IqNode(child))?;
            push_substitution(walker, subst, depth)?;
            measure_variables(walker, project)?;
        }
        IqNode::Filter { child, cond } => {
            walker.push(depth, Work::IqNode(child))?;
            push_conditions(walker, cond, depth)?;
        }
        IqNode::InnerJoin { children, cond } => {
            push_nodes(walker, children, depth)?;
            push_conditions(walker, cond, depth)?;
        }
        IqNode::LeftJoin { left, right, cond } => {
            walker.push(depth, Work::IqNode(left))?;
            walker.push(depth, Work::IqNode(right))?;
            push_conditions(walker, cond, depth)?;
        }
        IqNode::Union { children, project } => {
            push_nodes(walker, children, depth)?;
            measure_variables(walker, project)?;
        }
        IqNode::Aggregation {
            child,
            grouping,
            aggs,
        } => {
            walker.push(depth, Work::IqNode(child))?;
            measure_variables(walker, grouping)?;
            walker.collection(aggs.len())?;
            for aggregate in aggs {
                walker.push(depth, Work::IqAggDef(aggregate))?;
            }
        }
        IqNode::Distinct { child } => walker.push(depth, Work::IqNode(child))?,
        IqNode::Slice {
            child,
            offset: _,
            limit: _,
        } => walker.push(depth, Work::IqNode(child))?,
        IqNode::OrderBy { child, keys } => {
            walker.push(depth, Work::IqNode(child))?;
            super::model::push_order_keys(walker, keys, depth)?;
        }
        IqNode::Values { vars, rows } => {
            measure_variables(walker, vars)?;
            push_value_rows(walker, rows, depth)?;
        }
        IqNode::Extensional { scan, bind } => {
            walker.push(depth, Work::Scan(scan))?;
            walker.collection(bind.len())?;
            for (column, binding) in bind {
                walker.payload(column.len())?;
                walker.push(depth, Work::IqColOrConst(binding))?;
            }
        }
        IqNode::Intensional { pattern, graph } => {
            walker.push(depth, Work::TriplePattern(pattern))?;
            if let Some(graph) = graph {
                walker.push(depth, Work::NamedNodePattern(graph))?;
            }
        }
        IqNode::UnresolvedPath {
            subject,
            path,
            object,
            graph,
        } => {
            walker.push(depth, Work::TermPattern(subject))?;
            walker.push(depth, Work::PropertyPath(path))?;
            walker.push(depth, Work::TermPattern(object))?;
            if let Some(graph) = graph {
                walker.push(depth, Work::NamedNodePattern(graph))?;
            }
        }
        IqNode::Empty { vars } => measure_variables(walker, vars)?,
        IqNode::True => {}
        IqNode::Path { closure } => walker.push(depth, Work::PathClosure(closure))?,
    }
    Ok(())
}

pub(super) fn visit_condition<'a>(
    walker: &mut Walker<'a>,
    condition: &'a IqCond,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match condition {
        IqCond::Expr(expression) => walker.push(depth, Work::Expression(expression))?,
        IqCond::Sql(condition) => walker.push(depth, Work::SqlCond(condition))?,
        IqCond::And(conditions) | IqCond::Or(conditions) => {
            push_conditions(walker, conditions, depth)?
        }
        IqCond::Not(condition) => walker.push(depth, Work::IqCond(condition))?,
        IqCond::Exists(inner) => walker.push(depth, Work::IqNode(inner))?,
        IqCond::NotExists { inner, is_minus: _ } => walker.push(depth, Work::IqNode(inner))?,
    }
    Ok(())
}

pub(super) fn visit_aggregate<'a>(
    walker: &mut Walker<'a>,
    aggregate: &'a AggDef,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let AggDef {
        var,
        kind: _,
        arg,
        distinct: _,
        fixed_type: _,
    } = aggregate;
    walker.payload(var.len())?;
    if let Some(arg) = arg {
        walker.push(depth, Work::IqAggArg(arg))?;
    }
    Ok(())
}

pub(super) fn visit_aggregate_arg<'a>(
    walker: &mut Walker<'a>,
    arg: &'a AggArg,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match arg {
        AggArg::Var(var) => walker.payload(var.len())?,
        AggArg::Expr(term) => walker.push(depth, Work::TermDef(term))?,
    }
    Ok(())
}

pub(super) fn visit_col_or_const<'a>(
    walker: &mut Walker<'a>,
    binding: &'a ColOrConst,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match binding {
        ColOrConst::Col(column) => walker.payload(column.len())?,
        ColOrConst::Const(term) => walker.push(depth, Work::Term(term))?,
    }
    Ok(())
}

pub(super) fn visit_bind_def<'a>(
    walker: &mut Walker<'a>,
    binding: &'a BindDef,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match binding {
        BindDef::Resolved(term) => walker.push(depth, Work::TermDef(term))?,
        BindDef::Expr(expression) => walker.push(depth, Work::Expression(expression))?,
    }
    Ok(())
}

pub(super) fn push_nodes<'a>(
    walker: &mut Walker<'a>,
    nodes: &'a [IqNode],
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.collection(nodes.len())?;
    for node in nodes {
        walker.push(depth, Work::IqNode(node))?;
    }
    Ok(())
}

pub(super) fn push_conditions<'a>(
    walker: &mut Walker<'a>,
    conditions: &'a [IqCond],
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.collection(conditions.len())?;
    for condition in conditions {
        walker.push(depth, Work::IqCond(condition))?;
    }
    Ok(())
}

pub(super) fn push_substitution<'a>(
    walker: &mut Walker<'a>,
    substitution: &'a BTreeMap<Var, BindDef>,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.collection(substitution.len())?;
    for (variable, binding) in substitution {
        walker.payload(variable.len())?;
        walker.push(depth, Work::IqBindDef(binding))?;
    }
    Ok(())
}

pub(super) fn measure_variables(
    walker: &mut Walker<'_>,
    variables: &[Var],
) -> Result<(), PlanMeasureError> {
    walker.collection(variables.len())?;
    for variable in variables {
        walker.payload(variable.len())?;
    }
    Ok(())
}

pub(super) fn push_value_rows<'a>(
    walker: &mut Walker<'a>,
    rows: &'a [Vec<Option<TermDef>>],
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.collection(rows.len())?;
    for row in rows {
        walker.collection(row.len())?;
        for term in row {
            walker.checkpoint()?;
            if let Some(term) = term {
                walker.push(depth, Work::TermDef(term))?;
            }
        }
    }
    Ok(())
}
