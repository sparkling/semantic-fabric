//! ADR-0031 — RDF-star query rewrite: a `GraphPattern → GraphPattern` pre-pass
//! that desugars quoted-triple patterns onto the `ADR-0029` basic encoding,
//! applied once at the top of both `translate_tree` and `translate_inner_flat`
//! (`lib.rs`) — mirrors the DESCRIBE→CBD rewrite already living there (a
//! recursive algebra rebuild minting `__sf_`-prefixed synthetic variables), so
//! `build.rs`/`iq/*.rs`/`unfold.rs`/`cascade/`/`emit.rs` never see a
//! `TermPattern::Triple` at all (R1).
//!
//! Ground truth (pinned `spargebra 0.4.6+sparql-12`, ADR-0031 Context): bare
//! `<<s p o>>` is parser-desugared to `_:b rdf:reifies <<( s p o )>>` + `_:b` at
//! the original position; parenthesized `<<( s p o )>>` yields
//! `TermPattern::Triple` in place. The reifies wrapper must be recognized and
//! elided (R2), not translated — `rdf:reifies` is unmapped, so a mechanical
//! per-position replacement would silently unfold to zero rows forever.
//!
//! Rewrite rules per triple pattern (ADR-0031 Decision Outcome, order matters):
//! (1) subject-is-Triple → fresh identity var + 4 basic-encoding patterns;
//! (2) THEN if predicate is `rdf:reifies` and object is Triple → drop the
//! triple, emit the 4 patterns on the (possibly rule-1-substituted) subject;
//! (3) else object-is-Triple → fresh identity var + 4 patterns, symmetric with
//! (1); (4) the 4 patterns copy the quoted s/p/o verbatim; (5) a quoted
//! triple's own subject/object being ANOTHER quoted triple → `Unsupported`
//! (v1 does not nest); (6) recursion covers every `GraphPattern` container,
//! `Expression::Exists` bodies, and `GraphPattern::Path` endpoints (a fresh
//! var + the 4 patterns joined alongside the path node). `GraphPattern::Values`
//! is untouched (v1 boundary 7 — a ground quoted triple already 501s at
//! `unfold::ground_term_to_term`).

use spargebra::algebra::{
    AggregateExpression, Expression, GraphPattern, OrderExpression, PropertyPathExpression,
};
use spargebra::term::{NamedNode, NamedNodePattern, TermPattern, TriplePattern, Variable};
use spargebra::Query;

use crate::unfold::RDF_TYPE;
use crate::{Error, Result};

/// RDF 1.2's native "reifies" predicate (`oxrdf::vocab::rdf::REIFIES`, cited
/// verbatim in ADR-0031's Context) — `oxrdf` itself is only a dev-dependency
/// of this crate (everything else reaches these vocab IRIs through spargebra's
/// re-exported types), so this is hand-declared like every other vocabulary
/// constant in this crate (`unfold::RDF_TYPE`, `sf-mapping`'s r2rml.rs consts).
const RDF_REIFIES: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies";

// --- RDF 1.2 Interoperability "basic encoding" vocabulary (ADR-0029 §B.2) —
// MUST match `crates/sf-mapping/src/r2rml.rs`'s consts of the same name
// exactly (a different crate, so not shared by import): `sf-mapping` compiles
// `rml:StarMap` mappings onto these same predicates, so a query asking for a
// different IRI would never match a single mapped triple. `rdf:type` reuses
// `unfold::RDF_TYPE` rather than a third copy of the same string.
const RDF_PROPOSITION_FORM: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#PropositionForm";
const RDF_PROPOSITION_FORM_SUBJECT: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormSubject";
const RDF_PROPOSITION_FORM_PREDICATE: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormPredicate";
const RDF_PROPOSITION_FORM_OBJECT: &str =
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormObject";

/// Rewrite a whole query's WHERE pattern (rules R1-R5), threading one
/// whole-query fresh-variable counter (R3 — never the per-clause `__sf_ord`
/// pattern, which would collide across sibling BGPs/UNION arms/EXISTS bodies).
/// The CONSTRUCT template (a separate `Vec<TriplePattern>`, not a
/// `GraphPattern`) is untouched here — see [`construct_template_has_quoted_triple`]
/// for that v1 boundary (rule 9).
pub fn rewrite_query(query: &Query) -> Result<Query> {
    let mut n = 0usize;
    Ok(match query {
        Query::Select {
            dataset,
            pattern,
            base_iri,
        } => Query::Select {
            dataset: dataset.clone(),
            pattern: rewrite_pattern(pattern, &mut n)?,
            base_iri: base_iri.clone(),
        },
        Query::Construct {
            template,
            dataset,
            pattern,
            base_iri,
        } => Query::Construct {
            template: template.clone(),
            dataset: dataset.clone(),
            pattern: rewrite_pattern(pattern, &mut n)?,
            base_iri: base_iri.clone(),
        },
        Query::Describe {
            dataset,
            pattern,
            base_iri,
        } => Query::Describe {
            dataset: dataset.clone(),
            pattern: rewrite_pattern(pattern, &mut n)?,
            base_iri: base_iri.clone(),
        },
        Query::Ask {
            dataset,
            pattern,
            base_iri,
        } => Query::Ask {
            dataset: dataset.clone(),
            pattern: rewrite_pattern(pattern, &mut n)?,
            base_iri: base_iri.clone(),
        },
    })
}

/// Recurse through every `GraphPattern` container (rule 6), rewriting BGP
/// triple patterns (rules 1-5) and property-path endpoints (rule 6b) as they
/// are found. `Values` is returned unchanged (rule 7 — a ground quoted triple
/// is a v1 boundary handled downstream, untouched by this pass).
fn rewrite_pattern(gp: &GraphPattern, n: &mut usize) -> Result<GraphPattern> {
    Ok(match gp {
        GraphPattern::Bgp { patterns } => rewrite_bgp(patterns, n)?,
        GraphPattern::Path {
            subject,
            path,
            object,
        } => rewrite_path(subject, path, object, n)?,
        GraphPattern::Join { left, right } => GraphPattern::Join {
            left: Box::new(rewrite_pattern(left, n)?),
            right: Box::new(rewrite_pattern(right, n)?),
        },
        GraphPattern::LeftJoin {
            left,
            right,
            expression,
        } => GraphPattern::LeftJoin {
            left: Box::new(rewrite_pattern(left, n)?),
            right: Box::new(rewrite_pattern(right, n)?),
            expression: expression
                .as_ref()
                .map(|e| rewrite_expr(e, n))
                .transpose()?,
        },
        GraphPattern::Lateral { left, right } => GraphPattern::Lateral {
            left: Box::new(rewrite_pattern(left, n)?),
            right: Box::new(rewrite_pattern(right, n)?),
        },
        GraphPattern::Filter { expr, inner } => GraphPattern::Filter {
            expr: rewrite_expr(expr, n)?,
            inner: Box::new(rewrite_pattern(inner, n)?),
        },
        GraphPattern::Union { left, right } => GraphPattern::Union {
            left: Box::new(rewrite_pattern(left, n)?),
            right: Box::new(rewrite_pattern(right, n)?),
        },
        GraphPattern::Graph { name, inner } => GraphPattern::Graph {
            name: name.clone(),
            inner: Box::new(rewrite_pattern(inner, n)?),
        },
        GraphPattern::Extend {
            inner,
            variable,
            expression,
        } => GraphPattern::Extend {
            inner: Box::new(rewrite_pattern(inner, n)?),
            variable: variable.clone(),
            expression: rewrite_expr(expression, n)?,
        },
        GraphPattern::Minus { left, right } => GraphPattern::Minus {
            left: Box::new(rewrite_pattern(left, n)?),
            right: Box::new(rewrite_pattern(right, n)?),
        },
        GraphPattern::Values { .. } => gp.clone(),
        GraphPattern::OrderBy { inner, expression } => GraphPattern::OrderBy {
            inner: Box::new(rewrite_pattern(inner, n)?),
            expression: expression
                .iter()
                .map(|oe| rewrite_order_expr(oe, n))
                .collect::<Result<_>>()?,
        },
        GraphPattern::Project { inner, variables } => GraphPattern::Project {
            inner: Box::new(rewrite_pattern(inner, n)?),
            variables: variables.clone(),
        },
        GraphPattern::Distinct { inner } => GraphPattern::Distinct {
            inner: Box::new(rewrite_pattern(inner, n)?),
        },
        GraphPattern::Reduced { inner } => GraphPattern::Reduced {
            inner: Box::new(rewrite_pattern(inner, n)?),
        },
        GraphPattern::Slice {
            inner,
            start,
            length,
        } => GraphPattern::Slice {
            inner: Box::new(rewrite_pattern(inner, n)?),
            start: *start,
            length: *length,
        },
        GraphPattern::Group {
            inner,
            variables,
            aggregates,
        } => GraphPattern::Group {
            inner: Box::new(rewrite_pattern(inner, n)?),
            variables: variables.clone(),
            aggregates: aggregates
                .iter()
                .map(|(v, ae)| Ok((v.clone(), rewrite_agg_expr(ae, n)?)))
                .collect::<Result<_>>()?,
        },
        GraphPattern::Service {
            name,
            inner,
            silent,
        } => GraphPattern::Service {
            name: name.clone(),
            inner: Box::new(rewrite_pattern(inner, n)?),
            silent: *silent,
        },
    })
}

/// Rewrite a BGP: each triple pattern expands (rules 1-5) into zero or more
/// output patterns, concatenated in order into one flat `Bgp` (join order is
/// immaterial — a BGP is an unordered AND of patterns).
fn rewrite_bgp(patterns: &[TriplePattern], n: &mut usize) -> Result<GraphPattern> {
    let mut out = Vec::with_capacity(patterns.len());
    for tp in patterns {
        rewrite_triple(tp, n, &mut out)?;
    }
    Ok(GraphPattern::Bgp { patterns: out })
}

/// Rewrite one triple pattern per rules 1-5, appending its replacement
/// pattern(s) to `out`.
fn rewrite_triple(tp: &TriplePattern, n: &mut usize, out: &mut Vec<TriplePattern>) -> Result<()> {
    // Rule 1: subject-is-Triple → fresh var, its 4 patterns land in `out` now.
    let subject = substitute_triple(&tp.subject, n, out)?;

    // Rule 2 (load-bearing): `X rdf:reifies <<(...)>>`, checked on the
    // rule-1-substituted subject — drop the wrapper triple entirely.
    if is_reifies(&tp.predicate) {
        if let TermPattern::Triple(inner) = &tp.object {
            return emit_basic_encoding(&subject, inner, out);
        }
    }

    // Rule 3: object-is-Triple → fresh var, symmetric with rule 1.
    let object = substitute_triple(&tp.object, n, out)?;
    out.push(TriplePattern {
        subject,
        predicate: tp.predicate.clone(),
        object,
    });
    Ok(())
}

/// If `t` is a quoted-triple pattern, replace it with a fresh `__sf_star_{n}`
/// identity variable and append its 4 basic-encoding patterns (rule 4) to
/// `out` (rule 5's nesting check applies here). Otherwise return `t` as-is.
/// Shared by BGP triple-pattern substitution (rules 1/3) and property-path
/// endpoint substitution (rule 6b).
fn substitute_triple(
    t: &TermPattern,
    n: &mut usize,
    out: &mut Vec<TriplePattern>,
) -> Result<TermPattern> {
    match t {
        TermPattern::Triple(tp) => {
            let fresh = fresh_var(n);
            emit_basic_encoding(&fresh, tp, out)?;
            Ok(fresh)
        }
        other => Ok(other.clone()),
    }
}

/// Rule 4: the 4 basic-encoding patterns binding `identity` to quoted
/// `tp = (s, p, o)`, copying s/p/o verbatim. Rule 5: `tp`'s own subject/object
/// being ANOTHER quoted triple is a nesting depth v1 does not support → 501
/// (`tp.predicate` is structurally a `NamedNodePattern`, so it can never itself
/// be a quoted triple — ADR-0029 R4's mirror on the query side).
fn emit_basic_encoding(
    identity: &TermPattern,
    tp: &TriplePattern,
    out: &mut Vec<TriplePattern>,
) -> Result<()> {
    if matches!(tp.subject, TermPattern::Triple(_)) || matches!(tp.object, TermPattern::Triple(_)) {
        return Err(Error::Unsupported(
            "nested quoted-triple pattern (a quoted triple whose own subject or \
             object is itself a quoted triple) is not supported in v1 → 501 \
             (ADR-0031 rule 5)"
                .to_owned(),
        ));
    }
    out.push(TriplePattern {
        subject: identity.clone(),
        predicate: NamedNodePattern::NamedNode(NamedNode::new_unchecked(RDF_TYPE)),
        object: TermPattern::NamedNode(NamedNode::new_unchecked(RDF_PROPOSITION_FORM)),
    });
    out.push(TriplePattern {
        subject: identity.clone(),
        predicate: NamedNodePattern::NamedNode(NamedNode::new_unchecked(
            RDF_PROPOSITION_FORM_SUBJECT,
        )),
        object: tp.subject.clone(),
    });
    out.push(TriplePattern {
        subject: identity.clone(),
        predicate: NamedNodePattern::NamedNode(NamedNode::new_unchecked(
            RDF_PROPOSITION_FORM_PREDICATE,
        )),
        object: named_node_pattern_to_term_pattern(&tp.predicate),
    });
    out.push(TriplePattern {
        subject: identity.clone(),
        predicate: NamedNodePattern::NamedNode(NamedNode::new_unchecked(
            RDF_PROPOSITION_FORM_OBJECT,
        )),
        object: tp.object.clone(),
    });
    Ok(())
}

/// Rule 6b: a property-path endpoint that is itself a quoted-triple pattern
/// substitutes a fresh identity var (rules 1/3's `substitute_triple`), with
/// its basic-encoding patterns joined alongside the path node — the same
/// `GraphPattern::Join` injection the DESCRIBE→CBD rewrite uses (`lib.rs`).
/// Neither endpoint quoted ⇒ no extra patterns ⇒ the path node is returned
/// unchanged (the common, unaffected case).
fn rewrite_path(
    subject: &TermPattern,
    path: &PropertyPathExpression,
    object: &TermPattern,
    n: &mut usize,
) -> Result<GraphPattern> {
    let mut extra = Vec::new();
    let subject = substitute_triple(subject, n, &mut extra)?;
    let object = substitute_triple(object, n, &mut extra)?;
    let path_node = GraphPattern::Path {
        subject,
        path: path.clone(),
        object,
    };
    Ok(if extra.is_empty() {
        path_node
    } else {
        GraphPattern::Join {
            left: Box::new(GraphPattern::Bgp { patterns: extra }),
            right: Box::new(path_node),
        }
    })
}

/// Rule 6a: recurse through an expression tree looking for `EXISTS`/`NOT
/// EXISTS` bodies (the only `Expression` variant carrying a `GraphPattern`) —
/// reachable from FILTER, BIND, ORDER BY, and OPTIONAL's ON-expression.
/// Structural recursion otherwise (every other variant only carries
/// sub-expressions, never a pattern).
fn rewrite_expr(expr: &Expression, n: &mut usize) -> Result<Expression> {
    use Expression::*;
    Ok(match expr {
        NamedNode(_) | Literal(_) | Variable(_) | Bound(_) => expr.clone(),
        Or(a, b) => Or(Box::new(rewrite_expr(a, n)?), Box::new(rewrite_expr(b, n)?)),
        And(a, b) => And(Box::new(rewrite_expr(a, n)?), Box::new(rewrite_expr(b, n)?)),
        Equal(a, b) => Equal(Box::new(rewrite_expr(a, n)?), Box::new(rewrite_expr(b, n)?)),
        SameTerm(a, b) => SameTerm(Box::new(rewrite_expr(a, n)?), Box::new(rewrite_expr(b, n)?)),
        Greater(a, b) => Greater(Box::new(rewrite_expr(a, n)?), Box::new(rewrite_expr(b, n)?)),
        GreaterOrEqual(a, b) => {
            GreaterOrEqual(Box::new(rewrite_expr(a, n)?), Box::new(rewrite_expr(b, n)?))
        }
        Less(a, b) => Less(Box::new(rewrite_expr(a, n)?), Box::new(rewrite_expr(b, n)?)),
        LessOrEqual(a, b) => {
            LessOrEqual(Box::new(rewrite_expr(a, n)?), Box::new(rewrite_expr(b, n)?))
        }
        In(a, list) => In(
            Box::new(rewrite_expr(a, n)?),
            list.iter()
                .map(|e| rewrite_expr(e, n))
                .collect::<Result<_>>()?,
        ),
        Add(a, b) => Add(Box::new(rewrite_expr(a, n)?), Box::new(rewrite_expr(b, n)?)),
        Subtract(a, b) => Subtract(Box::new(rewrite_expr(a, n)?), Box::new(rewrite_expr(b, n)?)),
        Multiply(a, b) => Multiply(Box::new(rewrite_expr(a, n)?), Box::new(rewrite_expr(b, n)?)),
        Divide(a, b) => Divide(Box::new(rewrite_expr(a, n)?), Box::new(rewrite_expr(b, n)?)),
        UnaryPlus(a) => UnaryPlus(Box::new(rewrite_expr(a, n)?)),
        UnaryMinus(a) => UnaryMinus(Box::new(rewrite_expr(a, n)?)),
        Not(a) => Not(Box::new(rewrite_expr(a, n)?)),
        Exists(gp) => Exists(Box::new(rewrite_pattern(gp, n)?)),
        If(a, b, c) => If(
            Box::new(rewrite_expr(a, n)?),
            Box::new(rewrite_expr(b, n)?),
            Box::new(rewrite_expr(c, n)?),
        ),
        Coalesce(list) => Coalesce(
            list.iter()
                .map(|e| rewrite_expr(e, n))
                .collect::<Result<_>>()?,
        ),
        FunctionCall(f, args) => FunctionCall(
            f.clone(),
            args.iter()
                .map(|e| rewrite_expr(e, n))
                .collect::<Result<_>>()?,
        ),
    })
}

fn rewrite_order_expr(oe: &OrderExpression, n: &mut usize) -> Result<OrderExpression> {
    Ok(match oe {
        OrderExpression::Asc(e) => OrderExpression::Asc(rewrite_expr(e, n)?),
        OrderExpression::Desc(e) => OrderExpression::Desc(rewrite_expr(e, n)?),
    })
}

fn rewrite_agg_expr(ae: &AggregateExpression, n: &mut usize) -> Result<AggregateExpression> {
    Ok(match ae {
        AggregateExpression::CountSolutions { distinct } => AggregateExpression::CountSolutions {
            distinct: *distinct,
        },
        AggregateExpression::FunctionCall {
            name,
            expr,
            distinct,
        } => AggregateExpression::FunctionCall {
            name: name.clone(),
            expr: rewrite_expr(expr, n)?,
            distinct: *distinct,
        },
    })
}

/// A fresh whole-query identity variable (R3): `__sf_star_{n}`, unwritable in
/// real query text (spargebra rejects a leading double-underscore in surface
/// syntax the same way the CBD rewrite's `__sf_describe_*` relies on) so it
/// can never collide with a user variable.
fn fresh_var(n: &mut usize) -> TermPattern {
    let v = Variable::new_unchecked(format!("__sf_star_{n}"));
    *n += 1;
    TermPattern::Variable(v)
}

/// Whether `pred` is the constant `rdf:reifies` (rule 2's trigger). A variable
/// predicate can never BE the constant `rdf:reifies`, so only the `NamedNode`
/// arm can match.
fn is_reifies(pred: &NamedNodePattern) -> bool {
    matches!(pred, NamedNodePattern::NamedNode(p) if p.as_str() == RDF_REIFIES)
}

/// A quoted triple's predicate (`NamedNodePattern` — structurally never a
/// `Triple`) copied verbatim into the `rdf:propositionFormPredicate` object
/// position (a `TermPattern`).
fn named_node_pattern_to_term_pattern(p: &NamedNodePattern) -> TermPattern {
    match p {
        NamedNodePattern::NamedNode(n) => TermPattern::NamedNode(n.clone()),
        NamedNodePattern::Variable(v) => TermPattern::Variable(v.clone()),
    }
}

/// Rule 9 (v1 boundary): a CONSTRUCT template carrying a quoted-triple term in
/// subject or object position (predicate structurally cannot be one). v1 does
/// not fabricate native triple terms (R5) — `exec_core::instantiate`'s
/// `TermPattern → Term` closure silently returns `None` for `Triple` today
/// (falls into its wildcard arm), which would silently DROP that template
/// triple from CONSTRUCT output rather than erring. Checking here, at
/// translate time, turns that into an explicit, honest 501 instead (ADR-0007
/// sound-501 discipline) — see the call sites in `lib.rs`.
pub fn construct_template_has_quoted_triple(template: &[TriplePattern]) -> bool {
    template.iter().any(|tp| {
        matches!(tp.subject, TermPattern::Triple(_)) || matches!(tp.object, TermPattern::Triple(_))
    })
}

// Unit tests live in `star/tests.rs` (this module resolves to
// `crates/sf-sparql/src/star/tests.rs` — the same split `r2rml.rs`/`r2rml/tests.rs`
// already uses in `sf-mapping`) so this file stays within the project's
// 500-line file budget.
#[cfg(test)]
mod tests;
