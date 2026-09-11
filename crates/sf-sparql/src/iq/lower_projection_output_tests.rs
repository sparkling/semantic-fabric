use super::projection::*;
use super::*;
use crate::build::control::BuildVec;
use crate::compiler_control::CompileContext;
use crate::leftjoin::optional_work_test_support::{budget, every_stop, exact_and_short};
use sf_core::query_control::{QueryCharge, QueryControl};

fn work(control: &dyn QueryControl) -> BuildWork<'_> {
    BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(control)))
}
fn branch(label: &str) -> Branch {
    let mut b = Branch::empty();
    b.bindings.insert(
        "x".into(),
        TermDef::Const(sf_core::Literal::from(label).into()),
    );
    b
}

#[test]
fn projection_work_construction_output_is_exact_before_movement() {
    let size = std::mem::size_of::<Branch>() as u64;
    for len in [1, 3, 12] {
        let exact = len as u64 * (1 + size);
        let control = budget(exact);
        assert!(construction_output(len, work(&control)).unwrap().is_empty());
        assert_eq!(control.consumed(QueryCharge::CompilerWork), exact);
        assert!(construction_output(len, work(&budget(exact - 1))).is_err());
        every_stop(|control| construction_output(len, work(control)));
    }
}

#[test]
fn projection_work_union_pays_growth_independent_of_allocator_slack() {
    let size = std::mem::size_of::<Branch>() as u64;
    for spare in [0, 128] {
        let run = |control: &dyn QueryControl| {
            let mut values = Vec::with_capacity(spare);
            values.push(branch("prefix"));
            let mut out = BuildVec::new(values);
            for group in [vec![], vec![branch("a")], vec![branch("b"), branch("a")]] {
                append_union(&mut out, group, work(control))?;
            }
            Ok(out.into_inner())
        };
        // Three call visits + three moved entries. Growth 1→2 pays 2+1
        // carriers; 2→4 pays 4+2; the last entry fits the paid fourth slot.
        let control = budget(6 + 9 * size);
        let result = run(&control).unwrap();
        assert_eq!(control.consumed(QueryCharge::CompilerWork), 6 + 9 * size);
        assert_eq!(
            format!("{result:?}"),
            format!(
                "{:?}",
                [branch("prefix"), branch("a"), branch("b"), branch("a")]
            )
        );
        exact_and_short(result, run);
        every_stop(run);
    }
}

#[test]
fn projection_work_construction_union_and_post_spine_keep_order_and_components() {
    let row = |label: &str| IqNode::Values {
        vars: vec!["x".into(), "component".into(), "hidden".into()],
        rows: vec![vec![
            Some(TermDef::Const(
                sf_core::Literal::from(label).into()
            ));
            3
        ]],
    };
    for post_spine in [false, true] {
        let union = IqNode::Union {
            children: vec![
                row("first"),
                IqNode::Empty { vars: vec![] },
                row("second"),
                row("first"),
            ],
            project: vec!["x".into(), "component".into(), "hidden".into()],
        };
        let child = if post_spine {
            IqNode::OrderBy {
                child: Box::new(union),
                keys: vec![],
            }
        } else {
            union
        };
        let source = IqNode::Construction {
            child: Box::new(child),
            subst: BTreeMap::new(),
            project: vec!["x".into()],
        };
        let extra = HashSet::from(["component".into()]);
        let raw = lower_with_work_mode(
            source.clone(),
            sf_sql::Dialect::Sqlite,
            &extra,
            &Default::default(),
            CompilerWorkMode::Uncontrolled,
        )
        .unwrap();
        assert!(matches!(&raw.form, PlanForm::Select { vars } if vars == &["x"]));
        assert_eq!(raw.branches.len(), 3);
        for branch in &raw.branches {
            assert_eq!(
                branch
                    .bindings
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                ["component", "x"]
            );
        }
        let run = |control: &dyn QueryControl| {
            lower_with_work_mode(
                source.clone(),
                sf_sql::Dialect::Sqlite,
                &extra,
                &Default::default(),
                CompilerWorkMode::Metered(CompileContext::new(control)),
            )
        };
        exact_and_short(raw, run);
        every_stop(run);
    }
}
