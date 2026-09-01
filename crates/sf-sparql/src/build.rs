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
use spargebra::term::{NamedNodePattern, TriplePattern};

use crate::iq::node::{BindDef, IqCond, IqNode, Var};
use crate::iq::TermDef;
use crate::unfold::ground_term_to_term;
use crate::{Error, Result};

mod aggregate;
mod filter;
mod order;

use aggregate::lower_agg_def;
use filter::lower_filter_to_iqconds;
use order::order_keys;

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
/// Every deferred construct surfaces as [`Error::Unsupported`] (→ HTTP 501), never a
/// silent miscompile (see the module docs for the three context-free 501 classes).
pub fn build_tree(gp: &GraphPattern, current_graph: Option<&NamedNodePattern>) -> Result<IqNode> {
    match gp {
        // ---- leaves / BGP --------------------------------------------------------
        // 0 triples → True (the empty tuple, InnerJoin identity); 1 triple → a single
        // Intensional leaf; n>1 → a condition-free InnerJoin of one Intensional per
        // triple. Each leaf stays UNRESOLVED — resolution against the T-mappings (into
        // Extensional/Construction/Union) is a later milestone (design §2 / §6).
        GraphPattern::Bgp { patterns } => match patterns.as_slice() {
            [] => Ok(IqNode::True),
            [tp] => Ok(intensional(tp, current_graph)),
            many => Ok(IqNode::InnerJoin {
                children: many
                    .iter()
                    .map(|tp| intensional(tp, current_graph))
                    .collect(),
                cond: Vec::new(),
            }),
        },

        // A length-1 fixed-predicate path is one triple → an Intensional leaf (the
        // fast-path). Any closure operator (sequence `/`, alternative `|`, inverse `^`,
        // negated property set `!`, `?`/`+`/`*`) needs the mapping resolution that builds
        // a PathClosure (the hop relation reads the triples-maps), so it is carried
        // **verbatim** as an UNRESOLVED-PATH leaf that RESOLVE compiles via the flat
        // `path_branch` (design §5.2 item 3; M5 Wave 1). Like `Intensional`, the
        // `UnresolvedPath` leaf MUST NOT survive RESOLVE.
        GraphPattern::Path {
            subject,
            path,
            object,
        } => match path {
            PropertyPathExpression::NamedNode(p) => {
                let tp = TriplePattern {
                    subject: subject.clone(),
                    predicate: NamedNodePattern::NamedNode(p.clone()),
                    object: object.clone(),
                };
                Ok(intensional(&tp, current_graph))
            }
            _ => Ok(IqNode::UnresolvedPath {
                subject: subject.clone(),
                path: path.clone(),
                object: object.clone(),
                graph: current_graph.cloned(),
            }),
        },

        // ---- joins ---------------------------------------------------------------
        // ONE InnerJoin; no eager cartesian over branch lists, no union distribution
        // (that is normalization §4.16).
        GraphPattern::Join { left, right } => Ok(IqNode::InnerJoin {
            children: vec![
                build_tree(left, current_graph)?,
                build_tree(right, current_graph)?,
            ],
            cond: Vec::new(),
        }),

        // ONE LeftJoin regardless of the right side's shape (kills the flat
        // multi-scan / nested-OPTIONAL 501). The OPTIONAL ON-expression lowers to the
        // joining condition (empty when absent).
        GraphPattern::LeftJoin {
            left,
            right,
            expression,
        } => Ok(IqNode::LeftJoin {
            left: Box::new(build_tree(left, current_graph)?),
            right: Box::new(build_tree(right, current_graph)?),
            cond: match expression {
                Some(e) => lower_filter_to_iqconds(e, current_graph)?,
                None => Vec::new(),
            },
        }),

        // ---- selection -----------------------------------------------------------
        GraphPattern::Filter { expr, inner } => Ok(IqNode::Filter {
            child: Box::new(build_tree(inner, current_graph)?),
            cond: lower_filter_to_iqconds(expr, current_graph)?,
        }),

        // ---- bag union -----------------------------------------------------------
        // The common output signature is the de-duplicated union of the two arms'
        // scopes (NULL-padding each arm to it is a normalization concern, not built
        // here).
        GraphPattern::Union { left, right } => {
            let l = build_tree(left, current_graph)?;
            let r = build_tree(right, current_graph)?;
            let mut project = l.output_vars();
            for v in r.output_vars() {
                if !project.contains(&v) {
                    project.push(v);
                }
            }
            Ok(IqNode::Union {
                children: vec![l, r],
                project,
            })
        }

        // ---- MINUS (correlated anti-join, design §2) -----------------------------
        // Filter[ NOT EXISTS { right } ] over the left subtree. The disjoint-domain
        // no-op and the BOUND-shared-variable correlation are normalization/lowering
        // concerns (the §4.2 positional caveat), not built here.
        GraphPattern::Minus { left, right } => Ok(IqNode::Filter {
            child: Box::new(build_tree(left, current_graph)?),
            cond: vec![IqCond::NotExists {
                inner: Box::new(build_tree(right, current_graph)?),
                is_minus: true,
            }],
        }),

        // ---- GRAPH <g> { P } / GRAPH ?g { P } -------------------------------------
        // Either a constant graph IRI or a variable (ADR-0035) recurses with
        // `current_graph = Some(name)`, pushing it onto the inner Intensional /
        // UnresolvedPath leaves verbatim — BUILD is context-free (no mapping access),
        // so it cannot itself decide how a variable graph resolves; that is entirely
        // RESOLVE's job (`crate::iq::resolve`, per-candidate graph-map enumeration).
        GraphPattern::Graph { name, inner } => build_tree(inner, Some(name)),

        // ---- BIND(expr AS ?v) → Construction (design §2 Extend arm) --------------
        // A Construction over the inner subtree adding `?v := lower(expr)`; the
        // projected scope is the inner scope ++ `?v`. A non-constant expression is not
        // lowerable context-free → 501 (see module docs class 2).
        GraphPattern::Extend {
            inner,
            variable,
            expression,
        } => {
            let child = build_tree(inner, current_graph)?;
            let mut project = child.output_vars();
            let v: Var = variable.as_str().into();
            if !project.contains(&v) {
                project.push(v.clone());
            }
            let mut subst = BTreeMap::new();
            // BIND(?v := expr) is carried SYMBOLIC (BindDef::Expr) and resolved per
            // leaf-CQ at LOWER via the flat bind_term_def (M3 design §2.2): a variable /
            // CONCAT / arithmetic expression has no column until its triple resolves.
            subst.insert(v, BindDef::Expr(Box::new(expression.clone())));
            Ok(IqNode::Construction {
                child: Box::new(child),
                subst,
                project,
            })
        }

        // ---- GROUP BY + aggregates (design §2 Group arm) -------------------------
        // ONE Aggregation for single- and multi-branch inner alike (the node owns its
        // scope, deleting the flat single-vs-multi `rust_group` fork). The grouping
        // keys are plain variable names; each aggregate maps to an AggDef.
        GraphPattern::Group {
            inner,
            variables,
            aggregates,
        } => Ok(IqNode::Aggregation {
            child: Box::new(build_tree(inner, current_graph)?),
            grouping: variables.iter().map(|v| v.as_str().into()).collect(),
            aggs: aggregates
                .iter()
                .map(|(out, expr)| lower_agg_def(out, expr))
                .collect::<Result<_>>()?,
        }),

        // ---- projection / modifier spine -----------------------------------------
        GraphPattern::Project { inner, variables } => Ok(IqNode::Construction {
            child: Box::new(build_tree(inner, current_graph)?),
            subst: BTreeMap::new(),
            project: variables.iter().map(|v| v.as_str().into()).collect(),
        }),
        // DISTINCT and REDUCED both build a Distinct (REDUCED may dedup — sound).
        GraphPattern::Distinct { inner } | GraphPattern::Reduced { inner } => {
            Ok(IqNode::Distinct {
                child: Box::new(build_tree(inner, current_graph)?),
            })
        }
        GraphPattern::Slice {
            inner,
            start,
            length,
        } => Ok(IqNode::Slice {
            child: Box::new(build_tree(inner, current_graph)?),
            offset: *start,
            limit: *length,
        }),
        // ORDER BY — reuse the flat OrderKey lowering exactly (a variable key →
        // `expr: None`; a complex expression key → the stored Expression under a
        // synthetic `__sf_ord_{n}` variable, evaluated by exec at lowering, iq.rs).
        GraphPattern::OrderBy { inner, expression } => Ok(IqNode::OrderBy {
            child: Box::new(build_tree(inner, current_graph)?),
            keys: order_keys(expression),
        }),

        // ---- VALUES (inline literal table) ---------------------------------------
        // Reuse the flat ground-term lowering: a bound cell → `Const`, an UNDEF cell
        // (`None`) → an unbound (`None`) slot.
        GraphPattern::Values {
            variables,
            bindings,
        } => {
            let mut rows = Vec::with_capacity(bindings.len());
            for row in bindings {
                let mut cells = Vec::with_capacity(row.len());
                for cell in row {
                    cells.push(match cell {
                        Some(gt) => Some(TermDef::Const(ground_term_to_term(gt)?)),
                        None => None,
                    });
                }
                rows.push(cells);
            }
            Ok(IqNode::Values {
                vars: variables.iter().map(|v| v.as_str().into()).collect(),
                rows,
            })
        }

        // Out of v1 coverage (LATERAL, SERVICE, …) → 501, never silently dropped.
        other => Err(Error::Unsupported(format!(
            "graph pattern not supported → 501: {other:?}"
        ))),
    }
}

/// One unresolved triple-pattern leaf at the current active graph (design §2 Bgp
/// arm): the pattern is cloned verbatim and resolution against the T-mappings is
/// deferred to a later milestone (never resolved to `Extensional` here).
fn intensional(tp: &TriplePattern, current_graph: Option<&NamedNodePattern>) -> IqNode {
    IqNode::Intensional {
        pattern: tp.clone(),
        graph: current_graph.cloned(),
    }
}
