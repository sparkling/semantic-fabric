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
//! unrelated boundary (D6, see `differential_star.rs`'s locked test). (R6,
//! Wave 2b) `GraphPattern::Values` decomposes any column carrying a
//! `GroundTerm::Triple` cell into fresh component-var columns
//! ([`rewrite_values`]) — see [`StarEnv`]. (R7, Wave 2b) a CONSTRUCT template
//! is pre-substituted, not guarded ([`substitute_construct_template`]): every
//! env-composed variable it references is replaced with an explicit
//! `TermPattern::Triple` over its component vars, so `exec_core::instantiate`
//! (ADR-0032 D2) never needs to know about [`StarEnv`] at all.
//!
//! **Wave 2b additions (ADR-0032 D3 item 2-4):** a whole-query
//! variable → composed-info environment ([`StarEnv`]) is threaded alongside
//! the fresh-variable counter, populated by three sites — R2's NEW
//! reifies-bare-variable case ([`rewrite_triple`]), R6's VALUES decomposition
//! ([`rewrite_values`]), and a `BIND(TRIPLE(e1,e2,e3) AS ?t)` target
//! ([`rewrite_extend`]) — and consumed by the five triple-term functions
//! ([`rewrite_function_call`]), composed-aware `=`/`sameTerm`
//! ([`rewrite_equality`]), CONSTRUCT template pre-substitution
//! ([`substitute_construct_template`]), and the projection seam
//! ([`apply_composed_bindings`], called from `lib.rs` after a `Plan`'s
//! branches are otherwise finalized) that realizes a composed variable as a
//! native `Term::Triple` at reconstruction. A `Union`'s two arms are checked for
//! composed-ness agreement on any variable they both mention
//! ([`rewrite_union`]) — the uniform-composed-ness law this whole mechanism
//! depends on (a SPARQL variable cannot be "sometimes a triple term"
//! depending on which UNION arm produced it).

use std::collections::{BTreeMap, BTreeSet, HashSet};

use spargebra::algebra::{
    AggregateExpression, Expression, Function, GraphPattern, OrderExpression,
    PropertyPathExpression,
};
use spargebra::term::{
    GroundTerm, GroundTriple, Literal, NamedNode, NamedNodePattern, TermPattern, TriplePattern,
    Variable,
};
use spargebra::Query;

use crate::iq::{Branch, TermDef};
use crate::unfold::RDF_TYPE;
use crate::{Error, Plan, Result};

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

const XSD_BOOLEAN: &str = "http://www.w3.org/2001/XMLSchema#boolean";

/// The [`Function::Concat`] marker `rewrite_expr` produces for a provably-
/// non-composed SUBJECT/PREDICATE/OBJECT argument (ADR-0032 D3 / SPARQL
/// §17.4.6) — see [`error_marker_expr`]'s doc comment for the full rationale.
/// Any IRI works (the marker's SOUNDNESS rests on being a `NamedNode`, never
/// its specific value); this one is self-documenting if it ever surfaces in a
/// debug print or error message.
const ERROR_MARKER_IRI: &str = "urn:sf-star:error-marker";

/// ADR-0032 D3 §17.4.6 — the components of one composed (triple-term-valued)
/// SPARQL variable. Every field is a variable bound directly by a real query
/// pattern (never a `TermDef` — those don't exist yet at this AST-rewrite
/// stage): the 4 basic-encoding description patterns for the reifies-object-
/// variable case ([`rewrite_triple`]'s new branch), the decomposed columns for
/// a VALUES ground triple ([`rewrite_values`]), or a `TRIPLE(e1,e2,e3)` BIND's
/// synthetic per-component `Extend`s ([`rewrite_extend`]) — all three sites
/// bind `s_var`/`p_var`/`o_var` as ordinary variables via the ordinary unfold
/// machinery, so no new binding mechanism is needed downstream; only the
/// PROJECTION seam (`lib.rs`) needs to know they compose into a triple term.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposedInfo {
    pub s_var: Variable,
    pub p_var: Variable,
    pub o_var: Variable,
}

/// The whole-query variable → composed-info environment (ADR-0032 D3),
/// threaded mutably through the rewrite alongside the existing fresh-variable
/// counter. A `BTreeMap` (not a `HashMap`): iteration over it (the [`lib.rs`]
/// projection-override pass) must be deterministic, and `Variable` is
/// `Ord`-comparable (a thin wrapper over its name) with no reason to hash.
/// **Lookup-before-mint** everywhere a variable becomes composed: if the SAME
/// variable is independently composed from two syntactic positions in one
/// query (e.g. `?r rdf:reifies ?t` joined against a `VALUES ?t {...}` block on
/// the SAME `?t`), both sites MUST reuse the SAME `s_var`/`p_var`/`o_var` names
/// for the ordinary shared-variable join to correlate them — consult
/// [`StarEnv`] first, mint fresh component vars only on a genuine miss.
pub type StarEnv = BTreeMap<Variable, ComposedInfo>;

/// Rewrite a whole query's WHERE pattern (rules R1-R7 plus the ADR-0032 D3
/// composed-variable extensions), threading one whole-query fresh-variable
/// counter (shared by [`fresh_var`] and [`fresh_empty_var`] — never the
/// per-clause `__sf_ord` pattern, which would collide across sibling
/// BGPs/UNION arms/EXISTS bodies) and one whole-query [`StarEnv`]. The
/// returned env records every variable the rewrite determined to be
/// triple-term-valued; `lib.rs` consults it to realize the native decode at
/// projection and to pre-substitute the CONSTRUCT template
/// ([`substitute_construct_template`]) — the CONSTRUCT template itself (a
/// separate `Vec<TriplePattern>`, not a `GraphPattern`) is untouched HERE.
pub fn rewrite_query(query: &Query) -> Result<(Query, StarEnv)> {
    let mut n = 0usize;
    let mut env = StarEnv::new();
    let rewritten = match query {
        Query::Select {
            dataset,
            pattern,
            base_iri,
        } => Query::Select {
            dataset: dataset.clone(),
            pattern: rewrite_pattern(pattern, &mut n, &mut env)?,
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
            pattern: rewrite_pattern(pattern, &mut n, &mut env)?,
            base_iri: base_iri.clone(),
        },
        Query::Describe {
            dataset,
            pattern,
            base_iri,
        } => Query::Describe {
            dataset: dataset.clone(),
            pattern: rewrite_pattern(pattern, &mut n, &mut env)?,
            base_iri: base_iri.clone(),
        },
        Query::Ask {
            dataset,
            pattern,
            base_iri,
        } => Query::Ask {
            dataset: dataset.clone(),
            pattern: rewrite_pattern(pattern, &mut n, &mut env)?,
            base_iri: base_iri.clone(),
        },
    };
    Ok((rewritten, env))
}

/// Recurse through every `GraphPattern` container (rule R5), rewriting BGP
/// triple patterns (rules R1-R4, plus the new reifies-bare-variable case) and
/// property-path endpoints (rule R5b) as they are found. `Values` decomposes
/// any ground-triple-carrying column ([`rewrite_values`], ADR-0032 D3); `Extend`
/// (BIND) special-cases a `TRIPLE(e1,e2,e3)` target ([`rewrite_extend`]); a
/// `Union`'s two arms are checked for composed-ness agreement on any variable
/// they both mention ([`rewrite_union`]).
fn rewrite_pattern(gp: &GraphPattern, n: &mut usize, env: &mut StarEnv) -> Result<GraphPattern> {
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
        GraphPattern::Union { left, right } => rewrite_union(left, right, n, env)?,
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
/// is always false) — so the whole BGP short-circuits to [`empty_pattern`]
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

/// Look up `var`'s [`ComposedInfo`] in `env`, minting three fresh component
/// vars on a first sighting (lookup-before-mint, see [`StarEnv`]'s doc
/// comment for why reuse — not fresh minting per occurrence — is required
/// here).
fn composed_info_for(var: &Variable, n: &mut usize, env: &mut StarEnv) -> ComposedInfo {
    env.entry(var.clone())
        .or_insert_with(|| ComposedInfo {
            s_var: fresh_component_var(n),
            p_var: fresh_component_var(n),
            o_var: fresh_component_var(n),
        })
        .clone()
}

/// The reifies-bare-variable case's 4 basic-encoding patterns: like
/// [`emit_basic_encoding`], but the three components are UNKNOWN (there is no
/// syntactic `<<( s p o )>>` to read them from — `identity` is a bare
/// variable bound by a real pattern elsewhere), so they bind to `info`'s
/// fresh component vars instead of copying known `TermPattern`s. Exactly one
/// level (no further recursion): `info.o_var` is not itself statically known
/// to be composed — see `rewrite_expr`'s SUBJECT/PREDICATE/OBJECT handling
/// for why that is sound (engine-totality) rather than a missed case.
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
/// Structural recursion otherwise, EXCEPT `Equal`/`SameTerm` (ADR-0032 D3
/// item 4 — [`rewrite_equality`]) and `FunctionCall` (item 3 —
/// [`rewrite_function_call`]), which resolve composed-variable / triple-term-
/// literal operands STATICALLY wherever possible before falling back to
/// ordinary structural recursion.
fn rewrite_expr(expr: &Expression, n: &mut usize, env: &mut StarEnv) -> Result<Expression> {
    use Expression::*;
    Ok(match expr {
        NamedNode(_) | Literal(_) | Variable(_) | Bound(_) => expr.clone(),
        Or(a, b) => Or(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        And(a, b) => And(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        Equal(a, b) => rewrite_equality(a, b, n, env, false)?,
        SameTerm(a, b) => rewrite_equality(a, b, n, env, true)?,
        Greater(a, b) => Greater(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        GreaterOrEqual(a, b) => GreaterOrEqual(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        Less(a, b) => Less(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        LessOrEqual(a, b) => LessOrEqual(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        In(a, list) => In(
            Box::new(rewrite_expr(a, n, env)?),
            list.iter()
                .map(|e| rewrite_expr(e, n, env))
                .collect::<Result<_>>()?,
        ),
        Add(a, b) => Add(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        Subtract(a, b) => Subtract(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        Multiply(a, b) => Multiply(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        Divide(a, b) => Divide(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
        ),
        UnaryPlus(a) => UnaryPlus(Box::new(rewrite_expr(a, n, env)?)),
        UnaryMinus(a) => UnaryMinus(Box::new(rewrite_expr(a, n, env)?)),
        Not(a) => Not(Box::new(rewrite_expr(a, n, env)?)),
        Exists(gp) => Exists(Box::new(rewrite_pattern(gp, n, env)?)),
        If(a, b, c) => If(
            Box::new(rewrite_expr(a, n, env)?),
            Box::new(rewrite_expr(b, n, env)?),
            Box::new(rewrite_expr(c, n, env)?),
        ),
        Coalesce(list) => Coalesce(
            list.iter()
                .map(|e| rewrite_expr(e, n, env))
                .collect::<Result<_>>()?,
        ),
        FunctionCall(f, args) => rewrite_function_call(f, args, n, env)?,
    })
}

/// ADR-0032 D3 item 3 — the five triple-term functions, resolved statically
/// wherever possible (engine-totality: relational data can never contain a
/// native triple term, so composed-ness is always statically known to this
/// rewrite via [`StarEnv`] / a literal `TRIPLE(...)`/`<<(...)>>` operand —
/// `<<( … )>>` inside an expression position parses to the SAME
/// `FunctionCall(Function::Triple, [s,p,o])` shape as an explicit `TRIPLE(...)`
/// call, spargebra `parser.rs`'s `ExprTripleTerm` rule — verified in the pinned
/// 0.4.6 source; there is no separate AST node to special-case).
fn rewrite_function_call(
    f: &Function,
    args: &[Expression],
    n: &mut usize,
    env: &mut StarEnv,
) -> Result<Expression> {
    match (f, args) {
        (Function::Subject, [arg]) => {
            let (_, composed) = rewrite_and_check_composed(arg, n, env)?;
            Ok(match composed {
                Some((s, _, _)) => s,
                None => error_marker_expr(),
            })
        }
        (Function::Predicate, [arg]) => {
            let (_, composed) = rewrite_and_check_composed(arg, n, env)?;
            Ok(match composed {
                Some((_, p, _)) => p,
                None => error_marker_expr(),
            })
        }
        (Function::Object, [arg]) => {
            let (_, composed) = rewrite_and_check_composed(arg, n, env)?;
            Ok(match composed {
                Some((_, _, o)) => o,
                None => error_marker_expr(),
            })
        }
        // §17.4.6 asymmetry: isTRIPLE NEVER errors, unlike SUBJECT/PREDICATE/
        // OBJECT — always resolves to a definite boolean, both composed and
        // non-composed arms bind (never leave unbound), so the plain boolean
        // `Literal` (which `unify::bind_term_def`/`filter_cond` already, or
        // now, understand) is correct in EVERY context, unlike the error
        // marker (see `error_marker_expr`'s doc comment).
        (Function::IsTriple, [arg]) => {
            let (_, composed) = rewrite_and_check_composed(arg, n, env)?;
            Ok(bool_literal_expr(composed.is_some()))
        }
        // TRIPLE(e1,e2,e3) is "statically routable" only through call sites
        // that recognize it BEFORE generic recursion reaches here: a BIND
        // target (`rewrite_extend`) and an equality/sameTerm/SUBJECT/
        // PREDICATE/OBJECT/isTRIPLE operand (`rewrite_and_check_composed`,
        // used by both). Reaching this arm means neither applied — e.g. a
        // bare `FILTER(TRIPLE(...))`, or TRIPLE nested as an argument to some
        // other function — genuinely not statically routable in this wave.
        (Function::Triple, _) => Err(Error::Unsupported(
            "TRIPLE(...) outside a BIND target, an equality/sameTerm operand, or a \
             SUBJECT/PREDICATE/OBJECT/isTRIPLE argument is not statically routable \
             (ADR-0032 D3) → 501"
                .to_owned(),
        )),
        _ => Ok(Expression::FunctionCall(
            f.clone(),
            args.iter()
                .map(|e| rewrite_expr(e, n, env))
                .collect::<Result<_>>()?,
        )),
    }
}

/// ADR-0032 D3 item 4 — `=`/`sameTerm` where either side is composed
/// (§17.4.2): both composed → component-wise conjunction (subject/predicate
/// compared directly — RDF 1.2 §3.1: they can never themselves be triple
/// terms, so no recursion is needed there; object recurses, so nested
/// composed terms compare structurally all the way down); exactly one
/// composed → the constant `false` (a triple term can never equal a
/// non-triple-term value — well-defined, never an error, for BOTH operators);
/// neither composed → ordinary (unchanged) `Equal`/`SameTerm`.
fn rewrite_equality(
    a: &Expression,
    b: &Expression,
    n: &mut usize,
    env: &mut StarEnv,
    same_term: bool,
) -> Result<Expression> {
    let (ra, ca) = rewrite_and_check_composed(a, n, env)?;
    let (rb, cb) = rewrite_and_check_composed(b, n, env)?;
    let wrap = |l: Expression, r: Expression| {
        if same_term {
            Expression::SameTerm(Box::new(l), Box::new(r))
        } else {
            Expression::Equal(Box::new(l), Box::new(r))
        }
    };
    Ok(match (ca, cb) {
        (Some((sa, pa, oa)), Some((sb, pb, ob))) => {
            let cmp_s = wrap(sa, sb);
            let cmp_p = wrap(pa, pb);
            let cmp_o = rewrite_equality(&oa, &ob, n, env, same_term)?;
            Expression::And(
                Box::new(Expression::And(Box::new(cmp_s), Box::new(cmp_p))),
                Box::new(cmp_o),
            )
        }
        (Some(_), None) | (None, Some(_)) => bool_literal_expr(false),
        (None, None) => wrap(ra, rb),
    })
}

/// Rewrite `arg` (resolving any nested star construct it contains — e.g.
/// `OBJECT(?t)` where `?t` is composed resolves to `?t`'s own object
/// component var, which is then re-checked here), and additionally return its
/// three component sub-expressions if the REWRITTEN result is itself composed:
/// either an env-composed variable ([`StarEnv`]) or a literal
/// `TRIPLE(e1,e2,e3)`/`<<( e1 e2 e3 )>>` call (ADR-0032 D3 item 4's "ground
/// triple term literals in expressions ⇒ composed constants" — checked on the
/// RAW shape FIRST, before generic recursion, since `Function::Triple` is
/// Unsupported through any OTHER path — see [`rewrite_function_call`]).
/// A composed expression's three (subject, predicate, object) sub-expressions
/// — [`rewrite_and_check_composed`]'s result shape, factored out for clippy's
/// `type_complexity` lint.
type ComposedComponents = (Expression, Expression, Expression);

fn rewrite_and_check_composed(
    arg: &Expression,
    n: &mut usize,
    env: &mut StarEnv,
) -> Result<(Expression, Option<ComposedComponents>)> {
    if let Expression::FunctionCall(Function::Triple, parts) = arg {
        if let [e1, e2, e3] = parts.as_slice() {
            let (r1, _) = rewrite_and_check_composed(e1, n, env)?;
            let (r2, _) = rewrite_and_check_composed(e2, n, env)?;
            let (r3, _) = rewrite_and_check_composed(e3, n, env)?;
            let rewritten = Expression::FunctionCall(
                Function::Triple,
                vec![r1.clone(), r2.clone(), r3.clone()],
            );
            return Ok((rewritten, Some((r1, r2, r3))));
        }
    }
    let rewritten = rewrite_expr(arg, n, env)?;
    let composed = match &rewritten {
        Expression::Variable(v) => env.get(v).map(|info| {
            (
                Expression::Variable(info.s_var.clone()),
                Expression::Variable(info.p_var.clone()),
                Expression::Variable(info.o_var.clone()),
            )
        }),
        _ => None,
    };
    Ok((rewritten, composed))
}

/// SPARQL §17.4.6 SUBJECT/PREDICATE/OBJECT error on a provably-non-composed
/// argument (engine-totality — see [`rewrite_function_call`]'s doc comment):
/// this rewrite happens BEFORE it is known whether the containing context is
/// boolean (FILTER) or value (BIND), so it must pick ONE `Expression` shape
/// that is correct under BOTH downstream consumers (R5 — no silently
/// conflating error with a wrong bound value):
///
/// * FILTER (`unify::filter_cond`): this wave adds a `Function::Concat`
///   recognizer that treats this EXACT shape as the constant `false` — an
///   erroring FILTER operand eliminates the row, the same observable effect.
/// * BIND (`unify::bind_term_def`): its EXISTING `Function::Concat` arm
///   requires every operand to reconstruct as a `Term::Literal`
///   (`exec_core::build_term`'s refutable `let Some(Term::Literal(l)) = …
///   else { return Ok(None) }`); a `NamedNode` constant operand fails that
///   match, so `build_term` ALREADY (zero new runtime code) reconstructs this
///   to `None` — the exact §10 ASSIGN "expression error ⇒ variable unbound"
///   behavior.
///
/// A bare/fresh unbound variable was considered and rejected: both
/// `filter_cond`'s `var_col` and `bind_term_def`'s `Variable` arm require the
/// variable to already be a KNOWN column binding, so a truly-fresh name 501s
/// the WHOLE QUERY at translate time (wrong — this must be a per-row/
/// deterministic-always effect, not a translate-time failure) rather than
/// eliminating a row / leaving one BIND target unbound.
fn error_marker_expr() -> Expression {
    Expression::FunctionCall(
        Function::Concat,
        vec![Expression::NamedNode(NamedNode::new_unchecked(
            ERROR_MARKER_IRI,
        ))],
    )
}

/// A plain `xsd:boolean` literal — `isTRIPLE`'s always-a-value result
/// (§17.4.6), and the leaf of an `=`/`sameTerm` "exactly one side composed"
/// comparison ([`rewrite_equality`]). Already understood, unchanged, by
/// `unify::bind_term_def` (`Expression::Literal` arm) and (this wave)
/// `unify::filter_cond`'s new boolean-literal arm.
fn bool_literal_expr(v: bool) -> Expression {
    Expression::Literal(Literal::new_typed_literal(
        if v { "true" } else { "false" },
        NamedNode::new_unchecked(XSD_BOOLEAN),
    ))
}

fn rewrite_order_expr(
    oe: &OrderExpression,
    n: &mut usize,
    env: &mut StarEnv,
) -> Result<OrderExpression> {
    Ok(match oe {
        OrderExpression::Asc(e) => OrderExpression::Asc(rewrite_expr(e, n, env)?),
        OrderExpression::Desc(e) => OrderExpression::Desc(rewrite_expr(e, n, env)?),
    })
}

fn rewrite_agg_expr(
    ae: &AggregateExpression,
    n: &mut usize,
    env: &mut StarEnv,
) -> Result<AggregateExpression> {
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
            expr: rewrite_expr(expr, n, env)?,
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
    TermPattern::Variable(fresh_component_var(n))
}

/// Like [`fresh_var`], but returns the bare [`Variable`] (not wrapped in a
/// `TermPattern`) — for minting [`ComposedInfo`]'s `s_var`/`p_var`/`o_var`.
fn fresh_component_var(n: &mut usize) -> Variable {
    let v = Variable::new_unchecked(format!("__sf_star_{n}"));
    *n += 1;
    v
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

/// Rule R7 (Wave 2b — ADR-0032 D2/D3 item 2): pre-substitute a CONSTRUCT
/// template so `exec_core::instantiate` never needs to know about [`StarEnv`]
/// at all. Every occurrence of an env-composed variable in subject or object
/// position (predicate structurally cannot compose — RDF 1.2 predicates are
/// always IRIs) is replaced with an explicit `TermPattern::Triple` over its
/// component vars, recursively: a component var that is ITSELF composed
/// (e.g. from a VALUES clause's recursive nested-triple decomposition, or a
/// nested `TRIPLE(...)` BIND) substitutes again, giving nested composition
/// for free — mirrors `exec_core::build_term`'s own recursion for
/// `TermDef::ComposedTriple`'s `object` field. Supersedes the old
/// `construct_template_has_quoted_triple` 501 guard (removed — see `lib.rs`).
pub fn substitute_construct_template(
    template: &[TriplePattern],
    env: &StarEnv,
) -> Vec<TriplePattern> {
    template
        .iter()
        .map(|tp| TriplePattern {
            subject: substitute_composed_term(&tp.subject, env),
            predicate: tp.predicate.clone(),
            object: substitute_composed_term(&tp.object, env),
        })
        .collect()
}

/// One CONSTRUCT-template term slot — see [`substitute_construct_template`].
fn substitute_composed_term(t: &TermPattern, env: &StarEnv) -> TermPattern {
    match t {
        TermPattern::Variable(v) => match env.get(v) {
            Some(info) => TermPattern::Triple(Box::new(TriplePattern {
                subject: substitute_composed_term(&TermPattern::Variable(info.s_var.clone()), env),
                predicate: NamedNodePattern::Variable(info.p_var.clone()),
                object: substitute_composed_term(&TermPattern::Variable(info.o_var.clone()), env),
            })),
            None => t.clone(),
        },
        other => other.clone(),
    }
}

/// `BIND(expr AS ?v)` (rule R5a's Extend case, plus ADR-0032 D3 item 3's
/// `TRIPLE(e1,e2,e3)` BIND target): rewrites `inner` once, then delegates to
/// [`rewrite_extend_inner`] for the (possibly recursive) target-expression
/// handling.
fn rewrite_extend(
    inner: &GraphPattern,
    variable: &Variable,
    expression: &Expression,
    n: &mut usize,
    env: &mut StarEnv,
) -> Result<GraphPattern> {
    let rewritten_inner = rewrite_pattern(inner, n, env)?;
    rewrite_extend_inner(rewritten_inner, variable, expression, n, env)
}

/// The recursive core of [`rewrite_extend`], operating on an ALREADY-rewritten
/// `inner` so it can recurse onto itself for OBJECT-side `TRIPLE(...)`
/// nesting without re-rewriting `inner` at every level. A `TRIPLE(e1,e2,e3)`
/// target marks `variable` composed (`composed_info_for` reuses an
/// already-registered `variable`, e.g. one ALSO reified elsewhere) and
/// replaces the single BIND with THREE synthetic per-component `Extend`s —
/// `BIND(e1 AS s_var) BIND(e2 AS p_var) BIND(e3 AS o_var)`, innermost-first so
/// each is in scope for the next — reusing `unify::bind_term_def`'s existing
/// (narrow but adequate) machinery to lower e1/e2/e3 verbatim; `variable`
/// itself is never bound by any real pattern here — its projection is
/// realized wholly by `lib.rs`'s env-composed override, keyed off
/// `s_var`/`p_var`/`o_var` being bound (see [`StarEnv`]'s doc comment), not
/// off `variable`. `e3` (object position — the only position RDF 1.2 §3.1
/// allows to nest) recurses if it is ITSELF a `TRIPLE(...)` call, giving
/// arbitrary-depth nested composition for free. Anything else is an ordinary
/// BIND, `expression` rewritten in place (which also resolves a `TRIPLE(...)`
/// reached through equality/SUBJECT/PREDICATE/OBJECT/isTRIPLE — see
/// [`rewrite_and_check_composed`] — or leaves an otherwise-unroutable
/// `TRIPLE(...)` Unsupported, see [`rewrite_function_call`]).
fn rewrite_extend_inner(
    rewritten_inner: GraphPattern,
    variable: &Variable,
    expression: &Expression,
    n: &mut usize,
    env: &mut StarEnv,
) -> Result<GraphPattern> {
    if let Expression::FunctionCall(Function::Triple, parts) = expression {
        if let [e1, e2, e3] = parts.as_slice() {
            let info = composed_info_for(variable, n, env);
            let e1 = rewrite_expr(e1, n, env)?;
            let e2 = rewrite_expr(e2, n, env)?;
            let with_s = GraphPattern::Extend {
                inner: Box::new(rewritten_inner),
                variable: info.s_var.clone(),
                expression: e1,
            };
            let with_p = GraphPattern::Extend {
                inner: Box::new(with_s),
                variable: info.p_var.clone(),
                expression: e2,
            };
            return rewrite_extend_inner(with_p, &info.o_var, e3, n, env);
        }
    }
    Ok(GraphPattern::Extend {
        inner: Box::new(rewritten_inner),
        variable: variable.clone(),
        expression: rewrite_expr(expression, n, env)?,
    })
}

/// ADR-0032 D3 item 2, R6 (Wave 2b) — decompose any VALUES column carrying a
/// ground triple term. Column-major transpose, decompose each column
/// independently ([`decompose_column`]), transpose back — the row count and
/// row order are unaffected (`=_bag` preserving), only the column set/arity
/// changes for a composed variable.
fn rewrite_values(
    variables: &[Variable],
    bindings: &[Vec<Option<GroundTerm>>],
    n: &mut usize,
    env: &mut StarEnv,
) -> Result<GraphPattern> {
    let n_rows = bindings.len();
    let mut out_columns: Vec<(Variable, Vec<Option<GroundTerm>>)> =
        Vec::with_capacity(variables.len());
    for (i, var) in variables.iter().enumerate() {
        let cells: Vec<Option<GroundTerm>> = bindings.iter().map(|row| row[i].clone()).collect();
        decompose_column(var.clone(), cells, n, &mut out_columns, env)?;
    }
    let out_variables: Vec<Variable> = out_columns.iter().map(|(v, _)| v.clone()).collect();
    let out_bindings: Vec<Vec<Option<GroundTerm>>> = (0..n_rows)
        .map(|r| {
            out_columns
                .iter()
                .map(|(_, cells)| cells[r].clone())
                .collect()
        })
        .collect();
    Ok(GraphPattern::Values {
        variables: out_variables,
        bindings: out_bindings,
    })
}

/// One VALUES column: passed through unchanged unless it carries ANY
/// `GroundTerm::Triple` cell, in which case EVERY bound (non-UNDEF) cell MUST
/// be one too (a column mixing a triple-term cell with a NamedNode/Literal
/// cell for the same variable is a genuine shape ambiguity this transform
/// cannot represent in one flat table → explicit Unsupported, never a silent
/// prune — the uniform-composed-ness law, `differential_star.rs`-locked). A
/// triple cell decomposes into 3 columns: subject/predicate are always
/// `NamedNode` (`GroundTriple`'s own field types — RDF 1.2 §3.1, no
/// recursion possible there), object recurses ([`decompose_column`] again)
/// since it may itself be another ground triple, arbitrary depth.
fn decompose_column(
    var: Variable,
    cells: Vec<Option<GroundTerm>>,
    n: &mut usize,
    out: &mut Vec<(Variable, Vec<Option<GroundTerm>>)>,
    env: &mut StarEnv,
) -> Result<()> {
    let any_triple = cells
        .iter()
        .any(|c| matches!(c, Some(GroundTerm::Triple(_))));
    if !any_triple {
        out.push((var, cells));
        return Ok(());
    }
    if cells
        .iter()
        .any(|c| !matches!(c, None | Some(GroundTerm::Triple(_))))
    {
        return Err(Error::Unsupported(format!(
            "VALUES ?{} mixes a ground triple-term cell with a NamedNode/Literal cell for the \
             same variable → 501 (engine-total composed-ness must be uniform per var, ADR-0032 \
             D3)",
            var.as_str()
        )));
    }
    let info = composed_info_for(&var, n, env);
    let mut s_cells = Vec::with_capacity(cells.len());
    let mut p_cells = Vec::with_capacity(cells.len());
    let mut o_cells = Vec::with_capacity(cells.len());
    for cell in cells {
        match cell {
            Some(GroundTerm::Triple(t)) => {
                let GroundTriple {
                    subject,
                    predicate,
                    object,
                } = *t;
                s_cells.push(Some(GroundTerm::NamedNode(subject)));
                p_cells.push(Some(GroundTerm::NamedNode(predicate)));
                o_cells.push(Some(object));
            }
            None => {
                s_cells.push(None);
                p_cells.push(None);
                o_cells.push(None);
            }
            Some(_) => unreachable!("the mixed-shape check above already rejected this"),
        }
    }
    out.push((info.s_var.clone(), s_cells));
    out.push((info.p_var.clone(), p_cells));
    decompose_column(info.o_var.clone(), o_cells, n, out, env)
}

/// ADR-0032 D3 item 2 — the uniform-composed-ness law: a `Union`'s two arms
/// are checked for composed-ness agreement on any variable they BOTH
/// syntactically mention (collected from the ORIGINAL, pre-rewrite patterns —
/// [`collect_pattern_vars`]). Env lookup-before-mint means a variable
/// composed by one arm and REUSED (not re-composed) by the other is fine
/// (e.g. both arms independently reify the same `?t`) — checked here by
/// whether EACH arm's OWN rewritten output actually binds that variable's
/// `s_var` (not by "which arm minted it first", which would false-positive
/// on exactly that reuse case). A shared variable composed in one arm's
/// output but not the other's would make it observably "sometimes a triple
/// term" depending on which arm produced a given row — never allowed
/// silently (R5).
fn rewrite_union(
    left: &GraphPattern,
    right: &GraphPattern,
    n: &mut usize,
    env: &mut StarEnv,
) -> Result<GraphPattern> {
    let left_vars = collect_pattern_vars(left);
    let right_vars = collect_pattern_vars(right);

    let rw_left = rewrite_pattern(left, n, env)?;
    let rw_right = rewrite_pattern(right, n, env)?;

    let shared = left_vars.intersection(&right_vars);
    for v in shared {
        let Some(info) = env.get(v).cloned() else {
            continue; // not composed anywhere ⇒ nothing to reconcile
        };
        let left_composes = collect_pattern_vars(&rw_left).contains(&info.s_var);
        let right_composes = collect_pattern_vars(&rw_right).contains(&info.s_var);
        if left_composes != right_composes {
            return Err(Error::Unsupported(format!(
                "UNION arms disagree on whether ?{} is a triple term (composed by one arm, an \
                 ordinary binding in the other) → 501 (ADR-0032 D3 uniform-composed-ness law)",
                v.as_str()
            )));
        }
    }

    Ok(GraphPattern::Union {
        left: Box::new(rw_left),
        right: Box::new(rw_right),
    })
}

/// Every [`Variable`] mentioned anywhere in `gp` — triple-pattern subject/
/// object (recursing into a nested quoted triple), VALUES/Extend/Group/Path
/// variables, and `Expression::Variable`/`Bound` references (recursing into
/// EXISTS bodies) — used by [`rewrite_union`]'s uniform-composed-ness check.
/// Deliberately broad (a var mentioned only in a FILTER still counts): a
/// false positive here costs only an unnecessary — but harmless — agreement
/// check; missing a real disagreement would not be sound.
fn collect_pattern_vars(gp: &GraphPattern) -> BTreeSet<Variable> {
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

/// ADR-0032 D3 item 2 — expand a SELECT's projected-variable NAME list with
/// every env-composed variable's component var names, recursively (nested
/// composition). `lib.rs` passes the RESULT (not the bare SELECT list) as
/// `cascade::CascadeCtx::project` wherever a query might carry composed
/// variables: `cascade`'s pass 7 (projection shrinking) runs BEFORE
/// [`apply_composed_bindings`] installs the `ComposedTriple` binding that
/// visibly "uses" a component var — a component that has NO OTHER consumer
/// (e.g. `SELECT ?t WHERE { VALUES ?t { <<(...)>> } }`, pure pass-through, no
/// join/filter references it either) would otherwise look, to pass 7, like a
/// dead column and be pruned before the override ever runs. Fixed-point
/// iteration (not a single pass) so a NESTED composed component (itself an
/// `env` key) also pulls in ITS OWN components.
pub fn expand_projection_for_cascade(vars: &[String], env: &StarEnv) -> Vec<String> {
    let mut out: Vec<String> = vars.to_vec();
    let mut changed = true;
    while changed {
        changed = false;
        for (var, info) in env {
            if out.iter().any(|v| v == var.as_str()) {
                for c in [&info.s_var, &info.p_var, &info.o_var] {
                    let name = c.as_str().to_owned();
                    if !out.contains(&name) {
                        out.push(name);
                        changed = true;
                    }
                }
            }
        }
    }
    out
}

/// ADR-0032 D3 item 2 — every component var name across the WHOLE `env`,
/// regardless of which specific SELECT variables reference them: the
/// coarser "extra keep" set [`crate::iq::lower::lower`] needs for the TREE
/// path's OWN internal `Construction` "restrict to project" retains, which
/// run even EARLIER than `cascade`'s pass 7 (before this query's actual
/// projected-variable set is known at that stage) — see that function's
/// `extra_keep` parameter doc comment. Deliberately coarser than
/// [`expand_projection_for_cascade`] (which is query-projection-aware): safe
/// because over-keeping a column here is harmless, and [`apply_composed_bindings`]
/// only ever reads a component var that a projected composed variable's
/// [`ComposedInfo`] actually names.
pub fn all_component_var_names(env: &StarEnv) -> HashSet<String> {
    let mut out = HashSet::new();
    for info in env.values() {
        out.insert(info.s_var.as_str().to_owned());
        out.insert(info.p_var.as_str().to_owned());
        out.insert(info.o_var.as_str().to_owned());
    }
    out
}

/// ADR-0032 D3 item 2 — the projection seam: for every [`StarEnv`]-composed
/// variable, install a [`TermDef::ComposedTriple`] binding for it in every
/// branch where its components are available, so `exec_core::reconstruct`
/// realizes a native `Term::Triple` (D2's "every visible surface" mandate).
/// Called from `lib.rs` once a `Plan`'s branches are otherwise finalized
/// (AFTER `unfold`/the tree `lower` pipeline AND the cascade — `unify::unify`
/// never has to know about `ComposedTriple` in practice, see its own doc
/// comment). Unconditionally OVERWRITES any pre-existing raw binding for the
/// variable (e.g. the reifies-bare-variable case's own real pf-IRI binding —
/// every real join/filter unification involving it is already done by this
/// point, so the raw binding has nothing further to participate in).
///
/// Recurses into every branch's `subplan_joins` (ADR-0023 M5 derived-table
/// pooling), SAFELY — see [`apply_composed_bindings_checked`]'s doc comment
/// for why a naive mirror of `lib.rs::cascade_subplans`' recursion shape is
/// UNSOUND here (an EMPIRICALLY CONFIRMED SQL-emission crash, not a
/// hypothetical) and what the guard does instead.
///
/// **Deeper cross-boundary gap — FIXED (F4a)**: when the composed variable
/// itself CROSSES the SubPlan boundary — i.e. it is one of the inner
/// sub-SELECT's own declared `vars` (so it participates in the outer query),
/// but its component vars are NOT (they are synthetic, never user-selected)
/// — the gap was NOT here at all. `iq::lower::lower_as_subplan` used to build
/// the outer branch's binding for that variable by remapping ONLY `vars`
/// (the SPARQL-declared projected names) through the derived table's
/// columns; the component vars, though kept alive INSIDE the inner branch by
/// `extra_keep`, were never among `vars` and so never reached the outer
/// branch AT ALL — no raw SQL column carried them across, and no recursion
/// run AFTER `lower_as_subplan` (which had already frozen the outer remap)
/// could retroactively fix that. Empirically confirmed (before the fix):
/// `SELECT ?t ?friend WHERE { ?p ex:knows ?friend . { SELECT DISTINCT ?t
/// WHERE { ?r rdf:reifies ?t } } }` against the `differential_star.rs`
/// CENSUS fixture lowered to an outer branch whose `bindings["t"]` was still
/// the RAW (pre-composition) template def — and, once the OUTER top-level
/// `apply_composed_bindings` pass here ALSO (redundantly, unguarded)
/// recomposed `?t` INSIDE the now-mutated inner SubPlan branch, the
/// EARLIER-frozen outer positional references desynced from the
/// (unrelatedly) changed derived-table shape, crashing at SQL execution with
/// "no such column" — not merely a wrong answer.
///
/// Fixed by making `lower_as_subplan` itself `StarEnv`-aware: it now builds
/// each `vars` entry's `ComposedTriple` from the ARM's own bindings (which DO
/// have the components, in the SAME inner scope) BEFORE remapping, reusing
/// `remap_termdef`'s `TermDef::ComposedTriple` arm (see its own doc comment)
/// to remap subject/predicate/object each to their own derived-table
/// position — the SAME single-pass `arm_projections` every other var already
/// uses, so there is no later, out-of-sync mutation to desync from.
/// `&StarEnv` is threaded alongside `extra_keep` through the
/// `lower`/`lower_spine`/`lower_node`/`lower_aggregation`/`lower_as_subplan`
/// chain (`iq/lower.rs`). This function's own recursion into `subplan_joins`
/// (above) and [`apply_composed_bindings_checked`]'s guard are UNCHANGED and
/// stay in place as defense — they simply become no-ops for a variable
/// `lower_as_subplan` already composed correctly.
pub fn apply_composed_bindings(branches: &mut [Branch], env: &StarEnv) {
    for branch in branches {
        apply_to_one_branch(branch, env);
        for sp in &mut branch.subplan_joins {
            propagate_single_branch_distinct(&mut sp.plan);
            for inner in &mut sp.plan.branches {
                apply_composed_bindings_checked(inner, env);
            }
        }
    }
}

/// Mirror [`Plan::prepared_branches`]'s single-branch DISTINCT propagation
/// (never persisted by that method itself — it returns a fresh clone) onto
/// `plan`'s OWN stored branch, PERMANENTLY. Needed so a `projection()` check
/// run directly on `plan.branches` (as [`apply_composed_bindings_checked`]
/// does, never going through `prepared_branches`) sees the SAME view
/// `iq::lower::lower_as_subplan` (which froze the outer positional column
/// remap) and the later, real SQL emission (`emit::emit_subplan_sql` →
/// `Plan::emitted` → `prepared_branches` again) both use. Without this, a
/// branch still showing its pre-propagation `distinct: false` could
/// UNDER-detect a real footprint change in that check: a bindings-column
/// composing away can be silently "backfilled" by a WHERE-condition
/// reference to the SAME column ([`Branch::projection`] only excludes
/// WHERE/JOIN-ON columns under `distinct: true`), which the TRUE
/// (post-propagation) view correctly excludes but a stale `distinct: false`
/// view would not — confirmed reachable by direct inspection of the SubPlan
/// this file's own doc comments cite as the empirical crash repro
/// (`apply_composed_bindings_checked`'s doc comment): its raw `distinct:
/// false` branch and its `distinct: true`-forced view disagree (10 columns
/// vs. 4) on exactly this branch. Harmless elsewhere: the final emission's
/// OWN `prepared_branches` call re-derives the identical value from
/// `plan.distinct` regardless of whether this ran first. A multi-branch
/// SubPlan needs no such propagation — `iq::lower::lower_as_subplan`'s own
/// multi-branch DISTINCT-narrowing already sets `distinct: true` on EVERY
/// arm directly (ADR-0025 Tier-2 gap 2).
fn propagate_single_branch_distinct(plan: &mut Plan) {
    if plan.branches.len() == 1 {
        plan.branches[0].distinct = plan.distinct;
    }
}

/// Try composing every [`StarEnv`] variable in `branch.bindings` (recursing
/// into any FURTHER-nested `subplan_joins` the same way, each level guarded
/// independently), but keep the result ONLY if it does not change `branch`'s
/// [`Branch::projection`] — i.e. the exact raw-column list, same columns,
/// same order — otherwise discard the attempt and leave `branch` untouched.
///
/// **Why this guard exists (found empirically, not anticipated by the
/// original ask to "mirror `cascade_subplans`'s recursion shape")**: a
/// composed variable's OWN pre-composition binding (e.g. its raw
/// proposition-form template) may read raw columns nothing else in the
/// branch needs, while its `ComposedTriple` replacement reads its
/// components' OWN raw columns instead — columns already counted via THEIR
/// separate, `extra_keep`-kept bindings. Swapping the shape can therefore
/// shrink (or relocate) `projection()`'s deduplicated column list. That list
/// is exactly what `iq::lower::lower_as_subplan` used, EARLIER and ONCE, to
/// freeze the OUTER branch's positional column references
/// (`t{subplan_alias}.c{i}`) into that branch's OWN bindings — a frozen
/// snapshot this function has no way to reach or update (it runs strictly
/// afterward, from `lib.rs`, on the already-built `Plan`). A silent width
/// change here desyncs those already-frozen positions from the derived
/// table's ACTUAL (later re-emitted, ADR-0025 Tier-2 gap 2) SELECT list.
/// Confirmed by direct execution: `SELECT ?t ?friend WHERE { ?p ex:knows
/// ?friend . { SELECT DISTINCT ?t WHERE { ?r rdf:reifies ?t } } }` against
/// the CENSUS fixture, composing `?t` UNGUARDED inside its SubPlan branch,
/// shrank that branch's SQLite `DISTINCT` derived-table SELECT list from 4
/// columns to 2 (the ComposedTriple's `subject`/`object` columns already
/// counted via the separately-kept `__sf_star_0`/`__sf_star_2` bindings), and
/// the outer join's frozen `t6.c2`/`t6.c3` references then hit a real SQLite
/// `no such column` error at execution — a hard CRASH, not merely a wrong
/// answer. `cascade_subplans` avoids this identical hazard by running its
/// nested cascade with `project: None`, which disables the ONE pass
/// (projection shrinking) that could change a branch's column footprint;
/// this function has no equivalent lever (it always changes footprint when a
/// composed variable's own raw shape differs from its components'), so it
/// verifies safety directly instead. See [`apply_composed_bindings`]'s doc
/// comment for the closely related, NOT-closed-by-this-guard-either
/// cross-boundary gap.
fn apply_composed_bindings_checked(branch: &mut Branch, env: &StarEnv) {
    let before = branch.projection();
    let mut candidate = branch.clone();
    apply_to_one_branch(&mut candidate, env);
    for sp in &mut candidate.subplan_joins {
        propagate_single_branch_distinct(&mut sp.plan);
        for inner in &mut sp.plan.branches {
            apply_composed_bindings_checked(inner, env);
        }
    }
    if candidate.projection() == before {
        *branch = candidate;
    }
}

/// Install every [`StarEnv`] variable's [`TermDef::ComposedTriple`] binding
/// that `branch.bindings` currently has the components for — no recursion
/// into `subplan_joins`, no safety check; see [`apply_composed_bindings`] /
/// [`apply_composed_bindings_checked`] for the two call sites that add those.
fn apply_to_one_branch(branch: &mut Branch, env: &StarEnv) {
    // Two passes (collect then insert) — inserting while iterating `env`
    // would be fine (env isn't mutated), but collecting first keeps the
    // borrow of `branch.bindings` used by `composed_term_def` read-only
    // for the whole scan, independent of the mutation that follows.
    let updates: Vec<(String, TermDef)> = env
        .keys()
        .filter_map(|var| {
            composed_term_def(var, env, &branch.bindings).map(|def| (var.as_str().to_owned(), def))
        })
        .collect();
    for (var, def) in updates {
        branch.bindings.insert(var, def);
    }
}

/// Build `var`'s [`TermDef::ComposedTriple`] by resolving its
/// [`ComposedInfo`]'s three component vars: a component that is ITSELF an
/// `env` key resolves via ANOTHER recursive call (nested composition,
/// independent of `env`'s name-sorted iteration order — this recursion does
/// not touch `env`'s iteration at all); otherwise its current binding is read
/// from `bindings` directly. `None` if a non-composed component isn't bound
/// in THIS branch — e.g. a `UNION` arm that never composed this variable (a
/// normal, branch-local absence; [`rewrite_union`]'s check only rejects a
/// variable BOTH arms mention but disagree on — a variable only one arm
/// binds at all is ordinary SPARQL UNION behavior).
pub(crate) fn composed_term_def(
    var: &Variable,
    env: &StarEnv,
    bindings: &BTreeMap<String, TermDef>,
) -> Option<TermDef> {
    let info = env.get(var)?;
    let component = |v: &Variable| -> Option<TermDef> {
        if env.contains_key(v) {
            composed_term_def(v, env, bindings)
        } else {
            bindings.get(v.as_str()).cloned()
        }
    };
    Some(TermDef::ComposedTriple {
        subject: Box::new(component(&info.s_var)?),
        predicate: Box::new(component(&info.p_var)?),
        object: Box::new(component(&info.o_var)?),
    })
}

// Unit tests live in `star/tests.rs` (this module resolves to
// `crates/sf-sparql/src/star/tests.rs` — the same split `r2rml.rs`/`r2rml/tests.rs`
// already uses in `sf-mapping`). NOTE: Wave 2b's additions (ADR-0032 D3 items
// 2-4) grew this file well past the project's usual 500-line guideline
// (reported to the team lead, not addressed here — a further submodule split
// is a separate, follow-up refactor safer done against a stable baseline
// than mid-wave).
#[cfg(test)]
mod tests;
