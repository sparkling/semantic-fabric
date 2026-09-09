use super::*;
use crate::iq::iri_cmp::{IriComparison, IriOperand};

pub(super) fn operand(map: &TermMap, alias: usize) -> Option<IriOperand> {
    IriOperand::from_map(map, alias)
}

pub(super) fn identity(left: IriOperand, right: IriOperand) -> SqlCond {
    if let (IriOperand::Constant(a), IriOperand::Constant(b)) = (&left, &right) {
        return if a == b {
            SqlCond::And(vec![])
        } else {
            SqlCond::Or(vec![])
        };
    }
    SqlCond::IriCmp(Box::new(IriComparison { left, right }))
}

pub(super) fn filter(
    a: &Expression,
    b: &Expression,
    op: CmpOp,
    bindings: &BTreeMap<String, TermDef>,
) -> Option<SqlCond> {
    let from_def = |def: &TermDef| match def {
        TermDef::Derived { term_map, alias } => operand(term_map, *alias),
        TermDef::Const(Term::NamedNode(iri)) => Some(IriOperand::Constant(iri.clone())),
        _ => None,
    };
    let from_expr = |expr: &Expression| match expr {
        Expression::Variable(var) => bindings.get(var.as_str()).and_then(from_def),
        Expression::NamedNode(iri) => Some(IriOperand::Constant(iri.clone())),
        _ => None,
    };
    let (left, right) = (from_expr(a)?, from_expr(b)?);
    if [&left, &right].iter().all(|operand| {
        matches!(
            operand,
            IriOperand::Template { base: None, .. } | IriOperand::Constant(_)
        )
    }) && [&left, &right]
        .iter()
        .any(|op| matches!(op, IriOperand::Template { .. }))
    {
        return None; // Preserve the qualified static/native lowering.
    }
    Some(match op {
        CmpOp::Eq => identity(left, right),
        CmpOp::Ne => SqlCond::Not(Box::new(identity(left, right))),
        // SPARQL does not define ordered IRI value comparison.
        _ => SqlCond::ExpressionError,
    })
}
