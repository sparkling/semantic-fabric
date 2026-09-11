use super::*;
use crate::build::control::BuildWork;
use crate::compiler_control::CompileContext;
use crate::iq::*;
use crate::leftjoin::optional_work_test_support::{budget, every_stop, exact_and_short};
use crate::{CompilerWorkMode, Error};
use sf_core::ir::{LogicalSource, Template, TermSpec};
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};
use sf_sql::Dialect;

fn mode(control: &dyn QueryControl) -> CompilerWorkMode<'_> {
    CompilerWorkMode::Metered(CompileContext::new(control))
}
fn path() -> PathClosure {
    PathClosure {
        alias: 7,
        kind: PathKind::One,
        hop: HopExpr::Pred(HopRelation {
            source: LogicalSource::Table("edges".into()),
            subj_col: "a".into(),
            obj_col: "b".into(),
        }),
    }
}
fn condition(alias: usize, name: &str) -> SqlCond {
    SqlCond::IriCmp(Box::new(IriComparison {
        left: IriOperand::Template {
            parts: vec![
                IriPart::Literal("prefix".into()),
                IriPart::Column(ColRef::new(alias, name)),
            ],
            base: None,
        },
        right: IriOperand::Constant(NamedNode::new("urn:example").unwrap()),
    }))
}
fn derived(alias: usize, name: &str) -> TermDef {
    TermDef::Derived {
        alias,
        term_map: TermMap::Column(name.into(), TermSpec::iri()),
    }
}
fn scan(source: ScanSource) -> Scan {
    Scan { alias: 7, source }
}
fn projected(source: ScanSource, map: TermMap) -> ScanSource {
    ScanSource::Projection {
        input: Box::new(scan(source)),
        columns: vec![("out".into(), map)],
        guards: vec![],
        distinct: false,
        native_keys: vec![],
        lexical_keys: vec![],
    }
}
fn verdict(result: crate::Result<()>) -> crate::Result<Option<String>> {
    match result {
        Ok(()) => Ok(None),
        Err(Error::Unsupported(s)) => Ok(Some(s)),
        Err(e) => Err(e),
    }
}
fn check_source(branch: &Branch, condition: &SqlCond, dialect: Dialect) {
    let raw = validate_filter_source(condition, branch, dialect).err();
    let run = |control: &dyn QueryControl| {
        verdict(validate_filter_source_with_work_mode(
            condition,
            branch,
            dialect,
            mode(control),
        ))
    };
    exact_and_short(raw, run);
    every_stop(run);
}

#[test]
fn optional_filter_source_preserves_wrappers_scopes_and_refusal_order() {
    let mut direct = Branch::empty();
    direct.path = Some(path());
    let mut logical = Branch::single(scan(LogicalSource::Table("items".into()).into()));
    logical.bindings.insert("key".into(), derived(7, "key"));
    let mut cases = vec![(direct.clone(), "sf_s"), (logical, "key")];
    let path_source = ScanSource::Path {
        closure: Box::new(path()),
        cte_alias: 11,
    };
    cases.push((Branch::single(scan(path_source.clone())), "key"));
    let reference = ScanSource::RefAtom {
        input: Box::new(direct),
        columns: vec![ColRef::new(7, "sf_s")],
    };
    cases.push((Branch::single(scan(reference.clone())), "c0"));
    cases.push((Branch::single(scan(reference.clone())), "c00"));
    for map in [
        TermMap::Column("c0".into(), TermSpec::iri()),
        TermMap::Template(Template::parse("http://x/{c0}").unwrap(), TermSpec::iri()),
        TermMap::Constant(NamedNode::new("urn:constant").unwrap().into()),
    ] {
        cases.push((
            Branch::single(scan(projected(reference.clone(), map))),
            "out",
        ));
    }
    let mut optional = Branch::empty();
    optional.opts.push(OptJoin {
        scan: scan(path_source),
        on: vec![],
        extra: vec![],
    });
    cases.push((optional, "key"));
    for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
        for (branch, name) in &cases {
            for cond in [
                condition(7, name),
                condition(99, name),
                SqlCond::Not(Box::new(condition(7, name))),
                SqlCond::And(vec![SqlCond::ExpressionError, condition(7, name)]),
                SqlCond::Or(vec![condition(7, name), condition(99, name)]),
            ] {
                check_source(branch, &cond, dialect);
            }
        }
    }
    // .find selects the first output recipe, even if a later duplicate is a path.
    let mut source = projected(
        reference,
        TermMap::Constant(NamedNode::new("urn:c").unwrap().into()),
    );
    if let ScanSource::Projection { columns, .. } = &mut source {
        columns.push(("out".into(), TermMap::Column("c0".into(), TermSpec::iri())));
    }
    check_source(
        &Branch::single(scan(source)),
        &condition(7, "out"),
        Dialect::Sqlite,
    );
}

#[test]
fn optional_filter_projection_matches_binding_condition_and_aggregate_layouts() {
    let mut branch = Branch::empty();
    let template = TermMap::Template(
        Template::parse("prefix/{a}/{b}/{a}").unwrap(),
        TermSpec::iri(),
    );
    let mut defs = vec![
        TermDef::Const(NamedNode::new("urn:c").unwrap().into()),
        derived(7, "a"),
        TermDef::Derived {
            alias: 7,
            term_map: template.clone(),
        },
        TermDef::R2rmlBlank {
            alias: 7,
            term_map: template.clone(),
            graph: R2rmlGraphScope::Mapped {
                alias: 8,
                term_map: template,
            },
        },
        TermDef::Coalesce(Box::new(derived(7, "a")), Box::new(derived(8, "z"))),
        TermDef::Concat(vec![derived(8, "z"), derived(9, "q")]),
        TermDef::Agg {
            col: ColRef::new(10, "agg"),
            kind: AggKind::Avg,
            operand: Some(ColRef::new(88, "not_projected")),
            fixed_type: None,
        },
    ];
    defs.push(TermDef::ComposedTriple {
        subject: Box::new(derived(7, "a")),
        predicate: Box::new(derived(11, "p")),
        object: Box::new(TermDef::Concat(defs.clone())),
    });
    for (i, def) in defs.into_iter().enumerate() {
        branch.bindings.insert(format!("v{i}"), def);
    }
    branch.where_conds = vec![
        condition(12, "iri"),
        SqlCond::ColEq(ColRef::new(7, "a"), ColRef::new(12, "eq")),
        SqlCond::Not(Box::new(SqlCond::IsNull(ColRef::new(12, "null")))),
        SqlCond::And(vec![SqlCond::NativeCmp(
            ColRef::new(13, "native"),
            CmpOp::Eq,
            "x".into(),
        )]),
        SqlCond::TemplateEq(
            vec![
                Segment::Literal("prefix".into()),
                Segment::Column("a".into()),
            ],
            7,
            vec![Segment::Column("template".into())],
            13,
            true,
        ),
        SqlCond::Exists {
            scans: vec![],
            conds: vec![SqlCond::IsNull(ColRef::new(99, "opaque"))],
        },
    ];
    branch.opts.push(OptJoin {
        scan: scan(LogicalSource::Table("items".into()).into()),
        on: vec![SqlCond::DecodedIsNotNull(ColRef::new(14, "on"))],
        extra: vec![SqlCond::StrMatch {
            col: ColRef::new(15, "extra"),
            op: StrMatchOp::Like,
            param: "%x%".into(),
        }],
    });
    let ordinary = branch.clone();
    branch.agg = Some(Aggregation {
        keys: vec![GroupKey {
            var: "key".into(),
            cols: vec![ColRef::new(7, "a"), ColRef::new(7, "a")],
        }],
        aggs: vec![
            AggCol {
                var: "avg".into(),
                kind: AggKind::Avg,
                arg: Some(ColRef::new(7, "operand")),
                distinct: false,
                out: ColRef::new(90, "avg"),
                fixed_type: None,
            },
            AggCol {
                var: "count".into(),
                kind: AggKind::Count,
                arg: None,
                distinct: false,
                out: ColRef::new(90, "count"),
                fixed_type: None,
            },
        ],
    });
    let aggregate = branch.clone();
    branch.path = Some(path()); // Path wins over aggregate, exactly as emission.
    for branch in [ordinary, aggregate, branch] {
        for distinct in [false, true] {
            for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
                let raw = crate::emit::source_projection(&branch, distinct, dialect)
                    .iter()
                    .map(|c| c.as_ref().map(|c| (c.alias, c.column.to_string())))
                    .collect::<Vec<_>>();
                let run = |control: &dyn QueryControl| {
                    filter_projection::projection(
                        &branch,
                        distinct,
                        dialect,
                        BuildWork::new(mode(control)),
                    )
                    .map(|cols| {
                        cols.into_iter()
                            .map(|c| c.map(|c| (c.alias, c.name.to_string())))
                            .collect::<Vec<_>>()
                    })
                };
                exact_and_short(raw, run);
                // Run charge-by-charge once per layout, not per dialect/profile.
                if !distinct && dialect == Dialect::Sqlite {
                    every_stop(run);
                }
            }
        }
    }
}

#[test]
fn optional_filter_position_and_nested_depth_are_paid_and_bounded() {
    for name in [
        "c0",
        "c1",
        "c00",
        "c+0",
        "c-0",
        "c01",
        "c١",
        "c184467440737095516160",
        "c",
        "",
    ] {
        for index in [0, 1, usize::MAX] {
            let raw = name == format!("c{index}");
            let run = |control: &dyn QueryControl| {
                work::position(name, index, BuildWork::new(mode(control)))
            };
            exact_and_short(raw, run);
            every_stop(run);
        }
    }
    let mut source = LogicalSource::Table("items".into()).into();
    for _ in 0..130 {
        source = projected(source, TermMap::Column("out".into(), TermSpec::iri()));
    }
    let branch = Branch::single(scan(source));
    let control = budget(u64::MAX);
    assert!(matches!(
        validate_filter_source_with_work_mode(
            &condition(7, "out"),
            &branch,
            Dialect::Sqlite,
            mode(&control)
        ),
        Err(Error::QueryControl(
            QueryControlError::CompilerEnvelopeExceeded
        ))
    ));
    assert_eq!(
        control.checkpoint(),
        Err(QueryControlError::CompilerEnvelopeExceeded)
    );
    assert_eq!(control.consumed(QueryCharge::SourceWork), 0);
}

fn subplan(branches: Vec<Branch>, distinct: bool) -> Branch {
    let mut outer = Branch::empty();
    outer.subplan_joins.push(SubPlanJoin {
        alias: 4,
        left: false,
        on: vec![],
        plan: Box::new(crate::Plan {
            branches,
            form: crate::PlanForm::Ask,
            distinct,
            limit: None,
            offset: 0,
            order: vec![],
            rust_group: None,
            dialect: Dialect::Sqlite,
            dedup_scopes: vec![],
            construct_drops_some_branch_var: false,
        }),
    });
    outer
}

#[test]
fn optional_filter_nested_subplans_keep_distinct_and_avg_source_positions() {
    let mut inner = Branch::single(scan(ScanSource::Path {
        closure: Box::new(path()),
        cte_alias: 11,
    }));
    inner.bindings.insert("logical".into(), derived(99, "id"));
    inner
        .where_conds
        .push(SqlCond::IsNotNull(ColRef::new(7, "sf_s")));
    // Root DISTINCT overrides one inner branch, but not multiple UNION arms.
    for (branches, distinct, expected) in [
        (vec![inner.clone()], true, false),
        (vec![inner.clone()], false, true),
        (vec![inner.clone(); 2], true, true),
    ] {
        let outer = subplan(branches, distinct);
        let cond = condition(4, "c1");
        assert_eq!(
            validate_filter_source(&cond, &outer, Dialect::Sqlite).is_err(),
            expected
        );
        check_source(&outer, &cond, Dialect::Sqlite);
    }
    inner.agg = Some(Aggregation {
        keys: vec![GroupKey {
            var: "key".into(),
            cols: vec![ColRef::new(99, "id")],
        }],
        aggs: vec![AggCol {
            var: "avg".into(),
            kind: AggKind::Avg,
            arg: Some(ColRef::new(7, "sf_s")),
            distinct: false,
            out: ColRef::new(90, "avg"),
            fixed_type: None,
        }],
    });
    let outer = subplan(vec![inner], false);
    for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
        for index in 0..4 {
            check_source(&outer, &condition(4, &format!("c{index}")), dialect);
        }
        assert_eq!(
            validate_filter_source(&condition(4, "c2"), &outer, dialect).is_err(),
            dialect == Dialect::Sqlite
        );
    }
    // Nested source recursion shares a depth boundary, even though each layout
    // itself is tiny; do not reset it when entering another derived plan.
    let mut nested = Branch::empty();
    nested.bindings.insert("v".into(), derived(99, "id"));
    for _ in 0..130 {
        nested = subplan(vec![nested], false);
        nested.bindings.insert("v".into(), derived(4, "c0"));
    }
    let control = budget(u64::MAX);
    assert!(matches!(
        validate_filter_source_with_work_mode(
            &condition(4, "c0"),
            &nested,
            Dialect::Sqlite,
            mode(&control)
        ),
        Err(Error::QueryControl(
            QueryControlError::CompilerEnvelopeExceeded
        ))
    ));
}
