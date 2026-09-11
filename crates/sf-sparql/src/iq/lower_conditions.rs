//! Ordinary IQ condition admission. Owned final branches keep their payloads;
//! borrowed earlier branches prepay each actual SQL copy. EXISTS still delegates
//! to the existing lowering path, not a claim about its remaining inner work.
use super::*;
use crate::build::control::BuildVec;
use crate::leftjoin::work::push_owned;
use crate::plan_measure::clone_root::CompilerCloneRootV1;

/// Preserve outermost-first FILTER groups until Construction establishes bindings.
pub(super) fn peel_filters(
    mut node: IqNode,
    mode: CompilerWorkMode<'_>,
) -> Result<(IqNode, Vec<Vec<IqCond>>)> {
    let work = BuildWork::new(mode);
    work.checkpoint()?;
    let mut groups = BuildVec::new(Vec::new());
    while let IqNode::Filter { child, cond } = node {
        work.charge(1)?;
        work.push(&mut groups, cond)?;
        node = *child;
    }
    work.checkpoint()?;
    Ok((node, groups.into_inner()))
}

pub(super) fn apply_conds_to_branches(
    conds: Vec<IqCond>,
    branches: &mut Vec<Branch>,
    dialect: sf_sql::Dialect,
    mode: CompilerWorkMode<'_>,
) -> Result<()> {
    let work = BuildWork::new(mode);
    work.checkpoint()?;
    if conds.is_empty() || branches.is_empty() {
        return Ok(());
    }
    work.charge(1)?;
    let mut last = branches.pop().expect("nonempty branches");
    for branch in branches.iter_mut() {
        apply_conds(&conds, branch, dialect, mode)?;
    }
    apply_owned_conds(conds, &mut last, dialect, mode)?;
    // This owned transfer reuses the slot released by pop, without allocation.
    work.charge(1)?;
    branches.push(last);
    work.checkpoint()
}

pub(super) fn apply_conds(
    conds: &[IqCond],
    branch: &mut Branch,
    dialect: sf_sql::Dialect,
    mode: CompilerWorkMode<'_>,
) -> Result<()> {
    let work = BuildWork::new(mode);
    work.checkpoint()?;
    // Construction can retain empty FILTER groups: pay every visited group.
    work.charge(1)?;
    for cond in conds {
        let sql = lower_iq_cond(cond, branch, dialect, mode)?;
        append(branch, sql, work)?;
    }
    work.checkpoint()
}

pub(super) fn apply_owned_conds(
    conds: Vec<IqCond>,
    branch: &mut Branch,
    dialect: sf_sql::Dialect,
    mode: CompilerWorkMode<'_>,
) -> Result<()> {
    let work = BuildWork::new(mode);
    work.checkpoint()?;
    work.charge(1)?;
    for cond in conds {
        let sql = lower_owned_iq_cond(cond, branch, dialect, mode)?;
        append(branch, sql, work)?;
    }
    work.checkpoint()
}

fn append(branch: &mut Branch, condition: SqlCond, work: BuildWork<'_>) -> Result<()> {
    let _span = tracing::debug_span!("sf.compiler.condition_append").entered();
    // Keep previous conditions visible during construction/validation above.
    push_owned(work, &mut branch.where_conds, condition)
}

fn expression(
    expression: &Expression,
    outer: &Branch,
    dialect: sf_sql::Dialect,
    mode: CompilerWorkMode<'_>,
) -> Result<SqlCond> {
    let CompilerWorkMode::Metered(cx) = mode else {
        return filter_cond(expression, outer, dialect).map_err(Error::Unsupported);
    };
    let condition = {
        let _span = tracing::debug_span!("sf.compiler.condition_filter_construct").entered();
        cx.filter_condition(expression, &outer.bindings, dialect)?
    };
    {
        let _span = tracing::debug_span!("sf.compiler.condition_filter_validate").entered();
        crate::iq::iri_cmp::validate_filter_source_with_work_mode(
            &condition, outer, dialect, mode,
        )?;
    }
    cx.checkpoint()?;
    Ok(condition)
}

pub(super) fn lower_iq_cond(
    cond: &IqCond,
    outer: &Branch,
    dialect: sf_sql::Dialect,
    mode: CompilerWorkMode<'_>,
) -> Result<SqlCond> {
    let _span = tracing::debug_span!("sf.compiler.condition_borrowed").entered();
    borrowed(cond, outer, dialect, BuildWork::new(mode))
}

fn borrowed(
    cond: &IqCond,
    outer: &Branch,
    dialect: sf_sql::Dialect,
    work: BuildWork<'_>,
) -> Result<SqlCond> {
    let work = work.enter()?;
    let result = match cond {
        IqCond::Sql(sql) => {
            if let CompilerWorkMode::Metered(cx) = work.mode {
                cx.reserve_ast_copy(CompilerCloneRootV1::SqlCond(sql))?;
            }
            sql.clone()
        }
        IqCond::Expr(expr) => expression(expr, outer, dialect, work.mode)?,
        IqCond::And(conditions) | IqCond::Or(conditions) => {
            let mut out = work.vector(conditions.len())?;
            for condition in conditions {
                out.push(borrowed(condition, outer, dialect, work)?);
            }
            if matches!(cond, IqCond::And(_)) {
                SqlCond::And(out)
            } else {
                SqlCond::Or(out)
            }
        }
        IqCond::Not(condition) => {
            let condition = borrowed(condition, outer, dialect, work)?;
            SqlCond::Not(work.boxed(condition)?)
        }
        IqCond::Exists(node) => lower_iq_exists(node, outer, false, false, dialect, work.mode)?,
        IqCond::NotExists { inner, is_minus } => {
            lower_iq_exists(inner, outer, true, *is_minus, dialect, work.mode)?
        }
    };
    work.checkpoint()?;
    Ok(result)
}

pub(super) fn lower_owned_iq_cond(
    cond: IqCond,
    outer: &Branch,
    dialect: sf_sql::Dialect,
    mode: CompilerWorkMode<'_>,
) -> Result<SqlCond> {
    let _span = tracing::debug_span!("sf.compiler.condition_owned").entered();
    owned(cond, outer, dialect, BuildWork::new(mode))
}

fn owned(
    cond: IqCond,
    outer: &Branch,
    dialect: sf_sql::Dialect,
    work: BuildWork<'_>,
) -> Result<SqlCond> {
    let work = work.enter()?;
    let is_and = matches!(cond, IqCond::And(_));
    let result = match cond {
        IqCond::Sql(sql) => sql,
        IqCond::Expr(expr) => expression(&expr, outer, dialect, work.mode)?,
        IqCond::And(conditions) | IqCond::Or(conditions) => {
            let mut out = work.vector(conditions.len())?;
            for condition in conditions {
                out.push(owned(condition, outer, dialect, work)?);
            }
            if is_and {
                SqlCond::And(out)
            } else {
                SqlCond::Or(out)
            }
        }
        IqCond::Not(condition) => {
            let condition = owned(*condition, outer, dialect, work)?;
            SqlCond::Not(work.boxed(condition)?)
        }
        IqCond::Exists(node) => {
            lower_owned_iq_exists(*node, outer, false, false, dialect, work.mode)?
        }
        IqCond::NotExists { inner, is_minus } => {
            lower_owned_iq_exists(*inner, outer, true, is_minus, dialect, work.mode)?
        }
    };
    work.checkpoint()?;
    Ok(result)
}
