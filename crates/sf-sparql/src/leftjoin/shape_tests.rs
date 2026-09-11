use super::optional_work_test_support::{budget, every_stop, exact_and_short};
use super::*;
use crate::compiler_control::CompileContext;
use crate::iq::{Scan, ScanSource};
use sf_core::ir::{LogicalSource, TermMap, TermSpec};
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};

fn mode(control: &dyn QueryControl) -> CompilerWorkMode<'_> {
    CompilerWorkMode::Metered(CompileContext::new(control))
}
fn scan() -> Branch {
    Branch::single(Scan {
        alias: 0,
        source: LogicalSource::Table("items".into()).into(),
    })
}
fn constant(value: &str) -> TermDef {
    TermDef::Const(sf_core::Literal::from(value).into())
}
fn original_fast(left: &[Branch], right: &[Branch]) -> bool {
    right.len() == 1
        && right[0].core.len() == 1
        && !right[0].bindings.iter().any(|(var, def)| {
            matches!(def, TermDef::Const(_))
                && left
                    .iter()
                    .any(|branch| !matches!(branch.bindings.get(var), Some(TermDef::Const(_))))
        })
        && !matches!(right[0].core[0].source, ScanSource::RefAtom { .. })
}

#[test]
fn optional_shape_constant_gate_preserves_first_missing_nonconstant_and_ref_choices() {
    let mut right = scan();
    right.bindings.insert("café".into(), constant("right"));
    let mut left = scan();
    left.bindings.insert("aaa".into(), constant("prefix"));
    for value in [
        None,
        Some(constant("different left value")),
        Some(TermDef::Derived {
            alias: 0,
            term_map: TermMap::Column("id".into(), TermSpec::iri()),
        }),
    ] {
        left.bindings.remove("café");
        if let Some(value) = value {
            left.bindings.insert("café".into(), value);
        }
        for count in [0, 1, 3] {
            let left = vec![left.clone(); count];
            for right in [
                vec![right.clone()],
                vec![right.clone(); 2],
                vec![Branch::empty()],
            ] {
                let expected = original_fast(&left, &right);
                exact_and_short(expected, |control| {
                    shape::select_fast(&left, &right, mode(control))
                });
                every_stop(|control| shape::select_fast(&left, &right, mode(control)));
            }
        }
    }
    right.core[0].source = ScanSource::RefAtom {
        input: Box::new(scan()),
        columns: vec![],
    };
    assert!(!shape::select_fast(&[], &[right], mode(&budget(100))).unwrap());
}

#[test]
fn optional_shape_utf8_comparison_schedule_and_short_circuits_are_exact() {
    let mut right = scan();
    right.bindings.insert("café".into(), constant("right"));
    let mut left = scan();
    left.bindings.insert("aaa".into(), constant("prefix"));
    left.bindings
        .insert("café".into(), constant("unequal but still Const"));
    left.bindings.insert("zzz".into(), constant("unvisited"));
    // Dispatch + right opts visit + shape header + right binding visit + left
    // visit + (key visit + 3 bytes) + (key visit + 5 UTF-8 bytes) + Ref check.
    let expected = 1 + 1 + 1 + 1 + 1 + (1 + 3) + (1 + 5) + 1;
    let paid = budget(expected);
    assert!(shape::select_fast(&[left.clone()], &[right.clone()], mode(&paid)).unwrap());
    assert_eq!(paid.consumed(QueryCharge::CompilerWork), expected);
    let short = budget(expected - 1);
    assert!(matches!(
        shape::select_fast(&[left], &[right.clone()], mode(&short)),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    // First left branch lacks the binding: neither later left branches nor the
    // Ref check execute. Large unrelated payload must not affect this decision.
    let paid = budget(5);
    let mut later = scan();
    later
        .bindings
        .insert("café".into(), constant(&"unused".repeat(1000)));
    assert!(!shape::select_fast(&[scan(), later], &[right], mode(&paid)).unwrap());
    assert_eq!(paid.consumed(QueryCharge::CompilerWork), 5);
}

#[test]
fn optional_shape_opts_guard_preserves_first_failure_before_constant_analysis() {
    let mut with_opt = scan();
    with_opt.opts.push(OptJoin {
        scan: scan().core.remove(0),
        on: vec![],
        extra: vec![],
    });
    let right = vec![scan(), with_opt, scan()];
    let paid = budget(3);
    assert!(shape::right_has_options(&right, mode(&paid)).unwrap());
    assert_eq!(paid.consumed(QueryCharge::CompilerWork), 3);
    exact_and_short(true, |control| {
        shape::right_has_options(&right, mode(control))
    });
    every_stop(|control| shape::right_has_options(&right, mode(control)));
    let raw = left_join_branches(vec![], right.clone(), None, sf_sql::Dialect::Sqlite);
    let controlled = left_join_branches_with_work_mode(
        vec![],
        right,
        None,
        sf_sql::Dialect::Sqlite,
        mode(&budget(u64::MAX)),
    );
    let expected = "nested OPTIONAL inside an OPTIONAL right side is deferred → 501 (ADR-0007)";
    assert!(matches!(&raw, Err(Error::Unsupported(why)) if why == expected));
    assert_eq!(format!("{controlled:?}"), format!("{raw:?}"));
}
