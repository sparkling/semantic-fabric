//! Prospective logical-source metadata construction; raw wrappers share its semantics.
use super::*;
use sf_sql::source_work::SourceWork;
use source_control::validation_error as error;

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::datatype::XsdTypeCode;
    use sf_core::ir::{Template, TermSpec};
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    fn budget(n: u64) -> QueryBudget {
        QueryBudget::new(QueryLimits::new(u64::MAX, n, u64::MAX, u64::MAX))
    }

    #[test]
    fn controlled_metadata_type_merges_match_raw_unknown_and_missing_law() {
        let map = |bits: usize| -> HashMap<usize, Option<XsdTypeCode>> {
            (0..3)
                .filter_map(|i| match (bits >> (2 * i)) & 3 {
                    0 => None,
                    1 => Some((i, None)),
                    2 => Some((i, Some(XsdTypeCode::Integer))),
                    _ => Some((i, Some(XsdTypeCode::Double))),
                })
                .collect()
        };
        for old in 0..65 {
            for current in 0..64 {
                let old = (old < 64).then(|| map(old));
                let current = map(current);
                let mut expected_types = old.clone();
                literal_datatype::merge(&mut expected_types, current.clone());
                let mut expected_common = old.clone();
                if let Some(common) = &mut expected_common {
                    common.retain(|index, code| current.get(index) == Some(code));
                } else {
                    expected_common = Some(current.clone());
                }
                let run = |control: &QueryBudget| -> Result<_> {
                    let mut types = old.clone();
                    let mut common = old.clone();
                    merge_types(&mut types, current.clone(), SourceWork::new(Some(control)))?;
                    intersect(&mut common, current.clone(), SourceWork::new(Some(control)))?;
                    Ok((types, common))
                };
                let measured = budget(u64::MAX);
                let expected = (expected_types, expected_common);
                assert_eq!(run(&measured).unwrap(), expected);
                let n = measured.consumed(QueryCharge::SourceWork);
                assert_eq!(measured.consumed(QueryCharge::CompilerWork), 0);
                assert_eq!(run(&budget(n)).unwrap(), expected);
                assert!(matches!(
                    run(&budget(n - 1)),
                    Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                ));
            }
        }
    }

    #[test]
    fn controlled_metadata_dedup_proof_matches_raw_without_rejection() {
        for count in 0..3 {
            for spec in [
                TermSpec::iri(),
                TermSpec::plain_literal(),
                TermSpec::blank_node(),
            ] {
                for recipe in ["{a}", "{a}/{b}", "{a}{b}"] {
                    let mut branch = Branch::empty();
                    branch.core = (0..count)
                        .map(|alias| crate::iq::Scan {
                            alias,
                            source: LogicalSource::Table("items".into()).into(),
                        })
                        .collect();
                    branch.bindings.insert(
                        "x".into(),
                        TermDef::Derived {
                            alias: 0,
                            term_map: TermMap::Template(
                                Template::parse(recipe).unwrap(),
                                spec.clone(),
                            ),
                        },
                    );
                    for distinct in [false, true] {
                        let expected = crate::cascade::eligible_for_term_dedup_with_distinct(
                            &branch, distinct,
                        );
                        let run = |control: &QueryBudget| {
                            term_dedup(&branch, distinct, SourceWork::new(Some(control)))
                        };
                        let measured = budget(u64::MAX);
                        assert_eq!(run(&measured).unwrap(), expected);
                        let n = measured.consumed(QueryCharge::SourceWork);
                        assert_eq!(run(&budget(n)).unwrap(), expected);
                        assert!(matches!(
                            run(&budget(n - 1)),
                            Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
                        ));
                    }
                }
            }
        }
    }
}

pub(super) fn term_dedup(branch: &Branch, distinct: bool, work: SourceWork<'_>) -> Result<bool> {
    work.charge(1).map_err(error)?;
    if !distinct
        || branch.path.is_some()
        || branch.agg.is_some()
        || branch
            .core
            .len()
            .saturating_add(branch.opts.len())
            .saturating_add(branch.subplan_joins.len())
            > 1
    {
        return Ok(false);
    }
    let mut noninjective = false;
    let mut safe = true;
    for def in branch.bindings.values() {
        work.charge(1).map_err(error)?;
        let injective = match def {
            TermDef::Derived { term_map, .. } => ref_atom::injective_map(term_map, work)?,
            TermDef::R2rmlBlank {
                term_map, graph, ..
            } => {
                ref_atom::injective_map(term_map, work)?
                    && match graph {
                        R2rmlGraphScope::Default => true,
                        R2rmlGraphScope::Mapped { term_map, .. } => {
                            ref_atom::injective_map(term_map, work)?
                        }
                    }
            }
            _ => true,
        };
        noninjective |= !injective;
        safe &= crate::cascade::binding_is_term_dedup_safe_with_injectivity(def, injective);
    }
    work.checkpoint().map_err(error)?;
    Ok(noninjective && safe)
}

pub(super) fn index_insert<V>(
    map: &mut HashMap<usize, V>,
    key: usize,
    value: V,
    work: SourceWork<'_>,
) -> Result<()> {
    pay_index_insert(map.len(), std::mem::size_of::<(usize, V)>(), work)?;
    map.try_reserve(1)
        .map_err(|_| Error::Sql("metadata index allocation failed".into()))?;
    map.insert(key, value);
    work.checkpoint().map_err(error)
}

pub(super) fn pay_index_insert(len: usize, slot: usize, work: SourceWork<'_>) -> Result<()> {
    work.charge(1).map_err(error)?;
    work.product(len, len).map_err(error)?;
    work.product(len, slot).map_err(error)?;
    work.charge(len).map_err(error)?;
    work.charge(slot).map_err(error)
}

pub(super) fn merge_types(
    common: &mut Option<HashMap<usize, Option<sf_core::datatype::XsdTypeCode>>>,
    current: HashMap<usize, Option<sf_core::datatype::XsdTypeCode>>,
    work: SourceWork<'_>,
) -> Result<()> {
    work.charge(1).map_err(error)?;
    if let Some(old) = common {
        let total = old.len().saturating_add(current.len());
        // Prepay rehash, two full lookup/entry passes and capacity traversal.
        for _ in 0..3 {
            work.product(total, total).map_err(error)?;
        }
        work.charge(old.capacity()).map_err(error)?;
        work.charge(current.capacity()).map_err(error)?;
        work.product(
            total,
            std::mem::size_of::<(usize, Option<sf_core::datatype::XsdTypeCode>)>(),
        )
        .map_err(error)?;
        old.try_reserve(current.len())
            .map_err(|_| Error::Sql("metadata merge allocation failed".into()))?;
        work.charge(old.capacity()).map_err(error)?;
    }
    literal_datatype::merge(common, current);
    work.checkpoint().map_err(error)
}

pub(super) fn intersect<V: PartialEq>(
    common: &mut Option<HashMap<usize, V>>,
    current: HashMap<usize, V>,
    work: SourceWork<'_>,
) -> Result<()> {
    work.charge(1).map_err(error)?;
    if let Some(old) = common {
        work.charge(old.capacity()).map_err(error)?;
        work.product(old.len(), current.len().saturating_add(2))
            .map_err(error)?;
        old.retain(|key, value| current.get(key) == Some(value));
    } else {
        *common = Some(current);
    }
    work.checkpoint().map_err(error)
}

#[cfg(test)]
pub(super) fn source_actuals(source: &LogicalSource, catalog: &ColumnCatalog) -> AliasActuals {
    source_actuals_controlled(source, catalog, sf_sql::source_work::SourceWork::new(None))
        .expect("uncontrolled source metadata construction")
}

pub(super) fn metadata_lookup<'a, V>(
    map: &'a HashMap<String, V>,
    key: &str,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<Option<&'a V>> {
    work.charge(key.len())
        .map_err(source_control::validation_error)?;
    for candidate in map.keys() {
        work.charge(2).map_err(source_control::validation_error)?;
        work.charge(candidate.len().min(key.len()))
            .map_err(source_control::validation_error)?;
    }
    Ok(map.get(key))
}

pub(super) fn metadata_copy<V: Copy, W>(
    source: Option<&HashMap<String, V>>,
    work: sf_sql::source_work::SourceWork<'_>,
    mut value: impl FnMut(&str, V) -> Result<Option<W>>,
) -> Result<HashMap<String, W>> {
    let mut output = HashMap::new();
    if let Some(source) = source {
        // Prepay a stable envelope, independent of randomized insertion order.
        // Every name may be hashed/compared/moved on every insertion.
        for name in source.keys() {
            work.charge(2).map_err(source_control::validation_error)?;
            work.charge(name.len())
                .map_err(source_control::validation_error)?;
            work.product(source.len(), name.len())
                .map_err(source_control::validation_error)?;
        }
        work.product(source.len(), source.len())
            .map_err(source_control::validation_error)?;
        work.product(source.len(), std::mem::size_of::<(String, W)>())
            .map_err(source_control::validation_error)?;
        output
            .try_reserve(source.len())
            .map_err(|_| Error::Sql("source metadata allocation failed".into()))?;
        for (name, input) in source {
            if let Some(value) = value(name, *input)? {
                let name = work
                    .string(name)
                    .map_err(source_control::validation_error)?;
                output.insert(name, value);
            }
        }
    }
    work.checkpoint()
        .map_err(source_control::validation_error)?;
    Ok(output)
}

/// Pay the structural cost of copying `branch` and rewriting each of its scans
/// before either happens: the iterative walk pays every node once, each source
/// text is copied and then wrapped, and each binding is re-keyed. Owned payload
/// inside conditions and term definitions is counted per node, not per byte;
/// prior compiler admission bounds it.
pub(super) fn branch_copy_controlled(branch: &Branch, work: SourceWork<'_>) -> Result<()> {
    let Some(control) = work.control() else {
        return Ok(());
    };
    let sources =
        source_control::live_metadata_sources_controlled(std::slice::from_ref(branch), control)
            .map_err(error)?;
    for source in sources {
        let text = match source {
            LogicalSource::Table(text) | LogicalSource::Query(text) => text,
        };
        work.product(text.len(), 2).map_err(error)?;
    }
    for name in branch.bindings.keys() {
        work.charge(name.len() + 1).map_err(error)?;
    }
    work.checkpoint().map_err(error)
}

pub(super) fn source_actuals_controlled(
    source: &LogicalSource,
    catalog: &ColumnCatalog,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<AliasActuals> {
    use source_control::validation_error as error;
    let name = match source {
        LogicalSource::Table(name) | LogicalSource::Query(name) => name,
    };
    work.charge(name.len()).map_err(error)?;
    work.charge(2).map_err(error)?;
    let key = source_key(source);
    let datatypes = metadata_lookup(&catalog.datatypes_by_source, &key, work)?;
    let datatype_columns = metadata_copy(datatypes, work, |_, code| Ok(Some(Some(code))))?;
    let names = metadata_lookup(&catalog.by_source, &key, work)?
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut columns = work.vector(names.len()).map_err(error)?;
    for name in names {
        columns.push(work.string(name).map_err(error)?);
    }
    let mut actuals = AliasActuals {
        datatype_columns,
        natural_columns: HashMap::new(),
        scalar_columns: metadata_copy(
            metadata_lookup(&catalog.scalars_by_source, &key, work)?,
            work,
            |_, code| Ok(Some(code)),
        )?,
        sqlite_columns: metadata_copy(
            metadata_lookup(&catalog.sqlite_by_source, &key, work)?,
            work,
            |_, code| Ok(Some(code)),
        )?,
        lexical_columns: HashMap::new(),
        lexical_comparison_columns: HashMap::new(),
        source_kind: match source {
            LogicalSource::Table(_) => AliasSourceKind::Table,
            LogicalSource::Query(_) => AliasSourceKind::Query,
        },
        columns,
        path: false,
        text_columns: metadata_copy(
            metadata_lookup(&catalog.text_by_source, &key, work)?,
            work,
            |_, code| Ok(Some(code)),
        )?,
        static_iri_columns: HashSet::new(),
        iri_unreserved_columns: HashSet::new(),
    };
    actuals.natural_columns = metadata_copy(datatypes, work, |name, code| {
        let scalar = metadata_lookup(&actuals.scalar_columns, name, work)?.copied();
        let text = metadata_lookup(&actuals.text_columns, name, work)?.copied();
        Ok(natural_literal::qualified_source(code, scalar, text).then_some(Some(code)))
    })?;
    work.checkpoint().map_err(error)?;
    Ok(actuals)
}
