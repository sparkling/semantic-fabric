//! Literal identity is a (lexical, datatype, language) tuple, not SQL equality.
use super::*;
use crate::iq::literal_cmp::{LiteralComparison, LiteralOperand};

pub(super) fn render(
    cmp: &LiteralComparison,
    dialect: Dialect,
    catalog: &ColumnCatalog,
    actuals: &ActualColumns,
    params: &mut Vec<String>,
    pidx: &mut usize,
) -> Result<String> {
    if let Some(sql) = literal_datatype::mismatch(cmp, dialect, actuals)? {
        return Ok(sql);
    }
    if let Some(sql) = pg_float_value::comparison(cmp, dialect, actuals, params, pidx)? {
        return Ok(sql);
    }
    // Offline branches can have aliases and synthetic names, but no live
    // result metadata. A live prepare inserts a datatype map even when all
    // facts are absent: those columns must reach the strict decoder check.
    if !catalog.datatypes_by_source.is_empty() || cmp.columns().next().is_none() {
        if let Some(sql) = pg_decimal_value::comparison(cmp, dialect, actuals, params, pidx)? {
            return Ok(sql);
        }
    }
    if let (LiteralOperand::Constant(left), LiteralOperand::Constant(right)) =
        (&cmp.left, &cmp.right)
    {
        let value = match cmp.value_op {
            None => Some(left == right),
            Some(op) if cmp.base_numeric() => {
                use sf_core::numeric_compare::NumericOp;
                sf_core::numeric_compare::compare(
                    left.value(),
                    left.datatype().as_str(),
                    right.value(),
                    right.datatype().as_str(),
                    match op {
                        crate::iq::CmpOp::Eq => NumericOp::Eq,
                        crate::iq::CmpOp::Ne => NumericOp::Ne,
                        crate::iq::CmpOp::Lt => NumericOp::Lt,
                        crate::iq::CmpOp::Le => NumericOp::Le,
                        crate::iq::CmpOp::Gt => NumericOp::Gt,
                        crate::iq::CmpOp::Ge => NumericOp::Ge,
                    },
                )
                .map_err(|e| Error::Unsupported(e.to_string()))?
            }
            // Nonnumeric VALUES variable pairs retain their earlier lowering
            // outside this variant. Do not grant typed numeric predicates
            // generic SQL-string comparison authority for other RDF families.
            Some(_) => {
                return Err(Error::Unsupported(
                    "constant literal value comparison requires supported numeric operands".into(),
                ))
            }
        };
        return Ok(match value {
            Some(true) => "(1 = 1)",
            Some(false) => "(1 = 0)",
            None => "(NULL = 1)",
        }
        .into());
    }
    if let Some(sql) =
        natural_decimal::value_equality(cmp, dialect, catalog, actuals, params, pidx)?
    {
        return Ok(sql);
    }
    if let Some(sql) = natural_literal::comparison(cmp, dialect, catalog, actuals, params, pidx)? {
        return Ok(sql);
    }
    let mut bind = |value: &str| {
        params.push(value.to_owned());
        *pidx += 1;
        dialect.placeholder(*pidx)
    };
    // A missing live natural decoder is not evidence of xsd:string. Native
    // non-text decoder-equivalent identity remains a separate qualification;
    // retain its existing comparison instead of inventing a datatype here.
    let unknown_natural = [&cmp.left, &cmp.right].iter().any(|value| {
        matches!(value, LiteralOperand::Column { column, spec }
            if spec.datatype.is_none() && spec.language.is_none()
                && (dialect != Dialect::Sqlite
                    || lexical_key::column_decode(column, actuals).is_none()))
    });
    let decoded_numeric = dialect == Dialect::Sqlite
        && cmp.base_numeric()
        && cmp
            .columns()
            .all(|c| lexical_key::column_decode(c, actuals).is_some());
    if cmp.value_op.is_none()
        && unknown_natural
        && cmp.columns().count() == 2
        && !literal_datatype::implicit_column_pair(cmp)
        && ((matches!(dialect, Dialect::MySql | Dialect::Postgres)
            && cmp.columns().any(|column| literal_datatype::fact(column, actuals).is_some()))
            // A live prepare may know names but no datatypes (legacy drivers).
            // Do not turn formerly disjoint mixed mappings into raw equality.
            // Offline rendering and the old explicit-string lane are unchanged;
            // neither is newly qualified by the column-join implementation.
            || (!catalog.datatypes_by_source.is_empty()
                && [&cmp.left, &cmp.right].iter().any(|value| matches!(value,
                    LiteralOperand::Column { spec, .. } if spec.language.is_some()
                        || spec.datatype.as_ref().is_some_and(|dt| dt.as_str() != "http://www.w3.org/2001/XMLSchema#string")))))
    {
        return Err(Error::Unsupported(
            "mixed natural literal identity requires exact native decoder keys".into(),
        ));
    }
    if unknown_natural || (cmp.value_op.is_some() && !decoded_numeric) {
        let mut raw = |value: &LiteralOperand| match value {
            LiteralOperand::Column { column, .. } if cmp.value_op.is_some() => {
                path_comparison::rdf_text_column(column, dialect, catalog, actuals)
            }
            LiteralOperand::Column { column, .. } => {
                path_comparison::rdf_column(column, dialect, catalog, actuals)
            }
            LiteralOperand::Constant(literal) => bind(literal.value()),
        };
        return Ok(format!(
            "{} {} {}",
            raw(&cmp.left),
            cmp.value_op.unwrap_or(crate::iq::CmpOp::Eq).as_sql(),
            raw(&cmp.right)
        ));
    }
    // Render components in SQL text order: MySQL and SQLite use anonymous binds.
    let mut component = |value: &LiteralOperand, part: usize| -> Result<String> {
        match value {
            LiteralOperand::Constant(literal) => Ok(bind(match part {
                0 => literal.value(),
                1 => literal.datatype().as_str(),
                _ => literal.language().unwrap_or(""),
            })),
            LiteralOperand::Column { column, spec } => {
                let raw = colref(column, dialect, actuals);
                let natural = spec.datatype.is_none() && spec.language.is_none();
                let decode = (dialect == Dialect::Sqlite)
                    .then(|| lexical_key::column_decode(column, actuals))
                    .flatten();
                if part == 0 {
                    return Ok(if let Some(decode) = decode {
                        if let Some(datatype) =
                            spec.datatype.as_ref().filter(|_| spec.language.is_none())
                        {
                            lexical_key::typed(raw.clone(), decode, datatype.as_str(), catalog)
                        } else {
                            lexical_key::with_mode(raw.clone(), decode, natural, catalog)
                        }
                    } else {
                        path_comparison::rdf_column(column, dialect, catalog, actuals)
                    });
                }
                if part == 1 {
                    return Ok(if let Some(decode) = decode.filter(|_| natural) {
                        lexical_key::natural_datatype(&raw, decode)
                    } else {
                        bind(spec.datatype.as_ref().map(|dt| dt.as_str()).unwrap_or(
                            if spec.language.is_some() {
                                "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString"
                            } else {
                                "http://www.w3.org/2001/XMLSchema#string"
                            },
                        ))
                    });
                }
                Ok(bind(spec.language.as_deref().unwrap_or("")))
            }
        }
    };
    if let Some(op) = cmp.value_op {
        let left = component(&cmp.left, 0)?;
        let ldt = component(&cmp.left, 1)?;
        let right = component(&cmp.right, 0)?;
        let rdt = component(&cmp.right, 1)?;
        catalog
            .lexical_keys
            .store(true, std::sync::atomic::Ordering::Relaxed);
        return Ok(format!(
            "__sf_numeric_cmp_v1({left}, {ldt}, {right}, {rdt}, {})",
            match op {
                crate::iq::CmpOp::Eq => 0,
                crate::iq::CmpOp::Ne => 1,
                crate::iq::CmpOp::Lt => 2,
                crate::iq::CmpOp::Le => 3,
                crate::iq::CmpOp::Gt => 4,
                crate::iq::CmpOp::Ge => 5,
            }
        ));
    }
    let left = component(&cmp.left, 0)?;
    let right = component(&cmp.right, 0)?;
    let ldt = component(&cmp.left, 1)?;
    let rdt = component(&cmp.right, 1)?;
    let lang = component(&cmp.left, 2)?;
    let rang = component(&cmp.right, 2)?;
    let ldt = path_comparison::exact_text(ldt, dialect);
    let rdt = path_comparison::exact_text(rdt, dialect);
    let lang = path_comparison::exact_text(lang, dialect);
    let rang = path_comparison::exact_text(rang, dialect);
    let equality = format!("({left} = {right} AND {ldt} = {rdt} AND {lang} = {rang})");
    let missing = cmp
        .columns()
        .map(|c| format!("{} IS NULL", colref(c, dialect, actuals)))
        .collect::<Vec<_>>();
    Ok(if missing.is_empty() {
        equality
    } else {
        format!(
            "CASE WHEN {} THEN NULL ELSE {equality} END",
            missing.join(" OR ")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_decoder_preserves_value_lane_without_fabricating_callback_input() {
        let integer = sf_core::NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer");
        let mut cmp = LiteralComparison {
            left: LiteralOperand::Column {
                column: ColRef::new(0, "o"),
                spec: sf_core::ir::TermSpec::typed_literal(integer.clone()),
            },
            right: LiteralOperand::Constant(sf_core::Literal::new_typed_literal("9", integer)),
            value_op: Some(crate::iq::CmpOp::Gt),
        };
        for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
            let catalog = ColumnCatalog::default();
            let mut params = Vec::new();
            let sql = render(
                &cmp,
                dialect,
                &catalog,
                &ActualColumns::new(),
                &mut params,
                &mut 0,
            )
            .unwrap();
            assert!(!sql.contains("__sf_"), "{sql}");
            assert_eq!(params, ["9"]);
            assert!(!catalog
                .lexical_keys
                .load(std::sync::atomic::Ordering::Relaxed));
        }
        cmp.value_op = None;
        let LiteralOperand::Column { spec, .. } = &mut cmp.left else {
            unreachable!()
        };
        *spec = sf_core::ir::TermSpec::plain_literal();
        let mut params = Vec::new();
        let sql = render(
            &cmp,
            Dialect::Postgres,
            &ColumnCatalog::default(),
            &ActualColumns::new(),
            &mut params,
            &mut 0,
        )
        .unwrap();
        assert_eq!(params, ["9"]);
        assert!(!sql.contains("string"));
    }
}
