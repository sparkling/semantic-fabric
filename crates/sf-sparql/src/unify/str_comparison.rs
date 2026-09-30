//! STR lexical comparisons retain string mapping and live source qualification.
use super::*;
use crate::iq::literal_cmp::LiteralComparison;

const STRING: &str = "http://www.w3.org/2001/XMLSchema#string";

pub(super) fn comparison(
    left: &Expression,
    right: &Expression,
    op: CmpOp,
    bindings: &BTreeMap<String, TermDef>,
) -> Option<Result<SqlCond, String>> {
    fn str_arg(expr: &Expression) -> Option<&[Expression]> {
        match expr {
            Expression::FunctionCall(Function::Str, args) => Some(args),
            _ => None,
        }
    }
    let (args, literal, op) = if let Some(args) = str_arg(left) {
        (args, right, op)
    } else if let Some(args) = str_arg(right) {
        let flipped = match op {
            CmpOp::Lt => CmpOp::Gt,
            CmpOp::Le => CmpOp::Ge,
            CmpOp::Gt => CmpOp::Lt,
            CmpOp::Ge => CmpOp::Le,
            other => other,
        };
        (args, left, flipped)
    } else {
        return None;
    };
    Some(lower(args, literal, op, bindings))
}

fn lower(
    args: &[Expression],
    rhs: &Expression,
    op: CmpOp,
    bindings: &BTreeMap<String, TermDef>,
) -> Result<SqlCond, String> {
    let [Expression::Variable(var)] = args else {
        return Err("STR comparison requires one variable operand".into());
    };
    let Expression::Literal(right) = rhs else {
        return Err("STR comparison requires a string literal bound".into());
    };
    if right.datatype().as_str() != STRING || right.language().is_some() {
        return Err("STR comparison requires a plain or xsd:string literal bound".into());
    }
    let Some(term) = bindings.get(var.as_str()) else {
        // Preserve SPARQL error through NOT/AND/OR rather than folding to false.
        return Ok(SqlCond::ExpressionError);
    };
    let left = match term {
        TermDef::Derived {
            term_map: TermMap::Column(column, spec),
            alias,
        } if spec.term_type == TermType::Literal
            && spec.language.is_none()
            && spec.datatype.as_ref().is_none_or(|dt| dt.as_str() == STRING) =>
        {
            LiteralOperand::Column {
                column: ColRef::new(*alias, column.clone()),
                spec: spec.clone(),
            }
        }
        TermDef::Const(Term::Literal(left)) if left.datatype().as_str() == STRING => {
            let order = left.value().cmp(right.value());
            let yes = match op {
                CmpOp::Eq => order.is_eq(),
                CmpOp::Ne => !order.is_eq(),
                CmpOp::Lt => order.is_lt(),
                CmpOp::Le => !order.is_gt(),
                CmpOp::Gt => order.is_gt(),
                CmpOp::Ge => !order.is_lt(),
            };
            return Ok(if yes { SqlCond::And(vec![]) } else { SqlCond::Or(vec![]) });
        }
        _ => return Err("STR comparison requires a string column or constant; non-string datatype, IRI and constructed lexicals need separate qualification".into()),
    };
    // Retain typed operands so emission keeps its decoder/collation and NULL guards.
    Ok(SqlCond::LiteralCmp(Box::new(LiteralComparison {
        left,
        right: LiteralOperand::Constant(right.clone()),
        value_op: Some(op),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn var() -> Expression {
        Expression::Variable(Variable::new("x").unwrap())
    }

    #[test]
    fn unproven_constructions_refuse_instead_of_stripping_keys() {
        let str_x = Expression::FunctionCall(Function::Str, vec![var()]);
        let bound = Expression::Literal(Literal::new_simple_literal("2"));
        for spec in [
            TermSpec::iri(),
            TermSpec::blank_node(),
            TermSpec::lang_literal("en"),
            TermSpec::typed_literal(sf_core::NamedNode::new_unchecked(
                "http://www.w3.org/2001/XMLSchema#integer",
            )),
        ] {
            let bindings = [(
                "x".into(),
                TermDef::Derived {
                    term_map: TermMap::Column("v".into(), spec),
                    alias: 0,
                },
            )]
            .into_iter()
            .collect();
            assert!(comparison(&str_x, &bound, CmpOp::Gt, &bindings)
                .unwrap()
                .is_err());
        }
        assert!(matches!(
            comparison(&str_x, &bound, CmpOp::Gt, &BTreeMap::new()),
            Some(Ok(SqlCond::ExpressionError))
        ));
        let bindings = [(
            "x".into(),
            TermDef::Derived {
                term_map: TermMap::Template(
                    sf_core::ir::Template::parse("prefix-{v}").unwrap(),
                    TermSpec::plain_literal(),
                ),
                alias: 0,
            },
        )]
        .into_iter()
        .collect();
        assert!(comparison(&str_x, &bound, CmpOp::Gt, &bindings)
            .unwrap()
            .is_err());
    }
}
