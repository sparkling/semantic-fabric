use std::collections::BTreeMap;

use sf_core::ir::LogicalSource;
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use sf_core::{Literal, Term};

use crate::compiler_control::CompileContext;
use crate::iq::node::{BindDef, IqNode, Var};
use crate::iq::{Scan, TermDef};
use crate::plan_measure::clone_root::{
    measure_compiler_clone_collection_v1, CompilerCloneCollectionV1,
};
use crate::{CompilerWorkMode, Error};

const BOUND_VAR: &str = "bound-variable-with-owned-payload";

struct ConstructionUnionFixture {
    tree: IqNode,
    substitution_work: u64,
    project_work: u64,
    owned_substitution_pointer: usize,
    owned_project_pointer: usize,
}

fn budget(max_compiler_work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(
        max_compiler_work,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    ))
}

fn assert_control_error(error: Error, expected: QueryControlError) {
    match error {
        Error::QueryControl(actual) => assert_eq!(actual, expected),
        other => panic!("expected query-control error, got {other:?}"),
    }
}

fn nested_substitution() -> (BTreeMap<Var, BindDef>, usize) {
    let marker = Literal::new_simple_literal(
        "the-original-substitution-literal-allocation-must-reach-the-final-arm",
    );
    let pointer = marker.value().as_ptr() as usize;
    let mut substitution = BTreeMap::new();
    substitution.insert(
        BOUND_VAR.into(),
        BindDef::Resolved(TermDef::Concat(vec![
            TermDef::Const(Term::Literal(marker)),
            TermDef::Coalesce(
                Box::new(TermDef::Const(Term::Literal(Literal::new_simple_literal(
                    "nested-left-clone-payload",
                )))),
                Box::new(TermDef::Const(Term::Literal(Literal::new_simple_literal(
                    "nested-right-clone-payload",
                )))),
            ),
        ])),
    );
    (substitution, pointer)
}

fn substitution_payload_pointer(substitution: &BTreeMap<Var, BindDef>) -> usize {
    let Some(BindDef::Resolved(TermDef::Concat(parts))) = substitution.get(BOUND_VAR) else {
        panic!("expected the nested resolved substitution: {substitution:?}")
    };
    let Some(TermDef::Const(Term::Literal(marker))) = parts.first() else {
        panic!("expected the owned marker literal first: {parts:?}")
    };
    marker.value().as_ptr() as usize
}

fn projected_variables() -> Vec<Var> {
    vec![
        BOUND_VAR.into(),
        "second-projected-variable-with-owned-payload".into(),
    ]
}

fn scan_leaf(alias: usize, source: &str) -> IqNode {
    IqNode::Extensional {
        scan: Scan {
            alias,
            source: (LogicalSource::Query(source.to_owned())).into(),
        },
        bind: BTreeMap::new(),
    }
}

fn scan_alias(node: &IqNode) -> usize {
    let IqNode::Extensional { scan, .. } = node else {
        panic!("expected an extensional arm, got {node:?}")
    };
    scan.alias
}

fn construction_union_fixture() -> ConstructionUnionFixture {
    let (substitution, owned_substitution_pointer) = nested_substitution();
    let project = projected_variables();
    let owned_project_pointer = project[0].as_ptr() as usize;
    let substitution_work = measure_compiler_clone_collection_v1(
        CompilerCloneCollectionV1::IqSubstitution(&substitution),
    )
    .unwrap()
    .deep_clone_work;
    let project_work =
        measure_compiler_clone_collection_v1(CompilerCloneCollectionV1::Variables(&project))
            .unwrap()
            .deep_clone_work;

    assert!(substitution_work > 0);
    assert!(project_work > 0);

    ConstructionUnionFixture {
        tree: IqNode::Construction {
            child: Box::new(IqNode::Union {
                children: vec![
                    scan_leaf(3, "first-construction-union-arm"),
                    scan_leaf(1, "middle-construction-union-arm"),
                    scan_leaf(3, "duplicate-construction-union-arm"),
                ],
                project: Vec::new(),
            }),
            subst: substitution,
            project,
        },
        substitution_work,
        project_work,
        owned_substitution_pointer,
        owned_project_pointer,
    }
}

#[test]
fn exact_iq_substitution_clone_accepts_n_and_rejects_n_minus_one() {
    let (source, source_payload) = nested_substitution();
    let measure =
        measure_compiler_clone_collection_v1(CompilerCloneCollectionV1::IqSubstitution(&source))
            .unwrap();
    let exact = budget(measure.deep_clone_work);

    let cloned = CompileContext::new(&exact)
        .clone_iq_substitution(&source)
        .unwrap();

    assert_eq!(format!("{cloned:?}"), format!("{source:?}"));
    assert_ne!(substitution_payload_pointer(&cloned), source_payload);
    assert_eq!(
        exact.consumed(QueryCharge::CompilerWork),
        measure.deep_clone_work
    );

    let short = budget(measure.deep_clone_work - 1);
    assert_control_error(
        CompileContext::new(&short)
            .clone_iq_substitution(&source)
            .expect_err("N-1 must reject before the substitution clone"),
        QueryControlError::CompilerWorkExceeded,
    );
    assert_eq!(substitution_payload_pointer(&source), source_payload);
    assert_eq!(short.consumed(QueryCharge::CompilerWork), 0);
    assert_eq!(
        short.checkpoint(),
        Err(QueryControlError::CompilerWorkExceeded)
    );
}

#[test]
fn exact_variable_collection_clone_accepts_n_and_rejects_n_minus_one() {
    let source = projected_variables();
    let source_allocation = source.as_ptr();
    let source_payload = source[0].as_ptr();
    let measure =
        measure_compiler_clone_collection_v1(CompilerCloneCollectionV1::Variables(&source))
            .unwrap();
    let exact = budget(measure.deep_clone_work);

    let cloned = CompileContext::new(&exact)
        .clone_variables(&source)
        .unwrap();

    assert_eq!(cloned, source);
    assert_ne!(cloned.as_ptr(), source_allocation);
    assert_ne!(cloned[0].as_ptr(), source_payload);
    assert_eq!(
        exact.consumed(QueryCharge::CompilerWork),
        measure.deep_clone_work
    );

    let short = budget(measure.deep_clone_work - 1);
    assert_control_error(
        CompileContext::new(&short)
            .clone_variables(&source)
            .expect_err("N-1 must reject before the variable collection clone"),
        QueryControlError::CompilerWorkExceeded,
    );
    assert_eq!(source.as_ptr(), source_allocation);
    assert_eq!(source[0].as_ptr(), source_payload);
    assert_eq!(short.consumed(QueryCharge::CompilerWork), 0);
    assert_eq!(
        short.checkpoint(),
        Err(QueryControlError::CompilerWorkExceeded)
    );
}

#[test]
fn construction_over_three_arm_union_charges_exact_schedule_and_preserves_owners() {
    let fixture = construction_union_fixture();
    let expected = 2 * fixture.substitution_work + 3 * fixture.project_work;
    let control = budget(expected);
    let raw = crate::iq::normalize::normalize(fixture.tree.clone()).unwrap();

    let metered = crate::iq::normalize::normalize_with_work_mode(
        fixture.tree,
        CompilerWorkMode::Metered(CompileContext::new(&control)),
    )
    .unwrap();

    assert_eq!(format!("{metered:?}"), format!("{raw:?}"));
    assert_eq!(control.consumed(QueryCharge::CompilerWork), expected);
    let IqNode::Union { children, project } = metered else {
        panic!("the lifted Construction must expose the Union")
    };
    assert_eq!(children.len(), 3, "duplicate arms retain multiplicity");
    assert_eq!(project[0].as_ptr() as usize, fixture.owned_project_pointer);

    for (index, (arm, expected_alias)) in children.iter().zip([3, 1, 3]).enumerate() {
        let IqNode::Construction {
            child,
            subst,
            project: arm_project,
        } = arm
        else {
            panic!("expected a Construction on every Union arm: {arm:?}")
        };
        assert_eq!(scan_alias(child), expected_alias);
        assert_eq!(arm_project, &project);
        assert_ne!(
            arm_project[0].as_ptr() as usize,
            fixture.owned_project_pointer,
            "every arm receives an independent projection clone"
        );
        assert_eq!(
            substitution_payload_pointer(subst) == fixture.owned_substitution_pointer,
            index == 2,
            "only the final arm receives the original substitution"
        );
    }
}

fn assert_construction_union_rejection(max_work: u64, expected_consumed: u64) {
    let fixture = construction_union_fixture();
    let control = budget(max_work);

    assert_control_error(
        crate::iq::normalize::normalize_with_work_mode(
            fixture.tree,
            CompilerWorkMode::Metered(CompileContext::new(&control)),
        )
        .expect_err("the selected exact clone boundary must reject"),
        QueryControlError::CompilerWorkExceeded,
    );
    assert_eq!(
        control.consumed(QueryCharge::CompilerWork),
        expected_consumed
    );
    assert_eq!(
        control.checkpoint(),
        Err(QueryControlError::CompilerWorkExceeded)
    );
}

#[test]
fn construction_union_rejections_pin_substitution_then_projection_operation_order() {
    let fixture = construction_union_fixture();
    let substitution = fixture.substitution_work;
    let project = fixture.project_work;

    assert_construction_union_rejection(substitution - 1, 0);
    assert_construction_union_rejection(substitution + project - 1, substitution);
    assert_construction_union_rejection(2 * substitution + project - 1, substitution + project);
    assert_construction_union_rejection(
        2 * (substitution + project) - 1,
        2 * substitution + project,
    );
    assert_construction_union_rejection(
        2 * (substitution + project) + project - 1,
        2 * (substitution + project),
    );
}
