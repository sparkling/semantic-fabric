use super::*;
use crate::iq::TermDef;
use crate::leftjoin::optional_work_test_support::{budget, every_stop};
use crate::plan_measure::clone_root::measure_copy_root;
use sf_core::query_control::{QueryCharge, QueryControl, QueryControlError};
use spargebra::algebra::Expression;

#[test]
fn optional_filter_measurement_precedes_inclusive_product_reservation() {
    let expression = Expression::Bound(spargebra::term::Variable::new("name").unwrap());
    let bindings = BTreeMap::from([(
        "name".into(),
        crate::iq::TermDef::Derived {
            alias: 0,
            term_map: sf_core::ir::TermMap::Column("東京".into(), sf_core::ir::TermSpec::iri()),
        },
    )]);
    let measured = budget(u64::MAX);
    let cx = CompileContext::new(&measured);
    let a = cx
        .measure_root(CompilerCloneRootV1::Expression(&expression))
        .unwrap();
    let b = cx
        .measure_root(CompilerCloneRootV1::TermDef(&bindings["name"]))
        .unwrap();
    // One expression visit, then a lookup visit and four UTF-8 key bytes.
    let prefix = measured.consumed(QueryCharge::CompilerWork) + 6;
    let expected = prefix + 1024 + 128 * (a.deep_clone_work + 1) * (5 + b.deep_clone_work + 1);
    let exact = budget(expected);
    let raw = crate::unify::filter_cond(&expression, &bindings, sf_sql::Dialect::Sqlite).unwrap();
    let got = CompileContext::new(&exact)
        .filter_condition(&expression, &bindings, sf_sql::Dialect::Sqlite)
        .unwrap();
    assert_eq!(format!("{got:?}"), format!("{raw:?}"));
    assert_eq!(exact.consumed(QueryCharge::CompilerWork), expected);
    let short = budget(expected - 1);
    assert!(matches!(
        CompileContext::new(&short).filter_condition(
            &expression,
            &bindings,
            sf_sql::Dialect::Sqlite
        ),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    assert_eq!(short.consumed(QueryCharge::CompilerWork), prefix);
}

#[test]
fn optional_filter_product_overflow_is_sticky_without_a_reservation() {
    for (a, b) in [
        (u64::MAX, 0),
        (0, u64::MAX),
        (u64::MAX / 2, 3),
        (0, u64::MAX / 128),
    ] {
        let control = budget(u64::MAX);
        assert!(matches!(
            CompileContext::new(&control).reserve_filter_work(a, b),
            Err(Error::QueryControl(QueryControlError::AccountingOverflow))
        ));
        assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
        assert_eq!(
            control.checkpoint(),
            Err(QueryControlError::AccountingOverflow)
        );
    }
}

fn footprint_formula(
    expression: &Expression,
    bindings: &BTreeMap<String, TermDef>,
    nodes: u64,
    names: &[&str],
) -> (u64, u64) {
    let expression = measure_copy_root(CompilerCloneRootV1::Expression(expression)).unwrap();
    let mut prefix = expression.measurement_work + nodes;
    let mut footprint = 0;
    // This named occurrence list and node count are the fixture oracle, not
    // the production expression traversal or an observed whole compilation.
    for name in names {
        let comparisons = bindings
            .keys()
            .map(|key| 1 + key.len().min(name.len()) as u64)
            .sum::<u64>();
        prefix += comparisons;
        footprint += comparisons;
        if let Some(def) = bindings.get(*name) {
            let measure = measure_copy_root(CompilerCloneRootV1::TermDef(def)).unwrap();
            prefix += measure.measurement_work;
            footprint += measure.deep_clone_work;
        }
    }
    (
        prefix,
        prefix + 1024 + 128 * (expression.deep_clone_work + 1) * (footprint + 1),
    )
}

fn footprint_check(
    expression: &Expression,
    bindings: &BTreeMap<String, TermDef>,
    nodes: u64,
    names: &[&str],
) -> u64 {
    let (prefix, expected) = footprint_formula(expression, bindings, nodes, names);
    let raw = crate::unify::filter_cond(expression, bindings, sf_sql::Dialect::Sqlite)
        .map_err(Error::Unsupported);
    let decision = |control: &dyn QueryControl| match CompileContext::new(control).filter_condition(
        expression,
        bindings,
        sf_sql::Dialect::Sqlite,
    ) {
        Err(Error::QueryControl(cause)) => Err(Error::QueryControl(cause)),
        result => Ok(format!("{result:?}")),
    };
    let control = budget(expected);
    assert_eq!(decision(&control).unwrap(), format!("{raw:?}"));
    assert_eq!(control.consumed(QueryCharge::CompilerWork), expected);
    let short = budget(expected - 1);
    assert!(matches!(
        decision(&short),
        Err(Error::QueryControl(QueryControlError::CompilerWorkExceeded))
    ));
    assert_eq!(short.consumed(QueryCharge::CompilerWork), prefix);
    every_stop(decision);
    expected
}

#[test]
fn optional_filter_lookup_pays_names_but_not_unreferenced_payloads() {
    let expression = Expression::Bound(spargebra::term::Variable::new("name").unwrap());
    let mut costs = Vec::new();
    for bytes in [1, 10_000] {
        let bindings = BTreeMap::from([
            (
                "name".into(),
                TermDef::Derived {
                    alias: 0,
                    term_map: sf_core::ir::TermMap::Column(
                        "id".into(),
                        sf_core::ir::TermSpec::iri(),
                    ),
                },
            ),
            (
                "unrelated".into(),
                TermDef::Const(sf_core::Literal::from("x".repeat(bytes)).into()),
            ),
        ]);
        costs.push(footprint_check(&expression, &bindings, 1, &["name"]));
    }
    assert_eq!(
        costs[0], costs[1],
        "unused term payload is never read by raw FILTER"
    );
    let mut costs = Vec::new();
    for bytes in [1, 100] {
        let bindings = BTreeMap::from([(
            "name".into(),
            TermDef::Derived {
                alias: 0,
                term_map: sf_core::ir::TermMap::Column(
                    "x".repeat(bytes).into(),
                    sf_core::ir::TermSpec::iri(),
                ),
            },
        )]);
        costs.push(footprint_check(&expression, &bindings, 1, &["name"]));
    }
    assert!(
        costs[1] > costs[0],
        "referenced payload is still fully paid"
    );
    let bindings = BTreeMap::from([
        (
            "café_name".into(),
            TermDef::Const(sf_core::Literal::from(true).into()),
        ),
        (
            "café_name_extra".into(),
            TermDef::Const(sf_core::Literal::from(false).into()),
        ),
    ]);
    for name in ["café_name", "café_name_ex", "café_name_extra_long"] {
        let expr = Expression::Bound(spargebra::term::Variable::new(name).unwrap());
        footprint_check(&expr, &bindings, 1, &[name]);
    }
}

#[test]
fn optional_filter_lookup_occurrences_and_expression_variants_keep_error_order() {
    use spargebra::algebra::{Expression as E, Function, GraphPattern};
    let x = E::Variable(spargebra::term::Variable::new("x").unwrap());
    let absent = E::Bound(spargebra::term::Variable::new("absent").unwrap());
    let yes = E::Literal(sf_core::Literal::from(true));
    let bindings = BTreeMap::from([(
        "x".into(),
        TermDef::Const(sf_core::Literal::from(true).into()),
    )]);
    let mut cases: Vec<(E, u64, Vec<&str>)> = vec![
        (
            E::If(
                Box::new(absent.clone()),
                Box::new(x.clone()),
                Box::new(yes.clone()),
            ),
            4,
            vec!["absent", "x"],
        ),
        (
            E::In(Box::new(x.clone()), vec![absent.clone(), x.clone()]),
            4,
            vec!["x", "absent", "x"],
        ),
        (
            E::Coalesce(vec![x.clone(), absent.clone()]),
            3,
            vec!["x", "absent"],
        ),
        (
            E::FunctionCall(Function::Regex, vec![x.clone(), yes.clone()]),
            3,
            vec!["x"],
        ),
        (
            E::Exists(Box::new(GraphPattern::Bgp { patterns: vec![] })),
            1,
            vec![],
        ),
        (
            E::NamedNode(sf_core::NamedNode::new("urn:constant").unwrap()),
            1,
            vec![],
        ),
    ];
    for make in [
        E::Or,
        E::And,
        E::Equal,
        E::SameTerm,
        E::Greater,
        E::GreaterOrEqual,
        E::Less,
        E::LessOrEqual,
        E::Add,
        E::Subtract,
        E::Multiply,
        E::Divide,
    ] {
        cases.push((
            make(Box::new(x.clone()), Box::new(x.clone())),
            3,
            vec!["x", "x"],
        ));
    }
    for make in [E::UnaryPlus, E::UnaryMinus, E::Not] {
        cases.push((make(Box::new(absent.clone())), 2, vec!["absent"]));
    }
    for (expression, nodes, names) in cases {
        footprint_check(&expression, &bindings, nodes, &names);
    }
}
