use super::*;

const MAP: &str = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@base <http://document.example/first/> .
<#items> rr:logicalTable [rr:tableName "items"];
 rr:subjectMap [rr:template "../items/{id}"; rr:graphMap [rr:column "g"]];
 rr:predicateObjectMap [rr:predicate <value>; rr:objectMap [rr:column "o"; rr:termType rr:IRI]].
@base <http://document.example/last/> .
<#other> rr:logicalTable [rr:tableName "items"]; rr:subjectMap [rr:column "o"].
"#;

#[test]
fn processor_base_is_independent_of_every_turtle_base_directive() {
    let options = R2rmlOptions {
        processor_base_iri: "http://data.example/root/",
        document_base_iri: "http://initial.example/",
    };
    let maps = parse_r2rml_with_options(MAP, options).unwrap();
    assert_eq!(maps[0].id, "http://document.example/first/#items");
    assert_eq!(maps[1].id, "http://document.example/last/#other");
    let row: &[(&str, Option<&str>)] = &[
        ("id", Some("x")),
        ("g", Some("/graph")),
        ("o", Some("../value")),
    ];
    assert_eq!(
        sf_core::term::generate(&maps[0].subject.term, row)
            .unwrap()
            .unwrap()
            .to_string(),
        "<http://data.example/root/../items/x>"
    );
    assert_eq!(
        sf_core::term::generate(&maps[0].subject.graphs[0], row)
            .unwrap()
            .unwrap()
            .to_string(),
        "<http://data.example/root//graph>"
    );
    assert_eq!(
        sf_core::term::generate(&maps[1].subject.term, row)
            .unwrap()
            .unwrap()
            .to_string(),
        "<http://data.example/root/../value>"
    );
    assert!(
        matches!(&maps[0].predicate_object_maps[0].predicates[0], TermMap::Constant(Term::NamedNode(iri)) if iri.as_str() == "http://document.example/first/value")
    );
}

#[test]
fn invalid_or_oversized_bases_fail_before_turtle_without_echoing_input() {
    for bad in [
        "secret relative".to_owned(),
        format!("http://ex/{}", "x".repeat(MAX_R2RML_BASE_IRI_BYTES)),
    ] {
        for processor in [false, true] {
            let mut options = R2rmlOptions::default();
            if processor {
                options.processor_base_iri = &bad;
            } else {
                options.document_base_iri = &bad;
            }
            let error = parse_r2rml_with_options("not Turtle", options)
                .unwrap_err()
                .to_string();
            assert!(error.contains("R2RML base"), "{error}");
            assert!(!error.contains(&bad));
        }
    }
    for base in ["http://ex/no-slash", "http://ex/?q", "http://ex/#f"] {
        validate_r2rml_base(base).unwrap();
    }
}

#[test]
fn template_base_is_selected_after_expansion_not_from_its_first_literal() {
    for (recipe, value, expected) in [
        ("{v}://host/x", "http", "http://host/x"),
        ("{v}://host/x", "1", "http://data.example/1://host/x"),
        ("ur{v}:x", "n", "urn:x"),
        ("http://host:{v}/x", "80", "http://host:80/x"),
        (
            "http://host:{v}/x",
            "abc",
            "http://data.example/http://host:abc/x",
        ),
        ("{v}", "../x", "http://data.example/..%2Fx"),
        ("{v}", "", "http://data.example/"),
        ("http://host/%{v}", "20", "http://host/%20"),
        ("http://host/{v}", "%20", "http://host/%2520"),
    ] {
        let mapping = format!(
            r#"@prefix rr: <http://www.w3.org/ns/r2rml#> .
@base <http://document.example/> .
<#m> rr:logicalTable [rr:tableName "items"]; rr:subjectMap [rr:template "{recipe}"];
rr:predicateObjectMap [rr:predicate <http://ex/p>; rr:object "value"]."#
        );
        let maps = parse_r2rml_with_options(
            &mapping,
            R2rmlOptions {
                processor_base_iri: "http://data.example/",
                ..R2rmlOptions::default()
            },
        )
        .unwrap();
        let term = &maps[0].subject.term;
        let row = [("v", Some(value))];
        assert_eq!(
            sf_core::term::generate(term, &row[..])
                .unwrap()
                .unwrap()
                .to_string(),
            format!("<{expected}>"),
            "{recipe}/{value:?}"
        );
        assert!(sf_core::term::generate(term, &[("v", None)][..])
            .unwrap()
            .is_none());
    }
}
