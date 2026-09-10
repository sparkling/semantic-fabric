use super::*;
use crate::iq::iri_cmp::{IriComparison, IriOperand};

pub(super) fn operand(map: &TermMap, alias: usize) -> Option<IriOperand> {
    IriOperand::from_map(map, alias)
}

pub(super) fn static_constant(value: &Term, map: &TermMap, alias: usize) -> Option<Unify> {
    let (Term::NamedNode(iri), TermMap::Template(template, spec)) = (value, map) else {
        return None;
    };
    if spec.term_type != TermType::Iri || spec.base.is_some() {
        return None;
    }
    let TemplateShape::SingleSlot { prefix, suffix, .. } = split_template(template) else {
        return None;
    };
    let want = iri.as_str();
    if want.len() < prefix.len() + suffix.len()
        || !want.starts_with(&prefix)
        || !want.ends_with(&suffix)
    {
        return Some(Unify::Empty);
    }
    Some(Unify::Sat(vec![identity(
        operand(map, alias).unwrap(),
        IriOperand::Constant(iri.clone()),
    )]))
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
    dialect: Dialect,
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
    let template_constant = |template: &IriOperand, constant: &IriOperand| {
        // Forward identity encodes every slot; unlike inverse template matching,
        // it need not recover a unique source value from the constant. An outer
        // MySQL filter pushed into a rendered UNION may encounter multiple slots.
        // Other dialects retain the existing single-slot admission boundary.
        matches!(
            (template, constant),
            (
                IriOperand::Template { parts, base: None },
                IriOperand::Constant(_)
            )
            if dialect == Dialect::MySql
                || parts.iter().filter(|part| matches!(part, crate::iq::iri_cmp::IriPart::Column(_))).count() == 1
        )
    };
    if [&left, &right].iter().all(|operand| {
        matches!(
            operand,
            IriOperand::Template { base: None, .. } | IriOperand::Constant(_)
        )
    }) && [&left, &right]
        .iter()
        .any(|op| matches!(op, IriOperand::Template { .. }))
        && !template_constant(&left, &right)
        && !template_constant(&right, &left)
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

#[cfg(test)]
mod tests {
    use super::*;
    fn map(template: &str, spec: TermSpec) -> TermMap {
        TermMap::Template(sf_core::ir::Template::parse(template).unwrap(), spec)
    }
    #[test]
    fn static_gate_keeps_disjoint_kind_base_and_multi_slot_boundaries() {
        let value = Term::NamedNode(sf_core::NamedNode::new_unchecked("http://ex/a%2Fb/end"));
        let template = map("http://ex/{id}/end", TermSpec::iri());
        assert!(
            matches!(static_constant(&value, &template, 4), Some(Unify::Sat(parts)) if matches!(parts.as_slice(), [SqlCond::IriCmp(_)]))
        );
        for recipe in ["http://other/{id}/end", "http://ex/{id}/different"] {
            assert!(matches!(
                static_constant(&value, &map(recipe, TermSpec::iri()), 4),
                Some(Unify::Empty)
            ));
        }
        assert!(
            static_constant(&value, &map("http://ex/{a}-{b}/end", TermSpec::iri()), 4).is_none()
        );
        assert!(static_constant(
            &value,
            &map("http://ex/{id}/end", TermSpec::plain_literal()),
            4
        )
        .is_none());
        let mut late = TermSpec::iri();
        late.base = Some("http://base/".into());
        assert!(static_constant(&value, &map("{id}:end", late), 4).is_none());
    }
    #[test]
    fn static_template_pair_does_not_acquire_the_constant_profile() {
        let bindings = ["a", "b"]
            .into_iter()
            .enumerate()
            .map(|(alias, name)| {
                (
                    name.into(),
                    TermDef::Derived {
                        term_map: map("http://ex/{id}", TermSpec::iri()),
                        alias,
                    },
                )
            })
            .collect();
        let variable = |name| Expression::Variable(Variable::new(name).unwrap());
        for dialect in [Dialect::Sqlite, Dialect::MySql, Dialect::Postgres] {
            assert!(filter(
                &variable("a"),
                &variable("b"),
                CmpOp::Eq,
                &bindings,
                dialect
            )
            .is_none());
        }
    }

    #[test]
    fn multi_slot_constant_filter_uses_forward_identity_in_both_directions() {
        let bindings = BTreeMap::from([(
            "o".into(),
            TermDef::Derived {
                term_map: map("http://ex/{a}/{b}", TermSpec::iri()),
                alias: 3,
            },
        )]);
        let variable = Expression::Variable(Variable::new("o").unwrap());
        let constant = Expression::NamedNode(sf_core::NamedNode::new("http://ex/a%2Fb/").unwrap());
        for (a, b) in [(&variable, &constant), (&constant, &variable)] {
            assert!(matches!(
                filter(a, b, CmpOp::Eq, &bindings, Dialect::MySql),
                Some(SqlCond::IriCmp(_))
            ));
            assert!(matches!(
                filter(a, b, CmpOp::Ne, &bindings, Dialect::MySql),
                Some(SqlCond::Not(_))
            ));
            assert!(matches!(
                filter(a, b, CmpOp::Lt, &bindings, Dialect::MySql),
                Some(SqlCond::ExpressionError)
            ));
            for dialect in [Dialect::Sqlite, Dialect::Postgres] {
                for op in [CmpOp::Eq, CmpOp::Ne, CmpOp::Lt] {
                    assert!(filter(a, b, op, &bindings, dialect).is_none());
                }
            }
        }
    }

    #[test]
    fn single_slot_constant_filter_remains_available_across_dialects() {
        let bindings = BTreeMap::from([(
            "o".into(),
            TermDef::Derived {
                term_map: map("http://ex/{a}", TermSpec::iri()),
                alias: 3,
            },
        )]);
        let variable = Expression::Variable(Variable::new("o").unwrap());
        let constant = Expression::NamedNode(sf_core::NamedNode::new("http://ex/a").unwrap());
        for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
            for (a, b) in [(&variable, &constant), (&constant, &variable)] {
                assert!(matches!(
                    filter(a, b, CmpOp::Eq, &bindings, dialect),
                    Some(SqlCond::IriCmp(_))
                ));
            }
        }
    }
}
