use super::*;

use sf_core::ir::{LogicalSource, SubjectMap, TermMap, TriplesMap};
use sf_core::{NamedNode as CoreNamedNode, SourceId, SourceMapping, Term};
use sha2::{Digest, Sha256};

const G1: &str = "http://graphs.test/one";
const G2: &str = "http://graphs.test/two";
const G3: &str = "http://graphs.test/three";

fn mapping(source_id: SourceId, table: &str) -> SourceMapping {
    SourceMapping::new(
        source_id,
        vec![TriplesMap {
            id: "http://example.test/map/items".to_owned(),
            source: LogicalSource::Table(table.to_owned()),
            subject: SubjectMap {
                term: TermMap::Constant(Term::NamedNode(CoreNamedNode::new_unchecked(
                    "http://example.test/item",
                ))),
                classes: Vec::new(),
                graphs: Vec::new(),
            },
            predicate_object_maps: Vec::new(),
        }],
    )
}

fn mapping_digest(table: &str) -> MappingDigest {
    let source_id = SourceId::new(0).unwrap();
    MappingDigest::from_mapping(&mapping(source_id, table))
}

fn allow(iris: &[&str]) -> PinnedGraphAllowlist {
    PinnedGraphAllowlist::new(iris.iter().copied()).unwrap()
}

fn pins(table: &str, ontology: u8, admission: u8, iris: &[&str]) -> GeneratedProfileIdentity {
    mint(
        mapping_digest(table),
        OntologyDigest::from_sha256([ontology; 32]),
        SemanticAdmissionDigest::from_sha256([admission; 32]),
        &ProfileDescriptor::v1(),
        &allow(iris),
    )
}

fn baseline() -> GeneratedProfileIdentity {
    pins("items", 0x11, 0x22, &[G1, G2])
}

fn mint_profile(profile: &ProfileDescriptor) -> GeneratedProfileIdentity {
    mint(
        mapping_digest("items"),
        OntologyDigest::from_sha256([0x11; 32]),
        SemanticAdmissionDigest::from_sha256([0x22; 32]),
        profile,
        &allow(&[G1, G2]),
    )
}

fn framed(out: &mut Vec<u8>, bytes: &[u8]) {
    let len = u64::try_from(bytes.len()).unwrap();
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(bytes);
}

#[test]
fn golden_wire_matches_independently_built_preimage() {
    let mut preimage = Vec::new();
    framed(
        &mut preimage,
        b"semantic-fabric/generated-profile-identity/v1",
    );
    framed(&mut preimage, b"generated-query-profile/v1");
    framed(&mut preimage, b"forms/select-ask-only/v1");
    framed(&mut preimage, b"service/refuse/v1");
    framed(&mut preimage, b"dataset/pinned-graph-allowlist/v1");
    framed(&mut preimage, b"coverage/pinned-mapping-constants/v1");
    framed(&mut preimage, mapping_digest("items").as_bytes());
    framed(&mut preimage, &[0x11; 32]);
    framed(&mut preimage, &[0x22; 32]);
    preimage.extend_from_slice(&2u64.to_be_bytes());
    framed(&mut preimage, b"http://graphs.test/one");
    framed(&mut preimage, b"http://graphs.test/two");

    let mut expected = String::from("sfgp1:");
    for byte in Sha256::digest(&preimage).iter() {
        expected.push_str(&format!("{byte:02x}"));
    }
    let wire = baseline().wire();
    assert_eq!(wire, expected);
    assert_eq!(wire.len(), 70);
    assert!(wire.is_ascii());
}

#[test]
fn reconstructed_pins_are_stable_across_independent_objects() {
    let first = baseline();
    let second = pins("items", 0x11, 0x22, &[G1, G2]);
    assert_eq!(first, second);
    assert_eq!(first.wire(), second.wire());
    assert_eq!(mint_profile(&ProfileDescriptor::v1()), first);
}

#[test]
fn every_pinned_input_changes_identity() {
    let reference = baseline();
    let source_id = SourceId::new(1).unwrap();
    let moved = MappingDigest::from_mapping(&mapping(source_id, "items"));
    let other_source = mint(
        moved,
        OntologyDigest::from_sha256([0x11; 32]),
        SemanticAdmissionDigest::from_sha256([0x22; 32]),
        &ProfileDescriptor::v1(),
        &allow(&[G1, G2]),
    );
    let variants = [
        pins("other", 0x11, 0x22, &[G1, G2]),
        pins("items", 0x12, 0x22, &[G1, G2]),
        pins("items", 0x11, 0x23, &[G1, G2]),
        pins("items", 0x11, 0x22, &[G1]),
        pins("items", 0x11, 0x22, &[G1, G3]),
        pins("items", 0x11, 0x22, &[]),
        other_source,
    ];
    for (index, variant) in variants.iter().enumerate() {
        assert_ne!(*variant, reference, "variant {index}");
    }
}

#[test]
fn each_profile_rule_changes_identity() {
    let base = ProfileDescriptor::v1();
    let reference = mint_profile(&base);
    let descriptors = [
        ProfileDescriptor {
            version: ProfileVersion::TestAlternate,
            ..base
        },
        ProfileDescriptor {
            form: FormRules::TestAlternate,
            ..base
        },
        ProfileDescriptor {
            service: ServicePolicy::TestAlternate,
            ..base
        },
        ProfileDescriptor {
            dataset: DatasetPolicy::TestAlternate,
            ..base
        },
        ProfileDescriptor {
            coverage: CoveragePolicy::TestAlternate,
            ..base
        },
    ];
    for (index, descriptor) in descriptors.iter().enumerate() {
        assert_ne!(mint_profile(descriptor), reference, "descriptor {index}");
    }
}

#[test]
fn allowlist_order_does_not_change_identity() {
    assert_eq!(
        pins("items", 0x11, 0x22, &[G1, G2]),
        pins("items", 0x11, 0x22, &[G2, G1])
    );
    assert_eq!(
        pins("items", 0x11, 0x22, &[G3, G1, G2]),
        pins("items", 0x11, 0x22, &[G2, G3, G1])
    );
}

#[test]
fn ambiguous_concatenations_differ() {
    let split = pins("items", 0x11, 0x22, &["urn:a", "urn:b"]);
    let joined = pins("items", 0x11, 0x22, &["urn:aurn:b"]);
    let single = pins("items", 0x11, 0x22, &["urn:a"]);
    let empty = pins("items", 0x11, 0x22, &[]);
    assert_ne!(split, joined);
    assert_ne!(split, single);
    assert_ne!(joined, single);
    assert_ne!(empty, single);
}

#[test]
fn wire_roundtrips_to_comparison_claim_only() {
    let identity = baseline();
    let claim = GeneratedProfileClaim::parse(&identity.wire()).unwrap();
    assert!(claim.matches(&identity));
    assert_eq!(claim, identity.claim());
    let other = pins("other", 0x11, 0x22, &[G1, G2]);
    assert!(!claim.matches(&other));
}

#[test]
fn malformed_wire_is_refused() {
    let good = baseline().wire();
    let hex = &good[WIRE_PREFIX.len()..];
    let cases = [
        String::new(),
        "sfgp1:".to_owned(),
        hex.to_owned(),
        format!("sfgp2:{hex}"),
        format!("SFGP1:{hex}"),
        format!(" {good}"),
        format!("{good} "),
        format!("{good}0"),
        format!("sfgp1:{}", &hex[1..]),
        format!("sfgp1:{}", hex.to_uppercase()),
        format!("sfgp1:g{}", &hex[1..]),
        format!("sfgp1:{}", "é".repeat(32)),
        format!("sfgp1:{}", "é".repeat(31)),
    ];
    for case in cases {
        assert_eq!(
            GeneratedProfileClaim::parse(&case).unwrap_err(),
            IdentityError::MalformedWire,
            "{case:?}"
        );
    }
}

#[test]
fn invalid_and_duplicate_graphs_are_refused_without_disclosure() {
    for bad in [
        "",
        "secret-relative",
        "/secret/path",
        "http://secret.test/a b",
    ] {
        let error = PinnedGraphAllowlist::new([G1, bad]).unwrap_err();
        assert_eq!(error, IdentityError::InvalidGraphIri);
        assert!(!format!("{error} {error:?}").contains("secret"));
    }
    let dup = "http://secret.test/graph";
    let error = PinnedGraphAllowlist::new([dup, G1, dup]).unwrap_err();
    assert_eq!(error, IdentityError::DuplicateGraph);
    assert!(!format!("{error} {error:?}").contains("secret"));
}

#[test]
fn oversized_inputs_are_refused_prospectively() {
    let many: Vec<String> = (0..=MAX_GRAPHS)
        .map(|index| format!("http://graphs.test/{index}"))
        .collect();
    let error = PinnedGraphAllowlist::new(&many).unwrap_err();
    assert_eq!(error, IdentityError::TooManyGraphs);
    assert!(PinnedGraphAllowlist::new(&many[..MAX_GRAPHS]).is_ok());

    let endless = std::iter::repeat(G1);
    let error = PinnedGraphAllowlist::new(endless).unwrap_err();
    assert_eq!(error, IdentityError::TooManyGraphs);

    let padding = "a".repeat(MAX_GRAPH_IRI_BYTES);
    let long = format!("http://graphs.test/{padding}");
    let error = PinnedGraphAllowlist::new([long]).unwrap_err();
    assert_eq!(error, IdentityError::GraphTooLong);

    let sized = |index: usize| {
        let mut iri = format!("http://e.test/{index:02}/");
        iri.push_str(&"a".repeat(MAX_GRAPH_IRI_BYTES - iri.len()));
        iri
    };
    let fitting = MAX_GRAPH_TOTAL_BYTES / MAX_GRAPH_IRI_BYTES;
    let fits: Vec<String> = (0..fitting).map(sized).collect();
    assert!(PinnedGraphAllowlist::new(&fits).is_ok());
    let over: Vec<String> = (0..=fitting).map(sized).collect();
    let error = PinnedGraphAllowlist::new(&over).unwrap_err();
    assert_eq!(error, IdentityError::TotalBytesExceeded);
}

#[test]
fn debug_output_does_not_disclose_identity_or_graphs() {
    let identity = baseline();
    let wire = identity.wire();
    let list = allow(&["http://secret.test/private-graph"]);
    let rendered = format!(
        "{identity:?} {:?} {list:?} {:?}",
        identity.claim(),
        ProfileDescriptor::v1()
    );
    assert!(!rendered.contains(&wire[WIRE_PREFIX.len()..]));
    assert!(!rendered.contains("secret"));
    assert!(!rendered.contains("private-graph"));
}

#[test]
fn identity_depends_only_on_pinned_inputs() {
    let first = baseline();
    let noise: Vec<Box<u64>> = (0..64).map(Box::new).collect();
    let _clock = std::time::SystemTime::now();
    let second = std::thread::spawn(baseline).join().unwrap();
    drop(noise);
    assert_eq!(first, second);
    assert_eq!(first.wire(), second.wire());
}
