use sf_core::ir::{LogicalSource, Segment, Template, TermMap, TermSpec};

use super::{PlanMeasureError, Walker, Work};
use crate::iq::{
    AggCol, Aggregation, Branch, ColRef, GroupKey, HopExpr, HopRelation, OptJoin, OrderKey,
    PathClosure, R2rmlGraphScope, RustAgg, RustGroup, Scan, SqlCond, SubPlanJoin, TermDef,
};
use crate::{DedupScope, Plan, PlanForm};

pub(super) fn visit_plan<'a>(
    walker: &mut Walker<'a>,
    plan: &'a Plan,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.collection(plan.branches.len())?;
    for branch in &plan.branches {
        walker.push(depth, Work::Branch(branch))?;
    }
    walker.push(depth, Work::PlanForm(&plan.form))?;
    walker.collection(plan.order.len())?;
    for key in &plan.order {
        walker.push(depth, Work::OrderKey(key))?;
    }
    if let Some(group) = &plan.rust_group {
        walker.push(depth, Work::RustGroup(group))?;
    }
    walker.collection(plan.dedup_scopes.len())?;
    for scope in plan.dedup_scopes.iter().flatten() {
        walker.push(depth, Work::DedupScope(scope))?;
    }
    Ok(())
}

pub(super) fn visit_plan_form<'a>(
    walker: &mut Walker<'a>,
    form: &'a PlanForm,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match form {
        PlanForm::Select { vars } => {
            walker.collection(vars.len())?;
            for var in vars {
                walker.payload(var.len())?;
            }
        }
        PlanForm::Construct { template } => {
            walker.collection(template.len())?;
            for triple in template {
                walker.push(depth, Work::TriplePattern(triple))?;
            }
        }
        PlanForm::Ask => {}
    }
    Ok(())
}

pub(super) fn visit_dedup_scope<'a>(
    walker: &mut Walker<'a>,
    scope: &'a DedupScope,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.collection(scope.key_bindings.len())?;
    for (name, term) in &scope.key_bindings {
        walker.payload(name.len())?;
        walker.push(depth, Work::TermDef(term))?;
    }
    Ok(())
}

pub(super) fn visit_branch<'a>(
    walker: &mut Walker<'a>,
    branch: &'a Branch,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.collection(branch.core.len())?;
    for scan in &branch.core {
        walker.push(depth, Work::Scan(scan))?;
    }
    walker.collection(branch.opts.len())?;
    for opt in &branch.opts {
        walker.push(depth, Work::OptJoin(opt))?;
    }
    walker.collection(branch.bindings.len())?;
    for (name, term) in &branch.bindings {
        walker.payload(name.len())?;
        walker.push(depth, Work::TermDef(term))?;
    }
    walker.collection(branch.where_conds.len())?;
    for cond in &branch.where_conds {
        walker.push(depth, Work::SqlCond(cond))?;
    }
    walker.collection(branch.order.len())?;
    for key in &branch.order {
        walker.push(depth, Work::OrderKey(key))?;
    }
    if let Some(path) = &branch.path {
        walker.push(depth, Work::PathClosure(path))?;
    }
    if let Some(aggregation) = &branch.agg {
        walker.push(depth, Work::Aggregation(aggregation))?;
    }
    walker.collection(branch.subplan_joins.len())?;
    for join in &branch.subplan_joins {
        walker.push(depth, Work::SubPlanJoin(join))?;
    }
    Ok(())
}

pub(super) fn visit_scan<'a>(
    walker: &mut Walker<'a>,
    scan: &'a Scan,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.push(depth, Work::LogicalSource(&scan.source))
}

pub(super) fn visit_opt_join<'a>(
    walker: &mut Walker<'a>,
    join: &'a OptJoin,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.push(depth, Work::Scan(&join.scan))?;
    push_conditions(walker, &join.on, depth)?;
    push_conditions(walker, &join.extra, depth)
}

pub(super) fn visit_col_ref(walker: &mut Walker<'_>, col: &ColRef) -> Result<(), PlanMeasureError> {
    walker.payload(col.column.len())
}

pub(super) fn visit_term_def<'a>(
    walker: &mut Walker<'a>,
    term: &'a TermDef,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match term {
        TermDef::Const(term) => walker.push(depth, Work::Term(term))?,
        TermDef::Derived { term_map, .. } => walker.push(depth, Work::TermMap(term_map))?,
        TermDef::R2rmlBlank {
            term_map, graph, ..
        } => {
            walker.push(depth, Work::TermMap(term_map))?;
            walker.push(depth, Work::GraphScope(graph))?;
        }
        TermDef::Coalesce(left, right) => {
            walker.push(depth, Work::TermDef(left))?;
            walker.push(depth, Work::TermDef(right))?;
        }
        TermDef::Concat(parts) => {
            walker.collection(parts.len())?;
            for part in parts {
                walker.push(depth, Work::TermDef(part))?;
            }
        }
        TermDef::Agg { col, operand, .. } => {
            walker.push(depth, Work::ColRef(col))?;
            if let Some(operand) = operand {
                walker.push(depth, Work::ColRef(operand))?;
            }
        }
        TermDef::ComposedTriple {
            subject,
            predicate,
            object,
        } => {
            walker.push(depth, Work::TermDef(subject))?;
            walker.push(depth, Work::TermDef(predicate))?;
            walker.push(depth, Work::TermDef(object))?;
        }
    }
    Ok(())
}

pub(super) fn visit_graph_scope<'a>(
    walker: &mut Walker<'a>,
    graph: &'a R2rmlGraphScope,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    if let R2rmlGraphScope::Mapped { term_map, .. } = graph {
        walker.push(depth, Work::TermMap(term_map))?;
    }
    Ok(())
}

pub(super) fn visit_sql_cond<'a>(
    walker: &mut Walker<'a>,
    cond: &'a SqlCond,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match cond {
        SqlCond::ColEq(left, right) | SqlCond::NullSafeEq(left, right) => {
            walker.push(depth, Work::ColRef(left))?;
            walker.push(depth, Work::ColRef(right))?;
        }
        SqlCond::Cmp(col, _, param) => {
            walker.push(depth, Work::ColRef(col))?;
            walker.payload(param.len())?;
        }
        SqlCond::StrMatch { col, param, .. } => {
            walker.push(depth, Work::ColRef(col))?;
            walker.payload(param.len())?;
        }
        SqlCond::IsNotNull(col) | SqlCond::IsNull(col) => {
            walker.push(depth, Work::ColRef(col))?;
        }
        SqlCond::Not(inner) => walker.push(depth, Work::SqlCond(inner))?,
        SqlCond::And(conds) | SqlCond::Or(conds) => push_conditions(walker, conds, depth)?,
        SqlCond::NotExists { scans, conds } | SqlCond::Exists { scans, conds } => {
            walker.collection(scans.len())?;
            for scan in scans {
                walker.push(depth, Work::Scan(scan))?;
            }
            push_conditions(walker, conds, depth)?;
        }
        SqlCond::PathExists { pc, conds, .. } => {
            walker.push(depth, Work::PathClosure(pc))?;
            push_conditions(walker, conds, depth)?;
        }
        SqlCond::TemplateEq(left, _, right, _, _) => {
            push_segments(walker, left, depth)?;
            push_segments(walker, right, depth)?;
        }
    }
    Ok(())
}

pub(super) fn visit_order_key<'a>(
    walker: &mut Walker<'a>,
    key: &'a OrderKey,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.payload(key.var.len())?;
    if let Some(expr) = &key.expr {
        walker.push(depth, Work::Expression(expr))?;
    }
    Ok(())
}

pub(super) fn visit_path_closure<'a>(
    walker: &mut Walker<'a>,
    path: &'a PathClosure,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.push(depth, Work::HopExpr(&path.hop))
}

pub(super) fn visit_hop_expr<'a>(
    walker: &mut Walker<'a>,
    hop: &'a HopExpr,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match hop {
        HopExpr::Pred(relation) => walker.push(depth, Work::HopRelation(relation))?,
        HopExpr::Inverse(inner) => walker.push(depth, Work::HopExpr(inner))?,
        HopExpr::Seq(left, right) => {
            walker.push(depth, Work::HopExpr(left))?;
            walker.push(depth, Work::HopExpr(right))?;
        }
        HopExpr::Alt(parts) | HopExpr::Nps(parts) => {
            walker.collection(parts.len())?;
            for part in parts {
                walker.push(depth, Work::HopExpr(part))?;
            }
        }
    }
    Ok(())
}

pub(super) fn visit_hop_relation<'a>(
    walker: &mut Walker<'a>,
    relation: &'a HopRelation,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.push(depth, Work::LogicalSource(&relation.source))?;
    walker.payload(relation.subj_col.len())?;
    walker.payload(relation.obj_col.len())
}

pub(super) fn visit_aggregation<'a>(
    walker: &mut Walker<'a>,
    aggregation: &'a Aggregation,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.collection(aggregation.keys.len())?;
    for key in &aggregation.keys {
        walker.push(depth, Work::GroupKey(key))?;
    }
    walker.collection(aggregation.aggs.len())?;
    for aggregate in &aggregation.aggs {
        walker.push(depth, Work::AggCol(aggregate))?;
    }
    Ok(())
}

pub(super) fn visit_group_key<'a>(
    walker: &mut Walker<'a>,
    key: &'a GroupKey,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.payload(key.var.len())?;
    walker.collection(key.cols.len())?;
    for col in &key.cols {
        walker.push(depth, Work::ColRef(col))?;
    }
    Ok(())
}

pub(super) fn visit_agg_col<'a>(
    walker: &mut Walker<'a>,
    aggregate: &'a AggCol,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.payload(aggregate.var.len())?;
    if let Some(arg) = &aggregate.arg {
        walker.push(depth, Work::ColRef(arg))?;
    }
    walker.push(depth, Work::ColRef(&aggregate.out))
}

pub(super) fn visit_subplan<'a>(
    walker: &mut Walker<'a>,
    join: &'a SubPlanJoin,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.push(depth, Work::Plan(&join.plan))?;
    push_conditions(walker, &join.on, depth)
}

pub(super) fn visit_rust_group<'a>(
    walker: &mut Walker<'a>,
    group: &'a RustGroup,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.collection(group.keys.len())?;
    for key in &group.keys {
        walker.payload(key.len())?;
    }
    walker.collection(group.aggs.len())?;
    for aggregate in &group.aggs {
        walker.push(depth, Work::RustAgg(aggregate))?;
    }
    walker.collection(group.post_exprs.len())?;
    for (name, expr) in &group.post_exprs {
        walker.payload(name.len())?;
        walker.push(depth, Work::Expression(expr))?;
    }
    Ok(())
}

pub(super) fn visit_rust_agg(
    walker: &mut Walker<'_>,
    aggregate: &RustAgg,
) -> Result<(), PlanMeasureError> {
    walker.payload(aggregate.out_var.len())?;
    if let Some(arg) = &aggregate.arg_var {
        walker.payload(arg.len())?;
    }
    Ok(())
}

pub(super) fn visit_logical_source(
    walker: &mut Walker<'_>,
    source: &LogicalSource,
) -> Result<(), PlanMeasureError> {
    let (LogicalSource::Table(value) | LogicalSource::Query(value)) = source;
    walker.payload(value.len())
}

pub(super) fn visit_term_map<'a>(
    walker: &mut Walker<'a>,
    term_map: &'a TermMap,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match term_map {
        TermMap::Constant(term) => walker.push(depth, Work::Term(term))?,
        TermMap::Column(column, spec) => {
            walker.payload(column.len())?;
            walker.push(depth, Work::TermSpec(spec))?;
        }
        TermMap::Template(template, spec) => {
            walker.push(depth, Work::Template(template))?;
            walker.push(depth, Work::TermSpec(spec))?;
        }
    }
    Ok(())
}

pub(super) fn visit_template<'a>(
    walker: &mut Walker<'a>,
    template: &'a Template,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    push_segments(walker, template.segments(), depth)
}

pub(super) fn visit_term_spec<'a>(
    walker: &mut Walker<'a>,
    spec: &'a TermSpec,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    if let Some(datatype) = &spec.datatype {
        walker.push(depth, Work::NamedNode(datatype))?;
    }
    if let Some(language) = &spec.language {
        walker.payload(language.len())?;
    }
    if let Some(base) = &spec.base {
        walker.payload(base.len())?;
    }
    Ok(())
}

pub(super) fn visit_segment(
    walker: &mut Walker<'_>,
    segment: &Segment,
) -> Result<(), PlanMeasureError> {
    let (Segment::Literal(value) | Segment::Column(value)) = segment;
    walker.payload(value.len())
}

fn push_conditions<'a>(
    walker: &mut Walker<'a>,
    conds: &'a [SqlCond],
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.collection(conds.len())?;
    for cond in conds {
        walker.push(depth, Work::SqlCond(cond))?;
    }
    Ok(())
}

fn push_segments<'a>(
    walker: &mut Walker<'a>,
    segments: &'a [Segment],
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.collection(segments.len())?;
    for segment in segments {
        walker.push(depth, Work::Segment(segment))?;
    }
    Ok(())
}
