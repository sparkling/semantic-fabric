//! Preserve raw decoder validity independently of nullness and RDF identity.
use super::{Branch, SqlCond, TermDef};

pub(crate) fn validate_term(
    def: &TermDef,
    dialect: sf_sql::Dialect,
    conditions: &mut Vec<SqlCond>,
) {
    if dialect == sf_sql::Dialect::Postgres {
        conditions.extend(def.columns().into_iter().map(SqlCond::DecodedIsNotNull));
    }
}

pub(crate) fn references(cond: &SqlCond, alias: Option<usize>) -> bool {
    match cond {
        SqlCond::DecodedIsNotNull(c) => alias.is_none_or(|a| a == c.alias),
        SqlCond::Not(inner) => references(inner, alias),
        SqlCond::And(cs)
        | SqlCond::Or(cs)
        | SqlCond::Exists { conds: cs, .. }
        | SqlCond::NotExists { conds: cs, .. }
        | SqlCond::PathExists { conds: cs, .. } => cs.iter().any(|c| references(c, alias)),
        _ => false,
    }
}

pub(crate) fn branch_references(branch: &Branch, alias: usize) -> bool {
    branch
        .where_conds
        .iter()
        .chain(branch.opts.iter().flat_map(|o| o.on.iter().chain(&o.extra)))
        .any(|c| references(c, Some(alias)))
}
