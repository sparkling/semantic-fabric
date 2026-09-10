use super::*;
use sf_core::query_control::{
    QueryBudget, QueryCharge, QueryControl, QueryControlError, QueryLimits,
};
use spargebra::algebra::{Expression, OrderExpression};

fn budget(work: u64) -> QueryBudget {
    QueryBudget::new(QueryLimits::new(work, u64::MAX, u64::MAX, u64::MAX))
}

fn validator(control: &dyn QueryControl) -> Validator<'_, '_> {
    Validator {
        stack: Vec::new(),
        requested_capacity: 0,
        envelope: AlgebraEnvelopeV1::default(),
        control: Some(control),
    }
}

#[test]
fn stack_allocation_and_relocation_are_paid_before_use() {
    let query = select(empty());
    let frame_bytes = std::mem::size_of::<Frame<'_>>() as u64;
    let short = budget(frame_bytes);
    let mut walk = validator(&short);
    assert!(matches!(
        walk.push(1, Work::Query(&query)),
        Err(WalkError::Control(QueryControlError::CompilerWorkExceeded))
    ));
    assert_eq!(short.consumed(QueryCharge::CompilerWork), 1);
    assert_eq!(walk.stack.capacity(), 0);
    assert!(walk.stack.is_empty());

    let paid = budget(3 + 10 * frame_bytes);
    let mut walk = validator(&paid);
    // Actual excess capacity cannot lower the logical charge schedule.
    walk.stack.try_reserve_exact(128).unwrap();
    for expected in [1 + frame_bytes, 2 + 4 * frame_bytes, 3 + 10 * frame_bytes] {
        walk.push(1, Work::Query(&query)).unwrap();
        assert_eq!(paid.consumed(QueryCharge::CompilerWork), expected);
    }
    assert_eq!(walk.requested_capacity, 4);
    assert_eq!(walk.stack.len(), 3);
}

#[test]
fn unbound_values_slots_are_prepaid_even_without_child_frames() {
    let query = select(GraphPattern::Values {
        variables: vec![],
        bindings: vec![vec![None; 100]],
    });
    // Two nodes, one stack slot, one row, one hundred unbound cells.
    let exact = 2 + std::mem::size_of::<Frame<'_>>() as u64 + 1 + 100;
    let paid = budget(exact);
    assert_eq!(
        AlgebraEnvelopeV1::validate_with_control(&query, &paid).unwrap(),
        AlgebraEnvelopeV1::validate(&query).unwrap()
    );
    assert_eq!(paid.consumed(QueryCharge::CompilerWork), exact);
    let short = budget(exact - 1);
    assert_eq!(
        AlgebraEnvelopeV1::validate_with_control(&query, &short),
        Err(QueryControlError::CompilerWorkExceeded)
    );
    assert_eq!(short.consumed(QueryCharge::CompilerWork), exact - 100);
}

struct CancelOnPayment(QueryBudget);
impl QueryControl for CancelOnPayment {
    fn checkpoint(&self) -> Result<(), QueryControlError> {
        self.0.checkpoint()
    }
    fn consume(&self, charge: QueryCharge, amount: u64) -> Result<(), QueryControlError> {
        self.0.consume(charge, amount)?;
        if amount > 0 {
            self.0.terminate(QueryControlError::Cancelled);
        }
        Ok(())
    }
    fn terminate(&self, cause: QueryControlError) -> QueryControlError {
        self.0.terminate(cause)
    }
}

#[test]
fn cancellation_at_payment_prevents_stack_and_collection_mutation() {
    let query = select(empty());
    let control = CancelOnPayment(budget(u64::MAX));
    let mut walk = validator(&control);
    assert!(matches!(
        walk.push(1, Work::Query(&query)),
        Err(WalkError::Control(QueryControlError::Cancelled))
    ));
    assert_eq!(walk.stack.capacity(), 0);
    assert!(walk.stack.is_empty());
    assert_eq!(walk.envelope, AlgebraEnvelopeV1::default());

    let control = CancelOnPayment(budget(u64::MAX));
    let mut walk = validator(&control);
    assert!(matches!(
        walk.collection(100),
        Err(WalkError::Control(QueryControlError::Cancelled))
    ));
    assert_eq!(walk.envelope.collection_slots, 0);
    let control = CancelOnPayment(budget(u64::MAX));
    let mut walk = validator(&control);
    assert!(matches!(
        walk.payload(100),
        Err(WalkError::Control(QueryControlError::Cancelled))
    ));
    assert_eq!(walk.envelope.retained_payload_bytes, 0);
}

fn duplicate_projection() -> Query {
    select(GraphPattern::Project {
        inner: Box::new(GraphPattern::Extend {
            inner: Box::new(empty()),
            variable: var("λ"),
            expression: Expression::Variable(var("x")),
        }),
        variables: vec![var("λ"), var("λ")],
    })
}

#[test]
fn formatter_charge_uses_actual_extends_slots_and_utf8_variable_bytes() {
    let query = duplicate_projection();
    let envelope = AlgebraEnvelopeV1::validate(&query).unwrap();
    assert_eq!(
        (
            envelope.algebra_nodes,
            envelope.collection_slots,
            envelope.extend_nodes,
            envelope.project_slots,
            envelope.max_variable_bytes
        ),
        (9, 2, 1, 2, 2)
    );
    // N + E*(3Q+N+S)*(M+1) + Q*projection-entry bytes.
    let exact =
        9 + (6 + 9 + 2) * 3 + 2 * std::mem::size_of::<(&Variable, Option<&Expression>)>() as u64;
    let short = budget(exact - 1);
    assert_eq!(
        envelope.charge_canonical_preparation(&short),
        Err(QueryControlError::CompilerWorkExceeded)
    );
    assert_eq!(short.consumed(QueryCharge::CompilerWork), 0);
    let paid = budget(exact);
    envelope.charge_canonical_preparation(&paid).unwrap();
    assert_eq!(paid.consumed(QueryCharge::CompilerWork), exact);
    let cancelled = CancelOnPayment(budget(exact));
    assert_eq!(
        envelope.charge_canonical_preparation(&cancelled),
        Err(QueryControlError::Cancelled)
    );
}

#[test]
fn preparation_overflow_is_typed_sticky_and_redacted() {
    let envelope = AlgebraEnvelopeV1 {
        project_slots: usize::MAX,
        ..Default::default()
    };
    let control = budget(u64::MAX);
    assert_eq!(
        envelope.charge_canonical_preparation(&control),
        Err(QueryControlError::AccountingOverflow)
    );
    assert_eq!(
        control.checkpoint(),
        Err(QueryControlError::AccountingOverflow)
    );
    assert_eq!(control.consumed(QueryCharge::CompilerWork), 0);
}

#[test]
fn controlled_envelope_preserves_nested_exists_modifiers_and_empty_projects() {
    let graph = GraphPattern::Group {
        inner: Box::new(GraphPattern::OrderBy {
            inner: Box::new(GraphPattern::Values {
                variables: vec![var("x")],
                bindings: vec![vec![None]],
            }),
            expression: vec![OrderExpression::Asc(Expression::Variable(var("x")))],
        }),
        variables: vec![var("x")],
        aggregates: vec![],
    };
    let branch = GraphPattern::Project {
        inner: Box::new(GraphPattern::Extend {
            inner: Box::new(empty()),
            variable: var("y"),
            expression: Expression::Exists(Box::new(graph)),
        }),
        variables: vec![var("y")],
    };
    let query = select(GraphPattern::Project {
        variables: vec![],
        inner: Box::new(GraphPattern::Project {
            variables: vec![],
            inner: Box::new(GraphPattern::Join {
                left: Box::new(GraphPattern::Distinct {
                    inner: Box::new(branch.clone()),
                }),
                right: Box::new(GraphPattern::Slice {
                    inner: Box::new(branch),
                    start: 1,
                    length: Some(2),
                }),
            }),
        }),
    });
    let raw = AlgebraEnvelopeV1::validate(&query).unwrap();
    assert_eq!((raw.extend_nodes, raw.project_slots), (2, 2));
    let paid = budget(u64::MAX);
    assert_eq!(
        AlgebraEnvelopeV1::validate_with_control(&query, &paid).unwrap(),
        raw
    );
    raw.charge_canonical_preparation(&paid).unwrap();
    assert!(query.to_string().contains("EXISTS"));
}
