//! Rendering of typed scan relations. No generated SQL is a live source authority.
use super::*;
use crate::iq::scan::LexicalMode;
use crate::iq::{CmpOp, Scan, ScanSource};
#[path = "scan_template.rs"]
pub(super) mod template;

pub(super) fn logical_aliases<'a>(
    branch: &'a Branch,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<Vec<(usize, &'a LogicalSource)>> {
    use sf_sql::source_work::SourceVec;
    use source_control::validation_error as error;
    let mut output = SourceVec::default();
    for scan in branch
        .core
        .iter()
        .chain(branch.opts.iter().map(|join| &join.scan))
    {
        work.charge(1).map_err(error)?;
        if let Some(source) = scan.source.logical() {
            output.push((scan.alias, source), work).map_err(error)?;
        }
    }
    let mut pending = SourceVec::default();
    pending
        .push(branch.where_conds.as_slice(), work)
        .map_err(error)?;
    while !pending.as_slice().is_empty() {
        work.charge(1).map_err(error)?;
        let conditions = pending.pop().expect("nonempty metadata traversal");
        let Some((condition, rest)) = conditions.split_first() else {
            continue;
        };
        work.charge(1).map_err(error)?;
        if !rest.is_empty() {
            pending.push(rest, work).map_err(error)?;
        }
        match condition {
            SqlCond::Exists { scans, conds } | SqlCond::NotExists { scans, conds } => {
                for scan in scans {
                    work.charge(1).map_err(error)?;
                    if let Some(source) = scan.source.logical() {
                        output.push((scan.alias, source), work).map_err(error)?;
                    }
                }
                pending.push(conds.as_slice(), work).map_err(error)?;
            }
            SqlCond::Not(inner) => pending
                .push(std::slice::from_ref(inner.as_ref()), work)
                .map_err(error)?,
            SqlCond::And(conditions) | SqlCond::Or(conditions) => {
                pending.push(conditions.as_slice(), work).map_err(error)?;
            }
            _ => {}
        }
    }
    work.checkpoint().map_err(error)?;
    Ok(output.into_vec())
}

#[cfg(test)]
pub(super) fn scan_actuals(scan: &Scan, dialect: Dialect, catalog: &ColumnCatalog) -> AliasActuals {
    super::metadata::scan_actuals_controlled(
        scan,
        dialect,
        catalog,
        sf_sql::source_work::SourceWork::new(None),
    )
    .expect("uncontrolled scan metadata")
}

fn projection_sql(
    source: &ScanSource,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    let ScanSource::Projection {
        input,
        columns,
        guards,
        distinct,
        native_keys,
        lexical_keys,
    } = source
    else {
        unreachable!("projection renderer")
    };
    let distinct = *distinct;
    let actuals = HashMap::from([(
        input.alias,
        super::metadata::scan_actuals_controlled(input, dialect, catalog, work)?,
    )]);
    let resolved_keys =
        literal_roles::resolved_controlled(lexical_keys, dialect, &actuals[&input.alias], work)?;
    let lexical_keys = resolved_keys.as_slice();
    let inner = &actuals[&input.alias];
    let lexicalized =
        metadata_projection::mysql_lexical_columns(source, dialect, inner, lexical_keys, work)?;
    let column = |name: &str| {
        // Raw/offline APIs retain their existing authored-AS fallback. Live
        // execution always probes the original source and takes the typed path.
        if let Some(source) = input.source.logical() {
            if catalog.columns(source).is_none() {
                let sql = match source {
                    LogicalSource::Query(sql) => Some(sql.as_str()),
                    _ => None,
                };
                return render_immediate_source_column(
                    &format!("t{}", input.alias),
                    name,
                    sql,
                    dialect,
                );
            }
        }
        colref(&ColRef::new(input.alias, name), dialect, &actuals)
    };
    let mut items = Vec::with_capacity(columns.len());
    let mut expressions = Vec::with_capacity(columns.len());
    for (name, term) in columns {
        let expression = match term {
            TermMap::Column(raw, _) => match lexicalized.get(name.as_ref()) {
                Some(key) => path_comparison::exact_text(
                    iri_cmp::scalar_lexical(*key, &column(raw), dialect)?,
                    dialect,
                ),
                None => column(raw),
            },
            TermMap::Template(recipe, spec) => template::render(
                recipe,
                spec,
                input.alias,
                distinct,
                dialect,
                catalog,
                &actuals,
                work,
            )?,
            TermMap::Constant(_) => {
                return Err(Error::Unsupported("constant projection recipe".into()))
            }
        };
        items.push(format!("{expression} AS {}", dialect.quote_ident(name)));
        expressions.push(expression);
    }
    if items.is_empty() {
        items.push("1 AS __sf_dummy".into());
    }
    // Original lexical consumer proof plus a live decoder makes mixed SQLite
    // keys exact. Natural/base-IRI consumers cannot borrow that authority.
    let lexical = |raw: &str| {
        (dialect == Dialect::Sqlite)
            .then(|| {
                literal_roles::sqlite_key(
                    &ColRef::new(input.alias, raw),
                    lexical_keys,
                    catalog,
                    &actuals,
                )
            })
            .flatten()
    };
    let numeric = |raw: &str| {
        pg_numeric::key(
            &ColRef::new(input.alias, raw),
            dialect,
            &actuals,
            lexical_keys,
        )
    };
    let has_numeric = columns
        .iter()
        .any(|(_, term)| matches!(term, TermMap::Column(raw, _) if numeric(raw).is_some()));
    let has_iris = lexical_keys
        .iter()
        .any(|key| matches!(key.mode, LexicalMode::Iri { .. }));
    if distinct && has_iris && (dialect != Dialect::Sqlite || !native_keys.is_empty()) {
        return Err(Error::Unsupported(
            "resolved column-IRI dedup requires SQLite RDF-only scan keys".into(),
        ));
    }
    let iri_column = |raw: &str| {
        lexical_keys
            .iter()
            .any(|key| key.column.as_ref() == raw && matches!(key.mode, LexicalMode::Iri { .. }))
    };
    let window = distinct
        && (native_keys.is_empty() || has_numeric)
        && !columns.is_empty()
        && ((dialect == Dialect::Sqlite && columns.iter().all(|(_, term)| template::supports_distinct(term))) || columns.iter().all(|(_, term)| {
            matches!(term, TermMap::Column(raw, _) if iri_column(raw) || lexical(raw).is_some() || numeric(raw).is_some() || (has_numeric && pg_numeric::native_companion(&ColRef::new(input.alias, raw.clone()), dialect, &actuals, lexical_keys)) || path_comparison::column_text(
                &ColRef::new(input.alias, raw.clone()), &actuals).is_some())
        }) || (has_numeric && columns.iter().all(|(name, term)| {
            native_keys.iter().any(|(key, both)| key == name && !both)
                || matches!(term, TermMap::Column(raw, _) if numeric(raw).is_some()
                    || pg_numeric::native_companion(&ColRef::new(input.alias, raw.clone()), dialect, &actuals, lexical_keys)
                    || path_comparison::column_text(&ColRef::new(input.alias, raw.clone()), &actuals).is_some())
        })) || (!has_numeric && !guards.iter().any(|guard| matches!(guard, SqlCond::NativeCmp(..)))
            && !actuals[&input.alias].text_columns.is_empty()));
    if distinct && has_iris && !window {
        return Err(Error::Unsupported(
            "resolved column-IRI dedup requires exact keys for every scan consumer".into(),
        ));
    }
    if distinct && has_numeric && !window {
        return Err(Error::Unsupported(
            "numeric RDF dedup requires known decoder roles for every key".into(),
        ));
    }
    let mut rank = "__sf_rank".to_owned();
    while columns
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case(&rank))
    {
        rank.push('_');
    }
    let mut native_seen = HashSet::new();
    for (name, _) in native_keys {
        if !distinct
            || !native_seen.insert(name)
            || !columns.iter().any(|(output, term)| {
                output == name && matches!(term, TermMap::Column(raw, _) if raw == name)
            })
        {
            return Err(Error::Sql("invalid native projection key".into()));
        }
    }
    if window {
        let mut keys = Vec::new();
        for ((name, term), expression) in columns.iter().zip(&expressions) {
            let TermMap::Column(raw, _) = term else {
                keys.push(template::distinct_key(term, expression, dialect)?);
                continue;
            };
            let native = native_keys.iter().find(|(key, _)| key == name);
            if lexicalized.contains_key(name.as_ref()) {
                keys.push(path_comparison::exact_text(expression.clone(), dialect));
                continue;
            }
            if iri_column(raw) {
                for key in lexical_keys
                    .iter()
                    .filter(|key| key.column.as_ref() == raw.as_ref())
                {
                    if let LexicalMode::Iri { base } = &key.mode {
                        keys.push(iri_cmp::column(
                            &ColRef::new(input.alias, raw.clone()),
                            base.as_deref(),
                            dialect,
                            catalog,
                            &actuals,
                            params,
                            pidx,
                            work,
                        )?);
                    }
                }
                if lexical(raw).is_none() {
                    continue;
                }
            }
            if native.is_none_or(|(_, both)| *both) {
                keys.extend(if let Some(key) = numeric(raw) {
                    vec![key]
                } else if let Some(key) = lexical(raw).filter(|_| {
                    path_comparison::column_text(&ColRef::new(input.alias, raw.clone()), &actuals)
                        .is_none()
                }) {
                    key
                } else {
                    vec![path_comparison::rdf_column(
                        &ColRef::new(input.alias, raw.clone()),
                        dialect,
                        catalog,
                        &actuals,
                    )]
                });
            }
            if native.is_some() {
                let key = column(raw);
                keys.push(
                    if !catalog.suppress_path_collation
                        && path_comparison::column_text(
                            &ColRef::new(input.alias, raw.clone()),
                            &actuals,
                        )
                        .is_some()
                    {
                        path_comparison::exact_text(key, dialect)
                    } else {
                        key
                    },
                );
            }
        }
        items.push(format!(
            "ROW_NUMBER() OVER (PARTITION BY {}) AS {}",
            keys.join(", "),
            dialect.quote_ident(&rank)
        ));
    }
    let mut sql = format!(
        "SELECT {}{} FROM {}",
        if distinct && !window { "DISTINCT " } else { "" },
        items.join(", "),
        scan_ref_controlled(input, dialect, catalog, params, pidx, work)?
    );
    let protected = natural_literal::authorized_conjunction_controlled(
        &guards.iter().collect::<Vec<_>>(),
        dialect,
        catalog,
        &actuals,
        params,
        pidx,
        work,
    )?;
    let predicates = match protected {
        Some(sql) => vec![sql],
        None => guards
            .iter()
            .map(|guard| match guard {
                SqlCond::IsNull(c) if c.alias == input.alias => {
                    Ok(format!("{} IS NULL", column(&c.column)))
                }
                SqlCond::IsNotNull(c) if c.alias == input.alias => {
                    Ok(format!("{} IS NOT NULL", column(&c.column)))
                }
                SqlCond::NativeCmp(c, CmpOp::Eq, _) if c.alias == input.alias => {
                    render_cond_controlled(guard, dialect, catalog, &actuals, params, pidx, work)
                }
                _ if crate::iq::iri_cmp::atom_guard(guard, input.alias) => {
                    render_cond_controlled(guard, dialect, catalog, &actuals, params, pidx, work)
                }
                _ => Err(Error::Unsupported("projection guard shape".into())),
            })
            .collect::<Result<Vec<_>>>()?,
    };
    if !predicates.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&predicates.join(" AND "));
    }
    if window {
        let selected = columns
            .iter()
            .map(|(name, _)| format!("__sf_identity.{}", dialect.quote_ident(name)))
            .collect::<Vec<_>>()
            .join(", ");
        sql = format!(
            "SELECT {selected} FROM ({sql}) __sf_identity WHERE __sf_identity.{} = 1",
            dialect.quote_ident(&rank)
        );
    }
    if dialect == Dialect::MySql
        && actuals[&input.alias].natural_columns.values().any(|code| {
            matches!(
                code,
                Some(
                    sf_core::datatype::XsdTypeCode::Date
                        | sf_core::datatype::XsdTypeCode::DateTime
                        | sf_core::datatype::XsdTypeCode::Integer
                        | sf_core::datatype::XsdTypeCode::Boolean
                )
            )
        })
        && guards
            .iter()
            .any(|guard| matches!(guard, SqlCond::NativeCmp(..)))
    {
        // MySQL pushes predicates on window partition keys below ROW_NUMBER.
        // Its no-limit sentinel is a structural merge/pushdown barrier: natural
        // validation outside this relation can only inspect authorized rows.
        sql.push_str(" LIMIT ");
        sql.push_str(dialect.bare_offset_limit_sentinel().unwrap());
    }
    if dialect == Dialect::Postgres
        && actuals[&input.alias]
            .scalar_columns
            .values()
            .any(|key| *key == NativeScalarKey::PostgresNumeric)
        && guards
            .iter()
            .any(|guard| matches!(guard, SqlCond::NativeCmp(..)))
    {
        // Block pull-up/predicate pushdown across admitted rows. Outer fallible
        // NUMERIC validation cannot run against rows hidden by portable policy.
        sql.push_str(" OFFSET 0");
    }
    Ok(sql)
}

pub(super) use super::projection_layout::validate_output_controlled;
#[cfg(test)]
pub(super) use super::scan_ref;
pub(super) fn scan_ref_controlled(
    scan: &Scan,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    work.charge(1).map_err(source_control::validation_error)?;
    let alias = scan.alias;
    match &scan.source {
        ScanSource::RefAtom { input, columns } => {
            let sql = ref_atom::sql(input, columns, dialect, catalog, params, pidx, work)?;
            Ok(format!("({sql}) t{alias}"))
        }
        ScanSource::Logical(LogicalSource::Table(table)) => {
            Ok(format!("{} t{alias}", dialect.quote_ident(table)))
        }
        ScanSource::Logical(LogicalSource::Query(query)) => Ok(format!("({query}) t{alias}")),
        ScanSource::Path { closure, cte_alias } => {
            let sql = path_as_derived_table_sql(closure, *cte_alias, dialect, catalog)?;
            Ok(format!("({sql}) t{alias}"))
        }
        ScanSource::Projection { .. } => {
            let sql = projection_sql(&scan.source, dialect, catalog, params, pidx, work)?;
            Ok(format!("({sql}) t{alias}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::ir::{Template, TermSpec};

    #[test]
    fn transparent_templates_preserve_native_descriptor_without_inventing_text() {
        for dialect in [Dialect::Postgres, Dialect::Sqlite, Dialect::MySql] {
            for key in [
                None,
                Some(TextKey::Verbatim),
                Some(TextKey::SqliteCharacter(4)),
                Some(TextKey::PostgresCharacter),
            ] {
                let source = LogicalSource::Table("items".into());
                let mut catalog = ColumnCatalog::default();
                catalog
                    .insert_live_result(
                        &source,
                        vec![sf_sql::backend::ResultColumn {
                            natural_datatype: None,
                            native_scalar: None,
                            sqlite_decode: None,
                            name: "key".into(),
                            text_key: key,
                        }],
                    )
                    .unwrap();
                for (template, expected) in [
                    (
                        "{KEY}",
                        if dialect == Dialect::MySql || key.is_some() {
                            Some(TextKey::Verbatim)
                        } else {
                            key
                        },
                    ),
                    ("prefix/{KEY}", Some(TextKey::Verbatim)),
                ] {
                    let scan = Scan {
                        alias: 1,
                        source: ScanSource::Projection {
                            input: Box::new(Scan {
                                alias: 0,
                                source: source.clone().into(),
                            }),
                            columns: vec![(
                                "value".into(),
                                TermMap::Template(
                                    Template::parse(template).unwrap(),
                                    TermSpec::plain_literal(),
                                ),
                            )],
                            guards: vec![],
                            distinct: false,
                            native_keys: vec![],
                            lexical_keys: vec![],
                        },
                    };
                    let actuals = scan_actuals(&scan, dialect, &catalog);
                    assert_eq!(
                        actuals.text_columns.get("value").copied(),
                        expected,
                        "{dialect:?} {key:?} {template}"
                    );
                }
            }
        }
    }
}
