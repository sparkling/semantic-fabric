//! Bounded validation for the semantic-fabric `M ⋈ T` admission gate.
//!
//! This product crate contains no source connector, query engine, HTTP server,
//! or evidence harness. It owns the sealed SHACL shape set and validates a
//! caller-built RDF closure. Three Core shapes run in rudof Native mode; the
//! reviewed datatype SPARQL rule runs once as a digest-pinned batch over the
//! same store. The serving and conformance crates therefore exercise exactly
//! the same validator without depending on each other.

mod batched_datatype;
mod gate;
mod policy;
mod preflight;

pub use gate::{
    parse_turtle_graph, shape_set_digest, validate_graph, validate_turtle,
    validation_policy_digest, GateError, GateOutcome, GraphLimits, ValidationLimits,
    DEFAULT_GRAPH_LIMITS, DEFAULT_VALIDATION_LIMITS, META_SHAPES_TTL,
};
