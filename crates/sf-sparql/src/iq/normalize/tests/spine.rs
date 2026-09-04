use super::*;

/// A single-pattern query normalizes to ONE leaf-CQ (no Union wrapper): a
/// Construction over the Extensional scan, carrying both bound variables. The
/// column-valued object `?n` rides an R2RML §11 NULL guard (`IsNotNull`) on the
/// scan's condition (a NULL column ⇒ no triple).
#[test]
fn single_pattern_is_one_leaf_cq() {
    use crate::iq::SqlCond;
    let body = strip_spine(&norm("SELECT * WHERE { ?s <http://ex/name> ?n }")).clone();
    assert_leaf_cq(&body);
    let IqNode::Construction { child, subst, .. } = &body else {
        unreachable!()
    };
    let IqNode::InnerJoin { children, cond } = &**child else {
        panic!("the scan plus its §11 NULL guard is an InnerJoin over one leaf, got {child:?}");
    };
    assert!(
        matches!(children.as_slice(), [IqNode::Extensional { .. }]),
        "{children:?}"
    );
    assert!(
        cond.iter()
            .any(|c| matches!(c, IqCond::Sql(SqlCond::IsNotNull(_)))),
        "the §11 NULL guard for the column object rides the cond: {cond:?}"
    );
    assert!(
        subst.contains_key("s") && subst.contains_key("n"),
        "the single bindings map carries ?s and ?n: {subst:?}"
    );
}

/// A 2-triple BGP normalizes to ONE Construction over an InnerJoin of Extensional
/// leaves; the shared-variable equality is materialised as an `IqCond::Sql` on the
/// join (the tree form of the flat `merge`).
#[test]
fn bgp_join_lifts_to_one_construction_with_shared_var_equality() {
    let body = strip_spine(&norm(
        "SELECT * WHERE { ?s <http://ex/name> ?n . ?s <http://ex/dept> ?d }",
    ))
    .clone();
    assert_leaf_cq(&body);
    let IqNode::Construction { child, subst, .. } = &body else {
        unreachable!()
    };
    let IqNode::InnerJoin { children, cond } = &**child else {
        panic!("expected an InnerJoin of leaves, got {child:?}");
    };
    assert!(
        children
            .iter()
            .all(|c| matches!(c, IqNode::Extensional { .. })),
        "all join children are bare Extensional leaves: {children:?}"
    );
    assert!(
        cond.iter().any(|c| matches!(c, IqCond::Sql(_))),
        "the shared ?s equality rides the InnerJoin cond as IqCond::Sql: {cond:?}"
    );
    assert!(
        subst.contains_key("s") && subst.contains_key("n") && subst.contains_key("d"),
        "one merged bindings map carries every variable: {subst:?}"
    );
}

/// An InnerJoin with a multi-arm Union operand distributes to a Union of joins,
/// each arm a lifted leaf-CQ (design §4.16, either operand).
#[test]
fn inner_join_distributes_over_union() {
    // `?s ?p ?o` is a multi-arm Union; joined with the single-arm `?s :name ?n`.
    let body = strip_spine(&norm(
        "SELECT * WHERE { ?s ?p ?o . ?s <http://ex/name> ?n }",
    ))
    .clone();
    let IqNode::Union { children, .. } = &body else {
        panic!("a join over a Union must distribute to a Union of joins, got {body:?}");
    };
    assert!(
        children.len() >= 2,
        "expected ≥2 distributed arms: {body:?}"
    );
    for arm in children {
        assert_leaf_cq(arm);
    }
}

/// A LeftJoin whose RIGHT is a multi-arm Union STAYS a single LeftJoin — it is
/// NEVER split over the non-preserved side (ledger R2 / design §4.16).
#[test]
fn left_join_over_right_union_stays_single_left_join() {
    let body = strip_spine(&norm(
        "SELECT * WHERE { ?s <http://ex/name> ?n OPTIONAL { ?s ?p ?o } }",
    ))
    .clone();
    let IqNode::LeftJoin { right, .. } = &body else {
        panic!("a LeftJoin with a Union right must stay a single LeftJoin, got {body:?}");
    };
    assert!(
        matches!(**right, IqNode::Union { .. }),
        "the right Union is preserved (not distributed): {right:?}"
    );
}

/// A LeftJoin whose LEFT is a multi-arm Union distributes over the preserved side:
/// `(A∪B)⟕C ⇒ (A⟕C)∪(B⟕C)`.
#[test]
fn left_join_over_left_union_distributes() {
    let body = strip_spine(&norm(
        "SELECT * WHERE { ?s ?p ?o OPTIONAL { ?s <http://ex/name> ?n } }",
    ))
    .clone();
    let IqNode::Union { children, .. } = &body else {
        panic!("a LeftJoin over a LEFT Union must distribute to a Union, got {body:?}");
    };
    // Every distributed arm is a leaf-CQ whose relational body is a LeftJoin: the
    // outermost SELECT projection is pushed into the arms (so the Union surfaces to
    // the spine top), giving each arm a canonical `Construction` over its `LeftJoin`.
    for arm in children {
        assert_leaf_cq(arm);
        let IqNode::Construction { child, .. } = arm else {
            unreachable!("assert_leaf_cq guarantees a Construction root");
        };
        assert!(
            matches!(**child, IqNode::LeftJoin { .. }),
            "each distributed arm's body is a LeftJoin: {child:?}"
        );
    }
}

/// An `Empty` Union arm (an unmapped predicate) is pruned, keeping the surviving
/// arms — never collapsed/merged and never silently dropping the others.
#[test]
fn empty_union_arm_is_pruned() {
    let body = strip_spine(&norm(
        "SELECT * WHERE { { ?s <http://ex/name> ?n } UNION { ?s <http://ex/nope> ?o } \
         UNION { ?s <http://ex/dname> ?d } }",
    ))
    .clone();
    assert_eq!(arm_count(&body), 2, "the unmapped arm is pruned: {body:?}");
    if let IqNode::Union { children, .. } = &body {
        for arm in children {
            assert!(
                !matches!(arm, IqNode::Empty { .. }),
                "no Empty arm survives: {arm:?}"
            );
        }
    }
}

/// `rdf:type ?c` still resolves and normalizes to a leaf-CQ spine (the two class
/// atoms), exercising the class-atom path through NORMALIZE.
#[test]
fn class_atom_pattern_normalizes_to_spine() {
    let body = strip_spine(&norm(&format!("SELECT * WHERE {{ ?s <{RDF_TYPE}> ?c }}"))).clone();
    match &body {
        IqNode::Union { children, .. } => {
            for arm in children {
                assert_leaf_cq(arm);
            }
        }
        other => assert_leaf_cq(other),
    }
}
