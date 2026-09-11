use super::*;
use crate::compiler_control::CompileContext;
use crate::leftjoin::optional_work_test_support::{budget, every_stop, exact_and_short};
use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_core::query_control::{QueryCharge, QueryControl};
use sf_sql::Dialect;

fn values(n: usize) -> IqNode {
    IqNode::Values {
        vars: vec![],
        rows: vec![vec![]; n],
    }
}
fn data(alias: usize) -> IqNode {
    IqNode::Construction {
        child: Box::new(IqNode::Extensional {
            scan: Scan {
                alias,
                source: LogicalSource::Table(format!("items_{alias}")).into(),
            },
            bind: BTreeMap::new(),
        }),
        subst: BTreeMap::from([(
            "value".into(),
            BindDef::Resolved(TermDef::Derived {
                alias,
                term_map: TermMap::Column("value".into(), TermSpec::iri()),
            }),
        )]),
        project: vec!["value".into()],
    }
}
fn optional(left: IqNode, right: IqNode) -> IqNode {
    IqNode::LeftJoin {
        left: Box::new(left),
        right: Box::new(right),
        cond: vec![],
    }
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
fn variants() -> Vec<IqNode> {
    vec![
        optional(values(3), data(2)),
        optional(
            values(3),
            IqNode::Union {
                children: vec![data(2), data(3)],
                project: vec!["value".into()],
            },
        ),
        optional(values(3), optional(values(1), data(2))),
        optional(
            values(3),
            IqNode::Distinct {
                child: Box::new(data(2)),
            },
        ),
        IqNode::Union {
            children: vec![
                IqNode::True,
                IqNode::Distinct {
                    child: Box::new(optional(values(2), data(2))),
                },
            ],
            project: vec!["value".into()],
        },
    ]
}

#[test]
fn optional_work_lower_dispatches_fast_decomposed_nested_and_subplan_with_one_identity() {
    for tree in variants() {
        let raw = lower(
            tree.clone(),
            Dialect::Sqlite,
            &Default::default(),
            &Default::default(),
        )
        .unwrap();
        exact_and_short(raw, |control| run(tree.clone(), control));
    }
}

#[test]
fn optional_work_lower_each_charge_retains_sticky_cancel_and_deadline() {
    for tree in variants() {
        every_stop(|control| run(tree.clone(), control));
    }
}

#[test]
fn optional_work_lower_dispatch_cost_matches_the_independently_selected_helper() {
    let right = Branch {
        bindings: BTreeMap::from([(
            "value".into(),
            TermDef::Derived {
                alias: 2,
                term_map: TermMap::Column("value".into(), TermSpec::iri()),
            },
        )]),
        ..Branch::single(Scan {
            alias: 2,
            source: LogicalSource::Table("items_2".into()).into(),
        })
    };
    for decompose in [false, true] {
        let expected = budget(u64::MAX);
        let mode = CompilerWorkMode::Metered(CompileContext::new(&expected));
        let expected_branches = if decompose {
            left_join_decomposed(
                vec![Branch::empty(); 3],
                vec![right.clone()],
                None,
                Dialect::Sqlite,
                mode,
            )
        } else {
            left_join_branches(
                vec![Branch::empty(); 3],
                vec![right.clone()],
                None,
                Dialect::Sqlite,
                mode,
            )
        }
        .unwrap();
        let actual = budget(u64::MAX);
        let branches = lower_node(
            optional(values(3), data(2)),
            Dialect::Sqlite,
            decompose,
            &mut 3,
            &Default::default(),
            &Default::default(),
            CompilerWorkMode::Metered(CompileContext::new(&actual)),
        )
        .unwrap();
        assert_eq!(format!("{branches:?}"), format!("{expected_branches:?}"));
        // The data(2) Construction establishes exactly one borrowed binding:
        // fold1 + entry1 + copy + lookup1 + map carrier + key string(1+5).
        let copied = crate::plan_measure::clone_root::measure_copy_root(
            crate::plan_measure::clone_root::CompilerCloneRootV1::TermDef(&right.bindings["value"]),
        )
        .unwrap()
        .total_work;
        // One output slot/carrier plus retention entry1, bool slot/byte2,
        // key visit1, ?value comparison6 and retain visit1.
        let projection = 1 + std::mem::size_of::<Branch>() as u64 + 11;
        let construction =
            9 + copied + std::mem::size_of::<(String, TermDef)>() as u64 + projection;
        assert_eq!(
            actual.consumed(QueryCharge::CompilerWork),
            expected.consumed(QueryCharge::CompilerWork)
                + construction
                + super::base_work_tests::empty_rows_work(3)
                + super::base_work_tests::scan_work()
        );
        assert!(actual.consumed(QueryCharge::CompilerWork) > 0);
    }

    // Isolate existing SubPlan preparation from the OPTIONAL attachment. The
    // top-level dispatcher must pay BOTH, not merely the pre-existing copies.
    let subplan = IqNode::Distinct {
        child: Box::new(data(2)),
    };
    let expected = budget(u64::MAX);
    let mode = CompilerWorkMode::Metered(CompileContext::new(&expected));
    let right = lower_node(
        subplan.clone(),
        Dialect::Sqlite,
        true,
        &mut 3,
        &Default::default(),
        &Default::default(),
        mode,
    )
    .unwrap();
    let preparation = expected.consumed(QueryCharge::CompilerWork);
    left_join_over_subplan(vec![Branch::empty(); 3], &right[0], Dialect::Sqlite, mode).unwrap();
    assert!(expected.consumed(QueryCharge::CompilerWork) > preparation);
    let actual = budget(u64::MAX);
    let tree = optional(values(3), subplan);
    let (prefix, tail) = scope_test_support::entry_work(&tree);
    run(tree, &actual).unwrap();
    assert_eq!(
        actual.consumed(QueryCharge::CompilerWork),
        expected.consumed(QueryCharge::CompilerWork)
            + prefix
            + tail
            + super::base_work_tests::empty_rows_work(3)
    );
}

#[test]
fn optional_work_existing_sliced_subplan_rejection_is_unchanged() {
    let tree = optional(
        values(3),
        IqNode::Slice {
            child: Box::new(data(2)),
            offset: 0,
            limit: Some(2),
        },
    );
    let raw = lower(
        tree.clone(),
        Dialect::Sqlite,
        &Default::default(),
        &Default::default(),
    );
    let controlled = run(tree, &budget(u64::MAX));
    assert!(matches!(raw, Err(Error::Unsupported(_))));
    assert_eq!(format!("{raw:?}"), format!("{controlled:?}"));
}

#[test]
fn optional_work_forced_decomposition_moves_one_original_left_into_the_tail() {
    let left = Branch::single(Scan {
        alias: 1,
        source: LogicalSource::Table("left".into()).into(),
    });
    let original = left.core.as_ptr();
    let right = Branch::single(Scan {
        alias: 2,
        source: LogicalSource::Table("right".into()).into(),
    });
    let control = budget(u64::MAX);
    let out = left_join_decomposed(
        vec![left],
        vec![right],
        None,
        Dialect::Sqlite,
        CompilerWorkMode::Metered(CompileContext::new(&control)),
    )
    .unwrap();
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|b| b.opts.is_empty()));
    assert_eq!(out[1].core.as_ptr(), original);
    assert_eq!(out[1].where_conds.len(), 1);
    assert!(control.consumed(QueryCharge::CompilerWork) > 3);
}

#[test]
fn optional_work_subplan_output_retains_exact_nested_plan_and_nullable_left_copies() {
    let plan = lower(
        data(2),
        Dialect::Sqlite,
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    let mut right = Branch::empty();
    right.subplan_joins.push(SubPlanJoin {
        alias: 10,
        plan: Box::new(plan),
        left: false,
        on: vec![],
    });
    right.bindings.insert(
        "shared".into(),
        TermDef::Derived {
            alias: 10,
            term_map: TermMap::Column("c0".into(), TermSpec::iri()),
        },
    );
    let mut left = Branch::empty();
    left.opts.push(crate::iq::OptJoin {
        scan: Scan {
            alias: 1,
            source: LogicalSource::Table("left".into()).into(),
        },
        on: vec![],
        extra: vec![],
    });
    left.bindings.insert(
        "shared".into(),
        TermDef::Derived {
            alias: 1,
            term_map: TermMap::Column("value".into(), TermSpec::iri()),
        },
    );
    let raw = left_join_over_subplan(
        vec![left.clone(); 2],
        &right,
        Dialect::Sqlite,
        CompilerWorkMode::Uncontrolled,
    )
    .unwrap();
    assert!(matches!(raw[0].bindings["shared"], TermDef::Coalesce(_, _)));
    exact_and_short(raw, |control| {
        left_join_over_subplan(
            vec![left.clone(); 2],
            &right,
            Dialect::Sqlite,
            CompilerWorkMode::Metered(CompileContext::new(control)),
        )
    });
    every_stop(|control| {
        left_join_over_subplan(
            vec![left.clone(); 2],
            &right,
            Dialect::Sqlite,
            CompilerWorkMode::Metered(CompileContext::new(control)),
        )
    });
}
