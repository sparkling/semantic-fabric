//! Shape screen and graph-scoping rewrite for single-default-graph admission.
//! The screen borrows the original parsed pattern and refuses by name before
//! any copy. The rewrite moves the owned pattern, wraps only nonempty BGPs in
//! the selected constant GRAPH and reuses every existing child allocation.

use std::mem;

use spargebra::algebra::{Expression, GraphPattern};
use spargebra::term::{
    GroundTerm, NamedNode, NamedNodePattern, TermPattern, TriplePattern, Variable,
};

use super::{refuse, DatasetRule, GeneratedDatasetError};
use crate::build::control::BuildWork;

type Screened = Result<(), GeneratedDatasetError>;

/// Screen the root modifier spine, then the relational body. Slice, then
/// DISTINCT/REDUCED, then projection may each appear once, in parser order;
/// any other placement is a nested modifier.
pub(super) fn screen(root: &GraphPattern, work: BuildWork<'_>) -> Screened {
    let mut pattern = root;
    let mut work = work;
    let mut minimum = 0_u8;
    loop {
        work = work.enter()?;
        let (rank, inner): (u8, &GraphPattern) = match pattern {
            GraphPattern::Slice { inner, .. } => (0, &**inner),
            GraphPattern::Distinct { inner } | GraphPattern::Reduced { inner } => (1, &**inner),
            GraphPattern::Project { inner, variables } => {
                screen_variables(variables, work)?;
                (2, &**inner)
            }
            GraphPattern::OrderBy { .. } => return refuse(DatasetRule::OrderBy),
            body => return screen_body(body, work),
        };
        if rank < minimum {
            return refuse(DatasetRule::NestedModifier);
        }
        minimum = rank + 1;
        pattern = inner;
    }
}

// The wildcard arm fails closed for feature-gated variants (e.g. Lateral).
#[allow(unreachable_patterns)]
fn screen_body(pattern: &GraphPattern, work: BuildWork<'_>) -> Screened {
    let work = work.enter()?;
    match pattern {
        GraphPattern::Bgp { patterns } => {
            work.charge(patterns.len())?;
            for triple in patterns {
                screen_triple(triple, work)?;
            }
        }
        GraphPattern::Join { left, right } | GraphPattern::Union { left, right } => {
            screen_body(left, work)?;
            screen_body(right, work)?;
        }
        GraphPattern::LeftJoin {
            left,
            right,
            expression,
        } => {
            let simple = expression.is_none()
                && matches!(&**right, GraphPattern::Bgp { patterns } if !patterns.is_empty());
            if !simple {
                return refuse(DatasetRule::OptionalShape);
            }
            screen_body(left, work)?;
            screen_body(right, work)?;
        }
        GraphPattern::Filter { expr, inner } => {
            screen_expression(expr, work)?;
            screen_body(inner, work)?;
        }
        GraphPattern::Values {
            variables,
            bindings,
        } => screen_values(variables, bindings, work)?,
        GraphPattern::Graph { .. } => return refuse(DatasetRule::AuthoredGraph),
        GraphPattern::Service { .. } => return refuse(DatasetRule::Service),
        GraphPattern::Path { .. } => return refuse(DatasetRule::PropertyPath),
        GraphPattern::Group { .. } => return refuse(DatasetRule::Aggregate),
        GraphPattern::Extend { .. } => return refuse(extend_rule(pattern, work)?),
        GraphPattern::Minus { .. } => return refuse(DatasetRule::Minus),
        GraphPattern::OrderBy { .. } => return refuse(DatasetRule::OrderBy),
        GraphPattern::Project { .. }
        | GraphPattern::Distinct { .. }
        | GraphPattern::Reduced { .. }
        | GraphPattern::Slice { .. } => return refuse(DatasetRule::NestedModifier),
        _ => return refuse(DatasetRule::Unclassified),
    }
    work.checkpoint()?;
    Ok(())
}

/// An aggregate projection is an Extend chain over a Group; any other Extend is BIND.
fn extend_rule(
    mut pattern: &GraphPattern,
    work: BuildWork<'_>,
) -> Result<DatasetRule, GeneratedDatasetError> {
    while let GraphPattern::Extend { inner, .. } = pattern {
        work.charge(1)?;
        pattern = &**inner;
    }
    Ok(if matches!(pattern, GraphPattern::Group { .. }) {
        DatasetRule::Aggregate
    } else {
        DatasetRule::Bind
    })
}

fn screen_variables(variables: &[Variable], work: BuildWork<'_>) -> Screened {
    work.charge(variables.len())?;
    for variable in variables {
        work.charge(variable.as_str().len())?;
    }
    Ok(())
}

// The wildcard cell arm fails closed for a feature-gated quoted-triple term.
#[allow(unreachable_patterns)]
fn screen_values(
    variables: &[Variable],
    bindings: &[Vec<Option<GroundTerm>>],
    work: BuildWork<'_>,
) -> Screened {
    screen_variables(variables, work)?;
    work.charge(bindings.len())?;
    for row in bindings {
        work.charge(row.len())?;
        for cell in row {
            match cell {
                None => {}
                Some(GroundTerm::NamedNode(node)) => work.charge(node.as_str().len())?,
                Some(GroundTerm::Literal(literal)) => work.charge(literal.value().len())?,
                Some(_) => return refuse(DatasetRule::RdfStar),
            }
        }
    }
    Ok(())
}

fn screen_triple(triple: &TriplePattern, work: BuildWork<'_>) -> Screened {
    screen_term(&triple.subject, work)?;
    match &triple.predicate {
        NamedNodePattern::NamedNode(node) => work.charge(node.as_str().len())?,
        NamedNodePattern::Variable(variable) => work.charge(variable.as_str().len())?,
    }
    screen_term(&triple.object, work)
}

fn screen_term(term: &TermPattern, work: BuildWork<'_>) -> Screened {
    work.charge(1)?;
    match term {
        TermPattern::NamedNode(node) => work.charge(node.as_str().len())?,
        TermPattern::BlankNode(node) => work.charge(node.as_str().len())?,
        TermPattern::Literal(literal) => work.charge(literal.value().len())?,
        TermPattern::Variable(variable) => work.charge(variable.as_str().len())?,
        TermPattern::Triple(_) => return refuse(DatasetRule::RdfStar),
    }
    Ok(())
}

/// The existing simple FILTER subset: comparisons, `&&`, `||`, `!` and BOUND
/// over variables and constants. EXISTS and everything else refuse by name.
fn screen_expression(expression: &Expression, work: BuildWork<'_>) -> Screened {
    let work = work.enter()?;
    match expression {
        Expression::NamedNode(node) => work.charge(node.as_str().len())?,
        Expression::Literal(literal) => work.charge(literal.value().len())?,
        Expression::Variable(variable) | Expression::Bound(variable) => {
            work.charge(variable.as_str().len())?
        }
        Expression::Or(left, right)
        | Expression::And(left, right)
        | Expression::Equal(left, right)
        | Expression::Greater(left, right)
        | Expression::GreaterOrEqual(left, right)
        | Expression::Less(left, right)
        | Expression::LessOrEqual(left, right) => {
            screen_expression(left, work)?;
            screen_expression(right, work)?;
        }
        Expression::Not(inner) => match &**inner {
            Expression::Exists(_) => return refuse(DatasetRule::Exists),
            other => screen_expression(other, work)?,
        },
        Expression::Exists(_) => return refuse(DatasetRule::Exists),
        _ => return refuse(DatasetRule::UnsupportedExpression),
    }
    work.checkpoint()?;
    Ok(())
}

/// Scope every nonempty default-context BGP to `graph`. Empty BGPs and VALUES
/// keep their identity; no whole-WHERE wrapping, UNION, reparse or format.
pub(super) fn rewrite(
    pattern: GraphPattern,
    graph: &NamedNode,
    work: BuildWork<'_>,
) -> Result<GraphPattern, GeneratedDatasetError> {
    let work = work.enter()?;
    let rewritten = match pattern {
        GraphPattern::Bgp { patterns } => {
            if patterns.is_empty() {
                GraphPattern::Bgp { patterns }
            } else {
                work.charge(1)?;
                let name = NamedNodePattern::NamedNode(work.copied(graph)?);
                let inner = work.boxed(GraphPattern::Bgp { patterns })?;
                GraphPattern::Graph { name, inner }
            }
        }
        GraphPattern::Join { left, right } => GraphPattern::Join {
            left: nested(left, graph, work)?,
            right: nested(right, graph, work)?,
        },
        GraphPattern::Union { left, right } => GraphPattern::Union {
            left: nested(left, graph, work)?,
            right: nested(right, graph, work)?,
        },
        GraphPattern::LeftJoin {
            left,
            right,
            expression,
        } => GraphPattern::LeftJoin {
            left: nested(left, graph, work)?,
            right: nested(right, graph, work)?,
            expression,
        },
        GraphPattern::Filter { expr, inner } => GraphPattern::Filter {
            expr,
            inner: nested(inner, graph, work)?,
        },
        GraphPattern::Project { inner, variables } => GraphPattern::Project {
            inner: nested(inner, graph, work)?,
            variables,
        },
        GraphPattern::Distinct { inner } => GraphPattern::Distinct {
            inner: nested(inner, graph, work)?,
        },
        GraphPattern::Reduced { inner } => GraphPattern::Reduced {
            inner: nested(inner, graph, work)?,
        },
        GraphPattern::Slice {
            inner,
            start,
            length,
        } => GraphPattern::Slice {
            inner: nested(inner, graph, work)?,
            start,
            length,
        },
        values @ GraphPattern::Values { .. } => values,
        // Unreachable after `screen`; kept fail-closed.
        _ => return refuse(DatasetRule::Unclassified),
    };
    work.checkpoint()?;
    Ok(rewritten)
}

fn nested(
    mut inner: Box<GraphPattern>,
    graph: &NamedNode,
    work: BuildWork<'_>,
) -> Result<Box<GraphPattern>, GeneratedDatasetError> {
    let owned = mem::take(&mut *inner);
    *inner = rewrite(owned, graph, work)?;
    Ok(inner)
}
