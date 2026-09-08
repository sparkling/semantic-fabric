//! A native reference-object join is one RDF atom, not two independent RDF scans.
use super::*;
use crate::{Error, Result};

pub(crate) fn raw_origin<'a>(
    branch: &'a Branch,
    column: &'a ColRef,
) -> Option<(&'a LogicalSource, &'a str)> {
    branch
        .relation_scans()
        .into_iter()
        .find(|scan| scan.alias == column.alias)
        .and_then(|scan| scan.source.raw_column_origin(&column.column))
}

pub(crate) fn seal(input: Branch) -> Result<Branch> {
    // Join-less references retain their existing D1 admission/flag behavior.
    if !has_native_join(&input) {
        return Ok(input);
    }
    // Retain the existing non-injective-term admission path. Raw tuple keys do
    // not prove equivalence for non-injective constructions.
    if !input
        .bindings
        .values()
        .all(crate::cascade::binding_is_injective)
    {
        return Ok(input);
    }
    let alias = input.core[0].alias;
    let mut columns = Vec::new();
    for def in input.bindings.values() {
        for column in def.columns() {
            if !columns.contains(&column) {
                columns.push(column);
            }
        }
    }
    let bindings = input
        .bindings
        .iter()
        .map(|(var, def)| {
            Ok((
                var.clone(),
                crate::iq::lower::remap_termdef(def, &columns, alias)?,
            ))
        })
        .collect::<Result<_>>()?;
    validate_shape(&input, &columns)?;
    let mut result = Branch::single(Scan {
        alias,
        source: ScanSource::RefAtom {
            input: Box::new(input),
            columns,
        },
    });
    result.bindings = bindings;
    Ok(result)
}

pub(crate) fn validate_shape(input: &Branch, columns: &[ColRef]) -> Result<()> {
    if input.core.len() != 2
        || !has_native_join(input)
        || input.core[0].alias == input.core[1].alias
        || input.core.iter().any(|s| s.source.logical().is_none())
        || !input.opts.is_empty()
        || !input.subplan_joins.is_empty()
        || input.path.is_some()
        || input.agg.is_some()
        || input.distinct
        || input.limit.is_some()
        || input.offset != 0
        || !input.order.is_empty()
        || input.nps
        || !input
            .bindings
            .values()
            .all(crate::cascade::binding_is_injective)
    {
        return Err(Error::Unsupported("invalid reference atom relation".into()));
    }
    let mut expected = Vec::new();
    for def in input.bindings.values() {
        for col in def.columns() {
            if !expected.contains(&col) {
                expected.push(col);
            }
        }
    }
    if columns != expected {
        return Err(Error::Sql("reference atom output recipe mismatch".into()));
    }
    Ok(())
}

fn has_native_join(input: &Branch) -> bool {
    let [left, right] = input.core.as_slice() else {
        return false;
    };
    input.where_conds.iter().any(|condition| {
        matches!(condition, SqlCond::NativeColEq(a, b)
            if left.alias != right.alias
                && ((a.alias == left.alias && b.alias == right.alias)
                    || (a.alias == right.alias && b.alias == left.alias)))
    })
}
