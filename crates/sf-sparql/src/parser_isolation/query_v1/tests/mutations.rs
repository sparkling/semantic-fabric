use super::*;

#[test]
fn canonical_header_fields_and_lengths_are_closed() {
    let wire = canonical_wire();

    let mut bad_magic = wire.clone();
    bad_magic[0] ^= 0xff;
    assert_eq!(
        assert_rejected(&bad_magic, "magic"),
        QueryWireError::InvalidHeader
    );

    for (label, offset, value) in [
        ("version", 8, 2_u16),
        ("header length", 10, (HEADER_LEN as u16) + 1),
        ("record length", 12, (RECORD_LEN as u16) + 1),
        ("reserved header", 14, 1),
    ] {
        let mut mutated = wire.clone();
        write_u16(&mut mutated, offset, value);
        assert_eq!(
            assert_rejected(&mutated, label),
            QueryWireError::InvalidHeader,
            "{label}",
        );
    }

    let mut wrong_root = wire.clone();
    write_u32(&mut wrong_root, 16, 0);
    assert_eq!(
        assert_rejected(&wrong_root, "root index"),
        QueryWireError::InvalidHeader
    );

    let mut zero_records = wire.clone();
    write_u32(&mut zero_records, 16, 0);
    write_u32(&mut zero_records, 20, 0);
    assert_eq!(
        assert_rejected(&zero_records, "zero records"),
        QueryWireError::InvalidHeader
    );

    assert_eq!(
        assert_rejected(&wire[..wire.len() - 1], "truncated wire"),
        QueryWireError::InvalidLength
    );
    let mut trailing = wire.clone();
    trailing.push(0);
    assert_eq!(
        assert_rejected(&trailing, "trailing wire"),
        QueryWireError::InvalidLength
    );
}

#[test]
fn declared_counts_reject_n_plus_one_before_body_access() {
    let wire = canonical_wire();
    for (label, offset, value, expected) in [
        (
            "record count",
            20,
            (MAX_RECORDS_V1 + 1) as u32,
            QueryWireLimit::Records,
        ),
        (
            "edge count",
            24,
            (MAX_EDGES_V1 + 1) as u32,
            QueryWireLimit::Edges,
        ),
        (
            "scalar bytes",
            28,
            (MAX_SCALAR_BYTES_V1 + 1) as u32,
            QueryWireLimit::ScalarBytes,
        ),
    ] {
        let mut mutated = wire.clone();
        write_u32(&mut mutated, offset, value);
        if offset == 20 {
            write_u32(&mut mutated, 16, value - 1);
        }
        assert_eq!(
            assert_rejected(&mutated, label),
            QueryWireError::LimitExceeded(expected),
            "{label}",
        );
    }
}

#[test]
fn component_limit_accounting_accepts_zero_and_exact_n_but_rejects_n_plus_one() {
    assert_eq!(checked_wire_len(0, 0, 0), Ok(HEADER_LEN));
    assert!(checked_wire_len(MAX_RECORDS_V1, 0, 0).is_ok());
    assert!(checked_wire_len(0, MAX_EDGES_V1, 0).is_ok());
    assert!(checked_wire_len(0, 0, MAX_SCALAR_BYTES_V1).is_ok());
    assert!(
        checked_wire_len(MAX_RECORDS_V1, MAX_EDGES_V1, MAX_SCALAR_BYTES_V1)
            .is_ok_and(|length| length <= MAX_WIRE_BYTES_V1)
    );
    assert_eq!(
        checked_wire_len(MAX_RECORDS_V1 + 1, 0, 0),
        Err(QueryWireError::LimitExceeded(QueryWireLimit::Records))
    );
    assert_eq!(
        checked_wire_len(0, MAX_EDGES_V1 + 1, 0),
        Err(QueryWireError::LimitExceeded(QueryWireLimit::Edges))
    );
    assert_eq!(
        checked_wire_len(0, 0, MAX_SCALAR_BYTES_V1 + 1),
        Err(QueryWireError::LimitExceeded(QueryWireLimit::ScalarBytes))
    );
}

#[test]
fn raw_wire_limit_rejects_n_plus_one_before_header_access() {
    let oversized = vec![0; MAX_WIRE_BYTES_V1 + 1];
    assert_eq!(
        assert_rejected(&oversized, "oversized raw worker response"),
        QueryWireError::LimitExceeded(QueryWireLimit::WireBytes),
    );
}

#[test]
fn unknown_records_nonzero_reserved_fields_and_flags_are_rejected() {
    let wire = canonical_wire();
    let root = record_count(&wire) - 1;
    let mut unknown = wire.clone();
    let bgp = find_record(&wire, tag::GRAPH_BGP);
    write_u16(&mut unknown, record_offset(bgp), u16::MAX);
    assert_eq!(
        assert_rejected(&unknown, "unknown tag"),
        QueryWireError::UnknownRecordKind
    );

    let mut flags = wire.clone();
    write_u16(&mut flags, record_offset(root) + 2, 0x8000);
    assert_rejected(&flags, "unknown query flags");

    let mut field = wire.clone();
    write_u32(&mut field, record_offset(root) + 8, 1);
    assert_rejected(&field, "nonzero reserved query field");

    let variable = find_record(&wire, tag::VARIABLE);
    let mut scalar_reserved = wire.clone();
    write_u32(&mut scalar_reserved, record_offset(variable) + 12, 1);
    assert_rejected(&scalar_reserved, "nonzero reserved scalar field");
}

#[test]
fn edge_ranges_must_be_contiguous_and_inside_declared_table() {
    let wire = canonical_wire();
    let record = (0..record_count(&wire))
        .find(|&index| record_edge_len(&wire, index) > 0)
        .expect("edge-bearing record");

    let mut gap = wire.clone();
    let start = record_edge_start(&gap, record) as u32;
    write_u32(&mut gap, record_offset(record) + 24, start + 1);
    assert_rejected(&gap, "edge gap");

    let mut excessive = wire.clone();
    write_u32(
        &mut excessive,
        record_offset(record) + 28,
        edge_count(&wire) as u32 + 1,
    );
    assert_rejected(&excessive, "edge range overflow");

    let later = (0..record_count(&wire))
        .find(|&index| record_edge_start(&wire, index) > 0)
        .expect("later edge range");
    let mut overlap = wire.clone();
    write_u32(
        &mut overlap,
        record_offset(later) + 24,
        record_edge_start(&wire, later) as u32 - 1,
    );
    assert_rejected(&overlap, "overlapping edge range");

    let mut missing = wire.clone();
    write_u32(&mut missing, 24, edge_count(&wire) as u32 - 1);
    missing.remove(edge_table_offset(&wire) + (edge_count(&wire) - 1) * 4);
    assert_rejected(&missing, "unaccounted record edge");
}

#[test]
fn forward_out_of_range_duplicate_and_no_index_edges_are_rejected() {
    let wire = canonical_wire();
    let multi = (0..record_count(&wire))
        .find(|&index| record_edge_len(&wire, index) >= 2)
        .expect("multi-edge record");

    let mut forward = wire.clone();
    write_u32(&mut forward, edge_offset(&wire, multi, 0), multi as u32);
    assert_rejected(&forward, "forward edge");

    let mut outside = wire.clone();
    write_u32(
        &mut outside,
        edge_offset(&wire, multi, 0),
        record_count(&wire) as u32,
    );
    assert_rejected(&outside, "out-of-range edge");

    let mut duplicate = wire.clone();
    let first = read_u32(&wire, edge_offset(&wire, multi, 0));
    write_u32(&mut duplicate, edge_offset(&wire, multi, 1), first);
    assert_rejected(&duplicate, "duplicate edge and orphaned record");

    let non_values = (0..record_count(&wire))
        .find(|&index| {
            record_edge_len(&wire, index) > 0 && record_tag(&wire, index) != tag::VALUES_ROW
        })
        .expect("ordinary edge-bearing record");
    let mut no_index = wire.clone();
    write_u32(&mut no_index, edge_offset(&wire, non_values, 0), NO_INDEX);
    assert_rejected(&no_index, "NO_INDEX outside VALUES row");
}

#[test]
fn child_type_confusion_is_rejected_even_when_ownership_is_preserved() {
    let wire = canonical_wire();
    let mut confused = wire.clone();
    let term_variable = find_record(&wire, tag::TERM_VARIABLE);
    write_u16(
        &mut confused,
        record_offset(term_variable),
        tag::NAMED_PATTERN_VARIABLE,
    );
    assert_eq!(
        assert_rejected(&confused, "typed child swap"),
        QueryWireError::TypeMismatch
    );
}

#[test]
fn scalar_ranges_utf8_and_checked_domain_constructors_are_closed() {
    let wire = canonical_wire();
    let variable = find_record(&wire, tag::VARIABLE);

    let mut gap = wire.clone();
    let offset = read_u32(&gap, record_offset(variable) + 4);
    write_u32(&mut gap, record_offset(variable) + 4, offset + 1);
    assert_rejected(&gap, "scalar range gap");

    let mut invalid_utf8 = wire.clone();
    invalid_utf8[scalar_range(&wire, variable).start] = 0xff;
    assert_eq!(
        assert_rejected(&invalid_utf8, "invalid UTF-8"),
        QueryWireError::InvalidScalar
    );

    let mut invalid_variable = wire.clone();
    invalid_variable[scalar_range(&wire, variable).start] = b'-';
    assert_eq!(
        assert_rejected(&invalid_variable, "invalid variable"),
        QueryWireError::InvalidScalar
    );

    let named = find_record(&wire, tag::NAMED_NODE);
    let mut invalid_iri = wire.clone();
    invalid_iri[scalar_range(&wire, named).start] = b' ';
    assert_eq!(
        assert_rejected(&invalid_iri, "invalid IRI"),
        QueryWireError::InvalidScalar
    );

    let blank = find_record(&wire, tag::BLANK_NODE);
    let mut invalid_blank = wire.clone();
    invalid_blank[scalar_range(&wire, blank).start] = b'.';
    assert_eq!(
        assert_rejected(&invalid_blank, "invalid blank node"),
        QueryWireError::InvalidScalar
    );

    let later_scalar = (0..record_count(&wire))
        .find(|&index| {
            matches!(
                record_tag(&wire, index),
                tag::NAMED_NODE | tag::VARIABLE | tag::BLANK_NODE | tag::LITERAL | tag::BASE_IRI
            ) && read_u32(&wire, record_offset(index) + 4) > 0
        })
        .expect("later scalar record");
    let mut overlap = wire.clone();
    let start = read_u32(&wire, record_offset(later_scalar) + 4);
    write_u32(&mut overlap, record_offset(later_scalar) + 4, start - 1);
    assert_rejected(&overlap, "overlapping scalar range");

    let language_literal = (0..record_count(&wire))
        .find(|&index| {
            record_tag(&wire, index) == tag::LITERAL
                && read_u32(&wire, record_offset(index) + 20) == 2
        })
        .expect("language literal");
    let mut invalid_language = wire.clone();
    invalid_language[auxiliary_scalar_range(&wire, language_literal).start] = b'_';
    assert_eq!(
        assert_rejected(&invalid_language, "invalid language tag"),
        QueryWireError::InvalidScalar
    );
}

#[test]
fn literal_discriminants_and_unaccounted_scalar_tail_are_rejected() {
    let wire = canonical_wire();
    let literal = find_record(&wire, tag::LITERAL);

    let mut kind = wire.clone();
    write_u32(&mut kind, record_offset(literal) + 20, 99);
    assert_rejected(&kind, "unknown literal kind");

    let mut flags = wire.clone();
    write_u16(&mut flags, record_offset(literal) + 2, 1);
    assert_rejected(&flags, "literal reserved flags");

    let mut scalar_tail = wire.clone();
    write_u32(&mut scalar_tail, 28, scalar_len(&wire) as u32 + 1);
    scalar_tail.push(b'x');
    assert_rejected(&scalar_tail, "unaccounted scalar tail");
}

#[test]
fn canonical_replay_rejects_semantically_ignored_alternate_encoding() {
    let wire = canonical_wire();
    let bgp = find_record(&wire, tag::GRAPH_BGP);
    let mut alternate = wire.clone();
    write_u32(&mut alternate, record_offset(bgp) + 8, 1);
    assert_rejected(&alternate, "ignored graph field");
}

#[test]
fn encoder_rejects_unchecked_rdf_scalars_before_emitting_wire() {
    use spargebra::algebra::GraphPattern;
    use spargebra::term::{Literal, NamedNode, TermPattern, TriplePattern, Variable};

    let ask = |pattern| Query::Ask {
        dataset: None,
        pattern,
        base_iri: None,
    };
    let empty = || Box::new(GraphPattern::Bgp { patterns: vec![] });

    let invalid_variable = ask(GraphPattern::Project {
        inner: empty(),
        variables: vec![Variable::new_unchecked("-")],
    });
    assert_eq!(
        encode(&invalid_variable),
        Err(QueryWireError::InvalidScalar)
    );

    let invalid_named_node = ask(GraphPattern::Bgp {
        patterns: vec![TriplePattern {
            subject: TermPattern::NamedNode(NamedNode::new_unchecked(" ")),
            predicate: Variable::new_unchecked("p").into(),
            object: Variable::new_unchecked("o").into(),
        }],
    });
    assert_eq!(
        encode(&invalid_named_node),
        Err(QueryWireError::InvalidScalar)
    );

    let uppercase_language = ask(GraphPattern::Bgp {
        patterns: vec![TriplePattern {
            subject: Variable::new_unchecked("s").into(),
            predicate: Variable::new_unchecked("p").into(),
            object: Literal::new_language_tagged_literal_unchecked("value", "EN").into(),
        }],
    });
    assert_eq!(
        encode(&uppercase_language),
        Err(QueryWireError::NonCanonical)
    );

    let invalid_datatype = ask(GraphPattern::Bgp {
        patterns: vec![TriplePattern {
            subject: Variable::new_unchecked("s").into(),
            predicate: Variable::new_unchecked("p").into(),
            object: Literal::new_typed_literal("value", NamedNode::new_unchecked(" ")).into(),
        }],
    });
    assert_eq!(
        encode(&invalid_datatype),
        Err(QueryWireError::InvalidScalar)
    );
}
