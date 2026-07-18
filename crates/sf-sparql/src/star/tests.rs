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
    let mut env = StarEnv::new();
    let rewritten =
        rewrite_pattern(&gp, &mut n, &mut env).expect("no-elision rewrite must succeed");
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
    let mut env = StarEnv::new();
    let rewritten = rewrite_pattern(&gp, &mut n, &mut env).expect("must succeed, never error");
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
    let mut env = StarEnv::new();
    let rewritten =
        rewrite_pattern(&gp, &mut n, &mut env).expect("object substitution must succeed");
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
    let mut env = StarEnv::new();
    let rewritten =
        rewrite_pattern(&gp, &mut n, &mut env).expect("object-side nesting must succeed");
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
    let mut env = StarEnv::new();
    let rewritten = rewrite_pattern(&gp, &mut n, &mut env).expect("must succeed, never error");
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
    let mut env = StarEnv::new();
    let rewritten = rewrite_pattern(&gp, &mut n, &mut env).expect("must succeed");
    let GraphPattern::Filter { expr, inner } = rewritten else {
        panic!("expected a Filter");
    };
    let GraphPattern::Bgp {
        patterns: outer_patterns,
    } = *inner
    else {
        panic!("expected a Bgp");
    };
    // `GraphPattern::Filter { expr, inner }` rewrites `inner` before `expr`
    // (ADR-0032 D3 item 3 — `inner` is FILTER's own scope, so a variable it
    // composes must already be in `env` before `expr` is checked; see the
    // rewrite arm's own doc comment), so the OUTER BGP — `inner` — draws from
    // the counter FIRST and the EXISTS body — nested inside `expr` — second.
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
        outer_patterns[4].object,
        var("__sf_star_0"),
        "the outer BGP (Filter's `inner`) is rewritten first"
    );
    assert_eq!(
        inner_patterns[4].object,
        var("__sf_star_1"),
        "the EXISTS body must continue the SAME counter, not restart at 0"
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
    let mut env = StarEnv::new();
    let rewritten = rewrite_pattern(&gp, &mut n, &mut env).expect("Values must pass through");
    assert_eq!(rewritten, gp);
    assert_eq!(n, 0);
}

// ============================================================================
// Wave 2b (ADR-0032 D3 item 2-4): the composed-variable environment, the
// reifies-bare-variable / VALUES-decompose / TRIPLE-BIND env-population
// sites, the five triple-term functions, composed-aware equality, and
// CONSTRUCT template pre-substitution. Cross-mapping / SQL-level behavior for
// all of these is covered by `sf-conformance/tests/differential_star.rs`;
// these are pure AST-shape assertions.
// ============================================================================

#[test]
fn reifies_bare_variable_object_composes_t_and_keeps_the_pattern() {
    // `?r rdf:reifies ?t` — ?t a BARE variable (not `<<(...)>>`): the pattern
    // is KEPT unchanged (?t stays bound to the real pf-IRI via ordinary
    // pattern matching) and the 4 description patterns are ADDED on ?t using
    // FRESH component vars, with ?t registered composed in the env.
    let gp = bgp_of(vec![TriplePattern {
        subject: var("r"),
        predicate: pred(RDF_REIFIES),
        object: var("t"),
    }]);
    let mut n = 0;
    let mut env = StarEnv::new();
    let rewritten = rewrite_pattern(&gp, &mut n, &mut env).expect("must succeed");
    let GraphPattern::Bgp { patterns } = rewritten else {
        panic!("expected a Bgp");
    };
    // 4 description patterns on ?t + the KEPT reifies triple = 5.
    assert_eq!(patterns.len(), 5, "got {patterns:#?}");
    assert_basic_encoding(&patterns, 0, &var("t"));
    assert_eq!(patterns[4].subject, var("r"));
    assert_eq!(patterns[4].predicate, pred(RDF_REIFIES));
    assert_eq!(patterns[4].object, var("t"));
    // The env now knows ?t is composed, pointing at the SAME fresh vars the
    // description patterns bound.
    let t = Variable::new_unchecked("t");
    let info = env.get(&t).expect("?t must be registered composed");
    assert_eq!(
        patterns[1].object,
        TermPattern::Variable(info.s_var.clone())
    );
    assert_eq!(
        patterns[2].object,
        TermPattern::Variable(info.p_var.clone())
    );
    assert_eq!(
        patterns[3].object,
        TermPattern::Variable(info.o_var.clone())
    );
}

#[test]
fn reifies_bare_variable_env_lookup_reuses_component_vars_across_occurrences() {
    // The SAME ?t reified twice (e.g. two BGP conjuncts, or a Join) must
    // reuse the SAME component vars both times — never mint a second,
    // disjoint set — so an ordinary shared-variable join correlates them.
    let gp = bgp_of(vec![
        TriplePattern {
            subject: var("r1"),
            predicate: pred(RDF_REIFIES),
            object: var("t"),
        },
        TriplePattern {
            subject: var("r2"),
            predicate: pred(RDF_REIFIES),
            object: var("t"),
        },
    ]);
    let mut n = 0;
    let mut env = StarEnv::new();
    rewrite_pattern(&gp, &mut n, &mut env).expect("must succeed");
    assert_eq!(env.len(), 1, "only ONE env entry for the shared ?t");
}

#[test]
fn values_decomposes_a_ground_triple_column() {
    // `VALUES ?t { <<( ex:a ex:hasAge ex:b )>> }` — decomposes ?t's column
    // into 3 fresh component columns carrying the ground s/p/o values as
    // rows, registering ?t composed.
    let quoted = GroundTriple {
        subject: OxNamedNode::new_unchecked("http://example.com/a"),
        predicate: OxNamedNode::new_unchecked("http://example.com/hasAge"),
        object: GroundTerm::Literal(Literal::new_simple_literal("30")),
    };
    let gp = GraphPattern::Values {
        variables: vec![Variable::new_unchecked("t")],
        bindings: vec![vec![Some(GroundTerm::Triple(Box::new(quoted)))]],
    };
    let mut n = 0;
    let mut env = StarEnv::new();
    let rewritten = rewrite_pattern(&gp, &mut n, &mut env).expect("must decompose");
    let GraphPattern::Values {
        variables,
        bindings,
    } = rewritten
    else {
        panic!("expected Values");
    };
    assert_eq!(variables.len(), 3, "?t's column replaced by 3 components");
    assert_eq!(bindings.len(), 1, "row count unchanged");
    assert_eq!(bindings[0].len(), 3);
    assert_eq!(
        bindings[0][0],
        Some(GroundTerm::NamedNode(OxNamedNode::new_unchecked(
            "http://example.com/a"
        )))
    );
    assert_eq!(
        bindings[0][1],
        Some(GroundTerm::NamedNode(OxNamedNode::new_unchecked(
            "http://example.com/hasAge"
        )))
    );
    assert_eq!(
        bindings[0][2],
        Some(GroundTerm::Literal(Literal::new_simple_literal("30")))
    );
    let t = Variable::new_unchecked("t");
    let info = env.get(&t).expect("?t must be registered composed");
    assert_eq!(
        variables,
        vec![info.s_var.clone(), info.p_var.clone(), info.o_var.clone()]
    );
}

#[test]
fn values_decomposes_nested_ground_triples_recursively() {
    // `VALUES ?t { <<( ex:a ex:p <<( ex:x ex:q ex:y )>> )>> }` — the outer's
    // object is ITSELF a ground triple: the object column recurses into its
    // own 3 components (5 columns total: outer s/p + inner s/p/o).
    let inner = GroundTriple {
        subject: OxNamedNode::new_unchecked("http://example.com/x"),
        predicate: OxNamedNode::new_unchecked("http://example.com/q"),
        object: GroundTerm::NamedNode(OxNamedNode::new_unchecked("http://example.com/y")),
    };
    let outer = GroundTriple {
        subject: OxNamedNode::new_unchecked("http://example.com/a"),
        predicate: OxNamedNode::new_unchecked("http://example.com/p"),
        object: GroundTerm::Triple(Box::new(inner)),
    };
    let gp = GraphPattern::Values {
        variables: vec![Variable::new_unchecked("t")],
        bindings: vec![vec![Some(GroundTerm::Triple(Box::new(outer)))]],
    };
    let mut n = 0;
    let mut env = StarEnv::new();
    let rewritten = rewrite_pattern(&gp, &mut n, &mut env).expect("must decompose recursively");
    let GraphPattern::Values { variables, .. } = rewritten else {
        panic!("expected Values");
    };
    assert_eq!(variables.len(), 5, "outer s/p + inner s/p/o");
    let t = Variable::new_unchecked("t");
    let outer_info = env.get(&t).expect("?t composed").clone();
    let inner_info = env
        .get(&outer_info.o_var)
        .expect("the outer's o_var must ALSO be registered composed (nesting)");
    assert_eq!(
        variables,
        vec![
            outer_info.s_var,
            outer_info.p_var,
            inner_info.s_var.clone(),
            inner_info.p_var.clone(),
            inner_info.o_var.clone()
        ]
    );
}

#[test]
fn values_mixed_triple_and_plain_cells_is_unsupported() {
    // A column mixing a ground triple-term cell with a NamedNode cell for the
    // SAME variable is a genuine shape ambiguity → explicit Unsupported.
    let quoted = GroundTriple {
        subject: OxNamedNode::new_unchecked("http://example.com/a"),
        predicate: OxNamedNode::new_unchecked("http://example.com/hasAge"),
        object: GroundTerm::Literal(Literal::new_simple_literal("30")),
    };
    let gp = GraphPattern::Values {
        variables: vec![Variable::new_unchecked("t")],
        bindings: vec![
            vec![Some(GroundTerm::Triple(Box::new(quoted)))],
            vec![Some(GroundTerm::NamedNode(OxNamedNode::new_unchecked(
                "http://example.com/plain",
            )))],
        ],
    };
    let mut n = 0;
    let mut env = StarEnv::new();
    let result = rewrite_pattern(&gp, &mut n, &mut env);
    assert!(
        matches!(result, Err(Error::Unsupported(_))),
        "expected Unsupported, got {result:?}"
    );
}

#[test]
fn is_triple_resolves_statically_to_a_boolean_literal() {
    // `FILTER isTRIPLE(?t)` — ?t composed (via a preceding reifies-bare-var
    // pattern in the SAME BGP) → `true`; a non-composed argument → `false`.
    // Never an error, either way (§17.4.6 asymmetry).
    let composed_gp = bgp_of(vec![TriplePattern {
        subject: var("r"),
        predicate: pred(RDF_REIFIES),
        object: var("t"),
    }]);
    let filter = GraphPattern::Filter {
        expr: Expression::FunctionCall(
            Function::IsTriple,
            vec![Expression::Variable(Variable::new_unchecked("t"))],
        ),
        inner: Box::new(composed_gp),
    };
    let mut n = 0;
    let mut env = StarEnv::new();
    let rewritten = rewrite_pattern(&filter, &mut n, &mut env).expect("must succeed");
    let GraphPattern::Filter { expr, .. } = rewritten else {
        panic!("expected Filter");
    };
    assert_eq!(expr, bool_literal_expr(true));

    let mut n2 = 0;
    let mut env2 = StarEnv::new();
    let non_composed = Expression::FunctionCall(
        Function::IsTriple,
        vec![Expression::Variable(Variable::new_unchecked("plain"))],
    );
    let rewritten2 =
        rewrite_expr(&non_composed, &mut n2, &mut env2).expect("must succeed, never error");
    assert_eq!(rewritten2, bool_literal_expr(false));
}

#[test]
fn subject_predicate_object_on_composed_var_resolve_to_component_vars() {
    let composed_gp = bgp_of(vec![TriplePattern {
        subject: var("r"),
        predicate: pred(RDF_REIFIES),
        object: var("t"),
    }]);
    let mut n = 0;
    let mut env = StarEnv::new();
    rewrite_pattern(&composed_gp, &mut n, &mut env).expect("must succeed");
    let info = env
        .get(&Variable::new_unchecked("t"))
        .expect("?t composed")
        .clone();

    let t_expr = Expression::Variable(Variable::new_unchecked("t"));
    for (func, expected) in [
        (Function::Subject, &info.s_var),
        (Function::Predicate, &info.p_var),
        (Function::Object, &info.o_var),
    ] {
        let call = Expression::FunctionCall(func, vec![t_expr.clone()]);
        let rewritten =
            rewrite_expr(&call, &mut n, &mut env).expect("composed argument must resolve");
        assert_eq!(rewritten, Expression::Variable(expected.clone()));
    }
}

#[test]
fn subject_on_non_composed_var_resolves_to_the_error_marker() {
    // Engine-totality: a variable never registered composed provably never
    // holds a triple term at runtime — SUBJECT/PREDICATE/OBJECT on it is the
    // §17.4.6 error, represented uniformly (see `error_marker_expr`'s doc
    // comment) as `CONCAT(<urn:sf-star:error-marker>)`.
    let mut n = 0;
    let mut env = StarEnv::new();
    let call = Expression::FunctionCall(
        Function::Subject,
        vec![Expression::Variable(Variable::new_unchecked("plain"))],
    );
    let rewritten = rewrite_expr(&call, &mut n, &mut env).expect("must succeed, never error");
    assert_eq!(rewritten, error_marker_expr());
}

#[test]
fn triple_bind_target_marks_the_var_composed_via_synthetic_extends() {
    // `BIND(TRIPLE(?a, ex:p, ?c) AS ?t)` — ?t becomes composed; the single
    // BIND is replaced by 3 chained synthetic Extends. `rewrite_extend_inner`
    // binds s_var then p_var, THEN RECURSES for the object position (?c,
    // here a plain Variable so the recursion hits its base case and just
    // wraps once more) — so the object Extend (o_var) ends up OUTERMOST,
    // wrapping predicate (p_var), wrapping subject (s_var), wrapping the
    // original (empty) inner.
    let a = Expression::Variable(Variable::new_unchecked("a"));
    let c = Expression::Variable(Variable::new_unchecked("c"));
    let p = Expression::NamedNode(OxNamedNode::new_unchecked("http://example.com/p"));
    let bind = GraphPattern::Extend {
        inner: Box::new(bgp_of(vec![])),
        variable: Variable::new_unchecked("t"),
        expression: Expression::FunctionCall(
            Function::Triple,
            vec![a.clone(), p.clone(), c.clone()],
        ),
    };
    let mut n = 0;
    let mut env = StarEnv::new();
    let rewritten = rewrite_pattern(&bind, &mut n, &mut env).expect("must succeed");
    let t = Variable::new_unchecked("t");
    let info = env.get(&t).expect("?t must be registered composed").clone();

    let GraphPattern::Extend {
        variable: o_var,
        expression: o_expr,
        inner,
    } = rewritten
    else {
        panic!("expected the outermost (object) Extend");
    };
    assert_eq!(o_var, info.o_var);
    assert_eq!(o_expr, c);
    let GraphPattern::Extend {
        variable: p_var,
        expression: p_expr,
        inner,
    } = *inner
    else {
        panic!("expected the middle (predicate) Extend");
    };
    assert_eq!(p_var, info.p_var);
    assert_eq!(p_expr, p);
    let GraphPattern::Extend {
        variable: s_var,
        expression: s_expr,
        ..
    } = *inner
    else {
        panic!("expected the innermost (subject) Extend");
    };
    assert_eq!(s_var, info.s_var);
    assert_eq!(s_expr, a);
}

#[test]
fn equality_both_composed_is_a_componentwise_conjunction() {
    // `FILTER(?t1 = ?t2)`, both composed — rewrites to a 3-way AND of
    // subject/predicate/object component comparisons.
    let gp = bgp_of(vec![
        TriplePattern {
            subject: var("r1"),
            predicate: pred(RDF_REIFIES),
            object: var("t1"),
        },
        TriplePattern {
            subject: var("r2"),
            predicate: pred(RDF_REIFIES),
            object: var("t2"),
        },
    ]);
    let mut n = 0;
    let mut env = StarEnv::new();
    rewrite_pattern(&gp, &mut n, &mut env).expect("must succeed");
    let info1 = env.get(&Variable::new_unchecked("t1")).unwrap().clone();
    let info2 = env.get(&Variable::new_unchecked("t2")).unwrap().clone();

    let eq = Expression::Equal(
        Box::new(Expression::Variable(Variable::new_unchecked("t1"))),
        Box::new(Expression::Variable(Variable::new_unchecked("t2"))),
    );
    let rewritten = rewrite_expr(&eq, &mut n, &mut env).expect("must succeed");
    let expected = Expression::And(
        Box::new(Expression::And(
            Box::new(Expression::Equal(
                Box::new(Expression::Variable(info1.s_var)),
                Box::new(Expression::Variable(info2.s_var)),
            )),
            Box::new(Expression::Equal(
                Box::new(Expression::Variable(info1.p_var)),
                Box::new(Expression::Variable(info2.p_var)),
            )),
        )),
        Box::new(Expression::Equal(
            Box::new(Expression::Variable(info1.o_var)),
            Box::new(Expression::Variable(info2.o_var)),
        )),
    );
    assert_eq!(rewritten, expected);
}

#[test]
fn equality_exactly_one_composed_is_constant_false() {
    let gp = bgp_of(vec![TriplePattern {
        subject: var("r"),
        predicate: pred(RDF_REIFIES),
        object: var("t"),
    }]);
    let mut n = 0;
    let mut env = StarEnv::new();
    rewrite_pattern(&gp, &mut n, &mut env).expect("must succeed");

    for expr in [
        Expression::Equal(
            Box::new(Expression::Variable(Variable::new_unchecked("t"))),
            Box::new(Expression::NamedNode(OxNamedNode::new_unchecked(
                "http://example.com/x",
            ))),
        ),
        Expression::SameTerm(
            Box::new(Expression::Variable(Variable::new_unchecked("t"))),
            Box::new(Expression::Variable(Variable::new_unchecked("plain"))),
        ),
    ] {
        let rewritten = rewrite_expr(&expr, &mut n, &mut env).expect("must succeed");
        assert_eq!(rewritten, bool_literal_expr(false), "expr={expr:?}");
    }
}

#[test]
fn union_arms_agreeing_on_composed_ness_succeeds() {
    // Both arms independently reify the SAME `?t` — env lookup-before-mint
    // means they reuse the SAME component vars, so this is NOT a
    // disagreement (contrast with the next test).
    let arm = |r: &str| {
        bgp_of(vec![TriplePattern {
            subject: var(r),
            predicate: pred(RDF_REIFIES),
            object: var("t"),
        }])
    };
    let gp = GraphPattern::Union {
        left: Box::new(arm("r1")),
        right: Box::new(arm("r2")),
    };
    let mut n = 0;
    let mut env = StarEnv::new();
    let result = rewrite_pattern(&gp, &mut n, &mut env);
    assert!(result.is_ok(), "got {result:?}");
    assert_eq!(env.len(), 1, "the shared ?t reuses ONE env entry");
}

#[test]
fn union_arms_disagreeing_on_composed_ness_is_unsupported() {
    // Left composes ?t (via reifies); right binds the SAME ?t as an
    // ordinary, non-composing pattern variable — the uniform-composed-ness
    // law rejects this explicitly rather than allowing ?t to be "sometimes a
    // triple term" depending on which arm produced a row.
    let left = bgp_of(vec![TriplePattern {
        subject: var("r"),
        predicate: pred(RDF_REIFIES),
        object: var("t"),
    }]);
    let right = bgp_of(vec![TriplePattern {
        subject: var("t"),
        predicate: pred("http://example.com/type"),
        object: iri("http://example.com/Foo"),
    }]);
    let gp = GraphPattern::Union {
        left: Box::new(left),
        right: Box::new(right),
    };
    let mut n = 0;
    let mut env = StarEnv::new();
    let result = rewrite_pattern(&gp, &mut n, &mut env);
    assert!(
        matches!(result, Err(Error::Unsupported(_))),
        "expected Unsupported, got {result:?}"
    );
}

#[test]
fn construct_template_substitutes_a_composed_variable_recursively() {
    let gp = bgp_of(vec![TriplePattern {
        subject: var("r"),
        predicate: pred(RDF_REIFIES),
        object: var("t"),
    }]);
    let mut n = 0;
    let mut env = StarEnv::new();
    rewrite_pattern(&gp, &mut n, &mut env).expect("must succeed");
    let info = env.get(&Variable::new_unchecked("t")).unwrap().clone();

    let plain = TriplePattern {
        subject: var("s"),
        predicate: pred("http://example.com/p"),
        object: var("o"),
    };
    let composed = TriplePattern {
        subject: var("q"),
        predicate: pred("http://example.com/hasQuote"),
        object: var("t"),
    };
    let out = substitute_construct_template(&[plain.clone(), composed], &env);
    assert_eq!(out[0], plain, "an ordinary triple is untouched");
    assert_eq!(
        out[1],
        TriplePattern {
            subject: var("q"),
            predicate: pred("http://example.com/hasQuote"),
            object: TermPattern::Triple(Box::new(TriplePattern {
                subject: var(info.s_var.as_str()),
                predicate: NamedNodePattern::Variable(info.p_var),
                object: var(info.o_var.as_str()),
            })),
        },
        "?t substitutes to an explicit TermPattern::Triple over its components"
    );
}
