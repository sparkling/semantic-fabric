use super::optional_work_test_support::{budget, every_stop, exact_and_short};
use super::*;
use crate::compiler_control::CompileContext;
use crate::plan_measure::clone_root::{measure_copy_root, CompilerCloneRootV1};
use sf_core::ir::{TermMap, TermSpec};
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};
use std::collections::BTreeMap;

fn mode(control: &dyn QueryControl) -> CompilerWorkMode<'_> {
    CompilerWorkMode::Metered(CompileContext::new(control))
}
fn term(alias: usize) -> TermDef {
    TermDef::Derived {
        alias,
        term_map: TermMap::Column("café".into(), TermSpec::iri()),
    }
}

#[test]
fn optional_materialization_right_only_pays_current_map_and_keeps_exact_payload() {
    let right = Branch {
        bindings: BTreeMap::from([("café".into(), term(2))]),
        ..Branch::empty()
    };
    let initial = BTreeMap::from([("aaa".into(), term(0)), ("zzz".into(), term(1))]);
    let copies = measure_copy_root(CompilerCloneRootV1::Branch(&right))
        .unwrap()
        .total_work;
    let entries = std::mem::size_of::<(String, TermDef)>() as u64 + 2 * (1 + 3);
    let run = |control: &dyn QueryControl| {
        let mut bindings = initial.clone();
        work::right_copy(&right, mode(control), |right| {
            materialization::insert_copied(
                &mut bindings,
                "café",
                &right.bindings["café"],
                mode(control),
            )
        })?;
        Ok(bindings)
    };
    let paid = budget(copies + entries);
    let result = run(&paid).unwrap();
    assert_eq!(paid.consumed(QueryCharge::CompilerWork), copies + entries);
    let mut expected = initial.clone();
    expected.insert("café".into(), term(2));
    assert_eq!(format!("{result:?}"), format!("{expected:?}"));
    exact_and_short(expected, run);
    every_stop(run);
    // Later edits must pay the map after previous inserts, not a stale prefix.
    let first = budget(u64::MAX);
    work::binding_edit(mode(&first), &initial, "new").unwrap();
    let second = budget(u64::MAX);
    work::binding_edit(mode(&second), &result, "new").unwrap();
    assert_eq!(
        second.consumed(QueryCharge::CompilerWork) - first.consumed(QueryCharge::CompilerWork),
        4
    );
}

#[test]
fn optional_materialization_owned_and_subplan_copied_coalesce_keep_left_first() {
    let right = Branch {
        bindings: BTreeMap::from([("shared".into(), term(2))]),
        ..Branch::empty()
    };
    let initial = BTreeMap::from([("aaa".into(), term(0)), ("shared".into(), term(1))]);
    let mut expected = initial.clone();
    expected.insert(
        "shared".into(),
        TermDef::Coalesce(Box::new(term(1)), Box::new(term(2))),
    );
    for copied in [false, true] {
        let run = |control: &dyn QueryControl| {
            let mut bindings = initial.clone();
            work::right_copy(&right, mode(control), |right| {
                let merge = if copied {
                    materialization::coalesce_copied
                } else {
                    materialization::coalesce_owned
                };
                merge(
                    &mut bindings,
                    "shared",
                    &right.bindings["shared"],
                    mode(control),
                )
            })?;
            Ok(bindings)
        };
        exact_and_short(expected.clone(), run);
        every_stop(run);
    }
    // The owned path must transfer the original left string allocation; no
    // unaccounted extra left payload clone may hide behind equal output terms.
    let mut bindings = initial;
    let TermDef::Derived {
        term_map: TermMap::Column(column, _),
        ..
    } = &bindings["shared"]
    else {
        panic!()
    };
    let original = column.as_ptr();
    let control = budget(u64::MAX);
    work::right_copy(&right, mode(&control), |right| {
        materialization::coalesce_owned(
            &mut bindings,
            "shared",
            &right.bindings["shared"],
            mode(&control),
        )
    })
    .unwrap();
    let TermDef::Coalesce(left, _) = &bindings["shared"] else {
        panic!()
    };
    let TermDef::Derived {
        term_map: TermMap::Column(column, _),
        ..
    } = left.as_ref()
    else {
        panic!()
    };
    assert_eq!(original, column.as_ptr());
}

#[test]
fn optional_materialization_vector_growth_ignores_slack_and_observes_each_stop() {
    // Here only vector work is under test; the caller's source-copy reservation
    // is tested independently by direct_match_copies_exact_left_and_right_fields.
    for capacity in [1, 1024] {
        let run = |control: &dyn QueryControl| {
            let mut values = Vec::with_capacity(capacity);
            values.push(1_u64);
            materialization::extend_copied(&mut values, &[2, 3, 4], mode(control))?;
            Ok(values)
        };
        let expected = 3 + 3 * 8 + 6 * 8;
        let paid = budget(expected);
        assert_eq!(run(&paid).unwrap(), [1, 2, 3, 4]);
        assert_eq!(paid.consumed(QueryCharge::CompilerWork), expected);
        exact_and_short(vec![1, 2, 3, 4], run);
        every_stop(run);
    }
    let cancelled = budget(0);
    cancelled.terminate(QueryControlError::Cancelled);
    assert!(matches!(
        materialization::extend_copied(&mut vec![1_u64], &[], mode(&cancelled)),
        Err(Error::QueryControl(QueryControlError::Cancelled))
    ));
}
