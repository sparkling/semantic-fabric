//! Evidence-harness compatibility wrapper around the sealed product gate.

pub use sf_validation::GateOutcome;

/// Validate the `data` graph (the `M ⋈ T` closure, Turtle) against `shapes`
/// (Turtle) in Native mode. The gate **passes** iff there are no `sh:Violation`
/// results.
pub fn validate(data_ttl: &str, shapes_ttl: &str) -> Result<GateOutcome, String> {
    if shapes_ttl.as_bytes() != sf_validation::META_SHAPES_TTL.as_bytes() {
        return Err("the M⋈T shape set is sealed".to_owned());
    }
    sf_validation::validate_turtle(data_ttl).map_err(|error| error.to_string())
}
