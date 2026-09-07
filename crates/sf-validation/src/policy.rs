//! Single runtime authority for the sealed validation policy and its receipt identity.

use sha2::{Digest, Sha256};

use crate::gate::{GateError, GraphLimits, ValidationLimits, REVIEWED_SHAPE_SET_DIGEST};

const POLICY_DOMAIN: &[u8] = b"semantic-fabric/shacl-execution-policy/v2";

#[derive(Clone, Copy)]
enum BlankDatatypeFocusPolicy {
    RejectAsValidationFailed,
    #[cfg(test)]
    AllowGlobal,
}

impl BlankDatatypeFocusPolicy {
    const fn tag(self) -> &'static [u8] {
        match self {
            Self::RejectAsValidationFailed => b"reject-as-validation-failed/v1",
            #[cfg(test)]
            Self::AllowGlobal => b"allow-global/v1",
        }
    }
}

#[derive(Clone, Copy)]
struct EngineIdentities {
    shacl: &'static [u8],
    rudof_rdf: &'static [u8],
    sparql_service: &'static [u8],
    oxigraph: &'static [u8],
    spareval: &'static [u8],
    spargebra: &'static [u8],
    oxrdf: &'static [u8],
    oxttl: &'static [u8],
    oxrdfio: &'static [u8],
}

#[derive(Clone, Copy)]
struct ValidationPolicy {
    graph_limits: GraphLimits,
    validation_limits: ValidationLimits,
    preflight_revision: &'static [u8],
    blank_datatype_focus: BlankDatatypeFocusPolicy,
    execution_topology: &'static [u8],
    feature_profile: &'static [u8],
    engines: EngineIdentities,
}

const POLICY: ValidationPolicy = ValidationPolicy {
    graph_limits: GraphLimits {
        max_utf8_bytes: 32 * 1024 * 1024,
        max_parsed_triples: 250_000,
    },
    validation_limits: ValidationLimits {
        max_work_units: 1_000_000,
        max_result_cardinality: 250_000,
    },
    preflight_revision: b"sealed-four-shape-work-accounting/v2",
    blank_datatype_focus: BlankDatatypeFocusPolicy::RejectAsValidationFailed,
    execution_topology: b"core=three-rudof-native;datatype=one-global-parsed-sealed-select",
    // Derived from `cargo tree -p sf-validation --locked --offline -e normal
    // --prefix none --format '{p}\t{f}'`; workspace-wide feature unions are
    // not authoritative.
    feature_profile: b"shacl=default+sparql;rudof_rdf=default+sparql;sparql_service=default+sparql;oxigraph=http-client+http-client-rustls-native+oxhttp+rdf-12;spareval=calendar-ext+default+sep-0002+sep-0006+sparql-12;spargebra=default+sep-0002+sep-0006+sparql-12;oxrdf=default+oxsdatatypes+rdf-12+rdfc-10;oxttl=default+rdf-12;oxrdfio=default+rdf-12",
    engines: EngineIdentities {
        shacl: b"shacl/0.3.14",
        rudof_rdf: b"rudof_rdf/0.3.14",
        sparql_service: b"sparql_service/0.3.14",
        oxigraph: b"oxigraph/0.5.9",
        spareval: b"spareval/0.2.6",
        spargebra: b"spargebra/0.4.6",
        oxrdf: b"oxrdf/0.3.3",
        oxttl: b"oxttl/0.2.3",
        oxrdfio: b"oxrdfio/0.2.5",
    },
};

pub(crate) const fn graph_limits() -> GraphLimits {
    POLICY.graph_limits
}

pub(crate) const fn validation_limits() -> ValidationLimits {
    POLICY.validation_limits
}

pub(crate) fn enforce_blank_datatype_focus(has_blank_focus: bool) -> Result<(), GateError> {
    if !has_blank_focus {
        return Ok(());
    }
    match POLICY.blank_datatype_focus {
        BlankDatatypeFocusPolicy::RejectAsValidationFailed => Err(GateError::ValidationFailed),
        #[cfg(test)]
        BlankDatatypeFocusPolicy::AllowGlobal => Ok(()),
    }
}

pub(crate) fn digest(actual_shape_digest: [u8; 32]) -> Result<[u8; 32], GateError> {
    digest_policy(
        &POLICY,
        actual_shape_digest,
        crate::batched_datatype::SELECT_DIGEST,
    )
}

fn digest_policy(
    policy: &ValidationPolicy,
    shape_digest: [u8; 32],
    select_digest: [u8; 32],
) -> Result<[u8; 32], GateError> {
    let mut hasher = Sha256::new();
    for field in [
        POLICY_DOMAIN,
        shape_digest.as_slice(),
        select_digest.as_slice(),
        policy.execution_topology,
        policy.feature_profile,
        policy.engines.shacl,
        policy.engines.rudof_rdf,
        policy.engines.sparql_service,
        policy.engines.oxigraph,
        policy.engines.spareval,
        policy.engines.spargebra,
        policy.engines.oxrdf,
        policy.engines.oxttl,
        policy.engines.oxrdfio,
    ] {
        hash_field(&mut hasher, field)?;
    }
    for limit in [
        policy.graph_limits.max_utf8_bytes,
        policy.graph_limits.max_parsed_triples,
        policy.validation_limits.max_work_units,
        policy.validation_limits.max_result_cardinality,
    ] {
        hash_field(&mut hasher, &checked_u64(limit)?.to_be_bytes())?;
    }
    hash_field(&mut hasher, policy.preflight_revision)?;
    hash_field(&mut hasher, policy.blank_datatype_focus.tag())?;
    Ok(hasher.finalize().into())
}

fn checked_u64(value: usize) -> Result<u64, GateError> {
    u64::try_from(value).map_err(|_| GateError::InvalidShapeSet)
}

fn hash_field(hasher: &mut Sha256, value: &[u8]) -> Result<(), GateError> {
    hasher.update(checked_u64(value.len())?.to_be_bytes());
    hasher.update(value);
    Ok(())
}

pub(crate) fn verify_shape_digest(actual: [u8; 32]) -> Result<(), GateError> {
    if actual == REVIEWED_SHAPE_SET_DIGEST {
        Ok(())
    } else {
        Err(GateError::InvalidShapeSet)
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;
