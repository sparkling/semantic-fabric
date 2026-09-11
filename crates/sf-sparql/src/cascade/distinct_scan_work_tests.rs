use super::*;
use crate::compiler_control::CompileContext;
use sf_core::query_control::{QueryBudget, QueryCharge, QueryControlError, QueryLimits};

#[test]
fn lexical_inventory_matches_raw_mixed_modes_conditions_and_exact_budget() {
    use crate::iq::iri_cmp::{IriComparison, IriOperand, IriPart};
    use sf_core::ir::{Template, TermSpec};
    let maps = [
        TermMap::Column("key".into(), TermSpec::plain_literal()),
        TermMap::Column("key".into(), TermSpec::iri().with_base("http://ex/")),
        TermMap::Column(
            "key".into(),
            TermSpec::typed_literal(sf_core::NamedNode::new_unchecked(
                "http://www.w3.org/2001/XMLSchema#integer",
            )),
        ),
        TermMap::Template(
            Template::parse("http://ex/{key}/{key}").unwrap(),
            TermSpec::iri(),
        ),
        TermMap::Template(
            Template::parse("{key}{other}").unwrap(),
            TermSpec::plain_literal(),
        ),
    ];
    let budget = |units| QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX));
    for left in &maps {
        for right in &maps {
            let mut branch = Branch::empty();
            branch.bindings.insert(
                "a".into(),
                TermDef::Derived {
                    alias: 1,
                    term_map: left.clone(),
                },
            );
            branch.bindings.insert(
                "b".into(),
                TermDef::Derived {
                    alias: 1,
                    term_map: right.clone(),
                },
            );
            branch.where_conds = vec![SqlCond::Not(Box::new(SqlCond::IriCmp(Box::new(
                IriComparison {
                    left: IriOperand::Column {
                        column: ColRef::new(1, "key"),
                        base: Some("http://other/".into()),
                    },
                    right: IriOperand::Template {
                        parts: vec![
                            IriPart::Literal("prefix".into()),
                            IriPart::Column(ColRef::new(1, "key")),
                        ],
                        base: None,
                    },
                },
            ))))];
            for include_conditions in [false, true] {
                let expected = format!("{:?}", keys_raw(&branch, 1, include_conditions));
                let run = |control: &QueryBudget| {
                    super::super::keys_with_work(
                        &branch,
                        1,
                        include_conditions,
                        BuildWork::new(crate::CompilerWorkMode::Metered(CompileContext::new(
                            control,
                        ))),
                    )
                };
                let measured = budget(u64::MAX);
                assert_eq!(format!("{:?}", run(&measured).unwrap()), expected);
                let units = measured.consumed(QueryCharge::CompilerWork);
                assert_eq!(format!("{:?}", run(&budget(units)).unwrap()), expected);
                assert!(matches!(
                    run(&budget(units - 1)),
                    Err(crate::Error::QueryControl(
                        QueryControlError::CompilerWorkExceeded
                    ))
                ));
                let stopped = budget(u64::MAX);
                stopped.terminate(QueryControlError::Cancelled);
                assert!(run(&stopped).is_err());
            }
        }
    }
}

fn keys_raw(branch: &Branch, alias: usize, with_conditions: bool) -> Vec<crate::iq::LexicalKey> {
    fn term(map: &TermMap, owner: usize, alias: usize, modes: &mut Modes) {
        if owner != alias {
            return;
        }
        let mode = match map {
            TermMap::Template(_, spec) => {
                (spec.term_type == sf_core::ir::TermType::Iri).then_some(LexicalMode::Decoded)
            }
            TermMap::Column(_, spec) if spec.term_type == sf_core::ir::TermType::Iri => {
                Some(LexicalMode::Iri {
                    base: spec.base.clone(),
                })
            }
            TermMap::Column(_, spec) if spec.term_type == sf_core::ir::TermType::BlankNode => {
                Some(LexicalMode::Decoded)
            }
            TermMap::Column(_, spec) if spec.language.is_some() => Some(LexicalMode::Decoded),
            TermMap::Column(_, spec) => spec.datatype.clone().map(|datatype| LexicalMode::TypedLiteral { datatype }),
            TermMap::Constant(_) => return,
        }.map(Consumer::Lexical).or_else(|| {
            matches!(map, TermMap::Column(_, spec) if spec.term_type == sf_core::ir::TermType::Literal
                && spec.datatype.is_none() && spec.language.is_none()).then_some(Consumer::Natural)
        });
        let mut record = |column: &str| {
            modes
                .entry(column.into())
                .and_modify(|value| match (value.as_mut(), mode.as_ref()) {
                    (Some(modes), Some(mode)) => {
                        modes.insert(mode.clone());
                    }
                    _ => *value = None,
                })
                .or_insert_with(|| mode.clone().map(|mode| BTreeSet::from([mode])));
        };
        match map {
            TermMap::Column(column, _) => record(column),
            TermMap::Template(template, _) => {
                for segment in template.segments() {
                    if let sf_core::ir::Segment::Column(column) = segment {
                        record(column);
                    }
                }
            }
            TermMap::Constant(_) => {}
        }
    }
    let mut modes = std::collections::BTreeMap::new();
    for def in branch.bindings.values() {
        match def {
            TermDef::Const(_) => {}
            TermDef::Derived {
                term_map,
                alias: owner,
            } => term(term_map, *owner, alias, &mut modes),
            TermDef::R2rmlBlank {
                term_map,
                alias: owner,
                graph,
            } => {
                term(term_map, *owner, alias, &mut modes);
                if let R2rmlGraphScope::Mapped {
                    term_map,
                    alias: owner,
                } = graph
                {
                    term(term_map, *owner, alias, &mut modes);
                }
            }
            _ => {
                for column in def
                    .columns()
                    .into_iter()
                    .filter(|column| column.alias == alias)
                {
                    modes.insert(column.column, None);
                }
            }
        }
    }
    fn condition(cond: &SqlCond, alias: usize, modes: &mut Modes) {
        match cond {
            SqlCond::IriCmp(cmp) => {
                for operand in [&cmp.left, &cmp.right] {
                    if let crate::iq::iri_cmp::IriOperand::Column { column, base } = operand {
                        let mut spec = sf_core::ir::TermSpec::iri();
                        spec.base = base.clone();
                        term(
                            &TermMap::Column(column.column.clone(), spec),
                            column.alias,
                            alias,
                            modes,
                        );
                    }
                    if let crate::iq::iri_cmp::IriOperand::Template { .. } = operand {
                        for column in operand.columns().filter(|c| c.alias == alias) {
                            modes
                                .entry(column.column.clone())
                                .and_modify(|value| {
                                    if let Some(modes) = value {
                                        modes.insert(Consumer::Lexical(LexicalMode::Decoded));
                                    }
                                })
                                .or_insert_with(|| {
                                    Some(BTreeSet::from([Consumer::Lexical(LexicalMode::Decoded)]))
                                });
                        }
                    }
                }
            }
            SqlCond::LiteralCmp(cmp) => {
                for operand in [&cmp.left, &cmp.right] {
                    if let crate::iq::literal_cmp::LiteralOperand::Column { column, spec } = operand
                    {
                        term(
                            &TermMap::Column(column.column.clone(), spec.clone()),
                            column.alias,
                            alias,
                            modes,
                        );
                    }
                }
            }
            SqlCond::And(cs)
            | SqlCond::Or(cs)
            | SqlCond::Exists { conds: cs, .. }
            | SqlCond::NotExists { conds: cs, .. }
            | SqlCond::PathExists { conds: cs, .. } => {
                for c in cs {
                    condition(c, alias, modes);
                }
            }
            SqlCond::Not(c) => condition(c, alias, modes),
            _ => {}
        }
    }
    for cond in branch
        .where_conds
        .iter()
        .chain(branch.opts.iter().flat_map(|o| o.on.iter().chain(&o.extra)))
    {
        if with_conditions {
            condition(cond, alias, &mut modes);
        }
    }
    modes
        .into_iter()
        .flat_map(|(column, modes)| {
            let modes = modes.unwrap_or_default();
            let natural = modes.contains(&Consumer::Natural);
            // A natural consumer must not erase a separate resolved-IRI key.
            // Keep the previous veto for that unqualified mixed combination.
            let veto = natural
                && modes
                    .iter()
                    .any(|m| matches!(m, Consumer::Lexical(LexicalMode::Iri { .. })));
            let natural_only = modes.len() == 1 && natural;
            let modes_have_typed = modes
                .iter()
                .any(|m| matches!(m, Consumer::Lexical(LexicalMode::TypedLiteral { .. })));
            modes
                .into_iter()
                .filter_map(move |consumer| match consumer {
                    Consumer::Natural if natural_only || modes_have_typed => {
                        Some(LexicalMode::Natural)
                    }
                    Consumer::Natural => None,
                    Consumer::Lexical(_) if veto => None,
                    Consumer::Lexical(LexicalMode::Decoded) if natural => {
                        Some(LexicalMode::DecodedWithNatural)
                    }
                    Consumer::Lexical(mode) => Some(mode),
                })
                .map(move |mode| crate::iq::LexicalKey {
                    column: column.clone(),
                    mode,
                })
        })
        .collect()
}

#[test]
fn native_inventory_matches_raw_roles_and_exact_budget() {
    let mut branch = Branch::empty();
    branch.bindings.insert(
        "x".into(),
        TermDef::Derived {
            alias: 1,
            term_map: TermMap::Column("rdf".into(), sf_core::ir::TermSpec::plain_literal()),
        },
    );
    branch.where_conds = vec![
        SqlCond::NativeColEq(ColRef::new(1, "native"), ColRef::new(2, "other")),
        SqlCond::Exists {
            scans: vec![],
            conds: vec![SqlCond::And(vec![
                SqlCond::NativeColEq(ColRef::new(1, "rdf"), ColRef::new(1, "native")),
                SqlCond::IsNotNull(ColRef::new(1, "native")),
            ])],
        },
    ];
    let expected = super::super::native_keys_raw(&branch, 1);
    assert_eq!(
        expected,
        vec![("native".into(), false), ("rdf".into(), true)]
    );
    let budget = |units| QueryBudget::new(QueryLimits::new(units, u64::MAX, u64::MAX, u64::MAX));
    let run = |control: &QueryBudget| {
        native_keys_with_work(
            &branch,
            1,
            BuildWork::new(crate::CompilerWorkMode::Metered(CompileContext::new(
                control,
            ))),
        )
    };
    let measured = budget(u64::MAX);
    assert_eq!(run(&measured).unwrap(), expected);
    let units = measured.consumed(QueryCharge::CompilerWork);
    assert_eq!(run(&budget(units)).unwrap(), expected);
    assert!(matches!(
        run(&budget(units - 1)),
        Err(crate::Error::QueryControl(
            QueryControlError::CompilerWorkExceeded
        ))
    ));
    let stopped = budget(u64::MAX);
    stopped.terminate(QueryControlError::Cancelled);
    assert!(run(&stopped).is_err());
}
