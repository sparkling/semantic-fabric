use super::base_work_tests::{empty_rows_work, run, scan, singleton_work};
use super::*;
use crate::compiler_control::CompileContext;
use crate::iq::node::ColOrConst;
use crate::leftjoin::optional_work_test_support::{budget, every_stop, exact_and_short};
use sf_core::query_control::{QueryCharge, QueryControlError};

fn invalid() -> IqNode {
    IqNode::Extensional {
        scan: scan(),
        bind: BTreeMap::from([("ignored".into(), ColOrConst::Col("x".into()))]),
    }
}

#[test]
fn base_work_extensional_guard_precedes_output_and_preserves_error() {
    let raw = lower_node(
        invalid(),
        sf_sql::Dialect::Sqlite,
        false,
        &mut 0,
        &HashSet::new(),
        &StarEnv::new(),
        CompilerWorkMode::Uncontrolled,
    )
    .unwrap_err();
    let Error::Unsupported(message) = raw else {
        panic!()
    };
    let exact = 2 + message.len() as u64; // leaf visit and fixed error string
    let control = budget(exact);
    assert!(matches!(run(invalid(), &control), Err(Error::Unsupported(s)) if s == message));
    assert_eq!(control.consumed(QueryCharge::CompilerWork), exact);
    assert!(matches!(
        run(invalid(), &budget(exact - 1)),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
}

#[test]
fn base_work_inner_seed_and_empty_short_circuit_have_exact_visits() {
    for children in [vec![], vec![IqNode::Empty { vars: vec![] }, invalid()]] {
        let expected = singleton_work() + if children.is_empty() { 0 } else { 2 };
        // Empty conditions do not pop; the empty first child prevents later visits.
        let source = IqNode::InnerJoin {
            children,
            cond: vec![],
        };
        let c = budget(expected);
        let result = run(source.clone(), &c).unwrap();
        assert_eq!(c.consumed(QueryCharge::CompilerWork), expected);
        assert!(result.iter().all(|b| b.bindings.is_empty()));
        assert!(run(source.clone(), &budget(expected - 1)).is_err());
        every_stop(|c| run(source.clone(), c));
        exact_and_short(result, |c| run(source.clone(), c));
    }
}

#[test]
fn base_work_inner_pays_seed_and_each_child_before_existing_product() {
    let values = IqNode::Values {
        vars: vec![],
        rows: vec![vec![]; 3],
    };
    let source = IqNode::InnerJoin {
        children: vec![values, IqNode::True],
        cond: vec![],
    };
    // Independently run only the existing product helpers on already-materialized
    // branches. The difference is exactly the new seed, leaves and two visits.
    let old = budget(u64::MAX);
    let mode = CompilerWorkMode::Metered(CompileContext::new(&old));
    let acc = join_branches_with_work_mode(vec![Branch::empty()], vec![Branch::empty(); 3], mode)
        .unwrap();
    let mut acc = join_branches_with_work_mode(acc, vec![Branch::empty()], mode).unwrap();
    apply_conds_to_branches(vec![], &mut acc, sf_sql::Dialect::Sqlite, mode).unwrap();
    let expected =
        old.consumed(QueryCharge::CompilerWork) + 2 * singleton_work() + empty_rows_work(3) + 2;
    let c = budget(expected);
    let result = run(source.clone(), &c).unwrap();
    assert_eq!(c.consumed(QueryCharge::CompilerWork), expected);
    assert_eq!(format!("{result:?}"), format!("{acc:?}"));
    assert!(run(source.clone(), &budget(expected - 1)).is_err());
    every_stop(|c| run(source.clone(), c));
}
