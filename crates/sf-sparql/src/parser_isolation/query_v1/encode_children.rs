use spargebra::algebra::{AggregateExpression, Expression, GraphPattern};
use spargebra::term::{GroundTerm, NamedNodePattern, TermPattern};
use spargebra::Query;

use super::encode::NodeRef;
use super::error::QueryWireError;
use super::function::{encode_aggregate_function, encode_function, AggregateFunctionRef};

pub(super) fn populate_children<'query>(
    node: NodeRef<'query>,
    output: &mut Vec<NodeRef<'query>>,
) -> Result<(), QueryWireError> {
    output.clear();
    output.try_reserve_exact(child_capacity(node)?)?;
    match node {
        NodeRef::Query(query) => query_children(query, output)?,
        NodeRef::Dataset(dataset) => {
            reserve(output, dataset.default.len())?;
            output.extend(dataset.default.iter().map(NodeRef::NamedNode));
            if let Some(named) = &dataset.named {
                reserve(output, named.len())?;
                output.extend(named.iter().map(NodeRef::NamedNode));
            }
        }
        NodeRef::Graph(graph) => graph_children(graph, output)?,
        NodeRef::Expression(expression) => expression_children(expression, output)?,
        NodeRef::Function(function) => {
            if let (_, Some(custom)) = encode_function(function) {
                output.push(NodeRef::NamedNode(custom));
            }
        }
        NodeRef::Path(path) => match path {
            spargebra::algebra::PropertyPathExpression::NamedNode(node) => {
                output.push(NodeRef::NamedNode(node));
            }
            spargebra::algebra::PropertyPathExpression::Reverse(inner)
            | spargebra::algebra::PropertyPathExpression::ZeroOrMore(inner)
            | spargebra::algebra::PropertyPathExpression::OneOrMore(inner)
            | spargebra::algebra::PropertyPathExpression::ZeroOrOne(inner) => {
                output.push(NodeRef::Path(inner));
            }
            spargebra::algebra::PropertyPathExpression::Sequence(left, right)
            | spargebra::algebra::PropertyPathExpression::Alternative(left, right) => {
                output.extend([NodeRef::Path(left), NodeRef::Path(right)]);
            }
            spargebra::algebra::PropertyPathExpression::NegatedPropertySet(nodes) => {
                reserve(output, nodes.len())?;
                output.extend(nodes.iter().map(NodeRef::NamedNode));
            }
        },
        NodeRef::Aggregate(aggregate) => match aggregate {
            AggregateExpression::CountSolutions { .. } => {}
            AggregateExpression::FunctionCall { name, expr, .. } => {
                output.extend([NodeRef::AggregateFunction(name), NodeRef::Expression(expr)]);
            }
        },
        NodeRef::AggregateFunction(function) => {
            if let AggregateFunctionRef::Custom(node) = encode_aggregate_function(function) {
                output.push(NodeRef::NamedNode(node));
            }
        }
        NodeRef::Order(order) => {
            let (spargebra::algebra::OrderExpression::Asc(expression)
            | spargebra::algebra::OrderExpression::Desc(expression)) = order;
            output.push(NodeRef::Expression(expression));
        }
        NodeRef::Triple(triple) => output.extend([
            NodeRef::Term(&triple.subject),
            NodeRef::NamedPattern(&triple.predicate),
            NodeRef::Term(&triple.object),
        ]),
        NodeRef::Term(term) => match term {
            TermPattern::NamedNode(node) => output.push(NodeRef::NamedNode(node)),
            TermPattern::BlankNode(node) => output.push(NodeRef::BlankNode(node)),
            TermPattern::Literal(literal) => output.push(NodeRef::Literal(literal)),
            TermPattern::Triple(triple) => output.push(NodeRef::Triple(triple)),
            TermPattern::Variable(variable) => output.push(NodeRef::Variable(variable)),
        },
        NodeRef::NamedPattern(pattern) => match pattern {
            NamedNodePattern::NamedNode(node) => output.push(NodeRef::NamedNode(node)),
            NamedNodePattern::Variable(variable) => output.push(NodeRef::Variable(variable)),
        },
        NodeRef::GroundTerm(term) => match term {
            GroundTerm::NamedNode(node) => output.push(NodeRef::NamedNode(node)),
            GroundTerm::Literal(literal) => output.push(NodeRef::Literal(literal)),
            GroundTerm::Triple(triple) => output.push(NodeRef::GroundTriple(triple)),
        },
        NodeRef::GroundTriple(triple) => output.extend([
            NodeRef::NamedNode(&triple.subject),
            NodeRef::NamedNode(&triple.predicate),
            NodeRef::GroundTerm(&triple.object),
        ]),
        NodeRef::ValuesRow(row) => {
            reserve(output, row.len())?;
            output.extend(
                row.iter()
                    .filter_map(|term| term.as_ref().map(NodeRef::GroundTerm)),
            );
        }
        NodeRef::AggregateBinding(variable, aggregate) => {
            output.extend([NodeRef::Variable(variable), NodeRef::Aggregate(aggregate)])
        }
        NodeRef::NamedNode(_)
        | NodeRef::Variable(_)
        | NodeRef::BlankNode(_)
        | NodeRef::Literal(_)
        | NodeRef::BaseIri(_) => {}
    }
    Ok(())
}

fn child_capacity(node: NodeRef<'_>) -> Result<usize, QueryWireError> {
    let count = match node {
        NodeRef::Query(query) => match query {
            Query::Select {
                dataset, base_iri, ..
            }
            | Query::Describe {
                dataset, base_iri, ..
            }
            | Query::Ask {
                dataset, base_iri, ..
            } => 1 + usize::from(dataset.is_some()) + usize::from(base_iri.is_some()),
            Query::Construct {
                template,
                dataset,
                base_iri,
                ..
            } => checked_sum(
                template.len(),
                1 + usize::from(dataset.is_some()) + usize::from(base_iri.is_some()),
            )?,
        },
        NodeRef::Dataset(dataset) => checked_sum(
            dataset.default.len(),
            dataset.named.as_ref().map_or(0, Vec::len),
        )?,
        NodeRef::Graph(graph) => graph_child_capacity(graph)?,
        NodeRef::Expression(expression) => expression_child_capacity(expression)?,
        NodeRef::Function(function) => usize::from(encode_function(function).1.is_some()),
        NodeRef::Path(path) => match path {
            spargebra::algebra::PropertyPathExpression::NamedNode(_)
            | spargebra::algebra::PropertyPathExpression::Reverse(_)
            | spargebra::algebra::PropertyPathExpression::ZeroOrMore(_)
            | spargebra::algebra::PropertyPathExpression::OneOrMore(_)
            | spargebra::algebra::PropertyPathExpression::ZeroOrOne(_) => 1,
            spargebra::algebra::PropertyPathExpression::Sequence(_, _)
            | spargebra::algebra::PropertyPathExpression::Alternative(_, _) => 2,
            spargebra::algebra::PropertyPathExpression::NegatedPropertySet(nodes) => nodes.len(),
        },
        NodeRef::Aggregate(AggregateExpression::CountSolutions { .. }) => 0,
        NodeRef::Aggregate(AggregateExpression::FunctionCall { .. }) => 2,
        NodeRef::AggregateFunction(function) => usize::from(matches!(
            encode_aggregate_function(function),
            AggregateFunctionRef::Custom(_)
        )),
        NodeRef::Order(_) => 1,
        NodeRef::Triple(_) | NodeRef::GroundTriple(_) => 3,
        NodeRef::Term(_) | NodeRef::NamedPattern(_) | NodeRef::GroundTerm(_) => 1,
        NodeRef::ValuesRow(row) => row.len(),
        NodeRef::AggregateBinding(_, _) => 2,
        NodeRef::NamedNode(_)
        | NodeRef::Variable(_)
        | NodeRef::BlankNode(_)
        | NodeRef::Literal(_)
        | NodeRef::BaseIri(_) => 0,
    };
    Ok(count)
}

fn graph_child_capacity(graph: &GraphPattern) -> Result<usize, QueryWireError> {
    Ok(match graph {
        GraphPattern::Bgp { patterns } => patterns.len(),
        GraphPattern::Path { .. } | GraphPattern::Extend { .. } => 3,
        GraphPattern::Join { .. }
        | GraphPattern::Lateral { .. }
        | GraphPattern::Filter { .. }
        | GraphPattern::Union { .. }
        | GraphPattern::Graph { .. }
        | GraphPattern::Minus { .. }
        | GraphPattern::Service { .. } => 2,
        GraphPattern::LeftJoin { expression, .. } => 2 + usize::from(expression.is_some()),
        GraphPattern::Values {
            variables,
            bindings,
        } => checked_sum(variables.len(), bindings.len())?,
        GraphPattern::OrderBy { expression, .. } => checked_sum(1, expression.len())?,
        GraphPattern::Project { variables, .. } => checked_sum(1, variables.len())?,
        GraphPattern::Distinct { .. }
        | GraphPattern::Reduced { .. }
        | GraphPattern::Slice { .. } => 1,
        GraphPattern::Group {
            variables,
            aggregates,
            ..
        } => checked_sum(checked_sum(1, variables.len())?, aggregates.len())?,
    })
}

fn expression_child_capacity(expression: &Expression) -> Result<usize, QueryWireError> {
    Ok(match expression {
        Expression::NamedNode(_)
        | Expression::Literal(_)
        | Expression::Variable(_)
        | Expression::UnaryPlus(_)
        | Expression::UnaryMinus(_)
        | Expression::Not(_)
        | Expression::Exists(_)
        | Expression::Bound(_) => 1,
        Expression::Or(_, _)
        | Expression::And(_, _)
        | Expression::Equal(_, _)
        | Expression::SameTerm(_, _)
        | Expression::Greater(_, _)
        | Expression::GreaterOrEqual(_, _)
        | Expression::Less(_, _)
        | Expression::LessOrEqual(_, _)
        | Expression::Add(_, _)
        | Expression::Subtract(_, _)
        | Expression::Multiply(_, _)
        | Expression::Divide(_, _) => 2,
        Expression::In(_, values) | Expression::FunctionCall(_, values) => {
            checked_sum(1, values.len())?
        }
        Expression::If(_, _, _) => 3,
        Expression::Coalesce(values) => values.len(),
    })
}

fn checked_sum(left: usize, right: usize) -> Result<usize, QueryWireError> {
    left.checked_add(right)
        .ok_or(QueryWireError::AccountingOverflow)
}

fn query_children<'query>(
    query: &'query Query,
    output: &mut Vec<NodeRef<'query>>,
) -> Result<(), QueryWireError> {
    match query {
        Query::Select {
            dataset,
            pattern,
            base_iri,
        }
        | Query::Describe {
            dataset,
            pattern,
            base_iri,
        }
        | Query::Ask {
            dataset,
            pattern,
            base_iri,
        } => {
            if let Some(dataset) = dataset {
                output.push(NodeRef::Dataset(dataset));
            }
            output.push(NodeRef::Graph(pattern));
            if let Some(base_iri) = base_iri {
                output.push(NodeRef::BaseIri(base_iri));
            }
        }
        Query::Construct {
            template,
            dataset,
            pattern,
            base_iri,
        } => {
            reserve(output, template.len().saturating_add(3))?;
            output.extend(template.iter().map(NodeRef::Triple));
            if let Some(dataset) = dataset {
                output.push(NodeRef::Dataset(dataset));
            }
            output.push(NodeRef::Graph(pattern));
            if let Some(base_iri) = base_iri {
                output.push(NodeRef::BaseIri(base_iri));
            }
        }
    }
    Ok(())
}

fn graph_children<'query>(
    graph: &'query GraphPattern,
    output: &mut Vec<NodeRef<'query>>,
) -> Result<(), QueryWireError> {
    match graph {
        GraphPattern::Bgp { patterns } => {
            reserve(output, patterns.len())?;
            output.extend(patterns.iter().map(NodeRef::Triple));
        }
        GraphPattern::Path {
            subject,
            path,
            object,
        } => output.extend([
            NodeRef::Term(subject),
            NodeRef::Path(path),
            NodeRef::Term(object),
        ]),
        GraphPattern::Join { left, right }
        | GraphPattern::Lateral { left, right }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right } => {
            output.extend([NodeRef::Graph(left), NodeRef::Graph(right)]);
        }
        GraphPattern::LeftJoin {
            left,
            right,
            expression,
        } => {
            output.extend([NodeRef::Graph(left), NodeRef::Graph(right)]);
            if let Some(expression) = expression {
                output.push(NodeRef::Expression(expression));
            }
        }
        GraphPattern::Filter { expr, inner } => {
            output.extend([NodeRef::Expression(expr), NodeRef::Graph(inner)]);
        }
        GraphPattern::Graph { name, inner } => {
            output.extend([NodeRef::NamedPattern(name), NodeRef::Graph(inner)]);
        }
        GraphPattern::Extend {
            inner,
            variable,
            expression,
        } => output.extend([
            NodeRef::Graph(inner),
            NodeRef::Variable(variable),
            NodeRef::Expression(expression),
        ]),
        GraphPattern::Values {
            variables,
            bindings,
        } => {
            reserve(output, variables.len().saturating_add(bindings.len()))?;
            output.extend(variables.iter().map(NodeRef::Variable));
            output.extend(bindings.iter().map(|row| NodeRef::ValuesRow(row)));
        }
        GraphPattern::OrderBy { inner, expression } => {
            reserve(output, expression.len().saturating_add(1))?;
            output.push(NodeRef::Graph(inner));
            output.extend(expression.iter().map(NodeRef::Order));
        }
        GraphPattern::Project { inner, variables } => {
            reserve(output, variables.len().saturating_add(1))?;
            output.push(NodeRef::Graph(inner));
            output.extend(variables.iter().map(NodeRef::Variable));
        }
        GraphPattern::Distinct { inner } | GraphPattern::Reduced { inner } => {
            output.push(NodeRef::Graph(inner));
        }
        GraphPattern::Slice { inner, .. } => output.push(NodeRef::Graph(inner)),
        GraphPattern::Group {
            inner,
            variables,
            aggregates,
        } => {
            reserve(
                output,
                1_usize
                    .saturating_add(variables.len())
                    .saturating_add(aggregates.len()),
            )?;
            output.push(NodeRef::Graph(inner));
            output.extend(variables.iter().map(NodeRef::Variable));
            output.extend(
                aggregates
                    .iter()
                    .map(|(variable, aggregate)| NodeRef::AggregateBinding(variable, aggregate)),
            );
        }
        GraphPattern::Service { name, inner, .. } => {
            output.extend([NodeRef::NamedPattern(name), NodeRef::Graph(inner)]);
        }
    }
    Ok(())
}

fn expression_children<'query>(
    expression: &'query Expression,
    output: &mut Vec<NodeRef<'query>>,
) -> Result<(), QueryWireError> {
    match expression {
        Expression::NamedNode(node) => output.push(NodeRef::NamedNode(node)),
        Expression::Literal(literal) => output.push(NodeRef::Literal(literal)),
        Expression::Variable(variable) | Expression::Bound(variable) => {
            output.push(NodeRef::Variable(variable));
        }
        Expression::Or(left, right)
        | Expression::And(left, right)
        | Expression::Equal(left, right)
        | Expression::SameTerm(left, right)
        | Expression::Greater(left, right)
        | Expression::GreaterOrEqual(left, right)
        | Expression::Less(left, right)
        | Expression::LessOrEqual(left, right)
        | Expression::Add(left, right)
        | Expression::Subtract(left, right)
        | Expression::Multiply(left, right)
        | Expression::Divide(left, right) => {
            output.extend([NodeRef::Expression(left), NodeRef::Expression(right)]);
        }
        Expression::In(left, choices) => {
            reserve(output, choices.len().saturating_add(1))?;
            output.push(NodeRef::Expression(left));
            output.extend(choices.iter().map(NodeRef::Expression));
        }
        Expression::UnaryPlus(inner) | Expression::UnaryMinus(inner) | Expression::Not(inner) => {
            output.push(NodeRef::Expression(inner))
        }
        Expression::Exists(graph) => output.push(NodeRef::Graph(graph)),
        Expression::If(condition, yes, no) => output.extend([
            NodeRef::Expression(condition),
            NodeRef::Expression(yes),
            NodeRef::Expression(no),
        ]),
        Expression::Coalesce(parts) => {
            reserve(output, parts.len())?;
            output.extend(parts.iter().map(NodeRef::Expression));
        }
        Expression::FunctionCall(function, arguments) => {
            reserve(output, arguments.len().saturating_add(1))?;
            output.push(NodeRef::Function(function));
            output.extend(arguments.iter().map(NodeRef::Expression));
        }
    }
    Ok(())
}

fn reserve<T>(output: &mut Vec<T>, additional: usize) -> Result<(), QueryWireError> {
    output.try_reserve(additional)?;
    Ok(())
}
