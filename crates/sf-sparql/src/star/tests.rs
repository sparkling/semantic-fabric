//! Unit tests for the ADR-0031 star rewrite: pure `GraphPattern`-shape
//! assertions (no DB, no mapping) — the SQL-level, cross-mapping behavior
//! (reifies elision matching real data, tree/flat parity, the locked v1
//! boundaries) is covered by `sf-conformance/tests/differential_star.rs`.
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

#[test]
fn reifies_wrapper_is_elided_not_translated() {
    // `_:b rdf:reifies <<( ?p ex:hasAge ?age )>> . _:b ex:assertedBy ?src .`
    // — spargebra's bare-syntax desugar shape (ADR-0031 Context).
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
    let rewritten = rewrite_pattern(&gp, &mut n).expect("reifies elision must succeed");
    let GraphPattern::Bgp { patterns } = rewritten else {
        panic!("expected a Bgp");
    };
    // The reifies wrapper triple is GONE (dropped, never translated — R2):
    // 4 basic-encoding patterns + the untouched assertedBy triple = 5.
    assert_eq!(patterns.len(), 5, "got {patterns:#?}");
    assert_basic_encoding(&patterns, 0, &b);
    assert_eq!(patterns[1].object, var("p"));
    assert_eq!(patterns[2].object, pred("http://example.com/hasAge").into());
    assert_eq!(patterns[3].object, var("age"));
    // The assertedBy triple is untouched, same blank node identity.
    assert_eq!(patterns[4].subject, b);
    assert_eq!(patterns[4].predicate, pred("http://example.com/assertedBy"));
    assert_eq!(patterns[4].object, var("src"));
    // No fresh variable was needed (the identity was the existing `_:b`).
    assert_eq!(n, 0);
}

#[test]
fn subject_substitution_mints_a_fresh_var_and_4_patterns() {
    // `<<( ?p ex:hasAge ?age )>> ex:assertedBy ?src` — parenthesized syntax,
    // parses directly to a Triple in subject position (no reifies).
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
    let rewritten = rewrite_pattern(&gp, &mut n).expect("subject substitution must succeed");
    let GraphPattern::Bgp { patterns } = rewritten else {
        panic!("expected a Bgp");
    };
    assert_eq!(patterns.len(), 5, "got {patterns:#?}");
    let fresh = var("__sf_star_0");
    assert_basic_encoding(&patterns, 0, &fresh);
    // The wrapper triple's subject was replaced with the fresh var.
    assert_eq!(patterns[4].subject, fresh);
    assert_eq!(patterns[4].predicate, pred("http://example.com/assertedBy"));
    assert_eq!(patterns[4].object, var("src"));
    assert_eq!(n, 1, "the whole-query counter must have advanced");
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
    // Rule 3 emits the object's 4 basic-encoding patterns as it discovers them
    // (before the wrapper triple is pushed) — same emission order as rule 1's
    // subject case, just mirrored onto the object position.
    let fresh = var("__sf_star_0");
    assert_basic_encoding(&patterns, 0, &fresh);
    assert_eq!(patterns[4].subject, var("q"));
    assert_eq!(patterns[4].predicate, pred("http://example.com/hasQuote"));
    assert_eq!(patterns[4].object, fresh);
}

#[test]
fn nested_quoted_triple_pattern_is_unsupported() {
    let innermost = TriplePattern {
        subject: var("a"),
        predicate: pred("http://example.com/p"),
        object: var("b"),
    };
    let outer = TriplePattern {
        subject: TermPattern::Triple(Box::new(innermost)),
        predicate: pred("http://example.com/q"),
        object: var("c"),
    };
    let gp = bgp_of(vec![TriplePattern {
        subject: TermPattern::Triple(Box::new(outer)),
        predicate: pred("http://example.com/assertedBy"),
        object: var("src"),
    }]);
    let err = rewrite_pattern(&gp, &mut 0).expect_err("nesting must be rejected");
    assert!(matches!(err, Error::Unsupported(_)));
}

#[test]
fn counter_does_not_collide_across_bgp_and_exists_body() {
    // Two independent quoted patterns — one in the outer BGP, one inside a
    // FILTER EXISTS body — must mint DISTINCT fresh vars (R3: one counter
    // spans the whole query, not reset per clause).
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
        subject: TermPattern::Triple(Box::new(quoted_inner)),
        predicate: pred("http://example.com/assertedBy"),
        object: var("src2"),
    }]);
    let gp = GraphPattern::Filter {
        expr: Expression::Exists(Box::new(exists_body)),
        inner: Box::new(bgp_of(vec![TriplePattern {
            subject: TermPattern::Triple(Box::new(quoted_outer)),
            predicate: pred("http://example.com/assertedBy"),
            object: var("src"),
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
        inner_patterns[4].subject,
        var("__sf_star_0"),
        "the EXISTS body is rewritten first"
    );
    assert_eq!(
        outer_patterns[4].subject,
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
