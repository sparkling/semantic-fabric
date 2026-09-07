use super::*;

const ALL_SHAPES: &str = r#"
@prefix mf:  <http://example.org/mapping-fabric#> .
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix sh:  <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix ex:  <http://ex/> .
ex:subject rr:class ex:Entity .
ex:pom rr:predicate ex:value; rr:objectMap ex:object .
ex:object rr:datatype xsd:string .
ex:nodeShape sh:property ex:propertyShape .
ex:propertyShape sh:path ex:value; sh:datatype xsd:string .
ex:Entity mf:stereotype mf:Entity .
"#;

fn all_shapes_graph() -> Graph {
    crate::parse_turtle_graph(ALL_SHAPES, crate::DEFAULT_GRAPH_LIMITS).unwrap()
}

#[test]
fn measures_every_sealed_shape_before_native_validation() {
    let workload = measure(&all_shapes_graph()).unwrap();

    assert_eq!(workload.class_targets, 1);
    assert_eq!(workload.predicate_targets, 1);
    assert_eq!(workload.datatype_focus_nodes, 1);
    assert_eq!(workload.datatype_mapping_bindings, 1);
    assert_eq!(workload.datatype_constraint_bindings, 1);
    assert_eq!(workload.datatype_candidates, 1);
    assert_eq!(workload.entity_targets, 1);
}

#[test]
fn missing_object_datatype_still_consumes_a_sparql_candidate() {
    let input = ALL_SHAPES.replace("ex:object rr:datatype xsd:string .", "");
    let graph = crate::parse_turtle_graph(&input, crate::DEFAULT_GRAPH_LIMITS).unwrap();

    assert_eq!(measure(&graph).unwrap().datatype_candidates, 1);
}

#[test]
fn exact_preflight_limits_are_admitted() {
    let graph = all_shapes_graph();
    let workload = measure(&graph).unwrap();
    assert_eq!(workload.work_units, 19);
    assert_eq!(workload.result_cardinality, 8);
    assert_eq!(
        admit(
            &graph,
            ValidationLimits {
                max_work_units: workload.work_units,
                max_result_cardinality: workload.result_cardinality,
            },
        ),
        Ok(workload)
    );
}

#[test]
fn n_plus_one_workload_is_rejected_with_a_closed_error() {
    let graph = all_shapes_graph();
    let workload = measure(&graph).unwrap();
    assert_eq!(
        admit(
            &graph,
            ValidationLimits {
                max_work_units: workload.work_units - 1,
                max_result_cardinality: usize::MAX,
            },
        ),
        Err(GateError::ValidationWorkLimit)
    );
    assert_eq!(
        GateError::ValidationWorkLimit.to_string(),
        "semantic validation exceeds its logical-work limit"
    );
}

#[test]
fn n_plus_one_result_cardinality_is_rejected_with_a_closed_error() {
    let graph = all_shapes_graph();
    let workload = measure(&graph).unwrap();
    assert_eq!(
        admit(
            &graph,
            ValidationLimits {
                max_work_units: usize::MAX,
                max_result_cardinality: workload.result_cardinality - 1,
            },
        ),
        Err(GateError::ValidationResultLimit)
    );
    assert_eq!(
        GateError::ValidationResultLimit.to_string(),
        "semantic validation exceeds its result-cardinality limit"
    );
}

#[test]
fn batch_work_accounts_for_large_intermediate_joins_without_results() {
    let mut input = String::from(
        "@prefix rr: <http://www.w3.org/ns/r2rml#> .\n\
             @prefix ex: <http://ex/> .\n",
    );
    for index in 0..32 {
        input.push_str(&format!("ex:pom rr:predicate <urn:p:{index}> .\n"));
        input.push_str(&format!("ex:pom rr:objectMap <urn:om:{index}> .\n"));
    }
    let graph = crate::parse_turtle_graph(&input, crate::DEFAULT_GRAPH_LIMITS).unwrap();
    let workload = measure(&graph).unwrap();

    assert_eq!(workload.datatype_mapping_bindings, 32 * 32);
    assert_eq!(workload.datatype_constraint_bindings, 0);
    assert_eq!(workload.datatype_candidates, 0);
    assert!(workload.work_units >= graph.len() + 1 + (32 * 32));

    let mut constraints = String::from(
        "@prefix sh: <http://www.w3.org/ns/shacl#> .\n\
             @prefix ex: <http://ex/> .\n",
    );
    for index in 0..16 {
        constraints.push_str(&format!(
            "<urn:owner:{index}> sh:property ex:propertyShape .\n\
                 ex:propertyShape sh:path <urn:path:{index}> .\n\
                 ex:propertyShape sh:datatype <urn:datatype:{index}> .\n"
        ));
    }
    let graph = crate::parse_turtle_graph(&constraints, crate::DEFAULT_GRAPH_LIMITS).unwrap();
    let workload = measure(&graph).unwrap();
    assert_eq!(workload.datatype_constraint_bindings, 16 * 16 * 16);
    assert_eq!(workload.datatype_candidates, 0);
    assert!(workload.work_units >= graph.len() + (16 * 16 * 16));
}

#[test]
fn arithmetic_overflow_fails_closed() {
    assert_eq!(
        checked_add(usize::MAX, 1, GateError::ValidationWorkLimit),
        Err(GateError::ValidationWorkLimit)
    );
    assert_eq!(
        checked_mul(usize::MAX, 2, GateError::ValidationResultLimit),
        Err(GateError::ValidationResultLimit)
    );
}
