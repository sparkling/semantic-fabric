//! Resolve authored literal roles only after obtaining source decoder evidence.
use super::*;
use crate::iq::{scan::LexicalMode, LexicalKey};

pub(super) fn resolved(
    keys: &[LexicalKey],
    dialect: Dialect,
    source: &AliasActuals,
) -> Vec<LexicalKey> {
    let mut groups: std::collections::BTreeMap<Box<str>, std::collections::BTreeSet<LexicalMode>> =
        Default::default();
    for key in keys {
        let name = resolve_col(&key.column, Some(&source.columns));
        let mode = match &key.mode {
            LexicalMode::TypedLiteral { datatype } => {
                let code = if dialect == Dialect::Sqlite {
                    source
                        .sqlite_columns
                        .get(name)
                        .and_then(|decode| decode.declared)
                } else {
                    source.natural_columns.get(name).copied().flatten()
                };
                match code {
                    Some(code) if datatype.as_ref() == code.iri() => LexicalMode::Natural,
                    Some(_) => LexicalMode::Decoded,
                    None => key.mode.clone(),
                }
            }
            mode => mode.clone(),
        };
        let group = groups.entry(key.column.clone()).or_default();
        if mode == LexicalMode::DecodedWithNatural {
            group.extend([LexicalMode::Decoded, LexicalMode::Natural]);
        } else {
            group.insert(mode);
        }
    }
    groups
        .into_iter()
        .flat_map(|(column, mut modes)| {
            if modes.contains(&LexicalMode::Decoded) && modes.contains(&LexicalMode::Natural) {
                modes.remove(&LexicalMode::Decoded);
                modes.remove(&LexicalMode::Natural);
                modes.insert(LexicalMode::DecodedWithNatural);
            }
            modes.into_iter().map(move |mode| LexicalKey {
                column: column.clone(),
                mode,
            })
        })
        .collect()
}

pub(super) fn sqlite_key(
    column: &ColRef,
    keys: &[LexicalKey],
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
) -> Option<Vec<String>> {
    let decode = lexical_key::column_decode(column, actuals)?;
    let modes: Vec<_> = keys
        .iter()
        .filter(|k| k.column == column.column)
        .map(|k| &k.mode)
        .collect();
    let raw = colref(column, Dialect::Sqlite, actuals);
    let decoded = modes
        .iter()
        .any(|m| matches!(m, LexicalMode::Decoded | LexicalMode::DecodedWithNatural));
    let natural = modes
        .iter()
        .any(|m| matches!(m, LexicalMode::Natural | LexicalMode::DecodedWithNatural));
    let mut expressions = Vec::new();
    if decoded {
        expressions.push(lexical_key::expression(raw.clone(), decode, catalog));
    }
    if natural {
        expressions.push(lexical_key::with_mode(raw.clone(), decode, true, catalog));
        expressions.push(lexical_key::natural_datatype(&raw, decode));
    }
    for mode in modes {
        if let LexicalMode::TypedLiteral { datatype } = mode {
            expressions.push(lexical_key::typed(
                raw.clone(),
                decode,
                datatype.as_str(),
                catalog,
            ));
        }
    }
    (!expressions.is_empty()).then_some(expressions)
}

/// Canonical output DISTINCT uses a separate key and returns original fields.
pub(super) fn sqlite_distinct(
    b: &Branch,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
) -> Option<Vec<String>> {
    let modes: HashMap<_, _> = actuals
        .iter()
        .map(|(alias, source)| {
            (
                *alias,
                resolved(
                    &crate::cascade::distinct_scan::binding_lexical_keys(b, *alias),
                    Dialect::Sqlite,
                    source,
                ),
            )
        })
        .collect();
    if !modes.values().flatten().any(|key| {
        matches!(
            key.mode,
            LexicalMode::Natural
                | LexicalMode::DecodedWithNatural
                | LexicalMode::TypedLiteral { .. }
        )
    }) {
        return None;
    }
    let mut changed = false;
    let keys = b
        .projection()
        .iter()
        .flat_map(|column| {
            let key = modes
                .get(&column.alias)
                .and_then(|keys| sqlite_key(column, keys, catalog, actuals));
            changed |= key.is_some();
            key.unwrap_or_else(|| {
                vec![path_comparison::rdf_column(
                    column,
                    Dialect::Sqlite,
                    catalog,
                    actuals,
                )]
            })
        })
        .collect();
    changed.then_some(keys)
}

pub(super) fn window(select: &str, keys: &[String], width: usize) -> (String, String) {
    let columns = (0..width)
        .map(|i| format!("__sf_literal_raw.c{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    (
        format!(
            "{select}, ROW_NUMBER() OVER (PARTITION BY {}) AS __sf_literal_rank",
            keys.join(", ")
        ),
        columns,
    )
}

/// SQLite may retain any raw representative of an exact RDF group. Other
/// dialects must qualify their own GROUP BY/representative rules separately.
pub(super) fn sqlite_group_keys(
    b: &Branch,
    agg: &Aggregation,
    column: &ColRef,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
) -> Option<Vec<String>> {
    let mut keys = Vec::new();
    for group in &agg.keys {
        if !group.cols.contains(column) {
            continue;
        }
        let mode = match b.bindings.get(&group.var)? {
            // Retain the admitted injective template's decoded component
            // alongside any typed literal role on the same physical cell.
            TermDef::Derived {
                term_map: TermMap::Template(..),
                ..
            } => LexicalMode::Decoded,
            TermDef::Derived {
                term_map: TermMap::Column(_, spec),
                ..
            } if spec.term_type == sf_core::ir::TermType::Literal => {
                if spec.language.is_some() {
                    LexicalMode::Decoded
                } else if let Some(datatype) = &spec.datatype {
                    LexicalMode::TypedLiteral {
                        datatype: datatype.clone(),
                    }
                } else {
                    LexicalMode::Natural
                }
            }
            _ => return None,
        };
        keys.push(LexicalKey {
            column: column.column.clone(),
            mode,
        });
    }
    sqlite_key(column, &keys, catalog, actuals)
}
