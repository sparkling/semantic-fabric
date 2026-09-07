pub mod fixture;
pub mod observation;
pub mod protocol;
pub mod query;

use sf_conformance::supported_surface::{ManifestSeal, Surface};

pub fn query_seal() -> ManifestSeal {
    ManifestSeal {
        profile_id: "sqlite-public-query-supported-surface-v1".to_owned(),
        surface: Surface::SparqlQuery,
        case_count: 12,
        manifest_sha256: "74ad8a80bf538c58523659459424a4e7c4ce2d55cc99af20f0f728cf19365814"
            .to_owned(),
    }
}

pub fn protocol_seal() -> ManifestSeal {
    ManifestSeal {
        profile_id: "sqlite-public-protocol-supported-surface-v1".to_owned(),
        surface: Surface::SparqlProtocol,
        case_count: 25,
        manifest_sha256: "3c550754503523aaa2435870e7363a6a3b2b67042b2930375f4629e16ee15212"
            .to_owned(),
    }
}
