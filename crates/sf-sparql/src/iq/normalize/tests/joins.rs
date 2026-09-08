use super::*;

// ---- synthetic R2 test: shared var UNDEF in one union arm = free dimension -----

fn col_def(col: &str, alias: usize) -> BindDef {
    BindDef::Resolved(TermDef::Derived {
        term_map: TermMap::Column(col.into(), TermSpec::plain_literal()),
        alias,
    })
}

/// An arm `Construction` binding the given `(var, column, alias)` triples over a
/// bare Extensional scan.
fn synth_arm(binds: &[(&str, &str, usize)], scan_alias: usize) -> IqNode {
    let mut subst = BTreeMap::new();
    let mut project: Vec<Var> = Vec::new();
    for (v, col, a) in binds {
        subst.insert((*v).into(), col_def(col, *a));
        project.push((*v).into());
    }
    IqNode::Construction {
        child: Box::new(IqNode::Extensional {
            scan: Scan {
                alias: scan_alias,
                source: (LogicalSource::Table("t".to_owned())).into(),
            },
            bind: BTreeMap::new(),
        }),
        subst,
        project,
    }
}

/// R2 (=_bag-CRITICAL): when an InnerJoin distributes into a Union whose arms bind
/// a shared variable differently, the arm that BINDS the shared var seeds an
/// equality; the arm that leaves it UNDEF/absent degenerates to a free dimension
/// (NO equality, rebound from the bound side) — exactly the flat `merge`
/// (`unfold.rs:1197-1199`).
#[test]
fn shared_var_undef_in_union_arm_is_a_free_dimension() {
    // A binds ?x,?y on alias 0; B1 binds ?x,?z on alias 1 (?x SHARED with A);
    // B2 binds only ?z on alias 2 (?x ABSENT — UNDEF in this arm).
    let a = synth_arm(&[("x", "x", 0), ("y", "y", 0)], 0);
    let b1 = synth_arm(&[("x", "x", 1), ("z", "z", 1)], 1);
    let b2 = synth_arm(&[("z", "z", 2)], 2);
    let tree = IqNode::InnerJoin {
        children: vec![
            a,
            IqNode::Union {
                children: vec![b1, b2],
                project: vec!["x".into(), "z".into()],
            },
        ],
        cond: vec![],
    };
    let out = normalize(tree).unwrap();
    let IqNode::Union { children, .. } = &out else {
        panic!("the join over a Union must distribute, got {out:?}");
    };
    assert_eq!(children.len(), 2, "two distributed arms: {out:?}");

    // Count how many distributed arms carry a shared-var equality (IqCond::Sql).
    let arms_with_eq = children
        .iter()
        .filter(|arm| {
            let IqNode::Construction { child, .. } = arm else {
                return false;
            };
            matches!(&**child, IqNode::InnerJoin { cond, .. }
                if cond.iter().any(|c| matches!(c, IqCond::Sql(_))))
        })
        .count();
    assert_eq!(
        arms_with_eq, 1,
        "exactly the ?x-binding arm seeds an equality; the UNDEF arm is a free \
         dimension with NO equality: {out:?}"
    );

    // The free-dimension arm still binds ?x (rebound from the bound side A) and
    // its InnerJoin carries no equality condition.
    let free = children
        .iter()
        .find(|arm| {
            let IqNode::Construction { child, .. } = arm else {
                return false;
            };
            matches!(&**child, IqNode::InnerJoin { cond, .. } if cond.is_empty())
        })
        .expect("a free-dimension arm with an empty join cond");
    let IqNode::Construction { subst, .. } = free else {
        unreachable!()
    };
    assert!(
        subst.contains_key("x"),
        "?x is rebound from the bound operand in the free-dimension arm: {subst:?}"
    );
}

/// §4.13 (refute-fix): an `InnerJoin` whose only data children are `True` (the empty
/// tuple) but which carries a residual `cond` is NOT the condition-free identity —
/// the cond MUST survive (here a ground `IqCond::Sql`), preserved as a `Filter` over
/// `True`, never silently collapsed to `True`.
#[test]
fn all_true_join_with_residual_cond_keeps_the_cond() {
    use crate::iq::{ColRef, SqlCond};
    let cond = vec![IqCond::Sql(SqlCond::IsNotNull(ColRef::new(0, "c")))];
    let tree = IqNode::InnerJoin {
        children: vec![IqNode::True, IqNode::True],
        cond: cond.clone(),
    };
    let out = normalize(tree).unwrap();
    match out {
        IqNode::Filter { child, cond: kept } => {
            assert!(matches!(*child, IqNode::True), "cond preserved over True");
            assert_eq!(kept.len(), 1, "the residual cond is not dropped: {kept:?}");
        }
        other => panic!(
            "a residual cond over only-True children must NOT collapse to True; got {other:?}"
        ),
    }
}

/// A disjoint shared-variable unification prunes the join to `Empty` (the flat
/// `merge` `Unify::Empty ⇒ None`), and the surviving Union arms remain — never a
/// silent row drop.
#[test]
fn disjoint_shared_var_prunes_the_arm() {
    // A binds ?x to an IRI-template; B binds ?x to a literal column ⇒ disjoint.
    let a = synth_arm(&[("x", "x", 0)], 0);
    let b = IqNode::Construction {
        child: Box::new(IqNode::Extensional {
            scan: Scan {
                alias: 1,
                source: (LogicalSource::Table("t".to_owned())).into(),
            },
            bind: BTreeMap::new(),
        }),
        subst: {
            let mut m = BTreeMap::new();
            m.insert(
                "x".into(),
                BindDef::Resolved(TermDef::Derived {
                    term_map: template_iri("http://ex/{x}"),
                    alias: 1,
                }),
            );
            m
        },
        project: vec!["x".into()],
    };
    // ?x is a plain literal column on the left and an IRI template on the right →
    // unify proves disjoint (IRI can never equal a literal).
    let tree = IqNode::InnerJoin {
        children: vec![a, b],
        cond: vec![],
    };
    let out = normalize(tree).unwrap();
    assert!(
        matches!(out, IqNode::Empty { .. }),
        "a disjoint shared variable prunes the join to Empty: {out:?}"
    );
}

/// A `Union` is in `under_join` position iff it is a direct operand of an
/// `InnerJoin`/`LeftJoin` (i.e. trapped, not lifted to the spine top).
fn union_trapped(n: &IqNode, under_join: bool) -> bool {
    match n {
        IqNode::Union { children, .. } => {
            under_join || children.iter().any(|c| union_trapped(c, false))
        }
        IqNode::InnerJoin { children, .. } => children.iter().any(|c| union_trapped(c, true)),
        IqNode::Construction { child, .. }
        | IqNode::Distinct { child }
        | IqNode::Slice { child, .. }
        | IqNode::OrderBy { child, .. }
        | IqNode::Aggregation { child, .. }
        | IqNode::Filter { child, .. } => union_trapped(child, under_join),
        IqNode::LeftJoin { left, right, .. } => {
            union_trapped(left, true) || union_trapped(right, true)
        }
        _ => false,
    }
}

/// =_bag-CRITICAL (R2 / spine): a pure-projection subquery over a multi-arm `Union`
/// (`{ SELECT ?s { ?s ?p ?o } }`) joined with another pattern MUST distribute — the
/// `Union` is lifted to the spine top, never left trapped as an opaque
/// `Construction{empty subst, Union}` inside the `InnerJoin` body (which would never
/// cross-product the union arms with the join's other operand).
#[test]
fn projection_over_union_does_not_trap_union_under_join() {
    let out = norm("SELECT * WHERE { { SELECT ?s WHERE { ?s ?p ?o } } ?s <http://ex/name> ?n }");
    assert!(
        !union_trapped(&out, false),
        "a projection over a Union must distribute to the spine top, not trap the \
         Union inside the join body: {out:?}"
    );
    let body = strip_spine(&out).clone();
    let IqNode::Union { children, .. } = &body else {
        panic!("the join over the subquery Union must distribute to a Union, got {body:?}");
    };
    for arm in children {
        assert_leaf_cq(arm);
    }
}
