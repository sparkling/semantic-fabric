//! Seal rendered IRI-template atom identity before joins, constants or projection
//! can hide the construction that determines cardinality.
use super::*;
use crate::iq::ScanSource;

#[path = "rendered_distinct_work.rs"]
mod work;
pub(super) use work::guard as guard_with_work;
pub(super) use work::wrap_with_work;

pub(crate) fn seal_late(branch: &mut Branch, dialect: sf_sql::Dialect) -> crate::Result<()> {
    if branch
        .where_conds
        .iter()
        .any(crate::iq::iri_cmp::contains_late_template)
        && !wrap(branch, dialect)
    {
        return Err(crate::Error::Unsupported(
            "late template atom requires qualified SQLite rendered IRI/constant keys".into(),
        ));
    }
    Ok(())
}

pub(super) fn wrap(branch: &mut Branch, dialect: sf_sql::Dialect) -> bool {
    wrap_with_work(
        branch,
        dialect,
        crate::build::control::BuildWork::new(crate::CompilerWorkMode::Uncontrolled),
    )
    .expect("uncontrolled rendered distinct")
}

#[cfg(test)]
fn wrap_raw(branch: &mut Branch, dialect: sf_sql::Dialect) -> bool {
    if dialect != sf_sql::Dialect::Sqlite
        || branch.core.len() != 1
        || !branch.opts.is_empty()
        || !branch.subplan_joins.is_empty()
        || branch.path.is_some()
        || branch.agg.is_some()
        || !branch.order.is_empty()
        || !matches!(branch.core[0].source, ScanSource::Logical(_))
        || !(branch
            .bindings
            .values()
            .any(|def| !binding_is_injective(def))
            || branch
                .where_conds
                .iter()
                .any(crate::iq::iri_cmp::contains_late_template))
    {
        return false;
    }
    let alias = branch.core[0].alias;
    if !branch
        .where_conds
        .iter()
        .all(|condition| crate::iq::iri_cmp::atom_guard(condition, alias))
        || !branch.bindings.values().all(|def| match def {
            TermDef::Const(_) => true,
            TermDef::Derived {
                term_map: TermMap::Template(_, spec),
                alias: owner,
            } => *owner == alias && spec.term_type == TermType::Iri,
            _ => false,
        })
    {
        return false;
    }
    let input = branch.core.pop().expect("one validated logical scan");
    let mut columns = Vec::new();
    for def in branch.bindings.values_mut() {
        let TermDef::Derived { term_map, .. } = def else {
            continue;
        };
        let TermMap::Template(_, spec) = term_map else {
            unreachable!("validated static IRI template")
        };
        let name: Box<str> = format!("rv{}", columns.len()).into();
        let mut resolved = spec.clone();
        resolved.base = None;
        let output = TermMap::Column(name.clone(), resolved);
        columns.push((name, std::mem::replace(term_map, output)));
    }
    branch.core.push(Scan {
        alias,
        source: ScanSource::Projection {
            input: Box::new(input),
            columns,
            guards: std::mem::take(&mut branch.where_conds),
            distinct: true,
            native_keys: Vec::new(),
            lexical_keys: Vec::new(),
        },
    });
    branch.distinct = false;
    true
}
