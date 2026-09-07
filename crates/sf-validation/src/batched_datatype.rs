//! One-query evaluator for the sealed mapping-datatype SHACL constraint.
//!
//! The batch is count-equivalent to rudof's per-focus execution only for the
//! ground, IRI-skolemised mapping projection admitted by `sf-mapping`. Raw
//! blank-node predicate-mapping subjects (the POM focus nodes) are deliberately
//! rejected by the public gate: rudof 0.3.14 cannot parse its generated
//! blank-node `VALUES` clause, while the global query can. Accepting them here
//! would silently broaden `validate_turtle` semantics during a performance
//! rewrite.

use rudof_rdf::rdf_core::query::QueryRDF;
use rudof_rdf::rdf_core::term::Object;
use sha2::{Digest, Sha256};
use shacl::ir::{IRComponent, IRSchema, IRShape};
use shacl::types::Severity;
use sparql_service::RdfData;

use crate::GateError;

pub(crate) const SHAPE_IRI: &str =
    "http://example.org/mapping-fabric#MappingDatatypeConformanceShape";
const CLASS_SHAPE_IRI: &str = "http://example.org/mapping-fabric#MappingClassConformanceShape";
const PREDICATE_SHAPE_IRI: &str =
    "http://example.org/mapping-fabric#MappingPredicateConformanceShape";
const ENTITY_SHAPE_IRI: &str = "http://example.org/mapping-fabric#EntitySubjectGroundingShape";

// SHA-256 of the exact sh:select lexical form in the reviewed shape document.
// The batch executes only a query whose parsed component matches this pin.
pub(crate) const SELECT_DIGEST: [u8; 32] = [
    0xc5, 0x54, 0x67, 0x08, 0x7b, 0x33, 0x49, 0x05, 0xc4, 0x08, 0x78, 0xf7, 0xc8, 0xa4, 0x0c, 0xc8,
    0x05, 0x3f, 0xb2, 0x3a, 0x5d, 0x2f, 0xa8, 0xb8, 0x97, 0x30, 0xb9, 0xf1, 0xa6, 0x4d, 0x60, 0x14,
];
/// Obtain the query from the exact parsed shape instead of maintaining a
/// second query literal. The caller-added deactivation must be present so the
/// same component cannot also execute through rudof's per-focus loop.
pub(crate) fn select(schema: &IRSchema) -> Result<&str, GateError> {
    let mut target_count = 0usize;
    for (id, shape) in schema.iter_with_targets() {
        let Object::Iri(iri) = id else {
            return Err(GateError::InvalidShapeSet);
        };
        let (expected_deactivated, expected_severity) = match iri.as_str() {
            CLASS_SHAPE_IRI | PREDICATE_SHAPE_IRI => (false, Severity::Violation),
            SHAPE_IRI => (true, Severity::Violation),
            ENTITY_SHAPE_IRI => (false, Severity::Warning),
            _ => return Err(GateError::InvalidShapeSet),
        };
        if !matches!(shape, IRShape::NodeShape(_))
            || shape.deactivated() != expected_deactivated
            || shape.severity() != &expected_severity
        {
            return Err(GateError::InvalidShapeSet);
        }
        target_count += 1;
    }
    if target_count != 4 {
        return Err(GateError::InvalidShapeSet);
    }

    let mut shapes = schema
        .iter()
        .filter(|(id, _)| matches!(id, Object::Iri(iri) if iri.as_str() == SHAPE_IRI));
    let (_, shape) = shapes.next().ok_or(GateError::InvalidShapeSet)?;
    if shapes.next().is_some() || !shape.deactivated() || shape.severity() != &Severity::Violation {
        return Err(GateError::InvalidShapeSet);
    }
    let mut rules = shape.components().iter().filter_map(|component| {
        if let IRComponent::BasicSparql(rule) = component {
            Some(rule)
        } else {
            None
        }
    });
    let rule = rules.next().ok_or(GateError::InvalidShapeSet)?;
    if rules.next().is_some() {
        return Err(GateError::InvalidShapeSet);
    }
    let select = rule.select().as_str();
    if <[u8; 32]>::from(Sha256::digest(select.as_bytes())) != SELECT_DIGEST
        || schema
            .iter()
            .flat_map(|(_, shape)| shape.components())
            .filter(|component| matches!(component, IRComponent::BasicSparql(_)))
            .count()
            != 1
    {
        return Err(GateError::InvalidShapeSet);
    }
    Ok(select)
}

pub(crate) fn violation_count(
    data: &RdfData,
    select: &str,
    candidate_bound: usize,
    result_limit: usize,
) -> Result<usize, GateError> {
    if candidate_bound > result_limit {
        return Err(GateError::ValidationResultLimit);
    }
    let solutions = data
        .query_select(select)
        .map_err(|_| GateError::ValidationFailed)?;
    let count = solutions.iter().count();
    if count > candidate_bound {
        // The preflight is the allocation/work proof for this exact query.  An
        // underestimate is an implementation fault, not authority to continue.
        return Err(GateError::ValidationFailed);
    }
    Ok(count)
}

#[cfg(test)]
#[path = "batched_datatype_tests.rs"]
mod tests;
