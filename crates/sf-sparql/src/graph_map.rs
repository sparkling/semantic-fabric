//! Query-time handling for normalized R2RML target-graph sets.

use sf_core::ir::TermMap;
use sf_core::{NamedNode, Term};

use crate::iq::{Branch, SqlCond, TermDef};
use crate::unfold::bind;
use crate::unify::{unify, Unify};
use crate::{Error, Result};

/// A graph map generating this reserved IRI targets the default graph.
pub(crate) const RR_DEFAULT_GRAPH: &str = "http://www.w3.org/ns/r2rml#defaultGraph";

pub(crate) fn is_default_graph(graph: &TermMap) -> bool {
    matches!(graph, TermMap::Constant(Term::NamedNode(node))
        if node.as_str() == RR_DEFAULT_GRAPH)
}

pub(crate) enum Filter {
    Always,
    Cond(SqlCond),
    Never,
}

/// Resolve membership in a fixed named/default graph. Dynamic graph maps use
/// the ordinary term unifier, yielding a source-side condition when reversible
/// and an explicit 501 when their shape cannot be constrained soundly.
pub(crate) fn filter(
    active: Option<&NamedNode>,
    graphs: &[&TermMap],
    alias: usize,
) -> Result<Filter> {
    if graphs.is_empty() {
        return Ok(if active.is_none() {
            Filter::Always
        } else {
            Filter::Never
        });
    }

    let target = TermDef::Const(Term::NamedNode(match active {
        Some(graph) => graph.clone(),
        None => NamedNode::new_unchecked(RR_DEFAULT_GRAPH),
    }));
    let mut candidates = Vec::new();
    let mut unsupported = None;
    for graph in graphs {
        match unify(&target, &term_def(graph, alias)) {
            Unify::Sat(conds) if conds.is_empty() => return Ok(Filter::Always),
            Unify::Sat(conds) => candidates.push(conjunction(conds)),
            Unify::Empty => {}
            Unify::Unsupported(reason) => unsupported = Some(reason),
        }
    }

    if let Some(reason) = unsupported {
        return Err(Error::Unsupported(format!(
            "GRAPH constraint against a row-dependent graph map cannot be reduced: {reason} → 501"
        )));
    }
    Ok(match candidates.len() {
        0 => Filter::Never,
        1 => Filter::Cond(candidates.pop().expect("one candidate")),
        _ => Filter::Cond(SqlCond::Or(candidates)),
    })
}

pub(crate) fn apply_filter(
    branch: &mut Branch,
    active: Option<&NamedNode>,
    graphs: &[&TermMap],
    alias: usize,
) -> Result<bool> {
    match filter(active, graphs, alias)? {
        Filter::Always => Ok(true),
        Filter::Cond(condition) => {
            branch.where_conds.push(condition);
            Ok(true)
        }
        Filter::Never => Ok(false),
    }
}

/// Bind one `GRAPH ?g` arm, excluding the default destination and NULL graph
/// terms. The reserved default IRI is checked by generated value, as required by
/// R2RML §9/§11, so column-valued graph maps cannot leak it as a named graph.
pub(crate) fn bind_variable(
    branch: &mut Branch,
    variable: &str,
    graph: &TermMap,
    alias: usize,
) -> Result<bool> {
    let definition = term_def(graph, alias);
    let default = TermDef::Const(Term::NamedNode(NamedNode::new_unchecked(RR_DEFAULT_GRAPH)));
    match unify(&default, &definition) {
        Unify::Sat(conds) if conds.is_empty() => return Ok(false),
        Unify::Sat(conds) => branch
            .where_conds
            .push(SqlCond::Not(Box::new(conjunction(conds)))),
        Unify::Empty => {}
        Unify::Unsupported(reason) => {
            return Err(Error::Unsupported(format!(
                "GRAPH ?g cannot exclude a row-dependent rr:defaultGraph value: {reason} → 501"
            )))
        }
    }
    for column in definition.columns() {
        if !branch
            .where_conds
            .iter()
            .any(|condition| matches!(condition, SqlCond::IsNotNull(found) if found == &column))
        {
            branch.where_conds.push(SqlCond::IsNotNull(column));
        }
    }
    bind(branch, variable, definition)
}

/// Paths cannot attach a row-level graph condition to their recursive hop CTE.
/// Static membership is accepted; a dynamic condition is an honest 501.
pub(crate) fn path_scope_matches(active: Option<&NamedNode>, graphs: &[&TermMap]) -> Result<bool> {
    match filter(active, graphs, 0)? {
        Filter::Always => Ok(true),
        Filter::Never => Ok(false),
        Filter::Cond(_) => Err(Error::Unsupported(
            "property path under a row-dependent target graph cannot be constrained soundly → 501"
                .to_owned(),
        )),
    }
}

fn term_def(graph: &TermMap, alias: usize) -> TermDef {
    match graph {
        TermMap::Constant(term) => TermDef::Const(term.clone()),
        dynamic => TermDef::Derived {
            term_map: dynamic.clone(),
            alias,
        },
    }
}

fn conjunction(mut conditions: Vec<SqlCond>) -> SqlCond {
    if conditions.len() == 1 {
        conditions.pop().expect("one condition")
    } else {
        SqlCond::And(conditions)
    }
}
