//! One exhaustive variable-role walk for classification and substitution.
//! The original AST envelope bounds recursion; each visit/slot is prepaid.
use spargebra::algebra::{AggregateExpression, Expression, GraphPattern, OrderExpression};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern, Variable};
use spargebra::Query;

use super::variables::Role;
use crate::build::control::BuildWork;
use crate::Result;

type Visit<'a> = dyn FnMut(&mut Variable, Role) -> Result<()> + 'a;
#[derive(Clone, Copy, PartialEq)]
enum Context {
    General,
    DescribeRoot,
    DescribeExtends,
}

pub(super) fn query(query: &mut Query, work: BuildWork<'_>, visit: &mut Visit<'_>) -> Result<()> {
    let work = work.enter()?;
    let context = if matches!(query, Query::Describe { .. }) {
        Context::DescribeRoot
    } else {
        Context::General
    };
    let pattern = match query {
        Query::Select { pattern, .. }
        | Query::Ask { pattern, .. }
        | Query::Describe { pattern, .. } => pattern,
        Query::Construct {
            template, pattern, ..
        } => {
            work.charge(template.len())?;
            for value in template {
                triple(value, work, visit)?;
            }
            pattern
        }
    };
    graph(pattern, context, false, work, visit)
}

fn variable(
    v: &mut Variable,
    role: Role,
    protected: bool,
    work: BuildWork<'_>,
    visit: &mut Visit<'_>,
) -> Result<()> {
    work.charge(1)?;
    visit(v, if protected { Role::Preserved } else { role })
}

fn graph(
    p: &mut GraphPattern,
    context: Context,
    protected: bool,
    work: BuildWork<'_>,
    visit: &mut Visit<'_>,
) -> Result<()> {
    let work = work.enter()?;
    match p {
        GraphPattern::Bgp { patterns } => {
            work.charge(patterns.len())?;
            for value in patterns {
                triple(value, work, visit)?;
            }
        }
        GraphPattern::Path {
            subject, object, ..
        } => {
            term(subject, work, visit)?;
            term(object, work, visit)?;
        }
        GraphPattern::Join { left, right }
        | GraphPattern::Lateral { left, right }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right } => {
            graph(left, Context::General, protected, work, visit)?;
            graph(right, Context::General, protected, work, visit)?;
        }
        GraphPattern::LeftJoin {
            left,
            right,
            expression: expr,
        } => {
            graph(left, Context::General, protected, work, visit)?;
            graph(right, Context::General, protected, work, visit)?;
            if let Some(expr) = expr {
                expression(expr, protected, work, visit)?;
            }
        }
        GraphPattern::Filter { expr, inner } => {
            expression(expr, protected, work, visit)?;
            graph(inner, Context::General, protected, work, visit)?;
        }
        GraphPattern::Graph { name, inner } | GraphPattern::Service { name, inner, .. } => {
            named(name, work, visit)?;
            graph(inner, Context::General, protected, work, visit)?;
        }
        GraphPattern::Extend {
            inner,
            variable: v,
            expression: expr,
        } => {
            let constant =
                context == Context::DescribeExtends && matches!(expr, Expression::NamedNode(_));
            variable(
                v,
                if constant {
                    Role::DescribeBind
                } else {
                    Role::Preserved
                },
                protected,
                work,
                visit,
            )?;
            expression(expr, protected, work, visit)?;
            graph(
                inner,
                if constant { context } else { Context::General },
                protected,
                work,
                visit,
            )?;
        }
        GraphPattern::Values { variables, .. } => {
            work.charge(variables.len())?;
            for v in variables {
                variable(v, Role::Preserved, true, work, visit)?;
            }
        }
        GraphPattern::OrderBy {
            inner,
            expression: orders,
        } => {
            work.charge(orders.len())?;
            for order in orders {
                let (OrderExpression::Asc(expr) | OrderExpression::Desc(expr)) = order;
                expression(expr, protected, work, visit)?;
            }
            graph(inner, context, protected, work, visit)?;
        }
        GraphPattern::Project { inner, variables } => {
            work.charge(variables.len())?;
            let describe = context == Context::DescribeRoot;
            for v in variables {
                variable(
                    v,
                    if describe {
                        Role::DescribeProject
                    } else {
                        Role::Preserved
                    },
                    protected,
                    work,
                    visit,
                )?;
            }
            graph(
                inner,
                if describe {
                    Context::DescribeExtends
                } else {
                    Context::General
                },
                protected,
                work,
                visit,
            )?;
        }
        GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. } => graph(inner, context, protected, work, visit)?,
        GraphPattern::Group {
            inner,
            variables,
            aggregates,
        } => {
            work.charge(variables.len())?;
            for v in variables {
                variable(v, Role::Preserved, true, work, visit)?;
            }
            work.charge(aggregates.len())?;
            for (v, aggregate) in aggregates {
                variable(v, Role::Aggregate, protected, work, visit)?;
                match aggregate {
                    AggregateExpression::CountSolutions { .. } => {}
                    AggregateExpression::FunctionCall { expr, .. } => {
                        expression(expr, true, work, visit)?
                    }
                }
            }
            graph(inner, Context::General, protected, work, visit)?;
        }
    }
    work.checkpoint()
}

fn expression(
    expr: &mut Expression,
    protected: bool,
    work: BuildWork<'_>,
    visit: &mut Visit<'_>,
) -> Result<()> {
    let work = work.enter()?;
    match expr {
        Expression::NamedNode(_) | Expression::Literal(_) => {}
        Expression::Variable(v) | Expression::Bound(v) => {
            variable(v, Role::Use, protected, work, visit)?
        }
        Expression::Or(a, b)
        | Expression::And(a, b)
        | Expression::Equal(a, b)
        | Expression::SameTerm(a, b)
        | Expression::Greater(a, b)
        | Expression::GreaterOrEqual(a, b)
        | Expression::Less(a, b)
        | Expression::LessOrEqual(a, b)
        | Expression::Add(a, b)
        | Expression::Subtract(a, b)
        | Expression::Multiply(a, b)
        | Expression::Divide(a, b) => {
            expression(a, protected, work, visit)?;
            expression(b, protected, work, visit)?;
        }
        Expression::In(a, values) => {
            expression(a, protected, work, visit)?;
            expressions(values, protected, work, visit)?;
        }
        Expression::UnaryPlus(a) | Expression::UnaryMinus(a) | Expression::Not(a) => {
            expression(a, protected, work, visit)?
        }
        Expression::Exists(pattern) => graph(pattern, Context::General, protected, work, visit)?,
        Expression::If(a, b, c) => {
            expression(a, protected, work, visit)?;
            expression(b, protected, work, visit)?;
            expression(c, protected, work, visit)?;
        }
        Expression::Coalesce(values) | Expression::FunctionCall(_, values) => {
            expressions(values, protected, work, visit)?
        }
    }
    work.checkpoint()
}

fn expressions(
    values: &mut [Expression],
    protected: bool,
    work: BuildWork<'_>,
    visit: &mut Visit<'_>,
) -> Result<()> {
    work.charge(values.len())?;
    for value in values {
        expression(value, protected, work, visit)?;
    }
    Ok(())
}

fn triple(triple: &mut TriplePattern, work: BuildWork<'_>, visit: &mut Visit<'_>) -> Result<()> {
    let work = work.enter()?;
    term(&mut triple.subject, work, visit)?;
    named(&mut triple.predicate, work, visit)?;
    term(&mut triple.object, work, visit)
}

fn term(term: &mut TermPattern, work: BuildWork<'_>, visit: &mut Visit<'_>) -> Result<()> {
    let work = work.enter()?;
    match term {
        TermPattern::Variable(v) => variable(v, Role::Preserved, true, work, visit)?,
        TermPattern::Triple(value) => triple(value, work, visit)?,
        TermPattern::NamedNode(_) | TermPattern::BlankNode(_) | TermPattern::Literal(_) => {}
    }
    Ok(())
}

fn named(term: &mut NamedNodePattern, work: BuildWork<'_>, visit: &mut Visit<'_>) -> Result<()> {
    let work = work.enter()?;
    if let NamedNodePattern::Variable(v) = term {
        variable(v, Role::Preserved, true, work, visit)?;
    }
    Ok(())
}
