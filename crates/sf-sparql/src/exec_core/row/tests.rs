use super::*;
use sf_core::ir::TermSpec;

fn blank(graph: R2rmlGraphScope, graph_value: &str) -> Term {
    let schema = vec![ColRef::new(0, "id"), ColRef::new(0, "graph")];
    let index = build_col_index(&schema);
    let values = vec![Some("shared".to_owned()), Some(graph_value.to_owned())];
    let codes = vec![None, None];
    let raw = RawRow {
        values: &values,
        codes: &codes,
        index: &index,
    };
    build_term(
        &TermDef::R2rmlBlank {
            term_map: TermMap::Column("id".into(), TermSpec::blank_node()),
            alias: 0,
            graph,
        },
        &raw,
    )
    .unwrap()
    .unwrap()
}

#[test]
fn generated_labels_are_injective_over_effective_graph_and_identifier() {
    let default = blank(R2rmlGraphScope::Default, "unused");
    let named = blank(
        R2rmlGraphScope::Mapped {
            term_map: TermMap::Constant(Term::NamedNode(sf_core::NamedNode::new_unchecked(
                "http://ex/g1",
            ))),
            alias: 0,
        },
        "unused",
    );
    let dynamic_named = blank(
        R2rmlGraphScope::Mapped {
            term_map: TermMap::Column("graph".into(), TermSpec::iri()),
            alias: 0,
        },
        "http://ex/g1",
    );
    let dynamic_default = blank(
        R2rmlGraphScope::Mapped {
            term_map: TermMap::Column("graph".into(), TermSpec::iri()),
            alias: 0,
        },
        RR_DEFAULT_GRAPH,
    );

    assert_eq!(named, dynamic_named);
    assert_eq!(default, dynamic_default);
    assert_ne!(default, named);
    assert!(matches!(default, Term::BlankNode(ref b) if b.as_str().starts_with("sfr1d_")));
    assert!(matches!(named, Term::BlankNode(ref b) if b.as_str().starts_with("sfr1n_")));
}

#[test]
fn explicit_rr_datatype_overrides_compatibility_type_for_value_two() {
    let schema = vec![ColRef::new(0, "flag")];
    let index = build_col_index(&schema);
    let values = vec![Some("2".to_owned())];
    let codes = vec![Some(XsdTypeCode::Boolean)];
    let raw = RawRow {
        values: &values,
        codes: &codes,
        index: &index,
    };
    let term_map = TermMap::Column(
        "flag".into(),
        TermSpec::typed_literal(sf_core::NamedNode::from(sf_core::vocab::xsd::INTEGER)),
    );

    let term = derived_term(&term_map, 0, &raw).unwrap().unwrap();
    let Term::Literal(literal) = term else {
        panic!("explicit rr:datatype did not produce a literal");
    };
    assert_eq!(literal.value(), "2");
    assert_eq!(literal.datatype(), sf_core::vocab::xsd::INTEGER);
}
