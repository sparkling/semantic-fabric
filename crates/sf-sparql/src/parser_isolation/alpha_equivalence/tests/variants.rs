use oxiri::Iri;
use oxrdf::BaseDirection;
use spargebra::algebra::{
    AggregateExpression, AggregateFunction, Expression, Function, GraphPattern, OrderExpression,
    PropertyPathExpression, QueryDataset,
};
use spargebra::term::{
    BlankNode, GroundTerm, GroundTriple, Literal, NamedNode, NamedNodePattern, TermPattern,
    TriplePattern, Variable,
};
use spargebra::Query;

use super::super::compare;
use super::super::AlphaVerdictV1;

#[test]
fn typed_comparator_covers_every_pinned_spargebra_variant() {
    let dataset = Some(QueryDataset {
        default: vec![iri("default")],
        named: Some(vec![iri("named")]),
    });
    for query in [
        Query::Select {
            dataset: dataset.clone(),
            pattern: empty(),
            base_iri: Some(
                Iri::parse("https://example.test/base")
                    .expect("valid base IRI")
                    .into(),
            ),
        },
        Query::Construct {
            template: vec![triple(TermPattern::Variable(var("s")))],
            dataset: dataset.clone(),
            pattern: empty(),
            base_iri: None,
        },
        Query::Describe {
            dataset: dataset.clone(),
            pattern: empty(),
            base_iri: None,
        },
        Query::Ask {
            dataset,
            pattern: empty(),
            base_iri: None,
        },
    ] {
        assert_equivalent(query);
    }

    let expr = Expression::Variable(var("condition"));
    let leaf = || Box::new(empty());
    for pattern in vec![
        empty(),
        GraphPattern::Path {
            subject: TermPattern::Variable(var("s")),
            path: PropertyPathExpression::NamedNode(iri("p")),
            object: TermPattern::Variable(var("o")),
        },
        GraphPattern::Join {
            left: leaf(),
            right: leaf(),
        },
        GraphPattern::LeftJoin {
            left: leaf(),
            right: leaf(),
            expression: None,
        },
        GraphPattern::LeftJoin {
            left: leaf(),
            right: leaf(),
            expression: Some(expr.clone()),
        },
        GraphPattern::Lateral {
            left: leaf(),
            right: leaf(),
        },
        GraphPattern::Filter {
            expr: expr.clone(),
            inner: leaf(),
        },
        GraphPattern::Union {
            left: leaf(),
            right: leaf(),
        },
        GraphPattern::Graph {
            name: NamedNodePattern::NamedNode(iri("graph")),
            inner: leaf(),
        },
        GraphPattern::Extend {
            inner: leaf(),
            variable: var("extended"),
            expression: expr.clone(),
        },
        GraphPattern::Minus {
            left: leaf(),
            right: leaf(),
        },
        GraphPattern::Values {
            variables: vec![var("value")],
            bindings: vec![vec![None]],
        },
        GraphPattern::OrderBy {
            inner: leaf(),
            expression: vec![OrderExpression::Asc(expr.clone())],
        },
        GraphPattern::Project {
            inner: leaf(),
            variables: vec![var("projected")],
        },
        GraphPattern::Distinct { inner: leaf() },
        GraphPattern::Reduced { inner: leaf() },
        GraphPattern::Slice {
            inner: leaf(),
            start: 1,
            length: None,
        },
        GraphPattern::Slice {
            inner: leaf(),
            start: 0,
            length: Some(1),
        },
        GraphPattern::Group {
            inner: leaf(),
            variables: vec![var("grouped")],
            aggregates: vec![(
                var("count"),
                AggregateExpression::CountSolutions { distinct: true },
            )],
        },
        GraphPattern::Service {
            name: NamedNodePattern::Variable(var("service")),
            inner: leaf(),
            silent: true,
        },
    ] {
        assert_equivalent(select(pattern));
    }

    let expression_leaf = || Expression::Variable(var("x"));
    let pair = |make: fn(Box<Expression>, Box<Expression>) -> Expression| {
        make(Box::new(expression_leaf()), Box::new(expression_leaf()))
    };
    for expr in vec![
        Expression::NamedNode(iri("expression")),
        Expression::Literal(Literal::new_simple_literal("literal")),
        expression_leaf(),
        pair(Expression::Or),
        pair(Expression::And),
        pair(Expression::Equal),
        pair(Expression::SameTerm),
        pair(Expression::Greater),
        pair(Expression::GreaterOrEqual),
        pair(Expression::Less),
        pair(Expression::LessOrEqual),
        Expression::In(Box::new(expression_leaf()), vec![expression_leaf()]),
        pair(Expression::Add),
        pair(Expression::Subtract),
        pair(Expression::Multiply),
        pair(Expression::Divide),
        Expression::UnaryPlus(Box::new(expression_leaf())),
        Expression::UnaryMinus(Box::new(expression_leaf())),
        Expression::Not(Box::new(expression_leaf())),
        Expression::Exists(Box::new(empty())),
        Expression::Bound(var("bound")),
        Expression::If(
            Box::new(expression_leaf()),
            Box::new(expression_leaf()),
            Box::new(expression_leaf()),
        ),
        Expression::Coalesce(vec![expression_leaf()]),
        Expression::FunctionCall(Function::Str, vec![expression_leaf()]),
    ] {
        assert_equivalent(select(GraphPattern::Filter {
            expr,
            inner: Box::new(empty()),
        }));
    }

    let named_path = || PropertyPathExpression::NamedNode(iri("path"));
    for path in [
        named_path(),
        PropertyPathExpression::Reverse(Box::new(named_path())),
        PropertyPathExpression::Sequence(Box::new(named_path()), Box::new(named_path())),
        PropertyPathExpression::Alternative(Box::new(named_path()), Box::new(named_path())),
        PropertyPathExpression::ZeroOrMore(Box::new(named_path())),
        PropertyPathExpression::OneOrMore(Box::new(named_path())),
        PropertyPathExpression::ZeroOrOne(Box::new(named_path())),
        PropertyPathExpression::NegatedPropertySet(vec![iri("excluded")]),
    ] {
        assert_equivalent(select(GraphPattern::Path {
            subject: TermPattern::Variable(var("s")),
            path,
            object: TermPattern::Variable(var("o")),
        }));
    }

    let embedded = TriplePattern {
        subject: TermPattern::NamedNode(iri("embedded-subject")),
        predicate: NamedNodePattern::Variable(var("embedded-predicate")),
        object: TermPattern::Variable(var("embedded-object")),
    };
    let directional =
        Literal::new_directional_language_tagged_literal("directional", "en", BaseDirection::Ltr)
            .expect("valid language tag");
    let triples = vec![
        triple(TermPattern::NamedNode(iri("named"))),
        triple(TermPattern::BlankNode(BlankNode::new_unchecked("blank"))),
        triple(TermPattern::Literal(Literal::new_simple_literal("simple"))),
        triple(TermPattern::Literal(
            Literal::new_language_tagged_literal("language", "cy").expect("valid language tag"),
        )),
        triple(TermPattern::Literal(directional)),
        triple(TermPattern::Literal(Literal::new_typed_literal(
            "typed",
            iri("datatype"),
        ))),
        triple(TermPattern::Triple(Box::new(embedded))),
        triple(TermPattern::Variable(var("variable"))),
    ];
    assert_equivalent(select(GraphPattern::Bgp { patterns: triples }));

    let ground_triple = GroundTriple {
        subject: iri("ground-subject"),
        predicate: iri("ground-predicate"),
        object: GroundTerm::Literal(Literal::new_simple_literal("ground-object")),
    };
    assert_equivalent(select(GraphPattern::Values {
        variables: vec![var("cell")],
        bindings: vec![
            vec![Some(GroundTerm::NamedNode(iri("ground-named")))],
            vec![Some(GroundTerm::Literal(Literal::new_simple_literal(
                "ground-literal",
            )))],
            vec![Some(GroundTerm::Triple(Box::new(ground_triple)))],
            vec![None],
        ],
    }));

    for function in every_function() {
        assert_equivalent(select(GraphPattern::Filter {
            expr: Expression::FunctionCall(function, vec![Expression::Variable(var("arg"))]),
            inner: Box::new(empty()),
        }));
    }
    for name in every_aggregate_function() {
        assert_equivalent(select(GraphPattern::Group {
            inner: Box::new(empty()),
            variables: Vec::new(),
            aggregates: vec![(
                var("result"),
                AggregateExpression::FunctionCall {
                    name,
                    expr: Expression::Variable(var("arg")),
                    distinct: false,
                },
            )],
        }));
    }
    for order in [
        OrderExpression::Asc(Expression::Variable(var("ascending"))),
        OrderExpression::Desc(Expression::Variable(var("descending"))),
    ] {
        assert_equivalent(select(GraphPattern::OrderBy {
            inner: Box::new(empty()),
            expression: vec![order],
        }));
    }
}

fn assert_equivalent(left: Query) {
    let right = left.clone();
    assert_eq!(
        compare::queries(&left, &right),
        AlphaVerdictV1::Equivalent,
        "pinned typed variant must compare exactly"
    );
}

fn iri(local: &str) -> NamedNode {
    NamedNode::new_unchecked(format!("https://example.test/{local}"))
}

fn var(name: &str) -> Variable {
    Variable::new_unchecked(name)
}

fn empty() -> GraphPattern {
    GraphPattern::Bgp {
        patterns: Vec::new(),
    }
}

fn select(pattern: GraphPattern) -> Query {
    Query::Select {
        dataset: None,
        pattern,
        base_iri: None,
    }
}

fn triple(subject: TermPattern) -> TriplePattern {
    TriplePattern {
        subject,
        predicate: NamedNodePattern::NamedNode(iri("predicate")),
        object: TermPattern::Variable(var("object")),
    }
}

fn every_function() -> Vec<Function> {
    vec![
        Function::Str,
        Function::Lang,
        Function::LangMatches,
        Function::Datatype,
        Function::Iri,
        Function::BNode,
        Function::Rand,
        Function::Abs,
        Function::Ceil,
        Function::Floor,
        Function::Round,
        Function::Concat,
        Function::SubStr,
        Function::StrLen,
        Function::Replace,
        Function::UCase,
        Function::LCase,
        Function::EncodeForUri,
        Function::Contains,
        Function::StrStarts,
        Function::StrEnds,
        Function::StrBefore,
        Function::StrAfter,
        Function::Year,
        Function::Month,
        Function::Day,
        Function::Hours,
        Function::Minutes,
        Function::Seconds,
        Function::Timezone,
        Function::Tz,
        Function::Now,
        Function::Uuid,
        Function::StrUuid,
        Function::Md5,
        Function::Sha1,
        Function::Sha256,
        Function::Sha384,
        Function::Sha512,
        Function::StrLang,
        Function::StrDt,
        Function::IsIri,
        Function::IsBlank,
        Function::IsLiteral,
        Function::IsNumeric,
        Function::Regex,
        Function::Triple,
        Function::Subject,
        Function::Predicate,
        Function::Object,
        Function::IsTriple,
        Function::LangDir,
        Function::HasLang,
        Function::HasLangDir,
        Function::StrLangDir,
        Function::Adjust,
        Function::Custom(iri("fn")),
    ]
}

fn every_aggregate_function() -> Vec<AggregateFunction> {
    vec![
        AggregateFunction::Count,
        AggregateFunction::Sum,
        AggregateFunction::Avg,
        AggregateFunction::Min,
        AggregateFunction::Max,
        AggregateFunction::GroupConcat { separator: None },
        AggregateFunction::GroupConcat {
            separator: Some(" | ".into()),
        },
        AggregateFunction::Sample,
        AggregateFunction::Custom(iri("aggregate")),
    ]
}
