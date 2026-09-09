//! Seal static IRI-template atom identity in SQL before outer joins/projection.
use super::*;
use crate::iq::ScanSource;

pub(super) fn wrap(branch: &mut Branch, dialect: sf_sql::Dialect) -> bool {
    if dialect != sf_sql::Dialect::Sqlite
        || branch.core.len() != 1
        || !branch.opts.is_empty()
        || !branch.subplan_joins.is_empty()
        || branch.path.is_some()
        || branch.agg.is_some()
        || !branch.order.is_empty()
        || !matches!(branch.core[0].source, ScanSource::Logical(_))
        || !branch
            .bindings
            .values()
            .any(|def| !binding_is_injective(def))
    {
        return false;
    }
    let alias = branch.core[0].alias;
    if !branch.where_conds.iter().all(|condition| {
        matches!(condition, SqlCond::IsNull(c) | SqlCond::IsNotNull(c) if c.alias == alias)
    }) || !branch.bindings.values().all(|def| match def {
        TermDef::Const(_) => true,
        TermDef::Derived {
            term_map: TermMap::Template(_, spec),
            alias: owner,
        } => *owner == alias && spec.term_type == TermType::Iri && spec.base.is_none(),
        _ => false,
    }) {
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
        let output = TermMap::Column(name.clone(), spec.clone());
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
