//! Native Ref witnesses are joined/authorized before decoded RDF tuple dedup.
use super::*;
use crate::iq::scan::LexicalMode;

pub(super) fn validate_shape_controlled(
    input: &Branch,
    columns: &[ColRef],
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<()> {
    use source_control::validation_error as error;
    let invalid = || Error::Unsupported("invalid reference atom relation".into());
    work.charge(1).map_err(error)?;
    let [left, right] = input.core.as_slice() else {
        return Err(invalid());
    };
    let mut joined = false;
    for condition in &input.where_conds {
        work.charge(1).map_err(error)?;
        if matches!(condition, SqlCond::NativeColEq(a, b)
            if left.alias != right.alias
                && ((a.alias == left.alias && b.alias == right.alias)
                    || (a.alias == right.alias && b.alias == left.alias)))
        {
            joined = true;
            break;
        }
    }
    if !joined
        || left.source.logical().is_none()
        || right.source.logical().is_none()
        || !input.opts.is_empty()
        || !input.subplan_joins.is_empty()
        || input.path.is_some()
        || input.agg.is_some()
        || input.distinct
        || input.limit.is_some()
        || input.offset != 0
        || !input.order.is_empty()
        || input.nps
    {
        return Err(invalid());
    }
    // Preserve the existing injectivity gate, including its top-level-only law.
    for definition in input.bindings.values() {
        work.charge(1).map_err(error)?;
        let supported = match definition {
            TermDef::Derived { term_map, .. } => {
                matches!(term_map, TermMap::Column(_, spec)
                    if spec.term_type == sf_core::ir::TermType::Iri)
                    || injective_map(term_map, work)?
            }
            TermDef::R2rmlBlank {
                term_map, graph, ..
            } => {
                injective_map(term_map, work)?
                    && match graph {
                        crate::iq::R2rmlGraphScope::Default => true,
                        crate::iq::R2rmlGraphScope::Mapped { term_map, .. } => {
                            injective_map(term_map, work)?
                        }
                    }
            }
            _ => true,
        };
        if !supported {
            return Err(invalid());
        }
    }
    // The accepted prefix is itself the dedup index: borrow it rather than
    // recursively cloning every binding's columns and building another vector.
    let mut next = 0;
    for definition in input.bindings.values() {
        source_control::validate_definition_columns(definition, work, |alias, name| {
            for existing in &columns[..next] {
                work.charge(1).map_err(error)?;
                if alias == existing.alias {
                    work.charge(name.len().min(existing.column.len()))
                        .map_err(error)?;
                    if name == existing.column.as_ref() {
                        return Ok(());
                    }
                }
            }
            work.charge(1).map_err(error)?;
            let Some(expected) = columns.get(next) else {
                return Err(Error::Sql("reference atom output recipe mismatch".into()));
            };
            work.charge(name.len().min(expected.column.len()))
                .map_err(error)?;
            if alias != expected.alias || name != expected.column.as_ref() {
                return Err(Error::Sql("reference atom output recipe mismatch".into()));
            }
            next += 1;
            Ok(())
        })?;
    }
    if next != columns.len() {
        return Err(Error::Sql("reference atom output recipe mismatch".into()));
    }
    work.checkpoint().map_err(error)
}

pub(super) fn injective_map(
    term: &TermMap,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<bool> {
    use sf_core::ir::{Segment, TermType};
    use source_control::validation_error as error;
    work.charge(1).map_err(error)?;
    match term {
        TermMap::Column(_, spec) => Ok(spec.term_type != TermType::Iri || spec.base.is_none()),
        TermMap::Constant(_) => Ok(true),
        TermMap::Template(template, spec) => {
            if spec.term_type == TermType::Iri && spec.base.is_some() {
                return Ok(false);
            }
            let mut previous = false;
            let mut count = 0_usize;
            for segment in template.segments() {
                work.charge(1).map_err(error)?;
                match segment {
                    Segment::Column(_) => {
                        if (spec.term_type == TermType::Iri && previous)
                            || (spec.term_type != TermType::Iri && count != 0)
                        {
                            return Ok(false);
                        }
                        count += 1;
                        previous = true;
                    }
                    Segment::Literal(text) if spec.term_type == TermType::Iri => {
                        for byte in text.bytes() {
                            work.charge(1).map_err(error)?;
                            if byte.is_ascii()
                                && !byte.is_ascii_alphanumeric()
                                && !matches!(byte, b'-' | b'.' | b'_' | b'~' | b'%')
                            {
                                previous = false;
                                break;
                            }
                        }
                    }
                    Segment::Literal(_) => {}
                }
            }
            Ok(true)
        }
    }
}

#[cfg(test)]
mod controlled_shape_tests {
    use super::*;
    use sf_core::ir::{Template, TermSpec};

    #[test]
    fn malformed_recipes_and_relation_shapes_keep_raw_errors() {
        let mut branch = Branch::empty();
        branch.core = (0..2)
            .map(|alias| crate::iq::Scan {
                alias,
                source: LogicalSource::Table("table".into()).into(),
            })
            .collect();
        let column = ColRef::new(0, Box::<str>::from("key"));
        branch.where_conds.push(SqlCond::NativeColEq(
            column.clone(),
            ColRef::new(1, Box::<str>::from("key")),
        ));
        branch.bindings.insert(
            "x".into(),
            TermDef::Derived {
                alias: 0,
                term_map: TermMap::Column("key".into(), TermSpec::iri()),
            },
        );
        for malformed_relation in [false, true] {
            branch.distinct = malformed_relation;
            for recipe in [
                vec![],
                vec![column.clone(), column.clone()],
                vec![ColRef::new(1, Box::<str>::from("wrong"))],
            ] {
                let raw = crate::iq::scan::ref_atom::validate_shape(&branch, &recipe);
                let controlled = validate_shape_controlled(
                    &branch,
                    &recipe,
                    sf_sql::source_work::SourceWork::new(None),
                );
                assert_eq!(
                    raw.unwrap_err().to_string(),
                    controlled.unwrap_err().to_string()
                );
            }
        }
    }

    #[test]
    fn injectivity_matches_existing_gate_for_template_separators_and_bases() {
        for template in [
            "constant",
            "{a}",
            "{a}{b}",
            "{a}/{b}",
            "{a}-{b}",
            "{a}é{b}",
            "{a}%2F{b}",
            "{a}/{b}/{c}",
        ] {
            for spec in [
                TermSpec::iri(),
                TermSpec::iri().with_base("https://base/"),
                TermSpec::blank_node(),
                TermSpec::plain_literal(),
            ] {
                let term_map = TermMap::Template(Template::parse(template).unwrap(), spec);
                let expected = crate::cascade::binding_is_injective(&TermDef::Derived {
                    alias: 0,
                    term_map: term_map.clone(),
                });
                assert_eq!(
                    injective_map(&term_map, sf_sql::source_work::SourceWork::new(None)).unwrap(),
                    expected,
                    "{template}: {term_map:?}"
                );
            }
        }
    }
}

#[cfg(test)]
pub(super) fn actuals(
    input: &Branch,
    columns: &[ColRef],
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> AliasActuals {
    from_sources(columns, &branch_actuals(input, dialect, catalog))
}

/// Positional metadata from a completed child branch, without recursive descent.
#[cfg(test)]
pub(super) fn from_sources(columns: &[ColRef], sources: &ActualColumns) -> AliasActuals {
    AliasActuals {
        datatype_columns: columns
            .iter()
            .enumerate()
            .filter_map(|(i, column)| {
                literal_datatype::fact(column, &sources).map(|code| (format!("c{i}"), code))
            })
            .collect(),
        natural_columns: columns
            .iter()
            .enumerate()
            .filter_map(|(i, column)| {
                natural_literal::column_fact(column, &sources).map(|code| (format!("c{i}"), code))
            })
            .collect(),
        scalar_columns: columns
            .iter()
            .enumerate()
            .filter_map(|(i, column)| {
                iri_cmp::scalar_column(column, &sources)
                    .filter(|key| {
                        !matches!(
                            key,
                            NativeScalarKey::MysqlDate | NativeScalarKey::MysqlDateTime
                        )
                    })
                    .map(|key| (format!("c{i}"), key))
            })
            .collect(),
        sqlite_columns: columns
            .iter()
            .enumerate()
            .filter_map(|(i, column)| {
                lexical_key::column_decode(column, &sources).map(|decode| (format!("c{i}"), decode))
            })
            .collect(),
        lexical_columns: HashMap::new(),
        lexical_comparison_columns: HashMap::new(),
        source_kind: AliasSourceKind::Derived,
        columns: (0..columns.len()).map(|i| format!("c{i}")).collect(),
        path: false,
        iri_unreserved_columns: columns
            .iter()
            .enumerate()
            .filter(|(_, c)| iri_cmp::unreserved_column(c, &sources))
            .map(|(i, _)| format!("c{i}"))
            .collect(),
        static_iri_columns: columns
            .iter()
            .enumerate()
            .filter(|(_, column)| iri_cmp::static_iri_column(column, &sources))
            .map(|(i, _)| format!("c{i}"))
            .collect(),
        text_columns: columns
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                path_comparison::column_text(c, &sources).map(|key| (format!("c{i}"), key))
            })
            .collect(),
    }
}

#[cfg(test)]
pub(super) fn validate_output(width: usize, name: &str) -> Result<()> {
    validate_output_controlled(width, name, sf_sql::source_work::SourceWork::new(None))
}

pub(super) fn validate_output_controlled(
    width: usize,
    name: &str,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<()> {
    let invalid = || Error::Sql("missing reference atom output".into());
    work.charge(1).map_err(source_control::validation_error)?;
    let digits = name.strip_prefix('c').ok_or_else(invalid)?;
    if digits.is_empty() || (digits.len() > 1 && digits.starts_with('0')) {
        return Err(invalid());
    }
    let mut index = 0_usize;
    for digit in digits.bytes() {
        work.charge(1).map_err(source_control::validation_error)?;
        if !digit.is_ascii_digit() {
            return Err(invalid());
        }
        index = index
            .checked_mul(10)
            .and_then(|value| value.checked_add(usize::from(digit - b'0')))
            .ok_or_else(invalid)?;
    }
    work.checkpoint()
        .map_err(source_control::validation_error)?;
    if index < width {
        Ok(())
    } else {
        Err(invalid())
    }
}

pub(super) fn sql(
    input: &Branch,
    columns: &[ColRef],
    dialect: Dialect,
    catalog: &ColumnCatalog,
    params: &mut Vec<String>,
    pidx: &mut usize,
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<String> {
    validate_shape_controlled(input, columns, work)?;
    let actuals = branch_actuals_controlled(input, dialect, catalog, work)?;
    let mut modes: HashMap<usize, Vec<crate::iq::LexicalKey>> = HashMap::new();
    for scan in &input.core {
        let resolved = literal_roles::resolved_controlled(
            &crate::cascade::distinct_scan::lexical_keys(input, scan.alias),
            dialect,
            &actuals[&scan.alias],
            work,
        )?;
        modes.insert(scan.alias, resolved);
    }
    fn column_modes<'a>(
        column: &'a ColRef,
        modes: &'a HashMap<usize, Vec<crate::iq::LexicalKey>>,
    ) -> impl Iterator<Item = &'a crate::iq::LexicalKey> {
        modes
            .get(&column.alias)
            .into_iter()
            .flatten()
            .filter(move |key| key.column == column.column)
    }
    // SQL numeric equality is not decoded RDF identity (notably +0/-0).
    // Only the live-proven text families authorize this new atom-level key.
    // Unknown families retain the previous per-source D1/native-join behavior;
    // neither a raw atom DISTINCT nor an undecuplicated join is that fallback.
    let window = columns.iter().all(|c| {
        if column_modes(c, &modes).any(|key| matches!(key.mode, LexicalMode::Iri { .. })) {
            dialect == Dialect::Sqlite && lexical_key::column_decode(c, &actuals).is_some()
        } else {
            path_comparison::column_text(c, &actuals).is_some()
                || (dialect == Dialect::Sqlite
                    && modes
                        .get(&c.alias)
                        .and_then(|keys| literal_roles::sqlite_key(c, keys, catalog, &actuals))
                        .is_some())
        }
    });
    let legacy = if window {
        None
    } else {
        metadata_source::branch_copy_controlled(input, work)?;
        let mut legacy = input.clone();
        crate::cascade::force_distinct_for_dup_safety(
            std::slice::from_mut(&mut legacy),
            &[],
            dialect,
        );
        Some(legacy)
    };
    let input = legacy.as_ref().unwrap_or(input);
    let actuals = if legacy.is_some() {
        branch_actuals_controlled(input, dialect, catalog, work)?
    } else {
        actuals
    };
    // PARTITION BY is in SELECT before FROM/WHERE; bind its bases first.
    let mut keys = Vec::new();
    if window {
        for column in columns {
            let mut resolved = false;
            for key in column_modes(column, &modes) {
                if let LexicalMode::Iri { base } = &key.mode {
                    keys.push(iri_cmp::column(
                        column,
                        base.as_deref(),
                        dialect,
                        catalog,
                        &actuals,
                        params,
                        pidx,
                        work,
                    )?);
                    resolved = true;
                }
            }
            if !resolved || column_modes(column, &modes).any(|key| key.mode == LexicalMode::Decoded)
            {
                keys.extend(
                    if let Some(key) = (dialect == Dialect::Sqlite)
                        .then(|| {
                            modes.get(&column.alias).and_then(|keys| {
                                literal_roles::sqlite_key(column, keys, catalog, &actuals)
                            })
                        })
                        .flatten()
                    {
                        key
                    } else {
                        vec![path_comparison::rdf_column(
                            column, dialect, catalog, &actuals,
                        )]
                    },
                );
            }
        }
    }
    let from = render_from_controlled(input, dialect, catalog, &actuals, params, pidx, work)?;
    let filter = render_where(
        &input.where_conds,
        dialect,
        catalog,
        &actuals,
        params,
        pidx,
        work,
    )?
    .map(|sql| format!(" WHERE {sql}"))
    .unwrap_or_default();
    let mut items = columns
        .iter()
        .enumerate()
        .map(|(i, c)| format!("{} AS c{i}", colref(c, dialect, &actuals)))
        .collect::<Vec<_>>();
    if !window {
        return Ok(format!("SELECT {} FROM {from}{filter}", items.join(", ")));
    }
    let keys = if keys.is_empty() {
        "1".to_owned()
    } else {
        keys.join(", ")
    };
    items.push(format!(
        "ROW_NUMBER() OVER (PARTITION BY {keys}) AS __sf_rank"
    ));
    let projected = if columns.is_empty() {
        "1 AS c0".to_owned()
    } else {
        (0..columns.len())
            .map(|i| format!("__sf_atom.c{i}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    Ok(format!("SELECT {projected} FROM (SELECT {} FROM {from}{filter}) __sf_atom WHERE __sf_atom.__sf_rank = 1", items.join(", ")))
}
