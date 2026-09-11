use super::optional_work_test_support::{budget, every_stop, exact_and_short};
use super::preparation::Preparation;
use super::*;
use crate::compiler_control::CompileContext;
use crate::iq::{AggKind, R2rmlGraphScope, Scan, SubPlanJoin};
use sf_core::ir::{LogicalSource, Template, TermMap, TermSpec};
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};
use sf_sql::Dialect;

fn mode(control: &dyn QueryControl) -> CompilerWorkMode<'_> {
    CompilerWorkMode::Metered(CompileContext::new(control))
}
fn constant() -> TermDef {
    TermDef::Const(sf_core::Literal::from("fixed").into())
}
fn derived(alias: usize) -> TermDef {
    TermDef::Derived {
        alias,
        term_map: TermMap::Column("café".into(), TermSpec::iri()),
    }
}
fn branch() -> Branch {
    let mut branch = Branch::empty();
    for alias in [1, 1] {
        branch.opts.push(OptJoin {
            scan: Scan {
                alias,
                source: LogicalSource::Table("items".into()).into(),
            },
            on: vec![],
            extra: vec![],
        });
    }
    for (alias, left) in [(2, false), (3, true)] {
        branch.subplan_joins.push(SubPlanJoin {
            alias,
            left,
            on: vec![],
            plan: Box::new(
                crate::iq::lower::lower(
                    crate::iq::node::IqNode::True,
                    Dialect::Sqlite,
                    &Default::default(),
                    &Default::default(),
                )
                .unwrap(),
            ),
        });
    }
    branch
}

#[test]
fn optional_preparation_nullable_terms_match_raw_without_column_materialization() {
    let left = branch();
    let mut terms = vec![
        constant(),
        derived(0),
        derived(1),
        derived(2),
        derived(3),
        TermDef::Derived {
            alias: 1,
            term_map: TermMap::Constant(sf_core::Literal::from("fixed").into()),
        },
        TermDef::Agg {
            col: ColRef::new(1, "count"),
            kind: AggKind::Count,
            operand: None,
            fixed_type: None,
        },
        TermDef::Agg {
            col: ColRef::new(2, "count"),
            kind: AggKind::Count,
            operand: None,
            fixed_type: None,
        },
        TermDef::Coalesce(Box::new(derived(0)), Box::new(derived(3))),
        TermDef::Concat(vec![constant(), derived(2), derived(1)]),
        TermDef::ComposedTriple {
            subject: Box::new(constant()),
            predicate: Box::new(derived(0)),
            object: Box::new(derived(3)),
        },
    ];
    for map in [
        TermMap::Constant(sf_core::BlankNode::new("fixed").unwrap().into()),
        TermMap::Template(Template::parse("only-literal").unwrap(), TermSpec::iri()),
        TermMap::Template(Template::parse("prefix/{café}").unwrap(), TermSpec::iri()),
        TermMap::Column("id".into(), TermSpec::iri()),
    ] {
        for graph in [
            R2rmlGraphScope::Default,
            R2rmlGraphScope::Mapped {
                alias: 3,
                term_map: TermMap::Column("graph".into(), TermSpec::iri()),
            },
        ] {
            terms.push(TermDef::R2rmlBlank {
                alias: 1,
                term_map: map.clone(),
                graph,
            });
        }
    }
    let expected: Vec<_> = terms
        .iter()
        .map(|term| def_is_nullable(term, &left.nullable_aliases()))
        .collect();
    let run = |control: &dyn QueryControl| {
        let prep = Preparation::new(&left, mode(control))?;
        terms
            .iter()
            .map(|term| prep.nullable(term))
            .collect::<Result<Vec<_>>>()
    };
    exact_and_short(expected, run);
    every_stop(run);
    // The correlation detector follows actual columns (including aggregate
    // output and mapped-graph columns), not the nullable detector's alias rule.
    let mut right = Branch::empty();
    right.bindings.insert("shared".into(), derived(4));
    for term in terms {
        let mut left = left.clone();
        left.bindings.insert("shared".into(), term);
        let expected = shared_reads_left_subplan(&left, &right);
        let run = |control: &dyn QueryControl| {
            Preparation::new(&left, mode(control))?.shared_reads_left_subplan(&left, &right)
        };
        exact_and_short(expected, run);
        every_stop(run);
    }
}

#[test]
fn optional_preparation_derived_correlations_preserve_funded_501_and_no_reads() {
    for alias in [0, 1, 2, 3] {
        for map in [
            TermMap::Constant(sf_core::Literal::from("fixed").into()),
            TermMap::Template(Template::parse("literal-only").unwrap(), TermSpec::iri()),
            TermMap::Template(Template::parse("prefix/{id}").unwrap(), TermSpec::iri()),
        ] {
            let mut left = branch();
            let mut right = Branch::empty();
            left.bindings.insert(
                "shared".into(),
                TermDef::Derived {
                    alias,
                    term_map: map,
                },
            );
            right.bindings.insert("shared".into(), derived(4));
            let expected = shared_reads_left_subplan(&left, &right);
            let run = |control: &dyn QueryControl| {
                Preparation::new(&left, mode(control))?.shared_reads_left_subplan(&left, &right)
            };
            exact_and_short(expected, run);
            every_stop(run);
            if expected {
                for anti in [false, true] {
                    let control = budget(u64::MAX);
                    let result = if anti {
                        not_exists_cond_for_with_work_mode(
                            &left,
                            &right,
                            None,
                            Dialect::Sqlite,
                            mode(&control),
                        )
                        .map(|_| ())
                    } else {
                        inner_join_one_with_work_mode(
                            &left,
                            &right,
                            None,
                            Dialect::Sqlite,
                            mode(&control),
                        )
                        .map(|_| ())
                    };
                    assert!(
                        matches!(result, Err(Error::Unsupported(message)) if message == SHARED_LEFT_SUBPLAN_501)
                    );
                    assert_eq!(control.consumed(QueryCharge::SourceWork), 0);
                }
            }
        }
    }
}

#[test]
fn optional_preparation_charges_empty_entry_and_every_filtered_segment() {
    let empty = Branch::empty();
    let control = budget(1);
    Preparation::new(&empty, mode(&control)).unwrap();
    assert_eq!(control.consumed(QueryCharge::CompilerWork), 1);
    assert!(Preparation::new(&empty, mode(&budget(0))).is_err());
    let mut left = branch();
    let def = TermDef::Concat((0..128).map(|_| constant()).collect());
    left.bindings.insert("shared".into(), def);
    let mut right = Branch::empty();
    right.bindings.insert("shared".into(), derived(4));
    every_stop(|control| {
        Preparation::new(&left, mode(control))?.shared_reads_left_subplan(&left, &right)
    });
    let mut nested = constant();
    for _ in 0..130 {
        nested = TermDef::Concat(vec![nested]);
    }
    let control = budget(u64::MAX);
    assert!(matches!(
        Preparation::new(&empty, mode(&control))
            .unwrap()
            .nullable(&nested),
        Err(Error::QueryControl(
            QueryControlError::CompilerEnvelopeExceeded
        ))
    ));
    assert_eq!(
        control.checkpoint(),
        Err(QueryControlError::CompilerEnvelopeExceeded)
    );
}

#[test]
fn optional_preparation_lookup_charges_utf8_comparisons_before_use() {
    let left = Branch::empty();
    let mut bindings = std::collections::BTreeMap::new();
    bindings.insert("café".into(), constant());
    // entry + lookup + comparison + five UTF-8 bytes, no owned key/value copy.
    let exact = budget(8);
    let prep = Preparation::new(&left, mode(&exact)).unwrap();
    assert!(prep.lookup(&bindings, "café").unwrap().is_some());
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), 8);
    let short = budget(7);
    assert!(Preparation::new(&left, mode(&short))
        .unwrap()
        .lookup(&bindings, "café")
        .is_err());
    assert_eq!(short.consumed(QueryCharge::CompilerWork), 3);
}
