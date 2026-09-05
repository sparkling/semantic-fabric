//! Whole-pattern variable collection: [`collect_pattern_vars`], shared by
//! rewrites that must distinguish generated variables from every authored
//! variable occurrence. Split out from the pattern walker itself
//! ([`super::walk`]) because it is a completely different kind of traversal —
//! collecting names rather than rewriting structure — with its own recursion
//! shape mirroring `GraphPattern`/`Expression` one-for-one.

use std::collections::BTreeSet;

use spargebra::algebra::{AggregateExpression, Expression, GraphPattern, OrderExpression};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern, Variable};
use spargebra::Query;

/// Every variable name already present in a parsed query before this module
/// introduces its own bindings. In addition to the query pattern, CONSTRUCT's
/// separate template must be included: a template-only variable is unbound by
/// definition, but capturing it with a generated component variable would make
/// it spuriously bound and change the produced graph.
pub(super) fn collect_query_vars(query: &Query) -> BTreeSet<Variable> {
    let mut out = match query {
        Query::Select { pattern, .. }
        | Query::Describe { pattern, .. }
        | Query::Ask { pattern, .. }
        | Query::Construct { pattern, .. } => collect_pattern_vars(pattern),
    };
    if let Query::Construct { template, .. } = query {
        for triple in template {
            collect_triple_vars(triple, &mut out);
        }
    }
    out
}

/// Every [`Variable`] mentioned anywhere in `gp` — triple-pattern subject/
/// object (recursing into a nested quoted triple), VALUES/Extend/Group/Path
/// variables, and `Expression::Variable`/`Bound` references (recursing into
/// EXISTS bodies) — used by [`super::top_level::rewrite_union`]'s uniform-
/// composed-ness check. Deliberately broad (a var mentioned only in a FILTER
/// still counts). Missing an occurrence can make a generated binding capture
/// authored syntax, so this traversal deliberately follows the complete
/// `GraphPattern` and `Expression` trees.
pub(crate) fn collect_pattern_vars(gp: &GraphPattern) -> BTreeSet<Variable> {
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
        GraphPattern::Graph { name, inner }
        | GraphPattern::Service {
            name,
            inner,
            silent: _,
        } => {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_inventory_includes_graph_service_and_extend_binding_sites() {
        let query = spargebra::SparqlParser::new()
            .parse_query(
                "SELECT ?s WHERE { \
                 GRAPH ?__sf_star_0 { ?s ?p ?o } \
                 BIND(?s AS ?__sf_star_1) \
                 SERVICE ?__sf_star_2 { ?a ?b ?c } \
                 }",
            )
            .expect("all three binding sites are legal SPARQL");

        let variables = collect_query_vars(&query);
        for name in ["__sf_star_0", "__sf_star_1", "__sf_star_2"] {
            assert!(
                variables.contains(&Variable::new_unchecked(name)),
                "missing binding-site variable {name}"
            );
        }
    }
}
