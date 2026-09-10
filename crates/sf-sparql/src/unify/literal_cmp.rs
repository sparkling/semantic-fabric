//! Typed operands must survive both literal matching and numeric FILTER lowering.
use super::*;
use crate::iq::literal_cmp::{LiteralComparison, LiteralOperand};

fn template_pair<'a>(
    a: &Expression,
    b: &Expression,
    bindings: &'a BTreeMap<String, TermDef>,
) -> Option<[(&'a TermDef, &'a TermSpec); 2]> {
    let get = |expr: &Expression| {
        let Expression::Variable(var) = expr else {
            return None;
        };
        let def = bindings.get(var.as_str())?;
        let TermDef::Derived {
            term_map: TermMap::Template(_, spec),
            ..
        } = def
        else {
            return None;
        };
        (spec.term_type == TermType::Literal).then_some((def, spec))
    };
    Some([get(a)?, get(b)?])
}

/// Template lexicals are RDF identity, not typed FILTER value construction.
/// Keep this distinction before align_templates erases the term specification.
pub(super) fn template_value_needs_construction(
    a: &Expression,
    b: &Expression,
    bindings: &BTreeMap<String, TermDef>,
) -> bool {
    template_pair(a, b, bindings).is_some_and(|pair| {
        pair.iter().any(|(_, spec)| {
            spec.language.is_none()
                && spec.datatype.as_ref().is_some_and(|datatype| {
                    datatype.as_str() != "http://www.w3.org/2001/XMLSchema#string"
                })
        })
    })
}

pub(super) fn template_identity(
    a: &Expression,
    b: &Expression,
    bindings: &BTreeMap<String, TermDef>,
) -> Option<Result<SqlCond, String>> {
    let [(left, _), (right, _)] = template_pair(a, b, bindings)?;
    Some(match unify(left, right) {
        Unify::Sat(conditions) => Ok(SqlCond::And(conditions)),
        Unify::Empty => Ok(SqlCond::Or(vec![])),
        Unify::Unsupported(why) => Err(why),
    })
}

/// A different RDF kind is never a raw-column equality. NULL source columns
/// still mean an unbound operand, not a false expression that NOT may admit.
pub(super) fn kind_mismatch(
    a: &Expression,
    b: &Expression,
    op: CmpOp,
    bindings: &BTreeMap<String, TermDef>,
) -> Option<SqlCond> {
    fn constant(term: &Term) -> Option<(TermType, Vec<ColRef>)> {
        Some((
            match term {
                Term::NamedNode(_) => TermType::Iri,
                Term::Literal(_) => TermType::Literal,
                Term::BlankNode(_) => TermType::BlankNode,
                _ => return None,
            },
            vec![],
        ))
    }
    let kind = |expr: &Expression| match expr {
        Expression::NamedNode(_) => Some((TermType::Iri, vec![])),
        Expression::Literal(_) => Some((TermType::Literal, vec![])),
        Expression::Variable(var) => match bindings.get(var.as_str())? {
            TermDef::Const(term) => constant(term),
            TermDef::Derived {
                term_map: TermMap::Column(column, spec),
                alias,
            } => Some((spec.term_type, vec![ColRef::new(*alias, column.clone())])),
            _ => None,
        },
        _ => None,
    };
    let (left, mut columns) = kind(a)?;
    let (right, other) = kind(b)?;
    if left == right {
        return None;
    }
    if !matches!(op, CmpOp::Eq | CmpOp::Ne) {
        return Some(SqlCond::ExpressionError);
    }
    columns.extend(other);
    // UNKNOWN AND false = false; UNKNOWN AND true = UNKNOWN. This preserves
    // SPARQL's unbound error without giving optimizers raw-key join authority.
    let mismatch = SqlCond::And(vec![
        SqlCond::ExpressionError,
        SqlCond::Or(columns.into_iter().map(SqlCond::IsNull).collect()),
    ]);
    Some(if op == CmpOp::Ne {
        SqlCond::Not(Box::new(mismatch))
    } else {
        mismatch
    })
}

pub(super) fn operand(map: &TermMap, alias: usize) -> Option<LiteralOperand> {
    let TermMap::Column(column, spec) = map else {
        return None;
    };
    (spec.term_type == TermType::Literal).then(|| LiteralOperand::Column {
        column: ColRef::new(alias, column.clone()),
        spec: spec.clone(),
    })
}

pub(super) fn identity(left: LiteralOperand, right: LiteralOperand) -> SqlCond {
    SqlCond::LiteralCmp(Box::new(LiteralComparison {
        left,
        right,
        value_op: None,
    }))
}

fn from_expr(expr: &Expression, bindings: &BTreeMap<String, TermDef>) -> Option<LiteralOperand> {
    match expr {
        Expression::Literal(value) => Some(LiteralOperand::Constant(value.clone())),
        Expression::Variable(var) => match bindings.get(var.as_str())? {
            TermDef::Derived { term_map, alias } => operand(term_map, *alias),
            TermDef::Const(Term::Literal(value)) => Some(LiteralOperand::Constant(value.clone())),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn filter(
    a: &Expression,
    b: &Expression,
    op: Option<CmpOp>,
    bindings: &BTreeMap<String, TermDef>,
) -> Option<SqlCond> {
    let left = from_expr(a, bindings)?;
    let right = from_expr(b, bindings)?;
    let cmp = LiteralComparison {
        left,
        right,
        value_op: op,
    };
    // Preserve the existing nonnumeric VALUES variable-pair equality path.
    // Only numeric constants gain value-promotion semantics in this slice.
    if op.is_some()
        && !cmp.base_numeric()
        && matches!(
            (&cmp.left, &cmp.right),
            (LiteralOperand::Constant(_), LiteralOperand::Constant(_))
        )
    {
        return None;
    }
    Some(SqlCond::LiteralCmp(Box::new(cmp)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_template_value_filters_do_not_borrow_identity_keys() {
        let variable = |name| Expression::Variable(Variable::new(name).unwrap());
        for (left, right) in [
            ("float", "float"),
            ("int", "decimal"),
            ("dateTime", "dateTime"),
        ] {
            let bindings = [("a", "v", left), ("b", "w", right)]
                .into_iter()
                .map(|(var, column, datatype)| {
                    (
                        var.to_owned(),
                        TermDef::Derived {
                            term_map: TermMap::Template(
                                sf_core::ir::Template::parse(&format!("{{{column}}}")).unwrap(),
                                TermSpec::typed_literal(
                                    sf_core::NamedNode::new(format!(
                                        "http://www.w3.org/2001/XMLSchema#{datatype}"
                                    ))
                                    .unwrap(),
                                ),
                            ),
                            alias: 0,
                        },
                    )
                })
                .collect();
            let a = variable("a");
            let b = variable("b");
            for expr in [
                Expression::Equal(Box::new(a.clone()), Box::new(b.clone())),
                Expression::Not(Box::new(Expression::Equal(
                    Box::new(a.clone()),
                    Box::new(b.clone()),
                ))),
            ] {
                let error = filter_cond(&expr, &bindings, Dialect::MySql).unwrap_err();
                assert!(error.contains("qualified value construction"), "{error}");
            }
            let same = filter_cond(
                &Expression::SameTerm(Box::new(a), Box::new(b)),
                &bindings,
                Dialect::MySql,
            )
            .unwrap();
            if left == right {
                assert!(
                    matches!(same, SqlCond::And(ref conditions) if matches!(conditions.as_slice(), [SqlCond::ColEq(..)]))
                );
            } else {
                assert!(matches!(same, SqlCond::Or(ref conditions) if conditions.is_empty()));
            }
        }
    }
}
