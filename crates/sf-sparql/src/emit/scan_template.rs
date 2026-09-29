//! Decoded template projections retain generated-text, not source-column, authority.
#[cfg(test)]
use super::scan_actuals as actuals;
use super::*;
use crate::iq::iri_cmp::{IriOperand, IriPart};
use sf_core::ir::{Template, TermSpec, TermType};

/// Preserve lexical-only DATE/DATETIME and signed float spellings before lossy
/// MySQL temp copies. Outputs become text, never native scalar authorities.
/// A natural literal or native comparison consumer withholds this authority.
#[cfg(test)]
pub(in crate::emit) fn mysql_lexical_columns(
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
            let mut modes = lexical_keys
                .iter()
                .filter(|key| resolve_col(&key.column, Some(&inner.columns)) == resolved);
            let decoded = modes
                .clone()
                .any(|key| key.column == *raw && key.mode == LexicalMode::Decoded);
            if dialect != Dialect::MySql
                || !decoded
                || !modes.all(|key| key.mode == LexicalMode::Decoded)
                || native_keys.iter().any(|(key, _)| key == name)
            {
                return None;
            }
            inner
                .scalar_columns
                .get(resolved)
                .filter(|key| {
                    matches!(
                        key,
                        NativeScalarKey::MysqlDate
                            | NativeScalarKey::MysqlDateTime
                            | NativeScalarKey::MysqlFloat4
                            | NativeScalarKey::MysqlFloat8
                    )
                })
                .map(|key| (name.to_string(), *key))
        })
        .collect()
}

#[cfg(test)]
pub(in crate::emit) fn output_decode(
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

#[allow(
    clippy::too_many_arguments,
    reason = "existing crate-internal render boundary; signature preserved"
)]
pub(super) fn render(
    recipe: &Template,
    spec: &TermSpec,
    alias: usize,
    distinct: bool,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    if spec.term_type == TermType::Iri && spec.base.is_some() {
        // Pay the recipe and modifier copies the operand view takes.
        for segment in recipe.segments() {
            let len = match segment {
                Segment::Literal(text) => text.len(),
                Segment::Column(name) => name.len(),
            };
            work.charge(len + 1)
                .map_err(source_control::validation_error)?;
        }
        work.charge(
            4 + spec.datatype.as_ref().map_or(0, |dt| dt.as_str().len())
                + spec.language.as_deref().map_or(0, str::len)
                + spec.base.as_deref().map_or(0, str::len),
        )
        .map_err(source_control::validation_error)?;
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
        let expression =
            iri_cmp::template_lexical_controlled(&parts, dialect, catalog, actuals, work)?;
        catalog
            .lexical_keys
            .store(true, std::sync::atomic::Ordering::Relaxed);
        return Ok(format!(
            "__sf_iri_key_v1({expression}, {})",
            sql_string_literal(base.as_deref().unwrap())
        ));
    }
    let iri = spec.term_type == TermType::Iri;
    if actuals
        .get(&alias)
        .is_some_and(|source| iri_cmp::qualified_static_template(recipe, spec, dialect, source))
    {
        // Encode each decoder-owned scalar alphabet once. Feeding its large
        // shortest-decimal expression through the arbitrary UTF-8 byte encoder
        // needlessly repeats that expression for every byte of every row.
        let parts = recipe
            .segments()
            .iter()
            .map(|segment| match segment {
                Segment::Literal(text) => Ok(sql_string_literal(text)),
                Segment::Column(name) => {
                    let column = ColRef::new(alias, name.clone());
                    if path_comparison::column_text(&column, actuals).is_some() {
                        iri_cmp::encode_text_column_controlled(
                            &column, dialect, catalog, actuals, work,
                        )
                    } else {
                        let key = iri_cmp::scalar_column(&column, actuals)
                            .expect("qualified template has a live decoder for every slot");
                        iri_cmp::scalar_template_key(
                            key,
                            &colref(&column, dialect, actuals),
                            dialect,
                        )
                    }
                }
            })
            .collect::<Result<Vec<_>>>()?;
        let expression = if parts.is_empty() {
            "''".into()
        } else {
            format!("CONCAT({})", parts.join(", "))
        };
        return Ok(path_comparison::exact_text(expression, dialect));
    }
    let mut decoded = HashMap::new();
    if dialect == Dialect::MySql {
        for segment in recipe.segments() {
            if let Segment::Column(name) = segment {
                let column = ColRef::new(alias, name.clone());
                if let Some(key) = iri_cmp::scalar_column(&column, actuals)
                    .filter(|key| mysql_float_value::identity::is_float(*key))
                {
                    decoded.insert(
                        name.as_ref(),
                        iri_cmp::scalar_lexical(key, &colref(&column, dialect, actuals), dialect)?,
                    );
                }
            }
        }
    }
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
    let expression = render_template_inline(
        recipe.segments(),
        iri,
        dialect,
        catalog,
        |name| {
            decoded.get(name).cloned().unwrap_or_else(|| {
                path_comparison::rdf_column(&ColRef::new(alias, name), dialect, catalog, actuals)
            })
        },
        work,
    )?;
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

/// Copy only total integer IRI constraints below an already authorized D1 copy.
/// VALUES/OPTIONAL otherwise materializes every permitted float for each branch.
/// Keep the original ON predicate and policy barrier; fallible decoders stay out.
#[cfg(test)]
pub(in crate::emit) fn restrict_optional<'a>(
    opt: &'a crate::iq::OptJoin,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> std::borrow::Cow<'a, crate::iq::Scan> {
    restrict_optional_controlled(
        opt,
        dialect,
        catalog,
        sf_sql::source_work::SourceWork::new(None),
    )
    .expect("uncontrolled optional restriction")
}

pub(in crate::emit) fn restrict_optional_controlled<'a>(
    opt: &'a crate::iq::OptJoin,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<std::borrow::Cow<'a, crate::iq::Scan>> {
    use crate::iq::ScanSource;
    let unchanged = std::borrow::Cow::Borrowed(&opt.scan);
    let ScanSource::Projection {
        input,
        columns,
        guards,
        distinct: true,
        ..
    } = &opt.scan.source
    else {
        return Ok(unchanged);
    };
    if dialect != Dialect::MySql
        || input.alias != opt.scan.alias
        || input.source.logical().is_none()
        || !guards
            .iter()
            .any(|guard| matches!(guard, SqlCond::NativeCmp(..)))
        || !columns
            .iter()
            .all(|(name, term)| matches!(term, TermMap::Column(raw, _) if raw == name))
    {
        return Ok(unchanged);
    }
    let actuals = HashMap::from([(
        input.alias,
        crate::emit::metadata::scan_actuals_controlled(input, dialect, catalog, work)?,
    )]);
    let additional: Vec<_> = opt
        .on
        .iter()
        .chain(&opt.extra)
        .filter(|guard| {
            let SqlCond::IriCmp(cmp) = guard else {
                return false;
            };
            let parts = match (&cmp.left, &cmp.right) {
                (IriOperand::Template { parts, base: None }, IriOperand::Constant(_))
                | (IriOperand::Constant(_), IriOperand::Template { parts, base: None }) => parts,
                _ => return false,
            };
            parts.iter().any(|part| matches!(part, IriPart::Column(_)))
                && parts.iter().all(|part| match part {
                    IriPart::Literal(_) => true,
                    IriPart::Column(c) => {
                        c.alias == input.alias
                            && columns.iter().any(|(name, _)| name == &c.column)
                            && crate::emit::iri_cmp::scalar_column(c, &actuals)
                                == Some(NativeScalarKey::Integer)
                            && crate::emit::literal_datatype::fact(c, &actuals)
                                == Some(Some(sf_core::datatype::XsdTypeCode::Integer))
                    }
                })
        })
        .cloned()
        .collect();
    if additional.is_empty() {
        return Ok(unchanged);
    }
    let mut scan = opt.scan.clone();
    let ScanSource::Projection { guards, .. } = &mut scan.source else {
        return Err(super::metadata::invariant());
    };
    guards.extend(additional);
    Ok(std::borrow::Cow::Owned(scan))
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
