use std::mem::size_of;

use sf_core::security_context::{
    PolicySnapshotId, RequestAttributesIdentity, SecurityContext, SecurityIdentityError,
    SubjectIdentity,
};

fn digest(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn context(policy: u8, subject: u8, attributes: u8) -> SecurityContext {
    SecurityContext::new(
        PolicySnapshotId::from_digest(digest(policy)).unwrap(),
        SubjectIdentity::from_digest(digest(subject)).unwrap(),
        RequestAttributesIdentity::from_digest(digest(attributes)).unwrap(),
    )
}

#[test]
fn every_identity_is_explicit_and_rejects_the_zero_sentinel() {
    assert_eq!(
        PolicySnapshotId::from_digest([0; 32]).unwrap_err(),
        SecurityIdentityError::EmptyPolicySnapshot
    );
    assert_eq!(
        SubjectIdentity::from_digest([0; 32]).unwrap_err(),
        SecurityIdentityError::EmptySubject
    );
    assert_eq!(
        RequestAttributesIdentity::from_digest([0; 32]).unwrap_err(),
        SecurityIdentityError::EmptyRequestAttributes
    );
}

#[test]
fn cache_identity_is_deterministic_and_partitions_every_request_dimension() {
    let baseline = context(1, 2, 3).cache_identity();

    assert_eq!(baseline, context(1, 2, 3).cache_identity());
    assert_ne!(baseline, context(4, 2, 3).cache_identity());
    assert_ne!(baseline, context(1, 4, 3).cache_identity());
    assert_ne!(baseline, context(1, 2, 4).cache_identity());
    assert_ne!(
        context(1, 2, 3).cache_identity(),
        context(2, 1, 3).cache_identity(),
        "field order must be domain-separated, not treated as an unordered bag"
    );
}

#[test]
fn context_round_trips_only_opaque_fixed_width_identities() {
    let context = context(7, 8, 9);

    assert_eq!(
        context.policy_snapshot(),
        PolicySnapshotId::from_digest(digest(7)).unwrap()
    );
    assert_eq!(
        context.subject(),
        SubjectIdentity::from_digest(digest(8)).unwrap()
    );
    assert_eq!(
        context.request_attributes(),
        RequestAttributesIdentity::from_digest(digest(9)).unwrap()
    );
    assert_eq!(size_of::<SecurityContext>(), 96);
}

#[test]
fn diagnostics_and_errors_never_render_identity_material() {
    let context = context(0xa1, 0xb2, 0xc3);
    let forbidden = ["a1".repeat(32), "b2".repeat(32), "c3".repeat(32)];
    let rendered = [
        format!("{context:?}"),
        format!("{context}"),
        format!("{:?}", context.policy_snapshot()),
        format!("{}", context.policy_snapshot()),
        format!("{:?}", context.subject()),
        format!("{}", context.subject()),
        format!("{:?}", context.request_attributes()),
        format!("{}", context.request_attributes()),
        format!("{:?}", context.cache_identity()),
        format!("{}", context.cache_identity()),
    ];

    for output in rendered {
        assert!(output.contains("redacted"), "output={output}");
        for secret in &forbidden {
            assert!(!output.contains(secret), "output={output}");
        }
    }

    for error in [
        SecurityIdentityError::EmptyPolicySnapshot,
        SecurityIdentityError::EmptySubject,
        SecurityIdentityError::EmptyRequestAttributes,
    ] {
        let output = format!("{error:?} {error}");
        for secret in &forbidden {
            assert!(!output.contains(secret), "output={output}");
        }
    }
}
