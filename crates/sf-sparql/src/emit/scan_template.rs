//! Decoded template projections retain generated-text, not source-column, authority.
use super::*;
use sf_core::ir::{Template, TermSpec, TermType};

pub(super) fn actuals(scan: &Scan, dialect: Dialect, catalog: &ColumnCatalog) -> AliasActuals {
    match &scan.source {
        ScanSource::RefAtom { input, columns } => {
            ref_atom::actuals(input, columns, dialect, catalog)
        }
        ScanSource::Logical(source) => source_actuals(source, catalog),
        ScanSource::Path { closure, .. } => path_actuals(closure, catalog),
        ScanSource::Projection {
            input,
            columns,
            lexical_keys,
            ..
        } => {
            let inner = scan_actuals(input, dialect, catalog);
            let original_lexical_keys = lexical_keys;
            let lexical_keys = literal_roles::resolved(lexical_keys, dialect, &inner);
            let temporals = mysql_temporals(&scan.source, dialect, &inner);
            let sqlite_columns: HashMap<_, _> = columns
                .iter()
                .filter_map(|(name, term)| {
                    output_decode(term, dialect, &inner).map(|decode| (name.to_string(), decode))
                })
                .collect();
            let lexical_columns = sqlite_columns
                .iter()
                .filter(|(name, _)| {
                    let mut modes = lexical_keys
                        .iter()
                        .filter(|key| key.column.as_ref() == name.as_str());
                    modes.clone().any(|key| key.mode == LexicalMode::Decoded)
                        && modes.all(|key| {
                            matches!(key.mode, LexicalMode::Decoded | LexicalMode::Iri { .. })
                        })
                })
                .map(|(name, decode)| (name.clone(), *decode))
                .collect();
            let lexical_comparison_columns = sqlite_columns
                .iter()
                .filter(|(name, _)| {
                    original_lexical_keys.iter().any(|key| {
                        key.column.as_ref() == name.as_str() && key.mode == LexicalMode::Decoded
                    })
                })
                .map(|(name, decode)| (name.clone(), *decode))
                .collect();
            AliasActuals {
                datatype_columns: columns
                    .iter()
                    .filter_map(|(name, term)| {
                        let TermMap::Column(raw, _) = term else {
                            return None;
                        };
                        let code = if temporals.contains_key(name.as_ref()) {
                            Some(Some(sf_core::datatype::XsdTypeCode::String))
                        } else {
                            inner
                                .datatype_columns
                                .get(resolve_col(raw, Some(&inner.columns)))
                                .copied()
                        };
                        code.map(|code| (name.to_string(), code))
                    })
                    .collect(),
                natural_columns: columns
                    .iter()
                    .filter_map(|(name, term)| {
                        let TermMap::Column(raw, _) = term else {
                            return None;
                        };
                        (!temporals.contains_key(name.as_ref()))
                            .then(|| {
                                inner
                                    .natural_columns
                                    .get(resolve_col(raw, Some(&inner.columns)))
                            })
                            .flatten()
                            .map(|code| (name.to_string(), *code))
                    })
                    .collect(),
                scalar_columns: iri_cmp::projected_scalars(columns, &inner),
                sqlite_columns,
                lexical_columns,
                lexical_comparison_columns,
                source_kind: AliasSourceKind::Derived,
                columns: columns.iter().map(|(name, _)| name.to_string()).collect(),
                path: false,
                text_columns: columns
                    .iter()
                    .filter_map(|(name, term)| {
                        let key = match term {
                            TermMap::Column(_, _) if temporals.contains_key(name.as_ref()) => {
                                Some(TextKey::Verbatim)
                            }
                            TermMap::Column(column, _) => inner
                                .text_columns
                                .get(resolve_col(column, Some(&inner.columns)))
                                .copied(),
                            TermMap::Template(template, spec) => match template.segments() {
                                [Segment::Column(column)]
                                    if dialect != Dialect::MySql
                                        && spec.term_type != sf_core::ir::TermType::Iri =>
                                {
                                    inner
                                        .text_columns
                                        .get(resolve_col(column, Some(&inner.columns)))
                                        .copied()
                                        .map(|_| TextKey::Verbatim)
                                }
                                _ => Some(TextKey::Verbatim),
                            },
                            TermMap::Constant(_) => None,
                        };
                        key.map(|key| (name.to_string(), key))
                    })
                    .collect(),
            }
        }
    }
}

/// Preserve lexical-only DATE/DATETIME consumers before lossy MySQL temp copies.
/// A natural literal or native comparison consumer withholds this authority.
pub(super) fn mysql_temporals(
    source: &ScanSource,
    dialect: Dialect,
    inner: &AliasActuals,
) -> HashMap<String, NativeScalarKey> {
    let ScanSource::Projection {
        columns,
        lexical_keys,
        native_keys,
        ..
    } = source
    else {
        return HashMap::new();
    };
    let lexical_keys = literal_roles::resolved(lexical_keys, dialect, inner);
    columns
        .iter()
        .filter_map(|(name, term)| {
            let TermMap::Column(raw, _) = term else {
                return None;
            };
            let resolved = resolve_col(raw, Some(&inner.columns));
            // Consumer vetoes were captured by authored spelling. Resolving a
            // different positive spelling must not revive a discarded veto.
            let mode = lexical_keys
                .iter()
                .any(|key| key.column == *raw && key.mode == LexicalMode::Decoded);
            if dialect != Dialect::MySql || !mode || native_keys.iter().any(|(key, _)| key == name)
            {
                return None;
            }
            inner
                .scalar_columns
                .get(resolved)
                .filter(|key| {
                    matches!(
                        key,
                        NativeScalarKey::MysqlDate | NativeScalarKey::MysqlDateTime
                    )
                })
                .map(|key| (name.to_string(), *key))
        })
        .collect()
}

pub(super) fn output_decode(
    term: &TermMap,
    dialect: Dialect,
    inner: &AliasActuals,
) -> Option<SqliteDecode> {
    match term {
        TermMap::Column(raw, _) => inner
            .sqlite_columns
            .get(resolve_col(raw, Some(&inner.columns)))
            .copied(),
        TermMap::Template(_, spec)
            if dialect == Dialect::Sqlite && spec.term_type == TermType::Iri =>
        {
            Some(SqliteDecode {
                declared: Some(sf_core::datatype::XsdTypeCode::String),
                padding: None,
            })
        }
        _ => None,
    }
}

pub(super) fn render(
    recipe: &Template,
    spec: &TermSpec,
    alias: usize,
    distinct: bool,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
) -> Result<String> {
    if spec.term_type == TermType::Iri && spec.base.is_some() {
        let crate::iq::iri_cmp::IriOperand::Template { parts, base } =
            crate::iq::iri_cmp::IriOperand::from_map(
                &TermMap::Template(recipe.clone(), spec.clone()),
                alias,
            )
            .unwrap()
        else {
            unreachable!()
        };
        // Both recipe and base are mapping-owned text, never query parameters.
        let expression = iri_cmp::template_lexical(&parts, dialect, catalog, actuals)?;
        catalog
            .lexical_keys
            .store(true, std::sync::atomic::Ordering::Relaxed);
        return Ok(format!(
            "__sf_iri_key_v1({expression}, {})",
            sql_string_literal(base.as_deref().unwrap())
        ));
    }
    let iri = spec.term_type == TermType::Iri;
    let mut decoded = HashMap::new();
    if dialect == Dialect::Sqlite && iri {
        for segment in recipe.segments() {
            let Segment::Column(name) = segment else {
                continue;
            };
            let column = ColRef::new(alias, name.clone());
            if let Some(decode) = lexical_key::column_decode(&column, actuals) {
                decoded.insert(
                    name.as_ref(),
                    lexical_key::expression(colref(&column, dialect, actuals), decode, catalog),
                );
            } else if distinct {
                return Err(Error::Unsupported(
                    "rendered IRI dedup requires each live SQLite decoder".into(),
                ));
            }
        }
    }
    let expression = render_template_inline(recipe.segments(), iri, dialect, |name| {
        decoded.get(name).cloned().unwrap_or_else(|| {
            path_comparison::rdf_column(&ColRef::new(alias, name), dialect, catalog, actuals)
        })
    })?;
    if distinct && iri && dialect == Dialect::Sqlite && spec.base.is_none() {
        catalog
            .lexical_keys
            .store(true, std::sync::atomic::Ordering::Relaxed);
        // Final key validation is required even when COUNT hides the generated term.
        Ok(format!("__sf_iri_key_v1({expression}, NULL)"))
    } else {
        Ok(expression)
    }
}

pub(super) fn distinct_key(term: &TermMap, expression: &str, dialect: Dialect) -> Result<String> {
    if supports_distinct(term) && dialect == Dialect::Sqlite {
        Ok(path_comparison::exact_text(expression.to_owned(), dialect))
    } else {
        Err(Error::Unsupported("distinct computed scan recipe".into()))
    }
}

pub(super) fn supports_distinct(term: &TermMap) -> bool {
    matches!(term, TermMap::Template(_, spec) if spec.term_type == TermType::Iri)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iq::LexicalKey;
    use sf_sql::backend::ResultColumn;

    #[test]
    fn mysql_temporal_copy_requires_original_lexical_only_consumers() {
        for key in [NativeScalarKey::MysqlDate, NativeScalarKey::MysqlDateTime] {
            for lexical in [false, true] {
                for native in [false, true] {
                    let source = LogicalSource::Table("items".into());
                    let mut catalog = ColumnCatalog::default();
                    catalog
                        .insert_live_result(
                            &source,
                            vec![
                                ResultColumn {
                                    natural_datatype: None,
                                    name: "src".into(),
                                    native_scalar: Some(key),
                                    text_key: None,
                                    sqlite_decode: None,
                                },
                                // Unused text forces the original window implementation.
                                ResultColumn {
                                    natural_datatype: None,
                                    name: "unused".into(),
                                    native_scalar: None,
                                    text_key: Some(TextKey::Verbatim),
                                    sqlite_decode: None,
                                },
                            ],
                        )
                        .unwrap();
                    let scan = Scan {
                        alias: 7,
                        source: ScanSource::Projection {
                            input: Box::new(Scan {
                                alias: 3,
                                source: source.clone().into(),
                            }),
                            columns: vec![(
                                "renamed".into(),
                                TermMap::Column("SRC".into(), TermSpec::plain_literal()),
                            )],
                            guards: vec![],
                            distinct: true,
                            native_keys: if native {
                                vec![("renamed".into(), true)]
                            } else {
                                vec![]
                            },
                            lexical_keys: if lexical {
                                vec![LexicalKey {
                                    column: "SRC".into(),
                                    mode: LexicalMode::Decoded,
                                }]
                            } else {
                                vec![]
                            },
                        },
                    };
                    let output = actuals(&scan, Dialect::MySql, &catalog);
                    assert!(
                        output.scalar_columns.is_empty(),
                        "raw temporal proof never survives a copy"
                    );
                    assert_eq!(
                        output.text_columns.contains_key("renamed"),
                        lexical && !native
                    );
                    let mut mixed = scan.clone();
                    if let ScanSource::Projection { columns, .. } = &mut mixed.source {
                        columns.push((
                            "natural".into(),
                            TermMap::Column("src".into(), TermSpec::plain_literal()),
                        ));
                    }
                    let mixed = actuals(&mixed, Dialect::MySql, &catalog);
                    assert!(
                        !mixed.text_columns.contains_key("natural"),
                        "folded source spelling must not borrow another consumer's positive proof"
                    );
                    assert!(actuals(&scan, Dialect::Postgres, &catalog)
                        .text_columns
                        .is_empty());
                    let input = Branch::single(Scan {
                        alias: 3,
                        source: source.clone().into(),
                    });
                    assert!(
                        ref_atom::actuals(
                            &input,
                            &[ColRef::new(3, "src")],
                            Dialect::MySql,
                            &catalog
                        )
                        .scalar_columns
                        .is_empty(),
                        "raw RefAtom copies confer no temporal recipe"
                    );
                    if !native {
                        let sql =
                            scan_ref(&scan, Dialect::MySql, &catalog, &mut vec![], &mut 0).unwrap();
                        assert_eq!(sql.contains("CAST(t3.`src` AS CHAR)"), lexical, "{sql}");
                        if lexical {
                            assert!(
                                sql.matches("CAST(t3.`src` AS CHAR)").count() >= 2,
                                "output and window key both preserve lexical: {sql}"
                            );
                            assert!(!sql.contains("PARTITION BY t3.`src`"), "{sql}");
                        }
                    }
                    catalog.insert(&source, vec!["src".into(), "unused".into()]);
                    assert!(
                        actuals(&scan, Dialect::MySql, &catalog)
                            .text_columns
                            .is_empty(),
                        "names-only refresh revokes cast authority"
                    );
                }
            }
        }
    }
}
