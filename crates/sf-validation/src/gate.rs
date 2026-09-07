use std::panic::{catch_unwind, AssertUnwindSafe};

use oxrdf::Graph;
use oxttl::TurtleParser;
use rudof_rdf::rdf_impl::OxigraphInMemory;
use sha2::{Digest, Sha256};
use shacl::ir::IRSchema;
use shacl::rdf::ShaclParser;
use shacl::types::Severity;
use shacl::validator::processor::{DataValidation, ShaclProcessor};
use shacl::validator::ShaclValidationMode;
use sparql_service::RdfData;

/// The exact, embedded mapping-output shapes used by every product and harness
/// call. Callers cannot replace or deactivate this set.
pub const META_SHAPES_TTL: &str = include_str!("../resources/meta-shapes.ttl");

// The batch rewrite is valid only for these reviewed bytes.  A shape change
// must deliberately update both this pin and the differential evidence.
pub(crate) const REVIEWED_SHAPE_SET_DIGEST: [u8; 32] = [
    0x88, 0x4c, 0xee, 0x08, 0xa9, 0xad, 0x9e, 0xd1, 0xe8, 0xe3, 0x03, 0x57, 0xa9, 0x1d, 0x98, 0x6b,
    0xdb, 0x1d, 0x5f, 0x2b, 0xe7, 0xed, 0x4f, 0xc4, 0x25, 0xb1, 0x3a, 0x90, 0x60, 0xb8, 0x47, 0xf9,
];
const SH_DEACTIVATED: &str = "http://www.w3.org/ns/shacl#deactivated";

/// Default input envelope. It comfortably covers the 2026 product-mock gold
/// while bounding parse memory and Native/SPARQL validation work at startup.
pub const DEFAULT_GRAPH_LIMITS: GraphLimits = crate::policy::graph_limits();

/// Fixed logical-work and prospective-report bounds for the sealed shapes.
pub const DEFAULT_VALIDATION_LIMITS: ValidationLimits = crate::policy::validation_limits();

/// Fixed limits applied before a graph can enter semantic admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GraphLimits {
    pub max_utf8_bytes: usize,
    pub max_parsed_triples: usize,
}

/// Limits for the deterministic preflight that runs before Native validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidationLimits {
    pub max_work_units: usize,
    pub max_result_cardinality: usize,
}

/// Redacted result of running the sealed four-shape gate.
///
/// The severity counts are faithful to the sealed rules' SPARQL bag
/// cardinality. The result is intentionally report-lossy: focus nodes, paths,
/// values, messages, and engine diagnostics are not exposed or retained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GateOutcome {
    pub violations: usize,
    pub warnings: usize,
}

impl GateOutcome {
    pub const fn conforms(self) -> bool {
        self.violations == 0
    }
}

/// A bounded, redacted gate failure. User RDF and SHACL engine diagnostics are
/// intentionally not retained because they may contain schema names or values.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum GateError {
    #[error("semantic graph exceeds its byte limit")]
    ByteLimit,
    #[error("semantic graph exceeds its triple limit")]
    TripleLimit,
    #[error("semantic validation exceeds its logical-work limit")]
    ValidationWorkLimit,
    #[error("semantic validation exceeds its result-cardinality limit")]
    ValidationResultLimit,
    #[error("semantic graph is not valid Turtle")]
    InvalidTurtle,
    #[error("sealed semantic shape set is invalid")]
    InvalidShapeSet,
    #[error("semantic validation failed")]
    ValidationFailed,
    #[error("semantic validation panicked")]
    ValidationPanicked,
}

/// Parse a bounded Turtle document into a set-valued RDF graph.
pub fn parse_turtle_graph(input: &str, limits: GraphLimits) -> Result<Graph, GateError> {
    if input.len() > limits.max_utf8_bytes {
        return Err(GateError::ByteLimit);
    }
    let mut graph = Graph::new();
    let mut parsed = 0usize;
    for triple in TurtleParser::new().for_slice(input) {
        parsed = parsed.checked_add(1).ok_or(GateError::TripleLimit)?;
        if parsed > limits.max_parsed_triples {
            return Err(GateError::TripleLimit);
        }
        graph.insert(&triple.map_err(|_| GateError::InvalidTurtle)?);
    }
    Ok(graph)
}

/// Validate one already-bounded closure graph against the sealed shape set.
pub fn validate_graph(graph: &Graph) -> Result<GateOutcome, GateError> {
    let graph_limits = crate::policy::graph_limits();
    let validation_limits = crate::policy::validation_limits();
    if graph.len() > graph_limits.max_parsed_triples {
        return Err(GateError::TripleLimit);
    }
    let workload = crate::preflight::admit(graph, validation_limits)?;
    // Preserve rudof 0.3.14's fail-closed behavior for raw blank-node POM
    // focus nodes. Product admission never hits this path because sf-mapping
    // emits an IRI-skolemised projection.
    crate::policy::enforce_blank_datatype_focus(workload.has_blank_datatype_focus)?;
    catch_unwind(AssertUnwindSafe(|| validate_graph_inner(graph, workload)))
        .map_err(|_| GateError::ValidationPanicked)?
}

/// Convenience entry point used by hermetic conformance fixtures.
pub fn validate_turtle(input: &str) -> Result<GateOutcome, GateError> {
    let graph = parse_turtle_graph(input, crate::policy::graph_limits())?;
    validate_graph(&graph)
}

/// SHA-256 of the exact embedded shape bytes used by the policy identity.
///
/// This identifies only the shape document. Admission receipts must bind
/// [`validation_policy_digest`] so the execution policy cannot be detached.
pub fn shape_set_digest() -> [u8; 32] {
    Sha256::digest(META_SHAPES_TTL.as_bytes()).into()
}

/// SHA-256 identity of the sealed shapes and the exact execution strategy.
///
/// Admission receipts bind this digest so changing the shape/query bytes,
/// Native/global topology, evaluator and parser identities/features, limits,
/// preflight revision, or blank-focus policy changes runtime identity.
pub fn validation_policy_digest() -> Result<[u8; 32], GateError> {
    crate::policy::digest(shape_set_digest())
}

fn validate_graph_inner(
    graph: &Graph,
    workload: crate::preflight::ValidationWorkload,
) -> Result<GateOutcome, GateError> {
    crate::policy::verify_shape_digest(shape_set_digest())?;
    // Retain the reviewed document as receipt identity, but deactivate its one
    // SPARQL shape for rudof: that exact rule is evaluated once below.  The
    // remaining three Core shapes still execute through rudof Native.
    let core_shapes = format!(
        "{META_SHAPES_TTL}\n<{}> <{SH_DEACTIVATED}> true .\n",
        crate::batched_datatype::SHAPE_IRI
    );
    let shapes = RdfData::from_str(
        &core_shapes,
        &rudof_rdf::rdf_core::RDFFormat::Turtle,
        None,
        &rudof_rdf::rdf_impl::ReaderMode::Strict,
    )
    .map_err(|_| GateError::InvalidShapeSet)?;
    let schema = ShaclParser::new(shapes)
        .parse()
        .map_err(|_| GateError::InvalidShapeSet)?;
    let schema_ir: IRSchema = schema.try_into().map_err(|_| GateError::InvalidShapeSet)?;
    let datatype_select = crate::batched_datatype::select(&schema_ir)?;

    let mut data = OxigraphInMemory::new();
    for triple in graph.iter() {
        data.add_triple_ref(triple.subject, triple.predicate, triple.object)
            .map_err(|_| GateError::ValidationFailed)?;
    }
    let mut rdf_data = RdfData::from_graph(data).map_err(|_| GateError::ValidationFailed)?;
    rdf_data
        .check_store()
        .map_err(|_| GateError::ValidationFailed)?;
    let datatype_violations = crate::batched_datatype::violation_count(
        &rdf_data,
        datatype_select,
        workload.datatype_candidates,
        crate::policy::validation_limits().max_result_cardinality,
    )?;
    let mut validator: DataValidation = rdf_data.into();
    let report = validator
        .validate(&schema_ir, &ShaclValidationMode::Native)
        .map_err(|_| GateError::ValidationFailed)?;
    compose_outcome(
        report.get_count_of(&Severity::Violation),
        report.get_count_of(&Severity::Warning),
        datatype_violations,
        workload.result_cardinality,
    )
}

fn compose_outcome(
    native_violations: usize,
    native_warnings: usize,
    datatype_violations: usize,
    result_bound: usize,
) -> Result<GateOutcome, GateError> {
    let violations = native_violations
        .checked_add(datatype_violations)
        .ok_or(GateError::ValidationFailed)?;
    let result_count = violations
        .checked_add(native_warnings)
        .ok_or(GateError::ValidationFailed)?;
    if result_count > result_bound {
        return Err(GateError::ValidationFailed);
    }
    Ok(GateOutcome {
        violations,
        warnings: native_warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFORMING: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex:  <http://ex/> .
ex:subject rr:class ex:Person .
ex:pom rr:predicate ex:name .
ex:Person a owl:Class .
ex:name a owl:DatatypeProperty .
"#;

    const DATATYPE_PREFIXES: &str = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix sh:  <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix ex:  <http://ex/> .
ex:pom rr:predicate ex:age; rr:objectMap ex:om .
ex:age a owl:DatatypeProperty .
ex:nodeShape sh:property ex:propertyShape .
ex:propertyShape sh:path ex:age; sh:datatype xsd:integer .
"#;

    #[test]
    fn sealed_gate_accepts_a_conforming_closure() {
        let outcome = validate_turtle(CONFORMING).unwrap();
        assert!(outcome.conforms(), "{outcome:?}");
        assert_eq!(outcome.violations, 0);
    }

    #[test]
    fn sealed_shape_bytes_have_the_reviewed_digest() {
        assert_eq!(shape_set_digest(), REVIEWED_SHAPE_SET_DIGEST);
    }

    #[test]
    fn dangling_predicate_is_a_violation() {
        let input = r#"
@prefix rr: <http://www.w3.org/ns/r2rml#> .
@prefix ex: <http://ex/> .
ex:pom rr:predicate ex:notDeclared .
"#;
        let outcome = validate_turtle(input).unwrap();
        assert!(!outcome.conforms(), "{outcome:?}");
        assert!(outcome.violations >= 1);
    }

    #[test]
    fn sealed_gate_executes_the_sparql_datatype_constraint() {
        let matching = format!("{DATATYPE_PREFIXES}\nex:om rr:datatype xsd:integer .");
        let mismatching = format!("{DATATYPE_PREFIXES}\nex:om rr:datatype xsd:string .");
        let matching = validate_turtle(&matching).unwrap();
        let mismatching = validate_turtle(&mismatching).unwrap();
        assert!(matching.conforms(), "{matching:?}");
        assert!(!mismatching.conforms(), "{mismatching:?}");
        assert!(mismatching.violations >= 1);
    }

    #[test]
    fn constrained_predicate_rejects_an_object_without_an_effective_datatype() {
        let outcome = validate_turtle(DATATYPE_PREFIXES).unwrap();
        assert!(!outcome.conforms(), "{outcome:?}");
    }

    #[test]
    fn entity_grounding_is_an_advisory_warning_with_a_live_complex_path() {
        let ungrounded = r#"
@prefix mf:  <http://example.org/mapping-fabric#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex:  <http://ex/> .
ex:Person a owl:Class; mf:stereotype mf:Entity .
"#;
        let grounded = r#"
@prefix mf:  <http://example.org/mapping-fabric#> .
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex:  <http://ex/> .
ex:Person a owl:Class; mf:stereotype mf:Entity .
ex:subjectMap rr:class ex:Person .
"#;
        let ungrounded = validate_turtle(ungrounded).unwrap();
        let grounded = validate_turtle(grounded).unwrap();
        assert!(ungrounded.conforms(), "warnings do not authorize failure");
        assert!(ungrounded.warnings >= 1, "{ungrounded:?}");
        assert_eq!(grounded.warnings, 0, "{grounded:?}");
    }

    #[test]
    fn byte_limit_rejects_before_parsing() {
        let limits = GraphLimits {
            max_utf8_bytes: 3,
            max_parsed_triples: 1,
        };
        assert_eq!(
            parse_turtle_graph("<urn:s> <urn:p> <urn:o> .", limits),
            Err(GateError::ByteLimit)
        );
    }

    #[test]
    fn parsed_occurrences_are_bounded_before_graph_deduplication() {
        let limits = GraphLimits {
            max_utf8_bytes: 1024,
            max_parsed_triples: 1,
        };
        let duplicate = "<urn:s> <urn:p> <urn:o> .\n<urn:s> <urn:p> <urn:o> .";
        assert_eq!(
            parse_turtle_graph(duplicate, limits),
            Err(GateError::TripleLimit)
        );
    }

    #[test]
    fn malformed_input_is_redacted() {
        assert_eq!(
            validate_turtle("<postgres://secret@host/table> ["),
            Err(GateError::InvalidTurtle)
        );
        assert!(!GateError::InvalidTurtle.to_string().contains("secret"));
    }

    #[test]
    fn post_composition_overflow_and_bound_underestimate_fail_closed() {
        assert_eq!(
            compose_outcome(usize::MAX, 0, 1, usize::MAX),
            Err(GateError::ValidationFailed)
        );
        assert_eq!(
            compose_outcome(1, 1, 1, 2),
            Err(GateError::ValidationFailed)
        );
        assert_eq!(
            compose_outcome(1, 1, 1, 3),
            Ok(GateOutcome {
                violations: 2,
                warnings: 1,
            })
        );
    }
}
