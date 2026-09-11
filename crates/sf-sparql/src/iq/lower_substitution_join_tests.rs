use super::substitution::insert_or_unify;
use super::*;
use crate::compiler_control::CompileContext;
use crate::leftjoin::optional_work_test_support::{budget, every_stop, exact_and_short};
use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_core::query_control::QueryControl;

fn derived(alias: usize) -> TermDef {
    TermDef::Derived {
        alias,
        term_map: TermMap::Column("value".into(), TermSpec::iri()),
    }
}
fn fixture(optional: &[usize], subplan: Option<(usize, bool)>) -> Branch {
    let mut b = Branch::empty();
    b.bindings.insert("x".into(), derived(1));
    for alias in optional {
        b.opts.push(crate::iq::OptJoin {
            scan: Scan {
                alias: *alias,
                source: LogicalSource::Table("items".into()).into(),
            },
            on: vec![],
            extra: vec![],
        });
    }
    if let Some((alias, left)) = subplan {
        b.subplan_joins.push(SubPlanJoin {
            alias,
            left,
            on: vec![],
            plan: Box::new(
                lower(
                    IqNode::True,
                    sf_sql::Dialect::Sqlite,
                    &Default::default(),
                    &Default::default(),
                )
                .unwrap(),
            ),
        });
    }
    b
}
fn run(branch: &Branch, td: &TermDef, mode: CompilerWorkMode<'_>) -> Result<String> {
    let mut branch = branch.clone();
    let result = insert_or_unify(&mut branch, &"x".into(), td.clone(), mode);
    match result {
        Err(Error::QueryControl(cause)) => Err(Error::QueryControl(cause)),
        other => Ok(format!("{other:?} / {branch:?}")),
    }
}

#[test]
fn substitution_shared_bindings_preserve_nullable_subplan_and_unify_verdicts() {
    for (branch, incoming, expected) in [
        (fixture(&[], None), derived(2), "Ok(false)"),
        (fixture(&[1], None), derived(2), "IsNull"),
        (fixture(&[2], None), derived(2), "IsNull"),
        (fixture(&[1, 2], None), derived(2), "TWO OPTIONALs"),
        (fixture(&[], Some((1, true))), derived(2), "LEFT-JOINed"),
        (fixture(&[], Some((1, false))), derived(2), "Ok(false)"),
        (fixture(&[], Some((1, true))), derived(1), "LEFT-JOINed"),
    ] {
        let raw = run(&branch, &incoming, CompilerWorkMode::Uncontrolled).unwrap();
        assert!(raw.contains(expected), "{raw}");
        let metered = |control: &dyn QueryControl| {
            run(
                &branch,
                &incoming,
                CompilerWorkMode::Metered(CompileContext::new(control)),
            )
        };
        exact_and_short(raw, metered);
        every_stop(metered);
    }
    let mut branch = fixture(&[1], None);
    insert_or_unify(
        &mut branch,
        &"x".into(),
        derived(2),
        CompilerWorkMode::Metered(CompileContext::new(&budget(u64::MAX))),
    )
    .unwrap();
    assert!(matches!(
        branch.bindings["x"],
        TermDef::Derived { alias: 2, .. }
    ));
}

#[test]
fn substitution_left_subplan_scan_matches_actual_columns_without_materialization() {
    let constant = TermDef::Const(sf_core::Literal::from("fixed").into());
    let terms = [
        constant.clone(),
        derived(1),
        derived(2),
        TermDef::Derived {
            alias: 1,
            term_map: TermMap::Constant(sf_core::Literal::from("fixed").into()),
        },
        TermDef::Derived {
            alias: 1,
            term_map: TermMap::Template(
                sf_core::ir::Template::parse("http://example.test/fixed").unwrap(),
                TermSpec::iri(),
            ),
        },
        TermDef::Derived {
            alias: 1,
            term_map: TermMap::Template(
                sf_core::ir::Template::parse("http://example.test/{id}").unwrap(),
                TermSpec::iri(),
            ),
        },
        TermDef::Coalesce(Box::new(constant.clone()), Box::new(derived(1))),
        TermDef::Concat(vec![constant.clone(), derived(1)]),
        TermDef::R2rmlBlank {
            alias: 2,
            term_map: TermMap::Constant(sf_core::Literal::from("fixed").into()),
            graph: R2rmlGraphScope::Mapped {
                alias: 1,
                term_map: TermMap::Column("graph".into(), TermSpec::iri()),
            },
        },
        TermDef::R2rmlBlank {
            alias: 1,
            term_map: TermMap::Column("blank".into(), TermSpec::iri()),
            graph: R2rmlGraphScope::Default,
        },
        TermDef::Agg {
            col: ColRef::new(1, "count"),
            kind: crate::iq::AggKind::Count,
            operand: None,
            fixed_type: None,
        },
        TermDef::ComposedTriple {
            subject: Box::new(constant.clone()),
            predicate: Box::new(constant),
            object: Box::new(derived(1)),
        },
    ];
    for subplan in [None, Some((1, false)), Some((1, true)), Some((2, true))] {
        let branch = fixture(&[], subplan);
        for term in &terms {
            let columns = term.columns();
            let expected = branch
                .subplan_joins
                .iter()
                .any(|sp| sp.left && columns.iter().any(|c| c.alias == sp.alias));
            let run = |control: &dyn QueryControl| {
                super::substitution::reads_left_subplan(
                    &branch,
                    term,
                    BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control))),
                )
            };
            exact_and_short(expected, run);
            every_stop(run);
        }
    }
}
