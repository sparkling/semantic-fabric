//! Typed operands must survive both literal matching and numeric FILTER lowering.
use super::*;
use crate::iq::literal_cmp::{LiteralComparison, LiteralOperand};

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
