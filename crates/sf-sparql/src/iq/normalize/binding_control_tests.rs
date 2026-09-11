use super::control::{RowVec, RowWork};
use super::structural_test_support::*;
use crate::compiler_control::CompileContext;
use crate::iq::node::{BindDef, IqNode};
use crate::iq::TermDef;
use crate::{CompilerWorkMode, Error};
use sf_core::ir::{Template, TermMap, TermSpec};
use sf_core::query_control::{QueryCharge, QueryControlError};
use std::collections::BTreeMap;

fn column(alias: usize, name: &str) -> BindDef {
    BindDef::Resolved(TermDef::Derived {
        term_map: TermMap::Column(name.into(), TermSpec::iri()),
        alias,
    })
}

#[test]
fn composed_substitution_keeps_outer_override_inner_only_and_exact_failures() {
    let tree = construction(
        construction(data(1, &[]), &[("x", value("inner")), ("y", value("keep"))]),
        &[("x", value("outer")), ("z", value("new"))],
    );
    exact_and_short(tree.clone());
    every_stop(tree.clone());
    let IqNode::Construction { subst, project, .. } = run(tree, &budget(u64::MAX)).unwrap() else {
        panic!();
    };
    assert_eq!(project, ["x", "z"].map(Into::into));
    assert_eq!(format!("{:?}", subst["x"]), format!("{:?}", value("outer")));
    assert_eq!(format!("{:?}", subst["y"]), format!("{:?}", value("keep")));
}

#[test]
fn merged_bindings_keep_rebind_sat_empty_unsupported_and_condition_order() {
    let symbolic = BindDef::Expr(Box::new(spargebra::algebra::Expression::Variable(
        spargebra::term::Variable::new("unbound").unwrap(),
    )));
    let template = BindDef::Resolved(TermDef::Derived {
        term_map: TermMap::Template(
            Template::parse("http://ex/{id}-{name}").unwrap(),
            TermSpec::iri(),
        ),
        alias: 1,
    });
    for (left, right) in [
        (vec![("x", value("a"))], vec![("y", value("b"))]),
        (vec![("x", value("a"))], vec![("x", value("a"))]),
        (vec![("x", value("a"))], vec![("x", value("b"))]),
        (
            vec![("x", column(0, "id")), ("y", column(0, "name"))],
            vec![("x", column(1, "id")), ("y", column(1, "name"))],
        ),
        (vec![("x", symbolic)], vec![("x", column(1, "id"))]),
        (
            vec![("x", BindDef::Resolved(TermDef::Concat(vec![])))],
            vec![("x", value("b"))],
        ),
        (vec![("x", template.clone())], vec![("x", template)]),
    ] {
        let tree = IqNode::InnerJoin {
            children: vec![
                construction(data(0, &[]), &left),
                construction(data(1, &[]), &right),
            ],
            cond: vec![truth()],
        };
        exact_and_short(tree.clone());
        every_stop(tree);
    }
}

#[test]
fn absent_binding_is_not_inserted_until_its_map_entry_is_paid() {
    let mut acc = BTreeMap::new();
    let incoming = BTreeMap::from([("new".into(), value("v"))]);
    // merge visit + empty lookup, then one entry slot + carrier bytes.
    let total = 2 + 1 + std::mem::size_of::<(Box<str>, BindDef)>() as u64;
    let short = budget(total - 1);
    let work = RowWork::new(CompilerWorkMode::Metered(CompileContext::new(&short)));
    let mut conditions = RowVec::new(vec![]);
    assert!(matches!(
        super::bindings::merge_into(&mut acc, incoming, &mut conditions, work),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    assert!(acc.is_empty());
    assert!(conditions.values.is_empty());
    assert_eq!(short.consumed(QueryCharge::CompilerWork), 3);
}
