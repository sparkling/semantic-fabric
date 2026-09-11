//! Independently run the entry/scope operations around, never through, LOWER's
//! relational body. Exact copy/product tests pay this prefix without calibrating
//! away the operation they intend to reject. Only non-aggregation fixtures.
use super::{scope, BuildWork, IqNode, Var};
use crate::compiler_control::CompileContext;
use crate::leftjoin::optional_work_test_support::budget;
use crate::CompilerWorkMode;
use sf_core::query_control::QueryCharge;

pub(crate) fn entry_work(tree: &IqNode) -> (u64, u64) {
    let control = budget(u64::MAX);
    let work = BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(&control)));
    scope::starting_alias(tree, work).unwrap();
    let mut project = None;
    let mut node = tree;
    loop {
        work.charge(1).unwrap();
        match node {
            IqNode::Distinct { child }
            | IqNode::Slice { child, .. }
            | IqNode::OrderBy { child, .. } => node = child,
            IqNode::Construction {
                child,
                project: vars,
                ..
            } if matches!(
                **child,
                IqNode::Distinct { .. } | IqNode::Slice { .. } | IqNode::OrderBy { .. }
            ) =>
            {
                scope::record_project(&mut project, vars, work).unwrap();
                node = child;
            }
            IqNode::Aggregation { .. } => panic!("calibration excludes aggregation"),
            other => {
                scope::record_output_scope(&mut project, other, work).unwrap();
                break;
            }
        }
    }
    let prefix = control.consumed(QueryCharge::CompilerWork);
    let project: Vec<Var> = project.expect("fixture must declare an output scope");
    let tail = budget(u64::MAX);
    scope::plan_vars(
        Some(project),
        &[],
        BuildWork::new(CompilerWorkMode::Metered(CompileContext::new(&tail))),
    )
    .unwrap();
    (prefix, tail.consumed(QueryCharge::CompilerWork))
}
