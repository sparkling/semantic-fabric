//! Preserve conjunction order and the EXISTS/MINUS distinction while visiting
//! every condition and reserving each replacement collection/box before use.
use super::control::RowWork;
use super::normalize_node;
use crate::iq::node::IqCond;
use crate::Result;

pub(super) fn normalize_conds(conds: Vec<IqCond>, work: RowWork<'_>) -> Result<Vec<IqCond>> {
    work.charge(1)?;
    let mut out = work.vector(conds.len())?;
    for cond in conds {
        out.push(normalize_cond(cond, work)?);
    }
    Ok(out)
}

fn normalize_cond(cond: IqCond, work: RowWork<'_>) -> Result<IqCond> {
    let work = work.enter()?;
    let result = match cond {
        leaf @ (IqCond::Expr(_) | IqCond::Sql(_)) => leaf,
        IqCond::And(cs) => IqCond::And(normalize_conds(cs, work)?),
        IqCond::Or(cs) => IqCond::Or(normalize_conds(cs, work)?),
        IqCond::Not(c) => IqCond::Not(work.boxed(normalize_cond(*c, work)?)?),
        IqCond::Exists(n) => IqCond::Exists(work.boxed(normalize_node(*n, work)?)?),
        IqCond::NotExists { inner, is_minus } => IqCond::NotExists {
            inner: work.boxed(normalize_node(*inner, work)?)?,
            is_minus,
        },
    };
    work.checkpoint()?;
    Ok(result)
}
