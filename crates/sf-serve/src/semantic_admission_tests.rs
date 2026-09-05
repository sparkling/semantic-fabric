use sf_core::{Column, SourceId, SourceMapping, TableSchema};
use sf_sparql::Epoch;

use crate::activation::RuntimeManager;
use crate::binding::RuntimeBinding;
use crate::semantic_admission::{MappingOrigin, SemanticAdmissionError, ValidatedMapping};
use crate::{
    Backend, IntrospectedSource, RuntimeSnapshot, RuntimeSource, SemanticOntology, SnapshotError,
};

const PREFIXES: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix sh:  <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix mf:  <http://example.org/mapping-fabric#> .
@prefix ex:  <http://example.test/> .
"#;

fn ontology(body: &str) -> SemanticOntology {
    SemanticOntology::from_turtle(&format!("{PREFIXES}\n{body}")).expect("test ontology is valid")
}

fn observed(sql_type: &str) -> IntrospectedSource {
    let mut table = TableSchema::new("items");
    table.columns = vec![Column::new("value", sql_type, false)];
    IntrospectedSource::unchecked(
        Backend::sqlite(rusqlite::Connection::open_in_memory().unwrap()),
        vec![table],
    )
}

fn mapping(index: usize, body: &str) -> SourceMapping {
    sf_mapping::parse_r2rml_for_source(
        &format!(
            "{PREFIXES}\n\
             <#items> a rr:TriplesMap;\n\
               rr:logicalTable [ rr:tableName \"items\" ];\n\
               rr:subjectMap [ rr:constant ex:item {body} ]."
        ),
        SourceId::new(index).unwrap(),
    )
    .expect("test mapping is valid")
}

fn pending(index: usize, body: &str, sql_type: &str, origin: MappingOrigin) -> RuntimeSource {
    let source = observed(sql_type);
    let mapping = mapping(index, body);
    match origin {
        MappingOrigin::Authored => RuntimeSource::new(source, mapping),
        MappingOrigin::Direct => RuntimeSource::direct(source, mapping),
    }
}

fn admission_cause(error: SnapshotError) -> SemanticAdmissionError {
    match error {
        SnapshotError::SemanticAdmission { cause, .. } => cause,
        other => panic!("expected semantic admission error, got {other:?}"),
    }
}

#[test]
fn runtime_gate_accepts_and_rejects_class_and_predicate_targets() {
    let class_mapping = " ; rr:class ex:Item";
    let property_mapping =
        " ]; rr:predicateObjectMap [ rr:predicate ex:value; rr:objectMap [ rr:constant ex:o ]";

    RuntimeSnapshot::single(
        Epoch(0),
        ontology("ex:Item a owl:Class ."),
        pending(0, class_mapping, "text", MappingOrigin::Authored),
    )
    .expect("declared class is admitted");
    assert!(matches!(
        admission_cause(
            RuntimeSnapshot::single(
                Epoch(0),
                ontology(""),
                pending(0, class_mapping, "text", MappingOrigin::Authored),
            )
            .unwrap_err()
        ),
        SemanticAdmissionError::Violations { count } if count > 0
    ));

    RuntimeSnapshot::single(
        Epoch(0),
        ontology("ex:value a rdf:Property ."),
        pending(0, property_mapping, "text", MappingOrigin::Authored),
    )
    .expect("declared predicate is admitted");
    assert!(matches!(
        admission_cause(
            RuntimeSnapshot::single(
                Epoch(0),
                ontology(""),
                pending(0, property_mapping, "text", MappingOrigin::Authored),
            )
            .unwrap_err()
        ),
        SemanticAdmissionError::Violations { count } if count > 0
    ));
}

#[test]
fn runtime_gate_checks_the_effective_implicit_source_datatype() {
    let body =
        " ]; rr:predicateObjectMap [ rr:predicate ex:value; rr:objectMap [ rr:column \"value\" ]";
    let integer_t = ontology(
        "ex:value a owl:DatatypeProperty .\n\
         ex:shape sh:property [ sh:path ex:value; sh:datatype xsd:integer ] .",
    );
    RuntimeSnapshot::single(
        Epoch(0),
        integer_t,
        pending(0, body, "integer", MappingOrigin::Authored),
    )
    .expect("integer execution type matches ontology");

    let string_t = ontology(
        "ex:value a owl:DatatypeProperty .\n\
         ex:shape sh:property [ sh:path ex:value; sh:datatype xsd:string ] .",
    );
    assert!(matches!(
        admission_cause(
            RuntimeSnapshot::single(
                Epoch(0),
                string_t,
                pending(0, body, "integer", MappingOrigin::Authored),
            )
            .unwrap_err()
        ),
        SemanticAdmissionError::Violations { count } if count > 0
    ));
    assert_eq!(
        admission_cause(
            RuntimeSnapshot::single(
                Epoch(0),
                ontology("ex:value a rdf:Property ."),
                pending(0, body, "unresolved_type", MappingOrigin::Authored),
            )
            .unwrap_err()
        ),
        SemanticAdmissionError::ImplicitDatatypeUnresolved
    );
}

#[test]
fn origin_and_declaration_only_ontology_changes_partition_runtime_identity() {
    let body = " ; rr:class ex:Item";
    let authored = RuntimeSnapshot::single(
        Epoch(0),
        ontology("ex:Item a owl:Class ."),
        pending(0, body, "text", MappingOrigin::Authored),
    )
    .unwrap();
    let direct = RuntimeSnapshot::single(
        Epoch(0),
        ontology("ex:Item a owl:Class ."),
        pending(0, body, "text", MappingOrigin::Direct),
    )
    .unwrap();
    let declaration_changed = RuntimeSnapshot::single(
        Epoch(0),
        ontology("ex:Item a owl:Class . ex:Unused a owl:Class ."),
        pending(0, body, "text", MappingOrigin::Authored),
    )
    .unwrap();
    let id = SourceId::new(0).unwrap();
    assert_eq!(
        authored.registry().digests(id).unwrap().mapping(),
        direct.registry().digests(id).unwrap().mapping()
    );
    assert_ne!(
        authored
            .registry()
            .digests(id)
            .unwrap()
            .semantic_admission(),
        direct.registry().digests(id).unwrap().semantic_admission()
    );
    assert_ne!(
        authored.ontology_digest(),
        declaration_changed.ontology_digest()
    );
    assert_ne!(
        authored
            .registry()
            .digests(id)
            .unwrap()
            .semantic_admission(),
        declaration_changed
            .registry()
            .digests(id)
            .unwrap()
            .semantic_admission()
    );
}

#[test]
fn receipts_cannot_be_detached_from_ontology_or_effective_source_types() {
    let body =
        " ]; rr:predicateObjectMap [ rr:predicate ex:value; rr:objectMap [ rr:column \"value\" ]";
    let first_t = ontology("ex:value a rdf:Property .");
    let first_source = observed("integer");
    let receipt = ValidatedMapping::validate(
        mapping(0, body),
        MappingOrigin::Authored,
        &first_t,
        &first_source,
    )
    .unwrap();
    assert_eq!(
        admission_cause(
            RuntimeSnapshot::single(
                Epoch(0),
                ontology("ex:value a rdf:Property . ex:Other a owl:Class ."),
                RuntimeSource::admitted(first_source, receipt),
            )
            .unwrap_err()
        ),
        SemanticAdmissionError::ReceiptOntologyMismatch
    );

    let first_t = ontology("ex:value a rdf:Property .");
    let first_source = observed("integer");
    let receipt = ValidatedMapping::validate(
        mapping(0, body),
        MappingOrigin::Authored,
        &first_t,
        &first_source,
    )
    .unwrap();
    assert_eq!(
        admission_cause(
            RuntimeSnapshot::single(
                Epoch(0),
                first_t,
                RuntimeSource::admitted(observed("text"), receipt),
            )
            .unwrap_err()
        ),
        SemanticAdmissionError::ReceiptSourceMismatch
    );
}

#[test]
fn ontology_cannot_use_reserved_projection_nodes_in_any_asserted_position() {
    const RESERVED: &str = "<urn:semantic-fabric:mjoin-t:v1:s0:m0:object:p0:o0>";
    let body =
        " ]; rr:predicateObjectMap [ rr:predicate ex:value; rr:objectMap [ rr:constant ex:o ]";
    for collision in [
        format!("{RESERVED} rr:datatype xsd:string ."),
        format!("ex:forged {RESERVED} ex:value ."),
        format!("ex:forged ex:value {RESERVED} ."),
    ] {
        let malicious = ontology(&format!(
            "ex:value a rdf:Property .\n\
             ex:shape sh:property [ sh:path ex:value; sh:datatype xsd:string ] .\n\
             {collision}"
        ));
        assert_eq!(
            admission_cause(
                RuntimeSnapshot::single(
                    Epoch(0),
                    malicious,
                    pending(0, body, "text", MappingOrigin::Authored),
                )
                .unwrap_err()
            ),
            SemanticAdmissionError::ProvenanceCollision
        );
    }
}

#[test]
fn federation_validates_every_arm_before_constructing_any_binding() {
    RuntimeBinding::reset_test_construction_count();
    let result = RuntimeSnapshot::new(
        Epoch(0),
        ontology("ex:ok a rdf:Property ."),
        vec![
            pending(
                0,
                " ]; rr:predicateObjectMap [ rr:predicate ex:ok; rr:objectMap [ rr:constant ex:o ]",
                "text",
                MappingOrigin::Authored,
            ),
            pending(
                1,
                " ]; rr:predicateObjectMap [ rr:predicate ex:dangling; rr:objectMap [ rr:constant ex:o ]",
                "text",
                MappingOrigin::Authored,
            ),
        ],
    );
    assert!(result.is_err());
    assert_eq!(RuntimeBinding::test_construction_count(), 0);
}

#[test]
fn a_semantically_invalid_candidate_cannot_replace_the_active_snapshot() {
    let active = RuntimeSnapshot::single(
        Epoch(0),
        ontology(""),
        RuntimeSource::new(
            observed("text"),
            SourceMapping::new(SourceId::new(0).unwrap(), Vec::new()),
        ),
    )
    .unwrap();
    let manager = RuntimeManager::new(active);
    let before = manager.lease().unwrap();
    let identity = before.weak_snapshot();
    let candidate = RuntimeSnapshot::single(
        Epoch(1),
        ontology(""),
        pending(
            0,
            " ]; rr:predicateObjectMap [ rr:predicate ex:dangling; rr:objectMap [ rr:constant ex:o ]",
            "text",
            MappingOrigin::Authored,
        ),
    );
    assert!(candidate.is_err());
    let after = manager.lease().unwrap();
    assert_eq!(before.activation_id(), after.activation_id());
    assert!(std::sync::Weak::ptr_eq(&identity, &after.weak_snapshot()));
}

#[test]
fn warnings_are_counted_without_authorizing_a_failure() {
    let snapshot = RuntimeSnapshot::single(
        Epoch(0),
        ontology("ex:Entity a owl:Class; mf:stereotype mf:Entity ."),
        RuntimeSource::new(
            observed("text"),
            SourceMapping::new(SourceId::new(0).unwrap(), Vec::new()),
        ),
    )
    .expect("grounding warning remains advisory");
    assert!(snapshot
        .registry()
        .semantic_warning_count(SourceId::new(0).unwrap())
        .is_some_and(|count| count > 0));
}

#[test]
fn dynamic_predicates_fail_closed() {
    let body =
        " ]; rr:predicateObjectMap [ rr:predicateMap [ rr:column \"value\" ]; rr:objectMap [ rr:constant ex:o ]";
    assert_eq!(
        admission_cause(
            RuntimeSnapshot::single(
                Epoch(0),
                ontology(""),
                pending(0, body, "text", MappingOrigin::Authored),
            )
            .unwrap_err()
        ),
        SemanticAdmissionError::DynamicPredicate
    );
}
