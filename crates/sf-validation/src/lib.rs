//! Bounded native validation for the semantic-fabric `M ⋈ T` admission gate.
//!
//! This product crate contains no source connector, query engine, HTTP server,
//! or evidence harness. It owns the sealed SHACL shape set and validates a
//! caller-built RDF closure in rudof Native mode. The serving and conformance
//! crates therefore exercise exactly the same validator without depending on
//! each other.

mod gate;
mod preflight;

pub use gate::{
    parse_turtle_graph, shape_set_digest, validate_graph, validate_turtle, GateError, GateOutcome,
    GraphLimits, ValidationLimits, DEFAULT_GRAPH_LIMITS, DEFAULT_VALIDATION_LIMITS,
    META_SHAPES_TTL,
};
