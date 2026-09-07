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
        manifest_sha256: "8710ada0f6fa5ec9a93c5d94336688a412352e789979894f1aed49378094ee62"
            .to_owned(),
    }
}

pub fn protocol_seal() -> ManifestSeal {
    ManifestSeal {
        profile_id: "sqlite-public-protocol-supported-surface-v1".to_owned(),
        surface: Surface::SparqlProtocol,
        case_count: 25,
        manifest_sha256: "e1d7545011a335100a6fbca59ecbd3e51ee229dbe93530bbd1b5557f6264f328"
            .to_owned(),
    }
}
