use super::substitution::fold_subst;
use super::*;
use crate::compiler_control::CompileContext;
use crate::leftjoin::optional_work_test_support::{budget, every_stop, exact_and_short};
use crate::plan_measure::clone_root::{measure_copy_root, CompilerCloneRootV1};
use sf_core::query_control::{QueryCharge, QueryControl};
use spargebra::algebra::{Expression as E, Function};
use spargebra::term::Variable;

fn mode(control: &dyn QueryControl) -> CompilerWorkMode<'_> {
    CompilerWorkMode::Metered(CompileContext::new(control))
}
fn var(name: &str) -> E {
    E::Variable(Variable::new(name).unwrap())
}
fn constant(text: &str) -> TermDef {
    TermDef::Const(sf_core::Literal::from(text).into())
}
fn folded(
    subst: &BTreeMap<Var, BindDef>,
    branch: &Branch,
    mode: CompilerWorkMode<'_>,
) -> Result<String> {
    let mut branch = branch.clone();
    let result = fold_subst(subst, &mut branch, mode);
    match result {
        Err(Error::QueryControl(cause)) => Err(Error::QueryControl(cause)),
        other => Ok(format!("{other:?} / {branch:?}")),
    }
}

#[test]
fn substitution_work_resolved_copy_and_empty_entry_are_hand_counted() {
    let source = constant("a");
    let m = measure_copy_root(CompilerCloneRootV1::TermDef(&source)).unwrap();
    let subst = BTreeMap::from([("x".into(), BindDef::Resolved(source))]);
    // fold1 + entry1 + exact copy + lookup1 + empty-map carrier + key (1+1).
    let expected = 5
        + m.measurement_work
        + m.deep_clone_work
        + std::mem::size_of::<(String, TermDef)>() as u64;
    let control = budget(expected);
    folded(&subst, &Branch::empty(), mode(&control)).unwrap();
    assert_eq!(control.consumed(QueryCharge::CompilerWork), expected);
    let short = budget(expected - 1);
    assert!(folded(&subst, &Branch::empty(), mode(&short)).is_err());
    let empty = budget(1);
    assert!(fold_subst(&BTreeMap::new(), &mut Branch::empty(), mode(&empty)).unwrap());
    assert_eq!(empty.consumed(QueryCharge::CompilerWork), 1);
}

#[test]
fn substitution_work_reverse_dependency_retries_and_last_error_are_preserved() {
    let chain = BTreeMap::from([
        (
            "a".into(),
            BindDef::Expr(Box::new(E::FunctionCall(
                Function::Concat,
                vec![var("z"), E::Literal(sf_core::Literal::from("!"))],
            ))),
        ),
        ("z".into(), BindDef::Expr(Box::new(var("source")))),
        ("source".into(), BindDef::Resolved(constant("one"))),
    ]);
    let failed = BTreeMap::from([
        ("a".into(), BindDef::Expr(Box::new(var("first")))),
        ("z".into(), BindDef::Expr(Box::new(var("last")))),
    ]);
    for subst in [&chain, &failed] {
        let raw = folded(subst, &Branch::empty(), CompilerWorkMode::Uncontrolled).unwrap();
        exact_and_short(raw, |control| {
            folded(subst, &Branch::empty(), mode(control))
        });
        every_stop(|control| folded(subst, &Branch::empty(), mode(control)));
    }
    let mut branch = Branch::empty();
    assert!(fold_subst(&chain, &mut branch, mode(&budget(u64::MAX))).unwrap());
    assert_eq!(
        format!("{:?}", branch.bindings["a"]),
        format!(
            "{:?}",
            TermDef::Concat(vec![constant("one"), constant("!")])
        )
    );
    let error = fold_subst(&failed, &mut Branch::empty(), mode(&budget(u64::MAX))).unwrap_err();
    assert!(
        matches!(error, Error::Unsupported(ref message) if message == "BIND references unbound ?last")
    );
}

#[test]
fn substitution_work_disjoint_resolved_binding_prunes_before_later_expression() {
    let mut branch = Branch::empty();
    branch.bindings.insert("a".into(), constant("left"));
    let subst = BTreeMap::from([
        ("a".into(), BindDef::Resolved(constant("right"))),
        ("z".into(), BindDef::Expr(Box::new(var("never")))),
    ]);
    let raw = folded(&subst, &branch, CompilerWorkMode::Uncontrolled).unwrap();
    exact_and_short(raw, |control| folded(&subst, &branch, mode(control)));
    every_stop(|control| folded(&subst, &branch, mode(control)));
    assert!(!fold_subst(&subst, &mut branch, mode(&budget(u64::MAX))).unwrap());
}

#[test]
fn substitution_work_post_modifier_disjoint_branch_is_pruned() {
    for kind in 0..4 {
        let child = Box::new(if kind == 3 {
            // A real SQL grouping key must be column-backed; an IRI key is
            // provably disjoint from the incoming literal below.
            IqNode::Construction {
                child: Box::new(IqNode::Extensional {
                    scan: Scan {
                        alias: 1,
                        source: sf_core::ir::LogicalSource::Table("items".into()).into(),
                    },
                    bind: BTreeMap::new(),
                }),
                project: vec!["x".into()],
                subst: BTreeMap::from([(
                    "x".into(),
                    BindDef::Resolved(TermDef::Derived {
                        alias: 1,
                        term_map: sf_core::ir::TermMap::Column(
                            "id".into(),
                            sf_core::ir::TermSpec::iri(),
                        ),
                    }),
                )]),
            }
        } else {
            IqNode::Values {
                vars: vec!["x".into()],
                rows: vec![vec![Some(constant("left"))]],
            }
        });
        let modifier = match kind {
            0 => IqNode::Distinct { child },
            1 => IqNode::Slice {
                child,
                offset: 0,
                limit: Some(5),
            },
            2 => IqNode::OrderBy {
                child,
                keys: vec![],
            },
            _ => IqNode::Aggregation {
                child,
                grouping: vec!["x".into()],
                aggs: vec![],
            },
        };
        let tree = IqNode::Construction {
            child: Box::new(modifier),
            project: vec!["x".into()],
            subst: BTreeMap::from([("x".into(), BindDef::Resolved(constant("right")))]),
        };
        for work_mode in [CompilerWorkMode::Uncontrolled, mode(&budget(u64::MAX))] {
            let plan = lower_with_work_mode(
                tree.clone(),
                sf_sql::Dialect::Sqlite,
                &Default::default(),
                &Default::default(),
                work_mode,
            )
            .unwrap();
            assert!(
                plan.rust_group.is_none(),
                "must exercise non-rust-group post-modifier fold"
            );
            assert!(
                plan.branches.is_empty(),
                "unsatisfiable post-modifier branch survived: {plan:?}"
            );
        }
    }
}
