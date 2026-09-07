use oxrdf::{BlankNode, GraphName, Literal, NamedNode, Term, Triple};
use sf_core::query_control::{
    QueryBudget, QueryLimits, ReservationError, ReservationLimits, ReservationShape,
};

use super::mapping::{compatible, compatible_bag_pairs, minus_matches, SemanticMapping};
use super::semantic_key::{
    checked_len_add, equal_after_hash_by, BlankNodeScope, ScopedTerm, SemanticKeyCaps,
    SemanticKeyError, SemanticSolutionKeyV1, WriterPassMismatch,
};

fn budget(max_retained_bytes: u64) -> QueryBudget {
    QueryBudget::new(
        QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX).with_reservation_limits(
            ReservationLimits::new(max_retained_bytes, u64::MAX, u64::MAX, u64::MAX, u64::MAX),
        ),
    )
}

fn default_scope() -> BlankNodeScope {
    BlankNodeScope::request("request-1", GraphName::DefaultGraph)
}

fn scoped(term: Term) -> ScopedTerm {
    ScopedTerm::new(term, default_scope())
}

fn iri(value: &str) -> ScopedTerm {
    scoped(Term::NamedNode(NamedNode::new_unchecked(value)))
}

fn plain(value: &str) -> ScopedTerm {
    scoped(Term::Literal(Literal::new_simple_literal(value)))
}

fn typed(value: &str, datatype: &str) -> ScopedTerm {
    scoped(Term::Literal(Literal::new_typed_literal(
        value,
        NamedNode::new_unchecked(datatype),
    )))
}

fn language(value: &str, tag: &str) -> ScopedTerm {
    scoped(Term::Literal(
        Literal::new_language_tagged_literal(value, tag).unwrap(),
    ))
}

fn blank(identifier: &str, scope: BlankNodeScope) -> ScopedTerm {
    ScopedTerm::new(Term::BlankNode(BlankNode::new_unchecked(identifier)), scope)
}

fn mapping(bindings: Vec<(&str, Option<ScopedTerm>)>) -> SemanticMapping {
    SemanticMapping::new(
        bindings
            .into_iter()
            .map(|(variable, value)| (Box::<str>::from(variable), value))
            .collect(),
    )
    .unwrap()
}

fn key(schema: &[&str], mapping: &SemanticMapping, budget: &QueryBudget) -> SemanticSolutionKeyV1 {
    SemanticSolutionKeyV1::encode(schema, mapping, budget, SemanticKeyCaps::default()).unwrap()
}

fn digest_hex(key: &SemanticSolutionKeyV1) -> String {
    key.digest()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn semantic_key_v1_has_a_pinned_known_answer_and_holds_its_reservation() {
    let budget = budget(1_024);
    let mapping = mapping(vec![
        ("s", Some(iri("https://example.test/s"))),
        ("label", Some(language("Colour", "EN-gb"))),
        ("missing", None),
    ]);

    let key = key(&["s", "label", "missing"], &mapping, &budget);
    assert_eq!(&key.bytes()[..5], b"SFSK\x01");
    assert_eq!(
        digest_hex(&key),
        "1da68119b69c912a0b6c7271d1fabc7086a3f2102135e9bd7127c46679be1bf7"
    );
    assert_eq!(
        budget.reserved().retained_bytes(),
        u64::try_from(key.bytes().len()).unwrap()
    );
    drop(key);
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
}

#[test]
fn key_distinguishes_term_kind_datatype_language_and_bound_domain() {
    let budget = budget(64 * 1024);
    let string_datatype = "http://www.w3.org/2001/XMLSchema#string";
    let integer_datatype = "http://www.w3.org/2001/XMLSchema#integer";

    let iri_key = key(
        &["x"],
        &mapping(vec![("x", Some(iri("https://example.test/value")))]),
        &budget,
    );
    let literal_key = key(
        &["x"],
        &mapping(vec![("x", Some(plain("https://example.test/value")))]),
        &budget,
    );
    assert_ne!(iri_key.bytes(), literal_key.bytes());

    let string_key = key(
        &["x"],
        &mapping(vec![("x", Some(typed("1", string_datatype)))]),
        &budget,
    );
    let integer_key = key(
        &["x"],
        &mapping(vec![("x", Some(typed("1", integer_datatype)))]),
        &budget,
    );
    assert_ne!(string_key.bytes(), integer_key.bytes());

    let english_key = key(
        &["x"],
        &mapping(vec![("x", Some(language("colour", "en")))]),
        &budget,
    );
    let french_key = key(
        &["x"],
        &mapping(vec![("x", Some(language("colour", "fr")))]),
        &budget,
    );
    assert_ne!(english_key.bytes(), french_key.bytes());

    let left_bound = key(
        &["x", "y"],
        &mapping(vec![("x", Some(plain("v"))), ("y", None)]),
        &budget,
    );
    let right_bound = key(
        &["x", "y"],
        &mapping(vec![("x", None), ("y", Some(plain("v")))]),
        &budget,
    );
    assert_ne!(left_bound.bytes(), right_bound.bytes());
}

#[test]
fn language_tags_are_ascii_case_normalized() {
    let budget = budget(4_096);
    let upper = key(
        &["x"],
        &mapping(vec![("x", Some(language("colour", "EN-GB")))]),
        &budget,
    );
    let lower = key(
        &["x"],
        &mapping(vec![("x", Some(language("colour", "en-gb")))]),
        &budget,
    );
    assert_eq!(upper, lower);
}

#[test]
fn graph_and_request_or_dataset_scope_are_part_of_blank_identity() {
    let budget = budget(8_192);
    let default = key(
        &["x"],
        &mapping(vec![(
            "x",
            Some(blank(
                "b1",
                BlankNodeScope::request("request-1", GraphName::DefaultGraph),
            )),
        )]),
        &budget,
    );
    let named = key(
        &["x"],
        &mapping(vec![(
            "x",
            Some(blank(
                "b1",
                BlankNodeScope::request(
                    "request-1",
                    GraphName::NamedNode(NamedNode::new_unchecked("https://example.test/g")),
                ),
            )),
        )]),
        &budget,
    );
    let dataset = key(
        &["x"],
        &mapping(vec![(
            "x",
            Some(blank(
                "b1",
                BlankNodeScope::dataset("dataset-1", GraphName::DefaultGraph),
            )),
        )]),
        &budget,
    );

    assert_ne!(default.bytes(), named.bytes());
    assert_ne!(default.bytes(), dataset.bytes());

    let default_mapping = mapping(vec![(
        "x",
        Some(blank(
            "b1",
            BlankNodeScope::request("request-1", GraphName::DefaultGraph),
        )),
    )]);
    let named_mapping = mapping(vec![(
        "x",
        Some(blank(
            "b1",
            BlankNodeScope::request(
                "request-1",
                GraphName::NamedNode(NamedNode::new_unchecked("https://example.test/g")),
            ),
        )),
    )]);
    assert!(!compatible(&default_mapping, &named_mapping));
}

fn quoted(object: Term) -> ScopedTerm {
    scoped(Term::Triple(Box::new(Triple::new(
        NamedNode::new_unchecked("https://example.test/s"),
        NamedNode::new_unchecked("https://example.test/p"),
        object,
    ))))
}

#[test]
fn rdf_star_structure_is_recursively_encoded() {
    let budget = budget(8_192);
    let shallow = key(
        &["x"],
        &mapping(vec![(
            "x",
            Some(quoted(Term::NamedNode(NamedNode::new_unchecked(
                "https://example.test/o",
            )))),
        )]),
        &budget,
    );
    let nested = key(
        &["x"],
        &mapping(vec![(
            "x",
            Some(quoted(
                quoted(Term::NamedNode(NamedNode::new_unchecked(
                    "https://example.test/o",
                )))
                .term()
                .clone(),
            )),
        )]),
        &budget,
    );
    assert_ne!(shallow.bytes(), nested.bytes());
}

#[test]
fn explicit_schema_order_is_bound_but_mapping_storage_order_is_not() {
    let budget = budget(8_192);
    let first = mapping(vec![("x", Some(plain("one"))), ("y", Some(plain("two")))]);
    let reversed_storage = mapping(vec![("y", Some(plain("two"))), ("x", Some(plain("one")))]);

    let canonical = key(&["x", "y"], &first, &budget);
    let same_schema = key(&["x", "y"], &reversed_storage, &budget);
    let permuted_schema = key(&["y", "x"], &first, &budget);
    assert_eq!(canonical, same_schema);
    assert_ne!(canonical.bytes(), permuted_schema.bytes());
}

#[test]
fn compatibility_and_minus_obey_unbound_and_shared_domain_laws() {
    let both_unbound_left = mapping(vec![("x", None)]);
    let both_unbound_right = mapping(vec![("x", None)]);
    assert!(compatible(&both_unbound_left, &both_unbound_right));
    assert!(!minus_matches(&both_unbound_left, &both_unbound_right));

    let one_bound = mapping(vec![("x", Some(plain("v")))]);
    assert!(compatible(&one_bound, &both_unbound_right));
    assert!(!minus_matches(&one_bound, &both_unbound_right));

    let disjoint = mapping(vec![("y", Some(plain("v")))]);
    assert!(compatible(&one_bound, &disjoint));
    assert!(!minus_matches(&one_bound, &disjoint));

    let equal = mapping(vec![("x", Some(plain("v")))]);
    let unequal = mapping(vec![("x", Some(plain("other")))]);
    assert!(compatible(&one_bound, &equal));
    assert!(minus_matches(&one_bound, &equal));
    assert!(!compatible(&one_bound, &unequal));
    assert!(!minus_matches(&one_bound, &unequal));
}

#[test]
fn injected_hash_collision_still_requires_complete_semantic_equality() {
    let budget = budget(4_096);
    let left = key(&["x"], &mapping(vec![("x", Some(plain("left")))]), &budget);
    let right = key(&["x"], &mapping(vec![("x", Some(plain("right")))]), &budget);

    assert_eq!(left.digest().len(), right.digest().len());
    assert!(!equal_after_hash_by(&left, &right, |_| 0u8));
    assert!(equal_after_hash_by(&left, &left, |_| 0u8));
}

#[test]
fn compatible_bag_pairing_preserves_exact_n_by_m_multiplicity() {
    let left: Vec<_> = (0..3)
        .map(|_| mapping(vec![("x", Some(plain("same")))]))
        .collect();
    let mut right: Vec<_> = (0..4)
        .map(|_| mapping(vec![("x", Some(plain("same")))]))
        .collect();
    right.push(mapping(vec![("x", Some(plain("different")))]));

    let pairs: Vec<_> = compatible_bag_pairs(&left, &right).collect();
    assert_eq!(pairs.len(), 3 * 4);
    assert_eq!(pairs.iter().filter(|(_, right)| *right == 4).count(), 0);
    for left_index in 0..3 {
        assert_eq!(
            pairs
                .iter()
                .filter(|(candidate, _)| *candidate == left_index)
                .count(),
            4
        );
    }
}

#[test]
fn malformed_schema_caps_and_lengths_fail_closed() {
    assert_eq!(
        SemanticMapping::new(vec![
            (Box::<str>::from("x"), None),
            (Box::<str>::from("x"), None),
        ])
        .unwrap_err(),
        SemanticKeyError::DuplicateVariable
    );
    assert_eq!(
        SemanticKeyCaps::new(1, 1, 65).unwrap_err(),
        SemanticKeyError::InvalidCaps
    );
    assert_eq!(
        checked_len_add(usize::MAX, 1).unwrap_err(),
        SemanticKeyError::LengthOverflow
    );

    let budget = budget(4_096);
    let valid_mapping = mapping(vec![("x", Some(plain("value")))]);
    assert_eq!(
        SemanticSolutionKeyV1::encode(
            &["x", "missing"],
            &valid_mapping,
            &budget,
            SemanticKeyCaps::default(),
        )
        .unwrap_err(),
        SemanticKeyError::SchemaMismatch
    );
    assert_eq!(budget.reserved(), ReservationShape::ZERO);

    let malformed_blank = mapping(vec![(
        "x",
        Some(blank(
            "b1",
            BlankNodeScope::request("", GraphName::DefaultGraph),
        )),
    )]);
    assert_eq!(
        SemanticSolutionKeyV1::encode(
            &["x"],
            &malformed_blank,
            &budget,
            SemanticKeyCaps::default(),
        )
        .unwrap_err(),
        SemanticKeyError::MalformedBlankScope
    );
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
}

fn nested_term(depth: usize) -> ScopedTerm {
    let mut term = Term::NamedNode(NamedNode::new_unchecked("https://example.test/end"));
    for _ in 0..depth {
        term = quoted(term).term().clone();
    }
    scoped(term)
}

#[test]
fn byte_and_recursion_caps_reject_before_any_reservation_survives() {
    let budget = budget(4_096);
    let oversized = mapping(vec![("x", Some(plain("a value beyond a tiny cap")))]);
    let tiny = SemanticKeyCaps::new(8, 1, 1).unwrap();
    assert_eq!(
        SemanticSolutionKeyV1::encode(&["x"], &oversized, &budget, tiny).unwrap_err(),
        SemanticKeyError::EncodedBytesExceeded
    );
    assert_eq!(budget.reserved(), ReservationShape::ZERO);

    let nested = mapping(vec![("x", Some(nested_term(2)))]);
    let one_level = SemanticKeyCaps::new(4_096, 1, 1).unwrap();
    assert_eq!(
        SemanticSolutionKeyV1::encode(&["x"], &nested, &budget, one_level).unwrap_err(),
        SemanticKeyError::TripleDepthExceeded
    );
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
}

#[test]
fn reservation_rejection_returns_no_partial_key_or_capacity() {
    let budget = budget(1);
    let mapping = mapping(vec![("x", Some(plain("value")))]);
    assert_eq!(
        SemanticSolutionKeyV1::encode(&["x"], &mapping, &budget, SemanticKeyCaps::default(),)
            .unwrap_err(),
        SemanticKeyError::Reservation(ReservationError::RetainedBytesExceeded)
    );
    assert_eq!(budget.reserved(), ReservationShape::ZERO);
}

#[test]
fn writer_measurement_mismatch_returns_no_key_and_releases_reservation() {
    let budget = budget(4_096);
    let mapping = mapping(vec![("x", Some(plain("value")))]);

    for mismatch in [
        WriterPassMismatch::FewerBytes,
        WriterPassMismatch::MoreBytes,
    ] {
        let error = SemanticSolutionKeyV1::encode_with_writer_mismatch(
            &["x"],
            &mapping,
            &budget,
            SemanticKeyCaps::default(),
            mismatch,
        )
        .unwrap_err();
        assert_eq!(error, SemanticKeyError::WriterMeasurementMismatch);
        assert_eq!(
            error.to_string(),
            "semantic-key writer does not match its measured length"
        );
        assert_eq!(budget.reserved(), ReservationShape::ZERO);
    }
}
