use super::optional_work_test_support::{every_stop, exact_and_short};
use super::*;
use crate::compiler_control::CompileContext;
use sf_core::ir::{LogicalSource, Template, TermMap, TermSpec};
use sf_core::query_control::QueryControl;
use sf_sql::Dialect;
use spargebra::algebra::{Expression as E, Function};
use spargebra::term::Variable;

fn variable(name: &str) -> E {
    E::Variable(Variable::new(name).unwrap())
}
fn decision<T: std::fmt::Debug>(result: Result<T>) -> Result<String> {
    match result {
        Err(Error::QueryControl(cause)) => Err(Error::QueryControl(cause)),
        other => Ok(format!("{other:?}")),
    }
}
fn fixture() -> Branch {
    let mut branch = Branch::single(crate::iq::Scan {
        alias: 1,
        source: LogicalSource::Table("items".into()).into(),
    });
    for (name, map) in [
        (
            "a",
            TermMap::Column("café".into(), TermSpec::plain_literal()),
        ),
        ("iri", TermMap::Column("id".into(), TermSpec::iri())),
        (
            "template",
            TermMap::Template(
                Template::parse("http://example.test/{id}").unwrap(),
                TermSpec::iri(),
            ),
        ),
    ] {
        branch.bindings.insert(
            name.into(),
            TermDef::Derived {
                alias: 1,
                term_map: map,
            },
        );
    }
    branch
}

#[test]
fn optional_filter_preserves_raw_conditions_refusals_and_each_terminal_charge() {
    let branch = fixture();
    let literal = E::Literal(sf_core::Literal::from("東京_%\\\n"));
    let equal = E::Equal(Box::new(variable("a")), Box::new(literal.clone()));
    let mut expressions = vec![
        equal.clone(),
        E::SameTerm(Box::new(variable("a")), Box::new(literal.clone())),
        E::Greater(Box::new(variable("a")), Box::new(literal.clone())),
        E::GreaterOrEqual(Box::new(variable("a")), Box::new(literal.clone())),
        E::Less(Box::new(variable("a")), Box::new(literal.clone())),
        E::LessOrEqual(Box::new(variable("a")), Box::new(literal.clone())),
        E::Not(Box::new(equal.clone())),
        E::And(Box::new(equal.clone()), Box::new(equal.clone())),
        E::Or(Box::new(equal.clone()), Box::new(equal)),
        E::Bound(Variable::new("a").unwrap()),
        E::Bound(Variable::new("missing").unwrap()),
        E::Equal(Box::new(variable("missing")), Box::new(literal.clone())),
        E::Equal(
            Box::new(variable("iri")),
            Box::new(E::NamedNode(
                sf_core::NamedNode::new("http://example.test/1").unwrap(),
            )),
        ),
        E::SameTerm(
            Box::new(variable("template")),
            Box::new(variable("template")),
        ),
        E::Literal(sf_core::Literal::from(true)),
        E::Literal(sf_core::Literal::from(false)),
        literal.clone(),
        variable("a"),
        E::Add(Box::new(literal.clone()), Box::new(literal.clone())),
    ];
    for f in [
        Function::Contains,
        Function::StrStarts,
        Function::StrEnds,
        Function::Regex,
        Function::Concat,
    ] {
        expressions.push(E::FunctionCall(f, vec![variable("a"), literal.clone()]));
    }
    expressions.push(E::FunctionCall(
        Function::Concat,
        vec![E::NamedNode(sf_core::NamedNode::new("urn:error").unwrap())],
    ));
    for dialect in [Dialect::Sqlite, Dialect::Postgres, Dialect::MySql] {
        for expression in &expressions {
            let raw = decision(
                crate::unify::filter_scopes(expression, &branch.bindings, dialect, &[&branch])
                    .map_err(Error::Unsupported),
            )
            .unwrap();
            let run = |control: &dyn QueryControl| {
                decision(work::filter_scopes(
                    CompilerWorkMode::Metered(CompileContext::new(control)),
                    expression,
                    &branch.bindings,
                    dialect,
                    &[&branch],
                ))
            };
            exact_and_short(raw, run);
            every_stop(run);
        }
    }
}

#[test]
fn optional_filter_fast_match_and_anti_keep_r5_placement_and_first_error() {
    let left = fixture();
    let mut right = Branch::single(crate::iq::Scan {
        alias: 2,
        source: LogicalSource::Table("other".into()).into(),
    });
    right.bindings.insert(
        "right".into(),
        TermDef::Derived {
            alias: 2,
            term_map: TermMap::Column("value".into(), TermSpec::plain_literal()),
        },
    );
    for expr in [
        E::Bound(Variable::new("right").unwrap()),
        E::Bound(Variable::new("missing").unwrap()),
    ] {
        for count in [1, 2] {
            let raw = decision(left_join_branches(
                vec![left.clone()],
                vec![right.clone(); count],
                Some(&expr),
                Dialect::Sqlite,
            ))
            .unwrap();
            let run = |control: &dyn QueryControl| {
                decision(left_join_branches_with_work_mode(
                    vec![left.clone()],
                    vec![right.clone(); count],
                    Some(&expr),
                    Dialect::Sqlite,
                    CompilerWorkMode::Metered(CompileContext::new(control)),
                ))
            };
            exact_and_short(raw, run);
            every_stop(run);
        }
    }
}
