use spargebra::algebra::{Expression, GraphPattern};
use spargebra::term::GroundTerm;

use super::*;

#[test]
fn should_accept_exact_algebra_node_limit_and_reject_the_next_node() {
    let accepted_terms = (MAX_ALGEBRA_NODES_V1 - 2) / 2;
    let accepted = values_with_named_terms(accepted_terms);
    let envelope = AlgebraEnvelopeV1::validate(&accepted).expect("exact node limit fits");
    assert_eq!(envelope.algebra_nodes, MAX_ALGEBRA_NODES_V1);

    let rejected = values_with_named_terms(accepted_terms + 1);
    assert_limit(
        AlgebraEnvelopeV1::validate(&rejected).expect_err("next node is rejected"),
        CompileEnvelopeLimit::AlgebraNodes,
        MAX_ALGEBRA_NODES_V1 + 1,
        MAX_ALGEBRA_NODES_V1,
    );
}

#[test]
fn should_accept_exact_depth_limit_and_reject_the_next_level_iteratively() {
    let accepted = select(nested_distinct(MAX_ALGEBRA_DEPTH_V1 - 2));
    let envelope = AlgebraEnvelopeV1::validate(&accepted).expect("exact depth fits");
    assert_eq!(envelope.max_depth, MAX_ALGEBRA_DEPTH_V1);

    let rejected = select(nested_distinct(MAX_ALGEBRA_DEPTH_V1 - 1));
    assert_limit(
        AlgebraEnvelopeV1::validate(&rejected).expect_err("next depth is rejected"),
        CompileEnvelopeLimit::AlgebraDepth,
        MAX_ALGEBRA_DEPTH_V1 + 1,
        MAX_ALGEBRA_DEPTH_V1,
    );
}

#[test]
fn should_accept_exact_collection_slot_limit_and_reject_the_next_slot() {
    let accepted = values_with_empty_cells(MAX_COLLECTION_SLOTS_V1 - 1);
    let envelope = AlgebraEnvelopeV1::validate(&accepted).expect("exact slot limit fits");
    assert_eq!(envelope.collection_slots, MAX_COLLECTION_SLOTS_V1);

    let rejected = values_with_empty_cells(MAX_COLLECTION_SLOTS_V1);
    assert_limit(
        AlgebraEnvelopeV1::validate(&rejected).expect_err("next slot is rejected"),
        CompileEnvelopeLimit::CollectionSlots,
        MAX_COLLECTION_SLOTS_V1 + 1,
        MAX_COLLECTION_SLOTS_V1,
    );
}

#[test]
fn should_accept_exact_payload_limit_and_reject_the_next_byte() {
    let accepted = filter_variable(MAX_RETAINED_PAYLOAD_BYTES_V1);
    let envelope = AlgebraEnvelopeV1::validate(&accepted).expect("exact payload limit fits");
    assert_eq!(
        envelope.retained_payload_bytes,
        MAX_RETAINED_PAYLOAD_BYTES_V1
    );

    let rejected = filter_variable(MAX_RETAINED_PAYLOAD_BYTES_V1 + 1);
    assert_limit(
        AlgebraEnvelopeV1::validate(&rejected).expect_err("next byte is rejected"),
        CompileEnvelopeLimit::RetainedPayloadBytes,
        MAX_RETAINED_PAYLOAD_BYTES_V1 + 1,
        MAX_RETAINED_PAYLOAD_BYTES_V1,
    );
}

#[test]
fn should_keep_typed_errors_free_of_submitted_payload() {
    let marker = "SECRET_QUERY_PAYLOAD";
    let submitted = format!("{}{marker}", "x".repeat(MAX_RETAINED_PAYLOAD_BYTES_V1));
    let rejected = filter_named_variable(&submitted);
    let error = AlgebraEnvelopeV1::validate(&rejected).expect_err("payload is rejected");

    assert!(!error.to_string().contains(marker));
    assert!(!format!("{error:?}").contains(marker));
}

fn nested_distinct(levels: usize) -> GraphPattern {
    let mut pattern = empty();
    for _ in 0..levels {
        pattern = GraphPattern::Distinct {
            inner: Box::new(pattern),
        };
    }
    pattern
}

fn values_with_named_terms(count: usize) -> Query {
    select(GraphPattern::Values {
        variables: Vec::new(),
        bindings: vec![vec![Some(GroundTerm::NamedNode(iri("x"))); count]],
    })
}

fn values_with_empty_cells(count: usize) -> Query {
    select(GraphPattern::Values {
        variables: Vec::new(),
        bindings: vec![vec![None; count]],
    })
}

fn filter_variable(bytes: usize) -> Query {
    filter_named_variable(&"x".repeat(bytes))
}

fn filter_named_variable(name: &str) -> Query {
    select(GraphPattern::Filter {
        expr: Expression::Variable(var(name)),
        inner: Box::new(empty()),
    })
}
