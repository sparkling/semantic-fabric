//! Resolve authored literal roles only after obtaining source decoder evidence.
use super::source_control::validation_error as source_error;
use super::*;
use crate::iq::{scan::LexicalMode, LexicalKey};
use sf_sql::source_work::{SourceVec, SourceWork};

#[cfg(test)]
pub(super) fn resolved(
    keys: &[LexicalKey],
    dialect: Dialect,
    source: &AliasActuals,
) -> Vec<LexicalKey> {
    resolved_controlled(keys, dialect, source, SourceWork::new(None))
        .expect("uncontrolled literal-role resolution")
}

fn pay_mode(mode: &LexicalMode, work: SourceWork<'_>) -> Result<()> {
    work.charge(std::mem::size_of::<LexicalMode>())
        .map_err(source_error)?;
    let bytes = match mode {
        LexicalMode::Iri { base: Some(base) } => base.len(),
        LexicalMode::TypedLiteral { datatype } => datatype.as_str().len(),
        _ => 0,
    };
    work.charge(bytes).map_err(source_error)
}

fn pay_names<'a>(
    names: impl Iterator<Item = &'a String>,
    name: &str,
    work: SourceWork<'_>,
) -> Result<()> {
    work.charge(name.len()).map_err(source_error)?;
    for column in names {
        work.charge(1).map_err(source_error)?;
        work.charge(column.len().min(name.len()))
            .map_err(source_error)?;
    }
    Ok(())
}

pub(super) fn resolved_controlled(
    keys: &[LexicalKey],
    dialect: Dialect,
    source: &AliasActuals,
    work: SourceWork<'_>,
) -> Result<Vec<LexicalKey>> {
    work.checkpoint().map_err(source_error)?;
    let mut groups: std::collections::BTreeMap<Box<str>, std::collections::BTreeSet<LexicalMode>> =
        Default::default();
    for key in keys {
        work.charge(1).map_err(source_error)?;
        // Prepay resolve_col's exact and ASCII-folded scans, including comparison bytes.
        for column in &source.columns {
            work.charge(2).map_err(source_error)?;
            work.product(2, column.len().min(key.column.len()))
                .map_err(source_error)?;
        }
        let name = resolve_col(&key.column, Some(&source.columns));
        pay_mode(&key.mode, work)?;
        let mode = match &key.mode {
            LexicalMode::TypedLiteral { datatype } => {
                let code = if dialect == Dialect::Sqlite {
                    pay_names(source.sqlite_columns.keys(), name, work)?;
                    source
                        .sqlite_columns
                        .get(name)
                        .and_then(|decode| decode.declared)
                } else {
                    pay_names(source.natural_columns.keys(), name, work)?;
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
        for column in groups.keys() {
            work.charge(1).map_err(source_error)?;
            work.charge(column.len().min(key.column.len()))
                .map_err(source_error)?;
        }
        work.charge(std::mem::size_of::<(
            Box<str>,
            std::collections::BTreeSet<LexicalMode>,
        )>())
        .map_err(source_error)?;
        work.charge(key.column.len()).map_err(source_error)?;
        let column = work
            .string(&key.column)
            .map_err(source_error)?
            .into_boxed_str();
        let group = groups.entry(column).or_default();
        for existing in group.iter() {
            pay_mode(existing, work)?;
            pay_mode(&mode, work)?;
        }
        work.product(2, std::mem::size_of::<LexicalMode>())
            .map_err(source_error)?;
        if mode == LexicalMode::DecodedWithNatural {
            group.extend([LexicalMode::Decoded, LexicalMode::Natural]);
        } else {
            group.insert(mode);
        }
    }
    let mut output = SourceVec::default();
    for (column, mut modes) in groups {
        work.charge(1).map_err(source_error)?;
        work.product(6, modes.len()).map_err(source_error)?;
        work.charge(std::mem::size_of::<LexicalMode>())
            .map_err(source_error)?;
        if modes.contains(&LexicalMode::Decoded) && modes.contains(&LexicalMode::Natural) {
            modes.remove(&LexicalMode::Decoded);
            modes.remove(&LexicalMode::Natural);
            modes.insert(LexicalMode::DecodedWithNatural);
        }
        for mode in modes {
            work.charge(column.len()).map_err(source_error)?;
            let name = work.string(&column).map_err(source_error)?.into_boxed_str();
            output
                .push(LexicalKey { column: name, mode }, work)
                .map_err(source_error)?;
        }
    }
    work.checkpoint().map_err(source_error)?;
    Ok(output.into_vec())
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

pub(super) fn binding_modes(
    bindings: &BindingView<'_>,
    dialect: Dialect,
    actuals: &ActualColumns,
    work: SourceWork<'_>,
) -> Result<Vec<(usize, Vec<LexicalKey>)>> {
    work.checkpoint().map_err(source_error)?;
    let mut output = SourceVec::default();
    for (alias, source) in actuals {
        work.charge(1).map_err(source_error)?;
        let keys = crate::cascade::distinct_scan::binding_lexical_keys_controlled(
            bindings.iter().map(|(_, def)| def),
            *alias,
            work,
        )?;
        let keys = resolved_controlled(&keys, dialect, source, work)?;
        output.push((*alias, keys), work).map_err(source_error)?;
    }
    work.checkpoint().map_err(source_error)?;
    Ok(output.into_vec())
}

pub(super) fn modes_for_alias<'a>(
    modes: &'a [(usize, Vec<LexicalKey>)],
    alias: usize,
    work: SourceWork<'_>,
) -> Result<Option<&'a [LexicalKey]>> {
    let mut found = None;
    for (candidate, keys) in modes {
        work.charge(1).map_err(source_error)?;
        if *candidate == alias {
            found = Some(keys.as_slice());
        }
    }
    work.checkpoint().map_err(source_error)?;
    Ok(found)
}

/// Canonical output DISTINCT uses a separate key and returns original fields.
pub(super) fn sqlite_distinct(
    bindings: &BindingView<'_>,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    projection: &[ColRef],
    work: sf_sql::source_work::SourceWork<'_>,
) -> Result<Option<Vec<String>>> {
    let modes = binding_modes(bindings, Dialect::Sqlite, actuals, work)?;
    let mut has_literal = false;
    for (_, keys) in &modes {
        work.charge(1).map_err(source_error)?;
        for key in keys {
            work.charge(1).map_err(source_error)?;
            has_literal |= matches!(
                key.mode,
                LexicalMode::Natural
                    | LexicalMode::DecodedWithNatural
                    | LexicalMode::TypedLiteral { .. }
            );
        }
    }
    if !has_literal {
        return Ok(None);
    }
    let mut changed = false;
    let mut keys = SourceVec::default();
    for column in projection {
        let key = modes_for_alias(&modes, column.alias, work)?
            .and_then(|keys| sqlite_key(column, keys, catalog, actuals));
        changed |= key.is_some();
        for key in key.unwrap_or_else(|| {
            vec![path_comparison::rdf_column(
                column,
                Dialect::Sqlite,
                catalog,
                actuals,
            )]
        }) {
            keys.push(key, work).map_err(source_error)?;
        }
    }
    Ok(changed.then_some(keys.into_vec()))
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

#[cfg(test)]
mod resolved_controlled_tests {
    use super::*;
    use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};

    fn actuals() -> AliasActuals {
        AliasActuals {
            datatype_columns: HashMap::new(),
            natural_columns: HashMap::new(),
            scalar_columns: HashMap::new(),
            source_kind: AliasSourceKind::Table,
            columns: Vec::new(),
            path: false,
            text_columns: HashMap::new(),
            static_iri_columns: HashSet::new(),
            iri_unreserved_columns: HashSet::new(),
            sqlite_columns: HashMap::new(),
            lexical_columns: HashMap::new(),
            lexical_comparison_columns: HashMap::new(),
        }
    }

    fn key(column: &str) -> LexicalKey {
        LexicalKey {
            column: column.into(),
            mode: LexicalMode::Decoded,
        }
    }

    fn charged(keys: &[LexicalKey], budget: u64) -> Result<u64> {
        let control = QueryBudget::new(QueryLimits::new(u64::MAX, budget, u64::MAX, u64::MAX));
        resolved_controlled(
            keys,
            Dialect::Postgres,
            &actuals(),
            SourceWork::new(Some(&control)),
        )?;
        Ok(control.consumed(QueryCharge::SourceWork))
    }

    /// `ref_atom::sql` and `binding_modes` both call `resolved_controlled` with
    /// the caller's real `SourceWork`; a regression to the raw, uncontrolled
    /// `resolved` twin at either call site would charge nothing regardless of
    /// how many keys are resolved. Exact/N-1 bisection (in the style of
    /// `stream/sqlite_metadata.rs`'s `controlled_recovery_...` test) proves
    /// every charged unit for one key is required, and the strict inequality
    /// against three keys proves the charge is genuinely per-key work, not a
    /// fixed baseline that would pass unchanged if the real work were dropped.
    #[test]
    fn charge_is_exact_per_call_and_strictly_scales_with_key_count() {
        let one = vec![key("a")];
        let three = vec![key("a"), key("b"), key("c")];

        let total_one = charged(&one, u64::MAX).expect("unbounded budget succeeds");
        assert!(
            total_one > 0,
            "resolving even one key must charge SourceWork"
        );
        assert_eq!(
            charged(&one, total_one).expect("exact total succeeds"),
            total_one
        );
        assert!(matches!(
            charged(&one, total_one - 1),
            Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
        ));

        let total_three = charged(&three, u64::MAX).expect("unbounded budget succeeds");
        assert!(
            total_three > total_one,
            "three keys ({total_three}) must charge strictly more than one ({total_one})"
        );
        assert_eq!(
            charged(&three, total_three).expect("exact total succeeds"),
            total_three
        );
        assert!(matches!(
            charged(&three, total_three - 1),
            Err(Error::QueryControl(QueryControlError::SourceWorkExceeded))
        ));
    }
}
