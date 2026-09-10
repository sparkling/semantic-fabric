//! Identity of finite native MySQL floating values, without a text-key grant.
use super::*;
use crate::iq::{Scan, ScanSource};
use sf_core::datatype::XsdTypeCode::Double;

pub(in crate::emit) fn is_float(key: NativeScalarKey) -> bool {
    matches!(
        key,
        NativeScalarKey::MysqlFloat4 | NativeScalarKey::MysqlFloat8
    )
}

/// JSON scalar conversion takes val_real into Json_double, preserving -0.
/// Only inspect the sign at zero; this is NOT a general MySQL lexical recipe.
pub(in crate::emit) fn negative_zero(raw: &str) -> String {
    format!("CASE WHEN {raw} IS NULL THEN NULL WHEN {raw}=0 THEN LEFT(JSON_UNQUOTE(JSON_EXTRACT(JSON_ARRAY({raw}),'$[0]')),1)='-' ELSE FALSE END")
}

/// Same-decoder RDF keys are injective in the native value plus zero sign.
/// Unlike floating widths compare their own decoded lexicals, never widened
/// numeric values: f32(1.1) and f64(1.1) spell alike but are different numbers.
/// NativeColEq (source foreign-key semantics) deliberately does not call this.
pub(in crate::emit) fn key_equality(
    a: &ColRef,
    b: &ColRef,
    dialect: Dialect,
    actuals: &ActualColumns,
) -> Result<Option<String>> {
    let (ak, bk) = (
        iri_cmp::scalar_column(a, actuals),
        iri_cmp::scalar_column(b, actuals),
    );
    if dialect != Dialect::MySql || ![ak, bk].into_iter().flatten().any(is_float) {
        return Ok(None);
    }
    if ![ak, bk].into_iter().all(|key| key.is_some_and(is_float))
        || [a, b]
            .iter()
            .any(|c| literal_datatype::fact(c, actuals) != Some(Some(Double)))
    {
        return Err(Error::Unsupported(
            "MySQL floating RDF join requires retained native floating decoders".into(),
        ));
    }
    let (left, right) = (colref(a, dialect, actuals), colref(b, dialect, actuals));
    if ak != bk {
        let lexical = |raw: &str, key| {
            iri_cmp::scalar_lexical(key, raw, dialect)
                .map(|sql| path_comparison::exact_text(sql, dialect))
        };
        return Ok(Some(format!(
            "({}={})",
            lexical(&left, ak.expect("checked floating decoder"))?,
            lexical(&right, bk.expect("checked floating decoder"))?
        )));
    }
    Ok(Some(format!(
        "({left}={right} AND ({})=({}))",
        negative_zero(&left),
        negative_zero(&right)
    )))
}

pub(in crate::emit) fn template_has_float(
    segments: &[sf_core::ir::Segment],
    alias: usize,
    actuals: &ActualColumns,
) -> bool {
    segments.iter().any(|segment| matches!(segment,
        sf_core::ir::Segment::Column(name)
            if iri_cmp::scalar_column(&ColRef::new(alias, name.clone()), actuals).is_some_and(is_float)))
}

/// A shape-mismatch comparison must use the same exact recipe as IRI constants,
/// never MySQL's implicit FLOAT/DOUBLE-to-text conversion inside CONCAT.
pub(in crate::emit) fn template_comparison(
    cond: &SqlCond,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<Option<String>> {
    use crate::iq::iri_cmp::{IriComparison, IriOperand, IriPart};
    let SqlCond::TemplateEq(left, a, right, b, iri) = cond else {
        return Ok(None);
    };
    if dialect != Dialect::MySql
        || !(template_has_float(left, *a, actuals) || template_has_float(right, *b, actuals))
    {
        return Ok(None);
    }
    if !iri {
        return Err(Error::Unsupported(
            "native MySQL floating literal-template comparison requires qualified construction"
                .into(),
        ));
    }
    let operand = |segments: &[sf_core::ir::Segment], alias| IriOperand::Template {
        parts: segments
            .iter()
            .map(|segment| match segment {
                sf_core::ir::Segment::Literal(text) => IriPart::Literal(text.clone()),
                sf_core::ir::Segment::Column(name) => {
                    IriPart::Column(ColRef::new(alias, name.clone()))
                }
            })
            .collect(),
        base: None,
    };
    iri_cmp::render(
        &IriComparison {
            left: operand(left, *a),
            right: operand(right, *b),
        },
        dialect,
        catalog,
        actuals,
        params,
        pidx,
    )
    .map(Some)
}

/// Within an unchanged finite decoder, its raw number and zero sign are an
/// injective key for Rust's decoded and natural terms. Return original fields.
/// This does not grant text rendering, IRI resolution or cross-width pooling.
pub(in crate::emit) fn decoder_key(
    column: &ColRef,
    actuals: &ActualColumns,
    modes: &[crate::iq::LexicalKey],
) -> Option<NativeScalarKey> {
    use crate::iq::scan::LexicalMode;
    let key = iri_cmp::scalar_column(column, actuals).filter(|k| is_float(*k))?;
    if literal_datatype::fact(column, actuals) != Some(Some(Double)) {
        return None;
    }
    modes
        .iter()
        .any(|k| {
            k.column == column.column
                && matches!(
                    k.mode,
                    LexicalMode::Decoded
                        | LexicalMode::DecodedWithNatural
                        | LexicalMode::Natural
                        | LexicalMode::TypedLiteral { .. }
                )
        })
        .then_some(key)
}

/// A pooled MySQL FLOAT/DOUBLE field may change width or fixed-scale spelling.
/// Hidden guards do not consume its output; independent root arms stay separate.
pub(in crate::emit) fn validate_union(
    branches: &[Branch],
    dialect: Dialect,
    catalog: &ColumnCatalog,
) -> Result<()> {
    if dialect != Dialect::MySql || branches.len() < 2 {
        return Ok(());
    }
    for branch in branches {
        let actuals = branch_actuals(branch, dialect, catalog);
        if branch
            .bindings
            .values()
            .flat_map(TermDef::columns)
            .any(|column| iri_cmp::scalar_column(&column, &actuals).is_some_and(is_float))
            || branch
                .core
                .iter()
                .chain(branch.opts.iter().map(|join| &join.scan))
                .any(|scan| has_rendered_float(scan, dialect, catalog))
        {
            return Err(Error::Unsupported(
                "native MySQL floating UNION requires qualified pooled decoder normalization"
                    .into(),
            ));
        }
    }
    Ok(())
}

/// Rendered pooling hides a template's source scalar behind a text column.
/// Check before that lineage is erased; raw D1/pass-through projections remain
/// governed by the output-consumer check above. This is only a UNION veto.
fn has_rendered_float(scan: &Scan, dialect: Dialect, catalog: &ColumnCatalog) -> bool {
    match &scan.source {
        ScanSource::Projection { input, columns, .. } => {
            let actuals = HashMap::from([(input.alias, scan_actuals(input, dialect, catalog))]);
            columns.iter().any(|(_, term)| {
                matches!(term,
                TermMap::Template(template, _)
                    if template_has_float(template.segments(), input.alias, &actuals))
            }) || has_rendered_float(input, dialect, catalog)
        }
        ScanSource::RefAtom { input, .. } => input
            .core
            .iter()
            .chain(input.opts.iter().map(|join| &join.scan))
            .any(|scan| has_rendered_float(scan, dialect, catalog)),
        ScanSource::Logical(_) | ScanSource::Path { .. } => false,
    }
}

pub(in crate::emit) fn comparison(
    cmp: &LiteralComparison,
    dialect: Dialect,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<Option<String>> {
    if dialect != Dialect::MySql
        || cmp.value_op.is_some()
        || ![&cmp.left, &cmp.right].iter().any(|operand| {
            matches!(operand, LiteralOperand::Column { column, spec }
                if spec.uses_natural_type(Some(Double))
                    && (iri_cmp::scalar_column(column, actuals).is_some_and(is_float)
                        || literal_datatype::fact(column, actuals) == Some(Some(Double))))
        })
    {
        return Ok(None);
    }
    if let (
        LiteralOperand::Column {
            column: left,
            spec: ls,
        },
        LiteralOperand::Column {
            column: right,
            spec: rs,
        },
    ) = (&cmp.left, &cmp.right)
    {
        // The same finite width's reconstructed terms are injective in the
        // original number plus zero sign. Do not rebuild both shortest decimals
        // for reflexive guards or same-width joins (especially in EXISTS arms).
        if column_key(left, ls, actuals)? == column_key(right, rs, actuals)? {
            let l = colref(left, dialect, actuals);
            if left == right {
                return Ok(Some(format!(
                    "CASE WHEN {l} IS NULL THEN NULL ELSE TRUE END"
                )));
            }
            let r = colref(right, dialect, actuals);
            return Ok(Some(format!(
                "CASE WHEN {l} IS NULL OR {r} IS NULL THEN NULL ELSE ({l}={r} AND ({})=({})) END",
                negative_zero(&l),
                negative_zero(&r)
            )));
        }
    }
    let left = operand(&cmp.left, actuals, params, pidx)?;
    let right = operand(&cmp.right, actuals, params, pidx)?;
    // Tags: 0 finite canonical term; 1 nonmatching constant; 2 missing column.
    Ok(Some(format!(
        r#"(WITH __sf_float_identity AS (
      SELECT {left} AS l,{right} AS r LIMIT 18446744073709551615)
      SELECT CASE WHEN JSON_EXTRACT(l,'$[0]')=2 OR JSON_EXTRACT(r,'$[0]')=2 THEN NULL
        WHEN JSON_EXTRACT(l,'$[0]')=1 OR JSON_EXTRACT(r,'$[0]')=1 THEN FALSE
        ELSE CAST(JSON_EXTRACT(l,'$[1]') AS DOUBLE)=CAST(JSON_EXTRACT(r,'$[1]') AS DOUBLE)
          AND JSON_EXTRACT(l,'$[2]')=JSON_EXTRACT(r,'$[2]') END FROM __sf_float_identity)"#
    )))
}

fn column_key(
    column: &ColRef,
    spec: &sf_core::ir::TermSpec,
    actuals: &ActualColumns,
) -> Result<NativeScalarKey> {
    iri_cmp::scalar_column(column, actuals)
        .filter(|k| is_float(*k))
        .filter(|_| literal_datatype::fact(column, actuals) == Some(Some(Double))
            && spec.uses_natural_type(Some(Double)))
        .ok_or_else(|| Error::Unsupported("MySQL floating identity requires retained native width and natural/matching Double operands".into()))
}

fn operand(
    value: &LiteralOperand,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    match value {
        LiteralOperand::Constant(literal) => {
            let mut canonical = String::new();
            let parsed = sf_core::numeric_compare::promote_to_double(
                literal.value(),
                literal.datatype().as_str(),
            );
            if literal.datatype() != Double.iri()
                || literal.language().is_some()
                || sf_core::datatype::canonical_lexical(literal.value(), Double, &mut canonical)
                    .is_err()
                || canonical != literal.value()
                || !parsed.is_some_and(f64::is_finite)
            {
                return Ok("JSON_ARRAY(1,CAST(0 AS DOUBLE),0)".into());
            }
            let value = parsed.unwrap();
            // Integer coefficient avoids MySQL's fractional scanner truncation.
            let (mantissa, exponent) = canonical
                .split_once('E')
                .expect("canonical Double exponent");
            let (whole, fraction) = mantissa.split_once('.').expect("canonical Double fraction");
            let exponent: i32 = exponent.parse().expect("bounded Double exponent");
            params.push(format!(
                "{whole}{fraction}e{}",
                exponent - fraction.len() as i32
            ));
            *pidx += 1;
            Ok(format!(
                "JSON_ARRAY(0,CAST(? AS DOUBLE),{})",
                if value == 0. && value.is_sign_negative() {
                    "1"
                } else {
                    "0"
                }
            ))
        }
        LiteralOperand::Column { column, spec } => {
            let key = column_key(column, spec, actuals)?;
            let raw = colref(column, Dialect::MySql, actuals);
            let number = if key == NativeScalarKey::MysqlFloat4 {
                format!(
                    "CAST(JSON_EXTRACT({},'$[1]') AS DOUBLE)",
                    shortest::float_value(&raw)
                )
            } else {
                format!("CAST(COALESCE({raw},0) AS DOUBLE)")
            };
            Ok(format!(
                "JSON_ARRAY(CASE WHEN {raw} IS NULL THEN 2 ELSE 0 END,{number},{})",
                negative_zero(&raw)
            ))
        }
    }
}

#[cfg(test)]
#[path = "mysql_float_identity_tests.rs"]
mod tests;
