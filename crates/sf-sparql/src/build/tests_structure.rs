use spargebra::algebra::{Expression, GraphPattern, PropertyPathExpression};
use spargebra::term::{NamedNodePattern, TermPattern};

use crate::iq::node::{BindDef, IqCond, IqNode};

use super::build_tree;
use super::test_support::{bgp, iri, pattern, triple, var, vec_var};

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
        IqNode::Intensional { graph: Some(NamedNodePattern::NamedNode(g)), .. }
            if g.as_str() == "http://g/"
    ));
}

/// ADR-0035: a variable graph name is no longer a build-time 501 — it pushes onto
/// the leaf exactly like a constant (§5.2 item 6 superseded); RESOLVE decides how
/// it enumerates, not BUILD (which has no mapping access to do so anyway).
#[test]
fn variable_graph_pushes_onto_intensional_leaf() {
    let t = build_tree(&pattern("SELECT * WHERE { GRAPH ?g { ?s ?p ?o } }"), None).unwrap();
    let IqNode::Construction { child, .. } = &t else {
        panic!("expected Project, got {t:?}");
    };
    assert!(matches!(
        child.as_ref(),
        IqNode::Intensional { graph: Some(NamedNodePattern::Variable(v)), .. }
            if v.as_str() == "g"
    ));
    // `?g` is part of the leaf's own output scope, alongside ?s/?p/?o.
    assert_eq!(child.output_vars(), vec_var(&["s", "p", "o", "g"]));
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
