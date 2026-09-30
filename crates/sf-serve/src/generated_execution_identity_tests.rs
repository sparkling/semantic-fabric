use super::*;
use sf_core::query_control::{
    QueryBudget, QueryLimits, ReservationLimits, UncontrolledQueryControl,
};
use sf_core::security_context::{
    PolicySnapshotId, RequestAttributesIdentity, SecurityContext, SubjectIdentity,
};
use sf_core::SourceId;
use std::time::Duration;

use crate::generated_http_test_support::{build, config, open, ASK, SELECT};
use crate::QueryShapeProfile::GeneratedSelectAsk as GENERATED;

fn issue(cfg: &ServeConfig, query: &str, budget: &RequestBudget) -> GeneratedResponseIdentity {
    let lease = cfg.runtime_lease().unwrap();
    let binding = lease
        .snapshot()
        .registry()
        .binding(SourceId::new(0).unwrap())
        .unwrap();
    let compiled = binding
        .compile_generated(query, &UncontrolledQueryControl)
        .unwrap();
    mint(binding, cfg, query, compiled.identity, budget).unwrap()
}

#[test]
fn exact_query_bytes_bind_while_legacy_profile_stays_equal() {
    let (cfg, _) = config(GENERATED, open());
    let first = issue(&cfg, SELECT, &cfg.request_budget());
    let same = issue(&cfg, SELECT, &cfg.request_budget());
    let spaced = issue(&cfg, &format!("{SELECT} "), &cfg.request_budget());
    let ask = issue(&cfg, ASK, &cfg.request_budget());
    assert_eq!(first.execution_wire(), same.execution_wire());
    for other in [spaced, ask] {
        assert_eq!(first.profile().wire(), other.profile().wire());
        assert_ne!(first.execution_wire(), other.execution_wire());
    }
    let wire = first.execution_wire();
    assert_eq!(wire.len(), 70);
    assert!(wire.starts_with("sfgx1:"));
    assert!(wire[6..]
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    assert_eq!(
        format!("{first:?}"),
        "GeneratedResponseIdentity(<redacted>)"
    );
}

#[test]
fn every_configured_ceiling_changes_identity() {
    let baseline = |index: Option<usize>| {
        let (cfg, _) = build(GENERATED, open(), |cfg| {
            if let Some(i) = index {
                let q = cfg.query_limits;
                let r = q.reservation_limits();
                let add = |slot: usize| u64::from(i == slot);
                match i {
                    0 => cfg.timeout += Duration::from_secs(1),
                    1 => cfg.timeout += Duration::from_nanos(1),
                    2 => cfg.set_max_query_len(cfg.max_query_len() + 1).unwrap(),
                    3 => cfg.set_max_order_rows(cfg.max_order_rows() + 1),
                    _ => {
                        // Decrease finite/unbounded values alike without overflow.
                        cfg.query_limits = QueryLimits::new(
                            q.max_compiler_work() - add(4),
                            q.max_source_work() - add(5),
                            q.max_result_items() - add(6),
                            q.max_serialized_bytes() - add(7),
                        )
                        .with_reservation_limits(ReservationLimits::new(
                            r.max_retained_bytes() - add(8),
                            r.max_spill_bytes() - add(9),
                            r.max_spill_files() - add(10),
                            r.max_file_descriptors() - add(11),
                            r.max_operator_tasks() - add(12),
                        ));
                    }
                }
            }
        });
        issue(&cfg, SELECT, &cfg.request_budget()).execution_wire()
    };
    let original = baseline(None);
    for index in 0..13 {
        assert_ne!(original, baseline(Some(index)), "ceiling {index}");
    }
}

fn secured(policy: u8, subject: u8, attributes: u8) -> RequestBudget {
    let mut budget = RequestBudget::after(
        Duration::from_secs(30),
        QueryLimits::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX),
    );
    let context = SecurityContext::new(
        PolicySnapshotId::from_digest([policy; 32]).unwrap(),
        SubjectIdentity::from_digest([subject; 32]).unwrap(),
        RequestAttributesIdentity::from_digest([attributes; 32]).unwrap(),
    );
    budget.retain_security(context).unwrap();
    budget
}

#[test]
fn policy_presence_and_value_bind_without_private_subject_partition() {
    let (cfg, _) = config(GENERATED, open());
    let plain = issue(&cfg, SELECT, &cfg.request_budget()).execution_wire();
    let secured_one = issue(&cfg, SELECT, &secured(1, 2, 3)).execution_wire();
    assert_ne!(plain, secured_one);
    assert_ne!(
        secured_one,
        issue(&cfg, SELECT, &secured(4, 2, 3)).execution_wire()
    );
    assert_eq!(
        secured_one,
        issue(&cfg, SELECT, &secured(1, 8, 9)).execution_wire()
    );
}

#[test]
fn mutable_consumption_does_not_change_configured_identity() {
    let (cfg, _) = config(GENERATED, open());
    let budget = cfg.request_budget();
    let first = issue(&cfg, SELECT, &budget).execution_wire();
    budget.consume(QueryCharge::SourceWork, 1).unwrap();
    assert_eq!(first, issue(&cfg, SELECT, &budget).execution_wire());
}

#[test]
fn trusted_binding_source_epoch_and_mapping_change_execution_identity() {
    use crate::generated_http_test_support::{ontology, source, MAPPING};
    use crate::semantic_admission::{MappingOrigin, ValidatedMapping};
    use sf_sparql::Epoch;
    let (cfg, _) = config(GENERATED, open());
    let bind = |source_index, epoch, mapping: &str| {
        let (observed, _) = source();
        let ontology = ontology(&[]);
        let mapping =
            sf_mapping::parse_r2rml_for_source(mapping, SourceId::new(source_index).unwrap())
                .unwrap();
        let validated =
            ValidatedMapping::validate(mapping, MappingOrigin::Authored, &ontology, &observed)
                .unwrap();
        RuntimeBinding::new(observed, validated, ontology.tbox().clone(), Epoch(epoch))
    };
    let original = bind(0, 0, MAPPING);
    let compiled = original
        .compile_generated(SELECT, &UncontrolledQueryControl)
        .unwrap();
    let profile = compiled.identity;
    let identity = |binding: &RuntimeBinding| {
        mint(binding, &cfg, SELECT, profile, &cfg.request_budget())
            .unwrap()
            .execution_wire()
    };
    let initial = identity(&original);
    assert_ne!(initial, identity(&bind(1, 0, MAPPING)));
    assert_ne!(initial, identity(&bind(0, 1, MAPPING)));
    assert_ne!(
        initial,
        identity(&bind(0, 0, &MAPPING.replace("/person/", "/member/")))
    );
    // Deliberately hold profile fixed here: these differences must come from
    // exact compiler facts, not merely copying the legacy profile hash.
    assert_eq!(initial, identity(&bind(0, 0, MAPPING)));
}

#[test]
fn framing_has_stable_golden_and_distinguishes_field_boundaries() {
    let free = UncontrolledQueryControl;
    let encode = |fields: &[&[u8]]| {
        let mut out = Framed::new(&free);
        for field in fields {
            out.field(field).unwrap();
        }
        format!("{:x}", out.hash.finalize())
    };
    assert_eq!(
        encode(&[b"ab", b"c"]),
        "601d5476e2ccfe2c87a2bba7a322659734a05749d5b5aa781f513e4912db0d5f"
    );
    assert_ne!(encode(&[b"ab", b"c"]), encode(&[b"a", b"bc"]));
    assert_ne!(encode(&[]), encode(&[b""]));
}

#[test]
fn query_hash_has_exact_paid_boundary_and_sticky_cancel() {
    let text = vec![b'x'; CHUNK * 2 + 17];
    let required = text.len() as u64 + 8;
    for (work, succeeds) in [(required, true), (required - 1, false)] {
        let budget = QueryBudget::new(QueryLimits::new(work, 1, 1, 1));
        let result = Framed::new(&budget).field(&text);
        assert_eq!(result.is_ok(), succeeds);
        if succeeds {
            assert_eq!(budget.consumed(QueryCharge::CompilerWork), required);
        } else {
            assert_eq!(result, Err(QueryControlError::CompilerWorkExceeded));
            assert_eq!(budget.checkpoint(), result);
        }
    }
    let budget = QueryBudget::new(QueryLimits::new(required, 1, 1, 1));
    budget.terminate(QueryControlError::Cancelled);
    assert_eq!(
        Framed::new(&budget).field(&text),
        Err(QueryControlError::Cancelled)
    );
    assert_eq!(budget.consumed(QueryCharge::CompilerWork), 0);
}

fn vector() -> Inputs<'static> {
    Inputs {
        query: "ASK {}",
        source: 7,
        dialect: b"sqlite",
        epoch: 9,
        digests: std::array::from_fn(|i| [i as u8 + 1; 32]),
        profile: format!("sfgp1:{}", "00".repeat(32)),
        policy: Some([11; 32]),
        ceilings: std::array::from_fn(|i| i as u64 + 1),
    }
}

#[test]
fn full_wire_golden_and_each_independent_input_are_bound() {
    let (cfg, _) = config(GENERATED, open());
    let profile = issue(&cfg, SELECT, &cfg.request_budget()).profile;
    let wire = |inputs: Inputs<'_>| {
        GeneratedResponseIdentity {
            profile,
            execution: inputs.digest(&UncontrolledQueryControl).unwrap(),
        }
        .execution_wire()
    };
    let baseline = wire(vector());
    // Independently computed SHA-256 of all versioned u64-BE framed fields,
    // including all eight distinct digests and thirteen ordered policy ceilings.
    assert_eq!(
        baseline,
        "sfgx1:169ddac8df7824a24f814c3dcb6c56fa5e78b8032cd07ce53c7faab43cb34d64"
    );
    for index in 0..8 {
        let mut input = vector();
        input.digests[index][0] ^= 0x80;
        assert_ne!(baseline, wire(input), "digest {index}");
    }
    for index in 0..13 {
        let mut input = vector();
        input.ceilings[index] += 1;
        assert_ne!(baseline, wire(input), "ceiling {index}");
    }
    for index in 0..7 {
        let mut input = vector();
        match index {
            0 => input.query = "ASK {} ",
            1 => input.source += 1,
            2 => input.dialect = dialect(Dialect::Postgres),
            3 => input.epoch += 1,
            4 => input.profile.replace_range(6..8, "01"),
            5 => input.policy = Some([12; 32]),
            6 => input.policy = None,
            _ => unreachable!(),
        }
        assert_ne!(baseline, wire(input), "field {index}");
    }
    let mut absent = vector();
    absent.policy = None;
    let mut zero = vector();
    zero.policy = Some([0; 32]);
    assert_ne!(wire(absent), wire(zero));
}
