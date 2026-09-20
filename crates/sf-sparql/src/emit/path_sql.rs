//! Prospective source work for borrowed property-path SQL construction.
use super::*;
use sf_sql::source_work::{SourceVec, SourceWork};
use source_control::validation_error as error;
#[cfg(test)]
mod callsite_tests;
pub(super) mod format;
mod leaf;
#[cfg(test)]
mod tests;
macro_rules! sql { ($work:expr, $($arg:tt)*) => { format::render($work, format_args!($($arg)*))? }; }
pub(super) fn prelude(
    pc: &PathClosure,
    alias: usize,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: SourceWork<'_>,
) -> Result<String> {
    let cte = sql!(work, "t{}", alias);
    let hop = hop_sql(&pc.hop, dialect, catalog, work)?;
    let (sf_s, sf_o) = (
        leaf::quote("sf_s", dialect, work)?,
        leaf::quote("sf_o", dialect, work)?,
    );
    Ok(match pc.kind {
        PathKind::One => {
            let one_distinct = if matches!(pc.hop, HopExpr::Nps(_)) {
                ""
            } else {
                "DISTINCT "
            };
            sql!(
                work,
                "WITH {cte}({sf_s}, {sf_o}) AS \
                 (SELECT {one_distinct}{sf_s}, {sf_o} FROM ({hop}) hx)"
            )
        }
        PathKind::ZeroOrOne => {
            let refl = leaf::reflexive(&pc.hop, dialect, catalog, work)?;
            sql!(
                work,
                "WITH {cte}({sf_s}, {sf_o}) AS (SELECT DISTINCT {sf_s}, {sf_o} FROM \
                 (SELECT {sf_s}, {sf_o} FROM ({hop}) hx UNION {refl}) z)"
            )
        }
        PathKind::OneOrMore | PathKind::ZeroOrMore => {
            let cte_raw = sql!(work, "t{}r", alias);
            let one_hop = sql!(work, "SELECT {sf_s}, {sf_o} FROM ({hop}) hx");
            let anchor = if matches!(pc.kind, PathKind::ZeroOrMore) {
                let refl = leaf::reflexive(&pc.hop, dialect, catalog, work)?;
                sql!(work, "{one_hop} UNION {refl}")
            } else {
                one_hop
            };
            let recursive = sql!(
                work,
                "SELECT c.{sf_s} AS {sf_s}, h.{sf_o} AS {sf_o} \
                 FROM {cte_raw} c JOIN ({hop}) h ON c.{sf_o} = h.{sf_s}"
            );
            sql!(
                work,
                "WITH RECURSIVE {cte_raw}({sf_s}, {sf_o}) AS ({anchor} UNION {recursive}), \
                 {cte}({sf_s}, {sf_o}) AS (SELECT DISTINCT {sf_s}, {sf_o} FROM {cte_raw})"
            )
        }
    })
}

pub(super) fn derived(
    pc: &PathClosure,
    alias: usize,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: SourceWork<'_>,
) -> Result<String> {
    let with = prelude(pc, alias, dialect, catalog, work)?;
    let sf_s = leaf::quote("sf_s", dialect, work)?;
    let sf_o = leaf::quote("sf_o", dialect, work)?;
    format::render(
        work,
        format_args!("{with} SELECT {sf_s}, {sf_o} FROM t{alias}"),
    )
}

fn hop_sql(
    hop: &HopExpr,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: SourceWork<'_>,
) -> Result<String> {
    enum Task<'a> {
        Enter(&'a HopExpr),
        Finish(&'a HopExpr),
    }
    let mut tasks = SourceVec::default();
    let mut results = SourceVec::<String>::default();
    tasks.push(Task::Enter(hop), work).map_err(error)?;
    let sf_s = leaf::quote("sf_s", dialect, work)?;
    let sf_o = leaf::quote("sf_o", dialect, work)?;
    while let Some(task) = tasks.pop() {
        work.charge(1).map_err(error)?;
        match task {
            Task::Enter(node) => {
                tasks.push(Task::Finish(node), work).map_err(error)?;
                match node {
                    HopExpr::Pred(_) => {}
                    HopExpr::Inverse(child) => {
                        tasks.push(Task::Enter(child), work).map_err(error)?
                    }
                    HopExpr::Seq(a, b) => {
                        tasks.push(Task::Enter(b), work).map_err(error)?;
                        tasks.push(Task::Enter(a), work).map_err(error)?;
                    }
                    HopExpr::Alt(parts) | HopExpr::Nps(parts) => {
                        for child in parts.iter().rev() {
                            tasks.push(Task::Enter(child), work).map_err(error)?;
                        }
                    }
                }
            }
            Task::Finish(node) => {
                let value = match node {
                    HopExpr::Pred(rel) => leaf::relation(rel, dialect, catalog, work)?,
                    HopExpr::Inverse(_) => {
                        let inner_sql = results.pop().expect("completed child");
                        sql!(
                            work,
                            "SELECT x.{sf_o} AS {sf_s}, x.{sf_s} AS {sf_o} FROM ({inner_sql}) x"
                        )
                    }
                    HopExpr::Seq(_, _) => {
                        let b_sql = results.pop().expect("right child");
                        let a_sql = results.pop().expect("left child");
                        sql!(work, "SELECT a.{sf_s} AS {sf_s}, b.{sf_o} AS {sf_o} FROM ({a_sql}) a JOIN ({b_sql}) b ON a.{sf_o} = b.{sf_s}")
                    }
                    HopExpr::Alt(parts) | HopExpr::Nps(parts) => {
                        let nps = matches!(node, HopExpr::Nps(_));
                        let mut arms = work.vector::<String>(parts.len()).map_err(error)?;
                        for _ in parts {
                            arms.push(results.pop().expect("completed arm"));
                        }
                        work.product(arms.len(), std::mem::size_of::<String>())
                            .map_err(error)?;
                        arms.reverse();
                        let distinct = if nps { "DISTINCT " } else { "" };
                        for arm in &mut arms {
                            *arm = sql!(work, "SELECT {distinct}{sf_s}, {sf_o} FROM ({arm}) u");
                        }
                        format::join(work, &arms, if nps { " UNION ALL " } else { " UNION " })?
                    }
                };
                results.push(value, work).map_err(error)?;
            }
        }
    }
    Ok(results.pop().expect("root hop"))
}
