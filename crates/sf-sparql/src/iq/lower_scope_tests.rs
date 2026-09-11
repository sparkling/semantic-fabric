use super::*;
use crate::compiler_control::CompileContext;
use crate::iq::iri_cmp::{IriComparison, IriOperand, IriPart};
use crate::leftjoin::optional_work_test_support::{budget, every_stop, exact_and_short};
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};
use sf_sql::Dialect;

fn work(control: &dyn QueryControl) -> BuildWork<'_> {
    BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control)))
}
fn run(tree: IqNode, control: &dyn QueryControl) -> Result<Plan> {
    lower_with_work_control(
        tree,
        Dialect::Sqlite,
        &Default::default(),
        &Default::default(),
        control,
    )
}
fn raw(tree: IqNode) -> Plan {
    lower(
        tree,
        Dialect::Sqlite,
        &Default::default(),
        &Default::default(),
    )
    .unwrap()
}
fn filter(cond: IqCond) -> IqNode {
    IqNode::Filter {
        child: Box::new(IqNode::True),
        cond: vec![cond],
    }
}
fn data(alias: usize) -> IqNode {
    IqNode::Extensional {
        scan: Scan {
            alias,
            source: sf_core::ir::LogicalSource::Table("items".into()).into(),
        },
        bind: BTreeMap::new(),
    }
}

#[test]
fn lower_scope_work_hand_counted_alias_and_spine_dispatches() {
    for depth in [0, 1, 8] {
        let mut tree = IqNode::True;
        for _ in 0..depth {
            tree = IqNode::Distinct {
                child: Box::new(tree),
            };
        }
        // Every alias-tree visit, one checked successor, every actual spine
        // dispatch and one empty output-scope visit, then the True leaf's
        // independently counted singleton carrier (not scope work).
        let expected = 2 * depth + 4 + base_work_tests::singleton_work();
        let control = budget(expected);
        let plan = run(tree.clone(), &control).unwrap();
        assert_eq!(format!("{plan:?}"), format!("{:?}", raw(tree.clone())));
        assert_eq!(control.consumed(QueryCharge::CompilerWork), expected);
        let short = budget(expected - 1);
        assert!(matches!(
            run(tree, &short),
            Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
        ));
    }
    let tree = filter(IqCond::Sql(SqlCond::Not(Box::new(SqlCond::IsNull(
        ColRef::new(7, "x"),
    )))));
    // Filter + True + condition slot + IqCond + Not + IsNull + successor.
    let control = budget(7);
    assert_eq!(scope::starting_alias(&tree, work(&control)).unwrap(), 8);
    assert_eq!(control.consumed(QueryCharge::CompilerWork), 7);
}

#[test]
fn lower_scope_work_aliases_cover_nested_iq_and_sql_conditions() {
    let cases = [
        filter(IqCond::Exists(Box::new(data(13)))),
        filter(IqCond::NotExists {
            inner: Box::new(data(14)),
            is_minus: true,
        }),
        filter(IqCond::Or(vec![IqCond::Not(Box::new(IqCond::Sql(
            SqlCond::IsNull(ColRef::new(15, "c")),
        )))])),
        filter(IqCond::Sql(SqlCond::Exists {
            scans: vec![Scan {
                alias: 16,
                source: sf_core::ir::LogicalSource::Table("items".into()).into(),
            }],
            conds: vec![SqlCond::NotExists {
                scans: vec![],
                conds: vec![SqlCond::IsNull(ColRef::new(17, "c"))],
            }],
        })),
    ];
    for (tree, expected) in cases.iter().zip([14, 15, 16, 18]) {
        exact_and_short(expected, |c| scope::starting_alias(tree, work(c)));
        every_stop(|c| scope::starting_alias(tree, work(c)));
    }
}

#[test]
fn lower_scope_work_visits_template_literals_even_without_columns() {
    for slots in [0, 32, 1024] {
        let tree = filter(IqCond::Sql(SqlCond::IriCmp(Box::new(IriComparison {
            left: IriOperand::Template {
                parts: vec![IriPart::Literal("literal".into()); slots],
                base: None,
            },
            right: IriOperand::Constant(
                sf_core::NamedNode::new("http://example.test/constant").unwrap(),
            ),
        }))));
        // Filter + True + condition slot + IqCond + SQL + two operands
        // + every template part (including filtered-out literals) + successor.
        let expected = 8 + slots as u64;
        let c = budget(expected);
        assert_eq!(scope::starting_alias(&tree, work(&c)).unwrap(), 1);
        assert_eq!(c.consumed(QueryCharge::CompilerWork), expected);
        exact_and_short(1, |c| scope::starting_alias(&tree, work(c)));
        if slots == 32 {
            every_stop(|c| scope::starting_alias(&tree, work(c)));
        }
    }
}

#[test]
fn lower_scope_work_scope_order_projection_copy_and_materialization() {
    let vars: Vec<Var> = vec!["z".into(), "é".into(), "a".into(), "z".into()];
    let raw_names: Vec<String> = vars.iter().map(ToString::to_string).collect();
    let expected = vars.len() * (1 + std::mem::size_of::<String>())
        + vars.iter().map(|v| 1 + v.len()).sum::<usize>();
    let c = budget(expected as u64);
    assert_eq!(
        scope::plan_vars(Some(vars.clone()), &[], work(&c)).unwrap(),
        raw_names
    );
    assert_eq!(c.consumed(QueryCharge::CompilerWork), expected as u64);
    exact_and_short(raw_names, |c| {
        scope::plan_vars(Some(vars.clone()), &[], work(c))
    });
    every_stop(|c| scope::plan_vars(Some(vars.clone()), &[], work(c)));

    let measured = crate::plan_measure::clone_root::measure_copy_collection(
        crate::plan_measure::clone_root::CompilerCloneCollectionV1::Variables(&vars),
    )
    .unwrap();
    let c = budget(measured.total_work);
    let mut target = None;
    scope::record_project(&mut target, &vars, work(&c)).unwrap();
    assert_eq!(target.as_ref().unwrap(), &vars);
    assert_eq!(c.consumed(QueryCharge::CompilerWork), measured.total_work);
    scope::record_project(&mut target, &["ignored".into()], work(&c)).unwrap();
    assert_eq!(c.consumed(QueryCharge::CompilerWork), measured.total_work);
    every_stop(|c| {
        let mut target = None;
        scope::record_project(&mut target, &vars, work(c))?;
        Ok(target)
    });
}

#[test]
fn lower_scope_work_fallback_is_sorted_unique_and_capacity_independent() {
    let mut a = Branch::empty();
    let mut b = Branch::empty();
    let term = TermDef::Const(sf_core::Literal::from("v").into());
    for name in ["z", "a", "é"] {
        a.bindings.insert(name.into(), term.clone());
    }
    for name in ["a", "b", "é"] {
        b.bindings.insert(name.into(), term.clone());
    }
    let branches = [a, b];
    let expected = visible_vars(&branches);
    assert_eq!(expected, ["a", "b", "z", "é"]);
    exact_and_short(expected, |c| scope::plan_vars(None, &branches, work(c)));
    every_stop(|c| scope::plan_vars(None, &branches, work(c)));
    let mut vars = vec!["z".into(), "a".into()];
    let a = budget(u64::MAX);
    scope::plan_vars(Some(vars.clone()), &[], work(&a)).unwrap();
    vars.reserve(1024);
    let b = budget(u64::MAX);
    scope::plan_vars(Some(vars), &[], work(&b)).unwrap();
    assert_eq!(
        a.consumed(QueryCharge::CompilerWork),
        b.consumed(QueryCharge::CompilerWork)
    );
}

#[test]
fn lower_scope_work_implicit_scope_and_nested_modifier_keep_one_identity() {
    let values = IqNode::Values {
        vars: vec!["z".into(), "a".into()],
        rows: vec![vec![None, None]],
    };
    let cases = [
        values.clone(),
        IqNode::Construction {
            child: Box::new(IqNode::Distinct {
                child: Box::new(values.clone()),
            }),
            subst: BTreeMap::new(),
            project: vec!["a".into()],
        },
        IqNode::Union {
            children: vec![
                IqNode::True,
                IqNode::OrderBy {
                    child: Box::new(IqNode::Distinct {
                        child: Box::new(IqNode::True),
                    }),
                    keys: vec![],
                },
            ],
            project: vec![],
        },
    ];
    for tree in cases {
        exact_and_short(raw(tree.clone()), |c| run(tree.clone(), c));
        every_stop(|c| run(tree.clone(), c));
    }
}

#[test]
fn lower_scope_work_checked_root_alias_overflow_is_sticky() {
    let c = budget(u64::MAX);
    assert!(matches!(
        run(data(usize::MAX), &c),
        Err(Error::QueryControl(QueryControlError::AccountingOverflow))
    ));
    assert_eq!(c.checkpoint(), Err(QueryControlError::AccountingOverflow));
    assert_eq!(c.consumed(QueryCharge::CompilerWork), 2);
    let c = budget(u64::MAX);
    let tree = filter(IqCond::Sql(SqlCond::IsNull(ColRef::new(usize::MAX, "x"))));
    assert!(matches!(
        scope::starting_alias(&tree, work(&c)),
        Err(Error::QueryControl(QueryControlError::AccountingOverflow))
    ));
}

#[test]
fn lower_scope_work_iq_and_sql_depth_reject_on_default_stack() {
    std::thread::spawn(|| {
        let max = crate::compile_envelope::algebra::MAX_ALGEBRA_DEPTH_V1;
        let mut tree = IqNode::True;
        for _ in 1..max {
            tree = IqNode::Distinct {
                child: Box::new(tree),
            };
        }
        let c = budget(u64::MAX);
        scope::starting_alias(&tree, work(&c)).unwrap();
        tree = IqNode::Distinct {
            child: Box::new(tree),
        };
        let c = budget(u64::MAX);
        assert!(matches!(
            scope::starting_alias(&tree, work(&c)),
            Err(Error::QueryControl(
                QueryControlError::CompilerEnvelopeExceeded
            ))
        ));
        assert_eq!(
            c.checkpoint(),
            Err(QueryControlError::CompilerEnvelopeExceeded)
        );
        let mut cond = SqlCond::IsNull(ColRef::new(2, "x"));
        for _ in 0..max {
            cond = SqlCond::Not(Box::new(cond));
        }
        let c = budget(u64::MAX);
        assert!(matches!(
            scope::starting_alias(&filter(IqCond::Sql(cond)), work(&c)),
            Err(Error::QueryControl(
                QueryControlError::CompilerEnvelopeExceeded
            ))
        ));
    })
    .join()
    .unwrap();
}
