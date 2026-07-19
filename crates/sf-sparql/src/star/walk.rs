//! The core recursive `GraphPattern` rewrite walker (rules R1-R5, R5b): BGP
//! triple patterns ([`rewrite_bgp`]/[`rewrite_triple`]), property-path
//! endpoints ([`rewrite_path`]), and the basic-encoding emission shared by
//! both ([`emit_basic_encoding`]/[`substitute_triple`]) — the "quote a
//! `TermPattern::Triple` into a fresh identity + 4 description patterns"
//! machinery every other rewrite rule (VALUES, BIND, the top-level UNION
//! relaxation) ultimately bottoms out in for an ordinary BGP position.
//! [`rewrite_pattern`] is the recursive dispatcher every `GraphPattern`
//! container goes through; it delegates VALUES/BIND to
//! [`super::decompose`], expressions to [`super::expr`], and the top-level-
//! relaxed `Union` case to [`super::top_level::rewrite_union`].

use spargebra::algebra::{GraphPattern, PropertyPathExpression};
use spargebra::term::{NamedNode, NamedNodePattern, TermPattern, TriplePattern};

use crate::unfold::RDF_TYPE;
use crate::Result;

use super::decompose::{rewrite_extend, rewrite_values};
use super::env::{composed_info_for, ComposedInfo, StarEnv};
use super::expr::{rewrite_agg_expr, rewrite_expr, rewrite_order_expr};
use super::top_level::rewrite_union;
use super::util::{
    empty_pattern, fresh_var, has_subject_position_triple_term, is_reifies,
    named_node_pattern_to_term_pattern, RDF_PROPOSITION_FORM, RDF_PROPOSITION_FORM_OBJECT,
    RDF_PROPOSITION_FORM_PREDICATE, RDF_PROPOSITION_FORM_SUBJECT,
};

/// Recurse through every `GraphPattern` container (rule R5), rewriting BGP
/// triple patterns (rules R1-R4, plus the new reifies-bare-variable case) and
/// property-path endpoints (rule R5b) as they are found. `Values` decomposes
/// any ground-triple-carrying column ([`super::decompose::rewrite_values`],
/// ADR-0032 D3); `Extend` (BIND) special-cases a `TRIPLE(e1,e2,e3)` target
/// ([`super::decompose::rewrite_extend`]); a `Union`'s two arms are checked
/// for composed-ness agreement on any variable they both mention
/// ([`super::top_level::rewrite_union`]).
pub(super) fn rewrite_pattern(
    gp: &GraphPattern,
    n: &mut usize,
    env: &mut StarEnv,
) -> Result<GraphPattern> {
    Ok(match gp {
        GraphPattern::Bgp { patterns } => rewrite_bgp(patterns, n, env)?,
        GraphPattern::Path {
            subject,
            path,
            object,
        } => rewrite_path(subject, path, object, n)?,
        GraphPattern::Join { left, right } => GraphPattern::Join {
            left: Box::new(rewrite_pattern(left, n, env)?),
            right: Box::new(rewrite_pattern(right, n, env)?),
        },
        GraphPattern::LeftJoin {
            left,
            right,
            expression,
        } => GraphPattern::LeftJoin {
            left: Box::new(rewrite_pattern(left, n, env)?),
            right: Box::new(rewrite_pattern(right, n, env)?),
            expression: expression
                .as_ref()
                .map(|e| rewrite_expr(e, n, env))
                .transpose()?,
        },
        GraphPattern::Lateral { left, right } => GraphPattern::Lateral {
            left: Box::new(rewrite_pattern(left, n, env)?),
            right: Box::new(rewrite_pattern(right, n, env)?),
        },
        // `inner` MUST rewrite before `expr` (ADR-0032 D3 item 3): FILTER's
        // own `inner` is the pattern the filter applies to and its natural
        // scope — the extremely common `?r rdf:reifies ?t . FILTER
        // isTRIPLE(?t)` shape composes `?t` from WITHIN `inner`, so `env`
        // must already reflect it before `expr` is checked. (Reversed from
        // this arm's pre-Wave-2b order, which rewrote `expr` first purely by
        // struct-literal field order — harmless before `env` existed, wrong
        // now; see `counter_does_not_collide_across_bgp_and_exists_body`'s
        // updated numbering.)
        GraphPattern::Filter { expr, inner } => {
            let inner = Box::new(rewrite_pattern(inner, n, env)?);
            GraphPattern::Filter {
                expr: rewrite_expr(expr, n, env)?,
                inner,
            }
        }
        GraphPattern::Union { left, right } => rewrite_union(left, right, n, env, false)?,
        GraphPattern::Graph { name, inner } => GraphPattern::Graph {
            name: name.clone(),
            inner: Box::new(rewrite_pattern(inner, n, env)?),
        },
        GraphPattern::Extend {
            inner,
            variable,
            expression,
        } => rewrite_extend(inner, variable, expression, n, env)?,
        GraphPattern::Minus { left, right } => GraphPattern::Minus {
            left: Box::new(rewrite_pattern(left, n, env)?),
            right: Box::new(rewrite_pattern(right, n, env)?),
        },
        GraphPattern::Values {
            variables,
            bindings,
        } => rewrite_values(variables, bindings, n, env)?,
        GraphPattern::OrderBy { inner, expression } => GraphPattern::OrderBy {
            inner: Box::new(rewrite_pattern(inner, n, env)?),
            expression: expression
                .iter()
                .map(|oe| rewrite_order_expr(oe, n, env))
                .collect::<Result<_>>()?,
        },
        GraphPattern::Project { inner, variables } => GraphPattern::Project {
            inner: Box::new(rewrite_pattern(inner, n, env)?),
            variables: variables.clone(),
        },
        GraphPattern::Distinct { inner } => GraphPattern::Distinct {
            inner: Box::new(rewrite_pattern(inner, n, env)?),
        },
        GraphPattern::Reduced { inner } => GraphPattern::Reduced {
            inner: Box::new(rewrite_pattern(inner, n, env)?),
        },
        GraphPattern::Slice {
            inner,
            start,
            length,
        } => GraphPattern::Slice {
            inner: Box::new(rewrite_pattern(inner, n, env)?),
            start: *start,
            length: *length,
        },
        GraphPattern::Group {
            inner,
            variables,
            aggregates,
        } => GraphPattern::Group {
            inner: Box::new(rewrite_pattern(inner, n, env)?),
            variables: variables.clone(),
            aggregates: aggregates
                .iter()
                .map(|(v, ae)| Ok((v.clone(), rewrite_agg_expr(ae, n, env)?)))
                .collect::<Result<_>>()?,
        },
        GraphPattern::Service {
            name,
            inner,
            silent,
        } => GraphPattern::Service {
            name: name.clone(),
            inner: Box::new(rewrite_pattern(inner, n, env)?),
            silent: *silent,
        },
    })
}

/// Rewrite a BGP: each triple pattern expands (rules R1-R4) into zero or more
/// output patterns, concatenated in order into one flat `Bgp` (join order is
/// immaterial — a BGP is an unordered AND of patterns). If ANY pattern in the
/// BGP is R1-statically-empty, the conjunction as a whole is (AND-with-false
/// is always false) — so the whole BGP short-circuits to [`super::util::empty_pattern`]
/// rather than building a `Bgp` at all.
fn rewrite_bgp(
    patterns: &[TriplePattern],
    n: &mut usize,
    env: &mut StarEnv,
) -> Result<GraphPattern> {
    let mut out = Vec::with_capacity(patterns.len());
    for tp in patterns {
        if !rewrite_triple(tp, n, env, &mut out)? {
            return Ok(empty_pattern(n));
        }
    }
    Ok(GraphPattern::Bgp { patterns: out })
}

/// Rewrite one triple pattern per rules R1-R4 plus the ADR-0032 D3
/// reifies-bare-variable case, appending its replacement pattern(s) to `out`.
/// Returns `Ok(false)` if the pattern is R1-statically-empty (the caller must
/// discard `out`'s partial contents and propagate emptiness — see
/// [`rewrite_bgp`]); `Ok(true)` otherwise.
fn rewrite_triple(
    tp: &TriplePattern,
    n: &mut usize,
    env: &mut StarEnv,
    out: &mut Vec<TriplePattern>,
) -> Result<bool> {
    // R1: a Triple-typed SUBJECT can never match (SPARQL 1.2 §18.1.3) —
    // checked first, uniformly, so it governs R2 (a hand-written
    // `<<(...)>> rdf:reifies <<(...)>>`, X itself a triple term) exactly
    // like the ordinary case below; no identity is minted for it.
    if matches!(tp.subject, TermPattern::Triple(_)) {
        return Ok(false);
    }

    if is_reifies(&tp.predicate) {
        // R2: `X rdf:reifies <<(...)>>` — no elision. `X` (verified non-Triple
        // above) stays untouched; the wrapper triple is KEPT, pointed at a
        // fresh `?pf` carrying the quoted shape's 4 description patterns (R4
        // recurses further if the quoted shape itself nests another quote
        // object-side).
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
        // ADR-0032 D3 item 2 (NEW) — `X rdf:reifies ?t`, ?t a BARE variable
        // (not syntactic `<<(...)>>`). Any conformant `rdf:reifies` triple's
        // object structurally denotes a triple term (RDF 1.2 Concepts §1.5),
        // and mapping emission ALWAYS produces the 4 description triples
        // alongside the reifies triple from the SAME source row (ADR-0032
        // D1) — so joining them in can never spuriously drop a row a plain,
        // undecorated reifies pattern would have matched. `X` stays
        // untouched (it is the REIFIER — never composed, D1 "reifier ≠
        // proposition"); `?t` becomes composed. `composed_info_for` reuses
        // an already-registered `?t` (e.g. from a sibling `VALUES ?t {...}`
        // decomposed elsewhere in the same query, or another reifies
        // pattern on the same ?t) rather than minting a second, disjoint set
        // of component vars — the ordinary shared-variable join then
        // correlates them for free.
        if let TermPattern::Variable(t) = &tp.object {
            let info = composed_info_for(t, n, env);
            emit_component_patterns(&tp.object, &info, out);
            out.push(tp.clone());
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

/// The reifies-bare-variable case's 4 basic-encoding patterns: like
/// [`emit_basic_encoding`], but the three components are UNKNOWN (there is no
/// syntactic `<<( s p o )>>` to read them from — `identity` is a bare
/// variable bound by a real pattern elsewhere), so they bind to `info`'s
/// fresh component vars instead of copying known `TermPattern`s. Exactly one
/// level (no further recursion): `info.o_var` is not itself statically known
/// to be composed — see [`super::expr::rewrite_expr`]'s SUBJECT/PREDICATE/OBJECT
/// handling for why that is sound (engine-totality) rather than a missed case.
fn emit_component_patterns(
    identity: &TermPattern,
    info: &ComposedInfo,
    out: &mut Vec<TriplePattern>,
) {
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
        object: TermPattern::Variable(info.s_var.clone()),
    });
    out.push(TriplePattern {
        subject: identity.clone(),
        predicate: NamedNodePattern::NamedNode(NamedNode::new_unchecked(
            RDF_PROPOSITION_FORM_PREDICATE,
        )),
        object: TermPattern::Variable(info.p_var.clone()),
    });
    out.push(TriplePattern {
        subject: identity.clone(),
        predicate: NamedNodePattern::NamedNode(NamedNode::new_unchecked(
            RDF_PROPOSITION_FORM_OBJECT,
        )),
        object: TermPattern::Variable(info.o_var.clone()),
    });
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
pub(super) fn substitute_triple(
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
pub(super) fn emit_basic_encoding(
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
/// empty propagates to [`super::util::empty_pattern`], consistent with
/// [`rewrite_bgp`] — unreachable by any locked test today (D6: the existing
/// path-endpoint test quotes a subject-safe shape) but the spec-consistent
/// choice regardless.
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
