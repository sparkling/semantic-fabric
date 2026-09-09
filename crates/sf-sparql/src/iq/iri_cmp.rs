//! Resolved IRI identity retains the mapping base through SQL lowering.
use super::ColRef;
use sf_core::ir::{Segment, TermMap, TermType};
use sf_core::NamedNode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IriPart {
    Literal(Box<str>),
    Column(ColRef),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IriOperand {
    Column {
        column: ColRef,
        base: Option<Box<str>>,
    },
    Template {
        parts: Vec<IriPart>,
        base: Option<Box<str>>,
    },
    Constant(NamedNode),
}

impl IriOperand {
    pub(crate) fn from_map(map: &TermMap, alias: usize) -> Option<Self> {
        match map {
            TermMap::Column(column, spec) if spec.term_type == TermType::Iri => {
                Some(Self::Column {
                    column: ColRef::new(alias, column.clone()),
                    base: spec.base.clone(),
                })
            }
            TermMap::Template(template, spec) if spec.term_type == TermType::Iri => {
                Some(Self::Template {
                    parts: template
                        .segments()
                        .iter()
                        .map(|part| match part {
                            Segment::Literal(text) => IriPart::Literal(text.clone()),
                            Segment::Column(name) => {
                                IriPart::Column(ColRef::new(alias, name.clone()))
                            }
                        })
                        .collect(),
                    base: spec.base.clone(),
                })
            }
            _ => None,
        }
    }

    pub(crate) fn columns(&self) -> impl Iterator<Item = &ColRef> {
        let column = match self {
            Self::Column { column, .. } => Some(column),
            _ => None,
        };
        let parts = match self {
            Self::Template { parts, .. } => parts.as_slice(),
            _ => &[],
        };
        column
            .into_iter()
            .chain(parts.iter().filter_map(|part| match part {
                IriPart::Column(column) => Some(column),
                _ => None,
            }))
    }
}

pub(crate) fn needs_resolution(map: &TermMap) -> bool {
    matches!(map, TermMap::Column(_, spec) if spec.term_type == TermType::Iri)
        || is_late_template(map)
}

pub(crate) fn is_late_template(map: &TermMap) -> bool {
    matches!(map, TermMap::Template(_, spec) if spec.term_type == TermType::Iri && spec.base.is_some())
}

pub(crate) fn validate_term(def: &super::TermDef, conditions: &mut Vec<super::SqlCond>) {
    if let super::TermDef::Derived { term_map, alias } = def {
        if is_late_template(term_map) {
            let left = IriOperand::from_map(term_map, *alias).unwrap();
            conditions.push(super::SqlCond::IriCmp(Box::new(IriComparison {
                right: left.clone(),
                left,
            })));
        }
    }
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
            .flat_map(IriOperand::columns)
    }

    pub(crate) fn rewrite_columns(&mut self, fix: impl Fn(&mut ColRef)) {
        for operand in [&mut self.left, &mut self.right] {
            match operand {
                IriOperand::Column { column, .. } => fix(column),
                IriOperand::Template { parts, .. } => {
                    for part in parts {
                        if let IriPart::Column(column) = part {
                            fix(column);
                        }
                    }
                }
                IriOperand::Constant(_) => {}
            }
        }
    }
}

/// A source-local IRI condition may filter an atom before rendered dedup. It
/// grants no raw-table restoration or policy bypass authority.
pub(crate) fn atom_guard(condition: &super::SqlCond, alias: usize) -> bool {
    match condition {
        super::SqlCond::IsNull(c)
        | super::SqlCond::IsNotNull(c)
        | super::SqlCond::DecodedIsNotNull(c) => c.alias == alias,
        super::SqlCond::IriCmp(cmp) => cmp.columns().all(|c| c.alias == alias),
        super::SqlCond::Not(inner) => atom_guard(inner, alias),
        super::SqlCond::And(parts) | super::SqlCond::Or(parts) => {
            parts.iter().all(|c| atom_guard(c, alias))
        }
        _ => false,
    }
}

pub(crate) fn contains_late_template(condition: &super::SqlCond) -> bool {
    match condition {
        super::SqlCond::IriCmp(cmp) => [&cmp.left, &cmp.right]
            .iter()
            .any(|op| matches!(op, IriOperand::Template { base: Some(_), .. })),
        super::SqlCond::Not(inner) => contains_late_template(inner),
        super::SqlCond::And(parts) | super::SqlCond::Or(parts) => {
            parts.iter().any(contains_late_template)
        }
        _ => false,
    }
}

/// Path endpoints do not yet carry the original per-cell decoder through a
/// fixed point. Preserve their pre-source FILTER boundary, not a guessed key.
pub(crate) fn validate_filter_source(
    condition: &super::SqlCond,
    branch: &super::Branch,
    dialect: sf_sql::Dialect,
) -> Result<(), String> {
    match condition {
        super::SqlCond::IriCmp(cmp)
            if cmp
                .columns()
                .any(|column| path_column(column, branch, dialect)) =>
        {
            Err("FILTER on a path endpoint requires an unimplemented decoder identity proof".into())
        }
        super::SqlCond::Not(inner) => validate_filter_source(inner, branch, dialect),
        super::SqlCond::And(parts) | super::SqlCond::Or(parts) => parts
            .iter()
            .try_for_each(|part| validate_filter_source(part, branch, dialect)),
        _ => Ok(()),
    }
}

fn path_column(column: &ColRef, branch: &super::Branch, dialect: sf_sql::Dialect) -> bool {
    if branch
        .path
        .as_ref()
        .is_some_and(|path| path.alias == column.alias)
    {
        return true;
    }
    if branch
        .core
        .iter()
        .chain(branch.opts.iter().map(|opt| &opt.scan))
        .any(|scan| {
            scan.alias == column.alias && path_scan_column(&column.column, &scan.source, dialect)
        })
    {
        return true;
    }
    branch
        .subplan_joins
        .iter()
        .filter(|join| join.alias == column.alias)
        .any(|join| {
            join.plan.branches.iter().any(|inner| {
                let distinct = if join.plan.branches.len() == 1 {
                    join.plan.distinct
                } else {
                    inner.distinct
                };
                crate::emit::source_projection(inner, distinct, dialect)
                    .iter()
                    .enumerate()
                    .any(|(index, source)| {
                        column.column.as_ref() == format!("c{index}")
                            && source
                                .as_ref()
                                .is_some_and(|source| path_column(source, inner, dialect))
                    })
            })
        })
}

fn path_scan_column(name: &str, source: &super::ScanSource, dialect: sf_sql::Dialect) -> bool {
    match source {
        super::ScanSource::Logical(_) => false,
        super::ScanSource::Path { .. } => true,
        super::ScanSource::RefAtom { input, columns } => columns.iter().enumerate()
            .any(|(index, column)| name == format!("c{index}") && path_column(column, input, dialect)),
        super::ScanSource::Projection { input, columns, .. } => columns.iter()
            .find(|(output, _)| output.as_ref() == name)
            .is_some_and(|(_, map)| match map {
                TermMap::Column(column, _) => path_scan_column(column, &input.source, dialect),
                TermMap::Template(template, _) => template.segments().iter().any(|part| {
                    matches!(part, Segment::Column(column) if path_scan_column(column, &input.source, dialect))
                }),
                _ => false,
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_part_alias_rewrite_and_guard_authority_follow_actual_columns() {
        let operand = IriOperand::Template {
            parts: vec![
                IriPart::Column(ColRef::new(0, "scheme")),
                IriPart::Literal("://host/".into()),
                IriPart::Column(ColRef::new(0, "id")),
            ],
            base: Some("http://base/".into()),
        };
        let mut comparison = IriComparison {
            left: operand.clone(),
            right: operand,
        };
        let guard = super::super::SqlCond::IriCmp(Box::new(comparison.clone()));
        assert!(atom_guard(&guard, 0));
        comparison.rewrite_columns(|c| {
            if c.column.as_ref() == "id" {
                c.alias = 4;
            }
        });
        assert_eq!(
            comparison.columns().map(|c| c.alias).collect::<Vec<_>>(),
            [0, 4, 0, 4]
        );
        assert!(!atom_guard(
            &super::super::SqlCond::IriCmp(Box::new(comparison)),
            0
        ));
    }
}
