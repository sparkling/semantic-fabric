use ::spargebra::algebra::{
    AggregateExpression, AggregateFunction, Expression, Function, GraphPattern, OrderExpression,
    PropertyPathExpression,
};
use ::spargebra::term::{
    GroundTerm, GroundTriple, Literal, NamedNodePattern, NamedOrBlankNode, Term, TermPattern,
    Triple, TriplePattern,
};
use sf_core::datatype::XsdTypeCode;

use super::{PlanMeasureError, Walker, Work};

pub(super) fn visit_expression<'a>(
    walker: &mut Walker<'a>,
    expr: &'a Expression,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match expr {
        Expression::NamedNode(node) => walker.push(depth, Work::NamedNode(node))?,
        Expression::Literal(literal) => walker.push(depth, Work::Literal(literal))?,
        Expression::Variable(variable) | Expression::Bound(variable) => {
            walker.push(depth, Work::Variable(variable))?;
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
            walker.push(depth, Work::Expression(left))?;
            walker.push(depth, Work::Expression(right))?;
        }
        Expression::In(left, choices) => {
            walker.push(depth, Work::Expression(left))?;
            push_expressions(walker, choices, depth)?;
        }
        Expression::UnaryPlus(inner) | Expression::UnaryMinus(inner) | Expression::Not(inner) => {
            walker.push(depth, Work::Expression(inner))?
        }
        Expression::Exists(pattern) => walker.push(depth, Work::GraphPattern(pattern))?,
        Expression::If(condition, yes, no) => {
            walker.push(depth, Work::Expression(condition))?;
            walker.push(depth, Work::Expression(yes))?;
            walker.push(depth, Work::Expression(no))?;
        }
        Expression::Coalesce(parts) => push_expressions(walker, parts, depth)?,
        Expression::FunctionCall(function, arguments) => {
            walker.push(depth, Work::Function(function))?;
            push_expressions(walker, arguments, depth)?;
        }
    }
    Ok(())
}

pub(super) fn visit_function<'a>(
    walker: &mut Walker<'a>,
    function: &'a Function,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match function {
        Function::Custom(node) => walker.push(depth, Work::NamedNode(node))?,
        Function::Str
        | Function::Lang
        | Function::LangMatches
        | Function::Datatype
        | Function::Iri
        | Function::BNode
        | Function::Rand
        | Function::Abs
        | Function::Ceil
        | Function::Floor
        | Function::Round
        | Function::Concat
        | Function::SubStr
        | Function::StrLen
        | Function::Replace
        | Function::UCase
        | Function::LCase
        | Function::EncodeForUri
        | Function::Contains
        | Function::StrStarts
        | Function::StrEnds
        | Function::StrBefore
        | Function::StrAfter
        | Function::Year
        | Function::Month
        | Function::Day
        | Function::Hours
        | Function::Minutes
        | Function::Seconds
        | Function::Timezone
        | Function::Tz
        | Function::Now
        | Function::Uuid
        | Function::StrUuid
        | Function::Md5
        | Function::Sha1
        | Function::Sha256
        | Function::Sha384
        | Function::Sha512
        | Function::StrLang
        | Function::StrDt
        | Function::IsIri
        | Function::IsBlank
        | Function::IsLiteral
        | Function::IsNumeric
        | Function::Regex
        | Function::Triple
        | Function::Subject
        | Function::Predicate
        | Function::Object
        | Function::IsTriple
        | Function::LangDir
        | Function::HasLang
        | Function::HasLangDir
        | Function::StrLangDir
        | Function::Adjust => {}
    }
    Ok(())
}

pub(super) fn visit_graph_pattern<'a>(
    walker: &mut Walker<'a>,
    pattern: &'a GraphPattern,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match pattern {
        GraphPattern::Bgp { patterns } => {
            walker.collection(patterns.len())?;
            for triple in patterns {
                walker.push(depth, Work::TriplePattern(triple))?;
            }
        }
        GraphPattern::Path {
            subject,
            path,
            object,
        } => {
            walker.push(depth, Work::TermPattern(subject))?;
            walker.push(depth, Work::PropertyPath(path))?;
            walker.push(depth, Work::TermPattern(object))?;
        }
        GraphPattern::Join { left, right }
        | GraphPattern::Lateral { left, right }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right } => {
            walker.push(depth, Work::GraphPattern(left))?;
            walker.push(depth, Work::GraphPattern(right))?;
        }
        GraphPattern::LeftJoin {
            left,
            right,
            expression,
        } => {
            walker.push(depth, Work::GraphPattern(left))?;
            walker.push(depth, Work::GraphPattern(right))?;
            if let Some(expr) = expression {
                walker.push(depth, Work::Expression(expr))?;
            }
        }
        GraphPattern::Filter { expr, inner } => {
            walker.push(depth, Work::Expression(expr))?;
            walker.push(depth, Work::GraphPattern(inner))?;
        }
        GraphPattern::Graph { name, inner }
        | GraphPattern::Service {
            name,
            inner,
            silent: _,
        } => {
            walker.push(depth, Work::NamedNodePattern(name))?;
            walker.push(depth, Work::GraphPattern(inner))?;
        }
        GraphPattern::Extend {
            inner,
            variable,
            expression,
        } => {
            walker.push(depth, Work::GraphPattern(inner))?;
            walker.push(depth, Work::Variable(variable))?;
            walker.push(depth, Work::Expression(expression))?;
        }
        GraphPattern::Values {
            variables,
            bindings,
        } => {
            walker.collection(variables.len())?;
            for variable in variables {
                walker.push(depth, Work::Variable(variable))?;
            }
            walker.collection(bindings.len())?;
            for row in bindings {
                walker.collection(row.len())?;
                for term in row.iter().flatten() {
                    walker.push(depth, Work::GroundTerm(term))?;
                }
            }
        }
        GraphPattern::OrderBy { inner, expression } => {
            walker.push(depth, Work::GraphPattern(inner))?;
            walker.collection(expression.len())?;
            for order in expression {
                walker.push(depth, Work::OrderExpression(order))?;
            }
        }
        GraphPattern::Project { inner, variables } => {
            walker.push(depth, Work::GraphPattern(inner))?;
            walker.collection(variables.len())?;
            for variable in variables {
                walker.push(depth, Work::Variable(variable))?;
            }
        }
        GraphPattern::Distinct { inner } | GraphPattern::Reduced { inner } => {
            walker.push(depth, Work::GraphPattern(inner))?;
        }
        GraphPattern::Slice {
            inner,
            start: _,
            length: _,
        } => {
            walker.push(depth, Work::GraphPattern(inner))?;
        }
        GraphPattern::Group {
            inner,
            variables,
            aggregates,
        } => {
            walker.push(depth, Work::GraphPattern(inner))?;
            walker.collection(variables.len())?;
            for variable in variables {
                walker.push(depth, Work::Variable(variable))?;
            }
            walker.collection(aggregates.len())?;
            for (variable, aggregate) in aggregates {
                walker.push(depth, Work::Variable(variable))?;
                walker.push(depth, Work::AggregateExpression(aggregate))?;
            }
        }
    }
    Ok(())
}

pub(super) fn visit_property_path<'a>(
    walker: &mut Walker<'a>,
    path: &'a PropertyPathExpression,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match path {
        PropertyPathExpression::NamedNode(node) => walker.push(depth, Work::NamedNode(node))?,
        PropertyPathExpression::Reverse(inner)
        | PropertyPathExpression::ZeroOrMore(inner)
        | PropertyPathExpression::OneOrMore(inner)
        | PropertyPathExpression::ZeroOrOne(inner) => {
            walker.push(depth, Work::PropertyPath(inner))?;
        }
        PropertyPathExpression::Sequence(left, right)
        | PropertyPathExpression::Alternative(left, right) => {
            walker.push(depth, Work::PropertyPath(left))?;
            walker.push(depth, Work::PropertyPath(right))?;
        }
        PropertyPathExpression::NegatedPropertySet(nodes) => {
            walker.collection(nodes.len())?;
            for node in nodes {
                walker.push(depth, Work::NamedNode(node))?;
            }
        }
    }
    Ok(())
}

pub(super) fn visit_aggregate_expression<'a>(
    walker: &mut Walker<'a>,
    aggregate: &'a AggregateExpression,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match aggregate {
        AggregateExpression::CountSolutions { distinct: _ } => {}
        AggregateExpression::FunctionCall {
            name,
            expr,
            distinct: _,
        } => {
            walker.push(depth, Work::AggregateFunction(name))?;
            walker.push(depth, Work::Expression(expr))?;
        }
    }
    Ok(())
}

pub(super) fn visit_aggregate_function<'a>(
    walker: &mut Walker<'a>,
    function: &'a AggregateFunction,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match function {
        AggregateFunction::GroupConcat { separator } => {
            if let Some(separator) = separator {
                walker.payload(separator.len())?;
            }
        }
        AggregateFunction::Custom(node) => walker.push(depth, Work::NamedNode(node))?,
        AggregateFunction::Count
        | AggregateFunction::Sum
        | AggregateFunction::Avg
        | AggregateFunction::Min
        | AggregateFunction::Max
        | AggregateFunction::Sample => {}
    }
    Ok(())
}

pub(super) fn visit_order_expression<'a>(
    walker: &mut Walker<'a>,
    order: &'a OrderExpression,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let (OrderExpression::Asc(expr) | OrderExpression::Desc(expr)) = order;
    walker.push(depth, Work::Expression(expr))
}

pub(super) fn visit_triple_pattern<'a>(
    walker: &mut Walker<'a>,
    triple: &'a TriplePattern,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let TriplePattern {
        subject,
        predicate,
        object,
    } = triple;
    walker.push(depth, Work::TermPattern(subject))?;
    walker.push(depth, Work::NamedNodePattern(predicate))?;
    walker.push(depth, Work::TermPattern(object))
}

pub(super) fn visit_term_pattern<'a>(
    walker: &mut Walker<'a>,
    term: &'a TermPattern,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match term {
        TermPattern::NamedNode(node) => walker.push(depth, Work::NamedNode(node))?,
        TermPattern::BlankNode(node) => walker.push(depth, Work::BlankNode(node))?,
        TermPattern::Literal(literal) => walker.push(depth, Work::Literal(literal))?,
        TermPattern::Triple(triple) => walker.push(depth, Work::TriplePattern(triple))?,
        TermPattern::Variable(variable) => walker.push(depth, Work::Variable(variable))?,
    }
    Ok(())
}

pub(super) fn visit_named_node_pattern<'a>(
    walker: &mut Walker<'a>,
    term: &'a NamedNodePattern,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match term {
        NamedNodePattern::NamedNode(node) => walker.push(depth, Work::NamedNode(node)),
        NamedNodePattern::Variable(variable) => walker.push(depth, Work::Variable(variable)),
    }
}

pub(super) fn visit_ground_term<'a>(
    walker: &mut Walker<'a>,
    term: &'a GroundTerm,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match term {
        GroundTerm::NamedNode(node) => walker.push(depth, Work::NamedNode(node))?,
        GroundTerm::Literal(literal) => walker.push(depth, Work::Literal(literal))?,
        GroundTerm::Triple(triple) => walker.push(depth, Work::GroundTriple(triple))?,
    }
    Ok(())
}

pub(super) fn visit_ground_triple<'a>(
    walker: &mut Walker<'a>,
    triple: &'a GroundTriple,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let GroundTriple {
        subject,
        predicate,
        object,
    } = triple;
    walker.push(depth, Work::NamedNode(subject))?;
    walker.push(depth, Work::NamedNode(predicate))?;
    walker.push(depth, Work::GroundTerm(object))
}

pub(super) fn visit_term<'a>(
    walker: &mut Walker<'a>,
    term: &'a Term,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match term {
        Term::NamedNode(node) => walker.push(depth, Work::NamedNode(node))?,
        Term::BlankNode(node) => walker.push(depth, Work::BlankNode(node))?,
        Term::Literal(literal) => walker.push(depth, Work::Literal(literal))?,
        Term::Triple(triple) => walker.push(depth, Work::Triple(triple))?,
    }
    Ok(())
}

pub(super) fn visit_triple<'a>(
    walker: &mut Walker<'a>,
    triple: &'a Triple,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    let Triple {
        subject,
        predicate,
        object,
    } = triple;
    walker.push(depth, Work::NamedOrBlankNode(subject))?;
    walker.push(depth, Work::NamedNode(predicate))?;
    walker.push(depth, Work::Term(object))
}

pub(super) fn visit_named_or_blank<'a>(
    walker: &mut Walker<'a>,
    node: &'a NamedOrBlankNode,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    match node {
        NamedOrBlankNode::NamedNode(node) => walker.push(depth, Work::NamedNode(node)),
        NamedOrBlankNode::BlankNode(node) => walker.push(depth, Work::BlankNode(node)),
    }
}

pub(super) fn visit_literal(
    walker: &mut Walker<'_>,
    literal: &Literal,
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.payload(literal.value().len())?;
    if let Some(language) = literal.language() {
        walker.payload(language.len())?;
    } else if literal.datatype() != XsdTypeCode::String.iri() {
        // `Literal` keeps this `NamedNode` private; public accessors expose its
        // exact IRI, so account for the hidden clone carrier without formatting.
        walker.inline_leaf(depth, literal.datatype().as_str().len())?;
    }
    Ok(())
}

fn push_expressions<'a>(
    walker: &mut Walker<'a>,
    expressions: &'a [Expression],
    depth: usize,
) -> Result<(), PlanMeasureError> {
    walker.collection(expressions.len())?;
    for expression in expressions {
        walker.push(depth, Work::Expression(expression))?;
    }
    Ok(())
}
