use super::optional_work_test_support::{budget, every_stop, exact_and_short};
use super::*;
use crate::compiler_control::CompileContext;
use crate::iq::R2rmlGraphScope;
use crate::plan_measure::clone_root::{measure_copy_root, CompilerCloneRootV1};
use sf_core::ir::{LogicalSource, Template, TermMap, TermSpec};
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};
use sf_sql::Dialect;

fn constant(value: &str) -> TermDef {
    TermDef::Const(sf_core::Literal::from(value).into())
}
fn derived(alias: usize, map: TermMap) -> TermDef {
    TermDef::Derived {
        alias,
        term_map: map,
    }
}
fn mode(control: &dyn QueryControl) -> CompilerWorkMode<'_> {
    CompilerWorkMode::Metered(CompileContext::new(control))
}
fn verdict(value: Unify) -> String {
    match value {
        Unify::Sat(conditions) => format!("Sat({conditions:?})"),
        Unify::Empty => "Empty".into(),
        Unify::Unsupported(why) => format!("Unsupported({why})"),
    }
}

#[test]
fn optional_unification_exact_allowance_preserves_all_verdicts_and_stops() {
    let column = TermMap::Column("café".into(), TermSpec::iri());
    let template = TermMap::Template(
        Template::parse("http://example.test/東京/{café}/{other}").unwrap(),
        TermSpec::iri(),
    );
    let scoped = TermDef::R2rmlBlank {
        alias: 1,
        term_map: TermMap::Constant(sf_core::BlankNode::new("fixed").unwrap().into()),
        graph: R2rmlGraphScope::Mapped {
            alias: 1,
            term_map: column.clone(),
        },
    };
    let pairs = [
        (constant("東京"), constant("東京")),
        (constant("left"), constant("right")),
        (derived(1, column.clone()), derived(2, column.clone())),
        (derived(1, template.clone()), derived(2, template)),
        (
            TermDef::Coalesce(Box::new(constant("a")), Box::new(constant("b"))),
            derived(2, column),
        ),
        (scoped.clone(), scoped.clone()),
        (
            scoped,
            TermDef::Const(sf_core::BlankNode::new("fixed").unwrap().into()),
        ),
    ];
    for (left, right) in pairs {
        let raw = verdict(crate::unify::unify(&left, &right));
        assert_eq!(
            verdict(work::unify_terms(CompilerWorkMode::Uncontrolled, &left, &right).unwrap()),
            raw
        );
        let run = |control: &dyn QueryControl| {
            work::unify_terms(mode(control), &left, &right).map(verdict)
        };
        exact_and_short(raw.clone(), run);
        every_stop(run);
        let a = measure_copy_root(CompilerCloneRootV1::TermDef(&left)).unwrap();
        let b = measure_copy_root(CompilerCloneRootV1::TermDef(&right)).unwrap();
        let prefix = a.measurement_work + b.measurement_work;
        let expected = prefix + 512 + 64 * (a.deep_clone_work + b.deep_clone_work);
        let exact = budget(expected);
        assert_eq!(run(&exact).unwrap(), raw);
        assert_eq!(exact.consumed(QueryCharge::CompilerWork), expected);
        let short = budget(expected - 1);
        assert!(matches!(
            run(&short),
            Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
        ));
        assert_eq!(short.consumed(QueryCharge::CompilerWork), prefix);
    }
}

fn branch(alias: usize, term: TermDef) -> Branch {
    let mut branch = Branch::single(crate::iq::Scan {
        alias,
        source: LogicalSource::Table(format!("items_{alias}")).into(),
    });
    branch.bindings.insert("shared".into(), term);
    branch
}
// Treat a semantic refusal as an observable verdict, never hide a budget stop.
fn decision<T: std::fmt::Debug>(result: Result<T>) -> Result<String> {
    match result {
        Err(Error::QueryControl(cause)) => Err(Error::QueryControl(cause)),
        other => Ok(format!("{other:?}")),
    }
}

#[test]
fn optional_unification_match_empty_and_unsupported_caller_order_is_unchanged() {
    for term in [
        constant("a"),
        constant("b"),
        TermDef::Concat(vec![constant("a")]),
    ] {
        let left = branch(1, term);
        let right = branch(2, constant("a"));
        let run = |mode: CompilerWorkMode<'_>| {
            Ok([
                decision(build_left_join(
                    left.clone(),
                    &right,
                    None,
                    Dialect::Sqlite,
                    mode,
                ))?,
                decision(inner_join_one_with_work_mode(
                    &left,
                    &right,
                    None,
                    Dialect::Sqlite,
                    mode,
                ))?,
                decision(not_exists_cond_for_with_work_mode(
                    &left,
                    &right,
                    None,
                    Dialect::Sqlite,
                    mode,
                ))?,
            ])
        };
        let raw = run(CompilerWorkMode::Uncontrolled).unwrap();
        let controlled = |control: &dyn QueryControl| run(mode(control));
        exact_and_short(raw, controlled);
        every_stop(controlled);
    }
    // The first shared variable proves emptiness. A later unsupported binding
    // must never replace that result, in either match or anti-match lowering.
    let mut left = branch(1, constant("left"));
    let mut right = branch(2, constant("right"));
    left.bindings.insert("z".into(), TermDef::Concat(vec![]));
    right.bindings.insert("z".into(), constant("later"));
    let control = budget(u64::MAX);
    assert!(
        inner_join_one_with_work_mode(&left, &right, None, Dialect::Sqlite, mode(&control))
            .unwrap()
            .is_none()
    );
    assert!(not_exists_cond_for_with_work_mode(
        &left,
        &right,
        None,
        Dialect::Sqlite,
        mode(&control)
    )
    .unwrap()
    .is_none());
    assert_eq!(
        format!(
            "{:?}",
            build_left_join(left.clone(), &right, None, Dialect::Sqlite, mode(&control)).unwrap()
        ),
        format!("{left:?}")
    );
    assert_eq!(control.consumed(QueryCharge::SourceWork), 0);
}
