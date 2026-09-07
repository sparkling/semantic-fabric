use oxrdf::Graph;
use rudof_rdf::rdf_impl::OxigraphInMemory;
use shacl::ir::IRSchema;
use shacl::rdf::ShaclParser;
use shacl::types::Severity;
use shacl::validator::processor::{DataValidation, ShaclProcessor};
use shacl::validator::ShaclValidationMode;

use super::*;
use crate::{GateOutcome, META_SHAPES_TTL};

const PREFIXES: &str = r#"
@prefix mf:  <http://example.org/mapping-fabric#> .
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix rml: <http://semweb.mmlab.be/ns/rml#> .
@prefix sh:  <http://www.w3.org/ns/shacl#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix ex:  <http://ex/> .
"#;

fn graph(input: &str) -> Graph {
    crate::parse_turtle_graph(input, crate::DEFAULT_GRAPH_LIMITS).unwrap()
}

fn rdf_data(graph: &Graph) -> RdfData {
    let mut data = OxigraphInMemory::new();
    for triple in graph.iter() {
        data.add_triple_ref(triple.subject, triple.predicate, triple.object)
            .unwrap();
    }
    RdfData::from_graph(data).unwrap()
}

fn per_focus_reference_result(input: &str) -> Result<GateOutcome, ()> {
    let shapes = RdfData::from_str(
        META_SHAPES_TTL,
        &rudof_rdf::rdf_core::RDFFormat::Turtle,
        None,
        &rudof_rdf::rdf_impl::ReaderMode::Strict,
    )
    .unwrap();
    let schema = ShaclParser::new(shapes).parse().unwrap();
    let schema: IRSchema = schema.try_into().unwrap();
    let mut validator: DataValidation = rdf_data(&graph(input)).into();
    let report = validator
        .validate(&schema, &ShaclValidationMode::Native)
        .map_err(|_| ())?;
    Ok(GateOutcome {
        violations: report.get_count_of(&Severity::Violation),
        warnings: report.get_count_of(&Severity::Warning),
    })
}

fn per_focus_reference(input: &str) -> GateOutcome {
    per_focus_reference_result(input).unwrap()
}

fn batch_schema() -> IRSchema {
    let shapes = format!(
        "{META_SHAPES_TTL}\n<{SHAPE_IRI}> \
             <http://www.w3.org/ns/shacl#deactivated> true .\n"
    );
    IRSchema::from_str(
        &shapes,
        &rudof_rdf::rdf_core::RDFFormat::Turtle,
        None,
        &rudof_rdf::rdf_impl::ReaderMode::Strict,
    )
    .unwrap()
}

fn direct_batch_count(input: &str) -> usize {
    let schema = batch_schema();
    let select = select(&schema).unwrap();
    let mut data = rdf_data(&graph(input));
    data.check_store().unwrap();
    violation_count(&data, select, usize::MAX, usize::MAX).unwrap()
}

fn assert_differential(name: &str, body: &str, expected: GateOutcome) {
    let input = format!("{PREFIXES}\n{body}");
    let batched = crate::validate_turtle(&input).unwrap();
    let reference = per_focus_reference(&input);
    assert_eq!(batched, expected, "batched outcome for {name}");
    assert_eq!(batched, reference, "per-focus differential for {name}");
}

#[test]
fn batched_rule_matches_per_focus_semantics_across_mutations() {
    let base = r#"
ex:pom rr:predicate ex:age; rr:objectMap ex:om .
ex:age a owl:DatatypeProperty .
ex:shape sh:property ex:propertyShape .
ex:propertyShape sh:path ex:age; sh:datatype xsd:integer .
"#;
    assert_differential(
        "matching datatype",
        &format!("{base}\nex:om rr:datatype xsd:integer ."),
        GateOutcome {
            violations: 0,
            warnings: 0,
        },
    );
    assert_differential(
        "mismatching datatype",
        &format!("{base}\nex:om rr:datatype xsd:string ."),
        GateOutcome {
            violations: 1,
            warnings: 0,
        },
    );
    assert_differential(
        "missing datatype",
        base,
        GateOutcome {
            violations: 1,
            warnings: 0,
        },
    );
    assert_differential(
        "unconstrained predicate",
        r#"
ex:pom rr:predicate ex:value; rr:objectMap ex:om .
ex:value a owl:DatatypeProperty .
ex:om rr:datatype xsd:string .
"#,
        GateOutcome {
            violations: 0,
            warnings: 0,
        },
    );
    assert_differential(
        "legacy RML alternatives",
        r#"
ex:pom rml:predicate ex:age; rml:objectMap ex:om .
ex:age a owl:DatatypeProperty .
ex:om rml:datatype xsd:string .
ex:shape sh:property ex:propertyShape .
ex:propertyShape sh:path ex:age; sh:datatype xsd:integer .
"#,
        GateOutcome {
            violations: 1,
            warnings: 0,
        },
    );
    for (name, predicate, object_map, datatype) in [
        ("rr focus with rml object-map", "rr", "rml", "rml"),
        ("rml focus with rr object-map", "rml", "rr", "rr"),
    ] {
        let body = format!(
            "ex:pom {predicate}:predicate ex:age; {object_map}:objectMap ex:om .\n\
                 ex:age a owl:DatatypeProperty .\n\
                 ex:om {datatype}:datatype xsd:string .\n\
                 ex:shape sh:property ex:propertyShape .\n\
                 ex:propertyShape sh:path ex:age; sh:datatype xsd:integer ."
        );
        assert_differential(
            name,
            &body,
            GateOutcome {
                violations: 1,
                warnings: 0,
            },
        );
    }
    assert_differential(
        "multiple focus nodes",
        r#"
ex:pom1 rr:predicate ex:age; rr:objectMap ex:om1 .
ex:pom2 rr:predicate ex:age; rr:objectMap ex:om2 .
ex:age a owl:DatatypeProperty .
ex:om1 rr:datatype xsd:string .
ex:om2 rr:datatype xsd:integer .
ex:shape sh:property ex:propertyShape .
ex:propertyShape sh:path ex:age; sh:datatype xsd:integer .
"#,
        GateOutcome {
            violations: 1,
            warnings: 0,
        },
    );
    assert_differential(
        "rr and rml focus and object-map alternatives",
        r#"
ex:pom rr:predicate ex:age; rml:predicate ex:age;
       rr:objectMap ex:om1; rml:objectMap ex:om2 .
ex:age a owl:DatatypeProperty .
ex:om1 rr:datatype xsd:string .
ex:om2 rml:datatype xsd:integer .
ex:shape sh:property ex:propertyShape .
ex:propertyShape sh:path ex:age; sh:datatype xsd:integer .
"#,
        GateOutcome {
            violations: 1,
            warnings: 0,
        },
    );
    assert_differential(
        "no object-map",
        r#"
ex:pom rr:predicate ex:age .
ex:age a owl:DatatypeProperty .
ex:shape sh:property ex:propertyShape .
ex:propertyShape sh:path ex:age; sh:datatype xsd:integer .
"#,
        GateOutcome {
            violations: 0,
            warnings: 0,
        },
    );
    assert_differential(
        "ownerless property-shape",
        r#"
ex:pom rr:predicate ex:age; rr:objectMap ex:om .
ex:age a owl:DatatypeProperty .
ex:om rr:datatype xsd:string .
ex:orphan sh:path ex:age; sh:datatype xsd:integer .
"#,
        GateOutcome {
            violations: 0,
            warnings: 0,
        },
    );
    assert_differential(
        "multiple owners and paths",
        r#"
ex:pom rr:predicate ex:age; rr:objectMap ex:om .
ex:age a owl:DatatypeProperty .
ex:om rr:datatype xsd:string .
ex:owner1 sh:property ex:propertyShape .
ex:owner2 sh:property ex:propertyShape .
ex:propertyShape sh:path ex:age, ex:other; sh:datatype xsd:integer .
"#,
        GateOutcome {
            violations: 2,
            warnings: 0,
        },
    );
    assert_differential(
        "violation and independent warning severities",
        &format!(
            "{base}\nex:om rr:datatype xsd:string .\n\
                 ex:Entity a owl:Class; mf:stereotype mf:Entity ."
        ),
        GateOutcome {
            violations: 1,
            warnings: 1,
        },
    );
}

#[test]
fn datatype_violation_is_not_double_counted_by_native_and_batch_paths() {
    assert_differential(
        "single datatype mismatch",
        r#"
ex:pom rr:predicate ex:age; rr:objectMap ex:om .
ex:age a owl:DatatypeProperty .
ex:om rr:datatype xsd:string .
ex:shape sh:property ex:propertyShape .
ex:propertyShape sh:path ex:age; sh:datatype xsd:integer .
"#,
        GateOutcome {
            violations: 1,
            warnings: 0,
        },
    );
}

#[test]
fn literal_predicate_and_object_map_preserve_reference_counts() {
    assert_differential(
        "literal predicate and object map",
        r#"
ex:pom rr:predicate "age"; rr:objectMap "literal object map" .
ex:shape sh:property ex:propertyShape .
ex:propertyShape sh:path "age"; sh:datatype xsd:integer .
"#,
        GateOutcome {
            violations: 2,
            warnings: 0,
        },
    );
}

#[test]
fn core_or_shape_preserves_one_result_per_invalid_focus_node() {
    assert_differential(
        "two invalid predicate targets",
        r#"
ex:pom1 rr:predicate ex:missing1 .
ex:pom2 rml:predicate ex:missing2 .
"#,
        GateOutcome {
            violations: 2,
            warnings: 0,
        },
    );
}

#[test]
fn blank_node_focus_preserves_the_old_fail_closed_boundary() {
    let input = format!(
        "{PREFIXES}\n\
             _:pom rr:predicate ex:age; rr:objectMap _:om .\n\
             ex:age a owl:DatatypeProperty .\n\
             _:om rr:datatype xsd:string .\n\
             ex:shape sh:property ex:propertyShape .\n\
             ex:propertyShape sh:path ex:age; sh:datatype xsd:integer ."
    );

    // This is the asymmetry motivating the public compatibility guard:
    // the global query is valid, whereas rudof's generated per-focus
    // `VALUES ?this { _:... }` query is not accepted by its parser.
    assert_eq!(direct_batch_count(&input), 1);
    assert!(per_focus_reference_result(&input).is_err());
    assert_eq!(
        crate::validate_turtle(&input),
        Err(GateError::ValidationFailed)
    );
}

#[test]
fn blank_object_map_and_shape_nodes_do_not_trigger_the_focus_guard() {
    assert_differential(
        "IRI focus with blank object map and shape nodes",
        r#"
ex:pom rr:predicate ex:age; rr:objectMap _:om .
ex:age a owl:DatatypeProperty .
_:om rr:datatype xsd:string .
_:nodeShape a sh:NodeShape; sh:property _:propertyShape .
_:propertyShape a sh:PropertyShape; sh:path ex:age; sh:datatype xsd:integer .
"#,
        GateOutcome {
            violations: 1,
            warnings: 0,
        },
    );
}

#[test]
fn sealed_schema_contract_guards_query_shapes_and_deactivation() {
    let schema = batch_schema();
    let selected = select(&schema).unwrap();
    assert_eq!(<[u8; 32]>::from(Sha256::digest(selected)), SELECT_DIGEST);

    let active_datatype = IRSchema::from_str(
        META_SHAPES_TTL,
        &rudof_rdf::rdf_core::RDFFormat::Turtle,
        None,
        &rudof_rdf::rdf_impl::ReaderMode::Strict,
    )
    .unwrap();
    assert_eq!(select(&active_datatype), Err(GateError::InvalidShapeSet));

    let core_deactivated = format!(
        "{META_SHAPES_TTL}\n<{SHAPE_IRI}> \
         <http://www.w3.org/ns/shacl#deactivated> true .\n\
         <{CLASS_SHAPE_IRI}> <http://www.w3.org/ns/shacl#deactivated> true .\n"
    );
    let core_deactivated = IRSchema::from_str(
        &core_deactivated,
        &rudof_rdf::rdf_core::RDFFormat::Turtle,
        None,
        &rudof_rdf::rdf_impl::ReaderMode::Strict,
    )
    .unwrap();
    assert_eq!(select(&core_deactivated), Err(GateError::InvalidShapeSet));

    let changed_query =
        META_SHAPES_TTL.replace("FILTER(!BOUND(?md) || ?md != ?td)", "FILTER(!BOUND(?md))");
    let changed_query = format!(
        "{changed_query}\n<{SHAPE_IRI}> \
             <http://www.w3.org/ns/shacl#deactivated> true .\n"
    );
    let changed_query = IRSchema::from_str(
        &changed_query,
        &rudof_rdf::rdf_core::RDFFormat::Turtle,
        None,
        &rudof_rdf::rdf_impl::ReaderMode::Strict,
    )
    .unwrap();
    assert_eq!(select(&changed_query), Err(GateError::InvalidShapeSet));

    let extra_target = format!(
        "{META_SHAPES_TTL}\n<{SHAPE_IRI}> \
             <http://www.w3.org/ns/shacl#deactivated> true .\n\
             <urn:extra> a <http://www.w3.org/ns/shacl#NodeShape>; \
             <http://www.w3.org/ns/shacl#targetNode> <urn:focus> .\n"
    );
    let extra_target = IRSchema::from_str(
        &extra_target,
        &rudof_rdf::rdf_core::RDFFormat::Turtle,
        None,
        &rudof_rdf::rdf_impl::ReaderMode::Strict,
    )
    .unwrap();
    assert_eq!(select(&extra_target), Err(GateError::InvalidShapeSet));

    let extra_sparql = format!(
        "{META_SHAPES_TTL}\n<{SHAPE_IRI}> \
             <http://www.w3.org/ns/shacl#deactivated> true .\n\
         <urn:extra-property> a <http://www.w3.org/ns/shacl#PropertyShape>; \
             <http://www.w3.org/ns/shacl#path> <urn:path>; \
             <http://www.w3.org/ns/shacl#sparql> [ \
             <http://www.w3.org/ns/shacl#select> \
             \"SELECT ?this WHERE {{ ?this <urn:path> ?value }}\" ] ."
    );
    let extra_sparql = IRSchema::from_str(
        &extra_sparql,
        &rudof_rdf::rdf_core::RDFFormat::Turtle,
        None,
        &rudof_rdf::rdf_impl::ReaderMode::Strict,
    )
    .unwrap();
    assert_eq!(select(&extra_sparql), Err(GateError::InvalidShapeSet));
}

#[test]
fn batched_rule_preserves_sparql_bag_cardinality() {
    assert_differential(
        "join multiplicity",
        r#"
ex:pom rr:predicate ex:value; rr:objectMap ex:o1, ex:o2 .
ex:value a owl:DatatypeProperty .
ex:o1 rr:datatype xsd:string, xsd:integer .
ex:owner1 sh:property ex:ps1 .
ex:owner2 sh:property ex:ps1 .
ex:ps1 sh:path ex:value; sh:datatype xsd:integer, xsd:decimal .
ex:owner3 sh:property ex:ps2 .
ex:ps2 sh:path ex:value; sh:datatype xsd:string .
"#,
        GateOutcome {
            violations: 12,
            warnings: 0,
        },
    );
}

#[test]
fn candidate_underestimate_fails_closed() {
    let input = format!(
        "{PREFIXES}\n\
             ex:pom rr:predicate ex:value; rr:objectMap ex:om .\n\
             ex:om rr:datatype xsd:string .\n\
             ex:shape sh:property ex:ps .\n\
             ex:ps sh:path ex:value; sh:datatype xsd:integer ."
    );
    let mut data = rdf_data(&graph(&input));
    data.check_store().unwrap();
    assert_eq!(
        violation_count(
            &data,
            r#"SELECT ?this WHERE {
                    ?this <http://www.w3.org/ns/r2rml#predicate> ?predicate .
                }"#,
            0,
            usize::MAX,
        ),
        Err(GateError::ValidationFailed)
    );
    assert_eq!(
        violation_count(&data, "SELECT ?this WHERE {}", 1, 0),
        Err(GateError::ValidationResultLimit)
    );
}

#[test]
fn logical_work_charges_every_batched_focus_without_join_candidates() {
    let mut input = PREFIXES.to_owned();
    for index in 0..128 {
        input.push_str(&format!(
            "<urn:pom:{index}> rr:predicate ex:unconstrained .\n"
        ));
    }
    let graph = graph(&input);
    let workload = crate::preflight::admit(
        &graph,
        crate::ValidationLimits {
            max_work_units: usize::MAX,
            max_result_cardinality: usize::MAX,
        },
    )
    .unwrap();
    assert_eq!(workload.datatype_focus_nodes, 128);
    assert_eq!(workload.datatype_candidates, 0);
    assert!(workload.work_units >= graph.len() + 128);
}
