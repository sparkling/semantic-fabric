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
    let Plan {
        branches,
        form,
        distinct: _,
        limit: _,
        offset: _,
        order,
        rust_group,
        dialect: _,
        dedup_scopes,
        construct_drops_some_branch_var: _,
    } = plan;
    push_branches(walker, branches, depth)?;
    walker.push(depth, Work::PlanForm(form))?;
    push_order_keys(walker, order, depth)?;
    if let Some(group) = rust_group {
        walker.push(depth, Work::RustGroup(group))?;
    }
    walker.collection(dedup_scopes.len())?;
    for scope in dedup_scopes.iter().flatten() {
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
    let DedupScope {
        group_id: _,
        key_bindings,
    } = scope;
    walker.collection(key_bindings.len())?;
    for (name, term) in key_bindings {
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
    let Branch {
        core,
        opts,
        bindings,
        where_conds,
        distinct: _,
        limit: _,
        offset: _,
        order,
        path,
        agg,
        subplan_joins,
        nps: _,
    } = branch;
    walker.collection(core.len())?;
    for scan in core {
        walker.push(depth, Work::Scan(scan))?;
    }
    walker.collection(opts.len())?;
    for opt in opts {
        walker.push(depth, Work::OptJoin(opt))?;
    }
    walker.collection(bindings.len())?;
    for (name, term) in bindings {
        walker.payload(name.len())?;
        walker.push(depth, Work::TermDef(term))?;
    }
    walker.collection(where_conds.len())?;
    for cond in where_conds {
        walker.push(depth, Work::SqlCond(cond))?;
    }
    push_order_keys(walker, order, depth)?;
    if let Some(path) = path {
        walker.push(depth, Work::PathClosure(path))?;
    }
    if let Some(aggregation) = agg {
        walker.push(depth, Work::Aggregation(aggregation))?;
    }
    walker.collection(subplan_joins.len())?;
    for join in subplan_joins {
        walker.push(depth, Work::SubPlanJoin(join))?;
    }
    Ok(())
}

pub(super) fn visit_scan<'a>(
    walker: &mut Walker<'a>,
    scan: &'a Scan,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let Scan { alias: _, source } = scan;
    match source {
        crate::iq::ScanSource::RefAtom { input, columns } => {
            walker.push(depth, Work::Branch(input))?;
            walker.collection(columns.len())?;
            for column in columns {
                walker.push(depth, Work::ColRef(column))?;
            }
            Ok(())
        }
        crate::iq::ScanSource::Logical(source) => walker.push(depth, Work::LogicalSource(source)),
        crate::iq::ScanSource::Path { closure, .. } => {
            walker.push(depth, Work::PathClosure(closure))
        }
        crate::iq::ScanSource::Projection {
            input,
            columns,
            guards,
            native_keys,
            ..
        } => {
            walker.push(depth, Work::Scan(input))?;
            walker.collection(columns.len())?;
            for (name, term) in columns {
                walker.payload(name.len())?;
                walker.push(depth, Work::TermMap(term))?;
            }
            walker.collection(native_keys.len())?;
            for (name, _) in native_keys {
                walker.payload(name.len())?;
            }
            push_conditions(walker, guards, depth)
        }
    }
}

pub(super) fn visit_opt_join<'a>(
    walker: &mut Walker<'a>,
    join: &'a OptJoin,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let OptJoin { scan, on, extra } = join;
    walker.push(depth, Work::Scan(scan))?;
    push_conditions(walker, on, depth)?;
    push_conditions(walker, extra, depth)
}

pub(super) fn visit_col_ref(walker: &mut Walker<'_>, col: &ColRef) -> Result<(), PlanMeasureError> {
    let ColRef { alias: _, column } = col;
    walker.payload(column.len())
}

pub(super) fn visit_term_def<'a>(
    walker: &mut Walker<'a>,
    term: &'a TermDef,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match term {
        TermDef::Const(term) => walker.push(depth, Work::Term(term))?,
        TermDef::Derived { term_map, alias: _ } => walker.push(depth, Work::TermMap(term_map))?,
        TermDef::R2rmlBlank {
            term_map,
            alias: _,
            graph,
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
        TermDef::Agg {
            col,
            kind: _,
            operand,
            fixed_type: _,
        } => {
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
    match graph {
        R2rmlGraphScope::Default => {}
        R2rmlGraphScope::Mapped { term_map, alias: _ } => {
            walker.push(depth, Work::TermMap(term_map))?
        }
    }
    Ok(())
}

pub(super) fn visit_sql_cond<'a>(
    walker: &mut Walker<'a>,
    cond: &'a SqlCond,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match cond {
        SqlCond::ColEq(left, right)
        | SqlCond::NativeColEq(left, right)
        | SqlCond::NullSafeEq(left, right) => {
            walker.push(depth, Work::ColRef(left))?;
            walker.push(depth, Work::ColRef(right))?;
        }
        SqlCond::Cmp(col, _operation, param) | SqlCond::NativeCmp(col, _operation, param) => {
            walker.push(depth, Work::ColRef(col))?;
            walker.payload(param.len())?;
        }
        SqlCond::StrMatch { col, op: _, param } => {
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
        SqlCond::PathExists {
            pc,
            conds,
            negated: _,
        } => {
            walker.push(depth, Work::PathClosure(pc))?;
            push_conditions(walker, conds, depth)?;
        }
        SqlCond::TemplateEq(left, _, right, _, _) => {
            super::mapping::push_segments(walker, left, depth)?;
            super::mapping::push_segments(walker, right, depth)?;
        }
    }
    Ok(())
}

pub(super) fn visit_order_key<'a>(
    walker: &mut Walker<'a>,
    key: &'a OrderKey,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let OrderKey {
        var,
        descending: _,
        expr,
    } = key;
    walker.payload(var.len())?;
    if let Some(expr) = expr {
        walker.push(depth, Work::Expression(expr))?;
    }
    Ok(())
}

pub(super) fn visit_path_closure<'a>(
    walker: &mut Walker<'a>,
    path: &'a PathClosure,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let PathClosure {
        alias: _,
        kind: _,
        hop,
    } = path;
    walker.push(depth, Work::HopExpr(hop))
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
    let HopRelation {
        source,
        subj_col,
        obj_col,
    } = relation;
    walker.push(depth, Work::LogicalSource(source))?;
    walker.payload(subj_col.len())?;
    walker.payload(obj_col.len())
}

pub(super) fn visit_aggregation<'a>(
    walker: &mut Walker<'a>,
    aggregation: &'a Aggregation,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let Aggregation { keys, aggs } = aggregation;
    walker.collection(keys.len())?;
    for key in keys {
        walker.push(depth, Work::GroupKey(key))?;
    }
    walker.collection(aggs.len())?;
    for aggregate in aggs {
        walker.push(depth, Work::AggCol(aggregate))?;
    }
    Ok(())
}

pub(super) fn visit_group_key<'a>(
    walker: &mut Walker<'a>,
    key: &'a GroupKey,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let GroupKey { var, cols } = key;
    walker.payload(var.len())?;
    walker.collection(cols.len())?;
    for col in cols {
        walker.push(depth, Work::ColRef(col))?;
    }
    Ok(())
}

pub(super) fn visit_agg_col<'a>(
    walker: &mut Walker<'a>,
    aggregate: &'a AggCol,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let AggCol {
        var,
        kind: _,
        arg,
        distinct: _,
        out,
        fixed_type: _,
    } = aggregate;
    walker.payload(var.len())?;
    if let Some(arg) = arg {
        walker.push(depth, Work::ColRef(arg))?;
    }
    walker.push(depth, Work::ColRef(out))
}

pub(super) fn visit_subplan<'a>(
    walker: &mut Walker<'a>,
    join: &'a SubPlanJoin,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let SubPlanJoin {
        alias: _,
        plan,
        on,
        left: _,
    } = join;
    walker.push(depth, Work::Plan(plan))?;
    push_conditions(walker, on, depth)
}

pub(super) fn visit_rust_group<'a>(
    walker: &mut Walker<'a>,
    group: &'a RustGroup,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let RustGroup {
        keys,
        aggs,
        post_exprs,
    } = group;
    walker.collection(keys.len())?;
    for key in keys {
        walker.payload(key.len())?;
    }
    walker.collection(aggs.len())?;
    for aggregate in aggs {
        walker.push(depth, Work::RustAgg(aggregate))?;
    }
    walker.collection(post_exprs.len())?;
    for (name, expr) in post_exprs {
        walker.payload(name.len())?;
        walker.push(depth, Work::Expression(expr))?;
    }
    Ok(())
}

pub(super) fn visit_rust_agg(
    walker: &mut Walker<'_>,
    aggregate: &RustAgg,
) -> Result<(), PlanMeasureError> {
    let RustAgg {
        out_var,
        kind: _,
        arg_var,
        distinct: _,
        fixed_type: _,
    } = aggregate;
    walker.payload(out_var.len())?;
    if let Some(arg) = arg_var {
        walker.payload(arg.len())?;
    }
    Ok(())
}

pub(super) fn push_branches<'a>(
    walker: &mut Walker<'a>,
    branches: &'a [Branch],
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.collection(branches.len())?;
    for branch in branches {
        walker.push(depth, Work::Branch(branch))?;
    }
    Ok(())
}

pub(super) fn push_order_keys<'a>(
    walker: &mut Walker<'a>,
    keys: &'a [OrderKey],
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.collection(keys.len())?;
    for key in keys {
        walker.push(depth, Work::OrderKey(key))?;
    }
    Ok(())
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
