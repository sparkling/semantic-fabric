//! The cross-repo `M ⋈ T` SHACL gate (ADR-0005 / ADR-0019): validate the
//! `M ⋈ T` closure (the source-mapping graph joined with the generated model `T`)
//! against the mapping-authoring layer's mapping-output validation shapes —
//! `mf:MappingClassConformanceShape`, `mf:MappingPredicateConformanceShape`,
//! `mf:MappingDatatypeConformanceShape`, `mf:EntitySubjectGroundingShape`.
//!
//! Runner = rudof's `shacl` crate in **`ShaclValidationMode::Native`** (pure
//! Rust). Its `sparql` feature is on by default, so Native is pinned explicitly
//! (ADR-0019 — keeps the property-pair `validate_sparql()` panic path
//! unreachable). The in-memory graph is `oxrdf`-native via `rudof_rdf`, so no
//! second RDF stack enters the engine (ADR-0005).

use rudof_rdf::rdf_core::RDFFormat;
use rudof_rdf::rdf_impl::ReaderMode;
use shacl::ir::IRSchema;
use shacl::rdf::ShaclParser;
use shacl::types::Severity;
use shacl::validator::processor::{DataValidation, ShaclProcessor};
use shacl::validator::ShaclValidationMode;
use sparql_service::RdfData;

/// Outcome of validating an `M ⋈ T` closure against the meta-shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateOutcome {
    /// No `sh:Violation` (the gate's pass condition; warnings are advisory —
    /// `mf:EntitySubjectGroundingShape` runs at `sh:Warning` during the
    /// subject-backfill window).
    pub conforms: bool,
    pub violations: usize,
    pub warnings: usize,
}

/// Validate the `data` graph (the `M ⋈ T` closure, Turtle) against `shapes`
/// (Turtle) in Native mode. The gate **passes** iff there are no `sh:Violation`
/// results.
pub fn validate(data_ttl: &str, shapes_ttl: &str) -> Result<GateOutcome, String> {
    let data = RdfData::from_str(data_ttl, &RDFFormat::Turtle, None, &ReaderMode::Strict)
        .map_err(|e| format!("load M⋈T data graph: {e}"))?;
    let shapes = RdfData::from_str(shapes_ttl, &RDFFormat::Turtle, None, &ReaderMode::Strict)
        .map_err(|e| format!("load meta-shapes: {e}"))?;

    let schema = ShaclParser::new(shapes)
        .parse()
        .map_err(|e| format!("parse SHACL shapes: {e}"))?;
    let schema_ir: IRSchema = schema
        .try_into()
        .map_err(|e| format!("compile SHACL IR: {e}"))?;

    let mut validator: DataValidation = data.into();
    let report = validator
        .validate(&schema_ir, &ShaclValidationMode::Native)
        .map_err(|e| format!("SHACL Native validation: {e}"))?;

    let violations = report.get_count_of(&Severity::Violation);
    let warnings = report.get_count_of(&Severity::Warning);
    Ok(GateOutcome {
        conforms: violations == 0,
        violations,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // The two SHACL-Core existence half-shapes, reproduced here for
    // a self-contained, deterministic gate test. The vendored full meta-shapes
    // (with the sh:sparql datatype shape and the inverse-path grounding shape)
    // live in tests/m-join-t/ for the cross-repo run.
    const CORE_SHAPES: &str = r#"
@prefix sh:  <http://www.w3.org/ns/shacl#> .
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix mf:  <http://example.org/mapping-fabric#> .
mf:MappingClassConformanceShape a sh:NodeShape ;
    sh:targetObjectsOf rr:class ;
    sh:class owl:Class .
mf:MappingPredicateConformanceShape a sh:NodeShape ;
    sh:targetObjectsOf rr:predicate ;
    sh:or ( [ sh:class owl:ObjectProperty ] [ sh:class owl:DatatypeProperty ]
            [ sh:class owl:AnnotationProperty ] [ sh:class rdf:Property ] ) .
"#;

    #[test]
    fn conforming_closure_passes_the_gate() {
        let data = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex:  <http://ex/> .
ex:map1 rr:class ex:Person .
ex:Person a owl:Class .
ex:map2 rr:predicate ex:name .
ex:name a owl:DatatypeProperty .
"#;
        let out = validate(data, CORE_SHAPES).unwrap();
        assert!(out.conforms, "{out:?}");
        assert_eq!(out.violations, 0);
    }

    #[test]
    fn dangling_target_class_violates_the_gate() {
        // ex:Ghost is named as an rr:class target but is not declared a class in T.
        let data = r#"
@prefix rr:  <http://www.w3.org/ns/r2rml#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix ex:  <http://ex/> .
ex:map1 rr:class ex:Ghost .
"#;
        let out = validate(data, CORE_SHAPES).unwrap();
        assert!(
            !out.conforms,
            "dangling rr:class must be a Violation: {out:?}"
        );
        assert!(out.violations >= 1);
    }
}
