//! Prepare-derived datatype evidence, distinct from decoder/lexical authority.
use super::*;
use crate::iq::literal_cmp::{LiteralComparison, LiteralOperand};
use sf_core::datatype::XsdTypeCode;

pub(super) fn fact(column: &ColRef, actuals: &ActualColumns) -> Option<Option<XsdTypeCode>> {
    let source = actuals.get(&column.alias)?;
    source
        .datatype_columns
        .get(resolve_col(&column.column, Some(&source.columns)))
        .copied()
}

pub(super) fn after_union(
    code: Option<XsdTypeCode>,
    dialect: Dialect,
    arms: usize,
) -> Option<XsdTypeCode> {
    // Without native width/signedness, equal XSD codes cannot prove MySQL's
    // result decoder: BIGINT may become DECIMAL; Boolean TINYINT may widen.
    if dialect == Dialect::MySql
        && arms > 1
        && matches!(code, Some(XsdTypeCode::Integer | XsdTypeCode::Boolean))
    {
        None
    } else {
        code
    }
}

pub(super) fn merge<K: std::hash::Hash + Eq + Clone>(
    common: &mut Option<HashMap<K, Option<XsdTypeCode>>>,
    current: HashMap<K, Option<XsdTypeCode>>,
) {
    match common {
        None => *common = Some(current),
        Some(common) => {
            for (key, code) in common.iter_mut() {
                if current.get(key) != Some(code) {
                    *code = None;
                }
            }
            for key in current.keys() {
                common.entry(key.clone()).or_insert(None);
            }
        }
    }
}

pub(super) fn mismatch(
    cmp: &LiteralComparison,
    dialect: Dialect,
    actuals: &ActualColumns,
) -> Result<Option<String>> {
    if cmp.value_op.is_some() || !matches!(dialect, Dialect::MySql | Dialect::Postgres) {
        return Ok(None);
    }
    fn components<'a>(
        value: &'a LiteralOperand,
        actuals: &ActualColumns,
    ) -> Result<Option<(&'a str, &'a str)>> {
        match value {
            LiteralOperand::Constant(literal) => Ok(Some((
                literal.datatype().as_str(),
                literal.language().unwrap_or(""),
            ))),
            LiteralOperand::Column { column, spec } => {
                if let Some(language) = &spec.language {
                    return Ok(Some((
                        "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString",
                        language,
                    )));
                }
                if let Some(datatype) = &spec.datatype {
                    return Ok(Some((datatype.as_str(), "")));
                }
                match fact(column, actuals) {
                    Some(Some(code)) => Ok(Some((code.iri().as_str(), ""))),
                    Some(None) => Err(Error::Unsupported("literal identity requires compatible decoder datatypes in every SubPlan arm".into())),
                    None => Ok(None),
                }
            }
        }
    }
    let (Some(left), Some(right)) = (
        components(&cmp.left, actuals)?,
        components(&cmp.right, actuals)?,
    ) else {
        return Ok(None);
    };
    if left == right {
        return Ok(None);
    }
    let missing = cmp
        .columns()
        .map(|column| format!("{} IS NULL", colref(column, dialect, actuals)))
        .collect::<Vec<_>>();
    Ok(Some(if missing.is_empty() {
        "FALSE".into()
    } else {
        format!(
            "CASE WHEN {} THEN NULL ELSE FALSE END",
            missing.join(" OR ")
        )
    }))
}

pub(super) fn implicit_column_pair(cmp: &LiteralComparison) -> bool {
    [&cmp.left, &cmp.right].iter().all(|operand| {
        matches!(operand,
        LiteralOperand::Column { spec, .. } if spec.datatype.is_none() && spec.language.is_none())
    })
}
