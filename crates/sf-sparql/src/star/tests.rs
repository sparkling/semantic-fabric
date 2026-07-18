//! Unit tests for the ADR-0032 D3 star rewrite: pure `GraphPattern`-shape
//! assertions (no DB, no mapping) — the SQL-level, cross-mapping behavior
//! (reifies join matching real data, tree/flat parity, the locked boundaries)
//! is covered by `sf-conformance/tests/differential_star.rs`.
//! `use super::*` re-uses the parent module's rewrite functions (all
//! private except the two `pub` entry points) plus its spargebra imports.

use super::*;
use oxrdf::{Literal, NamedNode as OxNamedNode};
use spargebra::term::BlankNode;

fn var(s: &str) -> TermPattern {
    TermPattern::Variable(Variable::new_unchecked(s))
}

fn iri(s: &str) -> TermPattern {
    TermPattern::NamedNode(OxNamedNode::new_unchecked(s))
}

fn pred(s: &str) -> NamedNodePattern {
    NamedNodePattern::NamedNode(OxNamedNode::new_unchecked(s))
}

fn bgp_of(patterns: Vec<TriplePattern>) -> GraphPattern {
    GraphPattern::Bgp { patterns }
}

/// The 4 basic-encoding predicates a rewritten identity carries, in the order
/// `emit_basic_encoding` pushes them (type, subject, predicate, object) — used
/// to assert shape without repeating IRIs everywhere.
fn assert_basic_encoding(out: &[TriplePattern], start: usize, identity: &TermPattern) {
    assert_eq!(out[start].subject, *identity);
    assert_eq!(out[start].predicate, pred(RDF_TYPE));
    assert_eq!(
        out[start].object,
        iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#PropositionForm")
    );
    assert_eq!(out[start + 1].subject, *identity);
    assert_eq!(
        out[start + 1].predicate,
        pred("http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormSubject")
    );
    assert_eq!(out[start + 2].subject, *identity);
    assert_eq!(
        out[start + 2].predicate,
        pred("http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormPredicate")
    );
    assert_eq!(out[start + 3].subject, *identity);
    assert_eq!(
        out[start + 3].predicate,
        pred("http://www.w3.org/1999/02/22-rdf-syntax-ns#propositionFormObject")
    );
}

/// A rewritten `GraphPattern` is the rule-R1 zero-solution replacement: a
/// `Values` clause with a single fresh `__sf_star_empty_*` variable and no
/// rows.
fn assert_empty(gp: &GraphPattern) {
    let GraphPattern::Values {
        variables,
        bindings,
    } = gp
    else {
        panic!("expected a zero-row Values, got {gp:#?}");
    };
    assert_eq!(variables.len(), 1);
    assert!(
        variables[0].as_str().starts_with("__sf_star_empty_"),
        "unexpected empty-marker variable: {}",
        variables[0]
    );
    assert!(bindings.is_empty());
}

#[test]
fn reifies_wrapper_is_kept_pointing_at_a_fresh_pf_var() {
    // `_:b rdf:reifies <<( ?p ex:hasAge ?age )>> . _:b ex:assertedBy ?src .`
    // — spargebra's bare-syntax desugar shape (ADR-0031 Context). ADR-0032 D3
    // rule R2: NO elision — the wrapper triple is KEPT, now pointing at a
    // fresh `?pf` var (decoupled from `_:b`, which stays the reifier).
    let b = TermPattern::BlankNode(BlankNode::new_unchecked("b"));
    let quoted = TriplePattern {
        subject: var("p"),
        predicate: pred("http://example.com/hasAge"),
        object: var("age"),
    };
    let gp = bgp_of(vec![
        TriplePattern {
            subject: b.clone(),
            predicate: pred(RDF_REIFIES),
            object: TermPattern::Triple(Box::new(quoted)),
        },
        TriplePattern {
            subject: b.clone(),
            predicate: pred("http://example.com/assertedBy"),
            object: var("src"),
        },
    ]);
    let mut n = 0;
    let rewritten = rewrite_pattern(&gp, &mut n).expect("no-elision rewrite must succeed");
    let GraphPattern::Bgp { patterns } = rewritten else {
        panic!("expected a Bgp");
    };
    // 4 basic-encoding patterns on the fresh `?pf` + the KEPT reifies triple
    // (subject `_:b`, object `?pf`) + the untouched assertedBy triple = 6.
    assert_eq!(patterns.len(), 6, "got {patterns:#?}");
    let pf = var("__sf_star_0");
    assert_basic_encoding(&patterns, 0, &pf);
    assert_eq!(patterns[1].object, var("p"));
    assert_eq!(patterns[2].object, pred("http://example.com/hasAge").into());
    assert_eq!(patterns[3].object, var("age"));
    // The reifies triple survives, `_:b` untouched, object now `?pf`.
    assert_eq!(patterns[4].subject, b);
    assert_eq!(patterns[4].predicate, pred(RDF_REIFIES));
    assert_eq!(patterns[4].object, pf);
    // The assertedBy triple is untouched, same blank node identity.
    assert_eq!(patterns[5].subject, b);
    assert_eq!(patterns[5].predicate, pred("http://example.com/assertedBy"));
    assert_eq!(patterns[5].object, var("src"));
    // One fresh var was needed for `?pf` (unlike v1, where the identity was
    // the existing `_:b` and the wrapper was dropped).
    assert_eq!(n, 1);
}

#[test]
fn subject_position_triple_term_rewrites_to_empty_values() {
    // `<<( ?p ex:hasAge ?age )>> ex:assertedBy ?src` — parenthesized syntax
    // parses directly to a Triple in SUBJECT position. SPARQL 1.2 §18.1.3: a
    // triple pattern with a Triple-typed subject can never match — rule R1
    // rewrites the whole BGP to a zero-row VALUES, never an error.
    let quoted = TriplePattern {
        subject: var("p"),
        predicate: pred("http://example.com/hasAge"),
        object: var("age"),
    };
    let gp = bgp_of(vec![TriplePattern {
        subject: TermPattern::Triple(Box::new(quoted)),
        predicate: pred("http://example.com/assertedBy"),
        object: var("src"),
    }]);
    let mut n = 0;
    let rewritten = rewrite_pattern(&gp, &mut n).expect("must succeed, never error");
    assert_empty(&rewritten);
    assert_eq!(n, 1, "one empty-marker var minted");
}

#[test]
fn object_substitution_is_symmetric_with_subject() {
    let quoted = TriplePattern {
        subject: var("p"),
        predicate: pred("http://example.com/hasAge"),
        object: var("age"),
    };
    let gp = bgp_of(vec![TriplePattern {
        subject: var("q"),
        predicate: pred("http://example.com/hasQuote"),
        object: TermPattern::Triple(Box::new(quoted)),
    }]);
    let mut n = 0;
    let rewritten = rewrite_pattern(&gp, &mut n).expect("object substitution must succeed");
    let GraphPattern::Bgp { patterns } = rewritten else {
        panic!("expected a Bgp");
    };
    assert_eq!(patterns.len(), 5, "got {patterns:#?}");
    // Rule R3 emits the object's 4 basic-encoding patterns as it discovers them
    // (before the wrapper triple is pushed) — same emission order as R2's
    // reifies case, just mirrored onto an ordinary predicate.
    let fresh = var("__sf_star_0");
    assert_basic_encoding(&patterns, 0, &fresh);
    assert_eq!(patterns[4].subject, var("q"));
    assert_eq!(patterns[4].predicate, pred("http://example.com/hasQuote"));
    assert_eq!(patterns[4].object, fresh);
}

#[test]
fn object_side_nesting_recurses_bottom_up() {
    // `?x ex:hasQuote <<( ?p ex:hasAge <<( ?p ex:hasScore ?s )>> )>>` — the
    // outer quote's own OBJECT is ANOTHER quoted triple (rule R4): the inner
    // quote must mint its own identity + 4 patterns FIRST (pattern EMISSION
    // is bottom-up), and the outer's propositionFormObject must point at
    // that inner identity rather than embedding the raw `TermPattern::Triple`.
    let inner_quoted = TriplePattern {
        subject: var("p"),
        predicate: pred("http://example.com/hasScore"),
        object: var("s"),
    };
    let outer_quoted = TriplePattern {
        subject: var("p"),
        predicate: pred("http://example.com/hasAge"),
        object: TermPattern::Triple(Box::new(inner_quoted)),
    };
    let gp = bgp_of(vec![TriplePattern {
        subject: var("x"),
        predicate: pred("http://example.com/hasQuote"),
        object: TermPattern::Triple(Box::new(outer_quoted)),
    }]);
    let mut n = 0;
    let rewritten = rewrite_pattern(&gp, &mut n).expect("object-side nesting must succeed");
    let GraphPattern::Bgp { patterns } = rewritten else {
        panic!("expected a Bgp");
    };
    assert_eq!(patterns.len(), 9, "got {patterns:#?}");
    // VARIABLE NUMBERING is outer-first (`substitute_triple` mints the outer
    // identity before ever recursing into `emit_basic_encoding`, which only
    // THEN mints the inner one while expanding the outer's own object) even
    // though pattern EMISSION is inner-first (bottom-up).
    let outer_id = var("__sf_star_0");
    let inner_id = var("__sf_star_1");
    assert_basic_encoding(&patterns, 0, &inner_id);
    assert_eq!(patterns[1].object, var("p"));
    assert_eq!(
        patterns[2].object,
        pred("http://example.com/hasScore").into()
    );
    assert_eq!(patterns[3].object, var("s"));
    assert_basic_encoding(&patterns, 4, &outer_id);
    assert_eq!(patterns[5].object, var("p"));
    assert_eq!(patterns[6].object, pred("http://example.com/hasAge").into());
    assert_eq!(
        patterns[7].object, inner_id,
        "outer's propositionFormObject must point at the inner identity, not embed the raw Triple"
    );
    assert_eq!(patterns[8].subject, var("x"));
    assert_eq!(patterns[8].predicate, pred("http://example.com/hasQuote"));
    assert_eq!(patterns[8].object, outer_id);
    assert_eq!(n, 2, "two identities minted, one per nesting level");
}

#[test]
fn object_chain_nested_subject_side_triple_term_is_empty() {
    // `?r rdf:reifies <<( <<( ?a ex:p ?b )>> ex:q ?c )>>` — the quoted
    // triple reached through the reifies rule's object has its OWN subject
    // be another quoted triple (subject-side nesting, spec-impossible at any
    // depth per RDF 1.2 Concepts §3.1). `has_subject_position_triple_term`'s
    // object-chain recursion must catch this even though it is not the
    // TOP-level pattern's own subject (that case is
    // `subject_position_triple_term_rewrites_to_empty_values`, above).
    let innermost = TriplePattern {
        subject: var("a"),
        predicate: pred("http://example.com/p"),
        object: var("b"),
    };
    let mid = TriplePattern {
        subject: TermPattern::Triple(Box::new(innermost)),
        predicate: pred("http://example.com/q"),
        object: var("c"),
    };
    let gp = bgp_of(vec![TriplePattern {
        subject: var("r"),
        predicate: pred(RDF_REIFIES),
        object: TermPattern::Triple(Box::new(mid)),
    }]);
    let mut n = 0;
    let rewritten = rewrite_pattern(&gp, &mut n).expect("must succeed, never error");
    assert_empty(&rewritten);
}

#[test]
fn counter_does_not_collide_across_bgp_and_exists_body() {
    // Two independent quoted patterns — one in the outer BGP, one inside a
    // FILTER EXISTS body — must mint DISTINCT fresh vars (the whole-query
    // counter spans the whole query, not reset per clause). Object position
    // (rule R3) is used for both: subject-position quotes (rule R1) no
    // longer mint an identity at all, they rewrite straight to empty.
    let quoted_outer = TriplePattern {
        subject: var("p"),
        predicate: pred("http://example.com/hasAge"),
        object: var("age"),
    };
    let quoted_inner = TriplePattern {
        subject: var("x"),
        predicate: pred("http://example.com/hasAge"),
        object: var("y"),
    };
    let exists_body = bgp_of(vec![TriplePattern {
        subject: var("z"),
        predicate: pred("http://example.com/assertedBy"),
        object: TermPattern::Triple(Box::new(quoted_inner)),
    }]);
    let gp = GraphPattern::Filter {
        expr: Expression::Exists(Box::new(exists_body)),
        inner: Box::new(bgp_of(vec![TriplePattern {
            subject: var("w"),
            predicate: pred("http://example.com/assertedBy"),
            object: TermPattern::Triple(Box::new(quoted_outer)),
        }])),
    };
    let mut n = 0;
    let rewritten = rewrite_pattern(&gp, &mut n).expect("must succeed");
    let GraphPattern::Filter { expr, inner } = rewritten else {
        panic!("expected a Filter");
    };
    let GraphPattern::Bgp {
        patterns: outer_patterns,
    } = *inner
    else {
        panic!("expected a Bgp");
    };
    // `GraphPattern::Filter { expr, inner }` rewrites `expr` before `inner`
    // (struct-literal source order in `rewrite_pattern`'s Filter arm), so the
    // EXISTS body — inside `expr` — draws from the counter FIRST.
    let Expression::Exists(body) = expr else {
        panic!("expected Exists");
    };
    let GraphPattern::Bgp {
        patterns: inner_patterns,
    } = *body
    else {
        panic!("expected a Bgp");
    };
    assert_eq!(
        inner_patterns[4].object,
        var("__sf_star_0"),
        "the EXISTS body is rewritten first"
    );
    assert_eq!(
        outer_patterns[4].object,
        var("__sf_star_1"),
        "the outer BGP must continue the SAME counter, not restart at 0"
    );
    assert_eq!(n, 2);
}

#[test]
fn values_is_untouched() {
    let gp = GraphPattern::Values {
        variables: vec![Variable::new_unchecked("t")],
        bindings: vec![vec![Some(spargebra::term::GroundTerm::Literal(
            Literal::new_simple_literal("x"),
        ))]],
    };
    let mut n = 0;
    let rewritten = rewrite_pattern(&gp, &mut n).expect("Values must pass through");
    assert_eq!(rewritten, gp);
    assert_eq!(n, 0);
}

#[test]
fn construct_template_flags_a_quoted_triple_in_either_position() {
    let plain = TriplePattern {
        subject: var("s"),
        predicate: pred("http://example.com/p"),
        object: var("o"),
    };
    assert!(!construct_template_has_quoted_triple(std::slice::from_ref(
        &plain
    )));

    let quoted = TriplePattern {
        subject: var("a"),
        predicate: pred("http://example.com/p"),
        object: var("b"),
    };
    let subject_quoted = TriplePattern {
        subject: TermPattern::Triple(Box::new(quoted.clone())),
        predicate: pred("http://example.com/q"),
        object: var("c"),
    };
    assert!(construct_template_has_quoted_triple(&[
        plain.clone(),
        subject_quoted
    ]));

    let object_quoted = TriplePattern {
        subject: var("c"),
        predicate: pred("http://example.com/q"),
        object: TermPattern::Triple(Box::new(quoted)),
    };
    assert!(construct_template_has_quoted_triple(&[
        plain,
        object_quoted
    ]));
}
