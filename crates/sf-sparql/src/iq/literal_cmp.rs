//! Literal term identity and numeric value predicates retain construction roles.
use super::{CmpOp, ColRef};
use sf_core::{ir::TermSpec, Literal};

/// Keep natural construction observable when projection/aggregation hides it.
/// The live emitter qualifies decoder-specific validation, never a text cast.
pub(crate) fn validate_term(def: &super::TermDef, conditions: &mut Vec<super::SqlCond>) {
    if let super::TermDef::Derived {
        term_map: sf_core::ir::TermMap::Column(column, spec),
        alias,
    } = def
    {
        if spec.term_type == sf_core::ir::TermType::Literal && spec.language.is_none() {
            let left = LiteralOperand::Column {
                column: ColRef::new(*alias, column.clone()),
                spec: spec.clone(),
            };
            conditions.push(super::SqlCond::LiteralCmp(Box::new(LiteralComparison {
                right: left.clone(),
                left,
                value_op: None,
            })));
        }
    }
}

#[derive(Debug, Clone)]
pub enum LiteralOperand {
    Column { column: ColRef, spec: TermSpec },
    Constant(Literal),
}

impl LiteralOperand {
    pub(crate) fn explicit(&self) -> bool {
        match self {
            Self::Constant(_) => true,
            Self::Column { spec, .. } => spec.datatype.is_some() || spec.language.is_some(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LiteralComparison {
    pub left: LiteralOperand,
    pub right: LiteralOperand,
    /// None means RDF term identity; Some means a FILTER value comparison.
    pub value_op: Option<CmpOp>,
}

impl LiteralComparison {
    pub(crate) fn base_numeric(&self) -> bool {
        [&self.left, &self.right].iter().all(|value| {
            let datatype = match value {
                LiteralOperand::Constant(lit) => Some(lit.datatype().as_str()),
                LiteralOperand::Column { spec, .. } if spec.language.is_none() => {
                    spec.datatype.as_ref().map(|dt| dt.as_str())
                }
                _ => None,
            };
            matches!(
                datatype,
                Some(
                    "http://www.w3.org/2001/XMLSchema#integer"
                        | "http://www.w3.org/2001/XMLSchema#decimal"
                        | "http://www.w3.org/2001/XMLSchema#float"
                        | "http://www.w3.org/2001/XMLSchema#double"
                )
            )
        })
    }

    pub fn columns(&self) -> impl Iterator<Item = &ColRef> {
        [&self.left, &self.right]
            .into_iter()
            .filter_map(|operand| match operand {
                LiteralOperand::Column { column, .. } => Some(column),
                LiteralOperand::Constant(_) => None,
            })
    }

    pub(crate) fn rewrite_columns(&mut self, fix: impl Fn(&mut ColRef)) {
        for operand in [&mut self.left, &mut self.right] {
            if let LiteralOperand::Column { column, .. } = operand {
                fix(column);
            }
        }
    }
}
