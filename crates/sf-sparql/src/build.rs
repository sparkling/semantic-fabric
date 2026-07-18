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
//! §2 is the contract. The `current_graph` recursion parameter is the resolved
//! constant active graph (`GRAPH <g> { … }`), pushed onto every `Intensional` leaf;
//! a variable graph name is a build-time 501 (design §5.2 item 6, out of charter).

use std::collections::BTreeMap;

use sf_core::datatype::XsdTypeCode;
use sf_core::{NamedNode, Term};
use spargebra::algebra::{
    AggregateExpression, AggregateFunction, Expression, GraphPattern, OrderExpression,
    PropertyPathExpression,
};
use spargebra::term::{NamedNodePattern, TriplePattern, Variable};

use crate::iq::node::{AggArg, AggDef, BindDef, IqCond, IqNode, Var};
use crate::iq::{AggKind, OrderKey, TermDef};
use crate::unfold::ground_term_to_term;
use crate::{Error, Result};

/// Build the operator-tree ([`IqNode`]) for a `spargebra` graph pattern (ADR-0023
/// M2, design-lock §2). `current_graph` is the resolved constant active graph (the
/// `GRAPH <g> { … }` context, `None` for the default graph); it is pushed onto every
/// [`IqNode::Intensional`] leaf the recursion produces.
///
/// Every deferred construct surfaces as [`Error::Unsupported`] (→ HTTP 501), never a
/// silent miscompile (see the module docs for the three context-free 501 classes).
pub fn build_tree(gp: &GraphPattern, current_graph: Option<&NamedNode>) -> Result<IqNode> {
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

        // ---- GRAPH <g> { P } -----------------------------------------------------
        // A constant graph IRI recurses with `current_graph = Some(g)`, pushing g onto
        // the inner Intensional leaves. A variable graph name is quad querying, out of
        // charter → 501 at build (design §5.2 item 6).
        GraphPattern::Graph { name, inner } => match name {
            NamedNodePattern::NamedNode(g) => build_tree(inner, Some(g)),
            NamedNodePattern::Variable(_) => Err(Error::Unsupported(
                "variable graph (quad querying out of charter)".to_owned(),
            )),
        },

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
fn intensional(tp: &TriplePattern, current_graph: Option<&NamedNode>) -> IqNode {
    IqNode::Intensional {
        pattern: tp.clone(),
        graph: current_graph.cloned(),
    }
}

/// Lower a SPARQL FILTER / OPTIONAL ON-expression to a conjunction of [`IqCond`]s,
/// splitting a top-level `&&` into independent conjuncts (design §2 Filter arm; §9
/// `IqCond` amendment). It mirrors the *Expression coverage* of the flat
/// [`crate::unfold::Unfolder::lower_filter_expr`]: `EXISTS`/`NOT EXISTS` build a
/// first-class subtree (the case the flat `SqlCond` cannot carry before lowering,
/// §9); `||`/`!` compose via [`IqCond::Or`]/[`IqCond::Not`]. A pushable leaf
/// (comparison/`BOUND`/`REGEX`/string match) needs the bound-column + dialect
/// resolution that the context-free builder lacks → a tracked sound-501 (M3); any
/// expression the flat model would itself 501 propagates the same 501.
fn lower_filter_to_iqconds(
    expr: &Expression,
    current_graph: Option<&NamedNode>,
) -> Result<Vec<IqCond>> {
    let mut out = Vec::new();
    collect_conjuncts(expr, current_graph, &mut out)?;
    Ok(out)
}

/// Flatten a top-level `&&` chain into independent conjuncts, lowering each.
fn collect_conjuncts(
    expr: &Expression,
    current_graph: Option<&NamedNode>,
    out: &mut Vec<IqCond>,
) -> Result<()> {
    match expr {
        Expression::And(a, b) => {
            collect_conjuncts(a, current_graph, out)?;
            collect_conjuncts(b, current_graph, out)
        }
        other => {
            out.push(lower_iqcond(other, current_graph)?);
            Ok(())
        }
    }
}

/// Lower a single (non-top-level-`&&`) FILTER expression to one [`IqCond`].
fn lower_iqcond(expr: &Expression, current_graph: Option<&NamedNode>) -> Result<IqCond> {
    match expr {
        Expression::Exists(p) => Ok(IqCond::Exists(Box::new(build_tree(p, current_graph)?))),
        Expression::Not(inner) => match inner.as_ref() {
            Expression::Exists(p) => Ok(IqCond::NotExists {
                inner: Box::new(build_tree(p, current_graph)?),
                is_minus: false,
            }),
            other => Ok(IqCond::Not(Box::new(lower_iqcond(other, current_graph)?))),
        },
        Expression::And(a, b) => Ok(IqCond::And(vec![
            lower_iqcond(a, current_graph)?,
            lower_iqcond(b, current_graph)?,
        ])),
        Expression::Or(a, b) => Ok(IqCond::Or(vec![
            lower_iqcond(a, current_graph)?,
            lower_iqcond(b, current_graph)?,
        ])),
        // A pushable leaf is carried SYMBOLIC (IqCond::Expr) and resolved to a SqlCond
        // per leaf-CQ at LOWER via the flat lower_filter_expr (M3 design §2.1): a FILTER
        // above a Union has no single column for a variable until the union is split.
        other => Ok(IqCond::Expr(Box::new(other.clone()))),
    }
}

/// Lower an aggregate-argument expression to a [`TermDef`] (design §2 Group arm; `BIND`
/// is now carried symbolically as [`BindDef::Expr`], not via this fn). Context-free, only
/// a constant IRI/literal is lowerable; a variable / `CONCAT` / arithmetic aggregate
/// argument stays a tracked sound-501 (M3 design §2.3).
fn lower_expr_to_termdef(expr: &Expression) -> Result<TermDef> {
    match expr {
        Expression::NamedNode(n) => Ok(TermDef::Const(Term::NamedNode(n.clone()))),
        Expression::Literal(l) => Ok(TermDef::Const(Term::Literal(l.clone()))),
        other => Err(Error::Unsupported(format!(
            "BIND/expression resolution needs bound columns (M3) → 501: {other:?}"
        ))),
    }
}

/// Map one `(output-variable, AggregateExpression)` to an [`AggDef`] (design §2
/// Group arm). `COUNT(*)` carries no argument (and rides `distinct` for
/// `COUNT(DISTINCT *)`, design §1); every other set function takes an argument
/// (a bare variable → [`AggArg::Var`], else a lowered constant expression →
/// [`AggArg::Expr`]). `GROUP_CONCAT`/`SAMPLE` need the M6 [`AggKind`] extension that
/// does not exist yet → a tracked sound-501 (do not invent the variant).
fn lower_agg_def(out: &Variable, expr: &AggregateExpression) -> Result<AggDef> {
    let var: Var = out.as_str().into();
    match expr {
        // COUNT(*) / COUNT(DISTINCT *) — no argument column; result xsd:integer.
        AggregateExpression::CountSolutions { distinct } => Ok(AggDef {
            var,
            kind: AggKind::Count,
            arg: None,
            distinct: *distinct,
            fixed_type: Some(XsdTypeCode::Integer),
        }),
        AggregateExpression::FunctionCall {
            name,
            expr,
            distinct,
        } => {
            // COUNT pins xsd:integer; SUM/AVG/MIN/MAX take the value's resolved §10
            // type at reconstruction (None) — mirrors the flat `lower_aggregate`.
            let (kind, fixed_type) = match name {
                AggregateFunction::Count => (AggKind::Count, Some(XsdTypeCode::Integer)),
                AggregateFunction::Sum => (AggKind::Sum, None),
                AggregateFunction::Avg => (AggKind::Avg, None),
                AggregateFunction::Min => (AggKind::Min, None),
                AggregateFunction::Max => (AggKind::Max, None),
                AggregateFunction::GroupConcat { .. } => {
                    return Err(Error::Unsupported(
                        "GROUP_CONCAT needs AggKind::GroupConcat (M6) → 501".to_owned(),
                    ))
                }
                AggregateFunction::Sample => {
                    return Err(Error::Unsupported(
                        "SAMPLE needs AggKind::Sample (M6) → 501".to_owned(),
                    ))
                }
                AggregateFunction::Custom(_) => {
                    return Err(Error::Unsupported(
                        "custom aggregate function → 501".to_owned(),
                    ))
                }
            };
            Ok(AggDef {
                var,
                kind,
                arg: Some(lower_agg_arg(expr)?),
                distinct: *distinct,
                fixed_type,
            })
        }
    }
}

/// An aggregate's argument: a bare variable → [`AggArg::Var`] (resolved context-free
/// — it is only a name); any other expression → [`AggArg::Expr`] over a lowered
/// [`TermDef`] (so a constant lowers; `SUM(?a + ?b)` defers, module docs class 2).
fn lower_agg_arg(expr: &Expression) -> Result<AggArg> {
    match expr {
        Expression::Variable(v) => Ok(AggArg::Var(v.as_str().into())),
        other => Ok(AggArg::Expr(lower_expr_to_termdef(other)?)),
    }
}

/// Reuse the flat ORDER BY lowering (design §2 / iq.rs [`OrderKey`]): a variable key
/// stores `expr: None`; a complex expression key stores the cloned [`Expression`]
/// under a synthetic `__sf_ord_{n}` variable that exec evaluates before sorting.
fn order_keys(expression: &[OrderExpression]) -> Vec<OrderKey> {
    let mut keys = Vec::with_capacity(expression.len());
    for oe in expression {
        let (expr, descending) = match oe {
            OrderExpression::Asc(e) => (e, false),
            OrderExpression::Desc(e) => (e, true),
        };
        match expr {
            Expression::Variable(v) => keys.push(OrderKey {
                var: v.as_str().to_owned(),
                descending,
                expr: None,
            }),
            other => {
                let syn = format!("__sf_ord_{}", keys.len());
                keys.push(OrderKey {
                    var: syn,
                    descending,
                    expr: Some(Box::new(other.clone())),
                });
            }
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use spargebra::algebra::GraphPattern;
    use spargebra::term::{GroundTerm, Literal, TermPattern};

    /// Parse a query and return its top-level WHERE graph pattern.
    fn pattern(q: &str) -> GraphPattern {
        match spargebra::SparqlParser::new().parse_query(q).unwrap() {
            spargebra::Query::Select { pattern, .. } => pattern,
            other => panic!("expected SELECT, got {other:?}"),
        }
    }

    fn var(v: &str) -> Variable {
        Variable::new(v).unwrap()
    }

    fn iri(s: &str) -> NamedNode {
        NamedNode::new(s).unwrap()
    }

    fn triple(s: &str, p: &str, o: &str) -> TriplePattern {
        TriplePattern {
            subject: TermPattern::Variable(var(s)),
            predicate: NamedNodePattern::Variable(var(p)),
            object: TermPattern::Variable(var(o)),
        }
    }

    fn bgp(tps: Vec<TriplePattern>) -> GraphPattern {
        GraphPattern::Bgp { patterns: tps }
    }

    #[test]
    fn empty_bgp_is_true() {
        assert!(matches!(
            build_tree(&bgp(vec![]), None).unwrap(),
            IqNode::True
        ));
    }

    #[test]
    fn single_triple_is_intensional_leaf() {
        let t = build_tree(&bgp(vec![triple("s", "p", "o")]), None).unwrap();
        assert!(matches!(t, IqNode::Intensional { graph: None, .. }));
        assert_eq!(t.output_vars(), vec_var(&["s", "p", "o"]));
    }

    #[test]
    fn multi_triple_bgp_is_inner_join_of_intensionals() {
        let t = build_tree(
            &bgp(vec![triple("s", "p", "o"), triple("o", "p2", "o2")]),
            None,
        )
        .unwrap();
        let IqNode::InnerJoin { children, cond } = &t else {
            panic!("expected InnerJoin, got {t:?}");
        };
        assert!(cond.is_empty());
        assert_eq!(children.len(), 2);
        assert!(children
            .iter()
            .all(|c| matches!(c, IqNode::Intensional { .. })));
        // scope is the de-duplicated union (?o appears in both triples, listed once).
        assert_eq!(t.output_vars(), vec_var(&["s", "p", "o", "p2", "o2"]));
    }

    #[test]
    fn join_builds_one_inner_join_no_distribution() {
        let t = build_tree(&pattern("SELECT * WHERE { ?s ?p ?o . { ?a ?b ?c } }"), None).unwrap();
        // Project over the join; the join is a single InnerJoin (no eager cartesian).
        let IqNode::Construction { child, .. } = &t else {
            panic!("expected Project Construction, got {t:?}");
        };
        assert!(matches!(child.as_ref(), IqNode::InnerJoin { .. }));
    }

    #[test]
    fn union_project_is_dedup_union_of_arm_scopes() {
        let t = build_tree(
            &pattern("SELECT * WHERE { { ?s ?p ?o } UNION { ?s ?p ?x } }"),
            None,
        )
        .unwrap();
        let IqNode::Construction { child, .. } = &t else {
            panic!("expected Project Construction, got {t:?}");
        };
        let IqNode::Union { children, project } = child.as_ref() else {
            panic!("expected Union, got {child:?}");
        };
        assert_eq!(children.len(), 2);
        // shared ?s/?p listed once; ?o then ?x in stable arm order.
        assert_eq!(*project, vec_var(&["s", "p", "o", "x"]));
    }

    #[test]
    fn optional_builds_left_join_empty_cond() {
        let t = build_tree(
            &pattern("SELECT * WHERE { ?s ?p ?o OPTIONAL { ?o ?p2 ?x } }"),
            None,
        )
        .unwrap();
        let IqNode::Construction { child, .. } = &t else {
            panic!("expected Project, got {t:?}");
        };
        let IqNode::LeftJoin { cond, .. } = child.as_ref() else {
            panic!("expected LeftJoin, got {child:?}");
        };
        assert!(cond.is_empty(), "no ON-expression ⇒ empty cond");
    }

    #[test]
    fn left_join_with_expr_lowers_on_condition() {
        // OPTIONAL with an ON-expression: the expr lowers into the LeftJoin `cond`
        // (design §2 LeftJoin arm). EXISTS builds a first-class subtree
        // (IqCond::Exists); a comparison leaf would 501 (filter_pushable_leaf...).
        let lj = GraphPattern::LeftJoin {
            left: Box::new(bgp(vec![triple("s", "p", "o")])),
            right: Box::new(bgp(vec![triple("o", "p2", "x")])),
            expression: Some(Expression::Exists(Box::new(bgp(vec![triple(
                "x", "p3", "y",
            )])))),
        };
        let t = build_tree(&lj, None).unwrap();
        let IqNode::LeftJoin { cond, .. } = &t else {
            panic!("expected LeftJoin, got {t:?}");
        };
        assert!(matches!(cond.as_slice(), [IqCond::Exists(_)]));
    }

    #[test]
    fn minus_builds_filter_not_exists() {
        let t = build_tree(
            &pattern("SELECT * WHERE { ?s ?p ?o MINUS { ?s ?p2 ?x } }"),
            None,
        )
        .unwrap();
        let IqNode::Construction { child, .. } = &t else {
            panic!("expected Project, got {t:?}");
        };
        let IqNode::Filter { cond, .. } = child.as_ref() else {
            panic!("expected Filter, got {child:?}");
        };
        assert!(matches!(
            cond.as_slice(),
            [IqCond::NotExists { is_minus: true, .. }]
        ));
    }

    /// The missing companion to `minus_builds_filter_not_exists` above: `MINUS`
    /// and `FILTER NOT EXISTS` build to the SAME `IqCond::NotExists` shape but
    /// must carry a DIFFERENT `is_minus` — this distinction not being exercised
    /// anywhere at build-time is exactly the gap that let a genuine `=_bag` bug
    /// (silently treating FILTER NOT EXISTS as if it had MINUS's disjoint-domain
    /// no-op) go untested through this whole layer.
    #[test]
    fn filter_not_exists_builds_not_exists_non_minus() {
        let t = build_tree(
            &pattern("SELECT * WHERE { ?s ?p ?o FILTER NOT EXISTS { ?s ?p2 ?x } }"),
            None,
        )
        .unwrap();
        let IqNode::Construction { child, .. } = &t else {
            panic!("expected Project, got {t:?}");
        };
        let IqNode::Filter { cond, .. } = child.as_ref() else {
            panic!("expected Filter, got {child:?}");
        };
        assert!(matches!(
            cond.as_slice(),
            [IqCond::NotExists {
                is_minus: false,
                ..
            }]
        ));
    }

    #[test]
    fn filter_exists_builds_exists_subtree() {
        let t = build_tree(
            &pattern("SELECT * WHERE { ?s ?p ?o FILTER EXISTS { ?s ?p2 ?x } }"),
            None,
        )
        .unwrap();
        let IqNode::Construction { child, .. } = &t else {
            panic!("expected Project, got {t:?}");
        };
        let IqNode::Filter { cond, .. } = child.as_ref() else {
            panic!("expected Filter, got {child:?}");
        };
        assert!(matches!(cond.as_slice(), [IqCond::Exists(_)]));
    }

    #[test]
    fn filter_pushable_leaf_is_symbolic_expr() {
        // A comparison is carried SYMBOLIC (IqCond::Expr), resolved per leaf-CQ at LOWER
        // (M3 design §2.1) — no longer a build-time 501.
        let t = build_tree(&pattern("SELECT * WHERE { ?s ?p ?o FILTER(?o > 5) }"), None).unwrap();
        let IqNode::Construction { child, .. } = &t else {
            panic!("expected Project, got {t:?}");
        };
        let IqNode::Filter { cond, .. } = child.as_ref() else {
            panic!("expected Filter, got {child:?}");
        };
        assert!(matches!(cond.as_slice(), [IqCond::Expr(_)]), "{cond:?}");
    }

    #[test]
    fn constant_graph_pushes_onto_intensional_leaf() {
        let t = build_tree(
            &pattern("SELECT * WHERE { GRAPH <http://g/> { ?s ?p ?o } }"),
            None,
        )
        .unwrap();
        let IqNode::Construction { child, .. } = &t else {
            panic!("expected Project, got {t:?}");
        };
        assert!(matches!(
            child.as_ref(),
            IqNode::Intensional { graph: Some(g), .. } if g.as_str() == "http://g/"
        ));
    }

    #[test]
    fn variable_graph_is_501() {
        let r = build_tree(&pattern("SELECT * WHERE { GRAPH ?g { ?s ?p ?o } }"), None);
        assert!(matches!(r, Err(Error::Unsupported(_))), "{r:?}");
    }

    #[test]
    fn path_closure_builds_unresolved_path_leaf() {
        // A genuine closure (`+`/`*`/`?`/`!`) is no longer a build-time 501: it builds
        // an `UnresolvedPath` leaf carrying the verbatim path components, which RESOLVE
        // compiles via the flat `path_branch` (M5 Wave 1). The subject/object vars are
        // published as the leaf's scope.
        for q in [
            "SELECT * WHERE { ?s <http://p>+ ?o }",
            "SELECT * WHERE { ?s <http://p>* ?o }",
            "SELECT * WHERE { ?s <http://p>? ?o }",
            "SELECT * WHERE { ?s !<http://p> ?o }",
        ] {
            // strip the SELECT * Project Construction the parser wraps the WHERE in.
            let t = match build_tree(&pattern(q), None).unwrap() {
                IqNode::Construction { child, .. } => *child,
                other => other,
            };
            assert!(
                matches!(t, IqNode::UnresolvedPath { graph: None, .. }),
                "{q}: {t:?}"
            );
            assert_eq!(t.output_vars(), vec_var(&["s", "o"]));
        }
    }

    #[test]
    fn fixed_predicate_path_builds_intensional() {
        // A length-1 NamedNode path (≡ one triple) → an Intensional leaf.
        let p = GraphPattern::Path {
            subject: TermPattern::Variable(var("s")),
            path: PropertyPathExpression::NamedNode(iri("http://p")),
            object: TermPattern::Variable(var("o")),
        };
        let t = build_tree(&p, None).unwrap();
        assert!(matches!(t, IqNode::Intensional { .. }));
        assert_eq!(t.output_vars(), vec_var(&["s", "o"]));
    }

    #[test]
    fn extend_constant_builds_construction() {
        let inner = bgp(vec![triple("s", "p", "o")]);
        let e = GraphPattern::Extend {
            inner: Box::new(inner),
            variable: var("c"),
            expression: Expression::NamedNode(iri("http://x")),
        };
        let t = build_tree(&e, None).unwrap();
        let IqNode::Construction { subst, project, .. } = &t else {
            panic!("expected Construction, got {t:?}");
        };
        // BIND is carried symbolic (BindDef::Expr) and resolved at LOWER (M3 §2.2).
        assert!(matches!(subst.get("c"), Some(BindDef::Expr(_))));
        assert_eq!(*project, vec_var(&["s", "p", "o", "c"]));
    }

    #[test]
    fn extend_variable_expression_is_symbolic_bind() {
        // BIND(?o AS ?c): a variable term is carried SYMBOLIC (BindDef::Expr), resolved
        // per leaf-CQ at LOWER (M3 design §2.2) — no longer a build-time 501.
        let e = GraphPattern::Extend {
            inner: Box::new(bgp(vec![triple("s", "p", "o")])),
            variable: var("c"),
            expression: Expression::Variable(var("o")),
        };
        let t = build_tree(&e, None).unwrap();
        let IqNode::Construction { subst, .. } = &t else {
            panic!("expected Construction, got {t:?}");
        };
        assert!(matches!(subst.get("c"), Some(BindDef::Expr(_))));
    }

    #[test]
    fn group_builds_aggregation_with_scope() {
        let g = GraphPattern::Group {
            inner: Box::new(bgp(vec![triple("s", "p", "o")])),
            variables: vec![var("s")],
            aggregates: vec![(
                var("c"),
                AggregateExpression::CountSolutions { distinct: false },
            )],
        };
        let t = build_tree(&g, None).unwrap();
        let IqNode::Aggregation { grouping, aggs, .. } = &t else {
            panic!("expected Aggregation, got {t:?}");
        };
        assert_eq!(*grouping, vec_var(&["s"]));
        assert!(matches!(
            aggs.as_slice(),
            [AggDef {
                kind: AggKind::Count,
                arg: None,
                distinct: false,
                ..
            }]
        ));
        // the node owns its scope: grouping keys ++ aggregate output vars.
        assert_eq!(t.output_vars(), vec_var(&["s", "c"]));
    }

    #[test]
    fn group_sum_of_variable_builds_agg_arg_var() {
        // SUM(?o): a bare-variable argument resolves context-free to AggArg::Var, and
        // SUM pins no result type (fixed_type None — unlike COUNT's xsd:integer).
        let g = GraphPattern::Group {
            inner: Box::new(bgp(vec![triple("s", "p", "o")])),
            variables: vec![var("s")],
            aggregates: vec![(
                var("t"),
                AggregateExpression::FunctionCall {
                    name: AggregateFunction::Sum,
                    expr: Expression::Variable(var("o")),
                    distinct: false,
                },
            )],
        };
        let t = build_tree(&g, None).unwrap();
        let IqNode::Aggregation { aggs, .. } = &t else {
            panic!("expected Aggregation, got {t:?}");
        };
        assert!(matches!(
            aggs.as_slice(),
            [AggDef {
                kind: AggKind::Sum,
                arg: Some(AggArg::Var(_)),
                distinct: false,
                fixed_type: None,
                ..
            }]
        ));
        // grouping key ++ the aggregate output var.
        assert_eq!(t.output_vars(), vec_var(&["s", "t"]));
    }

    #[test]
    fn count_distinct_star_rides_distinct_flag() {
        // The tree expresses COUNT(DISTINCT *) (the flat model 501'd it).
        let g = GraphPattern::Group {
            inner: Box::new(bgp(vec![triple("s", "p", "o")])),
            variables: vec![],
            aggregates: vec![(
                var("c"),
                AggregateExpression::CountSolutions { distinct: true },
            )],
        };
        let t = build_tree(&g, None).unwrap();
        let IqNode::Aggregation { aggs, .. } = &t else {
            panic!("expected Aggregation");
        };
        assert!(matches!(
            aggs.as_slice(),
            [AggDef {
                kind: AggKind::Count,
                arg: None,
                distinct: true,
                ..
            }]
        ));
    }

    #[test]
    fn group_concat_and_sample_are_tracked_501() {
        for f in [
            AggregateFunction::GroupConcat { separator: None },
            AggregateFunction::Sample,
        ] {
            let g = GraphPattern::Group {
                inner: Box::new(bgp(vec![triple("s", "p", "o")])),
                variables: vec![],
                aggregates: vec![(
                    var("c"),
                    AggregateExpression::FunctionCall {
                        name: f,
                        expr: Expression::Variable(var("o")),
                        distinct: false,
                    },
                )],
            };
            assert!(matches!(build_tree(&g, None), Err(Error::Unsupported(_))));
        }
    }

    #[test]
    fn project_builds_construction_with_empty_subst() {
        // Project(P, vars) → a Construction that adds NO substitution, carrying only
        // the declared projection (design §2 Project arm); its output scope is exactly
        // that projection.
        let p = GraphPattern::Project {
            inner: Box::new(bgp(vec![triple("s", "p", "o")])),
            variables: vec![var("s"), var("o")],
        };
        let t = build_tree(&p, None).unwrap();
        let IqNode::Construction {
            subst,
            project,
            child,
        } = &t
        else {
            panic!("expected Construction, got {t:?}");
        };
        assert!(subst.is_empty(), "Project adds no substitution");
        assert_eq!(*project, vec_var(&["s", "o"]));
        assert!(matches!(child.as_ref(), IqNode::Intensional { .. }));
        assert_eq!(t.output_vars(), vec_var(&["s", "o"]));
    }

    #[test]
    fn distinct_and_reduced_both_build_distinct() {
        // spargebra wraps the projection: `SELECT DISTINCT *` ⇒ Distinct{ Project{…} },
        // so the built tree is Distinct over a Construction.
        let d = build_tree(&pattern("SELECT DISTINCT * WHERE { ?s ?p ?o }"), None).unwrap();
        let IqNode::Distinct { child } = &d else {
            panic!("expected Distinct, got {d:?}");
        };
        assert!(matches!(child.as_ref(), IqNode::Construction { .. }));

        // REDUCED also builds a Distinct (REDUCED may dedup — sound).
        let r = build_tree(&pattern("SELECT REDUCED * WHERE { ?s ?p ?o }"), None).unwrap();
        let IqNode::Distinct { child } = &r else {
            panic!("expected Distinct, got {r:?}");
        };
        assert!(matches!(child.as_ref(), IqNode::Construction { .. }));
    }

    #[test]
    fn slice_carries_offset_and_limit() {
        let t = build_tree(
            &pattern("SELECT * WHERE { ?s ?p ?o } LIMIT 5 OFFSET 2"),
            None,
        )
        .unwrap();
        // Slice sits over the Project (spargebra: Slice{ Project{ ... } }).
        let IqNode::Slice { offset, limit, .. } = &t else {
            panic!("expected Slice, got {t:?}");
        };
        assert_eq!(*offset, 2);
        assert_eq!(*limit, Some(5));
    }

    #[test]
    fn order_by_variable_and_expression_keys() {
        let t = build_tree(
            &pattern("SELECT * WHERE { ?s ?p ?o } ORDER BY ?o DESC(STRLEN(?o))"),
            None,
        )
        .unwrap();
        let IqNode::Construction { child, .. } = &t else {
            panic!("expected Project, got {t:?}");
        };
        let IqNode::OrderBy { keys, .. } = child.as_ref() else {
            panic!("expected OrderBy, got {child:?}");
        };
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].var, "o");
        assert!(keys[0].expr.is_none() && !keys[0].descending);
        assert!(keys[1].expr.is_some() && keys[1].descending);
    }

    #[test]
    fn values_lowers_const_and_undef_cells() {
        let v = GraphPattern::Values {
            variables: vec![var("x")],
            bindings: vec![
                vec![Some(GroundTerm::Literal(Literal::new_simple_literal("a")))],
                vec![None],
            ],
        };
        let t = build_tree(&v, None).unwrap();
        let IqNode::Values { vars, rows } = &t else {
            panic!("expected Values, got {t:?}");
        };
        assert_eq!(*vars, vec_var(&["x"]));
        assert!(matches!(rows[0].as_slice(), [Some(TermDef::Const(_))]));
        assert!(matches!(rows[1].as_slice(), [None]));
        assert_eq!(t.output_vars(), vec_var(&["x"]));
    }

    #[test]
    fn unsupported_pattern_is_501() {
        // SERVICE is out of v1 coverage → 501 (never silently dropped).
        let r = build_tree(
            &pattern("SELECT * WHERE { SERVICE <http://x/> { ?s ?p ?o } }"),
            None,
        );
        assert!(matches!(r, Err(Error::Unsupported(_))), "{r:?}");
    }

    #[test]
    fn output_vars_flows_through_a_representative_tree() {
        // Capstone over IqNode::output_vars (design §1 scope rules): Slice/Distinct are
        // scope-transparent, and a LeftJoin keeps right-only vars in scope (nullable),
        // de-duplicating the shared ?o. Assemble Slice{ Distinct{ LeftJoin{ {s,p,o},
        // {o,p2,x} } } } from builder-produced leaves and assert the merged scope.
        let tree = IqNode::Slice {
            child: Box::new(IqNode::Distinct {
                child: Box::new(IqNode::LeftJoin {
                    left: Box::new(build_tree(&bgp(vec![triple("s", "p", "o")]), None).unwrap()),
                    right: Box::new(build_tree(&bgp(vec![triple("o", "p2", "x")]), None).unwrap()),
                    cond: Vec::new(),
                }),
            }),
            offset: 0,
            limit: Some(10),
        };
        assert_eq!(tree.output_vars(), vec_var(&["s", "p", "o", "p2", "x"]));
    }

    /// `Vec<Var>` from a slice of names (test ergonomics).
    fn vec_var(names: &[&str]) -> Vec<Var> {
        names.iter().map(|s| (*s).into()).collect()
    }
}
