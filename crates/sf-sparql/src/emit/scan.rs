//! Rendering of typed scan relations. No generated SQL is a live source authority.
use super::*;
use crate::iq::{CmpOp, Scan, ScanSource};

pub(super) fn scan_actuals(scan: &Scan, dialect: Dialect, catalog: &ColumnCatalog) -> AliasActuals {
    match &scan.source {
        ScanSource::RefAtom { input, columns } => {
            ref_atom::actuals(input, columns, dialect, catalog)
        }
        ScanSource::Logical(source) => source_actuals(source, catalog),
        ScanSource::Path { closure, .. } => path_actuals(closure, catalog),
        ScanSource::Projection { input, columns, .. } => {
            let inner = scan_actuals(input, dialect, catalog);
            AliasActuals {
                source_kind: AliasSourceKind::Derived,
                columns: columns.iter().map(|(name, _)| name.to_string()).collect(),
                path: false,
                text_columns: columns
                    .iter()
                    .filter_map(|(name, term)| {
                        let key = match term {
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

fn projection_sql(
    source: &ScanSource,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    let ScanSource::Projection {
        input,
        columns,
        guards,
        distinct,
        native_keys,
    } = source
    else {
        unreachable!("projection renderer")
    };
    let distinct = *distinct;
    let actuals = HashMap::from([(input.alias, scan_actuals(input, dialect, catalog))]);
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
    for (name, term) in columns {
        let expression = match term {
            TermMap::Column(name, _) => column(name),
            TermMap::Template(template, spec) => render_template_inline(
                template.segments(),
                spec.term_type == sf_core::ir::TermType::Iri,
                dialect,
                |name| {
                    path_comparison::rdf_column(
                        &ColRef::new(input.alias, name),
                        dialect,
                        catalog,
                        &actuals,
                    )
                },
            )?,
            TermMap::Constant(_) => {
                return Err(Error::Unsupported("constant projection recipe".into()))
            }
        };
        items.push(format!("{expression} AS {}", dialect.quote_ident(name)));
    }
    if items.is_empty() {
        items.push("1 AS __sf_dummy".into());
    }
    // Residual native consumers retain their raw relation. Policies inside this
    // projection already filtered raw rows; their newly enabled window needs
    // decoder proof for EVERY RDF key, never SQL numeric equality for +/-0.
    let policy_guarded = guards.iter().any(|g| matches!(g, SqlCond::NativeCmp(..)));
    let window = distinct
        && native_keys.is_empty()
        && !columns.is_empty()
        && if policy_guarded {
            columns.iter().all(|(_, term)| {
                matches!(term, TermMap::Column(raw, _) if path_comparison::column_text(
                    &ColRef::new(input.alias, raw.clone()), &actuals).is_some())
            })
        } else {
            !actuals[&input.alias].text_columns.is_empty()
        };
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
        for (name, term) in columns {
            let TermMap::Column(raw, _) = term else {
                return Err(Error::Unsupported("distinct computed scan recipe".into()));
            };
            let native = native_keys.iter().find(|(key, _)| key == name);
            if native.is_none_or(|(_, both)| *both) {
                keys.push(path_comparison::rdf_column(
                    &ColRef::new(input.alias, raw.clone()),
                    dialect,
                    catalog,
                    &actuals,
                ));
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
        scan_ref(input, dialect, catalog, params, pidx)?
    );
    let predicates = guards
        .iter()
        .map(|guard| match guard {
            SqlCond::IsNull(c) if c.alias == input.alias => {
                Ok(format!("{} IS NULL", column(&c.column)))
            }
            SqlCond::IsNotNull(c) if c.alias == input.alias => {
                Ok(format!("{} IS NOT NULL", column(&c.column)))
            }
            SqlCond::NativeCmp(c, CmpOp::Eq, _) if c.alias == input.alias => {
                render_cond(guard, dialect, catalog, &actuals, params, pidx)
            }
            _ => Err(Error::Unsupported("projection guard shape".into())),
        })
        .collect::<Result<Vec<_>>>()?;
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
    Ok(sql)
}

pub(super) fn validate_projection(
    input: &Scan,
    columns: &[(Box<str>, TermMap)],
    guards: &[SqlCond],
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> Result<()> {
    if let ScanSource::Projection {
        input,
        columns,
        guards,
        ..
    } = &input.source
    {
        validate_projection(input, columns, guards, dialect, catalog)?;
    }
    let mut outputs = HashSet::new();
    for (name, term) in columns {
        if !outputs.insert(name) {
            return Err(Error::Sql("duplicate projection output".into()));
        }
        let raw = match term {
            TermMap::Column(column, _) => vec![column.as_ref()],
            TermMap::Template(template, _) => template
                .segments()
                .iter()
                .filter_map(|segment| match segment {
                    Segment::Column(column) => Some(column.as_ref()),
                    _ => None,
                })
                .collect(),
            TermMap::Constant(_) => {
                return Err(Error::Unsupported("constant projection recipe".into()))
            }
        };
        for column in raw {
            validate_input_column(input, column, dialect, catalog)?;
        }
    }
    for guard in guards {
        match guard {
            SqlCond::IsNull(c) | SqlCond::IsNotNull(c) | SqlCond::NativeCmp(c, CmpOp::Eq, _)
                if c.alias == input.alias =>
            {
                validate_input_column(input, &c.column, dialect, catalog)?
            }
            _ => return Err(Error::Unsupported("projection guard shape".into())),
        }
    }
    Ok(())
}

fn validate_input_column(
    input: &Scan,
    name: &str,
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> Result<()> {
    match &input.source {
        ScanSource::RefAtom { input, columns } => {
            validate_live_columns(std::slice::from_ref(input), dialect, catalog)?;
            ref_atom::validate_output(columns.len(), name)
        }
        ScanSource::Logical(source) => catalog.validate_live_column(source, name, dialect),
        ScanSource::Projection { columns, .. } => validate_output(columns, name),
        ScanSource::Path { .. } => Err(Error::Unsupported("projection over path".into())),
    }
}

pub(super) fn validate_output(columns: &[(Box<str>, TermMap)], name: &str) -> Result<()> {
    if columns.iter().any(|(output, _)| output.as_ref() == name)
        || columns
            .iter()
            .filter(|(output, _)| output.eq_ignore_ascii_case(name))
            .count()
            == 1
    {
        Ok(())
    } else {
        Err(Error::Sql("missing or ambiguous projection output".into()))
    }
}

pub(super) fn scan_ref(
    scan: &Scan,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    let alias = scan.alias;
    match &scan.source {
        ScanSource::RefAtom { input, columns } => {
            let sql = ref_atom::sql(input, columns, dialect, catalog, params, pidx)?;
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
            let sql = projection_sql(&scan.source, dialect, catalog, params, pidx)?;
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
