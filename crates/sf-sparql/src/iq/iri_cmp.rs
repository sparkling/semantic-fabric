//! Resolved column-IRI identity retains the mapping base through SQL lowering.
use super::ColRef;
use sf_core::NamedNode;

#[derive(Debug, Clone)]
pub enum IriOperand {
    Column {
        column: ColRef,
        base: Option<Box<str>>,
    },
    Constant(NamedNode),
}

#[derive(Debug, Clone)]
pub struct IriComparison {
    pub left: IriOperand,
    pub right: IriOperand,
}

impl IriComparison {
    pub fn columns(&self) -> impl Iterator<Item = &ColRef> {
        [&self.left, &self.right]
            .into_iter()
            .filter_map(|operand| match operand {
                IriOperand::Column { column, .. } => Some(column),
                IriOperand::Constant(_) => None,
            })
    }

    pub(crate) fn rewrite_columns(&mut self, fix: impl Fn(&mut ColRef)) {
        for operand in [&mut self.left, &mut self.right] {
            if let IriOperand::Column { column, .. } = operand {
                fix(column);
            }
        }
    }
}
