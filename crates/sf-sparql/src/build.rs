//! Build — the `spargebra` algebra → operator-tree ([`IqNode`]) builder (ADR-0023
//! M2, design-lock `docs/design/ADR-0023-design-lock.md` §2). It is the structural
//! counterpart of [`crate::unfold::Unfolder::translate_pattern`]: where the flat
//! translation **eagerly flattens** each arm into a `Vec<Branch>` (distributing
//! joins/unions, resolving every triple against the mappings as it goes), this
//! builder produces the `IqNode` **tree** node-by-node, distributing **nothing** —
//! a triple pattern becomes an unresolved [`IqNode::Intensional`] leaf, a `Join`
//! becomes one [`IqNode::InnerJoin`], a `Union` becomes one [`IqNode::Union`], and
//! every node publishes a bottom-up scope via [`IqNode::output_vars`].
//!
//! ## Status: PRODUCTION (tree default since ADR-0023 M8; banner corrected 2026-07-18)
//!
//! This builder IS the live engine's first stage: `translate`/`translate_with`
//! route through [`crate::translate_tree`] by default (`lib.rs`), and the flat
//! [`crate::unfold`] is the `=_bag` oracle / fallback — the reverse of what this
//! banner said during M2/M3 bring-up. The builder is **context-free**: it has
//! no resolved column bindings, no mapping set, and no SQL dialect. Three things
//! therefore cannot be produced here and surface as **tracked sound-501s** (never a
//! silent wrong answer — the no-deferrals mandate is met by the explicit 501 plus
//! the milestone that retires it):
//!
//! 1. **A pushable FILTER leaf** (`?x > 5`, `BOUND(?x)`, `REGEX`, `CONTAINS`, …).
//!    The flat [`crate::unfold::Unfolder::lower_filter_expr`] lowers these to a
//!    [`SqlCond`](crate::iq::SqlCond) over **raw columns**, which needs the bound
//!    column + dialect that only resolution (M3) supplies. The boolean *structure*
//!    (`&&` split into conjuncts, `||`/`!`, `EXISTS`/`NOT EXISTS`) is built fully;
//!    only the resolvable leaf is deferred.
//! 2. **A non-constant `BIND` / aggregate-argument expression** (`?y`, `CONCAT(…)`,
//!    `?a + ?b`). [`crate::unify::bind_term_def`] needs the inner bindings to lower a
//!    variable/computed term; context-free, only a constant IRI/literal lowers (the
//!    "non-lowerable → 501" clause of design §2's `Extend`/`Group` arms).
//! 3. **A property-path closure** (`P+`, `P*`, `p?`, `^p`, `p/q`, `p|q`, `!p`). The
//!    [`IqNode::Path`] leaf needs mapping resolution to build its
//!    [`PathClosure`](crate::iq::PathClosure) (design §5.2 item 3), so BUILD carries the
//!    path **verbatim** as an [`IqNode::UnresolvedPath`] leaf (a transient leaf like
//!    `Intensional`) that RESOLVE compiles via the flat `path_branch` (M5 Wave 1); only a
//!    length-1 fixed-predicate path (≡ one triple) is built directly, as an `Intensional`
//!    leaf. This is NOT a 501 — the closure is resolved, not deferred.
//!
//! ## Arm mapping (design-lock §2)
//!
//! Each [`GraphPattern`] arm builds exactly one subtree; the table in the design-lock
//! §2 is the contract. The `current_graph` recursion parameter is the active `GRAPH`
//! context (`None` = default graph), pushed onto every `Intensional`/`UnresolvedPath`
//! leaf: a constant `GRAPH <g> { … }` or a variable `GRAPH ?g { … }` (ADR-0035,
//! superseding design §5.2 item 6's former build-time 501) both thread through
//! identically here — BUILD is context-free, so which of the two it is only matters
//! to RESOLVE (`crate::iq::resolve`), which enumerates the variable case per
//! candidate's effective graph map.

use std::collections::BTreeMap;

use spargebra::algebra::{GraphPattern, PropertyPathExpression};
use spargebra::term::{GroundTerm, NamedNodePattern, TriplePattern};

use crate::iq::node::{BindDef, IqCond, IqNode, Var};
use crate::iq::TermDef;
use crate::{CompilerWorkMode, Result};

mod aggregate;
pub(crate) mod control;
mod filter;
mod order;
mod scope;

use aggregate::lower_agg_def;
use control::{BuildVec, BuildWork};
use filter::lower_filter_to_iqconds;
use order::order_keys;

#[cfg(test)]
mod control_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests_modifiers;
#[cfg(test)]
mod tests_structure;

/// Build the operator-tree ([`IqNode`]) for a `spargebra` graph pattern (ADR-0023
/// M2, design-lock §2). `current_graph` is the active `GRAPH` context (`None` for the
/// default graph; `Some(NamedNode(g))`/`Some(Variable(v))` for `GRAPH <g>`/`GRAPH ?v`
/// — ADR-0035); it is pushed onto every [`IqNode::Intensional`]/[`IqNode::
/// UnresolvedPath`] leaf the recursion produces.
///
/// Every deferred construct surfaces as [`crate::Error::Unsupported`] (→ HTTP 501), never a
/// silent miscompile (see the module docs for the three context-free 501 classes).
pub fn build_tree(gp: &GraphPattern, current_graph: Option<&NamedNodePattern>) -> Result<IqNode> {
    build_tree_with_work_mode(gp, current_graph, CompilerWorkMode::Uncontrolled)
}

/// Build an already parsed pattern using the request's shared work/cancellation
/// control. This covers structural BUILD, not parsing, later compiler stages or
/// total allocation/destruction safety; it does not activate a governed profile.
pub fn build_tree_with_work_control(
    gp: &GraphPattern,
    current_graph: Option<&NamedNodePattern>,
    control: &dyn sf_core::query_control::QueryControl,
) -> Result<IqNode> {
    build_tree_with_work_mode(
        gp,
        current_graph,
        CompilerWorkMode::Metered(crate::compiler_control::CompileContext::new(control)),
    )
}

pub(crate) fn build_tree_with_work_mode(
    gp: &GraphPattern,
    current_graph: Option<&NamedNodePattern>,
    mode: CompilerWorkMode<'_>,
) -> Result<IqNode> {
    build(gp, current_graph, BuildWork::new(mode))
}

// Keep recursive dispatcher frames small in debug and release builds. Large
// arm-local temporaries live in non-inlined helpers, not on every ancestor frame.
fn build(
    gp: &GraphPattern,
    graph: Option<&NamedNodePattern>,
    work: BuildWork<'_>,
) -> Result<IqNode> {
    let work = work.enter()?;
    let result = match gp {
        GraphPattern::Bgp { .. } | GraphPattern::Path { .. } | GraphPattern::Values { .. } => {
            build_leaf(gp, graph, work)
        }
        GraphPattern::Graph { name, inner } => build(inner, Some(name), work),
        GraphPattern::Join { .. } => build_join(gp, graph, work),
        GraphPattern::LeftJoin { .. } => build_left_join(gp, graph, work),
        GraphPattern::Union { .. } => build_union(gp, graph, work),
        GraphPattern::Minus { .. } => build_minus(gp, graph, work),
        GraphPattern::Filter { .. } => build_filter(gp, graph, work),
        GraphPattern::Extend { .. } => build_extend(gp, graph, work),
        GraphPattern::Group { .. } => build_group(gp, graph, work),
        GraphPattern::Project { .. }
        | GraphPattern::Distinct { .. }
        | GraphPattern::Reduced { .. }
        | GraphPattern::Slice { .. }
        | GraphPattern::OrderBy { .. } => build_modifier(gp, graph, work),
        other => Err(work.unsupported("graph pattern not supported → 501", || {
            format!("graph pattern not supported → 501: {other:?}")
        })?),
    };
    work.checkpoint()?;
    result
}

#[inline(never)]
fn build_leaf(
    gp: &GraphPattern,
    graph: Option<&NamedNodePattern>,
    work: BuildWork<'_>,
) -> Result<IqNode> {
    match gp {
        GraphPattern::Bgp { patterns } => match patterns.as_slice() {
            [] => Ok(IqNode::True),
            [tp] => intensional(tp, graph, work),
            many => {
                let mut children = work.vector(many.len())?;
                for tp in many {
                    children.push(intensional(tp, graph, work)?);
                }
                Ok(IqNode::InnerJoin {
                    children,
                    cond: Vec::new(),
                })
            }
        },
        GraphPattern::Path {
            subject,
            path,
            object,
        } => match path {
            PropertyPathExpression::NamedNode(p) => Ok(IqNode::Intensional {
                // Construct the final triple once; no intermediate recursive copy.
                pattern: TriplePattern {
                    subject: work.copied(subject)?,
                    predicate: NamedNodePattern::NamedNode(work.copied(p)?),
                    object: work.copied(object)?,
                },
                graph: graph.map(|g| work.copied(g)).transpose()?,
            }),
            _ => Ok(IqNode::UnresolvedPath {
                subject: work.copied(subject)?,
                path: work.copied(path)?,
                object: work.copied(object)?,
                graph: graph.map(|g| work.copied(g)).transpose()?,
            }),
        },
        GraphPattern::Values {
            variables,
            bindings,
        } => {
            let mut rows = work.vector(bindings.len())?;
            for row in bindings {
                let mut cells = work.vector(row.len())?;
                for cell in row {
                    work.charge(1)?;
                    cells.push(match cell {
                        Some(GroundTerm::NamedNode(n)) => {
                            Some(TermDef::Const(sf_core::Term::NamedNode(work.copied(n)?)))
                        }
                        Some(GroundTerm::Literal(l)) => {
                            Some(TermDef::Const(sf_core::Term::Literal(work.copied(l)?)))
                        }
                        Some(gt) => {
                            return Err(work.unsupported(
                                "VALUES ground term not supported in v1 → 501",
                                || format!("VALUES ground term not supported in v1 → 501: {gt:?}"),
                            )?)
                        }
                        None => None,
                    });
                }
                rows.push(cells);
            }
            Ok(IqNode::Values {
                vars: work.variables(variables.iter().map(|v| v.as_str()))?,
                rows,
            })
        }
        _ => unreachable!("leaf dispatch"),
    }
}

#[inline(never)]
fn build_join(
    gp: &GraphPattern,
    graph: Option<&NamedNodePattern>,
    work: BuildWork<'_>,
) -> Result<IqNode> {
    let GraphPattern::Join { left, right } = gp else {
        unreachable!("join dispatch")
    };
    let mut children = work.vector(2)?;
    children.push(build(left, graph, work)?);
    children.push(build(right, graph, work)?);
    Ok(IqNode::InnerJoin {
        children,
        cond: Vec::new(),
    })
}

#[inline(never)]
fn build_left_join(
    gp: &GraphPattern,
    graph: Option<&NamedNodePattern>,
    work: BuildWork<'_>,
) -> Result<IqNode> {
    let GraphPattern::LeftJoin {
        left,
        right,
        expression,
    } = gp
    else {
        unreachable!("left-join dispatch")
    };
    Ok(IqNode::LeftJoin {
        left: work.boxed(build(left, graph, work)?)?,
        right: work.boxed(build(right, graph, work)?)?,
        cond: match expression {
            Some(e) => lower_filter_to_iqconds(e, graph, work)?,
            None => Vec::new(),
        },
    })
}

#[inline(never)]
fn build_union(
    gp: &GraphPattern,
    graph: Option<&NamedNodePattern>,
    work: BuildWork<'_>,
) -> Result<IqNode> {
    let GraphPattern::Union { left, right } = gp else {
        unreachable!("union dispatch")
    };
    let left = build(left, graph, work)?;
    let right = build(right, graph, work)?;
    let scope_work = BuildWork::new(work.mode);
    let mut project = BuildVec::new(scope::output_vars(&left, scope_work)?);
    for v in scope::output_vars(&right, scope_work)? {
        work.unique(&mut project, v)?;
    }
    let mut children = work.vector(2)?;
    children.push(left);
    children.push(right);
    Ok(IqNode::Union {
        children,
        project: project.into_inner(),
    })
}

#[inline(never)]
fn build_minus(
    gp: &GraphPattern,
    graph: Option<&NamedNodePattern>,
    work: BuildWork<'_>,
) -> Result<IqNode> {
    let GraphPattern::Minus { left, right } = gp else {
        unreachable!("minus dispatch")
    };
    let child = work.boxed(build(left, graph, work)?)?;
    let mut cond = work.vector(1)?;
    cond.push(IqCond::NotExists {
        inner: work.boxed(build(right, graph, work)?)?,
        is_minus: true,
    });
    Ok(IqNode::Filter { child, cond })
}

#[inline(never)]
fn build_filter(
    gp: &GraphPattern,
    graph: Option<&NamedNodePattern>,
    work: BuildWork<'_>,
) -> Result<IqNode> {
    let GraphPattern::Filter { expr, inner } = gp else {
        unreachable!("filter dispatch")
    };
    Ok(IqNode::Filter {
        child: work.boxed(build(inner, graph, work)?)?,
        cond: lower_filter_to_iqconds(expr, graph, work)?,
    })
}

#[inline(never)]
fn build_extend(
    gp: &GraphPattern,
    graph: Option<&NamedNodePattern>,
    work: BuildWork<'_>,
) -> Result<IqNode> {
    let GraphPattern::Extend {
        inner,
        variable,
        expression,
    } = gp
    else {
        unreachable!("extend dispatch")
    };
    let child = build(inner, graph, work)?;
    let mut project = BuildVec::new(scope::output_vars(&child, BuildWork::new(work.mode))?);
    let v: Var = work.variable(variable.as_str())?;
    if !work.contains(&project.values, &v)? {
        work.push(&mut project, work.variable(&v)?)?;
    }
    let mut subst = BTreeMap::new();
    // Symbolic expression, resolved only once column bindings are available.
    let expr = work.boxed(work.copied(expression)?)?;
    work.charge(1)?;
    work.charge(std::mem::size_of::<(Var, BindDef)>())?;
    subst.insert(v, BindDef::Expr(expr));
    Ok(IqNode::Construction {
        child: work.boxed(child)?,
        subst,
        project: project.into_inner(),
    })
}

#[inline(never)]
fn build_group(
    gp: &GraphPattern,
    graph: Option<&NamedNodePattern>,
    work: BuildWork<'_>,
) -> Result<IqNode> {
    let GraphPattern::Group {
        inner,
        variables,
        aggregates,
    } = gp
    else {
        unreachable!("group dispatch")
    };
    let child = work.boxed(build(inner, graph, work)?)?;
    let grouping = work.variables(variables.iter().map(|v| v.as_str()))?;
    let mut aggs = work.vector(aggregates.len())?;
    for (out, expr) in aggregates {
        aggs.push(lower_agg_def(out, expr, work)?);
    }
    Ok(IqNode::Aggregation {
        child,
        grouping,
        aggs,
    })
}

#[inline(never)]
fn build_modifier(
    gp: &GraphPattern,
    graph: Option<&NamedNodePattern>,
    work: BuildWork<'_>,
) -> Result<IqNode> {
    // Build the child before modifier-local temporaries occupy the recursive stack.
    let inner = match gp {
        GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::OrderBy { inner, .. } => inner,
        _ => unreachable!("modifier dispatch"),
    };
    let child = work.boxed(build(inner, graph, work)?)?;
    finish_modifier(gp, child, work)
}

#[inline(never)]
fn finish_modifier(gp: &GraphPattern, child: Box<IqNode>, work: BuildWork<'_>) -> Result<IqNode> {
    match gp {
        GraphPattern::Project { variables, .. } => Ok(IqNode::Construction {
            child,
            subst: BTreeMap::new(),
            project: work.variables(variables.iter().map(|v| v.as_str()))?,
        }),
        GraphPattern::Distinct { .. } | GraphPattern::Reduced { .. } => {
            Ok(IqNode::Distinct { child })
        }
        GraphPattern::Slice { start, length, .. } => Ok(IqNode::Slice {
            child,
            offset: *start,
            limit: *length,
        }),
        GraphPattern::OrderBy { expression, .. } => Ok(IqNode::OrderBy {
            child,
            keys: order_keys(expression, work)?,
        }),
        _ => unreachable!("modifier dispatch"),
    }
}

/// One owned unresolved leaf; mapping resolution is a later stage.
fn intensional(
    tp: &TriplePattern,
    graph: Option<&NamedNodePattern>,
    work: BuildWork<'_>,
) -> Result<IqNode> {
    work.charge(1)?;
    Ok(IqNode::Intensional {
        pattern: work.copied(tp)?,
        graph: graph.map(|g| work.copied(g)).transpose()?,
    })
}
