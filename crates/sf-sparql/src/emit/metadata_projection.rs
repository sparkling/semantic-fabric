use super::*;
use crate::iq::scan::LexicalMode;
use crate::iq::ScanSource;
use metadata_ref_atom::{contains, insert, set_insert};
use metadata_source::metadata_lookup as lookup;
use sf_core::ir::TermType;
use sf_sql::source_work::SourceWork;
use source_control::validation_error as error;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iq::{LexicalKey, Scan};
    use sf_core::ir::TermSpec;
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};

    #[test]
    fn controlled_projection_facts_keep_duplicates_and_exact_admission() {
        use sf_core::datatype::XsdTypeCode;
        let budget = |n| QueryBudget::new(QueryLimits::new(u64::MAX, n, u64::MAX, u64::MAX));
        for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
            let logical = LogicalSource::Table("items".into());
            let mut inner = source_actuals(&logical, &ColumnCatalog::default());
            inner.columns = vec!["id".into(), "ID".into()];
            inner
                .datatype_columns
                .insert("id".into(), Some(XsdTypeCode::Integer));
            inner
                .natural_columns
                .insert("id".into(), Some(XsdTypeCode::Integer));
            inner
                .scalar_columns
                .insert("id".into(), NativeScalarKey::Integer);
            inner.text_columns.insert("id".into(), TextKey::Verbatim);
            inner.static_iri_columns.insert("id".into());
            inner.iri_unreserved_columns.insert("id".into());
            let decode = SqliteDecode {
                declared: Some(XsdTypeCode::Integer),
                padding: None,
            };
            inner.sqlite_columns.insert("id".into(), decode);
            let columns = vec![
                ("id".into(), TermMap::Column("id".into(), TermSpec::iri())),
                (
                    "id".into(),
                    TermMap::Column("missing".into(), TermSpec::iri()),
                ),
                ("ID".into(), TermMap::Column("Id".into(), TermSpec::iri())),
            ];
            // Independent retained raw helpers protect duplicate filter-map semantics.
            assert_eq!(
                iri_cmp::projected_scalars(&columns, &inner),
                inner.scalar_columns
            );
            assert_eq!(
                scan::template::output_decode(&columns[0].1, dialect, &inner),
                Some(decode)
            );
            assert_eq!(
                scan::template::output_decode(&columns[1].1, dialect, &inner),
                None
            );
            let source = ScanSource::Projection {
                input: Box::new(Scan {
                    alias: 0,
                    source: logical.into(),
                }),
                columns,
                guards: vec![],
                distinct: false,
                native_keys: vec![],
                lexical_keys: vec![LexicalKey {
                    column: "id".into(),
                    mode: LexicalMode::Decoded,
                }],
            };
            let mut expected = inner.clone();
            expected.source_kind = AliasSourceKind::Derived;
            expected.columns = vec!["id".into(), "id".into(), "ID".into()];
            expected.lexical_columns.insert("id".into(), decode);
            expected
                .lexical_comparison_columns
                .insert("id".into(), decode);
            let run = |control: &QueryBudget| {
                actuals(&source, dialect, &inner, SourceWork::new(Some(control)))
            };
            let measured = budget(u64::MAX);
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

    #[test]
    fn controlled_projection_lexicalization_matches_raw_consumer_vetoes() {
        let budget = |n| QueryBudget::new(QueryLimits::new(u64::MAX, n, u64::MAX, u64::MAX));
        for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
            for raw in ["id", "ID", "Id", "missing"] {
                for mode in [
                    LexicalMode::Decoded,
                    LexicalMode::Natural,
                    LexicalMode::DecodedWithNatural,
                ] {
                    for native in [false, true] {
                        let logical = LogicalSource::Table("items".into());
                        let mut inner = source_actuals(&logical, &ColumnCatalog::default());
                        inner.columns = vec!["id".into(), "ID".into()];
                        inner
                            .scalar_columns
                            .insert("id".into(), NativeScalarKey::MysqlFloat8);
                        inner
                            .scalar_columns
                            .insert("ID".into(), NativeScalarKey::MysqlDate);
                        let keys = vec![LexicalKey {
                            column: raw.into(),
                            mode: mode.clone(),
                        }];
                        let source = ScanSource::Projection {
                            input: Box::new(Scan {
                                alias: 0,
                                source: logical.into(),
                            }),
                            columns: vec![(
                                "value".into(),
                                TermMap::Column(raw.into(), TermSpec::plain_literal()),
                            )],
                            guards: vec![],
                            distinct: false,
                            native_keys: if native {
                                vec![("value".into(), false)]
                            } else {
                                vec![]
                            },
                            lexical_keys: keys.clone(),
                        };
                        let expected =
                            scan::template::mysql_lexical_columns(&source, dialect, &inner);
                        assert_eq!(
                            !expected.is_empty(),
                            dialect == Dialect::MySql
                                && matches!(raw, "id" | "ID")
                                && mode == LexicalMode::Decoded
                                && !native
                        );
                        let run = |control: &QueryBudget| {
                            let work = SourceWork::new(Some(control));
                            let resolved =
                                literal_roles::resolved_controlled(&keys, dialect, &inner, work)?;
                            mysql_lexical_columns(&source, dialect, &inner, &resolved, work)
                        };
                        let measured = budget(u64::MAX);
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
        }
    }
}

fn resolved_name<'a>(
    raw: &'a str,
    inner: &'a AliasActuals,
    work: SourceWork<'_>,
) -> Result<&'a str> {
    for candidate in &inner.columns {
        work.charge(3).map_err(error)?;
        work.product(2, candidate.len().min(raw.len()))
            .map_err(error)?;
    }
    Ok(resolve_col(raw, Some(&inner.columns)))
}

/// Same consumer-veto law as the raw helper, with prospective work admission.
pub(super) fn mysql_lexical_columns(
    source: &ScanSource,
    dialect: Dialect,
    inner: &AliasActuals,
    keys: &[crate::iq::LexicalKey],
    work: SourceWork<'_>,
) -> Result<HashMap<String, NativeScalarKey>> {
    let ScanSource::Projection {
        columns,
        native_keys,
        ..
    } = source
    else {
        return Err(metadata::invariant());
    };
    let mut out = HashMap::new();
    for (name, term) in columns {
        work.charge(1).map_err(error)?;
        let TermMap::Column(raw, _) = term else {
            continue;
        };
        let resolved = resolved_name(raw, inner, work)?;
        let mut decoded = false;
        let mut only_decoded = true;
        for key in keys {
            work.charge(1).map_err(error)?;
            let candidate = resolved_name(&key.column, inner, work)?;
            work.charge(candidate.len().min(resolved.len()))
                .map_err(error)?;
            work.charge(key.column.len().min(raw.len()))
                .map_err(error)?;
            if candidate == resolved {
                decoded |= key.column == *raw && key.mode == LexicalMode::Decoded;
                only_decoded &= key.mode == LexicalMode::Decoded;
            }
        }
        let mut native = false;
        for (key, _) in native_keys {
            work.charge(1).map_err(error)?;
            work.charge(key.len().min(name.len())).map_err(error)?;
            native |= key == name;
        }
        if dialect != Dialect::MySql || !decoded || !only_decoded || native {
            continue;
        }
        if let Some(key) = metadata_source::metadata_lookup(&inner.scalar_columns, resolved, work)?
            .copied()
            .filter(|key| {
                matches!(
                    key,
                    NativeScalarKey::MysqlDate
                        | NativeScalarKey::MysqlDateTime
                        | NativeScalarKey::MysqlFloat4
                        | NativeScalarKey::MysqlFloat8
                )
            })
        {
            metadata_ref_atom::insert(&mut out, name, key, work)?;
        }
    }
    work.checkpoint().map_err(error)?;
    Ok(out)
}

/// Derive one projection from already-built child facts; never recurse here.
pub(super) fn actuals(
    source: &ScanSource,
    dialect: Dialect,
    inner: &AliasActuals,
    work: SourceWork<'_>,
) -> Result<AliasActuals> {
    let ScanSource::Projection {
        columns,
        lexical_keys,
        ..
    } = source
    else {
        return Err(metadata::invariant());
    };
    let original_lexical_keys = lexical_keys;
    let lexical_keys = literal_roles::resolved_controlled(lexical_keys, dialect, inner, work)?;
    let lexicalized = mysql_lexical_columns(source, dialect, inner, &lexical_keys, work)?;
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
        iri_unreserved_columns: HashSet::new(),
        static_iri_columns: HashSet::new(),
        text_columns: HashMap::new(),
    };
    for (name, term) in columns {
        work.charge(1).map_err(error)?;
        out.columns.push(work.string(name).map_err(error)?);
        match term {
            TermMap::Column(raw, spec) => {
                let raw = resolved_name(raw, inner, work)?;
                let converted = lookup(&lexicalized, name, work)?.copied();
                let datatype = if converted.is_some() {
                    Some(Some(sf_core::datatype::XsdTypeCode::String))
                } else {
                    lookup(&inner.datatype_columns, raw, work)?.copied()
                };
                if let Some(code) = datatype {
                    insert(&mut out.datatype_columns, name, code, work)?;
                }
                if converted.is_none() {
                    if let Some(code) = lookup(&inner.natural_columns, raw, work)?.copied() {
                        insert(&mut out.natural_columns, name, code, work)?;
                    }
                    if let Some(key) =
                        lookup(&inner.scalar_columns, raw, work)?
                            .copied()
                            .filter(|key| {
                                !matches!(
                                    key,
                                    NativeScalarKey::MysqlDate | NativeScalarKey::MysqlDateTime
                                )
                            })
                    {
                        insert(&mut out.scalar_columns, name, key, work)?;
                    }
                }
                if let Some(decode) = lookup(&inner.sqlite_columns, raw, work)?.copied() {
                    insert(&mut out.sqlite_columns, name, decode, work)?;
                }
                let text = if converted.is_some() {
                    Some(TextKey::Verbatim)
                } else {
                    lookup(&inner.text_columns, raw, work)?.copied()
                };
                if let Some(key) = text {
                    insert(&mut out.text_columns, name, key, work)?;
                }
                if converted.is_some_and(mysql_float_value::identity::is_float)
                    || contains(&inner.iri_unreserved_columns, raw, work)?
                {
                    set_insert(&mut out.iri_unreserved_columns, name, work)?;
                }
                if spec.term_type == TermType::Iri
                    && spec.base.is_none()
                    && contains(&inner.static_iri_columns, raw, work)?
                {
                    set_insert(&mut out.static_iri_columns, name, work)?;
                }
            }
            TermMap::Template(template, spec) => {
                if dialect == Dialect::Sqlite && spec.term_type == TermType::Iri {
                    insert(
                        &mut out.sqlite_columns,
                        name,
                        SqliteDecode {
                            declared: Some(sf_core::datatype::XsdTypeCode::String),
                            padding: None,
                        },
                        work,
                    )?;
                }
                let text = match template.segments() {
                    [Segment::Column(raw)]
                        if dialect != Dialect::MySql && spec.term_type != TermType::Iri =>
                    {
                        let raw = resolved_name(raw, inner, work)?;
                        lookup(&inner.text_columns, raw, work)?.map(|_| TextKey::Verbatim)
                    }
                    _ => Some(TextKey::Verbatim),
                };
                if let Some(key) = text {
                    insert(&mut out.text_columns, name, key, work)?;
                }
                if dialect == Dialect::MySql
                    && spec.term_type == TermType::Iri
                    && spec.base.is_none()
                {
                    let mut qualified = true;
                    for segment in template.segments() {
                        work.charge(1).map_err(error)?;
                        if let Segment::Column(raw) = segment {
                            let raw = resolved_name(raw, inner, work)?;
                            qualified &= lookup(&inner.text_columns, raw, work)?.is_some()
                                || lookup(&inner.scalar_columns, raw, work)?.is_some_and(|key| {
                                    iri_cmp::supports_scalar_template_key(*key, dialect)
                                });
                        }
                    }
                    if qualified {
                        set_insert(&mut out.static_iri_columns, name, work)?;
                    }
                }
            }
            TermMap::Constant(_) => {}
        }
    }
    // Iterate output names, not randomized map order. Duplicate names repeat the
    // same fact; absence never erases an earlier successful projection fact.
    for name in &out.columns {
        work.charge(1).map_err(error)?;
        let Some(decode) = lookup(&out.sqlite_columns, name, work)?.copied() else {
            continue;
        };
        let mut decoded = false;
        let mut allowed = true;
        for key in &lexical_keys {
            work.charge(1).map_err(error)?;
            work.charge(key.column.len().min(name.len()))
                .map_err(error)?;
            if key.column.as_ref() == name {
                decoded |= key.mode == LexicalMode::Decoded;
                allowed &= matches!(key.mode, LexicalMode::Decoded | LexicalMode::Iri { .. });
            }
        }
        if decoded && allowed {
            insert(&mut out.lexical_columns, name, decode, work)?;
        }
        let mut comparison = false;
        for key in original_lexical_keys {
            work.charge(1).map_err(error)?;
            work.charge(key.column.len().min(name.len()))
                .map_err(error)?;
            comparison |= key.column.as_ref() == name && key.mode == LexicalMode::Decoded;
        }
        if comparison {
            insert(&mut out.lexical_comparison_columns, name, decode, work)?;
        }
    }
    work.checkpoint().map_err(error)?;
    Ok(out)
}
