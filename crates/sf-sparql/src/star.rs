//! ADR-0032 D3 — RDF-star query rewrite: a `GraphPattern → GraphPattern`
//! pre-pass that desugars quoted-triple patterns onto the native-reification
//! encoding Wave 1 now emits (`sf-mapping`'s `r2rml/star.rs`), applied once at
//! the top of both `translate_tree` and `translate_inner_flat` (`lib.rs`) —
//! mirrors the DESCRIBE→CBD rewrite already living there (a recursive algebra
//! rebuild minting `__sf_`-prefixed synthetic variables), so
//! `build.rs`/`iq/*.rs`/`unfold.rs`/`cascade/`/`emit.rs` never see a
//! `TermPattern::Triple` at all (R1).
//!
//! This supersedes ADR-0031's rules R2/R5 in place (ADR-0032 D3). Ground
//! truth (pinned `spargebra 0.4.6+sparql-12`, unchanged from ADR-0031):
//! bare `<<s p o>>` is parser-desugared to `_:b rdf:reifies <<( s p o )>>` +
//! `_:b` at the original position; parenthesized `<<( s p o )>>` yields
//! `TermPattern::Triple` in place.
//!
//! Rewrite rules per triple pattern (ADR-0032 D3, order matters):
//! (R1) a triple pattern whose own SUBJECT is a triple term — the outer
//! pattern's, OR (recursively) any quoted triple reached through an R4
//! object-chain — can never match (SPARQL 1.2 §18.1.3): rewritten to a
//! **statically empty** group, never an error, never a match. Checked before
//! R2 inspects the predicate, so `X rdf:reifies TT` with X itself a triple
//! term is equally empty. (R2) `X rdf:reifies TT` (all bare/explicit-reifier/
//! annotation sugar desugars here — parser-verified): **no elision**. `X`
//! stays untouched; the wrapper triple is KEPT as `X rdf:reifies ?pf` (fresh
//! var) with the 4 basic-encoding patterns appended on `?pf` — matches only
//! genuinely reified statements (a v1 unsoundness: bare-sugar over-matching
//! unreified object-position triple terms, fixed). (R3) else object-is-Triple
//! → fresh `?pf` + 4 patterns, symmetric with R2's minting. (R4) a quoted
//! triple's own OBJECT being ANOTHER quoted triple recurses bottom-up,
//! arbitrary depth (mirrors `sf-mapping`'s recursive `quote_shape`): the
//! inner quote mints its own `?pf` first, spliced in as the outer's
//! `propositionFormObject`. (R5) recursion covers every `GraphPattern`
//! container, `Expression::Exists` bodies, and `GraphPattern::Path` endpoints
//! (a fresh var + the 4 patterns joined alongside the path node) — path
//! endpoints keep v1's exact shape and inherit the same pre-existing,
//! unrelated boundary (D6, see `differential_star.rs`'s locked test). (R6)
//! `GraphPattern::Values` is untouched (a ground quoted triple already 501s
//! at `unfold::ground_term_to_term`, D6/Wave-2b territory). (R7) a CONSTRUCT
//! template quoting a triple stays a 501 guard here — D2's real instantiation
//! is Wave 2b's `exec_core` work, out of this file's scope.

use spargebra::algebra::{
    AggregateExpression, Expression, GraphPattern, OrderExpression, PropertyPathExpression,
};
use spargebra::term::{NamedNode, NamedNodePattern, TermPattern, TriplePattern, Variable};
use spargebra::Query;

use crate::unfold::RDF_TYPE;
use crate::Result;

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

/// Rewrite a whole query's WHERE pattern (rules R1-R7), threading one
/// whole-query fresh-variable counter (shared by [`fresh_var`] and
/// [`fresh_empty_var`] — never the per-clause `__sf_ord` pattern, which would
/// collide across sibling BGPs/UNION arms/EXISTS bodies). The CONSTRUCT
/// template (a separate `Vec<TriplePattern>`, not a `GraphPattern`) is
/// untouched here — see [`construct_template_has_quoted_triple`] for that
/// still-locked boundary (R7).
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

/// Recurse through every `GraphPattern` container (rule R5), rewriting BGP
/// triple patterns (rules R1-R4) and property-path endpoints (rule R5b) as
/// they are found. `Values` is returned unchanged (rule R6 — a ground quoted
/// triple is a boundary handled downstream, untouched by this pass).
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

/// Rewrite a BGP: each triple pattern expands (rules R1-R4) into zero or more
/// output patterns, concatenated in order into one flat `Bgp` (join order is
/// immaterial — a BGP is an unordered AND of patterns). If ANY pattern in the
/// BGP is R1-statically-empty, the conjunction as a whole is (AND-with-false
/// is always false) — so the whole BGP short-circuits to [`empty_pattern`]
/// rather than building a `Bgp` at all.
fn rewrite_bgp(patterns: &[TriplePattern], n: &mut usize) -> Result<GraphPattern> {
    let mut out = Vec::with_capacity(patterns.len());
    for tp in patterns {
        if !rewrite_triple(tp, n, &mut out)? {
            return Ok(empty_pattern(n));
        }
    }
    Ok(GraphPattern::Bgp { patterns: out })
}

/// Rewrite one triple pattern per rules R1-R4, appending its replacement
/// pattern(s) to `out`. Returns `Ok(false)` if the pattern is R1-statically-
/// empty (the caller must discard `out`'s partial contents and propagate
/// emptiness — see [`rewrite_bgp`]); `Ok(true)` otherwise.
fn rewrite_triple(tp: &TriplePattern, n: &mut usize, out: &mut Vec<TriplePattern>) -> Result<bool> {
    // R1: a Triple-typed SUBJECT can never match (SPARQL 1.2 §18.1.3) —
    // checked first, uniformly, so it governs R2 (a hand-written
    // `<<(...)>> rdf:reifies <<(...)>>`, X itself a triple term) exactly
    // like the ordinary case below; no identity is minted for it.
    if matches!(tp.subject, TermPattern::Triple(_)) {
        return Ok(false);
    }

    // R2: `X rdf:reifies <<(...)>>` — no elision. `X` (verified non-Triple
    // above) stays untouched; the wrapper triple is KEPT, pointed at a fresh
    // `?pf` carrying the quoted shape's 4 description patterns (R4 recurses
    // further if the quoted shape itself nests another quote object-side).
    if is_reifies(&tp.predicate) {
        if let TermPattern::Triple(inner) = &tp.object {
            if has_subject_position_triple_term(inner) {
                return Ok(false);
            }
            let pf = fresh_var(n);
            emit_basic_encoding(&pf, inner, n, out)?;
            out.push(TriplePattern {
                subject: tp.subject.clone(),
                predicate: tp.predicate.clone(),
                object: pf,
            });
            return Ok(true);
        }
    }

    // R3: object-is-Triple → fresh `?pf` + 4 patterns (R4 recurses further
    // for a nested object). A non-Triple object passes through untouched.
    let Some(object) = substitute_triple(&tp.object, n, out)? else {
        return Ok(false);
    };
    out.push(TriplePattern {
        subject: tp.subject.clone(),
        predicate: tp.predicate.clone(),
        object,
    });
    Ok(true)
}

/// If `t` is a quoted-triple pattern, replace it with a fresh `__sf_star_{n}`
/// identity variable and append its 4 basic-encoding patterns (rule R3/R4) to
/// `out`. Returns `Ok(None)` if `t`'s own quoted shape is R1-statically-empty
/// (its subject, or — recursively via R4's object chain — a nested quote's
/// subject, is itself a triple term): there is no legal identity to mint for
/// an impossible quote, so the caller must propagate emptiness instead of
/// substituting. Otherwise returns the (possibly unchanged) term. Shared by
/// BGP object-position substitution (R3) and property-path endpoint
/// substitution (R5b — see [`rewrite_path`]'s own handling of `None`).
fn substitute_triple(
    t: &TermPattern,
    n: &mut usize,
    out: &mut Vec<TriplePattern>,
) -> Result<Option<TermPattern>> {
    match t {
        TermPattern::Triple(tp) => {
            if has_subject_position_triple_term(tp) {
                return Ok(None);
            }
            let fresh = fresh_var(n);
            emit_basic_encoding(&fresh, tp, n, out)?;
            Ok(Some(fresh))
        }
        other => Ok(Some(other.clone())),
    }
}

/// R1's recursive trigger: does `tp` — or (R4) any quoted triple reached
/// through its own OBJECT chain — have a Triple-typed SUBJECT? Subject-side
/// nesting is spec-impossible at any depth (RDF 1.2 Concepts §3.1: triple
/// terms are object-position-only), so it is never checked on the object
/// side of the recursion (there is no other legal place for it to hide).
fn has_subject_position_triple_term(tp: &TriplePattern) -> bool {
    matches!(tp.subject, TermPattern::Triple(_))
        || matches!(&tp.object, TermPattern::Triple(inner) if has_subject_position_triple_term(inner))
}

/// Rule R3/R4: the 4 basic-encoding patterns binding `identity` to quoted
/// `tp = (s, p, o)`. `tp.subject` is copied verbatim — the caller
/// ([`substitute_triple`] / R2's reifies branch) has already verified,
/// transitively, that neither `tp` nor anything nested in its object chain
/// has a Triple-typed subject, so no check is needed here. `tp.object` being
/// ANOTHER quoted triple (R4) recurses bottom-up: the inner quote mints its
/// own fresh identity + 4 patterns FIRST, and that identity becomes THIS
/// level's `propositionFormObject` value — mirrors `sf-mapping`'s recursive
/// `quote_shape` (`r2rml/star.rs`), which splices inner proposition ids into
/// outer ones the same way, bottom-up, arbitrary depth.
fn emit_basic_encoding(
    identity: &TermPattern,
    tp: &TriplePattern,
    n: &mut usize,
    out: &mut Vec<TriplePattern>,
) -> Result<()> {
    debug_assert!(
        !has_subject_position_triple_term(tp),
        "caller must check has_subject_position_triple_term before minting an identity (R1)"
    );
    let object = match &tp.object {
        TermPattern::Triple(inner) => {
            let inner_identity = fresh_var(n);
            emit_basic_encoding(&inner_identity, inner, n, out)?;
            inner_identity
        }
        other => other.clone(),
    };
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
        object,
    });
    Ok(())
}

/// Rule R5b: a property-path endpoint that is itself a quoted-triple pattern
/// substitutes a fresh identity var ([`substitute_triple`]), with its
/// basic-encoding patterns joined alongside the path node — the same
/// `GraphPattern::Join` injection the DESCRIBE→CBD rewrite uses (`lib.rs`).
/// Neither endpoint quoted ⇒ no extra patterns ⇒ the path node is returned
/// unchanged (the common, unaffected case). Either endpoint R1-statically-
/// empty propagates to [`empty_pattern`], consistent with [`rewrite_bgp`] —
/// unreachable by any locked test today (D6: the existing path-endpoint test
/// quotes a subject-safe shape) but the spec-consistent choice regardless.
fn rewrite_path(
    subject: &TermPattern,
    path: &PropertyPathExpression,
    object: &TermPattern,
    n: &mut usize,
) -> Result<GraphPattern> {
    let mut extra = Vec::new();
    let Some(subject) = substitute_triple(subject, n, &mut extra)? else {
        return Ok(empty_pattern(n));
    };
    let Some(object) = substitute_triple(object, n, &mut extra)? else {
        return Ok(empty_pattern(n));
    };
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

/// Rule R5a: recurse through an expression tree looking for `EXISTS`/`NOT
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

/// A fresh whole-query identity variable (shared counter, see
/// [`rewrite_query`]): `__sf_star_{n}`, unwritable in real query text
/// (spargebra rejects a leading double-underscore in surface syntax the same
/// way the CBD rewrite's `__sf_describe_*` relies on) so it can never
/// collide with a user variable.
fn fresh_var(n: &mut usize) -> TermPattern {
    let v = Variable::new_unchecked(format!("__sf_star_{n}"));
    *n += 1;
    TermPattern::Variable(v)
}

/// Rule R1's zero-solution replacement: a fresh, equally-unwritable
/// `__sf_star_empty_{n}` variable naming an empty `VALUES` clause (see
/// [`empty_pattern`]) — kept textually distinct from [`fresh_var`]'s identity
/// variables (which DO bind real terms) purely for readability when a
/// rewritten query is inspected; both draw from the same whole-query counter.
fn fresh_empty_var(n: &mut usize) -> Variable {
    let v = Variable::new_unchecked(format!("__sf_star_empty_{n}"));
    *n += 1;
    v
}

/// Rule R1 (SPARQL 1.2 §18.1.3): the statically-empty replacement for a
/// pattern containing a subject-position triple term — a `VALUES` clause
/// with zero rows, the standard zero-solution element on both translation
/// paths: `unfold::translate_pattern`'s `Values` arm folds zero binding rows
/// into zero branches (`TransPattern::plain(Vec::new())`), and the tree
/// path's `IqNode::Values` lowering does the same (`iq/lower.rs`) — indeed
/// `iq/normalize.rs` already treats an empty result as the `Join`/`Union`
/// absorbing/identity element via its own purpose-built `IqNode::Empty`.
/// Never an error, never a match.
fn empty_pattern(n: &mut usize) -> GraphPattern {
    GraphPattern::Values {
        variables: vec![fresh_empty_var(n)],
        bindings: Vec::new(),
    }
}

/// Whether `pred` is the constant `rdf:reifies` (rule R2's trigger). A
/// variable predicate can never BE the constant `rdf:reifies`, so only the
/// `NamedNode` arm can match.
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

/// Rule R7 (still a locked boundary — Wave 2b territory): a CONSTRUCT
/// template carrying a quoted-triple term in subject or object position
/// (predicate structurally cannot be one). This rewrite does not fabricate
/// native triple terms — `exec_core::instantiate`'s `TermPattern → Term`
/// closure silently returns `None` for `Triple` today (falls into its
/// wildcard arm), which would silently DROP that template triple from
/// CONSTRUCT output rather than erring. Checking here, at translate time,
/// turns that into an explicit, honest 501 instead (ADR-0007 sound-501
/// discipline) — see the call sites in `lib.rs`. ADR-0032 D2 replaces this
/// with real instantiation + spec-defined dropping of illegal output, but
/// that requires `exec_core` changes outside this file's scope.
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
