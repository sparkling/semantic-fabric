use super::*;
use spargebra::algebra::{AggregateExpression, Expression};
use spargebra::term::Variable;

fn parse(source: &str) -> Query {
    spargebra::SparqlParser::new().parse_query(source).unwrap()
}

fn var(name: &str) -> Variable {
    Variable::new(name).unwrap()
}
fn empty() -> GraphPattern {
    GraphPattern::Bgp { patterns: vec![] }
}
fn group(name: &str) -> GraphPattern {
    GraphPattern::Group {
        inner: Box::new(empty()),
        variables: vec![],
        aggregates: vec![(
            var(name),
            AggregateExpression::CountSolutions { distinct: false },
        )],
    }
}
fn select(pattern: GraphPattern) -> Query {
    Query::Select {
        dataset: None,
        pattern,
        base_iri: None,
    }
}
fn projected(inner: GraphPattern, name: &str) -> GraphPattern {
    GraphPattern::Project {
        inner: Box::new(inner),
        variables: vec![var(name)],
    }
}
fn wrapped(inner: GraphPattern, internal: &str) -> GraphPattern {
    projected(
        GraphPattern::Extend {
            inner: Box::new(inner),
            variable: var("answer"),
            expression: Expression::Variable(var(internal)),
        },
        "answer",
    )
}

#[test]
fn cache_identity_implicit_and_explicit_outputs_remain_distinct() {
    for wrap in [
        |p| p,
        |p| GraphPattern::Distinct { inner: Box::new(p) },
        |p| GraphPattern::Project {
            inner: Box::new(p),
            variables: vec![],
        },
        |p| GraphPattern::OrderBy {
            inner: Box::new(p),
            expression: vec![],
        },
        |p| GraphPattern::Slice {
            inner: Box::new(p),
            start: 1,
            length: Some(2),
        },
    ] {
        let a = select(wrap(group("a")));
        let b = select(wrap(group("b")));
        assert_eq!(raw(&a).to_string(), a.to_string());
        assert_ne!(raw(&a).to_string(), raw(&b).to_string());
    }
    let a = select(projected(group("a"), "a"));
    let b = select(projected(group("b"), "b"));
    assert_ne!(raw(&a).to_string(), raw(&b).to_string());
    // An empty outer projection does not hide an effective inner projection.
    let a = select(GraphPattern::Project {
        inner: Box::new(wrapped(group("a"), "a")),
        variables: vec![],
    });
    let b = select(GraphPattern::Project {
        inner: Box::new(wrapped(group("b"), "b")),
        variables: vec![],
    });
    assert_eq!(raw(&a).to_string(), raw(&b).to_string());
}

#[test]
fn cache_identity_preserves_authored_collisions_and_nested_projection() {
    for name in ["a", "b"] {
        for inner in [
            GraphPattern::Join {
                left: Box::new(group(name)),
                right: Box::new(GraphPattern::Values {
                    variables: vec![var(name)],
                    bindings: vec![],
                }),
            },
            GraphPattern::Join {
                left: Box::new(group(name)),
                right: Box::new(group(name)),
            },
            projected(group(name), name),
            GraphPattern::Filter {
                inner: Box::new(group(name)),
                expr: Expression::Exists(Box::new(GraphPattern::Values {
                    variables: vec![var(name)],
                    bindings: vec![],
                })),
            },
            GraphPattern::Group {
                inner: Box::new(group(name)),
                variables: vec![var(name)],
                aggregates: vec![],
            },
        ] {
            let query = select(wrapped(inner, name));
            assert_eq!(raw(&query).to_string(), query.to_string());
        }
    }
}

#[test]
fn cache_identity_internal_names_are_fresh_and_ast_is_untouched() {
    let source = "SELECT ?__sf_cache0 (COUNT(*) AS ?answer) WHERE { VALUES ?__sf_cache0 { 1 } } GROUP BY ?__sf_cache0";
    let a = parse(source);
    let before = format!("{a:?}");
    let canonical = raw(&a).to_string();
    assert!(canonical.contains("?__sf_cache1"));
    assert!(canonical.contains("?__sf_cache0"));
    assert_eq!(canonical, raw(&parse(source)).to_string());
    assert_eq!(format!("{a:?}"), before);
    // A candidate's original name is reserved too, even if it looks canonical.
    let query = select(wrapped(group("__sf_cache0"), "__sf_cache0"));
    assert!(raw(&query).to_string().contains("?__sf_cache1"));
}

#[test]
fn cache_identity_describe_normalizes_only_isolated_constant_targets() {
    let a = parse("DESCRIBE <urn:a> <urn:b> ?s WHERE { ?s <urn:p> ?o }");
    let b = parse("DESCRIBE <urn:a> <urn:b> ?s WHERE { ?s <urn:p> ?o }");
    assert_eq!(raw(&a).to_string(), raw(&b).to_string());
    let a = parse("DESCRIBE <urn:a>");
    let b = parse("DESCRIBE ?x WHERE { BIND(<urn:a> AS ?x) }");
    assert_eq!(raw(&a).to_string(), raw(&b).to_string());
    for source in [
        "DESCRIBE ?x WHERE { ?x <urn:p> ?o }",
        "DESCRIBE ?x WHERE { BIND(<urn:a> AS ?x) FILTER(?x = <urn:a>) }",
        "DESCRIBE ?x ?y WHERE { BIND(<urn:a> AS ?x) BIND(?x AS ?y) }",
    ] {
        let query = parse(source);
        assert_eq!(raw(&query).to_string(), query.to_string());
    }
}

#[test]
fn cache_identity_correlated_reference_only_renaming_is_consistent() {
    let make = |name| {
        select(wrapped(
            GraphPattern::Filter {
                inner: Box::new(group(name)),
                expr: Expression::Exists(Box::new(GraphPattern::Filter {
                    inner: Box::new(empty()),
                    expr: Expression::Bound(var(name)),
                })),
            },
            name,
        ))
    };
    assert_eq!(raw(&make("a")).to_string(), raw(&make("b")).to_string());
    for construct in [false, true] {
        let make = |name| {
            let pattern = GraphPattern::Filter {
                inner: Box::new(group(name)),
                expr: Expression::Exists(Box::new(GraphPattern::Filter {
                    inner: Box::new(empty()),
                    expr: Expression::Bound(var(name)),
                })),
            };
            if construct {
                Query::Construct {
                    template: vec![],
                    dataset: None,
                    pattern,
                    base_iri: None,
                }
            } else {
                Query::Ask {
                    dataset: None,
                    pattern,
                    base_iri: None,
                }
            }
        };
        let a = make("a");
        assert!(matches!(raw(&a), Cow::Owned(_)));
        assert_eq!(raw(&a).to_string(), raw(&make("b")).to_string());
    }
}
