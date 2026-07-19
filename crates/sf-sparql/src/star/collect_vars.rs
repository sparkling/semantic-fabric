//! Whole-pattern variable collection: [`collect_pattern_vars`], the one
//! entry point [`super::top_level::rewrite_union`] uses for its uniform-
//! composed-ness agreement check (which variables do a `Union`'s two arms
//! BOTH syntactically mention?). Split out from the pattern walker itself
//! ([`super::walk`]) because it is a completely different kind of traversal —
//! collecting names rather than rewriting structure — with its own recursion
//! shape mirroring `GraphPattern`/`Expression` one-for-one.

use std::collections::BTreeSet;

use spargebra::algebra::{AggregateExpression, Expression, GraphPattern, OrderExpression};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern, Variable};

/// Every [`Variable`] mentioned anywhere in `gp` — triple-pattern subject/
/// object (recursing into a nested quoted triple), VALUES/Extend/Group/Path
/// variables, and `Expression::Variable`/`Bound` references (recursing into
/// EXISTS bodies) — used by [`super::top_level::rewrite_union`]'s uniform-
/// composed-ness check. Deliberately broad (a var mentioned only in a FILTER
/// still counts): a false positive here costs only an unnecessary — but
/// harmless — agreement check; missing a real disagreement would not be
/// sound.
pub(super) fn collect_pattern_vars(gp: &GraphPattern) -> BTreeSet<Variable> {
    let mut out = BTreeSet::new();
    collect_pattern_vars_into(gp, &mut out);
    out
}

fn collect_pattern_vars_into(gp: &GraphPattern, out: &mut BTreeSet<Variable>) {
    match gp {
        GraphPattern::Bgp { patterns } => {
            for tp in patterns {
                collect_triple_vars(tp, out);
            }
        }
        GraphPattern::Path {
            subject, object, ..
        } => {
            collect_term_pattern_vars(subject, out);
            collect_term_pattern_vars(object, out);
        }
        GraphPattern::Join { left, right }
        | GraphPattern::Lateral { left, right }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right } => {
            collect_pattern_vars_into(left, out);
            collect_pattern_vars_into(right, out);
        }
        GraphPattern::LeftJoin {
            left,
            right,
            expression,
        } => {
            collect_pattern_vars_into(left, out);
            collect_pattern_vars_into(right, out);
            if let Some(e) = expression {
                collect_expr_vars(e, out);
            }
        }
        GraphPattern::Filter { expr, inner } => {
            collect_expr_vars(expr, out);
            collect_pattern_vars_into(inner, out);
        }
        GraphPattern::Graph { name, inner } => {
            if let NamedNodePattern::Variable(v) = name {
                out.insert(v.clone());
            }
            collect_pattern_vars_into(inner, out);
        }
        GraphPattern::Extend {
            inner,
            variable,
            expression,
        } => {
            out.insert(variable.clone());
            collect_expr_vars(expression, out);
            collect_pattern_vars_into(inner, out);
        }
        GraphPattern::Values { variables, .. } => {
            out.extend(variables.iter().cloned());
        }
        GraphPattern::OrderBy { inner, expression } => {
            collect_pattern_vars_into(inner, out);
            for oe in expression {
                let (OrderExpression::Asc(e) | OrderExpression::Desc(e)) = oe;
                collect_expr_vars(e, out);
            }
        }
        GraphPattern::Project { inner, variables } => {
            out.extend(variables.iter().cloned());
            collect_pattern_vars_into(inner, out);
        }
        GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. } => {
            collect_pattern_vars_into(inner, out);
        }
        GraphPattern::Group {
            inner,
            variables,
            aggregates,
        } => {
            out.extend(variables.iter().cloned());
            for (v, ae) in aggregates {
                out.insert(v.clone());
                if let AggregateExpression::FunctionCall { expr, .. } = ae {
                    collect_expr_vars(expr, out);
                }
            }
            collect_pattern_vars_into(inner, out);
        }
        GraphPattern::Service { inner, .. } => {
            collect_pattern_vars_into(inner, out);
        }
    }
}

fn collect_triple_vars(tp: &TriplePattern, out: &mut BTreeSet<Variable>) {
    collect_term_pattern_vars(&tp.subject, out);
    if let NamedNodePattern::Variable(v) = &tp.predicate {
        out.insert(v.clone());
    }
    collect_term_pattern_vars(&tp.object, out);
}

fn collect_term_pattern_vars(t: &TermPattern, out: &mut BTreeSet<Variable>) {
    match t {
        TermPattern::Variable(v) => {
            out.insert(v.clone());
        }
        TermPattern::Triple(tp) => collect_triple_vars(tp, out),
        _ => {}
    }
}

fn collect_expr_vars(e: &Expression, out: &mut BTreeSet<Variable>) {
    use Expression::*;
    match e {
        Variable(v) | Bound(v) => {
            out.insert(v.clone());
        }
        NamedNode(_) | Literal(_) => {}
        Or(a, b)
        | And(a, b)
        | Equal(a, b)
        | SameTerm(a, b)
        | Greater(a, b)
        | GreaterOrEqual(a, b)
        | Less(a, b)
        | LessOrEqual(a, b)
        | Add(a, b)
        | Subtract(a, b)
        | Multiply(a, b)
        | Divide(a, b) => {
            collect_expr_vars(a, out);
            collect_expr_vars(b, out);
        }
        In(a, list) => {
            collect_expr_vars(a, out);
            for e in list {
                collect_expr_vars(e, out);
            }
        }
        UnaryPlus(a) | UnaryMinus(a) | Not(a) => collect_expr_vars(a, out),
        Exists(gp) => collect_pattern_vars_into(gp, out),
        If(a, b, c) => {
            collect_expr_vars(a, out);
            collect_expr_vars(b, out);
            collect_expr_vars(c, out);
        }
        Coalesce(list) => {
            for e in list {
                collect_expr_vars(e, out);
            }
        }
        FunctionCall(_, args) => {
            for e in args {
                collect_expr_vars(e, out);
            }
        }
    }
}
