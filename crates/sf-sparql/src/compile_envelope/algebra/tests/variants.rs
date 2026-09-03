use oxrdf::BaseDirection;
use spargebra::algebra::{
    AggregateExpression, AggregateFunction, Expression, Function, GraphPattern, OrderExpression,
    PropertyPathExpression, QueryDataset,
};
use spargebra::term::{
    BlankNode, GroundTerm, GroundTriple, Literal, NamedNodePattern, TermPattern, TriplePattern,
};

use super::*;

#[test]
fn should_walk_every_query_variant_and_dataset_arm() {
    let dataset = Some(QueryDataset {
        default: vec![iri("default")],
        named: Some(vec![iri("named")]),
    });
    let template = vec![triple(TermPattern::Variable(var("s")))];
    let queries = [
        Query::Select {
            dataset: dataset.clone(),
            pattern: empty(),
            base_iri: None,
        },
        Query::Construct {
            template,
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
    ];

    for query in queries {
        AlgebraEnvelopeV1::validate(&query).expect("query variant fits");
    }
}

#[test]
fn should_walk_every_graph_pattern_variant_and_optional_arm() {
    let expression = Expression::Variable(var("condition"));
    let leaf = || Box::new(empty());
    let patterns = vec![
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
            expression: Some(expression.clone()),
        },
        GraphPattern::Lateral {
            left: leaf(),
            right: leaf(),
        },
        GraphPattern::Filter {
            expr: expression.clone(),
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
            expression: expression.clone(),
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
            expression: vec![OrderExpression::Asc(expression.clone())],
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
    ];

    for pattern in patterns {
        AlgebraEnvelopeV1::validate(&select(pattern)).expect("graph-pattern variant fits");
    }
}

#[test]
fn should_walk_every_expression_variant() {
    let leaf = || Expression::Variable(var("x"));
    let pair = |make: fn(Box<Expression>, Box<Expression>) -> Expression| {
        make(Box::new(leaf()), Box::new(leaf()))
    };
    let expressions = vec![
        Expression::NamedNode(iri("expression")),
        Expression::Literal(Literal::new_simple_literal("literal")),
        leaf(),
        pair(Expression::Or),
        pair(Expression::And),
        pair(Expression::Equal),
        pair(Expression::SameTerm),
        pair(Expression::Greater),
        pair(Expression::GreaterOrEqual),
        pair(Expression::Less),
        pair(Expression::LessOrEqual),
        Expression::In(Box::new(leaf()), vec![leaf()]),
        pair(Expression::Add),
        pair(Expression::Subtract),
        pair(Expression::Multiply),
        pair(Expression::Divide),
        Expression::UnaryPlus(Box::new(leaf())),
        Expression::UnaryMinus(Box::new(leaf())),
        Expression::Not(Box::new(leaf())),
        Expression::Exists(Box::new(empty())),
        Expression::Bound(var("bound")),
        Expression::If(Box::new(leaf()), Box::new(leaf()), Box::new(leaf())),
        Expression::Coalesce(vec![leaf()]),
        Expression::FunctionCall(Function::Str, vec![leaf()]),
    ];

    for expr in expressions {
        let pattern = GraphPattern::Filter {
            expr,
            inner: Box::new(empty()),
        };
        AlgebraEnvelopeV1::validate(&select(pattern)).expect("expression variant fits");
    }
}

#[test]
fn should_walk_every_property_path_variant() {
    let named = || PropertyPathExpression::NamedNode(iri("path"));
    let paths = vec![
        named(),
        PropertyPathExpression::Reverse(Box::new(named())),
        PropertyPathExpression::Sequence(Box::new(named()), Box::new(named())),
        PropertyPathExpression::Alternative(Box::new(named()), Box::new(named())),
        PropertyPathExpression::ZeroOrMore(Box::new(named())),
        PropertyPathExpression::OneOrMore(Box::new(named())),
        PropertyPathExpression::ZeroOrOne(Box::new(named())),
        PropertyPathExpression::NegatedPropertySet(vec![iri("excluded")]),
    ];

    for path in paths {
        let pattern = GraphPattern::Path {
            subject: TermPattern::Variable(var("s")),
            path,
            object: TermPattern::Variable(var("o")),
        };
        AlgebraEnvelopeV1::validate(&select(pattern)).expect("path variant fits");
    }
}

#[test]
fn should_walk_every_term_ground_and_triple_variant() {
    let embedded = TriplePattern {
        subject: TermPattern::NamedNode(iri("embedded-subject")),
        predicate: NamedNodePattern::Variable(var("embedded-predicate")),
        object: TermPattern::Variable(var("embedded-object")),
    };
    let directional =
        Literal::new_directional_language_tagged_literal("directional", "en", BaseDirection::Ltr)
            .expect("valid language tag");
    let patterns = vec![
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
    AlgebraEnvelopeV1::validate(&select(GraphPattern::Bgp { patterns }))
        .expect("term and triple variants fit");

    let ground_triple = GroundTriple {
        subject: iri("ground-subject"),
        predicate: iri("ground-predicate"),
        object: GroundTerm::Literal(Literal::new_simple_literal("ground-object")),
    };
    let values = GraphPattern::Values {
        variables: vec![var("cell")],
        bindings: vec![
            vec![Some(GroundTerm::NamedNode(iri("ground-named")))],
            vec![Some(GroundTerm::Literal(Literal::new_simple_literal(
                "ground-literal",
            )))],
            vec![Some(GroundTerm::Triple(Box::new(ground_triple)))],
            vec![None],
        ],
    };
    AlgebraEnvelopeV1::validate(&select(values)).expect("ground and VALUES variants fit");
}

#[test]
fn should_count_only_retained_literal_strings_in_utf8_bytes() {
    let datatype = iri("datatype");
    let cases = [
        (Literal::new_simple_literal("é"), "é".len()),
        (
            Literal::new_language_tagged_literal("é", "cy").expect("valid language tag"),
            "é".len() + "cy".len(),
        ),
        (
            Literal::new_directional_language_tagged_literal("é", "cy", BaseDirection::Rtl)
                .expect("valid language tag"),
            "é".len() + "cy".len(),
        ),
        (
            Literal::new_typed_literal("é", datatype.clone()),
            "é".len() + datatype.as_str().len(),
        ),
    ];

    for (literal, expected) in cases {
        let query = select(GraphPattern::Filter {
            expr: Expression::Literal(literal),
            inner: Box::new(empty()),
        });
        let envelope = AlgebraEnvelopeV1::validate(&query).expect("literal fits");
        assert_eq!(envelope.retained_payload_bytes, expected);
    }
}

#[test]
fn should_walk_every_function_and_aggregate_variant() {
    for function in every_function() {
        let pattern = GraphPattern::Filter {
            expr: Expression::FunctionCall(function, vec![Expression::Variable(var("arg"))]),
            inner: Box::new(empty()),
        };
        AlgebraEnvelopeV1::validate(&select(pattern)).expect("function variant fits");
    }

    for name in every_aggregate_function() {
        let pattern = GraphPattern::Group {
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
        };
        AlgebraEnvelopeV1::validate(&select(pattern)).expect("aggregate variant fits");
    }

    for order in [
        OrderExpression::Asc(Expression::Variable(var("ascending"))),
        OrderExpression::Desc(Expression::Variable(var("descending"))),
    ] {
        let pattern = GraphPattern::OrderBy {
            inner: Box::new(empty()),
            expression: vec![order],
        };
        AlgebraEnvelopeV1::validate(&select(pattern)).expect("order variant fits");
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
