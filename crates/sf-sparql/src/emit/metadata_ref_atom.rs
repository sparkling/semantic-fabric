//! Paid positional reference-atom metadata; no child traversal or recipe cloning.
use super::*;
use metadata_source::metadata_lookup;
use sf_sql::source_work::SourceWork;
use source_control::validation_error as error;

fn allocation() -> Error {
    Error::Sql("reference metadata allocation failed".into())
}

fn pay_insert<'a>(
    names: impl Iterator<Item = &'a String>,
    len: usize,
    slot: usize,
    name: &str,
    work: SourceWork<'_>,
) -> Result<()> {
    work.charge(1).map_err(error)?;
    work.charge(name.len()).map_err(error)?;
    for candidate in names {
        work.charge(2).map_err(error)?;
        work.charge(candidate.len()).map_err(error)?; // possible rehash
        work.charge(candidate.len().min(name.len()))
            .map_err(error)?;
    }
    work.product(len, len).map_err(error)?; // worst-case probes while rehashing existing entries
    work.product(len, slot).map_err(error)?; // possible entry moves
    work.charge(slot).map_err(error)
}

pub(super) fn insert<V>(
    map: &mut HashMap<String, V>,
    name: &str,
    value: V,
    work: SourceWork<'_>,
) -> Result<()> {
    pay_insert(
        map.keys(),
        map.len(),
        std::mem::size_of::<(String, V)>(),
        name,
        work,
    )?;
    let name = work.string(name).map_err(error)?;
    map.try_reserve(1).map_err(|_| allocation())?;
    map.insert(name, value);
    work.checkpoint().map_err(error)
}

pub(super) fn set_insert(
    set: &mut HashSet<String>,
    name: &str,
    work: SourceWork<'_>,
) -> Result<()> {
    pay_insert(
        set.iter(),
        set.len(),
        std::mem::size_of::<String>(),
        name,
        work,
    )?;
    let name = work.string(name).map_err(error)?;
    set.try_reserve(1).map_err(|_| allocation())?;
    set.insert(name);
    work.checkpoint().map_err(error)
}

pub(super) fn contains(set: &HashSet<String>, name: &str, work: SourceWork<'_>) -> Result<bool> {
    work.charge(name.len()).map_err(error)?;
    for candidate in set {
        work.charge(2).map_err(error)?;
        work.charge(candidate.len().min(name.len()))
            .map_err(error)?;
    }
    Ok(set.contains(name))
}

pub(super) fn actuals(
    columns: &[ColRef],
    sources: &ActualColumns,
    work: SourceWork<'_>,
) -> Result<AliasActuals> {
    let mut out = AliasActuals {
        datatype_columns: HashMap::new(),
        natural_columns: HashMap::new(),
        scalar_columns: HashMap::new(),
        sqlite_columns: HashMap::new(),
        lexical_columns: HashMap::new(),
        lexical_comparison_columns: HashMap::new(),
        source_kind: AliasSourceKind::Derived,
        columns: work.vector(columns.len()).map_err(error)?,
        path: false,
        text_columns: HashMap::new(),
        static_iri_columns: HashSet::new(),
        iri_unreserved_columns: HashSet::new(),
    };
    for (index, column) in columns.iter().enumerate() {
        use std::fmt::Write;
        work.charge(1).map_err(error)?;
        let digits = index.checked_ilog10().unwrap_or(0) as usize + 1;
        work.product(2, digits + 1).map_err(error)?;
        let mut name = String::new();
        name.try_reserve_exact(digits + 1)
            .map_err(|_| allocation())?;
        write!(&mut name, "c{index}").map_err(|_| allocation())?;
        work.charge(1).map_err(error)?;
        work.charge(sources.len()).map_err(error)?;
        if let Some(source) = sources.get(&column.alias) {
            for candidate in &source.columns {
                work.charge(3).map_err(error)?;
                work.product(2, candidate.len().min(column.column.len()))
                    .map_err(error)?;
            }
            let raw = resolve_col(&column.column, Some(&source.columns));
            let datatype = metadata_lookup(&source.datatype_columns, raw, work)?.copied();
            let natural = metadata_lookup(&source.natural_columns, raw, work)?.copied();
            let scalar = metadata_lookup(&source.scalar_columns, raw, work)?.copied();
            let decode = metadata_lookup(&source.sqlite_columns, raw, work)?.copied();
            let text = metadata_lookup(&source.text_columns, raw, work)?.copied();
            if let Some(code) = datatype {
                insert(&mut out.datatype_columns, &name, code, work)?;
            }
            if let Some(code) = natural {
                insert(&mut out.natural_columns, &name, code, work)?;
            }
            if let Some(key) = scalar.filter(|key| {
                !matches!(
                    key,
                    NativeScalarKey::MysqlDate | NativeScalarKey::MysqlDateTime
                )
            }) {
                insert(&mut out.scalar_columns, &name, key, work)?;
            }
            if let Some(decode) = decode {
                insert(&mut out.sqlite_columns, &name, decode, work)?;
            }
            if let Some(key) = text {
                insert(&mut out.text_columns, &name, key, work)?;
            }
            if contains(&source.static_iri_columns, raw, work)? {
                set_insert(&mut out.static_iri_columns, &name, work)?;
            }
            if contains(&source.iri_unreserved_columns, raw, work)?
                && text == Some(TextKey::Verbatim)
                && scalar.is_none()
                && datatype == Some(Some(sf_core::datatype::XsdTypeCode::String))
            {
                set_insert(&mut out.iri_unreserved_columns, &name, work)?;
            }
        }
        out.columns.push(name);
    }
    work.checkpoint().map_err(error)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::datatype::XsdTypeCode;
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};
    #[test]
    fn controlled_reference_metadata_preserves_all_raw_facts_and_exact_admission() {
        let budget = |n| QueryBudget::new(QueryLimits::new(u64::MAX, n, u64::MAX, u64::MAX));
        for scalar in [
            None,
            Some(NativeScalarKey::MysqlDate),
            Some(NativeScalarKey::MysqlDateTime),
            Some(NativeScalarKey::PostgresFloat8),
        ] {
            let mut source = source_actuals(
                &LogicalSource::Table("items".into()),
                &ColumnCatalog::default(),
            );
            source.columns = ["id", "ID", "value"].map(str::to_string).into();
            source.datatype_columns = [
                ("id".into(), Some(XsdTypeCode::String)),
                ("ID".into(), None),
            ]
            .into();
            source.natural_columns = [
                ("id".into(), None),
                ("value".into(), Some(XsdTypeCode::Integer)),
            ]
            .into();
            source.text_columns.insert("id".into(), TextKey::Verbatim);
            source.sqlite_columns.insert(
                "id".into(),
                SqliteDecode {
                    declared: None,
                    padding: Some(2),
                },
            );
            source
                .static_iri_columns
                .extend(["id".into(), "value".into()]);
            source.iri_unreserved_columns.insert("id".into());
            if let Some(scalar) = scalar {
                source.scalar_columns.insert("id".into(), scalar);
            }
            let sources = [(0, source)].into();
            let columns: Vec<_> = ["id", "ID", "Id", "VALUE", "missing", "id"]
                .into_iter()
                .flat_map(|name| [ColRef::new(0, name), ColRef::new(9, name)])
                .collect();
            let expected = ref_atom::from_sources(&columns, &sources);
            let run =
                |control: &QueryBudget| actuals(&columns, &sources, SourceWork::new(Some(control)));
            let measured = budget(u64::MAX);
            assert_eq!(run(&measured).unwrap(), expected);
            assert_eq!(expected.columns[10], "c10");
            assert_eq!(
                expected.iri_unreserved_columns.contains("c0"),
                scalar.is_none()
            );
            assert_eq!(
                expected.scalar_columns.contains_key("c0"),
                scalar == Some(NativeScalarKey::PostgresFloat8)
            );
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
